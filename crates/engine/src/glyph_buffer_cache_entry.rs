//! Bitmap-font glyph-buffer cache entries; no cache hooks are installed here.

/// CBitmapFont glyph-buffer cache entry stride.
pub const STRIDE: usize = 0x20;
/// Glyph cache key, u32.
pub const KEY: usize = 0;
/// Last-use timestamp, f64.
pub const LAST_USE_TIME: usize = 8;
/// Glyph cache texture resource identifier, i32.
pub const TEXTURE_RESOURCE: usize = 0x10;
/// Glyph cache vertex count, i32.
pub const VERTEX_COUNT: usize = 0x14;
/// Glyph cache GPU vertex-buffer pointer.
pub const VERTEX_BUFFER: usize = 0x18;
