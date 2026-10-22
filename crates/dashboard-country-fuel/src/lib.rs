//! Country fuel: each country's fuel stock, gains, consumption by use and demand, with the awards behind each change.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::country_fuel`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod country_fuel;

    dashboard_base::reply_warnings!(
        country_fuel::FuelReply,
        country_fuel::FuelAwardsReply,
        country_fuel::FuelAwardHistoryReply,
        country_fuel::FuelConsumptionReply,
        country_fuel::FuelDemandReply,
        country_fuel::FuelDemandHistoryReply,
        country_fuel::FuelDemandStackReply,
    );
}
