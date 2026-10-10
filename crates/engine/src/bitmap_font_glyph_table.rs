//! Glyph table pointed to by a bitmap-font family; its concrete type is unknown.

/// Bucket array offset; codepoint bucket selection uses the masked scalar.
pub const BUCKETS: usize = 0;
/// Allocation size.
pub const SIZE: usize = 0xc08;
/// Bucket mask for requested and fallback Unicode scalars.
pub const CODEPOINT_BUCKET_MASK: u32 = 0x7f;
/// Bucket count before the line-height field.
pub const BUCKET_COUNT: usize = 128;
/// Parsed line height, i32.
pub const LINE_HEIGHT: usize = 0xc00;
/// Parsed baseline, i32.
pub const BASELINE: usize = 0xc04;
