//! `NProject::CPrototypeProjectState`; archived collector `read_project_state_row`.
//!
//! The old reader took u32 values at +0x48 and +0x50. Their spacing allows
//! wider storage; native width and scaling need engine-writer confirmation
//! before a new row is defined. The offsets do not authorize narrowing.

/// NProject::CPrototypeProjectState primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x02a1_9300;
/// CPrototypeProjectState +0x48 phase progress candidate; old state walker read u32, full width unproved.
pub const PHASE_PROGRESS: usize = 0x48;
/// CPrototypeProjectState +0x50 phase target candidate; old state walker read u32, full width unproved.
pub const PHASE_TARGET: usize = 0x50;
/// CPrototypeProjectState +0x78 current-state enum read as u32 by archived `read_project_state_enum_snapshot`.
pub const STATE_ENUM: usize = 0x78;
/// CPrototypeProjectState +0x8c iterations read as u32 by archived state walker; distinct from CProject +0x74.
pub const ITERATIONS: usize = 0x8c;
