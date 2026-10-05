//! Native child trees for live naval combat hovers. 14195C7D0 and 141963700
//! call 14195EEC0, whose ship route is 140C29020 -> 140C230A0 ->
//! 14054D0F0; the convoy route is 1415B4BB0 -> 14054D0F0. Side screening
//! and positioning use 14160A7B0/141609460 -> 1415B4BB0. Only live combat
//! sides, escaping members and drawable carrier air reach this reader.

use crate::{
    Memory, ccountry, cfex_air, cfex_member, cnaval_combat, cnaval_combatant, cpdx_array::Array,
    modifier_accumulator, modifier_definition, modifier_keys, msvc_string,
    naval_ship_modifier_inputs, read,
};

/// NNavy::COMBAT_INITIAL_DURATION native global read by 14195C7D0 and
/// loaded by 14092B4B0; 140C29020 uses it for early combat sources.
pub const COMBAT_INITIAL_DURATION_RVA: usize = 0x0331_d1ac;

/// Native hover keys reached by 1415B4BB0/140C230A0, sorted for one
/// 140551820 accumulator census per node.
pub const KEYS: [u32; 9] = [
    modifier_keys::SORTIE_EFFICIENCY,
    modifier_keys::NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT,
    modifier_keys::NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK,
    modifier_keys::NAVAL_RETREAT_SPEED,
    modifier_keys::SUB_RETREAT_SPEED,
    modifier_keys::CONVOY_RETREAT_SPEED,
    modifier_keys::POSITIONING,
    modifier_keys::SCREENING_WITHOUT_SCREENS,
    modifier_keys::FIGHTER_SORTIE_EFFICIENCY,
];
pub const REQUEST_CAPACITY: &str = "naval modifier request arena capacity exceeded";
pub const ROOT_CAPACITY: &str = "naval modifier root arena capacity exceeded";
pub const NODE_CAPACITY: &str = "naval modifier node arena capacity exceeded";
pub const TEXT_CAPACITY: &str = "naval modifier text arena capacity exceeded";
pub const DEPTH_CAPACITY: &str = "naval modifier depth arena capacity exceeded";
pub const STACK_HEADROOM: &str = "naval modifier census worker stack headroom exhausted";

// Leave 64 KiB for guard pages, guarded node reads and returning the refusal.
// This protects the worker's actual reserve (4 MiB in acquisition), not a
// chosen limit on native tree depth.
const STACK_MARGIN: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct StackLimits {
    low: usize,
    high: usize,
}

impl StackLimits {
    #[cfg(windows)]
    fn current() -> Result<Self, &'static str> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        }
        let mut limits = Self { low: 0, high: 0 };
        // Windows returns the current thread's stack reservation boundaries;
        // no game memory or function is involved.
        unsafe { GetCurrentThreadStackLimits(&mut limits.low, &mut limits.high) };
        if limits.low == 0 || limits.low >= limits.high {
            return Err("naval modifier census worker stack limits unavailable");
        }
        Ok(limits)
    }

    #[cfg(not(windows))]
    fn current() -> Result<Self, &'static str> {
        Err("naval modifier census worker stack limits unavailable")
    }

    fn check_at(&self, pointer: usize) -> Result<(), &'static str> {
        if pointer >= self.high || pointer.saturating_sub(self.low) < STACK_MARGIN {
            Err(STACK_HEADROOM)
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    fn check_current(&self) -> Result<(), &'static str> {
        let marker = 0_u8;
        self.check_at(std::ptr::addr_of!(marker) as usize)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Use {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub source_scope: u8,
    pub source_index: u32,
    /// Source order before requests are sorted by accumulator identity.
    pub order: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub accumulator: usize,
    pub key: u32,
    pub usage: Use,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RawNode {
    pub children: u32,
    pub name: [usize; 2],
    /// Effective 140551820 values in [`KEYS`] order; absent keys are zero.
    pub values: [i64; KEYS.len()],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    pub accumulator: usize,
    pub nodes: [usize; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    address: usize,
    children: Array,
    next: u32,
}

/// All storage is pack-owned and grows only after the hourly block. A short
/// arena refuses the complete Table 270 for that hour and reports demand.
/// `*_len` is a stored prefix; `*_required` is full demand, counting each tree once.
/// Root.nodes uses census offsets; every short arena returns Err, never publishes.
pub struct Buffers<'a> {
    pub requests: &'a mut [Request],
    pub request_len: usize,
    pub request_required: usize,
    pub roots: &'a mut [Root],
    pub root_len: usize,
    pub root_required: usize,
    pub nodes: &'a mut [RawNode],
    pub node_len: usize,
    pub node_required: usize,
    pub text: &'a mut [u8],
    pub text_len: usize,
    pub text_required: usize,
    pub stack: &'a mut [Frame],
    pub stack_required: usize,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("naval modifier breakdown pointer overflow")
}
fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("naval modifier breakdown field unreadable")
}
fn push(
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
    member: &cfex_member::DetailsRow,
    scope: u8,
    index: u32,
    key: u32,
    accumulator: usize,
) -> Result<(), &'static str> {
    push_side(
        emit,
        member.member.combat,
        member.member.side,
        member.member.battle_slot,
        scope,
        index,
        key,
        accumulator,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep native identity and source coordinates explicit when recording a request"
)]
fn push_side(
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
    combat: u64,
    side: u8,
    battle_slot: u32,
    scope: u8,
    index: u32,
    key: u32,
    accumulator: usize,
) -> Result<(), &'static str> {
    emit(Request {
        accumulator,
        key,
        usage: Use {
            combat,
            side,
            battle_slot,
            source_scope: scope,
            source_index: index,
            order: 0,
        },
    })
}

