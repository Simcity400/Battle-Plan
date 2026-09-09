//! CNudgeIdler fields used by the excluded camera presentation capture.
//!
//! Class/vtable association: archived `replay_ui_exact_evidence.json` controller
//! RTTI census (replay_interface/re_evidence/document_migration, lines 93–99).
//! Field reads: production_render_submit_live/src/a729_hook.rs:6516–6550,6622–6650.
//! The historical event-list field overlaps the first camera carrier; its
//! interpretation requires engine rereading before any reuse.

/// CNudgeIdler vtable; archived tooltip RTTI census identifies the old camera hook's otherwise unnamed third owner.
pub const VTABLE_RVA: usize = 0x0299_3698;
/// CNudgeIdler embedded camera root +0x638; authenticate_camera_root verifies the owner-to-root address join.
pub const CAMERA_ROOT: usize = 0x638;
/// CNudgeIdler old event-list candidate +0x648; aliases root+0x10 render-view start, so semantics remain conflicting.
pub const CAMERA_EVENT_LIST_CANDIDATE: usize = 0x648;
/// CNudgeIdler action-source pointer +0x508; begin_camera_event_producer checks this old owner field is non-null.
pub const CAMERA_ACTION_SOURCE: usize = 0x508;
/// CNudgeIdler cursor-source pointer +0x500; old wheel hook compares the argument with this owner field.
pub const CAMERA_CURSOR_SOURCE: usize = 0x500;
