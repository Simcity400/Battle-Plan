//! CFEXGroup, harvested from the old anchored naval combat walk.
//! Forward side vectors can omit groups; the old reader followed +0x20 rings
//! and authenticated ownership through +0x38. That is not an hourly proof.

/// CFEXGroup vtable; old collector matched .?AVCFEXGroup@@ RTTI.
pub const VTABLE_RVA: usize = 0x02a4_2d18;
/// CFEXGroup CFEXMember pointer array; old naval group walk reads +0x08.
pub const MEMBERS: usize = 0x08;
/// CFEXGroup member capacity u32; old naval group header validation reads +0x10.
pub const MEMBERS_CAPACITY: usize = 0x10;
/// CFEXGroup member count u32; old naval group header validation reads +0x14.
pub const MEMBERS_COUNT: usize = 0x14;
/// CFEXGroup next ring node pointer; old naval_ring_closure_into follows +0x20.
pub const NEXT: usize = 0x20;
/// CFEXGroup owning CNavalCombatant pointer; old walk rejects mismatching +0x38.
pub const SIDE: usize = 0x38;
