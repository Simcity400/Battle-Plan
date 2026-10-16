//! Construction capture: the construction rest's owned raw copy, buildings and their held building store, and the building writer observer that forwards to the territory, raid, rail and nuclear strike observers. The host owns dispatch, the World job's construction part, loans and table assembly.

use collector_certificates::dirty_set;
use collector_certificates::sampled_verify;
use collector_certificates::watch_registry;
use collector_country_counters::country_counters;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::raid_changes;
use collector_observers::rail_changes;
use collector_observers::territory_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod building_changes;
pub mod construction;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
