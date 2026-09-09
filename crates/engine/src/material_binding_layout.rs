//! Entity material binding record paired with a mesh draw.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Material binding row stride; a729_hook.rs:10544 and s1_pdxmesh_caster.rs:16.
pub const STRIDE: usize = 0x60;
/// Shader index i32 for pass n is at 0x0c + n * 4; a729_hook.rs:10555-10567.
pub const PASS_SHADER_INDICES: usize = 0x0c;
/// Native i32 shader index stride; a729_hook.rs:10559 multiplies the pass index by four.
pub const SHADER_INDEX_STRIDE: usize = 0x04;
/// Variant path shader index i32; a729_hook.rs:10565 selects this field.
pub const VARIANT_SHADER_INDEX: usize = 0x2c;
