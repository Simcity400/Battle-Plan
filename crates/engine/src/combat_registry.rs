//! In-place registry at `CGameState+0x260`, shared by land and naval combats.
//!
//! The old reader calls this a manager but does not establish a distinct
//! `CCombatManager` class or RTTI name. Entries point to complete combat objects,
//! and the land reader advances each entry by `0x10` to its secondary base.

/// Registry pointer array; archived collector `attach_log/offsets.rs:753`, walked at `attach_log.rs:41172`.
pub const DATA: usize = 0x08;
/// Registry u32 entry count; archived collector `attach_log/offsets.rs:754`, walked at `attach_log.rs:41150`.
pub const COUNT: usize = 0x14;
/// Registry holds eight-byte complete-object pointers; archived `walk_land_combat_manager_entries` and manager byte copy.
pub const ENTRY_STRIDE: usize = 8;
