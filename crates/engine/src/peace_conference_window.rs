//! Peace-conference presentation window, excluded from hourly acquisition.

/// GUI refresh entry.
pub const REFRESH_RVA: usize = 0x0184_eb30;
/// GUI refresh entry sentinel.
pub const REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xf9, 0x48,
];
/// Window full conference reference, two u32 words.
pub const CONFERENCE_REFERENCE: usize = 0x5108;
