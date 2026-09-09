//! Content-map node payload; descriptive name, not a recovered RTTI class name.

/// Content map's std::string key; engine_modset_provenance::read_map reads node+0x20.
pub const KEY: usize = 0x20;
/// Inline content value follows the string key; engine_modset_provenance::read_map reads node+0x40.
pub const VALUE: usize = 0x40;
