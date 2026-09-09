//! Anonymous glyph hash-bucket layout read by the old A25 descriptor lookup.
//! whose pointer lives at family+0; the old A25 reader missed that dereference.

/// Glyph bucket stride; a25_post_emit_hook.rs lookup_exact_descriptor multiplies the codepoint bucket by 0x18.
pub const STRIDE: usize = 0x18;
/// Glyph bucket sorted entry pointer; lookup_exact_descriptor reads the first pointer of the bucket.
pub const ENTRIES: usize = 0;
/// Glyph bucket entry count, i32; lookup_exact_descriptor reads bucket+0x0c.
pub const ENTRY_COUNT: usize = 0x0c;
