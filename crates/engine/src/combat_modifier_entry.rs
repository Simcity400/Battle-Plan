//! Native CUnit combat-modifier entry.

/// CUnit combat-modifier stride.
pub const STRIDE: usize = 0x18;
/// CUnit combat-modifier u32 id.
pub const ID: usize = 0x00;
/// CUnit combat-modifier first raw i64 value.
pub const VALUE_A: usize = 0x08;
/// CUnit combat-modifier second raw i64 value.
pub const VALUE_B: usize = 0x10;
