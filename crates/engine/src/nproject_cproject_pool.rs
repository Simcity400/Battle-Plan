//! `NProject::CProjectPool`; archived collector `read_country_project_lifecycle`.

/// NProject::CProjectPool primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x029b_0238;
/// CProjectPool +0x10 first inline CProject pointer; lifecycle walker advances by CProject's 0x1c0 stride.
pub const PROJECTS: usize = 0x10;
/// CProjectPool +0x18 packed capacity/count u64; archived lifecycle walker splits low/high u32 halves.
pub const PACKED_CAPACITY_COUNT: usize = 0x18;
/// CProjectPool +0x18 capacity u32; low half of archived lifecycle walker's packed header.
pub const CAPACITY: usize = 0x18;
/// CProjectPool +0x1c count u32; high half of archived lifecycle walker's packed header.
pub const COUNT: usize = 0x1c;
