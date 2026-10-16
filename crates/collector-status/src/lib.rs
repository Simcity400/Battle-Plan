//! Borrowed status objects, sequences, formatting adapters and checked streaming JSON serialization used by capture diagnostics. Construction allocates no heap storage. The DLL host owns whole-capture status composition and fault exercises.

pub mod status;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
