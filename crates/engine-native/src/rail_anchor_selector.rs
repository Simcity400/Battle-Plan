//! Anchor-selector map reached through the game state's strategic navy manager. Presentation
//! layout; no hourly sampling lifetime is established.

/// Anchor-selector bucket pointer.
pub const BUCKETS: usize = 0x28;
/// Anchor-selector hash mask, u32.
pub const MASK: usize = 0x34;
/// Anchor-selector maximum probe distance, u8.
pub const MAX_PROBE: usize = 0x38;
/// Selector key hash multiplier, used twice with xor and right shift by 16.
pub const HASH_MULTIPLIER: u32 = 0x045d_9f3b;
