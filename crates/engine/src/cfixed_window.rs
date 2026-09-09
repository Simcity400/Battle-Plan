//! `CFixedWindow` runtime object; these offsets use its enclosing window pointer.
//!
//! Retained from the old runtime-text registration reader, not an hourly state walk.

/// CFixedWindow authored CGuiType pointer +0x90; runtime_text_inputs_live/src/a729_hook.rs:642 obtains the registered window's name through it.
pub const GUI_TYPE: usize = 0x90;
/// CFixedWindow embedded CTernary<CTextBox*> map +0x1C60; runtime_text_inputs_live/src/a729_hook.rs:641 subtracts it from the typed map pointer.
pub const TEXT_BOX_MAP: usize = 0x1c60;
/// CFixedWindow list +0x1E58; archived engine_evidence/artifacts/func_14230c610.decompile.c:397 passes longlong* this+0x3CB, so the index is multiplied by eight.
pub const DROP_DESTINATION_LIST: usize = 0x1e58;
pub const CONSTRUCT_WITH_DROP_LIST_RVA: usize = 0x0230_c610;
