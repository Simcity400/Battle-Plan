//! Air fuel simulation globals; no enclosing type is established.

/// Air fuel i64 multiplier applied to each contributing wing.
pub const AIR_WEIGHT_RVA: usize = 0x033e_4f80;
/// Air context i64 threshold used by the training gate.
pub const CONTEXT_THRESHOLD_RVA: usize = 0x033e_0b40;
/// Ace modifier i64 count numerator, scaled by assigned-wing plane count.
pub const ACE_COUNT_RVA: usize = 0x033e_2448;
/// Ace modifier i64 cap on the count-derived multiplier.
pub const ACE_CAP_RVA: usize = 0x033e_24c8;
/// Air context i64 subtractend.
pub const CONTEXT_SUBTRACT_RVA: usize = 0x033e_b160;
/// Air context i64 multiplier.
pub const CONTEXT_SCALE_RVA: usize = 0x0312_9ea8;
/// Mission i64-cost table pointer, indexed by the selected mission bit.
pub const MISSION_COSTS_RVA: usize = 0x033e_7130;
/// Capacity i64-threshold table pointer, indexed by mission capacity class.
pub const CAPACITY_THRESHOLDS_RVA: usize = 0x033e_70e8;
/// Air fuel fixed-point scale; products retain signed i64 arithmetic.
pub const FIXED_SCALE: i64 = 100_000;
/// Training mission cost index for current type zero.
pub const TRAINING_COST_INDEX: usize = 11;
