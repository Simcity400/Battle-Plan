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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static OWNED: AtomicU64 = AtomicU64::new(0);
static CAPTURE_GENERATION: AtomicU64 = AtomicU64::new(0);
static CAPTURE_READERS: AtomicUsize = AtomicUsize::new(0);

pub struct Owner {
    generation: u64,
    _thread: PhantomData<*const ()>,
}

impl Owner {
    /// # Safety
    /// Claim once per attach, from the runtime's exclusive attach operation.
    /// Drop only after the hooks and workers have quiesced at detach. Retain the
    /// owner if detach cannot establish quiescence.
    pub unsafe fn claim() -> Self {
        let generation = NEXT_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .expect("collector attach generation exhausted");
        assert_eq!(
            OWNED.compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire),
            Ok(0),
            "collector already owned"
        );
        CAPTURE_GENERATION.store(generation, Ordering::SeqCst);
        Self {
            generation,
            _thread: PhantomData,
        }
    }

    pub fn capture(&self) -> CaptureAccess {
        CaptureAccess(self.generation)
    }

    pub fn worker(&self) -> WorkerStart {
        WorkerStart {
            generation: self.generation,
        }
    }

    pub fn revoke_capture(&self) {
        CAPTURE_GENERATION.store(0, Ordering::SeqCst);
    }

    pub fn capture_quiet(&self) -> bool {
        CAPTURE_READERS.load(Ordering::SeqCst) == 0
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.revoke_capture();
        while !self.capture_quiet() {
            std::thread::yield_now();
        }
        OWNED.store(0, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureAccess(u64);

impl CaptureAccess {
    pub fn pin(&self) -> Option<CapturePin> {
        if CAPTURE_GENERATION.load(Ordering::SeqCst) != self.0 {
            return None;
        }
        CAPTURE_READERS.fetch_add(1, Ordering::SeqCst);
        let pin = CapturePin(());
        (CAPTURE_GENERATION.load(Ordering::SeqCst) == self.0).then_some(pin)
    }
}

pub struct CapturePin(());

impl Drop for CapturePin {
    fn drop(&mut self) {
        CAPTURE_READERS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct WorkerStart {
    generation: u64,
}

impl WorkerStart {
    pub fn bind(self) -> Worker {
        assert!(
            self.generation == 0 || self.is_live(),
            "collector worker permit expired"
        );
        Worker {
            generation: self.generation,
            _thread: PhantomData,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test() -> Self {
        Self { generation: 0 }
    }

    #[cfg(feature = "worker_sampler")]
    pub fn sampler_join(&self) -> Option<SamplerJoin> {
        self.is_live().then_some(SamplerJoin(self.generation))
    }

    pub(crate) fn is_live(&self) -> bool {
        self.generation != 0 && OWNED.load(Ordering::Acquire) == self.generation
    }
}

#[cfg(feature = "worker_sampler")]
pub struct SamplerJoin(u64);

#[cfg(feature = "worker_sampler")]
impl SamplerJoin {
    pub fn before_join(self) {
        if self.0 != 0
            && let Some(_pin) = CaptureAccess(self.0).pin()
        {
            worker_sampler_state::stop_before_thread_exit();
        }
    }
}

pub struct Worker {
    generation: u64,
    _thread: PhantomData<*const ()>,
}

impl Worker {
    pub fn is_owned(&self) -> bool {
        self.generation != 0 && OWNED.load(Ordering::Acquire) == self.generation
    }

    pub(crate) fn permitted(&self) -> bool {
        worker_permitted(self.generation)
    }

    pub(crate) fn pin(&self) -> Option<CapturePin> {
        (self.generation != 0)
            .then(|| CaptureAccess(self.generation))
            .and_then(|access| access.pin())
    }

    #[cfg(feature = "worker_sampler")]
    pub fn sampled(&self) -> thread_cpu::StopOnThreadExit<'_> {
        thread_cpu::StopOnThreadExit(self)
    }
}

fn worker_permitted(generation: u64) -> bool {
    generation == 0 || OWNED.load(Ordering::Acquire) == generation
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
