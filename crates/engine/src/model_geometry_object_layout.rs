//! Unidentified model-geometry receiver used by native bounds and collision routines.
//!
//! Source: `replay_interface/native/map_viewer/src/native_model_selection.rs`.
//! Exact-build exports are in `portable_replay/native/engine_evidence/artifacts/`.
//! The original C++ class and full allocation size are unproven; the 0x78-byte
//! copied-native fixture is not sizeof evidence. This receiver is not established
//! as an alias of render_entity_layout: its transform translation at +0x40
//! conflicts with that layout's definition-pointer observation.

/// Model transform [f32; 16]; native_model_selection.rs:412 copies it to object+0x10.
/// func_1422627e0.decompile.c:62 passes object+0x10 to the matrix decomposition helper.
pub const TRANSFORM: usize = 0x10;
/// func_1422627e0.decompile.c:4,47-62 confirms the entry and this receiver's mesh/matrix inputs.
pub const COLLISION_RVA: usize = 0x0226_27e0;
/// Native world-bounds function; native_model_selection.rs:378 declares its copied-native ABI.
/// func_142261df0.decompile.c:4,23-35 confirms the entry and this receiver's static/animated bounds branches.
pub const WORLD_BOUNDS_RVA: usize = 0x0226_1df0;
