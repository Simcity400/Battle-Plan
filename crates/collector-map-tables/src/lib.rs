//! Ordinary map part tables: the battle-sound list, map facilities, naval bases and saved event targets. The host owns the ordinary map part's dispatch, loans and table assembly.

use collector_memory::read_accounting;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod battle_sounds;
pub mod map_facilities;
pub mod naval_bases;
pub mod saved_event_targets;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
