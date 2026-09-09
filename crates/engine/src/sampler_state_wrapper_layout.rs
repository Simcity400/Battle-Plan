//! Anonymous native sampler wrapper allocated by FUN1423fd550.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_1423fd550.decompile.c`
//! and `func_1424005a0.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! Original C++ class name unproved; presentation knowledge only.

/// Wrapper allocation byte size; FUN1423fd550 allocates 0x50 bytes for API object plus native descriptor.
pub const SIZE: usize = 0x50;
/// D3D sampler pointer output; FUN1423fd550 gives the wrapper base to CreateSamplerState's output parameter.
pub const D3D_SAMPLER: usize = 0;
/// Copied native descriptor; FUN1423fd550 copies its input descriptor to wrapper+0x10.
pub const DESCRIPTION: usize = 0x10;