fn record_request(mut request: Request, buffers: &mut Buffers<'_>) -> Result<(), &'static str> {
    request.usage.order = u32::try_from(buffers.request_required)
        .map_err(|_| "naval modifier request order overflow")?;
    buffers.request_required = buffers
        .request_required
        .checked_add(1)
        .ok_or("naval modifier request length overflow")?;
    if let Some(slot) = buffers.requests.get_mut(buffers.request_len) {
        *slot = request;
        buffers.request_len += 1;
    }
    Ok(())
}

/// Only refusal hours re-enumerate requests to select successive distinct
/// accumulator addresses and their actual key unions. Each tree is walked
/// once, even with zero request storage; demand equals the next full capture.
fn census_requested_roots(
    memory: &impl Memory,
    base: usize,
    enumerate: &mut impl FnMut(
        &mut dyn FnMut(Request) -> Result<(), &'static str>,
    ) -> Result<(), &'static str>,
    buffers: &mut Buffers<'_>,
    stack_limits: &Result<StackLimits, &'static str>,
) -> Result<(), &'static str> {
    let mut previous = None;
    loop {
        let mut next = None;
        let mut wanted = [false; KEYS.len()];
        enumerate(&mut |request| {
            if previous.is_some_and(|previous| request.accumulator <= previous) {
                return Ok(());
            }
            if next.is_none_or(|next| request.accumulator < next) {
                next = Some(request.accumulator);
                wanted.fill(false);
            }
            if next == Some(request.accumulator) {
                wanted[KEYS
                    .binary_search(&request.key)
                    .map_err(|_| "naval modifier hover key unsupported")?] = true;
            }
            Ok(())
        })?;
        let Some(root) = next else { return Ok(()) };
        capture_requested_root(memory, base, root, wanted, false, buffers, stack_limits)?;
        previous = Some(root);
    }
}

/// 14160C6D0's forward-list predicate for the country-only 0x1B3 hover.
fn eligible_submarine(members: &[cfex_member::DetailsRow], side: u8) -> bool {
    members.iter().any(|member| {
        member.member.side == side
            && member.member_source == 0
            && member.group_index != u32::MAX
            && member.live_ship != 0
            && member.member.state & 0x34 == 0
            && member.formation_flags & (1 << 7) != 0
    })
}

/// 141609460 and 14160A7B0 call 1415B4BB0 for each live side. The latter
/// walks country, weather, terrain/ocean and commander sources in this order.
/// 0x1B3 passes null province and commander, so it has country only.
fn side_trees(
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: &cnaval_combat::DetailsRow,
    members: &[cfex_member::DetailsRow],
    initial: i32,
) -> Result<(), &'static str> {
    let body = combat.native_combat;
    if field::<u64>(memory, body, 0)? != at(base, cnaval_combat::VTABLE_RVA)? as u64
        || field::<u64>(memory, body, cnaval_combat::HANDLE)? != combat.combat
    {
        return Err("naval side breakdown combat identity mismatch");
    }
    for (side_index, offset) in [cnaval_combat::ATTACKER, cnaval_combat::DEFENDER]
        .into_iter()
        .enumerate()
    {
        let side = field::<u64>(memory, body, offset)? as usize;
        if field::<u64>(memory, side, 0)? != at(base, cnaval_combatant::VTABLE_RVA)? as u64
            || field::<u64>(memory, side, cnaval_combatant::COMBAT)? as usize != body
        {
            return Err("naval side breakdown side identity mismatch");
        }
        // Both native description paths skip modifier roots for an empty side.
        if field::<i32>(memory, side, 0x48)? == 0 {
            continue;
        }
        // CCombatant+0x38 is the head pointer, not the raw tag. The first
        // node's +0 word is the selector 1415B4BB0 resolves.
        let country_head = field::<u64>(memory, side, 0x38)? as usize;
        let country_link: u32 = field(memory, country_head, 0)?;
        let country = ccountry::Reader::new(memory, base, game_state)?.resolve(country_link)?;
        let province = field::<u64>(memory, body, cnaval_combat::PROVINCE)? as usize;
        let definition = field::<u64>(memory, province, 0xc8)? as usize;
        let region: u32 = field(memory, definition, 0x58)?;
        let weather =
            field::<u64>(memory, game_state, crate::cgame_state::WEATHER_MANAGER)? as usize;
        let regions = Array::read(memory, at(weather, 0x40)?, 0x160)
            .ok_or("naval side breakdown weather region array invalid")?;
        // 140F04E40 returns null outside the live region array; 1415B4BB0
        // simply omits that optional weather source.
        let region_at = regions.element(region, 0x160);
        let terrain = naval_ship_modifier_inputs::terrain(memory, base, definition)?;
        let current_leader = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
        // 1416094A5 calls the opposite-side getter with this side in RCX;
        // 1416094B1 saves its +0x110, then 14160969A passes it for 0x5A.
        let opposite_offset = if side_index == 0 {
            cnaval_combat::DEFENDER
        } else {
            cnaval_combat::ATTACKER
        };
        let opposite = field::<u64>(memory, body, opposite_offset)? as usize;
        if field::<u64>(memory, opposite, 0)? != at(base, cnaval_combatant::VTABLE_RVA)? as u64
            || field::<u64>(memory, opposite, cnaval_combatant::COMBAT)? as usize != body
        {
            return Err("naval side breakdown opposite identity mismatch");
        }
        let opposing_leader = field::<u64>(memory, opposite, cnaval_combatant::COMMANDER)? as usize;
        // 14160A7B0 discards the 0x1B1 description. Its 0x1B3 branch
        // requires 14160C6D0's eligible-submarine count to be zero. The
        // capital-screening condition is recomputed on demand by the hover,
        // so its stored scalar is not a safe capture gate.
        let keys = [
            (!eligible_submarine(members, side_index as u8)).then_some(0x1b3),
            Some(0x1b2),
            (combat.duration < initial && field::<u8>(memory, side, 0xb8)? == 0).then_some(0x5a),
        ];
        for key in keys.into_iter().flatten() {
            push_side(
                emit,
                combat.combat,
                side_index as u8,
                u32::MAX,
                0,
                0,
                key,
                at(country.pointer, 0x5b8)?,
            )?;
            if key == 0x1b3 {
                continue;
            }
            if let Some(region_at) = region_at {
                push_side(
                    emit,
                    combat.combat,
                    side_index as u8,
                    u32::MAX,
                    6,
                    0,
                    key,
                    at(region_at, 0x58)?,
                )?;
            }
            push_side(
                emit,
                combat.combat,
                side_index as u8,
                u32::MAX,
                5,
                0,
                key,
                at(terrain, 0xb8)?,
            )?;
            let leader = if key == 0x5a {
                opposing_leader
            } else {
                current_leader
            };
            if leader != 0 {
                if field::<u64>(memory, leader, 0)?
                    != at(base, crate::cnavy_leader::VTABLE_RVA)? as u64
                {
                    return Err("naval side breakdown commander identity mismatch");
                }
                push_side(
                    emit,
                    combat.combat,
                    side_index as u8,
                    u32::MAX,
                    9,
                    0,
                    key,
                    at(leader, 0x418)?,
                )?;
            }
        }
    }
    Ok(())
}

