//! Occupation table bucket specialization, with u32 keys and pointer values.
//! An empty bucket can retain a stale value pointer. Membership comes from the distance byte.

/// Occupation robin-hood bucket stride.
pub const STRIDE: usize = 0x18;
/// Bucket distance-plus-one byte selecting occupancy.
pub const DISTANCE_PLUS_ONE: usize = 0x04;
/// Bucket u32 key.
pub const KEY: usize = 0x08;
/// Bucket pointer value.
pub const VALUE: usize = 0x10;
/// Empty marker; a zero distance skips even a nonzero stale value.
pub const EMPTY_DISTANCE: u8 = 0;
/// Iterator terminator byte, refused inside the live span.
pub const ITERATOR_SENTINEL_DISTANCE: u8 = 0xff;
