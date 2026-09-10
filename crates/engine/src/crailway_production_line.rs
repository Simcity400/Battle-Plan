//! CRailwayProductionLine; inherited fields live in cgeneral_production_line and cproduction_line.

use crate::{Memory, cpdx_array::Array, read};

/// CRailwayProductionLine primary vtable; construction lane lib.rs:105 and pinned executable RTTI agree.
pub const VTABLE_RVA: usize = 0x02a1_5130;
/// CRailwayProductionLine observed CReferenceObject type; construction lane LINE_TYPE_RAILWAY_CONSTRUCTION.
pub const REFERENCE_TYPE_ID: u32 = 4_713;
/// CRailwayProductionLine initial railway level i32; exact 1419F0F20 serializes signed dword, correcting the old unsigned read.
pub const BASE_LEVEL: usize = 0x70;
/// CRailwayProductionLine requested railway level i32; exact 1419F0F20 serializes signed dword, correcting the old unsigned read.
pub const TARGET_LEVEL: usize = 0x74;
/// CRailwayProductionLine completion flag u8; read_railway_construction_queue_from_manager.
pub const COMPLETION_FLAG: usize = 0x78;
/// CRailwayProductionLine province-id route array; construction lane lib.rs RAILWAY_LINE_ROUTE_PTR.
pub const ROUTE: usize = 0x80;
/// CRailwayProductionLine route array u32 capacity; construction lane lib.rs RAILWAY_LINE_ROUTE_CAP.
pub const ROUTE_CAPACITY: usize = 0x88;
/// CRailwayProductionLine route array u32 count; construction lane lib.rs RAILWAY_LINE_ROUTE_SIZE.
pub const ROUTE_COUNT: usize = 0x8c;
/// CRailwayProductionLine route entries are u32 province ids; old queue reader multiplies index by 4.
pub const ROUTE_PROVINCE_STRIDE: usize = 0x04;
/// CRailwayProductionLine current segment i32, with -1 allowed; old railway queue reader.
pub const CURRENT_SEGMENT: usize = 0x98;
/// CRailwayProductionLine remaining construction cost i64 fixed5; 1419F0B70 sums segment costs and computes produced=cost-remaining.
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
