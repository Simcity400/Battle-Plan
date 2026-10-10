//! Native glyph vertex in CBitmapFont's global scratch array; class name unknown.
//! Components remain IEEE-754 f32 bit patterns and a packed native color word.

/// Glyph vertex stride.
pub const STRIDE: usize = 0x1c;
/// Glyph quad vertex count.
pub const VERTICES_PER_GLYPH: usize = 6;
/// Glyph vertex x coordinate, f32.
pub const X: usize = 0;
/// Glyph vertex y coordinate, f32.
pub const Y: usize = 4;
/// Glyph vertex third position component, f32.
pub const Z: usize = 8;
/// Glyph vertex first texture coordinate, f32.
pub const U: usize = 0x0c;
/// Glyph vertex second texture coordinate, f32.
pub const V: usize = 0x10;
/// Glyph vertex packed primary color, u32.
pub const PRIMARY_COLOR: usize = 0x14;
/// Glyph vertex extra attribute, f32 with unclassified semantics.
pub const EXTRA_ATTRIBUTE: usize = 0x18;
