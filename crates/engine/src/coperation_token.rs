//! Operation-token definition used by the country membership census.
//! The string at +0x50 is passed through localisation. Its precise
//! authored-identity or display meaning is unresolved.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0270_4dd0;
/// Authored statement lexer ordinal, u32.
pub const NUMERIC_ID: usize = 0x08;
/// MSVC string passed through localisation; authored identity is unconfirmed.
pub const ID_STRING_CANDIDATE: usize = 0x50;
