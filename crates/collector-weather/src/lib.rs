use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::country_input_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
#[cfg(test)]
use collector_test_support as _;

pub mod weather;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
