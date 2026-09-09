//! SGfxRasterizerStateReader, named by archived FUN142348f80.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348f80.*`
//! at `archive/pre-rewrite-2026-09-09`. Presentation parser knowledge only.

/// SGfxRasterizerStateReader vtable; FUN142348f80 instruction 14234955f loads 142b3b218 in its named construction branch.
pub const VTABLE_RVA: usize = 0x02b3_b218;
/// Declaration name std::string; FUN142348f80 constructs it immediately after the reader vptr.
pub const NAME: usize = 0x08;
/// Inline native rasterizer description; FUN142348f80 copies the constructor output after the reader's name at +0x28.
pub const DESCRIPTION: usize = 0x28;
