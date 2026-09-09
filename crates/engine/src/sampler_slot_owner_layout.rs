//! Anonymous sampler-slot owner passed as FUN142345450's third argument.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142345450.decompile.c`
//! and caller `func_142344df0.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! This may be a shader-program row but pointer identity is not established here.
//! Later duplicate physical slots replace earlier entries. Presentation knowledge only.

/// Sampler-wrapper pointer array; FUN142345450 indexes owner+0x20 by the descriptor's physical slot.
pub const SAMPLERS: usize = 0x20;
/// Sampler-array capacity i32; FUN142345450 grows it at owner+0x28.
pub const CAPACITY: usize = 0x28;
/// Sampler-array count i32; FUN142345450 zero-fills missing slots through owner+0x2c.
pub const COUNT: usize = 0x2c;
/// Sampler-array allocator pointer; FUN142345450 invokes allocator methods through owner+0x30.
pub const ALLOCATOR: usize = 0x30;
/// Sampler-wrapper pointer stride; FUN142345450 multiplies the physical slot by eight.
pub const ENTRY_STRIDE: usize = 8;
/// Sampler declaration array lowering; FUN142345450 creates native wrappers and assigns their physical slots.
pub const POPULATE_RVA: usize = 0x0234_5450;
