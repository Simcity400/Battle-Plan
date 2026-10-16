//! Railway gun capture: hourly railway gun counter inputs, route cells, stored names and ETA operands, with the name sizing hints the sizing polls raise. The host owns dispatch, loans and table assembly.

use collector_division_parts::supply_inputs;
use collector_division_parts::unit_combat_references;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;

pub mod railway_guns;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
