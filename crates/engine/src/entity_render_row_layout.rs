//! Outer entity render row shared in the ordinary and generic Pdxmesh caster walks.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Outer row stride; a729_hook.rs:10464 and s1_pdxmesh_caster.rs:14 agree.
pub const STRIDE: usize = 0x50;
/// Group index read as u32 by a729_hook.rs:10469 and i32 in s1_pdxmesh_caster; preserve raw 32 bits.
pub const GROUP: usize = 0x00;
/// Entity pointer; a729_hook.rs:10469 reads it.
pub const ENTITY: usize = 0x08;
/// World matrix, 16 raw f32 words; a729_hook.rs:10480 copies 64 bytes.
pub const WORLD_TRANSFORM: usize = 0x10;
