//! Automatic-design callback state, separate from view and handler layouts.

/// Automatic-design operation; receives state, variant and an automatic byte, and returns success.
pub const AUTODESIGN_RVA: usize = 0x0147_ddd0;
/// Automatic-design entry bytes.
pub const AUTODESIGN_SENTINEL: [u8; 16] = [
    0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
