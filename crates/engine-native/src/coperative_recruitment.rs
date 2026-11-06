//! Recruitment fields relative to the inline subobject rather than agency.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a5_2608;
/// COperativeRecruitment allocation/subobject size.
pub const SIZE: usize = 0x58;
/// COperativeRecruitment owning CCountry pointer.
pub const OWNER_COUNTRY: usize = 0x08;
/// First operative pointer-vector data.
pub const POOL_A: usize = 0x10;
/// Recruitable-operative pointer-vector data.
pub const RECRUITABLE_OPERATIVES: usize = 0x28;
/// Not-to-spy-master operative pointer-vector data.
pub const NOT_TO_SPY_MASTER: usize = 0x40;
/// Recruitment constructor, receiving the agency recruitment subobject.
pub const CONSTRUCTOR_RVA: usize = 0x0161_4770;
