
use collector_memory::read_accounting;
use collector_primitives::{clock, country_slots as country_scale, raw_memory, table_capture};
use collector_projection::{projection, table_conversion};
use collector_raw::raw;
#[cfg(test)]
use collector_test_support as _;

pub mod economy;
pub mod fuel_demand;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
