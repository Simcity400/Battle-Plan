
use collector_certificates::{dirty_set, watch_registry};
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_memory::terminal as allocation;
#[cfg(test)]
use collector_primitives::table_capture;
use collector_primitives::{clock, raw_memory};
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;
use collector_timing::hook_timing;

pub mod advisor_pool_changes;
pub mod air_base_carry_changes;
pub mod air_losses;
pub mod area_changes;
pub mod building_bombing;
pub mod census_changes;
pub mod character_changes;
pub mod combat_engagement_refreshes;
pub mod country_input_changes;
pub mod doctrine_changes;
pub mod crew_changes;
pub mod event_feed_history;
pub mod event_feed_observer;
pub mod event_window_observer;
pub mod event_ledger;
pub mod market_changes;
pub mod fleet_region_changes;
pub mod naval_tree_changes;
pub mod nuclear_strikes;
pub mod opinion_changes;
pub mod orders_group_changes;
pub mod order_membership_changes;
pub mod occupation_changes;
pub mod output_log;
pub mod political_status_changes;
pub mod project_variable_changes;
pub mod province_changes;
pub mod province_combat_changes;
pub mod raid_changes;
pub mod rail_changes;
pub mod retreat_ban_changes;
pub mod roster_changes;
pub mod script_event_observer;
pub mod territory_changes;
pub mod unit_history_changes;
pub mod unit_leader_changes;
pub mod unit_leader_trait_changes;
pub mod war_changes;
pub mod war_provenance;
pub mod wing_changes;
pub mod wing_preferences;
pub mod world_input_changes;

#[cfg(all(test, feature = "tap-foreign-probe"))]
#[path = "../../../tools/tap-foreign-unwind/runtime.rs"]
mod foreign_unwind_probe;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

pub mod wargoal_changes;
pub mod division_panel_changes;
pub mod variant_stat_witness;

fn verified(base: usize, key: engine_sites::Rva) -> Result<(), &'static str> {
    engine_sites::hook(base, key)?.sentinel::<16>().map(drop)
}

fn located(base: usize, key: engine_sites::Rva) -> Result<(), &'static str> {
    verified(base, key).inspect_err(|cause| {
        hook::installation::refused(hook::installation::Kind::Sentinel, None, cause);
    })
}
