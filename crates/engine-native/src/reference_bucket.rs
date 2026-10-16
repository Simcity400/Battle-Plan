//! Reference-registry hash bucket.

/// Bucket byte stride.
pub const STRIDE: usize = 0x18;
/// Cached u32 hash, authenticated against the type/ID hash.
pub const HASH: usize = 0x00;
/// Distance-plus-one byte.
pub const DISTANCE: usize = 0x04;
/// Type word, u32; compared after the ID.
pub const TYPE: usize = 0x08;
/// ID word, u32; compared before the type.
pub const ID: usize = 0x0c;
/// Object pointer.
pub const OBJECT: usize = 0x10;
/// Terminal distance byte.
pub const TERMINAL_DISTANCE: u8 = 0xff;
