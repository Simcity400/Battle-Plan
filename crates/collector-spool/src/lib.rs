#![recursion_limit = "512"]

pub mod columnar;
pub mod history;
pub mod map_spool;
pub mod site_withholding;
pub mod slow_hours;
pub mod spool_compare;
pub mod spool_segments;
pub mod spool_writer;
pub mod spooling;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
