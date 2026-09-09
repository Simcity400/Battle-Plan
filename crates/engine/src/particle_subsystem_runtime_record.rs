//!
//! The destination lVar12 is a 0x550-byte-stride output record, distinct from
//! the authored lVar11 input record. This descriptive name does not claim a
//! C++ class or equate it with the separate particle instance allocation.

/// Destination attribute compiled from authored position-Y inputs.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:344`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_14232ecc0.decompile.c:45-46,64` in the pinned a729 export
/// passes lVar12 + 0x1a0 with lVar11 + 0x3b0 and lVar11 + 0x3f8; the editor names those source attributes posy and posy_var at func_142085a00.decompile.c:109-112.
pub const POS_Y_ATTRIBUTE: usize = 0x1a0;
