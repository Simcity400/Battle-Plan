//! Event-option presentation and admission layout.
//! The predicate object is inline; its function slot is described separately.

/// Event option inline predicate.
pub const PREDICATE: usize = 0x198;
/// Event option admission mode, u8.
pub const ADMISSION_MODE: usize = 0x1f4;
/// Event option mode gate, u8.
pub const ADMISSION_MODE_ENABLED: usize = 0x1f5;
/// Option-admission operation.
pub const ADMISSION_RVA: usize = 0x0152_79a0;
/// Option-admission entry bytes.
pub const ADMISSION_SENTINEL: [u8; 16] = [
    0x80, 0xb9, 0xf5, 0x01, 0x00, 0x00, 0x00, 0x74, 0x0c, 0x44, 0x38, 0x81, 0xf4, 0x01, 0x00, 0x00,
];
