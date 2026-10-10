//! CInstantTextObject presentation layout, distinct from CTextBox and
//! CTextSprite. Their join remains unproven and GUI render acquisition is excluded.

/// CInstantTextObject primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_3518;
/// CInstantTextObject native allocation size.
pub const SIZE: usize = 0x240;
/// CInstantTextObject constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022d_7a00;
/// CInstantTextObject graphics pointer.
pub const GRAPHICS: usize = 0x170;
/// CInstantTextObject CBitmapFont pointer.
pub const FONT: usize = 0x178;
/// CInstantTextObject final MSVC string.
pub const FINAL_TEXT: usize = 0x188;
/// CInstantTextObject flags byte.
pub const FLAGS: usize = 0x238;
/// CInstantTextObject text setter.
pub const TEXT_SETTER_RVA: usize = 0x022d_7d10;
/// CInstantTextObject render vtable slot.
pub const RENDER_VSLOT: usize = 0x50;
/// Instant-text render function.
pub const RENDER_RVA: usize = 0x022d_8350;
/// Instant-text render entry bytes.
pub const RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
