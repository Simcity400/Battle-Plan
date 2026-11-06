//! Agency upgrade database, including its global first-definition registry and
//! its owning branch registry. The loader keeps the first global pointer for a
//! repeated token across branches; valid branch-owned definitions may be absent
//! from that registry. Acquisition authenticates exact branch membership rather
//! than selecting a definition by token.

/// Primary database vtable.
pub const VTABLE_RVA: usize = 0x0285_ccb0;
/// Global definition-pointer vector with a null object in slot zero.
pub const DEFINITIONS: usize = 0x40;

/// Database singleton storage.
pub const SINGLETON_RVA: usize = 0x033d_d200;
/// Branch-pointer vector; slot zero is the null object, authored branches start at one.
pub const ITEMS: usize = 0x70;
/// Signed live branch count including the null object.
pub const COUNT: usize = 0x7c;
/// Branch-pointer stride.
pub const ITEM_STRIDE: usize = 8;
