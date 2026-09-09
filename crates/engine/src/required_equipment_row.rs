//! Required-equipment row in the old garrison-alert walk; native row class unknown.
//!
//! Its pointer denotes an equipment type, not a CEquipmentVariant. Equal byte
//! layout with equipment_entry does not establish an identical native type.

/// Required-equipment row stride; old a729_garrison_alert.rs:112 and read_required_equipment_pool use 0x10 bytes.
pub const STRIDE: usize = 0x10;
/// Required-equipment type pointer; old read_required_equipment_pool reads row+0, later followed through archetype+0x4D8.
pub const EQUIPMENT_TYPE: usize = 0x00;
/// Required-equipment amount, signed i64 fixed point; old read_required_equipment_pool reads row+0x08.
pub const AMOUNT: usize = 0x08;
/// Required-equipment fixed-point denominator; old a729_garrison_alert.rs:60 records 100,000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