/// The keys selected by 14195C7D0. The initial-duration define is read from
/// its signed native global; 14092B4B0 loads that value from NNavy.
fn keys(member: &cfex_member::DetailsRow, duration: i32, initial: i32) -> &'static [u32] {
    if member.member.state != 8 {
        return &[];
    }
    if member.live_ship != 0 {
        if member.formation_flags & (1 << 7) != 0 {
            &[0x5c]
        } else if duration > initial {
            &[0x5b, 0x59]
        } else {
            &[0x5b]
        }
    } else if member.live_convoy != 0 {
        &[0x5d]
    } else {
        &[]
    }
}

/// Capture only roots reached by live side, state-8 member and drawable
/// carrier hovers. The combat walk groups members by combat, so each combat
/// uses its own indexed slice. Requests are sorted in place by accumulator;
/// each distinct accumulator is walked once, regardless of side/key/ship use.
#[allow(
    clippy::too_many_arguments,
    reason = "Capture consumes separately owned native input slices and caller-owned buffers"
)]
pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combats: &[cnaval_combat::DetailsRow],
    members: &[cfex_member::DetailsRow],
    planes: &[cfex_air::Row],
    sources: &[naval_ship_modifier_inputs::SourceRow],
    buffers: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    if combats.is_empty() {
        return Ok(());
    }
    // Query once for this job; an unavailable limit refuses only a walk that
    // actually needs the recursive fallback.
    let stack_limits = StackLimits::current();
    let mut enumerate = |emit: &mut dyn FnMut(Request) -> Result<(), &'static str>| {
        enumerate_requests(
            memory,
            base,
            game_state,
            combats,
            members,
            planes,
            sources,
            &mut |request| emit(request),
        )
    };
    enumerate(&mut |request| record_request(request, buffers))?;
    let store = buffers.request_required <= buffers.requests.len();
    if !store {
        census_requested_roots(memory, base, &mut enumerate, buffers, &stack_limits)?;
    } else {
        buffers.requests[..buffers.request_len]
            .sort_unstable_by_key(|r| (r.accumulator, r.key, r.usage.order));
        let mut at = 0;
        while at < buffers.request_len {
            let root = buffers.requests[at].accumulator;
            let mut end = at + 1;
            let mut wanted = [false; KEYS.len()];
            wanted[KEYS
                .binary_search(&buffers.requests[at].key)
                .map_err(|_| "naval modifier hover key unsupported")?] = true;
            while end < buffers.request_len && buffers.requests[end].accumulator == root {
                wanted[KEYS
                    .binary_search(&buffers.requests[end].key)
                    .map_err(|_| "naval modifier hover key unsupported")?] = true;
                end += 1;
            }
            capture_requested_root(memory, base, root, wanted, true, buffers, &stack_limits)?;
            at = end;
        }
    }
    // Partial storage never becomes a published tree. All observed demands
    // survive the refusal and drive the existing off-thread arena growth.
    if buffers.request_required > buffers.requests.len() {
        Err(REQUEST_CAPACITY)
    } else if buffers.stack_required > buffers.stack.len() {
        Err(DEPTH_CAPACITY)
    } else if buffers.root_required > buffers.roots.len() {
        Err(ROOT_CAPACITY)
    } else if buffers.node_required > buffers.nodes.len() {
        Err(NODE_CAPACITY)
    } else if buffers.text_required > buffers.text.len() {
        Err(TEXT_CAPACITY)
    } else {
        Ok(())
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Separate native census inputs and request consumer"
)]
fn enumerate_requests(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combats: &[cnaval_combat::DetailsRow],
    members: &[cfex_member::DetailsRow],
    planes: &[cfex_air::Row],
    sources: &[naval_ship_modifier_inputs::SourceRow],
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let initial: i32 = field(memory, base, COMBAT_INITIAL_DURATION_RVA)?;
    let mut member_cursor = 0;
    for combat in combats {
        let start = member_cursor;
        while members
            .get(member_cursor)
            .is_some_and(|m| m.member.combat == combat.combat)
        {
            member_cursor += 1;
        }
        let combat_members = &members[start..member_cursor];
        side_trees(
            emit,
            memory,
            base,
            game_state,
            combat,
            combat_members,
            initial,
        )?;
        for member in combat_members {
            let mut selected = [0_u32; 4];
            let mut count = 0;
            for &key in keys(member, combat.duration, initial) {
                selected[count] = key;
                count += 1;
            }
            for (side, side_row) in combat.sides.iter().enumerate() {
                if side as u8 != member.member.side {
                    continue;
                }
                let [first, amount] = side_row.planes;
                let end = first
                    .checked_add(amount)
                    .ok_or("naval carrier plane span overflow")?;
                for plane in planes
                    .get(first..end)
                    .ok_or("naval carrier plane span invalid")?
                {
                    if plane.source != 0
                        || plane.group_index == u32::MAX
                        || plane.variant == 0
                        || plane.carrier_state & 4 != 0
                        || plane.carrier_member_present == 0
                        || plane.carrier_slot < 0
                        || plane.carrier_slot as u32 != member.member.battle_slot
                    {
                        continue;
                    }
                    if !selected[..count].contains(&0x24) {
                        selected[count] = 0x24;
                        count += 1;
                    }
                    if plane.efficiency.pool_type_flags & 0x1_0000_0400 != 0
                        && !selected[..count].contains(&0x1b6)
                    {
                        selected[count] = 0x1b6;
                        count += 1;
                    }
                }
            }
            for &key in &selected[..count] {
                if member.live_ship != 0 {
                    let [first, amount] = member.weapon_context.sources;
                    let end = first
                        .checked_add(amount)
                        .ok_or("naval modifier source span overflow")?;
                    for source in sources
                        .get(first..end)
                        .ok_or("naval modifier source span invalid")?
                    {
                        if source.accumulator != 0 && matches!(source.scope, 0..=15) {
                            push(
                                emit,
                                member,
                                source.scope,
                                source.index,
                                key,
                                source.accumulator,
                            )?;
                        }
                    }
                } else if member.live_convoy != 0 {
                    convoy(memory, base, game_state, member, key, emit)?;
                } else if key != 0x24 && key != 0x1b6 {
                    return Err("state-8 naval member has no live ship or convoy");
                } else {
                    return Err("naval carrier air member has no live ship row");
                }
            }
        }
        for (side, side_row) in combat.sides.iter().enumerate() {
            let [first, amount] = side_row.planes;
            let end = first
                .checked_add(amount)
                .ok_or("naval carrier plane span overflow")?;
            for plane in planes
                .get(first..end)
                .ok_or("naval carrier plane span invalid")?
            {
                if plane.source == 0
                    && plane.group_index != u32::MAX
                    && plane.variant != 0
                    && plane.carrier_state & 4 == 0
                    && plane.carrier_member_present != 0
                    && plane.carrier_slot >= 0
                    && !combat_members.iter().any(|m| {
                        m.member.side == side as u8
                            && m.member.battle_slot == plane.carrier_slot as u32
                    })
                {
                    return Err("naval carrier air member has no live ship row");
                }
            }
        }
    }
    if member_cursor != members.len() {
        return Err("naval modifier member is outside the indexed combat census");
    }
    Ok(())
}

