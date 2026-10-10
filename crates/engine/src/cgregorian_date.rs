//! `CGregorianDate`; inherited CDate storage contains an i32 raw hour.

/// CGregorianDate primary vtable.
pub const VTABLE_RVA: usize = 0x0270_2190;
/// CGregorianDate CDate subobject.
pub const DATE_BASE: usize = 0x08;
/// Unsupported Gregorian-date vtable candidate.
pub const REJECTED_LEGACY_VTABLE_RVA: usize = 0x026f_21a0;
