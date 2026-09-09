//! CEquipmentVariant mission override stat; exact C++ row type is unproved.

/// Variant mission-stat stride; old recorder_host/native_design_catalog.rs:330 advances 0x10 bytes.
pub const STRIDE: usize = 0x10;
/// Variant mission-stat u32 key; old native_design_catalog.rs:308 and mission_override.
pub const KEY: usize = 0x00;
/// Variant mission-stat raw i64; old native_design_catalog.rs:332 reads eight bytes at +8.
pub const VALUE: usize = 0x08;
