//! Between-tick native writer observers that several capture families share: country inputs and the families forwarding to them, rosters, characters, advisor pools, opinions, output and event ledgers, nuclear strikes, wars, naval trees and leaders, retreat bans, raids, air base carry, rail, territory, the event feed with its history and war provenance, political status, world inputs, wings, air losses and building bombing. One patch owner per native site forwards to the others. The host installs the hooks, drains the stores at the hourly boundary and composes the capture families that read them.

use collector_certificates::{dirty_set, sampled_verify, watch_registry};
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_memory::terminal as allocation;
use collector_primitives::{clock, raw_memory, table_capture};
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
pub mod country_input_changes;
pub mod event_feed_history;
pub mod event_feed_observer;
pub mod event_ledger;
pub mod naval_tree_changes;
pub mod nuclear_strikes;
pub mod opinion_changes;
pub mod output_log;
pub mod political_status_changes;
pub mod province_changes;
pub mod raid_changes;
pub mod rail_changes;
pub mod retreat_ban_changes;
pub mod roster_changes;
pub mod territory_changes;
pub mod unit_leader_changes;
pub mod unit_leader_trait_changes;
pub mod war_changes;
pub mod war_provenance;
pub mod wing_changes;
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
