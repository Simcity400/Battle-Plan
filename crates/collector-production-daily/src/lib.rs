//! Daily production slot capture: industrial organisations and the country and production line modifiers, read in census slot ranges. The host owns dispatch, the range split and table assembly.

use collector_extended_support::outcomes::failed_capture;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod industrial_organisations;
pub mod production_modifiers;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
