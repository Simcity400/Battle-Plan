//! Sprite lookup on the graphics embedded hash table.
//! Function metadata only; no event acquisition hook is enabled.

/// Sprite lookup entry using graphics +0x100.
pub const LOOKUP_RVA: usize = 0x00a2_d0d0;
/// Exact sprite lookup entry sentinel.
pub const LOOKUP_SENTINEL: [u8; 15] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
];
