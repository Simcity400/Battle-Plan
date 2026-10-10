//! Ordered operation-resource map entry.

/// Operation-resource map-row stride.
pub const STRIDE: usize = 0x18;
/// Resource-definition pointer supplying the sort key.
pub const DEFINITION: usize = 0x00;
/// Raw i64 available amount, compared directly with the requirement amount.
pub const VALUE: usize = 0x10;
