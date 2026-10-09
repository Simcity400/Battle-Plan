//! CAgencyUpgradeDatabase, including its global first-definition registry and
//! its owning branch registry. Loader@140617860 keeps the first global pointer
//! for a repeated token across branches; valid branch-owned definitions may be
//! absent from that global registry. Acquired capture authenticates exact branch
//! membership rather than selecting a definition by token.

/// Constructor@140615EC0 stores this primary vtable; confirmed pinned RTTI.
pub const VTABLE_RVA: usize = 0x027c_8d40;
/// Lookup@140617670 and loader@140617860 use this global definition-pointer vector.
/// Constructor@140615EC0 installs a native null object in slot zero.
pub const DEFINITIONS: usize = 0x40;

/// Accessor@14039D6B0 returns the database singleton stored here.
pub const SINGLETON_RVA: usize = 0x0331_69f0;
/// Constructor@140615EC0 and loader@140617860 own this branch-pointer vector,
/// with the native null object in slot zero and authored branches from one.
pub const ITEMS: usize = 0x70;
/// Signed live count including the null object; loader@140617860.
pub const COUNT: usize = 0x7c;
/// Loader@140617860 indexes branch pointers by eight bytes.
pub const ITEM_STRIDE: usize = 8;
