//! Force sizes: army size, supply health, air force and navy by country.
//!
//! One product crate above `dashboard-base`: the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports them, registers them and prepares
//! and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod air_force;
    pub mod army_size;
    #[cfg(any(test, feature = "test-support"))]
    pub mod force_fixtures;
    pub mod naval_management;
    pub mod navy;
    pub mod supply_health;
    pub mod volunteers_expeditionaries;

    dashboard_base::reply_warnings!(
        army_size::ArmySizeReply,
        supply_health::SupplyHealthReply,
        air_force::AirForceReply,
        navy::NavyReply,
        naval_management::NavalManagementReply,
        volunteers_expeditionaries::VolunteersExpeditionariesReply,
    );
}
