//! CCountryDatabase tag strings and source paths; IDs here are canonical slots, not raw country links.

/// Country-database singleton pointer.
pub const SINGLETON_RVA: usize = 0x0333_0d98;
/// Tag-string array.
pub const TAGS: usize = 0;
/// First dynamic canonical slot, u32, set to tag count at the marker.
pub const DYNAMIC_TAG_BOUNDARY: usize = 0x88;
