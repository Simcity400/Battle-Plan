//! Exact-a729 current-POV province visibility producer.
//!
//! The engine owns a complete byte vector indexed by province database ID.
//! This lane preserves those bytes verbatim. It deliberately does not invent
//! spotted-unit rows: the province-intel producer does not prove that separate
//! unit-culling domain.

pub use compact_contract::pov_visibility::{
    PovVisibilitySnapshot, CARRIER_MAGIC, CARRIER_VERSION, DOMAIN_ID, MAX_POV_COUNTRY_ID,
    MAX_PROVINCE_SLOTS,
};

#[cfg(windows)]
mod a729;
#[cfg(windows)]
pub use a729::{
    drain_latest, install, installed, installed_hook_receipts, take_released_hook_receipts,
    uninstall, PovVisibilityDrainError, PovVisibilityHookError, HOOK_RECEIPT_COUNT,
    PRODUCER_LOGICAL_NAME, PRODUCER_RVA,
};
