//! SGfxBlendStateReader, named by archived FUN142348f80.
//!
//! Source: `portable_replay/native/engine_evidence/artifacts/func_142348f80.*`
//! and `func_142347e10.decompile.c` at `archive/pre-rewrite-2026-09-09`.
//! Presentation parser knowledge only; no captured table or hook is enabled.

/// SGfxBlendStateReader vtable; FUN142348f80 instruction 1423491b1 loads 142b3b178, matching its named construction branch.
pub const VTABLE_RVA: usize = 0x02b3_b178;
/// Declaration name std::string; FUN142348f80 constructs it immediately after the reader vptr.
pub const NAME: usize = 0x08;
/// Inline native blend description; FUN142347e10 reads BlendEnable at reader+0x28.
pub const DESCRIPTION: usize = 0x28;
/// SGfxBlendStateReader field dispatcher; FUN142347e10 writes the inline description from ordered authored tokens.
pub const READ_FIELD_RVA: usize = 0x0234_7e10;
