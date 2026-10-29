//! CAirGroup layout; hourly consistency is unproven.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub theatre: u64,
    /// Actual position in the parent CAirTheatre's native group vector.
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
}

/// Exact air-group reference-vector entry. No filter or presentation order is substituted for the
/// native position. Reverse wing ownership must be checked against the wing's group reference when
/// joining this table to the wing census.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub wing: u64,
    pub group: u64,
    pub ordinal: u32,
}

/// Air-group primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_6300;
/// Reference type checked against the handle.
pub const TYPE: u32 = 78;
/// Full handle containing u32 type and id.
pub const HANDLE: usize = 0x08;
/// Literal name string.
pub const NAME: usize = 0x50;
/// Full parent CAirTheatre reference checked for equality.
pub const THEATRE_HANDLE: usize = 0x70;
/// Vector of full raw CAirWing references.
pub const WINGS: usize = 0x78;
