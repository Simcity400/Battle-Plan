//! Anonymous hub-motorization settings record in CCountrySupplySystem + 0x188.
//!
//! This 0x28-byte record is distinct from the companion's 0xe8-byte simulation
//! node. The old source provides no RTTI name for this inline record.

/// Hub-settings record stride; old hub_motorization_live/src/lib.rs:25 and derive_object_rows.
pub const STRIDE: usize = 0x28;
/// Hub-settings u32 province id; old hub_motorization_live/src/lib.rs:26 and read at 545-549.
pub const PROVINCE: usize = 0x00;
/// Hub-settings u32 state id; old hub_motorization_live/src/lib.rs:27 and read at 550-554.
pub const STATE: usize = 0x04;
/// Hub-settings motorization map data pointer; old hub_motorization_live/src/lib.rs:28 and read at 561-565.
pub const MOTORIZATION_DATA: usize = 0x10;
/// Hub-settings motorization map u32 count; old hub_motorization_live/src/lib.rs:29 and read at 566-570.
pub const MOTORIZATION_COUNT: usize = 0x1c;
