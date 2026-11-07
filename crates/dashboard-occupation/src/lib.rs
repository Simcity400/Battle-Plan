//! Occupied states and occupation trends: occupied states as recorded in prepared replay-block windows, and occupier means from the same preparation.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::occupied_states`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod occupation_reserve;
    pub mod occupation_trend;
    pub mod occupied_states;

    dashboard_base::reply_warnings!(
        occupation_reserve::OccupationReserveReply,
        occupied_states::OccupiedStatesReply,
        occupied_states::OccupiedStatesManifestReply,
        occupation_trend::OccupationTrendReply,
    );
}
