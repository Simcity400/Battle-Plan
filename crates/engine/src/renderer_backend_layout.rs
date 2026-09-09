//! Backend reached through the backbuffer renderer at +0x268.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// D3D device pointer; a729_hook.rs:8079 reads it.
pub const DEVICE: usize = 0x20;
/// Swapchain pointer; a729_hook.rs:8078 reads it.
pub const SWAPCHAIN: usize = 0x30;
/// Backbuffer render-target-view pointer; a729_hook.rs:8080 reads it.
pub const BACKBUFFER_VIEW: usize = 0x40;
/// Backend readiness u32; a729_hook.rs:8081 requires one after recreation.
pub const READY: usize = 0x25c;
