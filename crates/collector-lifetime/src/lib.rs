//! Native world-reset observation and occurrence admission shared by between-tick captures. An invalidation starts a new occurrence at the next readable hourly boundary. Hook installation and lifetime state remain inside the same collector DLL.

use collector_hooks::hook;
use collector_timing::hook_timing;

pub mod world_lifetime;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
