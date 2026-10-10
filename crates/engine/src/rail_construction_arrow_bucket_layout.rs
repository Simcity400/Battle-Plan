//! Construction-arrow endpoint hash bucket.

/// Endpoint bucket stride.
pub const STRIDE: usize = 0x30;
/// Endpoint bucket occupancy, i8.
pub const OCCUPANCY: usize = 4;
/// Endpoint bucket empty marker.
pub const EMPTY: i8 = -1;
/// Canonical lower endpoint province ID, u32.
pub const LOWER_PROVINCE: usize = 8;
/// Canonical higher endpoint province ID, u32.
pub const HIGHER_PROVINCE: usize = 0x0c;
/// Selected endpoint entity pointer.
pub const ENTITY: usize = 0x10;
/// Lookup reads chain length u8 at current +0x34, which is next slot +4.
pub const NEXT_SLOT_CHAIN_LENGTH: usize = 0x34;
