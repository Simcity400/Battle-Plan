//! Native CUnit combat-modifier entry.

/// CUnit combat-modifier stride; old land_combat_detail_events_live/src/passive_modifier_carrier.rs:12,315-325.
pub const STRIDE: usize = 0x18;
/// CUnit combat-modifier u32 id; old passive_modifier_carrier.rs:322.
pub const ID: usize = 0x00;
/// CUnit combat-modifier first raw i64 value; old passive_modifier_carrier.rs:13,323.
pub const VALUE_A: usize = 0x08;
/// CUnit combat-modifier second raw i64 value; old passive_modifier_carrier.rs:14,324.
pub const VALUE_B: usize = 0x10;
