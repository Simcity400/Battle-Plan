//! Global intel-alert state.

/// Inline global state; the address is not a pointer to dereference.
pub const GLOBAL_RVA: usize = 0x0332_4490;
/// Intel alert state's completed i32 count.
pub const COMPLETED_COUNT: usize = 0x0c;
/// Intel alert state's prepared i32 count.
pub const PREPARED_COUNT: usize = 0x24;
/// Intel alert state's available i32 count.
pub const AVAILABLE_COUNT: usize = 0x6c;
