//! Naval counter capture: task force counter inputs and supply, carried under the naval tree observer's certificates. The host owns the ordinary task force part's dispatch, loans and table assembly.

use collector_certificates::sampled_verify;
#[cfg(test)]
use collector_certificates::watch_registry;
use collector_division_parts::supply_inputs;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::naval_tree_changes;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;

pub mod naval_counters;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
