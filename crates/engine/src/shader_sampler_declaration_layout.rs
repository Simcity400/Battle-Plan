//! Anonymous shader sampler declaration stored by SGfxSamplerReader.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348d70.decompile.c`
//! and `func_142345450.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! The original declaration class name is unproved; this is presentation knowledge only.

/// Declaration row size; FUN142348d70 allocates and copies array elements in steps of 0x70.
pub const STRIDE: usize = 0x70;
/// Declaration row allocation alignment; FUN142348d70 passes 0x10 to its array allocator.
pub const ALIGNMENT: usize = 0x10;
/// Declaration name std::string; FUN142348d70 copies the reader name to the row base.
pub const NAME: usize = 0;
/// Sampler Type u32; FUN142348d70 copies reader+0x30 to declaration+0x20.
pub const TYPE: usize = 0x20;
/// Native 0x40-byte sampler descriptor; FUN142345450 passes row+0x30 to the selected sampler lowering function.
pub const DESCRIPTION: usize = 0x30;
