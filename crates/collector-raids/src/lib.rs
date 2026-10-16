//! Raid input capture: raid launch inputs and raid state inputs, read beside their daily slices for the owners a raid writer marked. The host owns dispatch, loans and table assembly.

use collector_certificates::slicing;
use collector_extended_support::outcomes::MISSING;
use collector_extended_support::outcomes::failed_capture;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::raid_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod raid_launch_inputs;
pub mod raid_state_inputs;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
