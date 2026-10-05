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
    buffers: &mut Buffers<'_>,
    member: &cfex_member::DetailsRow,
    scope: u8,
    index: u32,
    key: u32,
    accumulator: usize,
) -> Result<(), &'static str> {
    push_side(
        buffers,
        member.member.combat,
        member.member.side,
        member.member.battle_slot,
        scope,
        index,
        key,
        accumulator,
    )
}

#[allow(clippy::too_many_arguments)] // One request: its combat, side, slot, scope, key and accumulator.
fn push_side(
    buffers: &mut Buffers<'_>,
    combat: u64,
    side: u8,
    battle_slot: u32,
    scope: u8,
    index: u32,
    key: u32,
    accumulator: usize,
) -> Result<(), &'static str> {
    let order = u32::try_from(buffers.request_required)
        .map_err(|_| "naval modifier request order overflow")?;
    let next = buffers
        .request_required
        .checked_add(1)
        .ok_or("naval modifier request length overflow")?;
    buffers.request_required = next;
    let slot = buffers
        .requests
        .get_mut(buffers.request_len)
        .ok_or(REQUEST_CAPACITY)?;
    *slot = Request {
        accumulator,
        key,
        usage: Use {
            combat,
            side,
            battle_slot,
            source_scope: scope,
            source_index: index,
            order,
        },
    };
    buffers.request_len += 1;
    Ok(())
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
    buffers: &mut Buffers<'_>,
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
                buffers,
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
                    buffers,
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
                buffers,
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
                    buffers,
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
#[allow(clippy::too_many_arguments)] // The hour's naval walk rows and the caller's arenas.
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
            buffers,
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
                                buffers,
                                member,
                                source.scope,
                                source.index,
                                key,
                                source.accumulator,
                            )?;
                        }
                    }
                } else if member.live_convoy != 0 {
                    convoy(memory, base, game_state, member, key, buffers)?;
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
        buffers.root_required = buffers.root_len + 1;
        if buffers.root_len == buffers.roots.len() {
            return Err(ROOT_CAPACITY);
        }
        let first = buffers.node_len;
        capture_root(memory, base, root, wanted, buffers)?;
        buffers.roots[buffers.root_len] = Root {
            accumulator: root,
            nodes: [first, buffers.node_len - first],
        };
        buffers.root_len += 1;
        at = end;
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
    buffers: &mut Buffers<'_>,
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
    let mut address = root;
    let mut depth = 0;
    loop {
        if address == 0 || buffers.stack[..depth].iter().any(|f| f.address == address) {
            return Err("naval modifier null node or ancestor cycle");
        }
        let children = Array::read(memory, at(address, 0x18)?, 8)
            .ok_or("naval modifier children unreadable or invalid")?;
        let mut raw = [modifier_accumulator::Value::default(); KEYS.len()];
        modifier_accumulator::lookup_many_at(
            memory,
            address,
            &keys[..key_count],
            &mut raw[..key_count],
        )?;
        let mut values = [0_i64; KEYS.len()];
        for i in 0..key_count {
            if raw[i].present {
                values[positions[i]] = metadata[i].effective(raw[i].raw);
            }
        }
        // With depth -1, 14054D0F0 prints only nonzero leaf values. An
        // interior node and an all-zero leaf never need display bytes.
        let name_len = if children.count == 0 && values.iter().any(|&value| value != 0) {
            let name_address = at(address, 0x48)?;
            let header = read::<[u8; msvc_string::STRIDE]>(memory, name_address)
                .ok_or("naval modifier display header unreadable")?;
            let name = msvc_string::Descriptor::from_header(memory, name_address, header)?;
            let name_end = buffers
                .text_len
                .checked_add(name.len)
                .ok_or("naval modifier text length overflow")?;
            buffers.text_required = buffers.text_required.max(name_end);
            if name_end > buffers.text.len() {
                return Err(TEXT_CAPACITY);
            }
            name.copy(memory, &mut buffers.text[buffers.text_len..name_end])?;
            name.len
        } else {
            0
        };
        buffers.node_required = buffers.node_len + 1;
        let node = buffers
            .nodes
            .get_mut(buffers.node_len)
            .ok_or(NODE_CAPACITY)?;
        *node = RawNode {
            children: children.count,
            name: [buffers.text_len, name_len],
            values,
        };
        buffers.node_len += 1;
        buffers.text_len += name_len;
        buffers.stack_required = buffers.stack_required.max(depth + 1);
        let frame = buffers.stack.get_mut(depth).ok_or(DEPTH_CAPACITY)?;
        *frame = Frame {
            address,
            children,
            next: 0,
        };
        depth += 1;
        loop {
            if depth == 0 {
                return Ok(());
            }
            let frame = &mut buffers.stack[depth - 1];
            if frame.next == frame.children.count {
                depth -= 1;
                continue;
            }
            let pointer = frame
                .children
                .element(frame.next, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or("naval modifier child pointer unreadable or null")?;
            frame.next += 1;
            address = pointer
                .checked_add(0x10)
                .ok_or("naval modifier child address overflow")?;
            break;
        }
    }
}

fn convoy(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    member: &cfex_member::DetailsRow,
    key: u32,
    buffers: &mut Buffers<'_>,
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
    push(buffers, member, 0, 0, key, at(country.pointer, 0x5b8)?)?;
    let combat: usize = field::<u64>(memory, side, cnaval_combatant::COMBAT)? as usize;
    let province: usize = field::<u64>(memory, combat, cnaval_combat::PROVINCE)? as usize;
    let definition: usize = field::<u64>(memory, province, 0xc8)? as usize;
    let region: u32 = field(memory, definition, 0x58)?;
    let weather: usize =
        field::<u64>(memory, game_state, crate::cgame_state::WEATHER_MANAGER)? as usize;
    let regions = Array::read(memory, at(weather, 0x40)?, 0x160)
        .ok_or("naval escape convoy weather region array invalid")?;
    if let Some(region_at) = regions.element(region, 0x160) {
        push(buffers, member, 6, 0, key, at(region_at, 0x58)?)?;
    }
    let terrain = naval_ship_modifier_inputs::terrain(memory, base, definition)?;
    push(buffers, member, 5, 0, key, at(terrain, 0xb8)?)?;
    let leader: usize = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
    if leader != 0 {
        if field::<u64>(memory, leader, 0)? != at(base, crate::cnavy_leader::VTABLE_RVA)? as u64 {
            return Err("naval escape convoy leader identity mismatch");
        }
        push(buffers, member, 9, 0, key, at(leader, 0x418)?)?;
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
        push_side(&mut buffers, 0x1234, 1, u32::MAX, 9, 0, 0x1b2, 0x200).unwrap();
        push_side(&mut buffers, 0x1234, 1, u32::MAX, 9, 0, 0x1b3, 0x200).unwrap();
        capture_root(
            &fixture,
            0,
            0x200,
            [false, false, false, false, false, false, true, true, false],
            &mut buffers,
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
