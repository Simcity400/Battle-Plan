//! MSVC small UTF-32 string used by CBitmapFont's draw path. The old reader read
//! only the low 32 bits of length; this does not establish a 32-bit native size_t.
//! This layout is separate from narrow MSVC strings because its inline capacity
//! is measured in four-byte codepoints.

/// UTF-32 inline storage or heap pointer; a25_post_emit_hook.rs read_utf32_codepoint reads the first pointer when capacity exceeds three.
pub const STORAGE: usize = 0;
/// UTF-32 length low word, read as u32 by read_utf32_codepoint at +0x10; full native field width was not proved by that reader.
pub const LENGTH: usize = 0x10;
/// UTF-32 capacity, read as 64-bit usize; read_utf32_codepoint reads object+0x18.
pub const CAPACITY: usize = 0x18;
/// UTF-32 small-string inline capacity in codepoints; read_utf32_codepoint uses inline storage when capacity is at most three.
pub const INLINE_CAPACITY: usize = 3;
/// UTF-32 codepoint stride; read_utf32_codepoint multiplies the source ordinal by four and reads u32.
pub const CODEPOINT_STRIDE: usize = 4;
