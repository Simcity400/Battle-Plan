
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_observers::output_log;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
#[cfg(feature = "callback_profile")]
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;
use collector_timing::hook_timing;

pub mod fuel_awards;
pub mod tables;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
