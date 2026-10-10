//! Intel-ledger presentation view; tab state is excluded from campaign acquisition.

/// Ledger view refresh.
pub const REFRESH_RVA: usize = 0x019c_7450;
/// Ledger-refresh entry bytes.
pub const REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48, 0x89, 0x78, 0x20, 0x55,
];

/// Ledger view selected tab id, native i32.
pub const SELECTED_TAB_ID: usize = 0x1480;
/// Four tab-record ID offsets, each signed i32, in stored order.
pub const TAB_RECORD_IDS: [usize; 4] = [0x1488, 0x14b8, 0x1518, 0x14e8];
