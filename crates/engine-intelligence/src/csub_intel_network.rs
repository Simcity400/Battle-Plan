//! Sub-intel-network elements stored inline in country intel-network
//! storage with stride 0xd8.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x298_ab98;
/// Sub-network constructor.
pub const CONSTRUCTOR_RVA: usize = 0x11b_aa40;

/// Subnetwork byte stride.
pub const STRIDE: usize = 0xd8;
/// Strength, i64, serialized and restored independently of gain modifier.
pub const STRENGTH: usize = 0x20;
/// Strength fixed-point denominator.
pub const STRENGTH_SCALE: i64 = 100_000;
