//! CCountryDatabase tag strings and source paths; IDs here are canonical slots, not raw country links.

/// Country-database singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_89c8;
/// Tag-string array.
pub const TAGS: usize = 0;
/// Country-file-path array growing at capacity.
pub const COUNTRY_FILE_PATHS: usize = 0x18;
/// First dynamic canonical slot, u32, set to tag count at the marker.
pub const DYNAMIC_TAG_BOUNDARY: usize = 0x88;
/// Tag loader.
pub const LOAD_TAGS_RVA: usize = 0x0070_e900;
/// Canonical tag lookup bounded by the live count.
pub const TAG_LOOKUP_RVA: usize = 0x0070_f170;
/// Dynamic-country allocator.
pub const ALLOCATE_DYNAMIC_COUNTRY_RVA: usize = 0x001d_a850;
/// Tag-loader marker token selecting the dynamic boundary.
pub const DYNAMIC_TAG_MARKER: u32 = 0x309f;
