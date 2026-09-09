//! Anonymous sprite returned by CGameBitmapFont's inline-icon lookup. Native
//! RTTI name has not been established; this does not assert CTextSprite identity.

/// Inline-icon sprite definition path, MSVC narrow string; a25_post_emit_hook.rs capture_inline_icon reads sprite+0x48.
pub const DEFINITION_PATH: usize = 0x48;
/// Inline-icon sprite texture-resource index, i32; capture_inline_icon reads sprite+0x1a0 and rejects negative values.
pub const TEXTURE_RESOURCE_INDEX: usize = 0x1a0;
