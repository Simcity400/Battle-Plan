//! Concurrent observation of native logistics balance writes and their available-stock operands. Writers retain independent cell refusals, prepared storage, first observations and lifetime checks. Hooks are ordinary Rust library code linked into the existing collector DLL.

use collector_hooks::hook;
use collector_primitives::raw_memory;
use collector_timing::hook_timing;

pub mod logistics_balance;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
