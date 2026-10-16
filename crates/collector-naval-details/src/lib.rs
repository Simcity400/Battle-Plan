//! Naval combat capture: live naval combat details, their modifier trees, result details and combat sources, read in the ordinary naval part. The host owns the part's dispatch, loans and table assembly.

use collector_certificates::sampled_verify;
use collector_combat::combat_details;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::naval_tree_changes;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;

pub mod naval_combat_sources;
pub mod naval_details;
pub mod naval_result_details;
pub mod naval_trees;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
