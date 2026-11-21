
#[cfg(any(test, feature = "test-support"))]
use collector_memory::read_accounting;
use collector_primitives::clock;
#[cfg(test)]
use collector_test_support as _;

#[cfg(any(test, feature = "test-support"))]
pub mod bench_world;
pub mod demand;
pub mod distribution;
pub mod hourly;
pub mod outcomes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
