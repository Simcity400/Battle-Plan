//! Draw context used by the final D3D submission functions, distinct from the map-scene camera pointer.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.
//!
//! Fixed-state/sampler receiver identity: archived `func_141268c60.decompile.c:121-129`
//! passes the same `param_2` to the raster, blend, texture/sampler and shader-program
//! binders. `func_1423ef860.decompile.c:232,251,259,266` installs FUN142400520,
//! FUN142400090, FUN142400330 and FUN1424005a0 in those dispatch slots.

/// Fallback index count i32; a729_hook.rs:8496 reads it for indexed instanced draws.
pub const FALLBACK_INDEX_COUNT: usize = 0x04;
/// Fallback vertex count i32; a729_hook.rs:7510 reads it for nonindexed draws.
pub const FALLBACK_VERTEX_COUNT: usize = 0x08;
/// D3D device-context pointer; a729_hook.rs:12711 reads it for state snapshots.
pub const D3D_DEVICE_CONTEXT: usize = 0x2a0;
/// Nonindexed final draw FUN1423fe970; lib.rs:74 and hook_particle_final_draw retain the context ABI.
pub const PARTICLE_FINAL_DRAW_RVA: u32 = 0x023F_E970;
/// Nonindexed final draw entry bytes checked by a729_hook.rs:276.
pub const PARTICLE_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
/// Indexed final draw FUN1423fe9b0; lib.rs:78 and hook_entity_final_draw retain the context ABI.
pub const ENTITY_FINAL_DRAW_RVA: u32 = 0x023F_E9B0;
/// Indexed final draw entry bytes checked by a729_hook.rs:279.
pub const ENTITY_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf9, 0x8b, 0xda, 0x85,
];
/// Indexed instanced final draw FUN1423fe9f0; lib.rs:79 identifies the context operation.
pub const INDEXED_INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_E9F0;
/// Nonindexed instanced final draw FUN1423fea40; lib.rs:80 identifies the context operation.
pub const INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_EA40;
/// Nonindexed instanced final draw entry bytes checked by a729_hook.rs:327.
pub const INSTANCED_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
/// Shader-program binder FUN142400520; lib.rs:82 and hook_shader_program_binder retain the context ABI.
pub const SHADER_PROGRAM_BINDER_RVA: u32 = 0x0240_0520;
/// Shader binder entry bytes checked by a729_hook.rs:282.
pub const SHADER_PROGRAM_BINDER_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xd2, 0x74, 0x79, 0x56, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0x98, 0x02, 0x00, 0x00,
];

/// Cached pixel-stage texture wrappers; FUN1424005a0 tests stage bit one and reads context+0x20+slot*8.
pub const PIXEL_TEXTURES: usize = 0x20;
/// Cached pixel-stage sampler wrappers; FUN1424005a0 tests stage bit one and reads context+0xa0+slot*8.
pub const PIXEL_SAMPLERS: usize = 0xa0;
/// Cached vertex-stage texture wrappers; FUN1424005a0 tests stage bit two and reads context+0x120+slot*8.
pub const VERTEX_TEXTURES: usize = 0x120;
/// Cached vertex-stage sampler wrappers; FUN1424005a0 tests stage bit two and reads context+0x1a0+slot*8.
pub const VERTEX_SAMPLERS: usize = 0x1a0;
/// Cached pointer entry stride; archived FUN1424005a0 multiplies physical slots by eight.
pub const TEXTURE_SAMPLER_ENTRY_STRIDE: usize = 8;
/// Native pixel-stage mask; archived FUN1424005a0 tests param4 bit one independently of the vertex-stage path.
pub const PIXEL_STAGE_MASK: u32 = 1;
/// Native vertex-stage mask; archived FUN1424005a0 tests param4 bit two independently of the pixel-stage path.
pub const VERTEX_STAGE_MASK: u32 = 2;
/// Texture/sampler binder; FUN141268c60 passes the same context as its shader binder to FUN1424005a0's dispatch slot.
pub const TEXTURE_SAMPLER_BINDER_RVA: usize = 0x0240_05a0;
/// Cached blend-state wrapper; archived FUN142400090 compares and replaces context+0x228.
pub const BLEND_STATE: usize = 0x228;
/// Cached rasterizer-state wrapper; archived FUN142400330 compares and replaces context+0x238.
pub const RASTERIZER_STATE: usize = 0x238;
/// Separate renderer pointer; FUN142400090/330 obtain fallback states through context+0x280 then renderer+0x58/+0x68.
pub const STATE_RENDERER: usize = 0x280;
/// Blend-state binder; FUN141268c60 passes its shader/sampler context to FUN142400090's dispatch slot.
pub const BLEND_STATE_BINDER_RVA: usize = 0x0240_0090;
/// Rasterizer-state binder; FUN141268c60 passes its shader/sampler context to FUN142400330's dispatch slot.
pub const RASTERIZER_STATE_BINDER_RVA: usize = 0x0240_0330;
