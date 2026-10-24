//! Country-embedded radar intel source. It owns the coverage groups
//! described in `radar_group`.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0298_9178;
/// Static-intel-source secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0298_9198;
/// Coverage-group pointer vector.
pub const GROUPS: usize = 0x28;
/// Coverage-group vector count, u32.
pub const GROUPS_COUNT: usize = 0x34;
/// Raw owner country reference copied from the constructor country identity.
pub const COUNTRY: usize = 0x40;
/// Radar-pool constructor.
pub const CONSTRUCTOR_RVA: usize = 0x010a_2e00;
