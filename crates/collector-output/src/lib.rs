
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_memory::terminal as allocation;
use collector_observers::opinion_changes;
use collector_observers::output_log;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod construction_ic;
pub mod construction_tables;
pub mod line_output;
pub mod line_tables;
pub mod output_continuity;
pub mod output_hooks;
pub mod rocket_output;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
