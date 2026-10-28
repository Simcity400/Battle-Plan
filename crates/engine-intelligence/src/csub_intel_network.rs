//! Sub-intel-network elements stored inline in country intel-network
//! storage with stride 0xd8.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x2a1_d898;
/// Sub-network constructor.
pub const CONSTRUCTOR_RVA: usize = 0x125_cf60;

/// Subnetwork byte stride.
pub const STRIDE: usize = 0xd8;
/// Gain factor, i64 fixed point; saved as `modifiers` `intel_network_gain_factor`.
pub const GAIN_FACTOR: usize = 0x08;
/// Strength, i64, serialized and restored independently of the gain factor.
pub const STRENGTH: usize = 0x20;
/// Strength target (the network strength cap), i64; saved as
/// `sub_network_strength_target`.
pub const STRENGTH_TARGET: usize = 0x40;
/// The target's part from the subnetwork's operatives, i64.
pub const TARGET_FROM_OPERATIVES: usize = 0x48;
/// The target's part from the target country's counterintelligence, i64.
pub const TARGET_FROM_COUNTERINTELLIGENCE: usize = 0x50;
/// The subnetwork's own quiet flag, one byte; saved as `sub_network_is_quiet`.
pub const QUIET: usize = 0x58;
