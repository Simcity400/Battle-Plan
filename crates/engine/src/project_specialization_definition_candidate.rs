//! Candidate specialization definition; no native RTTI class name identified.
//!
//! Discovery-only retained knowledge, with no new runtime proof or promotion.
//! This is the object pointed to by the candidate manager's +0x60 vector.
//! The A28 `FACILITY_SPECIALIZATION_SMOOTH_LIST_HOOK_CONTRACT_20260719.json:35`
//! identifies the copied four-byte value as a LexerToken in discovery evidence.
//! The direct a729/870ebf export is
//! `C:/hoi4_evidence/ghidra_agent_exports/20260719T222529Z-wave2-r10-addr-04-02-b748aca3/evidence/func_140aabad0.decompile.c`.
//! No definition vtable, size, or further fields are established here.

pub const LEXER_TOKEN: usize = 0x08;
