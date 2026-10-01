//! CPdx3DObject base of CPdxParticleObject, named by FUN1422c2410's vtable store.
//! Native presentation layout retained from the pre-rewrite exact-build decompiles.

/// CPdx3DObject vtable, func_1422c2410.instructions.txt:8 resolves the constructor store to VA142b29ba0.
pub const VTABLE_RVA: usize = 0x02b2_9ba0;
/// CPdx3DObject constructor, func_1422c2410.decompile.c:7-11 identifies the class.
pub const CONSTRUCTOR_RVA: usize = 0x022c_2410;
/// CPdx3DObject transform `f32[16]`, func_1422c2410.decompile.c:12-29 identity-initializes the block; world resolution is not implied.
pub const TRANSFORM: usize = 0x10;
/// CPdx3DObject descriptor pointer from the constructor's second argument, func_1422c2410.decompile.c:22; particle traversal reads it in a729_hook.rs:6339.
pub const DESCRIPTOR: usize = 0x58;
/// CPdx3DObject flags u8, initialized by func_1422c2410.decompile.c:21 and read by a729_hook.rs:11111.
pub const FLAGS: usize = 0x60;
/// CPdx3DObject constructor ORs these low bits into the existing flags byte, func_1422c2410.decompile.c:21.
pub const CONSTRUCTOR_FLAGS_MASK: u8 = 7;
/// CPdx3DObject enabled bit, propagated to particle runtime+0x54 by func_14227ecb0.decompile.c:11.
pub const ENABLED_MASK: u8 = 1;
