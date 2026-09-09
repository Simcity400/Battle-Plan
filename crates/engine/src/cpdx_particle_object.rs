//! CPdxParticleObject, constructed by FUN14227ea10 and derived from CPdx3DObject.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The class is named by func_14227ea10.decompile.c:17.
//! Presentation knowledge only; this module does not enable a recorded table or hook.
//! Inherited transform, descriptor and flags fields live in `cpdx_3d_object`.

/// Particle runtime pointer; a729_hook.rs:6319 and :11110 follow it.
pub const RUNTIME: usize = 0x70;
/// CPdxParticleObject vtable stored by FUN14227ea10, instructions:18 at VA142b2a498.
pub const VTABLE_RVA: usize = 0x02b2_a498;
/// CPdxParticleObject allocation size, effect_census_authority.json:84-89 names FUN142281980's 0x80 allocation.
pub const SIZE: usize = 0x80;
/// CPdxParticleObject time-scale f32, func_14227ea10.decompile.c:18 writes 1.0 at +0x78; old live round-trip confirmed the field.
pub const TIME_SCALE: usize = 0x78;
/// FUN14227ecb0 propagates node+0x60 bit0 to runtime+0x54, exact decompile:7-11.
pub const PROPAGATE_ENABLE_RVA: usize = 0x0227_ecb0;
/// Particle object constructor FUN14227ea10; lib.rs:68 and hook_object_ctor preserve this endpoint.
pub const PARTICLE_OBJECT_CTOR_RVA: u32 = 0x0227_EA10;
/// Particle object constructor entry bytes checked by a729_hook.rs:212.
pub const OBJECT_CTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Particle bucket submit FUN14227ecd0; lib.rs:69 and hook_bucket preserve this endpoint.
pub const PARTICLE_BUCKET_SUBMIT_RVA: u32 = 0x0227_ECD0;
/// Particle bucket submit entry bytes checked by a729_hook.rs:215.
pub const BUCKET_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x41, 0x58, 0x4c, 0x8d,
];
