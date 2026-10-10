//! Operation-token database numeric-ID map slot.

/// Operation-token database slot stride.
pub const STRIDE: usize = 0x38;
/// Slot occupancy byte: zero skips, any nonzero value is occupied.
pub const OCCUPIED: usize = 0x04;
/// Operation-token u32 numeric ID used for membership lookup.
pub const NUMERIC_ID: usize = 0x08;
/// Operation-token definition pointer.
pub const DEFINITION: usize = 0x10;
