//! String/hash lookup key shared by definition databases.

/// Definition-key record stride.
pub const STRIDE: usize = 0x28;
/// Stored key string at record +0.
pub const TEXT: usize = 0;
/// Case-folded FNV-1a hash, u32 at +0x20.
pub const HASH: usize = 0x20;
