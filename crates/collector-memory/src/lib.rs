//! Terminal witness storage, scoped allocation context, checked raw-reserve diagnostics and optional fault injection. The collector host installs the native allocation callbacks and cancels its frame wait after the first failure is published. Witness bytes and committed-watermark order are owned here.

#[cfg(any(test, feature = "test-support"))]
use collector_test_support as test_support;

pub mod read_accounting;
#[cfg(feature = "acquisition_profile")]
pub mod acquisition_profile;
pub mod terminal;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
