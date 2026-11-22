#![recursion_limit = "512"]

pub mod acquisition;
#[cfg(feature = "callback_profile")]
pub mod callback_profile;
pub mod cpu_sets;
pub mod hour_stages;
pub mod panic_failure;
pub mod private_heap;
pub mod roles;
#[cfg(feature = "worker_sampler")]
pub mod worker_sampler;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
