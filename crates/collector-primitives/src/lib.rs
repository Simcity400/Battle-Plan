//! Shared typed capture rows and arenas, observed country sizing, observer publication lanes, address-table hashing, raw-memory capacity accounting and session clock reads. Buffer growth remains off the game thread and keeps exact observed demand and refusal outcomes.

use collector_status::status;

pub mod clock;
pub mod country_slots;
pub mod raw_memory;
pub mod shared_lane;
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
