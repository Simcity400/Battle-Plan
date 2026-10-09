//! CForceGovernmentAction layout for the hash-pinned executable.

/// Primary vtable, confirmed by a729 RTTI and constructor 141506BE0.
pub const VTABLE_RVA: usize = 0x029b_c650;
/// Raw country operand: this class uses 14150DC80/14150D230 at slots 2/4.
/// 141509550 allocates only 0x58 bytes; it has no puppet auxiliary at +0x58.
pub const COUNTRY: usize = 0x50;
