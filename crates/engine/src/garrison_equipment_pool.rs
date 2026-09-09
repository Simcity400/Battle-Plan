//! Equipment-pool header used by the old garrison-alert reader.
//!
//! The native container class is unknown. This +0x08 data layout differs from
//! CEquipmentVariantPool's +0x20 data layout and must not be interchanged.
//! Required and allocation pools have distinct entry layouts.

/// Garrison equipment-pool data pointer; old read_equipment_pool_bytes reads +0x08.
pub const DATA: usize = 0x08;
/// Garrison equipment-pool capacity, i32; old read_equipment_pool_bytes reads +0x10 and compares with count.
pub const CAPACITY: usize = 0x10;
/// Garrison equipment-pool count, i32; old read_equipment_pool_bytes reads +0x14.
pub const COUNT: usize = 0x14;
/// Engine pool accumulation clamp magnitude; old garrison-alert read pools clamp each signed i64 sum to this value.
pub const AMOUNT_CLAMP: i64 = 0x53e2_d620_fb40;
