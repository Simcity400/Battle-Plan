//!
//! The element is separate from the surrounding collection owner. Its full
//! C++ identity is unproven; this module preserves only the observed stride.

/// Byte distance between elements of the owner's +0x3b0 collection.
/// Source: `portable_replay/native/engine_evidence/src/xmap20_g7_census_adjudication.rs:309`.
/// Confirmation: `portable_replay/native/engine_evidence/artifacts/func_141b6f0c0.decompile.c:193-199` in the pinned a729 export
/// steps by 0x68 and divides the byte difference by 0x68 to form the record index.
pub const STRIDE: usize = 0x68;
