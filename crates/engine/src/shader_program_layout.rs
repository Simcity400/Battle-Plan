//! Native shader-program row indexed by effect lookup and entity rendering.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Shader-program row stride; a729_hook.rs:10266 and :10573 agree.
pub const STRIDE: usize = 0x38;
/// Fallback shader-program object for negative material indices; a729_hook.rs:198 and :10571.
pub const ENTITY_SHADER_FALLBACK_RVA: usize = 0x035C_1970;