fn capture_requested_root(
    memory: &impl Memory,
    base: usize,
    root: usize,
    wanted: [bool; KEYS.len()],
    store: bool,
    buffers: &mut Buffers<'_>,
    stack_limits: &Result<StackLimits, &'static str>,
) -> Result<(), &'static str> {
    let index = buffers.root_required;
    buffers.root_required = index
        .checked_add(1)
        .ok_or("naval modifier root length overflow")?;
    let first = buffers.node_required;
    capture_root(memory, base, root, wanted, store, buffers, stack_limits)?;
    if store && let Some(slot) = buffers.roots.get_mut(index) {
        *slot = Root {
            accumulator: root,
            nodes: [first, buffers.node_required - first],
        };
        buffers.root_len += 1;
    }
    Ok(())
}

/// FormatModifierBreakdown@14054D0F0 receives depth -1 from both
/// 1415B4BB0 and 140C230A0: its full native tree must be captured. Each
/// distinct accumulator is traversed once; only requested keys are read
/// from its sorted entry array in that visit. Zero subtrees are pruned after
/// capture, outside the hourly block.
fn capture_root(
    memory: &impl Memory,
    base: usize,
    root: usize,
    wanted: [bool; KEYS.len()],
    store: bool,
    buffers: &mut Buffers<'_>,
    stack_limits: &Result<StackLimits, &'static str>,
) -> Result<(), &'static str> {
    let mut keys = [0_u32; KEYS.len()];
    let mut positions = [0_usize; KEYS.len()];
    let mut metadata = [modifier_definition::Operands::default(); KEYS.len()];
    let mut key_count = 0;
    for (position, &key) in KEYS.iter().enumerate() {
        if wanted[position] {
            keys[key_count] = key;
            positions[key_count] = position;
            metadata[key_count] = modifier_definition::operands(memory, base, key)?;
            key_count += 1;
        }
    }
    let selection = Selection {
        keys,
        positions,
        metadata,
        len: key_count,
    };
    let mut address = root;
    let mut depth = 0;
    loop {
        if address == 0 || buffers.stack[..depth].iter().any(|f| f.address == address) {
            return Err("naval modifier null node or ancestor cycle");
        }
        // Once retained scratch ends, count the entire remaining subtree with
        // call-stack frames. Later hours need not discover deeper demand.
        if depth == buffers.stack.len() {
            let limits = stack_limits.as_ref().map_err(|reason| *reason)?;
            limits.check_current()?;
            walk_without_scratch(
                memory, address, &selection, store, buffers, depth, None, limits,
            )?;
        } else {
            let children = capture_node(memory, address, &selection, store, buffers)?;
            buffers.stack_required = buffers.stack_required.max(depth + 1);
            buffers.stack[depth] = Frame {
                address,
                children,
                next: 0,
            };
            depth += 1;
        }
        loop {
            if depth == 0 {
                return Ok(());
            }
            let frame = &mut buffers.stack[depth - 1];
            if frame.next == frame.children.count {
                depth -= 1;
                continue;
            }
            address = child_address(memory, frame.children, frame.next)?;
            frame.next += 1;
            break;
        }
    }
}

