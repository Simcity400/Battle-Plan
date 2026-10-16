//! Subject-country peace participant identity.
/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029a_b9e8;
/// The subject relation uses +58 as overlord and +60 as the selected country,
/// which may have a newly allocated reference.
pub const OVERLORD: usize = 0x58;
pub const RESULT_COUNTRY: usize = 0x60;
