//! Terminal peace presentation view, excluded from hourly acquisition.

/// Terminal-summary producer entry.
pub const SUMMARY_RVA: usize = 0x00b6_ff40;
/// Terminal-summary entry sentinel.
pub const SUMMARY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];
/// View summary pointer, read after production returns.
pub const SUMMARY: usize = 0xab8;
