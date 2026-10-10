//! Mapmode setup receiver; the exact C++ class name remains unproven.

/// Mapmode controller setup.
pub const SETUP_RVA: usize = 0x0156_3c80;
/// Controller-setup entry bytes.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0x6c,
];
/// Mapmode controller GUI runtime pointer.
pub const RUNTIME_ROOT: usize = 0x2ed8;
