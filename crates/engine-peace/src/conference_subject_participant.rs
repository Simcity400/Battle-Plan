//! Subject-country peace participant identity.
/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a3_c228;
/// The subject relation uses its overlord word as overlord and the following word as the selected
/// country, which may have a newly allocated reference.
pub const OVERLORD: usize = 0x58;
pub const RESULT_COUNTRY: usize = 0x60;
