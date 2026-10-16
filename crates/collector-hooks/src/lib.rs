//! Windows x64 patch publication, thread suspension and synchronization, Tap bridges and publication measurements. These ordinary Rust library modules are linked into the collector DLL. They preserve native registers, flags, unwind registration and continuation, and retain executable storage whenever restoration or quiescence cannot be proved.

use collector_primitives::clock;
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;

pub mod hook;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
