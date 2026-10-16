//! Air fuel simulation globals; no enclosing type is established.

/// Air fuel i64 multiplier applied to each contributing wing.
pub const AIR_WEIGHT_RVA: usize = 0x0331_a808;
/// Air context i64 threshold used by the training gate.
pub const CONTEXT_THRESHOLD_RVA: usize = 0x0331_cfb0;
/// Ace modifier i64 count numerator, scaled by assigned-wing plane count.
pub const ACE_COUNT_RVA: usize = 0x0331_e758;
/// Ace modifier i64 cap on the count-derived multiplier.
pub const ACE_CAP_RVA: usize = 0x0331_e7e0;
/// Air context i64 subtractend.
pub const CONTEXT_SUBTRACT_RVA: usize = 0x0332_4468;
/// Air context i64 multiplier.
pub const CONTEXT_SCALE_RVA: usize = 0x0309_9db8;
/// Mission i64-cost table pointer, indexed by the selected mission bit.
pub const MISSION_COSTS_RVA: usize = 0x0332_0800;
/// Capacity i64-threshold table pointer, indexed by mission capacity class.
pub const CAPACITY_THRESHOLDS_RVA: usize = 0x0332_07b8;
/// Signed i64 table-element stride.
pub const TABLE_ELEMENT_STRIDE: usize = 8;
/// Air fuel fixed-point scale; products retain signed i64 arithmetic.
pub const FIXED_SCALE: i64 = 100_000;
/// Training mission cost index for current type zero.
pub const TRAINING_COST_INDEX: usize = 11;
