//! `CGregorianDate`; inherited CDate storage contains an i32 raw hour.

/// CGregorianDate primary vtable; build_profile.rs:513 and pinned PE RTTI confirm 0x2702190.
pub const VTABLE_RVA: usize = 0x0270_2190;
/// CGregorianDate CDate subobject; pinned RTTI places it at +8, which contains the signed raw hour.
pub const DATE_BASE: usize = 0x08;
/// Rejected old CGregorianDate vtable candidate; collector offsets.rs:241 used 0x26f21a0, wrong RTTI on the pinned PE.
pub const REJECTED_LEGACY_VTABLE_RVA: usize = 0x026f_21a0;
