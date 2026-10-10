//! Peace-beneficiary bidding presentation item.

/// Item constructor.
pub const CONSTRUCTOR_RVA: usize = 0x01e3_4b30;
/// Constructor sentinel bytes.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Overlap-widget pointer available after construction returns.
pub const OVERLAP: usize = 0x5d0;
