//! Country political state: elections, governments in exile and balances of
//! power at one hour, and the balance-of-power history.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::politics_state`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries its history.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod politics_state;

    dashboard_base::reply_warnings!(
        politics_state::PoliticsStateReply,
        politics_state::history::PoliticsHistoryReply,
        politics_state::civil_wars::CivilWarsReply,
    );
}
