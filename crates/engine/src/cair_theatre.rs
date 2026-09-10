//! CAirTheatre, pinned a729; harvested layout only, hourly consistency unproven.

/// Persistent theatre identity and literal name, without selector presentation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    /// Actual position in the owning country's native theatre pointer vector.
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
}

/// CAirTheatre primary vtable; executable RTTI confirms old offsets.rs AIR_THEATRE_VTABLE_RVA.
pub const VTABLE_RVA: usize = 0x027d_0ef0;
/// CAirTheatre reference type; checked by collector theatre_selector::air against its +8 handle.
pub const TYPE: u32 = 77;
/// CAirTheatre paired u32 type/id handle; collector theatre_selector::reference_id reads a qword here.
pub const HANDLE: usize = 0x08;
pub const COUNTRY: usize = 0x18;
/// CAirTheatre std::string name; collector theatre_selector::air reads offsets.rs AIR_THEATRE_OFF_NAME.
pub const NAME: usize = 0x20;
/// CAirTheatre CAirGroup* vector data; collector theatre_selector::air validates child back-links.
pub const GROUPS: usize = 0x40;
/// CAirTheatre group-vector u32 capacity; collector theatre_selector::air checks the header.
pub const GROUPS_CAPACITY: usize = 0x48;
/// CAirTheatre group-vector u32 count; collector theatre_selector::air preserves engine order.
pub const GROUPS_COUNT: usize = 0x4c;
