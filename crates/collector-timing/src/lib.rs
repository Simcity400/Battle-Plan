//! Sampled per-thread hook timing, TSC calibration, original-call accounting and borrowed status output. Timer reads and sampling policy belong here; hook installation and capture-family behavior belong to their callers.

use collector_primitives::clock;
use collector_status::status;

pub mod hook_timing;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
