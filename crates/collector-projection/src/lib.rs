//! Checked projection and conversion of owned native rows into replay table outcomes. Sorting, exact row counts, duplicate refusals, cached comparisons and terminal allocation guards are shared by capture packs and spool composition.

use collector_memory::terminal as allocation;

pub mod certified;
pub mod projection;
pub mod table_conversion;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
