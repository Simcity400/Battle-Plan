//! Lexical capture: the game's interned token dictionary, its owned raw copy and conversion. The host owns dispatch, loans and table assembly.

use collector_memory::read_accounting;
#[cfg(any(test, feature = "test-support"))]
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
#[cfg(test)]
use collector_test_support as _;

pub mod lexical;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
