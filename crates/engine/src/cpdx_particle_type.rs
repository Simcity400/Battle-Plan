//! CPdxParticleType descriptor reached from CPdxParticleObject +0x58.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The class is named by func_142281570.decompile.c:15.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Render subbucket u16; a729_hook.rs:6339 reads it.
pub const RENDER_SUBBUCKET: usize = 0x38;
/// CPdxParticleType vtable stored by FUN142281570, instructions:21 at VA142b2a838.
pub const VTABLE_RVA: usize = 0x02b2_a838;
/// CPdxParticleType allocation size, effect_census_authority.json:315-317 documents FUN14208f200 allocating 0xd8.
pub const SIZE: usize = 0xd8;
/// CPdxParticleType synthesized attachment token std::string, render_stream/src/lib.rs:2442-2446 and effect_census_authority.json:320-323.
pub const ATTACHMENT_TOKEN: usize = 0x10;
/// CPdxParticleType authored scalar f32 sent to runtime+0x48, func_14227ea10.decompile.c:25.
pub const AUTHORED_SCALE: usize = 0x30;
/// CPdxParticleType resolved authored-type pointer, func_14227ea10.decompile.c:19 and effect_census_authority.json:330-333.
pub const RESOLVED_TYPE: usize = 0x40;
/// CPdxParticleType authored effect token std::string, effect_census_authority.json:335-338 records FUN142281a80 binding it.
pub const EFFECT_TOKEN: usize = 0xb8;
/// CPdxParticleType constructor, func_142281570.decompile.c:7-15 identifies the class.
pub const CONSTRUCTOR_RVA: usize = 0x0228_1570;
/// FUN142281a80 binds descriptor token and resolved type, effect_census_authority.json:92-97.
pub const BIND_RVA: usize = 0x0228_1a80;
/// Engine-synthesized attachment name prefix location, effect_census_authority.json:321; not authored content.
pub const ATTACHMENT_PREFIX_RVA: usize = 0x02ae_6b58;
/// Process-global attachment name counter, effect_census_authority.json:321; identities are not stable across processes.
pub const ATTACHMENT_COUNTER_RVA: usize = 0x0343_6210;
