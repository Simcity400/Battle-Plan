//! CFEXGroup layout. Forward side vectors can omit groups; +0x20 links
//! the opposing group and +0x38 authenticates the owning side.

/// CFEXGroup vtable.
pub const VTABLE_RVA: usize = 0x02a4_2d18;
/// CFEXGroup CFEXMember pointer array.
pub const MEMBERS: usize = 0x08;
/// CFEXGroup member capacity u32.
pub const MEMBERS_CAPACITY: usize = 0x10;
/// CFEXGroup member count u32.
pub const MEMBERS_COUNT: usize = 0x14;
/// CFEXGroup opposing group pointer.
pub const OPPONENT_GROUP: usize = 0x20;
/// Group owning combat-side pointer, authenticated against the reached side.
pub const SIDE: usize = 0x38;
