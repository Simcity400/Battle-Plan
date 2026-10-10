//! Power-balance side definition.

/// Side-definition stride.
pub const STRIDE: usize = 0x120;
/// Side-validity byte used for trending-side selection.
pub const VALID: usize = 0x08;
/// Side identity MSVC string.
pub const IDENTITY: usize = 0x10;
/// Side fallback sprite MSVC string.
pub const GFX: usize = 0x38;
