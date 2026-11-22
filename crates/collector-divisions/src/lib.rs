#![recursion_limit = "512"]

pub mod division_census;
pub mod extended;
pub mod orders;
pub mod orders_parts;
pub mod province_unit_changes;

#[cfg(any(test, feature = "test-support"))]
#[path = "../../engine/tests/native/support/unit_world.rs"]
pub mod unit_fixture;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

#[cfg(any(test, feature = "test-support"))]
pub mod census_test_fixture;
