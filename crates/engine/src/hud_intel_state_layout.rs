//! Global intel alert state; the old lane did not establish its C++ class name.

pub const GLOBAL_RVA: usize = 0x0332_4490;
/// Intel alert state's completed i32 count; old a729_hook.rs:45,336 reads +0C.
pub const COMPLETED_COUNT: usize = 0x0c;
/// Intel alert state's prepared i32 count; old a729_hook.rs:46,334 reads +24.
pub const PREPARED_COUNT: usize = 0x24;
/// Intel alert state's available i32 count; old a729_hook.rs:47,332 reads +6C.
pub const AVAILABLE_COUNT: usize = 0x6c;
