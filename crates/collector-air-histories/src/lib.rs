
use collector_certificates::slicing;
use collector_certificates::watch_registry;
use collector_hooks::hook;
#[cfg(any(test, feature = "test-support"))]
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
use collector_timing::hook_timing;

pub mod air_histories;
pub mod history_changes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
