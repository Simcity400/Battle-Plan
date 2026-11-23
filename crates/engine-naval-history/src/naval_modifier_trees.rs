use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read};
use engine_native::{ccountry, cnaval_combat, cnaval_combatant};
use engine_native_parts::{modifier_accumulator, modifier_definition};
use engine_native_naval::naval_ship_modifier_inputs;

pub use engine_executable::native_values::naval_modifiers::KEYS;

pub const COUNTRY_TREE: usize = 0x5b8;
pub const COMMANDER_TREE: usize = 0x418;

pub use game_model::naval_modifier_trees::{Kind, SideSource, Owner, ship_owner, side_owner};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub accumulator: usize,
    pub receiver: usize,
    pub kind: u8,
    pub owner: u64,
    pub sub: u32,
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ordinal: u16,
    pub scope: u8,
}

fn key(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("naval tree pointer overflow")
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("naval tree pointer overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("naval tree field unreadable")
}

#[allow(
    clippy::too_many_arguments,
    reason = "the request census consumes separately owned native input slices"
)]
pub fn enumerate(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combats: &[engine_native_naval::naval_combat_details::DetailsRow],
    members: &[engine_native_naval::naval_member_details::DetailsRow],
    planes: &[engine_native_naval::naval_air_details::Row],
    sources: &[naval_ship_modifier_inputs::SourceRow],
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut cursor = 0;
    for combat in combats {
        let start = cursor;
        while members
            .get(cursor)
            .is_some_and(|member| member.member.combat == combat.combat)
        {
            cursor += 1;
        }
        let combat_members = &members[start..cursor];
        side_requests(memory, base, game_state, combat, emit)?;
        for (side, side_row) in combat.sides.iter().enumerate() {
            let [first, amount] = side_row.planes;
            let end = first
                .checked_add(amount)
                .ok_or("naval carrier plane span overflow")?;
            for plane in planes
                .get(first..end)
                .ok_or("naval carrier plane span invalid")?
            {
                if linked_air(plane)
                    && !combat_members.iter().any(|member| {
                        member.member.side == side as u8
                            && member.member.battle_slot == plane.carrier_slot as u32
                    })
                {
                    return Err("naval carrier air member has no live ship row");
                }
            }
        }
        for member in combat_members {
            if member.live_ship == 0 {
                continue;
            }
            let escaping = member.member.state == 8;
            let carrier = || -> Result<bool, &'static str> {
                let [first, amount] = combat
                    .sides
                    .get(member.member.side as usize)
                    .ok_or("naval member side invalid")?
                    .planes;
                let end = first
                    .checked_add(amount)
                    .ok_or("naval carrier plane span overflow")?;
                Ok(planes
                    .get(first..end)
                    .ok_or("naval carrier plane span invalid")?
                    .iter()
                    .any(|plane| {
                        linked_air(plane) && plane.carrier_slot as u32 == member.member.battle_slot
                    }))
            };
            if !escaping && !carrier()? {
                continue;
            }
            let context = &member.weapon_context;
            let [first, amount] = context.sources;
            let end = first
                .checked_add(amount)
                .ok_or("naval modifier source span overflow")?;
            for (ordinal, source) in sources
                .get(first..end)
                .ok_or("naval modifier source span invalid")?
                .iter()
                .enumerate()
            {
                if source.accumulator == 0 {
                    continue;
                }
                let Some((kind, owner, sub)) = ship_owner(
                    source.scope,
                    source.index,
                    source.selector,
                    source.reference,
                    member.member.ship,
                    context.task_force,
                    context.country,
                    context.commander,
                    context.ship_class,
                    context.subunit,
                ) else {
                    continue;
                };
                let receiver = match kind {
                    Kind::Country => source.accumulator.wrapping_sub(COUNTRY_TREE),
                    Kind::Commander => source.accumulator.wrapping_sub(COMMANDER_TREE),
                    _ => 0,
                };
                emit(Request {
                    accumulator: source.accumulator,
                    receiver,
                    kind: kind as u8,
                    owner,
                    sub,
                    combat: combat.combat,
                    side: member.member.side,
                    battle_slot: member.member.battle_slot,
                    ordinal: u16::try_from(ordinal).map_err(|_| "naval source ordinal overflow")?,
                    scope: source.scope,
                })?;
            }
        }
    }
    if cursor != members.len() {
        return Err("naval modifier member is outside the indexed combat census");
    }
    Ok(())
}

fn linked_air(plane: &engine_native_naval::naval_air_details::Row) -> bool {
    plane.source == 0
        && plane.group_index != u32::MAX
        && plane.variant != 0
        && plane.carrier_state & 4 == 0
        && plane.carrier_member_present != 0
        && plane.carrier_slot >= 0
}

