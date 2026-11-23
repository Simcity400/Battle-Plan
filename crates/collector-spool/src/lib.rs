#![recursion_limit = "512"]

pub mod columnar;
pub mod history;
mod map_spool;
mod site_withholding;
pub mod slow_hours;
mod spool_compare;
mod spool_segments;
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
