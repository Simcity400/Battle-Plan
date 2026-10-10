//! Topbar presentation-controller layout.
//! This state does not supply whole-world country operands.

/// Topbar controller setup.
pub const SETUP_RVA: usize = 0x0188_ba30;
/// Controller-setup entry bytes.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];
/// Topbar controller GUI runtime pointer.
pub const RUNTIME_ROOT: usize = 0x78;
/// Topbar controller selected country tag DB id, u32.
pub const COUNTRY_TAG_DB_ID: usize = 0x9830;
/// Topbar controller market presentation-state pointer.
pub const MARKET_STATE: usize = 0x98d8;
