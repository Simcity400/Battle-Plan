//! CAirTheatre layout; hourly consistency is unproven.

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

/// Air-theatre primary vtable.
pub const VTABLE_RVA: usize = 0x0286_4770;
/// Reference type checked against the handle.
pub const TYPE: u32 = 77;
/// Full handle containing u32 type and id.
pub const HANDLE: usize = 0x08;
/// Raw country reference copied during construction and restored by loading.
pub const COUNTRY: usize = 0x18;
/// Literal name string.
pub const NAME: usize = 0x20;
/// Child CAirGroup pointer vector requiring matching parent references.
pub const GROUPS: usize = 0x40;
/// Group-vector count, u32; occurrence order is preserved.
pub const GROUPS_COUNT: usize = 0x4c;
