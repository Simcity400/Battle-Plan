
use engine_sites::Rva;
use crate::{Memory, cfex_member, cpdx_array::Array, cship, memory::Raw, read};

pub const VTABLE_RVA: Rva = Rva(0x02a5_d3b8);
pub const COMBAT: usize = 0x18;
pub const UNITS: usize = 0x20;
pub const ENGAGED: usize = 0xda;
pub const GROUPS: usize = 0xe8;
pub const GROUPS_COUNT: usize = 0xf4;
pub const POSITIONING: usize = 0x158;
pub const ANTI_AIR: usize = 0x170;

pub const COMMANDER: usize = 0x130;
pub const SCREENING: usize = 0x148;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
}

impl Row {
    pub fn key(self) -> u128 {
        (u128::from(self.combat) << 33)
            | (u128::from(self.side) << 32)
            | u128::from(self.battle_slot)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Side {
    pub pointer: usize,
    pub scalars: Result<(i64, i64), &'static str>,
}

pub fn side(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    combat: usize,
    read_scalars: bool,
) -> Result<Side, &'static str> {
    let (mut vtable, mut parent, mut positioning, mut anti_air) = ([0; 8], [0; 8], [0; 8], [0; 8]);
    let copied = if read_scalars {
        memory.copy_scattered(
            [0, COMBAT, POSITIONING, ANTI_AIR]
                .map(|offset| pointer.checked_add(offset).unwrap_or(0)),
            &mut [&mut vtable, &mut parent, &mut positioning, &mut anti_air],
        )
    } else {
        memory.copy_scattered(
            addresses(pointer, [0, COMBAT])?,
            &mut [&mut vtable, &mut parent],
        )
    };
    if copied == 0 {
        return Err("naval field unreadable");
    }
    if u64::from_le_bytes(vtable)
        != engine_sites::at(base, VTABLE_RVA)
            .ok_or("naval side vtable overflow")? as u64
    {
        return Err("naval side identity or combat back-reference mismatch");
    }
    if copied < 2 {
        return Err("naval field unreadable");
    }
    if u64::from_le_bytes(parent) != combat as u64 {
        return Err("naval side identity or combat back-reference mismatch");
    }
    Ok(Side {
        pointer,
        scalars: if !read_scalars {
            Err("naval side scalars not requested")
        } else if copied < 4 {
            Err("naval side scalars unreadable")
        } else {
            Ok((
                i64::from_le_bytes(positioning),
                i64::from_le_bytes(anti_air),
            ))
        },
    })
}

pub fn groups(memory: &impl Memory, side: Side) -> Result<Array, &'static str> {
    Array::read(
        memory,
        side.pointer
            .checked_add(GROUPS)
            .ok_or("naval side address overflow")?,
        8,
    )
    .ok_or("naval side group array unreadable or invalid")
}

pub fn member(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    group: usize,
    combat: u64,
    side: u8,
) -> Result<Row, &'static str> {
    let (mut vtable, mut parent, mut ship) = ([0; 8], [0; 8], [0; 8]);
    let (mut slot, mut owner, mut state) = ([0; 4], [0; 4], [0; 4]);
    let copied = memory.copy_scattered(
        addresses(
            pointer,
            [
                0,
                cfex_member::GROUP,
                cfex_member::SHIP_HANDLE,
                cfex_member::BATTLE_SLOT,
                cfex_member::OWNER,
                cfex_member::STATE,
            ],
        )?,
        &mut [
            &mut vtable,
            &mut parent,
            &mut ship,
            &mut slot,
            &mut owner,
            &mut state,
        ],
    );
    if copied == 0 {
        return Err("naval field unreadable");
    }
    if u64::from_le_bytes(vtable)
        != engine_sites::at(base, cfex_member::VTABLE_RVA)
            .ok_or("naval member vtable overflow")? as u64
    {
        return Err("naval member identity or group back-reference mismatch");
    }
    if copied < 2 {
        return Err("naval field unreadable");
    }
    if u64::from_le_bytes(parent) != group as u64 {
        return Err("naval member identity or group back-reference mismatch");
    }
    if combat == 0 {
        return Err("naval member combat reference invalid");
    }
    if copied < 3 {
        return Err("naval field unreadable");
    }
    let ship = u64::from_le_bytes(ship);
    if ship != 0 && (ship as u32 != cship::HANDLE_TYPE || ship >> 32 == 0) {
        return Err("naval member ship reference invalid");
    }
    if copied < 6 {
        return Err("naval field unreadable");
    }
    Ok(Row {
        combat,
        side,
        battle_slot: u32::from_le_bytes(slot),
        ship,
        owner: u32::from_le_bytes(owner),
        state: u32::from_le_bytes(state),
    })
}

pub fn addresses<const N: usize>(
    pointer: usize,
    offsets: [usize; N],
) -> Result<[usize; N], &'static str> {
    if pointer == 0 {
        return Err("null naval object pointer");
    }
    let mut addresses = [0; N];
    for (address, offset) in addresses.iter_mut().zip(offsets) {
        *address = pointer
            .checked_add(offset)
            .ok_or("naval address overflow")?;
    }
    Ok(addresses)
}

pub fn field<T: Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    if pointer == 0 {
        return Err("null naval object pointer");
    }
    read(
        memory,
        pointer
            .checked_add(offset)
            .ok_or("naval address overflow")?,
    )
    .ok_or("naval field unreadable")
}
