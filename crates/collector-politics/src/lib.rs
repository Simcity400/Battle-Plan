
use collector_hooks::hook;
use collector_observers::country_input_changes;
use collector_observers::event_feed_observer;
use collector_observers::naval_tree_changes;
use collector_observers::political_status_changes;
use collector_observers::script_event_observer;
use collector_observers::war_provenance;
use collector_primitives::clock;
use collector_primitives::shared_lane;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;
use collector_timing::hook_timing;

pub mod politics;
pub mod politics_changes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
