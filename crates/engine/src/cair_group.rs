//! CAirGroup, pinned a729; harvested layout only, hourly consistency unproven.

/// CAirGroup primary vtable; executable RTTI confirms old offsets.rs AIR_GROUP_VTABLE_RVA.
pub const VTABLE_RVA: usize = 0x029b_6340;
/// CAirGroup reference type; checked by collector theatre_selector::air against its +8 handle.
pub const TYPE: u32 = 78;
/// CAirGroup paired u32 type/id handle; collector theatre_selector::reference_id reads a qword here.
pub const HANDLE: usize = 0x08;
/// CAirGroup std::string name; collector theatre_selector::air reads offsets.rs AIR_GROUP_OFF_NAME.
pub const NAME: usize = 0x50;
/// CAirGroup parent CAirTheatre paired-u32 reference; collector theatre_selector::air checks equality.
pub const THEATRE_HANDLE: usize = 0x70;
/// CAirGroup vector of paired-u32 CAirWing references; collector theatre_selector::air reads raw qwords.
pub const WINGS: usize = 0x78;
/// CAirGroup wing-vector u32 capacity; collector theatre_selector::air checks the header.
pub const WINGS_CAPACITY: usize = 0x80;
/// CAirGroup wing-vector u32 count; collector theatre_selector::air preserves engine order.
pub const WINGS_COUNT: usize = 0x84;
