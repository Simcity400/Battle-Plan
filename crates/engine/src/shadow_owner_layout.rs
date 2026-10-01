//! Shadow owner reached through graphical map +0x60.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Light view-projection matrix, `f32[16]`; a729_hook.rs:8750 reads it.
pub const LIGHT_VIEW_PROJECTION: usize = 0x10;
/// West wrap light view-projection matrix, `f32[16]`; a729_hook.rs:8752 reads it.
pub const WEST_LIGHT_VIEW_PROJECTION: usize = 0x50;
/// East wrap light view-projection matrix, `f32[16]`; a729_hook.rs:8755 reads it.
pub const EAST_LIGHT_VIEW_PROJECTION: usize = 0x90;
/// Shadow texture matrix, `f32[16]`; a729_hook.rs:8757 reads it.
pub const TEXTURE_MATRIX: usize = 0xd0;
/// Light depth target pointer; a729_hook.rs:8718 reads it.
pub const LIGHT_DEPTH_TARGET: usize = 0x1d0;
/// Producer/final target pointer; a729_hook.rs:8719 reads it.
pub const FINAL_TARGET: usize = 0x1d8;
/// Intermediate blur target pointer; a729_hook.rs:8720 reads it.
pub const BLUR_INTERMEDIATE_TARGET: usize = 0x1e0;
/// Blur offset base `f32[3]`; a729_hook.rs:8760 reads native bits.
pub const BLUR_OFFSET_BASE: usize = 0x200;
/// Blur weights `f32[3]`; a729_hook.rs:8761 reads native bits.
pub const BLUR_WEIGHT: usize = 0x20c;
/// Blur width f32; a729_hook.rs:8762 reads native bits.
pub const BLUR_WIDTH: usize = 0x218;
/// Blur height f32; a729_hook.rs:8763 reads native bits.
pub const BLUR_HEIGHT: usize = 0x21c;
/// Blur camera factor f32; a729_hook.rs:8759 reads native bits.
pub const BLUR_CAMERA_FACTOR: usize = 0x228;
/// Light-frustum draw on this owner; engine_evidence/artifacts/func_140b481a0.decompile.c:396 passes CGraphicalMap+0x60.
pub const DEBUG_LIGHT_FRUSTUM_DRAW_RVA: usize = 0x022c_6720;
