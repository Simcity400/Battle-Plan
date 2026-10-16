//! Hired advisors capture: appointed and available advisors, advisor tokens, the country character records and their held walks. The host owns dispatch, loans and table assembly.

use collector_certificates::sampled_verify;
use collector_certificates::slicing;
use collector_extended_support::outcomes::failed_capture;
use collector_memory::read_accounting;
use collector_observers::advisor_pool_changes;
use collector_observers::character_changes;
use collector_observers::roster_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod hired_advisors;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
