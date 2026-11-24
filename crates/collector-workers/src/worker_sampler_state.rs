use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, HANDLE},
    System::Threading::{GetCurrentThread, GetCurrentThreadId, GetThreadTimes},
};

static JOBS: Mutex<Vec<(u32, u64, Weak<AtomicU16>)>> = Mutex::new(Vec::new());
pub(crate) static SAMPLER_CONTROL: Mutex<Weak<Control>> = Mutex::new(Weak::new());
pub(crate) static EMERGENCY_EXIT: AtomicBool = AtomicBool::new(false);
pub(crate) static EARLY_EXIT: AtomicBool = AtomicBool::new(false);
thread_local! {
    static JOB_SLOT: RefCell<Option<Arc<AtomicU16>>> = const { RefCell::new(None) };
}

pub(crate) struct WorkerRegistration {
    id: u32,
    slot: Arc<AtomicU16>,
    _thread: PhantomData<Rc<()>>,
}
pub(crate) fn register_worker() -> WorkerRegistration {
    let id = unsafe { GetCurrentThreadId() };
    let slot = Arc::new(AtomicU16::new(0));
    JOB_SLOT.replace(Some(Arc::clone(&slot)));
    let created = creation(unsafe { GetCurrentThread() }).unwrap_or(0);
    JOBS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((id, created, Arc::downgrade(&slot)));
    WorkerRegistration {
        id,
        slot,
        _thread: PhantomData,
    }
}
impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        // SAFETY: This owned sampled worker completes the exit handshake before leaving.
        unsafe { stop_before_thread_exit() };
        JOB_SLOT.replace(None);
        self.slot.store(0, Ordering::Release);
        JOBS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _, _)| *id != self.id);
    }
}

enum JobSlot {
    Worker(Arc<AtomicU16>),
    Game(&'static AtomicU16),
}

impl JobSlot {
    fn atomic(&self) -> &AtomicU16 {
        match self {
            Self::Worker(slot) => slot,
            Self::Game(slot) => slot,
        }
    }
}

pub struct JobGuard {
    slot: Option<JobSlot>,
    previous: u16,
    _thread: PhantomData<Rc<()>>,
}

/// # Safety
/// Register the role only for the live collector worker executing that job. Drop its guard on this
/// thread before another job or detach can consume the registration.
pub unsafe fn job(id: u16, game_slot: impl FnOnce() -> Option<&'static AtomicU16>) -> JobGuard {
    let slot = JOB_SLOT
        .with_borrow(Clone::clone)
        .map(JobSlot::Worker)
        .or_else(|| game_slot().map(JobSlot::Game));
    let previous = slot
        .as_ref()
        .map_or(0, |slot| slot.atomic().swap(id, Ordering::Release));
    JobGuard {
        slot,
        previous,
        _thread: PhantomData,
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Some(slot) = &self.slot {
            slot.atomic().store(self.previous, Ordering::Release);
        }
    }
}

pub struct SampledJob(Arc<AtomicU16>);

impl SampledJob {
    pub fn id(&self) -> u16 {
        self.0.load(Ordering::Acquire)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture(slot: Arc<AtomicU16>) -> Self {
        Self(slot)
    }
}

pub(crate) fn registered_job(id: u32) -> Option<(u64, SampledJob)> {
    JOBS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(registered, _, _)| *registered == id)
        .and_then(|(_, created, job)| job.upgrade().map(|job| (*created, SampledJob(job))))
}

pub(crate) fn clear_jobs() {
    let jobs = { std::mem::take(&mut *JOBS.lock().unwrap_or_else(|e| e.into_inner())) };
    drop(jobs);
}

pub struct Control {
    pub(crate) on_panic: fn(),
    pub stop: Arc<AtomicBool>,
    pub active: Arc<AtomicBool>,
    pub thread: Mutex<Option<JoinHandle<()>>>,
    pub ready: AtomicBool,
    pub(crate) join_complete: AtomicBool,
}
impl Control {
    pub fn new(
        stop: Arc<AtomicBool>,
        active: Arc<AtomicBool>,
        enabled: bool,
        on_panic: fn(),
    ) -> Self {
        Self {
            on_panic,
            stop,
            active,
            thread: Mutex::new(None),
            ready: AtomicBool::new(!enabled),
            join_complete: AtomicBool::new(!enabled),
        }
    }

    pub(crate) fn stop_join(&self) {
        self.stop_join_waiting(thread::yield_now);
    }

    pub(crate) fn stop_join_waiting(&self, mut wait: impl FnMut()) {
        self.stop.store(true, Ordering::Release);
        while !self.ready.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let thread = { self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() };
        if let Some(thread) = thread {
            let panicked = thread.join().is_err();
            if panicked {
                (self.on_panic)();
            }
            self.join_complete.store(true, Ordering::Release);
        } else {
            while !self.join_complete.load(Ordering::Acquire) {
                wait();
            }
        }
    }
}
/// # Safety
/// Only the owner of a live sampled worker or the runtime joining it may perform this handshake,
/// before allowing the sampled thread to exit.
pub unsafe fn stop_before_thread_exit() {
    EARLY_EXIT.store(true, Ordering::Release);
    let control = {
        SAMPLER_CONTROL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
    };
    if let Some(control) = control {
        if !control.stop.swap(true, Ordering::AcqRel) {
            EMERGENCY_EXIT.store(true, Ordering::Release);
        }
        control.stop_join();
    }
}
pub fn creation(handle: HANDLE) -> Option<u64> {
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }; 4];
    if unsafe { GetThreadTimes(handle, &raw mut created, &raw mut exited, &raw mut kernel, &raw mut user) } == 0 {
        return None;
    }
    Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