fn side_requests(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: &engine_native_naval::naval_combat_details::DetailsRow,
    emit: &mut impl FnMut(Request) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let body = combat.native_combat;
    if field::<u64>(memory, body, 0)? != key(base, cnaval_combat::VTABLE_RVA)? as u64
        || field::<u64>(memory, body, cnaval_combat::HANDLE)? != combat.combat
    {
        return Err("naval side tree combat identity mismatch");
    }
    let mut sides = [0_usize; 2];
    for (index, offset) in [cnaval_combat::ATTACKER, cnaval_combat::DEFENDER]
        .into_iter()
        .enumerate()
    {
        let side = field::<u64>(memory, body, offset)? as usize;
        if field::<u64>(memory, side, 0)? != key(base, cnaval_combatant::VTABLE_RVA)? as u64
            || field::<u64>(memory, side, cnaval_combatant::COMBAT)? as usize != body
        {
            return Err("naval side tree side identity mismatch");
        }
        sides[index] = side;
    }
    let leader = |side: usize| -> Result<usize, &'static str> {
        let leader = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
        if leader != 0
            && field::<u64>(memory, leader, 0)? != key(base, engine_native_parts::cnavy_leader::VTABLE_RVA)? as u64
        {
            return Err("naval side tree commander identity mismatch");
        }
        Ok(leader)
    };
    let province = field::<u64>(memory, body, cnaval_combat::PROVINCE)? as usize;
    let definition = field::<u64>(memory, province, 0xc8)? as usize;
    let region: u32 = field(memory, definition, 0x58)?;
    let weather = field::<u64>(memory, game_state, engine_native::cgame_state::WEATHER_MANAGER)? as usize;
    let regions = Array::read(memory, at(weather, 0x40)?, 0x160)
        .ok_or("naval side tree weather region array invalid")?;
    let region_at = regions.element(region, 0x160);
    let terrain = engine_native::cship::modifier_sources::terrain(memory, base, definition)?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    for (index, &side) in sides.iter().enumerate() {
        if field::<i32>(memory, side, 0x48)? == 0 {
            continue;
        }
        let head = field::<u64>(memory, side, 0x38)? as usize;
        let link: u32 = field(memory, head, 0)?;
        let country = countries.resolve(link)?;
        let own = leader(side)?;
        let opposing = leader(sides[1 - index])?;
        let handle = |leader: usize| -> Result<u64, &'static str> {
            if leader == 0 {
                Ok(0)
            } else {
                field(memory, leader, engine_native::cunit_leader::TYPE)
            }
        };
        let (own_handle, opposing_handle) = (handle(own)?, handle(opposing)?);
        for source in SideSource::ALL {
            let Some((kind, owner, sub)) = side_owner(
                source,
                combat.combat,
                Some(link),
                own_handle,
                opposing_handle,
            ) else {
                continue;
            };
            let (accumulator, receiver) = match source {
                SideSource::Country => (at(country.pointer, COUNTRY_TREE)?, country.pointer),
                SideSource::Weather => match region_at {
                    Some(region_at) => (at(region_at, 0x58)?, 0),
                    None => continue,
                },
                SideSource::Terrain => (at(terrain, 0xb8)?, 0),
                SideSource::Commander => (at(own, COMMANDER_TREE)?, own),
                SideSource::OpposingCommander => (at(opposing, COMMANDER_TREE)?, opposing),
            };
            emit(Request {
                accumulator,
                receiver,
                kind: kind as u8,
                owner,
                sub,
                combat: combat.combat,
                side: index as u8,
                battle_slot: u32::MAX,
                ordinal: source as u16,
                scope: u8::MAX,
            })?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawNode<const N: usize = { KEYS.len() }> {
    pub children: u32,
    pub name: [usize; 2],
    pub values: [i64; N],
}
impl<const N: usize> Default for RawNode<N> {
    fn default() -> Self {
        Self {
            children: 0,
            name: [0; 2],
            values: [0; N],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Definitions<const N: usize = { KEYS.len() }> {
    keys: [u32; N],
    operands: [modifier_definition::Operands; N],
}

impl Definitions {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        Self::requested(memory, base, KEYS)
    }
}
impl<const N: usize> Definitions<N> {
    pub fn requested(
        memory: &impl Memory,
        base: usize,
        keys: [u32; N],
    ) -> Result<Self, &'static str> {
        if !keys.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err("naval tree requested keys repeat or are out of order");
        }
        let mut operands = [modifier_definition::Operands::default(); N];
        for (slot, &key) in operands.iter_mut().zip(&keys) {
            *slot = modifier_definition::operands(memory, base, key)?;
        }
        Ok(Self { keys, operands })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    address: usize,
    children: Array,
    next: u32,
}

pub fn capture_tree<const N: usize>(
    memory: &impl Memory,
    root: usize,
    definitions: &Definitions<N>,
    nodes: &mut Vec<RawNode<N>>,
    text: &mut Vec<u8>,
    stack: &mut Vec<Frame>,
) -> Result<(), &'static str> {
    stack.clear();
    let mut address = root;
    loop {
        if address == 0 || stack.iter().any(|frame| frame.address == address) {
            return Err("naval modifier null node or ancestor cycle");
        }
        let children = capture_node(memory, address, definitions, nodes, text)?;
        crate::allocation::reserve(stack, 1).map_err(|_| "naval tree allocation failed")?;
        stack.push(Frame {
            address,
            children,
            next: 0,
        });
        loop {
            let Some(frame) = stack.last_mut() else {
                return Ok(());
            };
            if frame.next == frame.children.count {
                stack.pop();
                continue;
            }
            let pointer = frame
                .children
                .element(frame.next, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("naval modifier child pointer unreadable or null")?;
            frame.next += 1;
            address = pointer
                .checked_add(0x10)
                .ok_or("naval modifier child address overflow")?;
            break;
        }
    }
}

pub fn capture_nonzero_tree<const N: usize>(
    memory: &impl Memory,
    root: usize,
    definitions: &Definitions<N>,
    nodes: &mut Vec<RawNode<N>>,
    text: &mut Vec<u8>,
    stack: &mut Vec<Frame>,
) -> Result<(), &'static str> {
    let mut raw = [engine_native_memory::modifier_cache::Value::default(); N];
    modifier_accumulator::lookup_many_at(memory, root, &definitions.keys, &mut raw)?;
    if raw
        .into_iter()
        .zip(definitions.operands)
        .all(|(value, operands)| !value.present || operands.effective(value.raw) == 0)
    {
        return Ok(());
    }
    capture_tree(memory, root, definitions, nodes, text, stack)
}

fn capture_node<const N: usize>(
    memory: &impl Memory,
    address: usize,
    definitions: &Definitions<N>,
    nodes: &mut Vec<RawNode<N>>,
    text: &mut Vec<u8>,
) -> Result<Array, &'static str> {
    let children = Array::read(memory, at(address, 0x18)?, 8)
        .ok_or("naval modifier children unreadable or invalid")?;
    let mut raw = [engine_native_memory::modifier_cache::Value::default(); N];
    modifier_accumulator::lookup_many_at(memory, address, &definitions.keys, &mut raw)?;
    let mut values = [0_i64; N];
    for ((value, raw), operands) in values.iter_mut().zip(raw).zip(definitions.operands) {
        if raw.present {
            *value = operands.effective(raw.raw);
        }
    }
    let name = if children.count == 0 && values.iter().any(|&value| value != 0) {
        let mut used = text.len();
        let span = msvc_string::append(memory, at(address, 0x48)?, text, &mut used)?;
        text.truncate(used);
        [span.0, span.1]
    } else {
        [text.len(), 0]
    };
    crate::allocation::reserve(nodes, 1).map_err(|_| "naval tree allocation failed")?;
    nodes.push(RawNode {
        children: children.count,
        name,
        values,
    });
    Ok(children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_executable::modifier_keys;

    #[test]
    fn zero_hover_root_reads_no_children_or_labels() {
        let bytes = vec![0_u8; 0x110];
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let definitions = Definitions {
            keys: [modifier_keys::CONVOY_ESCORT_EFFICIENCY],
            operands: [modifier_definition::Operands::default()],
        };
        let (mut nodes, mut text, mut stack) = (Vec::new(), Vec::new(), Vec::new());
        capture_nonzero_tree(
            &memory,
            0x100,
            &definitions,
            &mut nodes,
            &mut text,
            &mut stack,
        )
        .unwrap();
        assert!(nodes.is_empty() && text.is_empty() && stack.is_empty());
    }

    #[test]
    fn keys_are_sorted_and_name_each_kind_once() {
        assert!(KEYS.windows(2).all(|pair| pair[0] < pair[1]));
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, index);
            assert_eq!(Kind::from_u8(index as u8), Some(kind));
        }
        assert_eq!(Kind::from_u8(Kind::ALL.len() as u8), None);
        assert!(Kind::Country.carried() && Kind::Commander.carried());
        assert!(!Kind::CommanderClass.carried() && !Kind::SideWeather.carried());
        for (index, source) in SideSource::ALL.into_iter().enumerate() {
            assert_eq!(SideSource::from_ordinal(index as u16), Some(source));
        }
    }

    #[test]
    fn the_ship_join_names_each_tree_scope_and_skips_the_rest() {
        let owner = |scope| ship_owner(scope, 3, 0x2a, 0x55, 41, 77, 5, 99, 6, 12);
        assert_eq!(owner(0), Some((Kind::Country, 5, 0)));
        assert_eq!(owner(4), Some((Kind::Doctrine, 77, 0x2a)));
        assert_eq!(owner(5), Some((Kind::Terrain, 77, 6)));
        assert_eq!(owner(9), Some((Kind::Commander, 99, 0)));
        assert_eq!(owner(10), Some((Kind::CommanderClass, 99, 12)));
        assert_eq!(owner(11), Some((Kind::Critical, 41, 3)));
        assert_eq!(owner(15), Some((Kind::Support3, 0x55, 0)));
        assert_eq!(owner(16), None);
        assert_eq!(owner(31), None);
        assert_eq!(side_owner(SideSource::Country, 1, None, 9, 9), None);
        assert_eq!(side_owner(SideSource::Commander, 1, Some(4), 0, 9), None);
        assert_eq!(
            side_owner(SideSource::OpposingCommander, 1, Some(4), 0, 9),
            Some((Kind::Commander, 9, 0))
        );
    }

    #[test]
    fn capture_walks_preorder_and_names_only_nonzero_leaves() {
        use engine_native_memory::memory::SliceMemory;
        let mut bytes = vec![0_u8; 0x4000];
        let put = |bytes: &mut Vec<u8>, at: usize, value: &[u8]| {
            bytes[at..at + value.len()].copy_from_slice(value);
        };
        let header = |data: u64, count: u32| {
            let mut out = [0_u8; 16];
            out[..8].copy_from_slice(&data.to_le_bytes());
            out[8..12].copy_from_slice(&count.to_le_bytes());
            out[12..16].copy_from_slice(&count.to_le_bytes());
            out
        };
        put(&mut bytes, 0x100, &header(0, 0));
        put(&mut bytes, 0x118, &header(0x800, 2));
        put(&mut bytes, 0x800, &0x200_u64.to_le_bytes());
        put(&mut bytes, 0x808, &0x300_u64.to_le_bytes());
        put(&mut bytes, 0x210, &header(0x900, 1));
        put(&mut bytes, 0x228, &header(0, 0));
        put(&mut bytes, 0x900, &modifier_keys::POSITIONING.to_le_bytes());
        put(&mut bytes, 0x908, &5_000_i64.to_le_bytes());
        put(&mut bytes, 0x210 + 0x48, b"Alpha");
        put(&mut bytes, 0x210 + 0x48 + 0x10, &5_u64.to_le_bytes());
        put(&mut bytes, 0x210 + 0x48 + 0x18, &15_u64.to_le_bytes());
        put(&mut bytes, 0x310, &header(0, 0));
        put(&mut bytes, 0x328, &header(0, 0));
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let definitions = Definitions {
            keys: KEYS,
            operands: [modifier_definition::Operands::default(); KEYS.len()],
        };
        let (mut nodes, mut text, mut stack) = (Vec::new(), Vec::new(), Vec::new());
        capture_tree(
            &memory,
            0x100,
            &definitions,
            &mut nodes,
            &mut text,
            &mut stack,
        )
        .unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].children, 2);
        let positioning = KEYS
            .iter()
            .position(|&key| key == modifier_keys::POSITIONING)
            .unwrap();
        assert_eq!(nodes[1].values[positioning], 5_000);
        assert_eq!(&text[nodes[1].name[0]..][..nodes[1].name[1]], b"Alpha");
        assert_eq!(nodes[2].name[1], 0);
        let narrow = Definitions {
            keys: [modifier_keys::CONVOY_ESCORT_EFFICIENCY],
            operands: [modifier_definition::Operands::default()],
        };
        let (mut narrow_nodes, mut narrow_text) = (Vec::new(), Vec::new());
        capture_tree(
            &memory,
            0x100,
            &narrow,
            &mut narrow_nodes,
            &mut narrow_text,
            &mut stack,
        )
        .unwrap();
        assert_eq!(narrow_nodes.len(), 3);
        assert!(narrow_nodes.iter().all(|node| node.values == [0]));
        assert!(narrow_text.is_empty());
        put(&mut bytes, 0x808, &0xf0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        nodes.clear();
        text.clear();
        assert!(
            capture_tree(
                &memory,
                0x100,
                &definitions,
                &mut nodes,
                &mut text,
                &mut stack
            )
            .is_err()
        );
    }
}
