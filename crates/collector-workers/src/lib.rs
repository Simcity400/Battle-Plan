#![recursion_limit = "512"]

pub mod acquisition;
#[cfg(feature = "callback_profile")]
pub mod callback_profile;
#[cfg(feature = "callback_profile")]
pub mod callback_profile_state;
pub mod cpu_sets;
pub mod event;
pub mod frame_fence;
pub mod frame_priority;
pub mod hour_stages;
pub mod module_threads;
pub mod panic_failure;
pub mod priority;
pub mod private_heap;
pub mod roles;
pub mod spool_helper;
pub mod spool_phases;
pub mod thread_cpu;
pub mod tick_gate;
pub mod worker_panic;
#[cfg(feature = "worker_sampler")]
pub mod worker_sampler;
#[cfg(feature = "worker_sampler")]
pub mod worker_sampler_state;

pub const SEGMENTS: usize = 15;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
