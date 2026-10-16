//! Naval battles, naval losses and ships sunk, built on one retained sinking scan.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::<name>`) and name what they
//! use at the crates that own it (dashboard-base, dashboard-text, dashboard-core
//! and dashboard-land-battles); the dashboard re-exports them, registers them
//! and prepares and queries them.

pub mod products {
    pub mod naval_battles;
    pub mod naval_losses;
    pub mod ships_sunk;
    pub mod sinkings;

    dashboard_base::reply_warnings!(naval_losses::NavalLossesReply, ships_sunk::ShipsSunkReply,);
}
