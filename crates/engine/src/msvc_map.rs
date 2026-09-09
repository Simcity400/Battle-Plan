//! MSVC std::map header used by content and other engine trees.

pub const HEAD: usize = 0;
/// MSVC std::map live node count, u64; old engine_modset_provenance::read_map confirms this field.
pub const COUNT: usize = 8;
