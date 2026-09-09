//! Allocation-equipment row in the old garrison-alert walk; native row class unknown.

/// Allocation-equipment row stride; old a729_garrison_alert.rs:113 and read_allocation_equipment_pool use 0x18 bytes.
pub const STRIDE: usize = 0x18;
/// Allocation-equipment archetype pointer; old read_allocation_equipment_pool reads row+0 and checks archetype id at +0x418.
pub const ARCHETYPE: usize = 0x00;
/// Allocation-equipment amount, signed i64 fixed point; old read_allocation_equipment_pool reads row+0x10.
pub const AMOUNT: usize = 0x10;
/// Allocation-equipment fixed-point denominator; old a729_garrison_alert.rs:60 records 100,000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
