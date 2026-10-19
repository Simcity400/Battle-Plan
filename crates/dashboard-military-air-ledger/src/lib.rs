//! Military air ledger: exact aircraft losses and bombing damage by country.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::military_air_ledger`) and names
//! what it uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod military_air_ledger;

    dashboard_base::reply_warnings!(military_air_ledger::MilitaryAirReply,);
}
