//! CFEXGroup, harvested from the old anchored naval combat walk.
//! Forward side vectors can omit groups; +0x20 points to the paired opposing
//! group, while +0x38 authenticates the owning side.

/// CFEXGroup vtable; old collector matched .?AVCFEXGroup@@ RTTI.
pub const VTABLE_RVA: usize = 0x02a4_2d18;
/// CFEXGroup CFEXMember pointer array; old naval group walk reads +0x08.
pub const MEMBERS: usize = 0x08;
/// CFEXGroup member capacity u32; old naval group header validation reads +0x10.
pub const MEMBERS_CAPACITY: usize = 0x10;
/// CFEXGroup member count u32; old naval group header validation reads +0x14.
pub const MEMBERS_COUNT: usize = 0x14;
pub const OPPONENT_GROUP: usize = 0x20;
/// CFEXGroup owning CNavalCombatant pointer; old walk rejects mismatching +0x38.
pub const SIDE: usize = 0x38;
