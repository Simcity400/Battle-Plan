//! Global arrays of CPdxArray headers for partitioned entity render lists.
//!
//! Presentation layout only; this module enables no recorded table or hook.
//! Header data/count offsets and the partition stride are owned by `cpdx_array`.

/// Generic Pdxmesh caster CPdxArray header array.
pub const PDXMESH_OUTER_LISTS_RVA: usize = 0x0344_fef0;
/// Regular entity CPdxArray header array.
pub const ENTITY_RENDER_LISTS_REGULAR_RVA: usize = 0x0344_FDD0;
/// Alternate entity CPdxArray header array.
pub const ENTITY_RENDER_LISTS_ALTERNATE_RVA: usize = 0x0344_FE60;
