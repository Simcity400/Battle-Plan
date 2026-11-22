#![recursion_limit = "512"]

pub mod extended;
pub mod land_events;

#[cfg(all(test, feature = "tap-foreign-probe"))]
#[path = "../../../tools/tap-foreign-unwind/runtime.rs"]
mod foreign_unwind_probe;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
