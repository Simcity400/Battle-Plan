//! CEquipmentVariant mission override group; exact C++ row type is unproved.

/// Variant mission-group stride.
pub const STRIDE: usize = 0x28;
/// Variant mission-group u32 key.
pub const KEY: usize = 0x00;
/// Variant mission-group stat-vector data.
pub const STAT_DATA: usize = 0x10;
/// Variant mission-group stat i32 count.
pub const STAT_COUNT: usize = 0x1c;
