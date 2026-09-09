//! Wrapper returned inside opaque texture description +0x70.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Texture pointer; a729_hook.rs:8166 reads this wrapper field.
pub const TEXTURE: usize = 0x00;
/// Render-target-view pointer; a729_hook.rs:8167 reads this wrapper field.
pub const RENDER_TARGET_VIEW: usize = 0x10;
