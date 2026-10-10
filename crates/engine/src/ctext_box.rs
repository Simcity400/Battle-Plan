//! Text-box presentation layout, excluded from hourly campaign capture.
//! Visible GUI instances do not establish a campaign-state sampling contract.

/// CTextBox primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_b418;
/// CTextBox authored GUI type pointer.
pub const GUI_TYPE: usize = 0x60;
/// CTextBox CTextSprite child pointer.
pub const TEXT_SPRITE: usize = 0x88;
/// CTextBox authored name MSVC string.
pub const AUTHORED_NAME: usize = 0x98;
/// CTextBox packed border i32 pair.
pub const BORDER: usize = 0x100;
/// CTextBox final resolved MSVC string.
pub const FINAL_TEXT: usize = 0x130;
/// CTextBox constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0228_e580;
/// CTextBox destructor.
pub const DESTRUCTOR_RVA: usize = 0x0228_ec70;
/// CTextBox final text setter.
pub const FINAL_TEXT_SETTER_RVA: usize = 0x0228_ee80;
/// CTextBox extent accessor.
pub const EXTENT_RVA: usize = 0x0228_f100;
/// CTextBox destructor entry bytes.
pub const DESTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9, 0xe8,
];
/// CTextBox final text setter entry bytes.
pub const FINAL_TEXT_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x4c,
];
/// CTextBox extent accessor entry bytes.
pub const EXTENT_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x89, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xda,
];
