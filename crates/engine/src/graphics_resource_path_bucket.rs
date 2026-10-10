//! Anonymous bucket in the texture manager's byte-exact path index.

/// Texture path-index bucket stride.
pub const STRIDE: usize = 0x30;
/// Texture path-index hash, u32.
pub const HASH: usize = 0;
/// Texture path-index occupancy/probe distance, u8.
pub const PROBE_DISTANCE: usize = 4;
/// Texture path-index key, MSVC narrow string.
pub const PATH: usize = 8;
/// Texture path-index resource index, u32.
pub const RESOURCE_INDEX: usize = 0x28;
