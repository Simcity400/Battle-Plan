//! `NProject::CProjectStateMachine`; archived collector `read_project_state_row`.

/// NProject::CProjectStateMachine primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x02a1_9358;
/// CProjectStateMachine +0x08 current state pointer; old reader requires one of the four inline states.
pub const CURRENT_STATE: usize = 0x08;
/// CProjectStateMachine +0x10 not-completed flag u8; old reader checks 0 or 1.
pub const NOT_COMPLETED: usize = 0x10;
/// CProjectStateMachine +0x18 inline CPrototypeProjectState; vtable-checked by archived state walker.
pub const PROTOTYPE_STATE: usize = 0x18;
/// CProjectStateMachine +0xe8 first inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_1: usize = 0xe8;
/// CProjectStateMachine +0x168 second inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_2: usize = 0x168;
/// CProjectStateMachine +0x1e8 third inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_3: usize = 0x1e8;
