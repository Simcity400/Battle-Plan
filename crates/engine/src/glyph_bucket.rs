//! Glyph hash buckets in a separate table owned by the font family.
//! Follow the family's table pointer before looking up a descriptor.

/// Glyph bucket stride.
pub const STRIDE: usize = 0x18;
/// Glyph bucket sorted entry pointer.
pub const ENTRIES: usize = 0;
/// Glyph bucket entry count, i32.
pub const ENTRY_COUNT: usize = 0x0c;
