//! Anonymous sorted glyph lookup entry; native class name is unknown.

/// Glyph lookup entry stride; a25_post_emit_hook.rs lookup_exact_descriptor multiplies binary-search indices by 0x10.
pub const STRIDE: usize = 0x10;
/// Glyph lookup codepoint key, u32; lookup_exact_descriptor reads entry+0 before comparing the requested scalar.
pub const CODEPOINT: usize = 0;
/// Glyph lookup descriptor pointer; lookup_exact_descriptor reads entry+8 after an exact key match.
pub const DESCRIPTOR: usize = 8;
