//! Intelligence-agency upgrade-window entry layout.

/// CUpgradeEntry constructor RVA.
pub const CONSTRUCTOR_RVA: usize = 0x01a0_0a50;
/// CUpgradeEntry constructor bytes.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// CUpgradeEntry overlap widget pointer +0x1068.
pub const OVERLAP: usize = 0x1068;
