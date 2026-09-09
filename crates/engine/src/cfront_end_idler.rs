//! CFrontEndIdler fields used by the excluded camera presentation capture.
//!
//! Class/vtable association: archived `replay_ui_exact_evidence.json` controller
//! RTTI census (replay_interface/re_evidence/document_migration, lines 79–85).
//! Field reads: production_render_submit_live/src/a729_hook.rs:6516–6550,6622–6650.

/// CFrontEndIdler vtable; old camera authentication matches RVA and archived tooltip RTTI census names the class.
pub const VTABLE_RVA: usize = 0x0293_3710;
/// CFrontEndIdler embedded camera root +0x610; authenticate_camera_root requires owner+offset to equal root.
pub const CAMERA_ROOT: usize = 0x610;
/// CFrontEndIdler camera event-list pointer slot +0x5f0; old owner dispatch selects it for wheel-event capture.
pub const CAMERA_EVENT_LIST: usize = 0x5f0;
/// CFrontEndIdler action-source pointer +0x508; begin_camera_event_producer checks it is non-null.
pub const CAMERA_ACTION_SOURCE: usize = 0x508;
/// CFrontEndIdler cursor-source pointer +0x500; old wheel hook compares the argument with this pointer.
pub const CAMERA_CURSOR_SOURCE: usize = 0x500;
