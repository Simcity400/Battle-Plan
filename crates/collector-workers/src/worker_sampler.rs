use std::{
    cell::Cell,
    ptr,
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

pub static JOBS: Mutex<Vec<(u32, u64, Weak<AtomicU16>)>> = Mutex::new(Vec::new());
pub static SAMPLER_CONTROL: Mutex<Weak<Control>> = Mutex::new(Weak::new());
pub static EMERGENCY_EXIT: AtomicBool = AtomicBool::new(false);
pub static EARLY_EXIT: AtomicBool = AtomicBool::new(false);
thread_local! { pub static JOB_SLOT: Cell<*const AtomicU16> = const { Cell::new(ptr::null()) }; }

pub struct WorkerRegistration {
    id: u32,
    slot: Arc<AtomicU16>,
}
pub fn register_worker() -> WorkerRegistration {
    let id = unsafe { GetCurrentThreadId() };
    let slot = Arc::new(AtomicU16::new(0));
    JOB_SLOT.set(Arc::as_ptr(&slot));
    let created = creation(unsafe { GetCurrentThread() }).unwrap_or(0);
    JOBS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((id, created, Arc::downgrade(&slot)));
    WorkerRegistration { id, slot }
}
impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        stop_before_thread_exit();
        JOB_SLOT.set(ptr::null());
        self.slot.store(0, Ordering::Release);
        JOBS.lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _, _)| *id != self.id);
    }
}
pub struct Control {
    pub on_panic: fn(),
    pub stop: Arc<AtomicBool>,
    pub active: Arc<AtomicBool>,
    pub thread: Mutex<Option<JoinHandle<()>>>,
    pub ready: AtomicBool,
    pub join_complete: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    pub follower_waits: std::sync::atomic::AtomicUsize,
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
            #[cfg(any(test, feature = "test-support"))]
            follower_waits: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn stop_join(&self) {
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
                #[cfg(any(test, feature = "test-support"))]
                self.follower_waits.fetch_add(1, Ordering::Release);
                thread::yield_now();
            }
        }
    }
}
pub fn stop_before_thread_exit() {
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
    if unsafe { GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
