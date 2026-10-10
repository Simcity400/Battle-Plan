//! Entity-definition presentation layout; no live table is enabled.

/// Definition allocation size.
pub const SIZE: usize = 0x1c0;
/// Animation rate f32.
pub const ANIMATION_RATE: usize = 0x1b4;
/// Freshness field compared with the description; width is unspecified.
pub const VERSION_COMPARE_FIELD: usize = 0x1a4;
/// Update operation; copies animation rate only when the authored gate is set.
pub const UPDATE_RVA: usize = 0x0227_cef0;
/// Create/reuse operation; copies animation rate after freshness checks without that gate.
pub const CREATE_OR_REUSE_RVA: usize = 0x0227_d8c0;
