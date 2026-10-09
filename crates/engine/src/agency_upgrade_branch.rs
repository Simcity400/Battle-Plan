//! CAgencyUpgradeBranch definitions owned by CAgencyUpgradeDatabase.
//! Loader@140617860 reuses a branch across files; reader@140618C40 rebuilds an
//! existing same-name definition within its exact branch. Authored identity
//! comes from the live lexical dictionary, never a vanilla catalog.

/// Loader@140617860 constructs this primary vtable; confirmed pinned RTTI.
pub const VTABLE_RVA: usize = 0x027c_8cc0;
/// Loader@140617860 stores the lexer token returned by AddOrFindDynamicToken.
pub const TOKEN: usize = 0x08;
/// Loader@140617860 constructs non-null branches with a nonzero valid byte.
pub const VALID: usize = 0x10;

/// Reader@140618C40 owns this member-pointer array; unlike the DB, it has no sentinel.
pub const UPGRADES: usize = 0x40;
/// Signed live count, reader@140618C40. CPdxArray validation admits its nonnegative range.
pub const UPGRADE_COUNT: usize = 0x4c;
/// Reader@140618C40 indexes definition pointers by eight bytes.
pub const UPGRADE_STRIDE: usize = 8;
