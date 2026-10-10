//! Agency upgrade branch definitions owned by the database. The loader reuses
//! a branch across files and rebuilds same-name definitions within that branch.
//! Authored identity comes from the live lexical dictionary.

/// Primary branch vtable.
pub const VTABLE_RVA: usize = 0x027c_8cc0;
/// Branch lexer token.
pub const TOKEN: usize = 0x08;
/// Non-null branch valid byte; any nonzero value is valid.
pub const VALID: usize = 0x10;

/// Member-pointer array with no sentinel.
pub const UPGRADES: usize = 0x40;
/// Signed member count; array validation admits its nonnegative range.
pub const UPGRADE_COUNT: usize = 0x4c;
/// Definition-pointer stride.
pub const UPGRADE_STRIDE: usize = 8;
