//! Inline key/value row in the sorted country modifier accumulator.

/// Modifier-accumulator row stride.
pub const STRIDE: usize = 0x10;
/// Modifier-entry u32 definition key.
pub const KEY: usize = 0x00;
/// Modifier-entry raw signed i64 fixed5 value.
pub const VALUE: usize = 0x08;
