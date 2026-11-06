//! Simple project state.

/// Simple-state primary vtable.
pub const VTABLE_RVA: usize = 0x02aa_8880;
/// Signed i64 fixed5 phase progress.
pub const PHASE_PROGRESS: usize = 0x48;
/// Signed i32 whole-unit phase target.
pub const PHASE_TARGET: usize = 0x50;
/// Simple-state u32 enum, read independently for all three inline states.
pub const STATE_ENUM: usize = 0x78;
