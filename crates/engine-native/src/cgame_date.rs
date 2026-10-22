//! CGameDate includes its secondary persistent subobject.
//!
//! The vtable pointers at `GREGORIAN_BASE` and `PERSISTENT_BASE` belong to one date object. Textual
//! save hours range from 1 through 24.

/// CGameDate primary vtable.
pub const VTABLE_RVA: usize = 0x0270_2388;
/// CGameDate CPersistent vtable.
pub const PERSISTENT_VTABLE_RVA: usize = 0x0270_23b8;
/// CGameDate CGregorianDate base.
pub const GREGORIAN_BASE: usize = 0;
/// Embedded date base containing signed i32 raw hour.
pub const DATE_BASE: usize = 0x08;
/// CGameDate CPersistent base.
pub const PERSISTENT_BASE: usize = 0x10;

/// Stored raw hour, preserving null and negative values. A date's primary and
/// persistent vtables authenticate the same embedded object.
pub fn read_raw(
    memory: &impl crate::Memory,
    base: usize,
    address: usize,
) -> Result<i32, &'static str> {
    let primary = base
        .checked_add(VTABLE_RVA)
        .ok_or("game date vtable address overflow")? as u64;
    let persistent = base
        .checked_add(PERSISTENT_VTABLE_RVA)
        .ok_or("game date vtable address overflow")? as u64;
    if crate::read::<u64>(memory, address) != Some(primary)
        || address
            .checked_add(PERSISTENT_BASE)
            .and_then(|at| crate::read::<u64>(memory, at))
            != Some(persistent)
    {
        return Err("game date vtable mismatch or unreadable");
    }
    address
        .checked_add(DATE_BASE)
        .and_then(|at| crate::read(memory, at))
        .ok_or("game date raw hour unreadable")
}
/// CGameDate null raw hour i32.
pub const NULL_RAW_HOUR: i32 = 43_808_760;
