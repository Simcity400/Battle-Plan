//! CRailwayProductionLine; inherited fields live in cgeneral_production_line and cproduction_line.

use crate::{Memory, cpdx_array::Array, read};

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a1_5130;
/// Railway-construction reference type identifier.
pub const REFERENCE_TYPE_ID: u32 = 4_713;
/// Initial railway level, signed i32, serialized as a dword.
pub const BASE_LEVEL: usize = 0x70;
/// Requested railway level, signed i32, serialized as a dword.
pub const TARGET_LEVEL: usize = 0x74;
/// Cancel byte serialized as `cancel`; independently completes the line
/// without requiring route completion.
pub const COMPLETION_FLAG: usize = 0x78;
/// Route province-ID array data.
pub const ROUTE: usize = 0x80;
/// Route array capacity, u32.
pub const ROUTE_CAPACITY: usize = 0x88;
/// Route array count, u32.
pub const ROUTE_COUNT: usize = 0x8c;
/// Route entries are u32 province IDs.
pub const ROUTE_PROVINCE_STRIDE: usize = 0x04;
/// Current segment, i32, permitting -1.
pub const CURRENT_SEGMENT: usize = 0x98;
/// Remaining construction cost, signed i64 fixed5; produced amount is total
/// segment cost minus remaining cost.
pub const REMAINING_COST: usize = 0xa0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub base_level: i32,
    pub target_level: i32,
    pub completed: u8,
    pub current_segment: i32,
    pub remaining_cost: i64,
    pub route_start: usize,
    pub route_len: usize,
}

pub(crate) fn route(memory: &impl Memory, pointer: usize) -> Result<Array, &'static str> {
    pointer
        .checked_add(ROUTE)
        .and_then(|p| Array::read(memory, p, ROUTE_PROVINCE_STRIDE))
        .ok_or("railway construction route unreadable or invalid")
}

pub(crate) fn fields(
    memory: &impl Memory,
    pointer: usize,
    row: &mut Row,
) -> Result<(), &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("railway construction address overflow")
    };
    let levels =
        read::<[u8; 9]>(memory, at(BASE_LEVEL)?).ok_or("railway construction levels unreadable")?;
    row.base_level = i32::from_le_bytes(levels[..4].try_into().unwrap());
    row.target_level = i32::from_le_bytes(levels[4..8].try_into().unwrap());
    row.completed = levels[8];
    if row.completed > 1 {
        return Err("railway construction completion flag is not boolean");
    }
    row.current_segment = read::<i32>(memory, at(CURRENT_SEGMENT)?)
        .ok_or("railway construction current segment unreadable")?;
    row.remaining_cost = read::<i64>(memory, at(REMAINING_COST)?)
        .ok_or("railway construction remaining cost unreadable")?;
    Ok(())
}
