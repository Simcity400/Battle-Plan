//! CNavyTheaterGroup, called a navy theatre by the old selector.

/// CNavyTheaterGroup primary vtable; old NAVY_THEATRE_VTABLE_RVA, pinned RTTI confirms actual class.
pub const VTABLE_RVA: usize = 0x029c_60f0;
/// CNavyTheaterGroup reference handle {u32 type, u32 id}; theatre_selector::navy reference_id read.
pub const HANDLE: usize = 0x08;
/// CNavyTheaterGroup CFleet pointer array; old theatre_selector::navy uses offsets.rs:985.
pub const FLEETS: usize = 0x20;
/// CNavyTheaterGroup fleet array capacity, u32; old offsets.rs:986 and theatre_selector::navy.
pub const FLEET_CAPACITY: usize = 0x28;
/// CNavyTheaterGroup fleet array count, u32; old offsets.rs:987 and theatre_selector::navy.
pub const FLEET_COUNT: usize = 0x2c;
/// CNavyTheaterGroup runtime name string; old offsets.rs:988 and theatre_selector::navy.
pub const NAME: usize = 0x38;
/// CNavyTheaterGroup priority ordinal, u8; old offsets.rs:1033 and theatre_selector::navy.
pub const PRIORITY_ORDINAL: usize = 0x58;
