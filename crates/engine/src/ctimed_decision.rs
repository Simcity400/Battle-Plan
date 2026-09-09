//! CTimedDecision lifecycle record retained from the old decision walker.

/// CTimedDecision primary vtable; offsets.rs:288, confirmed by pinned-exe RTTI and read_timed_decision_record.
pub const VTABLE_RVA: usize = 0x027d_4fd0;
/// CTimedDecision CDecision definition pointer; offsets.rs:289, read_timed_decision_record at attach_log.rs:171521.
pub const DEFINITION: usize = 0x10;
/// CTimedDecision i32 remaining days; offsets.rs:290, read_timed_decision_record at attach_log.rs:171526.
pub const DAYS: usize = 0x18;
/// CTimedDecision i32 lifecycle code; offsets.rs:291, read_timed_decision_record at attach_log.rs:171530.
pub const STATE: usize = 0x1c;
/// CTimedDecision active lifecycle code; offsets.rs:293 and read_active_timed_decisions at attach_log.rs:171235 filter zero.
pub const STATE_ACTIVE: i32 = 0;
