//! Purchase-draft presentation window.

/// Derived setup callable.
pub const SETUP_RVA: usize = 0x0203_4060;
/// Setup entry sentinel bytes.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Overlap-widget pointer available after setup returns.
pub const OVERLAP: usize = 0x1340;
