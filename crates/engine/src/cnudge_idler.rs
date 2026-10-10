//! CNudgeIdler fields used by excluded camera presentation capture.
//! The event-list field overlaps the first camera carrier; its interpretation
//! requires further layout validation before reuse.

/// CNudgeIdler vtable.
pub const VTABLE_RVA: usize = 0x0299_3698;
/// Embedded editor camera root; authenticate owner plus displacement against root.
pub const CAMERA_ROOT: usize = 0x638;
/// Event-list candidate at +0x648 aliases render-view start at root+0x10;
/// semantics remain conflicting.
pub const CAMERA_EVENT_LIST_CANDIDATE: usize = 0x648;
/// Editor action-source pointer, required nonnull.
pub const CAMERA_ACTION_SOURCE: usize = 0x508;
/// Editor cursor-source pointer, required to match the wheel-event argument.
pub const CAMERA_CURSOR_SOURCE: usize = 0x500;
