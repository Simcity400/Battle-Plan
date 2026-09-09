//! COccupationLaw, confirmed by the pinned executable's RTTI.
//! Preserve the string token itself; the old collector's FNV hash was a wire key.

/// COccupationLaw vtable; old D52 collector_module.rs:55, D66 build_profile.rs:270 and pinned RTTI agree.
pub const VTABLE_RVA: usize = 0x0292_8218;
/// COccupationLaw embedded MSVC string token; old D52 read_pdx_string and D66 read_occupation_law_pdx_string read size/capacity within it.
pub const NAME: usize = 0x10;
