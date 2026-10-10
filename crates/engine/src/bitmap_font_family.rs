//! Bitmap-font family/page entry; its concrete type is unknown.
//! The glyph table is a separately allocated object reached through its pointer.

/// Family/page entry stride.
pub const STRIDE: usize = 0x50;
/// Source-name string vector pointer.
pub const SOURCE_NAMES: usize = 0x38;
/// Pending source-name count, i32.
pub const SOURCE_NAME_COUNT: usize = 0x44;
/// Glyph-table pointer, dereferenced before bucket indexing.
pub const GLYPH_TABLE: usize = 0;
/// Texture-page array pointer.
pub const PAGES: usize = 8;
/// Texture-page capacity, i32.
pub const PAGE_CAPACITY: usize = 0x10;
/// Texture-page count, i32.
pub const PAGE_COUNT: usize = 0x14;
/// Texture-page allocator pointer.
pub const PAGE_ALLOCATOR: usize = 0x18;
/// Language-name string array pointer.
pub const LANGUAGES: usize = 0x20;
/// Language-name count, i32.
pub const LANGUAGE_COUNT: usize = 0x2c;
