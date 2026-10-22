//! CNavyLeader extends CUnitLeader with signed i32 skills.

/// CNavyLeader primary vtable.
pub const VTABLE_RVA: usize = 0x0293_f650;
/// CNavyLeader secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0293_f768;
/// CNavyLeader secondary subobject offset.
pub const SECONDARY_BASE: usize = 0xf58;
/// CNavyLeader attack i32.
pub const ATTACK: usize = 0xf68;
/// CNavyLeader defense i32.
pub const DEFENSE: usize = 0xf78;
/// CNavyLeader maneuvering i32.
pub const MANEUVERING: usize = 0xf88;
/// CNavyLeader coordination i32.
pub const COORDINATION: usize = 0xf98;
/// Navy-leader allocation extent, not an array stride.
pub const SIZE: usize = 0xfc8;
/// Navy-leader constructor installs both vtables and signed skill words.
pub const CONSTRUCTOR_RVA: usize = 0x00c0_3290;
