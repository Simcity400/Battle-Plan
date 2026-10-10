//! Text-box lookup map embedded in a fixed window.
//! This module describes its layout without installing a registration hook.

/// Text-box map vtable.
pub const VTABLE_RVA: usize = 0x02b3_8350;
/// Textbox-map registration function.
pub const REGISTER_RVA: usize = 0x0224_5570;
/// Text-box registration entry bytes.
pub const REGISTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x80, 0x79, 0x10, 0x01, 0x49, 0x8b,
];
