//! Session and map name capture: the session's world tension tables and dynamic modifiers, and province names and naval region access, published from their held reads while no world input writer ran. The host owns dispatch, loans and table assembly.

#[cfg(test)]
use collector_certificates::sampled_verify;
use collector_extended_support::outcomes::failed_capture;
use collector_memory::read_accounting;
use collector_observers::world_input_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
#[cfg(test)]
use collector_test_support as _;

pub mod map_names;
pub mod session;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
