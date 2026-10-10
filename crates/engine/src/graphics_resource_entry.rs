//! Texture-resource entries selected through the font's graphics-context manager.

/// Texture-resource entry stride.
pub const STRIDE: usize = 0x48;
/// Texture-resource native texture pointer.
pub const RESOURCE: usize = 0;
/// Texture-resource path, MSVC narrow string.
pub const PATH: usize = 8;
/// Owning reference count, i32; increased on acquisition and decreased on release.
pub const REFERENCE_COUNT: usize = 0x30;
/// Load-options block; written only for a new path registration.
pub const LOAD_OPTIONS: usize = 0x3c;
/// Texture-resource native options byte width.
pub const LOAD_OPTIONS_BYTES: usize = 0x0c;
