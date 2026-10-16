//! Anonymous per-actor entry in a supply-node settings record's map.

/// Hub motorization entry stride.
pub const STRIDE: usize = 0x08;
/// Hub settings raw CCountryTag link u32.
pub const ACTOR_COUNTRY: usize = 0x00;
/// Hub motorization level u8.
pub const LEVEL: usize = 0x04;
/// Hub settings lookup resolves raw country links.
pub const LOOKUP_ACTOR_RVA: usize = 0x0109_84d0;
