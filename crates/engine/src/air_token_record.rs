//! Candidate heap token record from the old air-variant dictionary scan.
//! Its concrete C++ type and the identity word borrowed from CFlagManager+18 are unproven.
//! This is distinct from D51 flag rows; no heap scan or guessed identity is ported.

/// Candidate token-record identity word at +0; refresh_compact_air_variant_token_dictionary matches a CFlagManager-derived word.
pub const IDENTITY_WORD_CANDIDATE: usize = 0x00;
/// Candidate token-record id storage; old air dictionary loads u64 at +8 then narrows only after its policy bound check.
pub const ID_STORAGE_CANDIDATE: usize = 0x08;
/// Candidate token-record pointer to NUL-terminated ASCII text; old air dictionary follows +10 after matching the identity word.
pub const TEXT_CANDIDATE: usize = 0x10;
