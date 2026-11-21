
#[cfg(test)]
use collector_extended_support::outcomes::MISSING;
use collector_extended_support::outcomes::truncated_source;
use collector_memory::read_accounting;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_production::production;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod army_icon_inputs;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
