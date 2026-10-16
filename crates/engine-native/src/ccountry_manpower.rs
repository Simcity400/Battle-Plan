//! Country-embedded manpower operands. Mobilization is independent of the free
//! manpower pool; these layouts do not supply a new manpower walker.

/// Residual, i32, added to the controlled-state pool sum.
pub const RESIDUAL: usize = 0xc;
/// Country-tag handle, u32.
pub const COUNTRY: usize = 8;
/// Signed i64 mobilization ratio.
pub const MOBILIZATION: usize = 0x10;
/// Daily mobilization and exile growth using the embedded object.
pub const DAILY_UPDATE_RVA: usize = 0x00ce_78c0;
/// Candidate pool getter summing signed state values and residual; exact pool/cache meaning is unknown.
pub const POOL_GETTER_RVA: usize = 0x00ce_7bd0;
/// Mobilization tooltip reading the ratio; excluded from capture.
pub const MOBILIZATION_TOOLTIP_RVA: usize = 0x00ce_83e0;
/// Alternative state-pool sum plus residual; validity and writer scope are unknown.
pub const ALTERNATE_POOL_GETTER_RVA: usize = 0x00ce_98a0;
/// Mobilization adjustment using effective modifier targets.
pub const ADJUST_MOBILIZATION_RVA: usize = 0x00ce_9930;
/// Companion adjustment using the same modifier targets.
pub const ADJUST_MOBILIZATION_TARGET_RVA: usize = 0x00ce_9cf0;
