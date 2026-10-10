//! CTheaterGroup, the named group inside a CTheatre.

/// A persistent named land-theatre group. The identity is the complete native
/// reference; `ordinal` is its actual position in its parent's pointer vector.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub priority: i32,
    pub name_start: usize,
    pub name_len: usize,
}

/// Native CTheaterGroup+48 membership, preserving the unfiltered vector order.
/// Both COrdersGroup and its CArmyGroup subclass use native type-53 handles.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub orders_group: u64,
    pub theater_group: u64,
    pub ordinal: u32,
}

/// CTheaterGroup vtable.
pub const VTABLE_RVA: usize = 0x029c_f050;
/// Theater-group reference type.
pub const REFERENCE_TYPE: u32 = 68;
/// CTheaterGroup packed type/id handle.
pub const HANDLE: usize = 0x08;
/// CTheaterGroup parent CTheatre pointer.
pub const THEATRE: usize = 0x18;
/// Signed i32 priority; assignment clamps it, while loading restores it directly.
pub const PRIORITY: usize = 0x20;
/// CTheaterGroup MSVC name string.
pub const NAME: usize = 0x28;
/// CTheaterGroup COrdersGroup pointer-vector data.
pub const ORDERS_GROUP_DATA: usize = 0x48;
/// CTheaterGroup orders-group u32 capacity.
pub const ORDERS_GROUP_CAPACITY: usize = 0x50;
/// CTheaterGroup orders-group u32 count.
pub const ORDERS_GROUP_COUNT: usize = 0x54;
/// CTheaterGroup allocation size.
pub const SIZE: usize = 0x60;
/// CTheaterGroup constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0162_7520;
/// Theater-group reference registration.
pub const REGISTER_REFERENCE_RVA: usize = 0x0162_7a10;
/// CTheaterGroup assignment.
pub const ASSIGN_RVA: usize = 0x0162_7740;
