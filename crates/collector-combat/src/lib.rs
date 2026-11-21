
use collector_memory::{read_accounting, terminal as allocation};
use collector_primitives::{clock, country_slots as country_scale, table_capture};
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod combat_details;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
