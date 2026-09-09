//! CNationalFocusProgress state retained from the old focus walker.
//! The old progress reader narrowed a native i64 field; retain its complete engine value.

/// CNationalFocusProgress primary vtable; build_profile.rs:804, confirmed by pinned-exe RTTI (old name FOCUS_STATUS).
pub const VTABLE_RVA: usize = 0x0272_3d30;
/// CNationalFocusProgress current CNationalFocus pointer; offsets.rs:299, dereferenced by read_country_focus at attach_log.rs:167415.
pub const CURRENT: usize = 0x10;
/// CNationalFocusProgress current CContinuousNationalFocus pointer; offsets.rs:300, read_country_focus at attach_log.rs:167417.
pub const CONTINUOUS: usize = 0x18;
/// CNationalFocusProgress i64 fixed-point progress; FUN141bb5df0 reads qword +0x38 at 141bb5eb1/141bb6057; old offsets.rs:301 reader narrowed to i32.
pub const PROGRESS: usize = 0x38;
/// CNationalFocusProgress completed-focus pointer-array data; offsets.rs:302, read_focus_completed_vector at attach_log.rs:167469.
pub const COMPLETED_DATA: usize = 0x40;
/// CNationalFocusProgress completed-focus u32 capacity; offsets.rs:303, read_focus_completed_vector at attach_log.rs:167456.
pub const COMPLETED_CAPACITY: usize = 0x48;
/// CNationalFocusProgress completed-focus u32 count; offsets.rs:304, read_country_focus at attach_log.rs:167403.
pub const COMPLETED_COUNT: usize = 0x4c;
/// CNationalFocusProgress completed-focus pointer stride; read_focus_completed_vector at attach_log.rs:167476 reads eight-byte pointers.
pub const COMPLETED_STRIDE: usize = 8;
pub const HAS_COMPLETED_RVA: usize = 0x002d_6780;
/// CNationalFocusProgress paused byte; build_profile.rs:806, read_country_focus at attach_log.rs:167444 accepts zero or one.
pub const PAUSED: usize = 0xb0;

/// CNationalFocusProgress completion entry FUN1402cef60; direct_frd_transitions_live/lib.rs:55 and a729_hook.rs:1154, retained without an event hook.
pub const COMPLETED_RVA: usize = 0x002c_ef60;
/// FUN1402cef60 entry prefix; direct_frd_transitions_live/a729_hook.rs:54-56 bound and checked these bytes before its old hook.
pub const COMPLETED_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];
