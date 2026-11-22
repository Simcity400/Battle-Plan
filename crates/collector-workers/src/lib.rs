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

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};

static OWNED: AtomicBool = AtomicBool::new(false);

pub struct Owner(PhantomData<*const ()>);

impl Owner {
    /// # Safety
    /// Claim once per attach, from the runtime's exclusive attach operation.
    /// Drop only after the hooks and workers have quiesced at detach. Retain the
    /// owner if detach cannot establish quiescence.
    pub unsafe fn claim() -> Self {
        assert!(
            !OWNED.swap(true, Ordering::Acquire),
            "collector already owned"
        );
        Self(PhantomData)
    }

    pub fn capture(&self) -> CaptureAccess {
        CaptureAccess { _private: () }
    }

    pub fn worker(&self) -> WorkerStart {
        WorkerStart { live: true }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        OWNED.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
pub struct CaptureAccess {
    _private: (),
}

pub struct WorkerStart {
    live: bool,
}

impl WorkerStart {
    pub fn bind(self) -> Worker {
        Worker {
            live: self.live,
            _thread: PhantomData,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test() -> Self {
        Self { live: false }
    }

    #[cfg(feature = "worker_sampler")]
    pub(crate) fn is_live(&self) -> bool {
        self.live
    }
}

pub struct Worker {
    live: bool,
    _thread: PhantomData<*const ()>,
}

impl Worker {
    pub fn is_owned(&self) -> bool {
        self.live
    }

    #[cfg(feature = "worker_sampler")]
    pub fn sampled(&self) -> thread_cpu::StopOnThreadExit<'_> {
        thread_cpu::StopOnThreadExit(self)
    }
}

pub const SEGMENTS: usize = 15;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
