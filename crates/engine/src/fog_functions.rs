//! Fog and gradient metadata; this module enables no game calls.

/// Intel cutoff lookup; normalizes the amount against its maximum and scans content-loaded cutoff pairs.
pub const MAP_INTEL_VALUE_RVA: usize = 0x0141_c160;
/// Province fog addition; signed i32 addition, clamped to a byte, with invalid provinces zeroed.
pub const SATURATING_ADD_PROVINCE_RVA: usize = 0x0160_6220;
/// Neighbor fog stamp; adds signed base and intel, clamps and maximum-stamps valid neighbors.
pub const STAMP_NEIGHBOURS_RVA: usize = 0x0160_6c60;
/// Country gradient shape builder for controlled provinces.
pub const BUILD_COUNTRY_GRADIENT_LAYER_RVA: usize = 0x00f1_eea0;
/// Gradient rekeying by the province's current controller.
pub const REKEY_PROVINCE_GRADIENT_RVA: usize = 0x00e0_1ff0;
