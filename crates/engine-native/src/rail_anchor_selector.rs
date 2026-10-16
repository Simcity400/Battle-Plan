//! Anchor-selector map reached through game-state +0x698.
//! Presentation layout; no hourly sampling lifetime is established.

/// Anchor-selector bucket pointer.
pub const BUCKETS: usize = 0x28;
/// Anchor-selector hash mask, u32.
pub const MASK: usize = 0x34;
/// Anchor-selector maximum probe distance, u8.
pub const MAX_PROBE: usize = 0x38;
/// Province-to-selector lookup routine.
pub const LOOKUP_RVA: usize = 0x00e6_9570;
/// Selector key hash multiplier, used twice with xor and right shift by 16.
pub const HASH_MULTIPLIER: u32 = 0x045d_9f3b;
/// Province-ID predicate: resolves this map, then tests naval-base +0x18 positive.
pub const HAS_POSITIVE_LEVEL_RVA: usize = 0x00e1_6b50;
/// Naval-base map rebuild using the game-state +0x698 owner.
pub const REBUILD_COUNTRY_PROVINCES_RVA: usize = 0x00e9_6170;
