#![recursion_limit = "512"]

pub mod air_detection_changes;
pub mod ground_crew_changes;
pub mod mission_changes;
pub mod naval_accidents;
pub mod naval_map_changes;
pub mod naval_mines;
pub mod navy_ledger_changes;
pub mod radar_changes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
