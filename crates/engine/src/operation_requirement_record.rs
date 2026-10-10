//! Operation requirement-vector entry; amount scale is unknown.

/// Operation requirement-row stride.
pub const STRIDE: usize = 0x20;
/// Raw i64 required amount, compared directly with the resource amount.
pub const AMOUNT: usize = 0x00;
/// Requirement-definition pointer selecting its resource field.
pub const DEFINITION: usize = 0x10;
/// Optional-flag byte; a nonzero value skips the requirement.
pub const OPTIONAL: usize = 0x18;
