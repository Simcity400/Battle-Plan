//! CTargetedDecision records, including those in CDecisionStatus's timed-targeted array.
//! The old normal/timed arrays assigned different names to the same lifecycle codes;
//! preserve raw codes until engine semantics are rechecked. Numeric targets were treated as state ids.

/// CTargetedDecision primary vtable; build_profile.rs:797, pinned-exe RTTI corrects the old TARGETED_TIMED_DECISION name.
pub const VTABLE_RVA: usize = 0x027d_5038;
/// CTargetedDecision CDecision definition pointer; build_profile.rs:798, read_active_targeted_decisions at attach_log.rs:171371.
pub const DEFINITION: usize = 0x10;
/// CTargetedDecision u32 country target reference; build_profile.rs:799, read_active_targeted_decisions at attach_log.rs:171405.
pub const COUNTRY_TARGET: usize = 0x20;
/// CTargetedDecision u32 numeric target; build_profile.rs:800, old reader treated it as a state id at attach_log.rs:171416.
pub const NUMERIC_TARGET: usize = 0x24;
/// CTargetedDecision i32 lifecycle code; build_profile.rs:801, read_active_targeted_decisions at attach_log.rs:171439.
pub const STATE: usize = 0x28;
/// CTargetedDecision ignore byte; build_profile.rs:802, read_active_targeted_decisions at attach_log.rs:171454 accepts zero or one.
pub const IGNORE: usize = 0x2c;
/// CTargetedDecision i32 remaining days; build_profile.rs:803, read_active_targeted_decisions at attach_log.rs:171467 preserves negative cooldown days.
pub const DAYS: usize = 0x30;
