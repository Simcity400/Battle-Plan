//! Air mission embedded in a wing; all offsets are relative to the mission
//! subobject.

/// Mission primary vtable.
pub const VTABLE_RVA: usize = 0x0297_c8d0;

/// Enabled mission mask, u32, including the training bit.
pub const ENABLED_MASK: usize = 0x10;
/// Current type or mask, i32; training clears it independently of the enabled mask.
pub const CURRENT_TYPE: usize = 0x14;
/// Effective aircraft retained by an admitted simulation mission update, i32.
pub const EFFECTIVE_AIRCRAFTS: usize = 0xe0;
/// Saved active byte required by regional missions outside the training path.
pub const ACTIVE: usize = 0x20;
/// Context-mode byte; two is unrestricted, zero and one select complementary gates.
pub const CONTEXT_MODE: usize = 0x18;
/// Capacity class, i32, indexing the global threshold table.
pub const CAPACITY_CLASS: usize = 0x1c;
/// Candidate wing pointer used by capacity and training-base gates.
pub const WING: usize = 0x28;
/// Strategic-region pointer.
pub const REGION: usize = 0x30;
/// Reached-province array data; each 16-byte entry holds province ID and distance.
/// Target scans test membership; scenario grouping requires a nonzero count.
pub const REACHED_PROVINCES_DATA: usize = 0x38;
#[cfg(any(test, feature = "test-support"))]
pub const REACHED_PROVINCES_CAPACITY: usize = 0x40;
#[cfg(any(test, feature = "test-support"))]
pub const REACHED_PROVINCES_COUNT: usize = 0x44;
/// Last target province, zero when absent; tested for transport, scout and bomber flights.
pub const LAST_TARGET_PROVINCE: usize = 0x70;
/// Training mission bit.
pub const TRAINING_MASK: u32 = 0x800;
/// Mask-to-bit-index helper used for mission fuel costs.
pub const MISSION_BIT_INDEX_RVA: usize = 0x0101_3ee0;
