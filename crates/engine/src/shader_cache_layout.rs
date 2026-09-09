//! Engine shader cache passed to FUN1423f4ec0.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Inline shader-program array; a729_hook.rs:10267 adds the indexed program offset.
pub const PROGRAMS: usize = 0x20;
/// Shader/effect lookup FUN1423f4ec0; lib.rs:83 and hook_engine_shader_effect_lookup identify the cache argument.
pub const ENGINE_SHADER_EFFECT_LOOKUP_RVA: u32 = 0x023F_4EC0;
/// Shader/effect lookup entry bytes checked by a729_hook.rs:285.
pub const ENGINE_SHADER_EFFECT_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55,
];
