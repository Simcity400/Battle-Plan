//! Anonymous bitmap-font atlas page element, separate from its family entry.

/// Atlas-page stride.
pub const STRIDE: usize = 0x0c;
/// Texture resource identifier, i32.
pub const TEXTURE_RESOURCE: usize = 0;
/// Width, i32, used to normalize glyph x extents.
pub const WIDTH: usize = 4;
/// Height, i32, used to normalize glyph y extents.
pub const HEIGHT: usize = 8;
