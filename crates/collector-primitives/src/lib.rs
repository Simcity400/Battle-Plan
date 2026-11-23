
use collector_status::status;

pub mod asymmetric;
pub mod build_gate;
pub mod capacity;
pub mod clock;
pub mod country_slots;
pub mod first_reason;
pub mod raw_memory;
pub mod shared_lane;
pub mod spill_stack;
pub mod table_capture;
pub mod table_hash;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
