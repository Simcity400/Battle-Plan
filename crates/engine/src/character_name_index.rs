//! Embedded character-manager name index. An empty or stale index cannot
//! establish roster completeness; the two manager vectors remain authoritative.

/// Name-index bucket pointer.
pub const BUCKETS: usize = 0x08;
/// Name-index populated count u32.
pub const COUNT: usize = 0x10;
/// Name-index mask u32.
pub const MASK: usize = 0x14;
/// Name-index maximum probe u8.
pub const MAXIMUM_PROBE: usize = 0x18;
/// Name-index image-owned empty buckets.
pub const EMPTY_BUCKETS_RVA: usize = 0x0306_f1c8;
/// Name-index u32 hash multiplier, applied twice.
pub const HASH_MULTIPLIER: u32 = 0x045d_9f3b;
