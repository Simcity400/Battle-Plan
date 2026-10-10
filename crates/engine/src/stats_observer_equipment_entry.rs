//! Variant/amount entry in the combat observer equipment-loss vector.
//! Field widths and amount scale require producer validation before a new walk.

/// Entry stride.
pub const STRIDE: usize = 0x10;
/// Equipment-variant pointer.
pub const VARIANT: usize = 0;
/// Amount read as i64; scale remains unestablished.
pub const AMOUNT: usize = 8;
