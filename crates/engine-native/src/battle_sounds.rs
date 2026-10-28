//! The battle ambience's battle list, as the game holds it.
//!
//! A global pointer names the game's battle-sound manager (null while it holds
//! none). The manager keeps its battles in a spatial tree whose nodes form one
//! array: node `i`'s children are `4i+1..=4i+4`, those below the node count.
//! Each node lists battle pointers; each battle holds its centre, radius, the
//! provinces it covers and the handle of its main sound instance. The game's
//! per-frame ambience pass visits the nodes in pre-order (a node, then each of
//! its children's subtrees in order) and each node's battles in array order;
//! the walk records the battles in exactly that order. Battles change only
//! where the game adds and removes combats (the hourly update and save
//! loading), never per frame, so the list read at the hourly boundary is the
//! one the next hour's frames sound. A merge keeps the battle that is sounding
//! for this client, so the list can depend on the recording client's camera.
//!
//! The battle's own division count and per-layer sounds are recomputed every
//! frame and not read. Reads are bounded by the live node, battle and member
//! counts; no allocation or game call happens outside caller-owned storage.
use crate::{
    Memory, cpdx_array,
    cpdx_array::Array,
    cprovince, read,
    walk::{Buffer, Outcome},
};

/// Global pointer to the battle-sound manager; null while the game holds none.
pub const MANAGER_RVA: usize = 0x033d_ea38;
/// The manager's spatial tree of battles.
pub const MANAGER_TREE: usize = 0x08;
/// The tree's node array header, relative to the tree.
pub const TREE_NODES: usize = 0x10;
/// Tree node stride.
pub const NODE_STRIDE: usize = 0x30;
/// A node's array of battle pointers.
pub const NODE_BATTLES: usize = 0x18;
/// Children per tree node.
pub const NODE_CHILDREN: u32 = 4;
/// Battle centre: f32 map x, then f32 map z (the map's y).
pub const BATTLE_CENTER: usize = 0x00;
/// Battle radius, f32.
pub const BATTLE_RADIUS: usize = 0x08;
/// The battle's array of member province pointers.
pub const BATTLE_PROVINCES: usize = 0x10;
/// Handle of the battle's main sound instance; nonzero while it sounds.
pub const BATTLE_SOUND: usize = 0x28;
/// The bytes read from each battle: everything up to its sound handle's end.
const BATTLE_BYTES: usize = BATTLE_SOUND + 8;

/// One battle, in the list's order. The coordinates keep their exact f32 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Position in the list; the game keeps no battle id.
    pub ordinal: u32,
    pub center_x: u32,
    pub center_z: u32,
    pub radius: u32,
    /// The member province ids in the caller's arena, in member-array order.
    pub provinces_start: usize,
    pub provinces_len: usize,
    /// The battle's main sound instance is live for the recording client.
    pub sounding: bool,
    /// The battle object's address: the identity its sounds keep while it
    /// lives, through merges it survives and regroupings it keeps.
    pub object: u64,
}

/// Storage the whole list needs: battles and member provinces.
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
    /// Member provinces written to the arena.
    pub provinces: usize,
    pub required: Demand,
}

/// Off-thread sizing from the live counts.
pub fn demand(memory: &impl Memory, base: usize) -> Option<Demand> {
    let walk = walk(memory, base, &mut [], &mut []);
    (!matches!(walk.outcome, Outcome::Unavailable { .. })).then_some(walk.required)
}

/// The list into fixed storage. Battles past the storage are counted, not
/// read, and the walk is truncated after the last battle that fit.
pub fn walk(memory: &impl Memory, base: usize, rows: &mut [Row], provinces: &mut [u32]) -> Walk {
    walk_buffer(memory, base, Buffer::Fixed(rows), Buffer::Fixed(provinces))
}

/// The list into worker storage, grown from the acquired counts before each
/// battle is copied.
pub fn walk_growing(
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

/// The node after `node` in the pre-order of a tree of `count` nodes: its
/// first child, else the next sibling of it or of its nearest ancestor that
/// has one.
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
    /// A battle did not fit: later battles are counted, not read.
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
    let global = base
        .checked_add(MANAGER_RVA)
        .ok_or("battle sound manager address overflow")?;
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

/// A member's province id, after its province identity.
fn province_id(memory: &impl Memory, base: usize, province: usize) -> Result<u32, &'static str> {
    if province == 0 {
        return Err("battle sound member province null");
    }
    let vtable = read::<u64>(memory, province).ok_or("battle sound member province unreadable")?;
    if usize::try_from(vtable).ok() != base.checked_add(cprovince::VTABLE_RVA) {
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
