//! CModifier wrapper embedded in CState and CResistance; its accumulator has a separate native layout.

/// CModifier vtable.
pub const VTABLE_RVA: usize = 0x0270_25d0;
/// CModifier embedded accumulator.
pub const ACCUMULATOR: usize = 0x10;
/// CModifier display-name std::string.
pub const DISPLAY_NAME: usize = 0x58;
