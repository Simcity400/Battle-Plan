//! Glyph kerning pairs with unsigned codepoint keys and signed adjustments.

/// Glyph kerning-pair stride.
pub const STRIDE: usize = 8;
/// Kerning other-codepoint key, u32.
pub const CODEPOINT: usize = 0;
/// Kerning adjustment, i32.
pub const ADVANCE_ADJUSTMENT: usize = 4;
