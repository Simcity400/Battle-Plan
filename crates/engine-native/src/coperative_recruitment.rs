//! Recruitment fields relative to the inline subobject rather than agency.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029c_2400;
/// COperativeRecruitment allocation/subobject size.
pub const SIZE: usize = 0x58;
/// COperativeRecruitment owning CCountry pointer.
pub const OWNER_COUNTRY: usize = 0x08;
/// First operative pointer-vector data.
pub const POOL_A: usize = 0x10;
/// First-vector signed i32 size, relative to the recruitment subobject.
pub const POOL_A_COUNT: usize = 0x1c;
/// Recruitable-operative pointer-vector data.
pub const RECRUITABLE_OPERATIVES: usize = 0x28;
/// COperativeRecruitment recruitable-vector u32 capacity.
pub const RECRUITABLE_OPERATIVE_CAPACITY: usize = 0x30;
/// COperativeRecruitment recruitable-vector i32 size.
pub const RECRUITABLE_OPERATIVE_COUNT: usize = 0x34;
/// Not-to-spy-master operative pointer-vector data.
pub const NOT_TO_SPY_MASTER: usize = 0x40;
/// COperativeRecruitment not-to-spy-master vector u32 capacity.
pub const NOT_TO_SPY_MASTER_CAPACITY: usize = 0x48;
/// COperativeRecruitment not-to-spy-master vector i32 size.
pub const NOT_TO_SPY_MASTER_COUNT: usize = 0x4c;
/// Recruitment constructor, receiving the agency recruitment subobject.
pub const CONSTRUCTOR_RVA: usize = 0x0156_6e10;
/// Insertion boundary for operatives rejected by the agency ceiling.
pub const INSERT_OPERATIVE_RVA: usize = 0x0156_7190;
/// Operative removal scanning all three vectors by pointer identity.
pub const REMOVE_OPERATIVE_RVA: usize = 0x0156_9770;
/// Recruitable-vector source entry.
pub const RECRUITABLE_VECTOR_SOURCE_RVA: usize = 0x0156_7cb0;
