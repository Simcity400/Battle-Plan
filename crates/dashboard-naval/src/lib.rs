//! Naval battles, naval losses and ships sunk, built on one retained sinking scan.
//!
//! One product crate above `dashboard-base`: the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text, dashboard-core
//! and dashboard-land-battles); the dashboard re-exports them, registers them
//! and prepares and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod naval_battles;
    pub mod naval_losses;
    pub mod ships_sunk;
    pub mod sinkings;

    dashboard_base::reply_warnings!(naval_losses::NavalLossesReply, ships_sunk::ShipsSunkReply,);
}
