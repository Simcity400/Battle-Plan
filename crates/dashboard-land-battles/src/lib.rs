//! Land battles and the division log: battle manifests, windows, rankings, courses and terrain, and each division's combat history.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports them, registers them and prepares
//! and queries them.

pub mod products {
    pub mod division_log;
    pub mod land_battles;
}
