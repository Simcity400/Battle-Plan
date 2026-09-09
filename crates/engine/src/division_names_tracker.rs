//! Country division-name tracker; the old walk did not authenticate a C++ RTTI name.

/// Division-name tracker's post-mortem vector count u32; old read_country_divisions_lost reads *(CCountry+0x70)+0x54.
pub const POST_MORTEM_COUNT: usize = 0x54;
