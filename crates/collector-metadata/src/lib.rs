//! Prepared native content rosters, recorded identity verification and executable layout reports for capture. Content facts are observed from the active installation and its mounted content. The DLL host prepares, samples and publishes these values.

use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;

pub mod fixed_layout;
pub mod roster;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