struct Selection {
    keys: [u32; KEYS.len()],
    positions: [usize; KEYS.len()],
    metadata: [modifier_definition::Operands; KEYS.len()],
    len: usize,
}

fn child_address(memory: &impl Memory, children: Array, index: u32) -> Result<usize, &'static str> {
    let pointer = children
        .element(index, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or("naval modifier child pointer unreadable or null")?;
    pointer
        .checked_add(0x10)
        .ok_or("naval modifier child address overflow")
}

// Keep entry/string temporaries out of recursive ancestor frames.
#[inline(never)]
fn capture_node(
    memory: &impl Memory,
    address: usize,
    selection: &Selection,
    store: bool,
    buffers: &mut Buffers<'_>,
) -> Result<Array, &'static str> {
    let children = Array::read(memory, at(address, 0x18)?, 8)
        .ok_or("naval modifier children unreadable or invalid")?;
    let mut raw = [modifier_accumulator::Value::default(); KEYS.len()];
    modifier_accumulator::lookup_many_at(
        memory,
        address,
        &selection.keys[..selection.len],
        &mut raw[..selection.len],
    )?;
    let mut values = [0_i64; KEYS.len()];
    for i in 0..selection.len {
        if raw[i].present {
            values[selection.positions[i]] = selection.metadata[i].effective(raw[i].raw);
        }
    }
    // With depth -1, 14054D0F0 prints only nonzero leaf values. An
    // interior node and an all-zero leaf never need display bytes.
    let name_first = buffers.text_required;
    let name_len = if children.count == 0 && values.iter().any(|&value| value != 0) {
        let name_address = at(address, 0x48)?;
        let header = read::<[u8; msvc_string::STRIDE]>(memory, name_address)
            .ok_or("naval modifier display header unreadable")?;
        let name = msvc_string::Descriptor::from_header(memory, name_address, header)?;
        let name_end = buffers
            .text_required
            .checked_add(name.len)
            .ok_or("naval modifier text length overflow")?;
        buffers.text_required = name_end;
        if store && let Some(output) = buffers.text.get_mut(name_first..name_end) {
            name.copy(memory, output)?;
            buffers.text_len = name_end;
        }
        name.len
    } else {
        0
    };
    let node_index = buffers.node_required;
    buffers.node_required = node_index
        .checked_add(1)
        .ok_or("naval modifier node length overflow")?;
    if store && let Some(node) = buffers.nodes.get_mut(node_index) {
        *node = RawNode {
            children: children.count,
            name: [name_first, name_len],
            values,
        };
        buffers.node_len += 1;
    }
    Ok(children)
}

struct Ancestor<'a> {
    address: usize,
    parent: Option<&'a Ancestor<'a>>,
}

/// Native retention is governed by accumulator+0xAC: 14054AD00/14054AF60
/// clone with depth-1; 1405FFDE0 -> 1406000C0 stops cloning at zero.
/// Copy-preserving 1406000C0 uses each child's retained depth, not a global
/// numerical bound. Hover 14054D0F0 receives -1 and walks all retained levels.
/// Ordinary jobs run only on acquisition workers (explicit 4 MiB reserve in
/// collector::acquisition); each descent checks actual reservation headroom,
/// with no chosen tree-depth cap or heap allocation.
#[inline(never)]
#[allow(
    clippy::too_many_arguments,
    reason = "Borrowed ancestor frames replace short heap scratch"
)]
fn walk_without_scratch(
    memory: &impl Memory,
    address: usize,
    selection: &Selection,
    store: bool,
    buffers: &mut Buffers<'_>,
    prefix_depth: usize,
    ancestors: Option<&Ancestor<'_>>,
    stack_limits: &StackLimits,
) -> Result<(), &'static str> {
    if address == 0
        || buffers.stack[..prefix_depth]
            .iter()
            .any(|f| f.address == address)
    {
        return Err("naval modifier null node or ancestor cycle");
    }
    let mut previous = ancestors;
    let mut depth = prefix_depth + 1;
    while let Some(ancestor) = previous {
        if ancestor.address == address {
            return Err("naval modifier null node or ancestor cycle");
        }
        depth += 1;
        previous = ancestor.parent;
    }
    let children = capture_node(memory, address, selection, store, buffers)?;
    buffers.stack_required = buffers.stack_required.max(depth);
    let current = Ancestor {
        address,
        parent: ancestors,
    };
    for index in 0..children.count {
        let child = child_address(memory, children, index)?;
        stack_limits.check_current()?;
        walk_without_scratch(
            memory,
            child,
            selection,
            store,
            buffers,
            prefix_depth,
            Some(&current),
            stack_limits,
        )?;
    }
    Ok(())
}

