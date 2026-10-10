//! Intel globals shared by country and air disclosure calculations; no hard-coded mod-overridable values.

/// Live country-intel maximums, stored as four contiguous i64 values.
pub const LEVEL_DENOMINATORS_RVA: usize = 0x0332_0148;
/// Fog-enabled byte; zero disables fog.
pub const FOG_ENABLED_RVA: usize = 0x0331_726a;
/// NIntel.AIR_MIN_INTEL_TO_SHOW_AIRWING_CLASSES i64 fixed5.
pub const AIR_DISCLOSURE_THRESHOLD_RVA: usize = 0x0331_b588;
/// Intel-network controlled-state weight, native i64.
pub const NETWORK_WEIGHT_CONTROLLED_RVA: usize = 0x0331_dc68;
/// Intel-network owned-state weight, native i64.
pub const NETWORK_WEIGHT_OWNED_RVA: usize = 0x0331_dd10;
/// Intel-network owned-state-worth weight, native i64.
pub const NETWORK_WEIGHT_OWNED_WORTH_RVA: usize = 0x0331_ddc8;
/// Live map-intel visibility cutoff storage.
pub const MAP_INTEL_VISIBILITY_CUTOFFS_RVA: usize = 0x0332_02b0;
/// Map-intel visibility cutoff loader.
pub const LOAD_MAP_INTEL_VISIBILITY_CUTOFFS_RVA: usize = 0x0088_94e0;
