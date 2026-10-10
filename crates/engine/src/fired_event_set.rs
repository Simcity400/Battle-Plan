//! Fired CEvent-pointer hash set embedded in CGameState; exact container type unknown.
//!
//! Stable headers, distinct nodes and events, cycle detection and total counts
//! are required for a complete walk. No hourly event acquisition is enabled here.

/// Fired-event hash set count, u32.
pub const COUNT: usize = 0x00;
/// Fired-event hash bucket count, u32.
pub const BUCKET_COUNT: usize = 0x04;
/// Fired-event hash bucket pointer array.
pub const BUCKETS: usize = 0x08;
/// Fired-event bucket pointer stride.
pub const BUCKET_STRIDE: usize = 8;
/// fired-set membership.
pub const CONTAINS_RVA: usize = 0x001d_ee80;
/// hash-container operation.
pub const CONTAINER_OPERATION_RVA: usize = 0x001c_9d50;
