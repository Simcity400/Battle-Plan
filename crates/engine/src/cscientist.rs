//! `CScientist`, reached through `CCharacter +0xc0`.
//!
//! Sources: `scientists_live_lane/src/lib.rs:6-24` and archived collector
//! `read_project_scientist_row` / `read_project_scientist_skills`.

/// CScientist primary vtable; pinned executable RTTI confirms the collector and scientist-lane value.
pub const VTABLE_RVA: usize = 0x0296_7e88;
/// CScientist +0x38 trait count read as u32 by archived collector and scientist lane; no trait walk was supplied.
pub const TRAIT_COUNT: usize = 0x38;
/// CScientist +0x110 pointer to specialization skill entries; both old readers use 0x20-byte entries.
pub const SKILLS: usize = 0x110;
