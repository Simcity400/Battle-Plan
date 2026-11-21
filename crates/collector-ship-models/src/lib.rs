
use collector_certificates::watch_registry;
use collector_extended_support::outcomes::MISSING;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::air_base_carry_changes;
use collector_observers::naval_tree_changes;
use collector_observers::unit_history_changes;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::projection;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

mod air_capacity;
pub mod ship_model_changes;
pub mod ship_models;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
