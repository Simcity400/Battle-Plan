//! Projects and raids capture: special projects and their history, raids, raid paths, facilities and counter-operatives, raid target statuses and the raid intel carry. The host owns dispatch, loans and table assembly.

use collector_certificates::slicing;
use collector_extended_support::outcomes::RaidReadCounts;
use collector_extended_support::outcomes::failed_capture;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::country_input_changes;
use collector_observers::naval_tree_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;

pub mod projects;
pub mod raid_intel_carry;
pub mod raid_target_statuses;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
