//! Ordinary country capture: country appearance, country counter inputs and the reserved visibility both read. The host owns the ordinary job, the navy pass installation, loans and table assembly.

use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_naval_movement::country_naval_movement_inputs;
use collector_observers::country_input_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;

pub mod country_appearance;
pub mod country_counters;
pub mod reserved_visibility;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
