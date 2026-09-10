//! `NProject::CSimpleProjectState`; archived collector `read_project_state_row`.

/// NProject::CSimpleProjectState primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x02a1_92a8;
/// CSimpleProjectState +0x48 signed i64 fixed5 phase progress; 1A244F0 serializes the qword, 1A218A0 initializes it.
pub const PHASE_PROGRESS: usize = 0x48;
/// CSimpleProjectState +0x50 signed i32 whole-unit target; common progress reader 1A237F0 sign-extends it.
pub const PHASE_TARGET: usize = 0x50;
/// CSimpleProjectState +0x78 state enum u32; read independently for all three inline simple states.
pub const STATE_ENUM: usize = 0x78;
