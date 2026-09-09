//! Resource map embedded in COperationInstance; engine template name unproved.

/// Operation resource-map data pointer at map+8; operation_is_prepared and build_profile.rs:534 read the header at instance+110.
pub const DATA: usize = 0x08;
/// Operation resource-map u32 element count at map+14; operation_is_prepared and build_profile.rs:535 read it directly.
pub const COUNT: usize = 0x14;
