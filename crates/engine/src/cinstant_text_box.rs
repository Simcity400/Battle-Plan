//! CInstantTextBox is distinct from CTextBox and CTextSprite.
//! Its constructor stores this receiver in the embedded observer at +0x148.
//! A join through another text-box owner and child remains unproven;
//! no producer is enabled here.

/// CInstantTextBox primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_fb08;
/// CInstantTextBox constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022b_50a0;
/// Instant-text layout pointer to a 0x240-byte object.
pub const TEXT_LAYOUT: usize = 0xc8;
/// CInstantTextBox MSVC text string.
pub const TEXT: usize = 0x110;
/// CInstantTextBox embedded CScrollbarObserverGlue<CInstantTextBox>.
pub const SCROLLBAR_OBSERVER: usize = 0x140;
/// CInstantTextBox text setter.
pub const TEXT_SETTER_RVA: usize = 0x022b_59e0;
/// CInstantTextBox scrollbar observer callback.
pub const SCROLLBAR_CALLBACK_RVA: usize = 0x022b_5e70;
/// CInstantTextBox nonempty-label virtual slot.
pub const NONEMPTY_LABEL_VSLOT: usize = 0x78;
/// CInstantTextBox empty-label virtual slot.
pub const EMPTY_LABEL_VSLOT: usize = 0x80;
/// CInstantTextBox text-setter entry bytes.
pub const TEXT_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57,
];
