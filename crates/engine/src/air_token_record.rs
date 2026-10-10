//! Candidate heap token record; its concrete type and the identity word
//! borrowed from the flag manager are unknown. Flag rows use a separate layout;
//! no heap scan or guessed identity is acquired.

/// Candidate identity word matched against a flag-manager-derived word.
pub const IDENTITY_WORD_CANDIDATE: usize = 0x00;
/// Candidate u64 id storage; narrow only after validating its permitted range.
pub const ID_STORAGE_CANDIDATE: usize = 0x08;
/// Candidate NUL-terminated ASCII text pointer, followed only after matching identity.
pub const TEXT_CANDIDATE: usize = 0x10;
