//! Mesh draw record selected through a mesh-owner draw group.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Mesh draw row stride; a729_hook.rs:10541 and s1_pdxmesh_caster.rs:15.
pub const STRIDE: usize = 0x108;
/// First draw effect key u32; s1_tree_caster.rs:132 reads it.
pub const EFFECT_KEY: usize = 0x24;
/// Draw skinning pointer; s1_pdxmesh_caster.rs walks it after selecting the draw record.
pub const SKINNING: usize = 0x40;
/// Draw variant u8; a729_hook.rs:10549 reads it.
pub const VARIANT: usize = 0x7d;
