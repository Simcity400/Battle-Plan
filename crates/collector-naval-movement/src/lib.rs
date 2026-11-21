
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
#[cfg(test)]
use collector_test_support as _;

pub mod country_naval_movement_inputs;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
