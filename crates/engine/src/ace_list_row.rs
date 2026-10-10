//! Ace-list presentation row; its concrete type is unknown.
//! These fields select presentation widgets independently of ace gameplay state.

/// Portrait widget pointer, checked around sprite writes.
pub const PORTRAIT_WIDGET: usize = 0x48;
/// Modifier text widget pointer, checked around final text writes.
pub const MODIFIER_TEXT_WIDGET: usize = 0x60;
/// Row binding function.
pub const BIND_RVA: usize = 0x0201_edc0;
/// Row binding entry sentinel.
pub const BIND_SENTINEL: [u8; 17] = [
    0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x48, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41,
    0x54,
];
