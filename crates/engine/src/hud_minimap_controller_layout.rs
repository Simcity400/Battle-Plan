//! Minimap setup receiver; the exact C++ class name remains unproven.

/// Minimap controller setup.
pub const SETUP_RVA: usize = 0x017b_9a80;
/// Controller-setup entry bytes.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Minimap controller GUI runtime pointer.
pub const RUNTIME_ROOT: usize = 0x1490;