fn convoy(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    member: &cfex_member::DetailsRow,
    key: u32,
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let side: usize = field::<u64>(memory, member.native_group, 0x38)? as usize;
    if field::<u64>(memory, side, 0)? != at(base, cnaval_combatant::VTABLE_RVA)? as u64 {
        return Err("naval escape convoy side identity mismatch");
    }
    // 14195EEC0 returns zero without invoking 1415B4BB0 for an empty side.
    if field::<i32>(memory, side, 0x48)? == 0 {
        return Ok(());
    }
    let country_head = field::<u64>(memory, side, 0x38)? as usize;
    let country_link: u32 = field(memory, country_head, 0)?;
    let country = ccountry::Reader::new(memory, base, game_state)?.resolve(country_link)?;
    push(emit, member, 0, 0, key, at(country.pointer, 0x5b8)?)?;
    let combat: usize = field::<u64>(memory, side, cnaval_combatant::COMBAT)? as usize;
    let province: usize = field::<u64>(memory, combat, cnaval_combat::PROVINCE)? as usize;
    let definition: usize = field::<u64>(memory, province, 0xc8)? as usize;
    let region: u32 = field(memory, definition, 0x58)?;
    let weather: usize =
        field::<u64>(memory, game_state, crate::cgame_state::WEATHER_MANAGER)? as usize;
    let regions = Array::read(memory, at(weather, 0x40)?, 0x160)
        .ok_or("naval escape convoy weather region array invalid")?;
    if let Some(region_at) = regions.element(region, 0x160) {
        push(emit, member, 6, 0, key, at(region_at, 0x58)?)?;
    }
    let terrain = naval_ship_modifier_inputs::terrain(memory, base, definition)?;
    push(emit, member, 5, 0, key, at(terrain, 0xb8)?)?;
    let leader: usize = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
    if leader != 0 {
        if field::<u64>(memory, leader, 0)? != at(base, crate::cnavy_leader::VTABLE_RVA)? as u64 {
            return Err("naval escape convoy leader identity mismatch");
        }
        push(emit, member, 9, 0, key, at(leader, 0x418)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{memory::SliceMemory, modifier_definition};

    struct Fixture(Vec<u8>);
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == modifier_definition::TABLE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
                return true;
            }
            if (0x3000_0000..0x3004_0000).contains(&address) {
                output.fill(0);
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(address, output)
        }
    }
    impl Fixture {
        fn new() -> Self {
            Self(vec![0; 0x2000])
        }
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0[at..at + bytes.len()].copy_from_slice(bytes);
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn entry(&mut self, at: usize, key: u32, value: i64) {
            self.put(at, &key.to_le_bytes());
            self.put(at + 8, &value.to_le_bytes());
        }
        fn node(&mut self, at: usize, entries: usize, key: u32, value: i64) {
            self.u64(at, entries as u64);
            self.u64(at + 8, 0x1_0000_0001);
            self.entry(entries, key, value);
        }
    }

    #[test]
    fn census_uses_distinct_accumulators_and_actual_key_unions() {
        struct Counted {
            fixture: Fixture,
            visits: std::cell::Cell<[usize; 2]>,
        }
        impl Memory for Counted {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if let Some(index) = [0x200 + 0x18, 0x400 + 0x18]
                    .iter()
                    .position(|&header| header == address && output.len() == 16)
                {
                    let mut visits = self.visits.get();
                    visits[index] += 1;
                    self.visits.set(visits);
                }
                self.fixture.copy(address, output)
            }
        }
        let mut fixture = Fixture::new();
        // The first root's nonzero positioning key is never requested: its
        // display name must not contribute to the census.
        fixture.node(0x200, 0x600, modifier_keys::POSITIONING, 7_000);
        fixture.node(0x400, 0x620, modifier_keys::POSITIONING, 8_000);
        fixture.put(0x400 + 0x48, b"law");
        fixture.u64(0x400 + 0x48 + 16, 3);
        fixture.u64(0x400 + 0x48 + 24, 15);
        let memory = Counted {
            fixture,
            visits: std::cell::Cell::new([0; 2]),
        };
        let requests = [
            Request {
                accumulator: 0x400,
                key: modifier_keys::POSITIONING,
                ..Default::default()
            },
            Request {
                accumulator: 0x200,
                key: modifier_keys::SORTIE_EFFICIENCY,
                ..Default::default()
            },
            Request {
                accumulator: 0x400,
                key: modifier_keys::SORTIE_EFFICIENCY,
                ..Default::default()
            },
            Request {
                accumulator: 0x200,
                key: modifier_keys::SCREENING_WITHOUT_SCREENS,
                ..Default::default()
            },
            Request {
                accumulator: 0x400,
                key: modifier_keys::POSITIONING,
                ..Default::default()
            },
        ];
        let mut buffers = Buffers {
            requests: &mut [],
            request_len: 0,
            request_required: 0,
            roots: &mut [],
            root_len: 0,
            root_required: 0,
            nodes: &mut [],
            node_len: 0,
            node_required: 0,
            text: &mut [],
            text_len: 0,
            text_required: 0,
            stack: &mut [],
            stack_required: 0,
        };
        census_requested_roots(
            &memory,
            0,
            &mut |emit| requests.iter().try_for_each(|request| emit(*request)),
            &mut buffers,
            &StackLimits::current(),
        )
        .unwrap();
        assert_eq!(memory.visits.get(), [1, 1]);
        assert_eq!(buffers.root_required, 2);
        assert_eq!(buffers.node_required, 2);
        assert_eq!(buffers.text_required, 3);
        assert_eq!(buffers.stack_required, 1);
    }

    #[test]
    fn census_stack_guard_checks_reservation_boundaries_and_margin() {
        let limits = StackLimits {
            low: 0x1000,
            high: 0x1000 + 4 * 1024 * 1024,
        };
        assert_eq!(limits.check_at(limits.low - 1), Err(STACK_HEADROOM));
        assert_eq!(
            limits.check_at(limits.low + STACK_MARGIN - 1),
            Err(STACK_HEADROOM)
        );
        assert_eq!(limits.check_at(limits.low + STACK_MARGIN), Ok(()));
        assert_eq!(limits.check_at(limits.high - 1), Ok(()));
        assert_eq!(limits.check_at(limits.high), Err(STACK_HEADROOM));
    }

    #[cfg(windows)]
    #[test]
    fn census_stack_guard_refuses_a_deep_chain_before_overflow() {
        // A smaller synthetic worker makes the resource guard deterministic
        // without exercising an actual stack overflow or a chosen depth cap.
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                struct Chain;
                const NODES: usize = 10_000;
                const START: usize = 0x1000;
                const STRIDE: usize = 0x100;
                const LINKS: usize = 0x100_0000;
                impl Memory for Chain {
                    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                        if let Some(offset) = address.checked_sub(START)
                            && offset / STRIDE < NODES
                            && output.len() == 16
                        {
                            output.fill(0);
                            match offset % STRIDE {
                                0 => return true, // Empty entry array.
                                0x18 => {
                                    if offset / STRIDE + 1 < NODES {
                                        output[..8].copy_from_slice(
                                            &((LINKS + offset / STRIDE * 8) as u64).to_le_bytes(),
                                        );
                                        output[8..12].copy_from_slice(&1_u32.to_le_bytes());
                                        output[12..].copy_from_slice(&1_u32.to_le_bytes());
                                    }
                                    return true;
                                }
                                _ => return false,
                            }
                        }
                        if let Some(offset) = address.checked_sub(LINKS)
                            && offset.is_multiple_of(8)
                            && offset / 8 + 1 < NODES
                            && output.len() == 8
                        {
                            output.copy_from_slice(
                                &((START + (offset / 8 + 1) * STRIDE - 0x10) as u64).to_le_bytes(),
                            );
                            return true;
                        }
                        false
                    }
                }
                let limits = StackLimits::current().unwrap();
                assert_eq!(limits.high - limits.low, 256 * 1024);
                let selection = Selection {
                    keys: [0; KEYS.len()],
                    positions: [0; KEYS.len()],
                    metadata: [modifier_definition::Operands::default(); KEYS.len()],
                    len: 0,
                };
                let mut buffers = Buffers {
                    requests: &mut [],
                    request_len: 0,
                    request_required: 0,
                    roots: &mut [],
                    root_len: 0,
                    root_required: 0,
                    nodes: &mut [],
                    node_len: 0,
                    node_required: 0,
                    text: &mut [],
                    text_len: 0,
                    text_required: 0,
                    stack: &mut [],
                    stack_required: 0,
                };
                limits.check_current().unwrap();
                assert_eq!(
                    walk_without_scratch(
                        &Chain,
                        START,
                        &selection,
                        false,
                        &mut buffers,
                        0,
                        None,
                        &limits
                    ),
                    Err(STACK_HEADROOM)
                );
                assert!(buffers.node_required > 1 && buffers.node_required < NODES);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn short_request_storage_counts_the_complete_use_census() {
        for capacity in [0, 1, 8] {
            let mut requests = [Request::default(); 8];
            let mut buffers = Buffers {
                requests: &mut requests[..capacity],
                request_len: 0,
                request_required: 0,
                roots: &mut [],
                root_len: 0,
                root_required: 0,
                nodes: &mut [],
                node_len: 0,
                node_required: 0,
                text: &mut [],
                text_len: 0,
                text_required: 0,
                stack: &mut [],
                stack_required: 0,
            };
            for slot in 0..16 {
                push_side(
                    &mut |request| record_request(request, &mut buffers),
                    7,
                    0,
                    slot,
                    0,
                    0,
                    0x1b2,
                    0x200,
                )
                .unwrap();
            }
            assert_eq!(buffers.request_required, 16);
            assert_eq!(buffers.request_len, capacity);
            assert_eq!(buffers.root_required, 0);
            assert_eq!(buffers.node_required, 0);
            assert_eq!(buffers.stack_required, 0);
            assert!(
                buffers.requests[..capacity]
                    .iter()
                    .enumerate()
                    .all(|(index, request)| request.usage.order == index as u32)
            );
        }
    }

    #[test]
    fn short_node_and_text_storage_still_counts_every_sibling() {
        let mut fixture = Fixture::new();
        fixture.node(0x200, 0x600, 0x1b2, 0);
        fixture.node(0x400, 0x620, 0x1b2, 7_000);
        fixture.node(0x500, 0x640, 0x1b2, 8_000);
        fixture.u64(0x200 + 0x18, 0x700);
        fixture.u64(0x200 + 0x20, 0x2_0000_0002);
        fixture.u64(0x700, 0x400 - 0x10);
        fixture.u64(0x708, 0x500 - 0x10);
        for at in [0x400, 0x500] {
            fixture.put(at + 0x48, b"law");
            fixture.u64(at + 0x48 + 16, 3);
            fixture.u64(at + 0x48 + 24, 15);
        }
        for (node_capacity, text_capacity, depth_capacity) in [(0, 0, 0), (1, 3, 1), (3, 6, 2)] {
            let mut nodes = [RawNode::default(); 3];
            let mut text = [0; 6];
            let mut stack = [Frame::default(); 2];
            let mut buffers = Buffers {
                requests: &mut [],
                request_len: 0,
                request_required: 0,
                roots: &mut [],
                root_len: 0,
                root_required: 0,
                nodes: &mut nodes[..node_capacity],
                node_len: 0,
                node_required: 0,
                text: &mut text[..text_capacity],
                text_len: 0,
                text_required: 0,
                stack: &mut stack[..depth_capacity],
                stack_required: 0,
            };
            capture_root(
                &fixture,
                0,
                0x200,
                [false, false, false, false, false, false, true, false, false],
                true,
                &mut buffers,
                &StackLimits::current(),
            )
            .unwrap();
            assert_eq!(buffers.node_required, 3);
            assert_eq!(buffers.text_required, 6);
            assert_eq!(buffers.stack_required, 2);
            assert_eq!(buffers.node_len, node_capacity);
            assert_eq!(buffers.text_len, text_capacity);
        }
    }

    #[test]
    fn ancestor_cycles_refuse_with_empty_partial_or_sufficient_depth_scratch() {
        let mut fixture = Fixture::new();
        fixture.node(0x200, 0x600, 0x1b2, 0);
        fixture.node(0x400, 0x620, 0x1b2, 0);
        fixture.u64(0x200 + 0x18, 0x700);
        fixture.u64(0x200 + 0x20, 0x1_0000_0001);
        fixture.u64(0x700, 0x400 - 0x10);
        fixture.u64(0x400 + 0x18, 0x708);
        fixture.u64(0x400 + 0x20, 0x1_0000_0001);
        fixture.u64(0x708, 0x200 - 0x10);
        for capacity in [0, 1, 2, 3] {
            let mut stack = [Frame::default(); 3];
            let mut buffers = Buffers {
                requests: &mut [],
                request_len: 0,
                request_required: 0,
                roots: &mut [],
                root_len: 0,
                root_required: 0,
                nodes: &mut [],
                node_len: 0,
                node_required: 0,
                text: &mut [],
                text_len: 0,
                text_required: 0,
                stack: &mut stack[..capacity],
                stack_required: 0,
            };
            assert_eq!(
                capture_root(
                    &fixture,
                    0,
                    0x200,
                    [true; KEYS.len()],
                    false,
                    &mut buffers,
                    &StackLimits::current()
                ),
                Err("naval modifier null node or ancestor cycle")
            );
            assert_eq!(buffers.node_required, 2);
        }
    }

    #[test]
    fn one_root_visit_keeps_both_keys_and_child_text() {
        let mut fixture = Fixture::new();
        fixture.node(0x200, 0x600, 0x1b2, 0);
        fixture.node(0x400, 0x620, 0x1b2, 7_000);
        fixture.u64(0x200 + 0x18, 0x700);
        fixture.u64(0x200 + 0x20, 0x1_0000_0001);
        fixture.u64(0x700, 0x400 - 0x10);
        fixture.u64(0x200 + 0x48 + 24, 15);
        fixture.put(0x400 + 0x48, b"admiral");
        fixture.u64(0x400 + 0x48 + 16, 7);
        fixture.u64(0x400 + 0x48 + 24, 15);
        let mut requests = [Request::default(); 2];
        let mut roots = [Root::default(); 1];
        let mut nodes = [RawNode::default(); 2];
        let mut text = [0_u8; 32];
        let mut stack = [Frame::default(); 2];
        let mut buffers = Buffers {
            requests: &mut requests,
            request_len: 0,
            request_required: 0,
            roots: &mut roots,
            root_len: 0,
            root_required: 0,
            nodes: &mut nodes,
            node_len: 0,
            node_required: 0,
            text: &mut text,
            text_len: 0,
            text_required: 0,
            stack: &mut stack,
            stack_required: 0,
        };
        push_side(
            &mut |request| record_request(request, &mut buffers),
            0x1234,
            1,
            u32::MAX,
            9,
            0,
            0x1b2,
            0x200,
        )
        .unwrap();
        push_side(
            &mut |request| record_request(request, &mut buffers),
            0x1234,
            1,
            u32::MAX,
            9,
            0,
            0x1b3,
            0x200,
        )
        .unwrap();
        capture_root(
            &fixture,
            0,
            0x200,
            [false, false, false, false, false, false, true, true, false],
            true,
            &mut buffers,
            &StackLimits::current(),
        )
        .unwrap();
        assert_eq!(buffers.request_len, 2);
        assert_eq!(buffers.node_len, 2);
        assert_eq!(buffers.nodes[1].values[6], 7_000);
        assert_eq!(&buffers.text[..buffers.text_len], b"admiral");
    }
    #[test]
    fn escape_branch_uses_native_duration_and_ship_flag() {
        let mut member = cfex_member::DetailsRow {
            member: cnaval_combatant::Row {
                state: 8,
                ..Default::default()
            },
            live_ship: 1,
            ..Default::default()
        };
        assert_eq!(keys(&member, 12, 12), &[0x5b]);
        assert_eq!(keys(&member, 13, 12), &[0x5b, 0x59]);
        member.formation_flags = 1 << 7;
        assert_eq!(keys(&member, 13, 12), &[0x5c]);
        member.live_ship = 0;
        member.live_convoy = 1;
        assert_eq!(keys(&member, 13, 12), &[0x5d]);
    }

    #[test]
    fn side_country_tree_is_unneeded_when_forward_eligible_submarine_exists() {
        let member = cfex_member::DetailsRow {
            member: cnaval_combatant::Row {
                combat: 7,
                side: 1,
                state: 2,
                ..Default::default()
            },
            member_source: 0,
            group_index: 0,
            live_ship: 1,
            formation_flags: 1 << 7,
            ..Default::default()
        };
        assert!(eligible_submarine(std::slice::from_ref(&member), 1));
        assert!(!eligible_submarine(std::slice::from_ref(&member), 0));
        assert!(!eligible_submarine(
            &[cfex_member::DetailsRow {
                member_source: 1,
                ..member
            }],
            1
        ));
        assert!(!eligible_submarine(
            &[cfex_member::DetailsRow {
                member: cnaval_combatant::Row {
                    state: 4,
                    ..member.member
                },
                ..member
            }],
            1
        ));
    }
}
