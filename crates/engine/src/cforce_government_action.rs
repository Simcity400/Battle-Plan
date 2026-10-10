//! CForceGovernmentAction layout for the hash-pinned executable.

/// Force-government action primary vtable.
pub const VTABLE_RVA: usize = 0x029b_c650;
/// Raw country operand. The 0x58-byte action has no puppet auxiliary at +0x58.
pub const COUNTRY: usize = 0x50;
