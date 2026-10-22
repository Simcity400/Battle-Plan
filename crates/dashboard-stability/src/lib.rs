//! Stability: displayed stability, war support and maximum command power over a branch with the breakdown behind them, each part over time, and the daily country scans that feed their prepared series.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::country_stability`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod prepared_stability;

pub mod products {
    pub mod country_stability;
    pub mod stability_parts;

    dashboard_base::reply_warnings!(
        country_stability::CountryStabilityReply,
        stability_parts::StabilityPartsReply,
    );
}
