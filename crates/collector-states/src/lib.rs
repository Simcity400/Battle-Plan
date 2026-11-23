#![recursion_limit = "512"]

pub mod acquisition_gather;
pub mod acquisition_states;
pub mod acquisition_wars;
pub mod extended;
pub mod scripted_maps;
#[cfg(any(test, feature = "test-support"))]
pub mod wars_test_kit;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
