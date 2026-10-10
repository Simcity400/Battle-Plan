//! Native sorted pair of CEquipmentUpgrade pointer and unsigned-byte level.
//! The remaining bytes after the level are padding, not discarded precision.

/// Equipment-upgrade entry stride.
pub const STRIDE: usize = 0x10;
/// Equipment-upgrade pointer.
pub const DEFINITION: usize = 0;
/// Equipment-upgrade u8 level.
pub const LEVEL_STORAGE: usize = 0x08;
