//! CNavyTheaterGroup layout.

/// CNavyTheaterGroup primary vtable.
pub const VTABLE_RVA: usize = 0x029c_60f0;
/// Full navy-theatre reference handle: u32 type and u32 id.
pub const HANDLE: usize = 0x08;
/// CNavyTheaterGroup CFleet pointer array.
pub const FLEETS: usize = 0x20;
/// CNavyTheaterGroup fleet array capacity, u32.
pub const FLEET_CAPACITY: usize = 0x28;
/// CNavyTheaterGroup fleet array count, u32.
pub const FLEET_COUNT: usize = 0x2c;
/// CNavyTheaterGroup runtime name string.
pub const NAME: usize = 0x38;
/// CNavyTheaterGroup priority ordinal, u8.
pub const PRIORITY_ORDINAL: usize = 0x58;
