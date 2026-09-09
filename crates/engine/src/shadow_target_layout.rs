//! Shadow texture target reached from the map shadow owner; distinct from the clear-operation receiver.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Shader-resource-view pointer; a729_hook.rs:8677 reads it.
pub const SHADER_RESOURCE_VIEW: usize = 0x00;
/// Texture-resource pointer; a729_hook.rs:8678 reads it.
pub const RESOURCE: usize = 0x08;
/// Output-view wrapper pointer; a729_hook.rs:8679 follows it, unlike direct clear target +0x10.
pub const OUTPUT_VIEW_WRAPPER: usize = 0x10;
