//! Land battles and the division log: battle manifests, windows, rankings, courses and terrain, and each division's combat history.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports them, registers them and prepares
//! and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod army_losses;
    pub mod combat_log;
    pub mod division_log;
    pub mod land_battles;
    pub mod reached;
}
