use engine_native_memory::{
    Memory,
    cpdx_array::{self, Array},
    read,
    walk::{Buffer, Outcome},
};
use engine_native::cprovince;
use engine_sites::Rva;

pub const MANAGER_RVA: Rva = Rva(0x033d_ea38);
pub const MANAGER_TREE: usize = 0x08;
pub const TREE_NODES: usize = 0x10;
pub const NODE_STRIDE: usize = 0x30;
pub const NODE_BATTLES: usize = 0x18;
pub const NODE_CHILDREN: u32 = 4;
pub const BATTLE_CENTER: usize = 0x00;
pub const BATTLE_RADIUS: usize = 0x08;
pub const BATTLE_PROVINCES: usize = 0x10;
pub const BATTLE_SOUND: usize = 0x28;
const BATTLE_BYTES: usize = BATTLE_SOUND + 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub ordinal: u32,
    pub center_x: u32,
    pub center_z: u32,
    pub radius: u32,
    pub provinces_start: usize,
    pub provinces_len: usize,
    pub sounding: bool,
    pub object: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub battles: usize,
    pub provinces: usize,
}

impl Demand {
    pub fn raise(&mut self, demand: Self) {
        *self = self.max(demand);
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            battles: self.battles.max(other.battles),
            provinces: self.provinces.max(other.provinces),
        }
    }
}

pub struct Walk {
    pub outcome: Outcome,
    pub provinces: usize,
    pub required: Demand,
}

pub fn demand(memory: &impl Memory, base: usize) -> Option<Demand> {
    let walk = count(memory, base);
    (!matches!(walk.outcome, Outcome::Unavailable { .. })).then_some(walk.required)
}

pub fn count(memory: &impl Memory, base: usize) -> Walk {
    walk_buffer(memory, base, Buffer::Count, Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    rows: &mut Vec<Row>,
    provinces: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        Buffer::Growing(rows),
        Buffer::Growing(provinces),
    )
}

pub fn next_node(node: u32, count: u32) -> Option<u32> {
    if let Some(child) = node
        .checked_mul(NODE_CHILDREN)
        .and_then(|first| first.checked_add(1))
        .filter(|&child| child < count)
    {
        return Some(child);
    }
    let mut node = node;
    while node != 0 {
        let last_child = (node - 1) % NODE_CHILDREN == NODE_CHILDREN - 1;
        if !last_child && node + 1 < count {
            return Some(node + 1);
        }
        node = (node - 1) / NODE_CHILDREN;
    }
    None
}

struct State<'a> {
    rows: Buffer<'a, Row>,
    provinces: Buffer<'a, u32>,
    len: usize,
    used: usize,
    required: Demand,
    full: bool,
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    rows: Buffer<'_, Row>,
    provinces: Buffer<'_, u32>,
) -> Walk {
    let mut state = State {
        rows,
        provinces,
        len: 0,
        used: 0,
        required: Demand::default(),
        full: false,
    };
    let outcome =
        list(memory, base, &mut state).unwrap_or_else(|reason| Outcome::Unavailable { reason });
    Walk {
        outcome,
        provinces: state.used,
        required: state.required,
    }
}

