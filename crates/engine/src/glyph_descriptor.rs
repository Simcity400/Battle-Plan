//! Anonymous bitmap-font glyph descriptor; native class name is unknown.
//! The first 0x20 bytes are native metrics, not a replay wire record.

/// Glyph descriptor metric prefix size.
pub const METRIC_BYTES: usize = 0x20;
/// Glyph descriptor allocation size.
pub const SIZE: usize = 0x38;
/// Glyph descriptor texture-page index, i32.
pub const PAGE_INDEX: usize = 0;
/// Glyph descriptor atlas x coordinate, i32.
pub const ATLAS_X: usize = 4;
/// Glyph descriptor atlas y coordinate, i32.
pub const ATLAS_Y: usize = 8;
/// Glyph descriptor width, i32.
pub const WIDTH: usize = 0x0c;
/// Glyph descriptor height, i32.
pub const HEIGHT: usize = 0x10;
/// Glyph descriptor x offset, i32.
pub const X_OFFSET: usize = 0x14;
/// Glyph descriptor y offset, i32.
pub const Y_OFFSET: usize = 0x18;
/// Glyph descriptor horizontal advance, i32.
pub const X_ADVANCE: usize = 0x1c;
/// Glyph descriptor kerning-pair pointer.
pub const KERNING_PAIRS: usize = 0x20;
/// Glyph descriptor kerning-vector capacity, i32.
pub const KERNING_PAIR_CAPACITY: usize = 0x28;
/// Glyph descriptor kerning-pair count, i32.
pub const KERNING_PAIR_COUNT: usize = 0x2c;
/// Glyph descriptor kerning-vector allocator pointer.
pub const KERNING_PAIR_ALLOCATOR: usize = 0x30;
