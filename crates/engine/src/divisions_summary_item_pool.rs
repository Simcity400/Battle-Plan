//! Divisions-summary presentation item pool.

/// Divisions-summary pool population function.
pub const POPULATE_RVA: usize = 0x01ce_6b40;
/// Divisions-summary pool population bytes.
pub const POPULATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// Divisions-summary pool item pointer array +0x18.
pub const ITEMS: usize = 0x18;
/// Divisions-summary pool count u32 +0x24.
pub const COUNT: usize = 0x24;
