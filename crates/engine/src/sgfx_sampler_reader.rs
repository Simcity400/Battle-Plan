//! SGfxSamplerReader, named by the archived FUN142348d70 constructor export.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348d70.*`
//! and `func_142348890.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! These presentation parser constants do not enable capture or hooks.

/// SGfxSamplerReader vtable; FUN142348d70 instruction 142348d96 loads 142b3afc8, matching the named decompile.
pub const VTABLE_RVA: usize = 0x02b3_afc8;
/// Sampler name std::string; FUN142348d70 places it 0x10 bytes after the reader vptr before parsing.
pub const NAME: usize = 0x10;
/// Sampler Type u32; FUN142348890 token 0xe1 scans to +0x30, outside the descriptor sent to D3D lowering.
pub const TYPE: usize = 0x30;
/// Inline native sampler descriptor; FUN142348890 scans Index at +0x40 and FUN142348d70 copies it to declaration+0x30.
pub const DESCRIPTION: usize = 0x40;
/// SGfxSamplerReader field dispatcher; FUN142348890 is the callback in sampler_reader_callbacks.txt at vtable+0x20.
pub const READ_FIELD_RVA: usize = 0x0234_8890;
/// Reader construction and declaration append; FUN142348d70 initializes defaults and appends 0x70-byte rows.
pub const READ_DECLARATION_RVA: usize = 0x0234_8d70;
/// Sampler Type token scanner; FUN142348890 dispatches token 0xe1 to FUN1423f8070 with reader+0x30.
pub const READ_TYPE_RVA: usize = 0x023f_8070;
