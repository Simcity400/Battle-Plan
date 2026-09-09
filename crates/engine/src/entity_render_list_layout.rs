//! Global arrays of CPdxArray headers for partitioned entity render lists.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.
//! Header data/count offsets and the partition stride are owned by `cpdx_array`.

/// Generic Pdxmesh caster CPdxArray header array; s1_pdxmesh_caster.rs:12,179–185 indexes it by shared header stride.
pub const PDXMESH_OUTER_LISTS_RVA: usize = 0x0344_fef0;
/// Regular entity CPdxArray header array; a729_hook.rs:196 and capture_entity_render_root select this global.
pub const ENTITY_RENDER_LISTS_REGULAR_RVA: usize = 0x0344_FDD0;
/// Alternate entity CPdxArray header array; a729_hook.rs:197 and capture_entity_render_root select this global.
pub const ENTITY_RENDER_LISTS_ALTERNATE_RVA: usize = 0x0344_FE60;
