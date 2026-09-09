//! Particle runtime allocated by FUN1423e8190.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Particle type pointer; a729_hook.rs:11245 reads it before subsystem arrays.
pub const TYPE: usize = 0x00;
/// Emitter-array header/data pointer; engine_evidence/artifacts/func_1423e8190.decompile.c:25 initializes +8, and XMAP20/effect_census_authority.json:355-359 confirms the emitter walk. This is an array pointer, not an inline emitter or elapsed-time field.
pub const EMITTERS: usize = 0x08;
/// Particle scale f32; a729_hook.rs:6280 keeps its raw 32 bits.
pub const SCALE: usize = 0x48;
/// Native f32 at +0x4c, called alpha by a729_hook.rs:11112; effect_census_authority.json:393-397 leaves its runtime meaning unproven.
pub const ALPHA: usize = 0x4c;
/// Native f32 at +0x50, called playback by a729_hook.rs:11113; effect_census_authority.json:393-397 leaves its runtime meaning unproven.
pub const PLAYBACK: usize = 0x50;
/// Runtime allocation size, func_1423e8190.decompile.c:20 requests this many bytes.
pub const SIZE: usize = 0x58;
/// Runtime back-pointer holds node+0x10, not the node base; func_14227ea10.decompile.c:24 and effect_census_authority.json:380-384.
pub const NODE_TRANSFORM_POINTER: usize = 0x40;
/// Runtime enabled u8, func_1423e9150.decompile.c:10 writes it; xmap20_effects.rs:68-75 corroborates census use.
pub const ENABLED: usize = 0x54;
/// Runtime second flag u8 is uninitialized until FUN1423e9020 writes 1; xmap20_effects.rs:77-81 does not prove stop-emission semantics.
pub const SECOND_FLAG: usize = 0x55;
/// FUN1423e9150 writes the runtime enable byte, exact decompile:7-10.
pub const ENABLE_SETTER_RVA: usize = 0x023e_9150;
/// FUN1423e9020 writes runtime+0x55; XMAP20/effect_census_authority.json:178-182 records the independent setter.
pub const SECOND_FLAG_SETTER_RVA: usize = 0x023e_9020;
/// FUN1409bc400 writes the node-transform pointer after census publication, effect_census_authority.json:31-35,380-384.
pub const NODE_TRANSFORM_SETTER_RVA: usize = 0x009b_c400;
/// FUN1423e9190 writes the authored f32 at runtime+0x48, func_1423e9190.decompile.c:7-10.
pub const AUTHORED_SCALE_SETTER_RVA: usize = 0x023e_9190;
/// Runtime allocator FUN1423e8190; lib.rs:70 and hook_create retain its object-returning ABI.
pub const PARTICLE_RUNTIME_CREATE_RVA: u32 = 0x023E_8190;
/// Runtime allocation entry bytes checked by a729_hook.rs:218.
pub const CREATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2, 0x48, 0x8b, 0xe9,
];
/// Runtime destruction FUN1423e8400; lib.rs:71 identifies the lifetime endpoint.
pub const PARTICLE_RUNTIME_DESTROY_RVA: u32 = 0x023E_8400;
/// Runtime destruction entry bytes checked by a729_hook.rs:221.
pub const DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0xf4, 0x00, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89,
];
/// Runtime packet builder FUN1423e8b90; lib.rs:72 and hook_builder retain normal and ribbon arrays.
pub const PARTICLE_PACKET_BUILDER_RVA: u32 = 0x023E_8B90;
/// Packet builder entry bytes checked by a729_hook.rs:224.
pub const BUILDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x55,
];
