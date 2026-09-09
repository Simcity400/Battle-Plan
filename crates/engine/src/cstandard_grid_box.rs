//! CStandardGridBox presentation geometry; no hourly walker is retained.
//!
//! Source shorthand: badge = archived `extractor/native/lanes/dynamic_collection_trace_live/src/a729_technology_badge_grid_hook.rs`;
//! observation = the same directory's `a729_controller_observation.rs`.

/// CStandardGridBox primary vtable; badge:22,585; pinned PE RTTI independently confirmed the class on 2026-09-09.
pub const VTABLE_RVA: usize = 0x02b2_f458;
/// CStandardGridBox secondary vtable; badge:23,589; pinned PE RTTI independently confirmed the class on 2026-09-09.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_f6e0;
/// CStandardGridBox secondary base offset; badge:41,589 checks the secondary vtable.
pub const SECONDARY_SUBOBJECT: usize = 0x1f8;
/// CStandardGridBox GUI pointer; badge:40,615 dereferences it before virtual geometry calls.
pub const GUI: usize = 0x140;
/// CStandardGridBox materialized row-pointer array; badge:37,600 and canvas_hook.rs:40 use the same offset.
pub const CHILDREN: usize = 0x1a0;
/// CStandardGridBox child u32 count; badge:38,593 and canvas_hook.rs:41 identify the materialized count.
pub const CHILD_COUNT: usize = 0x1ac;
/// CStandardGridBox content width i32 pixels; badge:39,619 reads the first component of the extent pair.
pub const CONTENT_WIDTH: usize = 0x168;
/// CStandardGridBox content height i32 pixels; badge:621 reads four bytes after content width.
pub const CONTENT_HEIGHT: usize = 0x16c;
/// CStandardGridBox position method; badge:21, hook_position captures geometry after its return.
pub const POSITION_RVA: usize = 0x022c_a2e0;
/// CStandardGridBox position entry bytes; badge:49-51 static sentinel checked before the old hook installed.
pub const POSITION_SENTINEL: [u8; 16] =
    *b"\x48\x83\xec\x28\x4c\x8b\x99\x40\x01\x00\x00\x44\x8b\xc2\x48\xc1";
/// CStandardGridBox append helper; observation:14 preserved the static equipment-grid callgraph address.
pub const APPEND_RVA: usize = 0x002d_e850;
/// CStandardGridBox finalization helper; observation:15 preserved the static equipment-grid callgraph address.
pub const FINALIZE_RVA: usize = 0x022b_2f10;
