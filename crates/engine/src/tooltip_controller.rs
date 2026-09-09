//! One receiver layout is kept here. Tooltip instance acquisition is presentation-only
//! and excluded; its vtable dispatch does not establish a simulation table.

/// Tooltip dispatch receiver's screen position i32 pair; resolved_tooltip_instances_live/src/a729_hook.rs:26,568 reads it.
pub const POSITION: usize = 0x4e0;
/// Tooltip dispatch receiver's CTextBox pointer; resolved_tooltip_instances_live/src/a729_hook.rs:25,557,607 checks exact CTextBox RTTI.
pub const TOOLTIP_OWNER: usize = 0x4e8;
/// Tooltip dispatch receiver's viewport pointer; resolved_tooltip_instances_live/src/a729_hook.rs:27,570 reads width and height through it.
pub const VIEWPORT: usize = 0x4f0;
/// Tooltip dispatch receiver's hover-context pointer; resolved_tooltip_instances_live/src/a729_hook.rs:28,620 follows it.
pub const HOVER_CONTEXT: usize = 0x4f8;
/// Tooltip controller dispatch virtual slot; replay_interface/re_evidence/document_migration/replay_ui_exact_evidence.json:73-76 confirms three implementations.
pub const PRODUCER_VSLOT: usize = 0x98;
/// Tooltip controller update entry; resolved_tooltip_instances_live/src/a729_hook.rs:18,39-41 checks the pinned entry bytes.
pub const UPDATE_RVA: usize = 0x002a_4080;
/// Tooltip controller update entry bytes; resolved_tooltip_instances_live/src/a729_hook.rs:39-41 checked a729.
pub const UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x88, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
