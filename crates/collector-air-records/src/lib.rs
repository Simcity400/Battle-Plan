
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_memory::read_accounting;
use collector_primitives::clock;
use collector_primitives::table_hash;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod air_record_changes;
pub mod air_records;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
