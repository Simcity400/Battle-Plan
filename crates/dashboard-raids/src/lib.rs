//! Raids: observed raid launches and results, as a log and a tally by country.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::raids`) and names what it uses
//! at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod raids;

    dashboard_base::reply_warnings!(raids::RaidsReply,);
}
