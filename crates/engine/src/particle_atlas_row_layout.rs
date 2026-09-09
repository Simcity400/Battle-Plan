//! Native per-subsystem particle atlas row.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Atlas row stride; a729_hook.rs:11287 indexes this many bytes.
pub const STRIDE: usize = 0x14;
/// Texture runtime index u32; a729_hook.rs:11295 decodes native bytes.
pub const TEXTURE_RUNTIME_INDEX: usize = 0x00;
/// Atlas half-pixel width f32; a729_hook.rs:11296 preserves raw bits.
pub const HALF_PIXEL_WIDTH: usize = 0x04;
/// Atlas half-pixel height f32; a729_hook.rs:11297 preserves raw bits.
pub const HALF_PIXEL_HEIGHT: usize = 0x08;
/// Atlas columns stored as f32; a729_hook.rs:11298 preserves native representation.
pub const COLUMNS: usize = 0x0c;
/// Atlas rows stored as f32; a729_hook.rs:11299 preserves native representation.
pub const ROWS: usize = 0x10;
