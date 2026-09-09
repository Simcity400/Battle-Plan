//! CEquipmentVariant mission override group; exact C++ row type is unproved.

/// Variant mission-group stride; old recorder_host/native_design_catalog.rs:324 advances 0x28 bytes.
pub const STRIDE: usize = 0x28;
/// Variant mission-group u32 key; old native_design_catalog.rs:308 and mission_override.
pub const KEY: usize = 0x00;
/// Variant mission-group stat-vector data; old native_design_catalog.rs:329.
pub const STAT_DATA: usize = 0x10;
/// Variant mission-group stat i32 count; old native_design_catalog.rs:328 and mission_count signed check.
pub const STAT_COUNT: usize = 0x1c;
