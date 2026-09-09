//! Variant/amount entry in the observer equipment-loss map's vector.
//!
//! The engine's exact C++ pair specialization is unproved. Widths describe the
//! old byte reader and must be confirmed against the engine before a new walk.

/// Observer equipment entry stride; old land_combat_detail_events_live/x2_carriers.rs:175,5356 reads 0x10-byte entries.
pub const STRIDE: usize = 0x10;
/// Observer equipment entry CEquipmentVariant pointer; old x2_carriers.rs:5357 reads the first eight bytes.
pub const VARIANT: usize = 0;
/// Observer equipment entry amount, read i64 by old x2_carriers.rs:5358; the old field alone does not prove its scale.
pub const AMOUNT: usize = 8;