fn list(memory: &impl Memory, base: usize, state: &mut State<'_>) -> Result<Outcome, &'static str> {
    let global =
        engine_sites::at(base, MANAGER_RVA).ok_or("battle sound manager address overflow")?;
    let manager = read::<u64>(memory, global).ok_or("battle sound manager pointer unreadable")?;
    if manager == 0 {
        return Ok(Outcome::Rows { len: 0 });
    }
    let nodes_at = usize::try_from(manager)
        .ok()
        .and_then(|manager| manager.checked_add(MANAGER_TREE + TREE_NODES))
        .ok_or("battle sound tree address overflow")?;
    let header = read::<[u8; 16]>(memory, nodes_at).ok_or("battle sound tree unreadable")?;
    let nodes =
        Array::from_header(header, NODE_STRIDE).ok_or("battle sound tree header invalid")?;
    let mut node = (nodes.count > 0).then_some(0);
    while let Some(index) = node {
        let battles = nodes
            .element(index, NODE_STRIDE)
            .and_then(|node| node.checked_add(NODE_BATTLES))
            .ok_or("battle sound tree node address overflow")?;
        let battles = Array::read(memory, battles, 8)
            .ok_or("battle sound tree node unreadable or invalid")?;
        cpdx_array::pointers(
            memory,
            battles,
            "battle sound battle pointer unreadable",
            |_, battle| visit(memory, base, battle, state),
        )?;
        node = next_node(index, nodes.count);
    }
    if read::<u64>(memory, global) != Some(manager)
        || read::<[u8; 16]>(memory, nodes_at) != Some(header)
    {
        return Err("battle sound tree changed during read");
    }
    Ok(if state.len == state.required.battles {
        Outcome::Rows { len: state.len }
    } else {
        Outcome::Truncated {
            len: state.len,
            unread_rows: u32::try_from(state.required.battles - state.len)
                .map_err(|_| "battle sound list overflow")?,
        }
    })
}

fn visit(
    memory: &impl Memory,
    base: usize,
    battle: usize,
    state: &mut State<'_>,
) -> Result<(), &'static str> {
    if battle == 0 {
        return Err("battle sound battle pointer null");
    }
    let bytes =
        read::<[u8; BATTLE_BYTES]>(memory, battle).ok_or("battle sound battle unreadable")?;
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let members = Array::from_header(
        bytes[BATTLE_PROVINCES..BATTLE_PROVINCES + 16]
            .try_into()
            .unwrap(),
        8,
    )
    .ok_or("battle sound member array invalid")?;
    let ordinal =
        u32::try_from(state.required.battles).map_err(|_| "battle sound list overflow")?;
    let count = members.count as usize;
    state.required.battles = state
        .required
        .battles
        .checked_add(1)
        .ok_or("battle sound list overflow")?;
    state.required.provinces = state
        .required
        .provinces
        .checked_add(count)
        .ok_or("battle sound member arena overflow")?;
    if state.full {
        return Ok(());
    }
    let end = state
        .used
        .checked_add(count)
        .ok_or("battle sound member arena overflow")?;
    if !state.rows.ensure(state.len + 1)? || !state.provinces.ensure(end)? {
        state.full = true;
        return Ok(());
    }
    let start = state.used;
    let mut next = start;
    let provinces = &mut state.provinces;
    cpdx_array::pointers(
        memory,
        members,
        "battle sound member pointer unreadable",
        |_, province| {
            provinces[next] = province_id(memory, base, province)?;
            next += 1;
            Ok(())
        },
    )?;
    state.rows[state.len] = Row {
        ordinal,
        center_x: word(BATTLE_CENTER),
        center_z: word(BATTLE_CENTER + 4),
        radius: word(BATTLE_RADIUS),
        provinces_start: start,
        provinces_len: count,
        sounding: bytes[BATTLE_SOUND..BATTLE_SOUND + 8] != [0_u8; 8],
        object: battle as u64,
    };
    state.len += 1;
    state.used = end;
    Ok(())
}

fn province_id(memory: &impl Memory, base: usize, province: usize) -> Result<u32, &'static str> {
    if province == 0 {
        return Err("battle sound member province null");
    }
    let vtable = read::<u64>(memory, province).ok_or("battle sound member province unreadable")?;
    if usize::try_from(vtable).ok() != engine_sites::at(base, cprovince::VTABLE_RVA) {
        return Err("battle sound member is not a province");
    }
    let id = province
        .checked_add(cprovince::ID)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("battle sound member province id unreadable")?;
    u32::try_from(id)
        .ok()
        .filter(|&id| id > 0)
        .ok_or("battle sound member province id not positive")
}

#[cfg(test)]
mod tests;
