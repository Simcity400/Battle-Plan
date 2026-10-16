//! Writer certificates, dirty marks, owner-slice selection, sampled verification and held research drift. Stores require complete reads, stable epochs, successful publication and valid writer certificates. Native capture orchestration and collector feature defaults belong to the DLL host.

use collector_primitives::{clock, table_hash};
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;

pub mod dirty_set;
pub mod drift;
pub mod sampled_verify;
pub mod slicing;
pub mod watch_registry;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
