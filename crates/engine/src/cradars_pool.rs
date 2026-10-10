//! Country-embedded radar intel source. It owns the coverage groups
//! described in `radar_group`.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0297_2548;
/// Static-intel-source secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0297_2568;
/// Coverage-group pointer vector.
pub const GROUPS: usize = 0x28;
/// Coverage-group vector capacity, u32, checked before growth.
pub const GROUPS_CAPACITY: usize = 0x30;
/// Coverage-group vector count, u32.
pub const GROUPS_COUNT: usize = 0x34;
/// Raw owner country reference copied from the constructor country identity.
pub const COUNTRY: usize = 0x40;
/// Radar-pool constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0108_db50;
/// Group clear releasing each group's three arrays.
pub const CLEAR_GROUPS_RVA: usize = 0x0108_f360;
/// Radar intel update.
pub const UPDATE_INTEL_RVA: usize = 0x0109_46f0;
