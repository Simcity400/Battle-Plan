//! Renderer passed to entity submission and window/backbuffer operations; receiver aliases need rechecking.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Renderer backend pointer; a729_hook.rs:8068 and :8075 follow it for the backbuffer.
pub const BACKEND: usize = 0x268;
/// Inline shader-program array for nonnegative indices; a729_hook.rs:10577 indexes it by 0x38.
pub const SHADER_PROGRAMS: usize = 0x558;
/// Generic Pdxmesh caster producer; s1_pdxmesh_caster.rs:11 identifies this renderer operation.
pub const PDXMESH_CASTER_PRODUCER_RVA: usize = 0x0226_4cc0;
/// Generic Pdxmesh caster entry bytes checked by s1_pdxmesh_caster.rs:20.
pub const PDXMESH_CASTER_PRODUCER_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x56, 0x48, 0x8d, 0xac, 0x24, 0x20, 0xfe, 0xff, 0xff, 0x48,
];
/// Renderer envelope FUN142228df0; lib.rs:67 and hook_envelope pass renderer, camera carrier and ordinal.
pub const GLOBAL_RENDER_ENVELOPE_RVA: u32 = 0x0222_8DF0;
/// Renderer envelope entry bytes checked by a729_hook.rs:209.
pub const ENVELOPE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Entity render root FUN1422644c0; lib.rs:77 and capture_entity_render_root retain its receiver.
pub const ENTITY_RENDER_ROOT_RVA: u32 = 0x0226_44C0;
/// Entity render entry bytes checked by a729_hook.rs:251.
pub const ENTITY_RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x55, 0x57, 0x41, 0x57, 0x48, 0x8d, 0xa8, 0x48, 0xfd,
];
/// Particle submit FUN14227f420; lib.rs:73 and hook_particle_render_submit retain its renderer receiver.
pub const PARTICLE_RENDER_SUBMIT_RVA: u32 = 0x0227_F420;
/// Particle submit entry bytes checked by a729_hook.rs:273.
pub const PARTICLE_RENDER_SUBMIT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
/// Window backbuffer producer FUN1420fddd0; a729_hook.rs:202 and :8064 identify its renderer receiver.
pub const WINDOW_BACKBUFFER_RTV_PRODUCER_RVA: usize = 0x020F_DDD0;
/// Window backbuffer producer entry bytes checked by a729_hook.rs:239.
pub const WINDOW_BACKBUFFER_RTV_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x55, 0x48, 0x8d, 0x68, 0xa1, 0x48, 0x81, 0xec, 0x00, 0x01, 0x00, 0x00, 0x48,
];
/// Opaque target producer FUN1420fda10; a729_hook.rs:203 and :8136 identify its renderer receiver.
pub const OPAQUE_TEXTURE_TARGET_PRODUCER_RVA: usize = 0x020F_DA10;
/// Opaque target producer entry bytes checked by a729_hook.rs:242.
pub const OPAQUE_TEXTURE_TARGET_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
];
/// Present root FUN1420b6530; lib.rs:92 and hook_present_render_root retain the renderer argument.
pub const PRESENT_RENDER_ROOT_RVA: u32 = 0x020B_6530;
/// Present root entry bytes checked by a729_hook.rs:309.
pub const PRESENT_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48,
];
