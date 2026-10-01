//! Render system reached through graphical map +0x18.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Selected light direction `f32[3]`; a729_hook.rs:8749 reads it.
pub const SELECTED_LIGHT_DIRECTION: usize = 0x51c;
/// Partition-frustum draw on this owner; engine_evidence/artifacts/func_140b481a0.decompile.c:361,365,369 passes CGraphicalMap+0x18.
pub const DEBUG_FRUSTUM_DRAW_RVA: usize = 0x0222_8a70;
