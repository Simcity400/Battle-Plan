//! Occupation table bucket specialization, with u32 keys and pointer values.
//! An empty bucket can retain a stale value pointer. Membership comes from the distance byte.

/// Occupation robin-hood bucket stride; old build_profile.rs:264 and compact_contract/occupation_robin_hood.rs:98-100 use 24 bytes.
pub const STRIDE: usize = 0x18;
/// Occupation robin-hood bucket distance-plus-one byte; old build_profile.rs:265 and the engine iterator's occupancy test.
pub const DISTANCE_PLUS_ONE: usize = 0x04;
/// Occupation robin-hood bucket u32 key; old build_profile.rs:266 and read_robin_hood_table read +8.
pub const KEY: usize = 0x08;
/// Occupation robin-hood bucket pointer value; old build_profile.rs:267 and read_robin_hood_table read +0x10.
pub const VALUE: usize = 0x10;
/// Occupation robin-hood bucket empty marker; old read_robin_hood_table skips distance-plus-one zero even when value is nonzero.
pub const EMPTY_DISTANCE: u8 = 0;
/// Occupation robin-hood iterator terminator byte; old build_profile.rs:268 and read_robin_hood_table reject 0xff inside the live span.
pub const ITERATOR_SENTINEL_DISTANCE: u8 = 0xff;
