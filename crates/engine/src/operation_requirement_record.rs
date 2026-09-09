//! Operation requirement-vector entry; engine class name and amount scale unproved.

/// Operation requirement row stride; operation_is_prepared and build_profile.rs:541 advance 0x20 bytes per entry.
pub const STRIDE: usize = 0x20;
/// Operation requirement amount as raw i64 at row+0; operation_is_prepared compares it directly to the resource-map amount.
pub const AMOUNT: usize = 0x00;
/// Operation requirement definition pointer at row+10; operation_is_prepared then reads definition+4D8, not row+4D8.
pub const DEFINITION: usize = 0x10;
/// Operation requirement optional flag byte at row+18; operation_is_prepared skips entries whose byte is nonzero.
pub const OPTIONAL: usize = 0x18;
