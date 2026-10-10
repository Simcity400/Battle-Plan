//! MSVC small UTF-32 string used by bitmap-font drawing. Only the low 32
//! length bits are described here; the full field width is unknown. Its inline
//! capacity is measured in four-byte codepoints, unlike narrow MSVC strings.

/// UTF-32 inline storage or heap pointer; capacity above three selects heap storage.
pub const STORAGE: usize = 0;
/// UTF-32 length low word read as u32; full field width is unknown.
pub const LENGTH: usize = 0x10;
/// UTF-32 capacity read as 64-bit usize.
pub const CAPACITY: usize = 0x18;
/// UTF-32 small-string inline capacity in codepoints.
pub const INLINE_CAPACITY: usize = 3;
/// UTF-32 u32 codepoint stride.
pub const CODEPOINT_STRIDE: usize = 4;
