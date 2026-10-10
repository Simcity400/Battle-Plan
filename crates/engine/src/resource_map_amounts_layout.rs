//! Resource-map output with signed i64 amounts and status bytes derived
//! from state and an optional resource scope.

/// Resource-map amount updater.
pub const UPDATE_RVA: usize = 0x018b_ad00;
/// Optional resource-scope pointer copied from argument three.
pub const RESOURCE_SCOPE: usize = 0;
/// Amount-array pointer, indexed with eight-byte resource stride.
pub const AMOUNTS: usize = 8;
/// Status-byte-array pointer, indexed by resource ordinal.
pub const STATUSES: usize = 0x20;
/// Signed i64 amount element stride.
pub const AMOUNT_STRIDE: usize = 8;
