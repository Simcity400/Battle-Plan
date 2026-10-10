//! National-focus detail presentation object, excluded from hourly tables.

/// Detail-refresh entry.
pub const REFRESH_RVA: usize = 0x0137_a600;
/// Detail-refresh entry sentinel.
pub const REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48,
];
/// Detail focus-definition pointer.
pub const DEFINITION: usize = 0xf50;
