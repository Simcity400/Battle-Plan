//! Nested output views reached from shadow texture target +0x10.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Render-target-view pointer; a729_hook.rs:8680 reads it.
pub const RENDER_TARGET_VIEW: usize = 0x00;
/// Optional depth/stencil-view pointer; a729_hook.rs:8681 reads it.
pub const DEPTH_STENCIL_VIEW: usize = 0x08;
/// Optional depth-resource pointer; a729_hook.rs:8682 reads it.
pub const DEPTH_RESOURCE: usize = 0x10;
