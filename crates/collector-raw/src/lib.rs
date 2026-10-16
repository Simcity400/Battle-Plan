//! Ordered owned raw-copy spans, typed decoding over those bytes, occurrence-aware lookup and prepared copy plans. Captured reads preserve exact addresses, order, duplicates, faults and table ownership. No deferred reader follows live pointers.

use collector_memory::terminal as allocation;
use collector_primitives::raw_memory;
use collector_snapshots::address_range;
use collector_status::status;

pub mod raw;
pub mod raw_plan;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
