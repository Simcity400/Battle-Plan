//! Ordered operation-resource map entry; engine template name unproved.

/// Operation resource-map row stride; operation_is_prepared and build_profile.rs:536 advance 0x18 bytes per entry.
pub const STRIDE: usize = 0x18;
/// Operation resource-map entry definition pointer at row+0; operation_is_prepared joins its +418 u32 sort key.
pub const DEFINITION: usize = 0x00;
/// Operation resource-map available amount as raw i64 at row+10; operation_is_prepared compares it directly to the requirement amount.
pub const VALUE: usize = 0x10;
