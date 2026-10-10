//! Registration definition passed to particle-runtime allocation; presentation layouts only.

/// Registration token in a fixed 64-byte terminated string.
pub const TOKEN: usize = 0x00;
/// Definition source token in a fixed 64-byte terminated string.
pub const SOURCE_TOKEN: usize = 0x40;
/// Inline token storage width.
pub const TOKEN_CAPACITY: usize = 0x40;
/// Registration u32 hash checked against FNV-1a of its token.
pub const TOKEN_HASH: usize = 0x80;
