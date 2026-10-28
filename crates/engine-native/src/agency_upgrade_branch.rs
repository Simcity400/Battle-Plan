//! Agency upgrade branch definitions owned by the database. The loader reuses
//! a branch across files and rebuilds same-name definitions within that branch.
//! Authored identity comes from the live lexical dictionary.

/// Primary branch vtable.
pub const VTABLE_RVA: usize = 0x0285_cc10;
/// Branch lexer token.
pub const TOKEN: usize = 0x08;
/// Non-null branch valid byte; any nonzero value is valid.
pub const VALID: usize = 0x10;

/// Member-pointer array with no sentinel.
pub const UPGRADES: usize = 0x40;
