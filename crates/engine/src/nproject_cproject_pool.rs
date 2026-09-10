//! `NProject::CProjectPool`; archived collector `read_country_project_lifecycle`.

use crate::{Memory, cpdx_array::Array, nproject_cprogram_status, nproject_cproject, read};

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

pub(crate) fn from_status(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<Array, &'static str> {
    let pool = status
        .checked_add(nproject_cprogram_status::PROJECT_POOL)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("project pool pointer unreadable or null")?;
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pool) != Some(vtable as u64) {
        return Err("project pool vtable mismatch");
    }
    pool.checked_add(PROJECTS)
        .and_then(|p| Array::read(memory, p, nproject_cproject::POOL_STRIDE))
        .ok_or("project pool array unreadable or invalid")
}
