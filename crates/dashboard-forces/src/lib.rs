//! Force sizes: army size, supply health, air force and navy by country, with the parts-race families that prepare and query them.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports them, registers them and prepares
//! and queries them.

pub mod products {
    pub mod air_force;
    pub mod army_size;
    pub mod navy;
    pub mod parts_race_prepared;
    pub mod supply_health;

    dashboard_base::reply_warnings!(
        army_size::ArmySizeReply,
        supply_health::SupplyHealthReply,
        air_force::AirForceReply,
        navy::NavyReply,
    );
}
