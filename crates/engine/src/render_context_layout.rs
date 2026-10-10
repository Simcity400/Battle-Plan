//! Final D3D draw context, distinct from the map-scene camera pointer.
//! Raster, blend, texture/sampler and shader-program binders share this
//! receiver. Presentation metadata; no recorded table or hook is enabled.

/// Fallback index count, i32, for indexed instanced draws.
pub const FALLBACK_INDEX_COUNT: usize = 0x04;
/// Fallback vertex count, i32, for nonindexed draws.
pub const FALLBACK_VERTEX_COUNT: usize = 0x08;
/// D3D device-context pointer.
pub const D3D_DEVICE_CONTEXT: usize = 0x2a0;
/// Final nonindexed draw routine.
pub const PARTICLE_FINAL_DRAW_RVA: u32 = 0x023F_E970;
/// Exact nonindexed draw entry sentinel.
pub const PARTICLE_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
/// Final indexed draw routine.
pub const ENTITY_FINAL_DRAW_RVA: u32 = 0x023F_E9B0;
/// Exact indexed draw entry sentinel.
pub const ENTITY_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf9, 0x8b, 0xda, 0x85,
];
/// Final indexed instanced draw routine.
pub const INDEXED_INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_E9F0;
/// Final nonindexed instanced draw routine.
pub const INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_EA40;
/// Exact nonindexed instanced draw entry sentinel.
pub const INSTANCED_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
/// Shader-program binder.
pub const SHADER_PROGRAM_BINDER_RVA: u32 = 0x0240_0520;
/// Exact shader binder entry sentinel.
pub const SHADER_PROGRAM_BINDER_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xd2, 0x74, 0x79, 0x56, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0x98, 0x02, 0x00, 0x00,
];

/// Cached pixel-stage texture wrappers at +0x20 + slot * 8, selected by stage bit one.
pub const PIXEL_TEXTURES: usize = 0x20;
/// Cached pixel-stage sampler wrappers at +0xa0 + slot * 8, selected by stage bit one.
pub const PIXEL_SAMPLERS: usize = 0xa0;
/// Cached vertex-stage texture wrappers at +0x120 + slot * 8, selected by stage bit two.
pub const VERTEX_TEXTURES: usize = 0x120;
/// Cached vertex-stage sampler wrappers at +0x1a0 + slot * 8, selected by stage bit two.
pub const VERTEX_SAMPLERS: usize = 0x1a0;
/// Cached pointer-entry stride.
pub const TEXTURE_SAMPLER_ENTRY_STRIDE: usize = 8;
/// Pixel-stage mask, tested independently of the vertex path.
pub const PIXEL_STAGE_MASK: u32 = 1;
/// Vertex-stage mask, tested independently of the pixel path.
pub const VERTEX_STAGE_MASK: u32 = 2;
/// Texture/sampler binder using this shared context.
pub const TEXTURE_SAMPLER_BINDER_RVA: usize = 0x0240_05a0;
/// Cached blend-state wrapper.
pub const BLEND_STATE: usize = 0x228;
/// Cached rasterizer-state wrapper.
pub const RASTERIZER_STATE: usize = 0x238;
/// Separate renderer pointer; fallback blend/raster states are at renderer +0x58/+0x68.
pub const STATE_RENDERER: usize = 0x280;
/// Blend-state binder using this shared context.
pub const BLEND_STATE_BINDER_RVA: usize = 0x0240_0090;
/// Rasterizer-state binder using this shared context.
pub const RASTERIZER_STATE_BINDER_RVA: usize = 0x0240_0330;
