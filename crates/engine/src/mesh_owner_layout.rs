//! Mesh owner reached through render entity +0x58.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.
//! Collision fields are also witnessed by `replay_interface/native/map_viewer/src/
//! native_model_selection.rs` and exact-build exports in
//! `portable_replay/native/engine_evidence/artifacts/`.

/// Draw-group array pointer; a729_hook.rs:10517 and s1_tree_caster.rs:116 follow it.
pub const DRAW_GROUPS: usize = 0x48;
/// Draw-group count i32 in s1_tree_caster.rs:109; the main entity path selects by outer-row group.
pub const DRAW_GROUP_COUNT: usize = 0x54;
/// Draw-group header stride; a729_hook.rs:10520 and s1_tree_caster.rs:121.
pub const GROUP_STRIDE: usize = 0x18;
/// LOD threshold array pointer; s1_tree_caster.rs:87 reads it.
pub const LOD_THRESHOLDS: usize = 0x138;
/// LOD threshold count i32; s1_tree_caster.rs:82 reads it.
pub const LOD_THRESHOLD_COUNT: usize = 0x144;
/// Native f32 LOD threshold stride; s1_tree_caster.rs:96 reads each threshold.
pub const LOD_THRESHOLD_STRIDE: usize = 0x04;

/// Mesh local bounds, two xyz corners [f32; 6]; map_viewer/src/native_model_selection.rs:405 supplies this field.
/// engine_evidence/artifacts/func_142261df0.decompile.c:26-29 copies three qwords from mesh+0xa0/+0xa8/+0xb0.
pub const LOCAL_BOUNDS: usize = 0xa0;
/// Mesh collision vertex [f32; 3] array pointer; map_viewer/src/native_model_selection.rs:408 supplies it.
/// engine_evidence/artifacts/func_1422627e0.decompile.c:89,96-102 reads mesh+0x108 and uses a 0xc-byte vertex stride.
pub const COLLISION_VERTICES: usize = 0x108;
/// Mesh collision u16 index array pointer; map_viewer/src/native_model_selection.rs:409 supplies it.
/// engine_evidence/artifacts/func_1422627e0.decompile.c:90,94-100 reads mesh+0x120 through a ushort pointer.
pub const COLLISION_INDICES: usize = 0x120;
/// Mesh collision index count i32; map_viewer/src/native_model_selection.rs:410 supplies it; fixture bytes do not prove sizeof.
/// engine_evidence/artifacts/func_1422627e0.instructions.txt:149 sign-extends the dword at mesh+0x12c.
pub const COLLISION_INDEX_COUNT: usize = 0x12c;
