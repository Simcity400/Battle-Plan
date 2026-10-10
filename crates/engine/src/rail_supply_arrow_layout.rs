//! Selected rail-supply arrow layout.

/// Arrow selector using a separate pool receiver.
pub const SLOT_SELECTOR_RVA: usize = 0x0164_ccb0;
/// Exact selector-entry instruction sentinel.
pub const SLOT_SELECTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Selected arrow vertex-buffer wrapper pointer.
pub const WRAPPER: usize = 0x48;
/// Selected arrow texture slot, i32.
pub const TEXTURE_SLOT: usize = 0x204;
