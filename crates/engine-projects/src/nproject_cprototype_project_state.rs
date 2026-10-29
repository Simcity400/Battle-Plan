//! Prototype project state with signed fixed5 progress and a signed whole-unit target.

/// Prototype-state primary vtable.
pub const VTABLE_RVA: usize = 0x02aa_7d38;
/// Signed i64 fixed5 phase progress.
pub const PHASE_PROGRESS: usize = 0x48;
/// Signed i32 whole-unit phase target, converted to fixed5 for comparison.
pub const PHASE_TARGET: usize = 0x50;
/// Prototype u32 current-state enum.
pub const STATE_ENUM: usize = 0x78;
/// Signed i32 prototype progress, clamped to 0..100 by writers.
pub const PROTOTYPE_PROGRESS: usize = 0x88;
/// Signed i32 prototype iterations, distinct from project history count.
pub const ITERATIONS: usize = 0x8c;
