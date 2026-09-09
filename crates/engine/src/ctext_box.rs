//! CTextBox presentation layout; retained knowledge only, excluded from hourly capture.
//! Sources are relative to archive/pre-rewrite-2026-09-09. These reads were tied to
//! visible GUI instances and establish no campaign-state sampling contract.

/// CTextBox primary vtable; a729 RTTI and resolved_tooltip_instances_live/src/a729_hook.rs:24 agree.
pub const VTABLE_RVA: usize = 0x02b2_b418;
/// CTextBox authored GUI type pointer; runtime_text_inputs_live/src/a729_hook.rs:33,637 follows it.
pub const GUI_TYPE: usize = 0x60;
/// CTextBox CTextSprite child pointer; replay_ui_exact_evidence.json:137-145 confirms constructor and RTTI.
pub const TEXT_SPRITE: usize = 0x88;
/// CTextBox authored name MSVC string; runtime_text_inputs_live/src/a729_hook.rs:32,636 reads it.
pub const AUTHORED_NAME: usize = 0x98;
/// CTextBox packed border i32 pair; replay_interface/re_evidence/document_migration/replay_ui_exact_evidence.json:151-152 traces forwarding.
pub const BORDER: usize = 0x100;
/// CTextBox final resolved MSVC string; resolved_tooltip_instances_live/src/a729_hook.rs:33,585 reads it after layout.
pub const FINAL_TEXT: usize = 0x130;
pub const CONSTRUCTOR_RVA: usize = 0x0228_e580;
/// CTextBox destructor; runtime_text_inputs_live/src/lib.rs:7 and a729_hook.rs:43-45 verify entry bytes.
pub const DESTRUCTOR_RVA: usize = 0x0228_ec70;
/// CTextBox final text setter; resolved_tooltip_instances_live/src/a729_hook.rs:20,45-47 verifies entry bytes.
pub const FINAL_TEXT_SETTER_RVA: usize = 0x0228_ee80;
/// CTextBox extent accessor; resolved_tooltip_instances_live/src/a729_hook.rs:19,42-44 verifies entry bytes.
pub const EXTENT_RVA: usize = 0x0228_f100;
/// CTextBox destructor entry bytes; runtime_text_inputs_live/src/a729_hook.rs:43-45 checked the pinned executable.
pub const DESTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9, 0xe8,
];
/// CTextBox final text setter entry bytes; resolved_tooltip_instances_live/src/a729_hook.rs:45-47 checked a729.
pub const FINAL_TEXT_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x4c,
];
/// CTextBox extent accessor entry bytes; resolved_tooltip_instances_live/src/a729_hook.rs:42-44 checked a729.
pub const EXTENT_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x89, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xda,
];
