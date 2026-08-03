//! Exact-a729 current-POV land and task-force map visibility frames.

pub use compact_contract::spotted_units::{
    SpottedUnitHandle, SpottedUnitsSnapshot, CARRIER_MAGIC, CARRIER_VERSION, DOMAIN_ID,
    MAX_POV_COUNTRY_ID, MAX_ROWS_PER_SURFACE,
};

#[cfg(windows)]
mod a729;
#[cfg(windows)]
pub use a729::{
    abort_partial_install_and_uninstall, drain_latest, install, installed, installed_hook_receipts,
    take_released_hook_receipts, uninstall, HOOK_RECEIPT_COUNT, PRODUCTION_HOOK_SPECS,
};
