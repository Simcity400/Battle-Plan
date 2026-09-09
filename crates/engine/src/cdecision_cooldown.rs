//! CDecisionCooldown state in CDecisionStatus's re-enable and removal arrays.

/// CDecisionCooldown primary vtable; build_profile.rs:810, confirmed by pinned-exe RTTI and read_decision_cooldowns.
pub const VTABLE_RVA: usize = 0x027d_4f80;
/// CDecisionCooldown CDecision pointer; build_profile.rs:811, read_decision_cooldowns at attach_log.rs:171268.
pub const DEFINITION: usize = 0x08;
/// CDecisionCooldown i32 remaining days; build_profile.rs:812, read_decision_cooldowns at attach_log.rs:171275 preserves signed values.
pub const DAYS: usize = 0x10;
