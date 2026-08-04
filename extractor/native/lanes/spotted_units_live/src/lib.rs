//! Exact-a729 current-POV land and task-force map visibility frames.

pub use compact_contract::spotted_units::{
    SpottedUnitHandle, SpottedUnitsSnapshot, CARRIER_MAGIC, CARRIER_VERSION, DOMAIN_ID,
    MAX_POV_COUNTRY_ID, MAX_ROWS_PER_SURFACE,
};

#[cfg(windows)]
mod a729;
#[cfg(windows)]
pub use a729::{
    abort_partial_install_and_uninstall, census_units, drain_latest, install, installed,
    installed_hook_receipts, release_pending, site_entry_counts, take_released_hook_receipts,
    uninstall, CENSUS_ABANDONED_PENDING, CENSUS_ADMITTED, CENSUS_CANDIDATES_SEEN, CENSUS_FRAMES_WITHOUT_ENUMERATION,
    CENSUS_REJECTED_BELOW_INTEL_THRESHOLD, CENSUS_REJECTED_BY_ELIGIBILITY,
    CENSUS_REJECTED_TASKFORCE_STATE, CENSUS_REJECTED_UNREADABLE_HANDLE, CENSUS_SURFACES,
    CENSUS_SURFACE_LAND, CENSUS_SURFACE_TASKFORCE, CENSUS_UNIT_COUNT, HOOK_RECEIPT_COUNT,
    PRODUCTION_HOOK_SPECS,
};
