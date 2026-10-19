//! Wars and war casualties: each war's sides, participants, names and outcome, and the casualties each country took and dealt, by pair and by domain.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::wars`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod war_casualties;
    pub mod wargoals;
    pub mod wars;

    dashboard_base::reply_warnings!(
        wars::WarsReply,
        wargoals::WargoalsReply,
        war_casualties::CasualtyReply,
        war_casualties::pairs::CasualtyPairsReply,
        war_casualties::breakdown::CasualtyBreakdownReply,
    );
}
