//! Anonymous path-index specialization embedded at texture-resource manager
//! +0x28. Its receiver is distinct from the manager; concrete C++ name unknown.

/// Byte-exact path lookup for resource registration and removal.
pub const LOOKUP_RVA: usize = 0x002c_6f30;
/// Path-index bucket array pointer.
pub const BUCKETS: usize = 8;
/// Path-index occupied count, i32.
pub const COUNT: usize = 0x10;
/// Path-index bucket mask, u32.
pub const MASK: usize = 0x14;
/// Path-index maximum probe distance, u8.
pub const MAX_PROBE_DISTANCE: usize = 0x18;
