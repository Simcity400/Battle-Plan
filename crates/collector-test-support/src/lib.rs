use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

pub struct FailingAllocator;

thread_local! {
    static ORDINAL: Cell<Option<usize>> = const { Cell::new(None) };
    static ATTEMPT: Cell<usize> = const { Cell::new(0) };
    static DENIED: Cell<usize> = const { Cell::new(0) };
}
pub struct Count(&'static std::thread::LocalKey<Cell<usize>>);
impl Count {
    pub fn load(&self, _: Ordering) -> usize {
        self.0.with(Cell::get)
    }
}
pub static ATTEMPTS: Count = Count(&ATTEMPT);
pub static DENIALS: Count = Count(&DENIED);
pub fn with_allocations_denied<T>(run: impl FnOnce() -> T) -> T {
    let guard = NullGuard::new(1);
    let output = run();
    drop(guard);
    output
}
fn deny() -> bool {
    let Ok(Some(ordinal)) = ORDINAL.try_with(Cell::get) else {
        return false;
    };
    let Ok(attempt) = ATTEMPT.try_with(|attempt| {
        attempt.set(attempt.get() + 1);
        attempt.get()
    }) else {
        return false;
    };
    if attempt < ordinal {
        return false;
    }
    let _ = DENIED.try_with(|denied| denied.set(denied.get() + 1));
    true
}
unsafe impl GlobalAlloc for FailingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }
    unsafe fn realloc(&self, old: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.realloc(old, layout, size) }
        }
    }
    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        unsafe { System.dealloc(allocation, layout) };
    }
}
pub struct NullGuard;
impl NullGuard {
    pub fn new(ordinal: usize) -> Self {
        ATTEMPT.set(0);
        DENIED.set(0);
        ORDINAL.set(Some(ordinal));
        Self
    }
}
impl Drop for NullGuard {
    fn drop(&mut self) {
        ORDINAL.set(None);
    }
}

pub fn per_call_ns(calls: usize, mut round: impl FnMut()) -> f64 {
    round();
    let mut times: Vec<f64> = (0..7)
        .map(|_| {
            let started = std::time::Instant::now();
            round();
            started.elapsed().as_nanos() as f64 / calls as f64
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[3]
}
/// Reserve the original test-thread stack before the harness starts.
///
/// # Safety
/// Call only from a test executable CRT initializer, before other threads run.
pub unsafe extern "C" fn reserve_test_stack() {
    if std::env::var_os("RUST_MIN_STACK").is_none() {
        unsafe { std::env::set_var("RUST_MIN_STACK", "33554432") };
    }
}
pub static TARGET_PHASE: AtomicU8 = AtomicU8::new(0);
pub static REACHED: AtomicBool = AtomicBool::new(false);

pub fn phase_enter(phase: u8) {
    if TARGET_PHASE.load(Ordering::Relaxed) == phase {
        REACHED.store(true, Ordering::Relaxed);
    }
}

pub const TEST_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(120);

pub fn watched(state: impl Fn() -> String, scenario: impl FnOnce(&Steps) + Send + 'static) {
    use std::sync::mpsc::RecvTimeoutError;
    let (steps, seen) = std::sync::mpsc::channel();
    let scenario = std::thread::spawn(move || scenario(&Steps(steps)));
    let mut last = String::from("starting");
    loop {
        match seen.recv_timeout(TEST_HANG_GUARD) {
            Ok(step) => last = step,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!("blocked for {TEST_HANG_GUARD:?} at: {last}; {}", state())
            }
        }
    }
    if let Err(panic) = scenario.join() {
        std::panic::resume_unwind(panic);
    }
}

pub struct Ending<T> {
    pub thread: std::thread::JoinHandle<()>,
    pub done: std::sync::mpsc::Receiver<T>,
}

pub fn spawn_ending<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> Ending<T> {
    let (send, done) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = send.send(run());
    });
    Ending { thread, done }
}

impl<T> Ending<T> {
    pub fn thread(&self) -> &std::thread::Thread {
        self.thread.thread()
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    pub fn end(self, what: impl FnOnce() -> String) -> T {
        use std::sync::mpsc::RecvTimeoutError;
        match self.done.recv_timeout(TEST_HANG_GUARD) {
            Ok(result) => {
                let _ = self.thread.join();
                result
            }
            Err(RecvTimeoutError::Disconnected) => match self.thread.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => unreachable!("a thread that returned sent its result"),
            },
            Err(RecvTimeoutError::Timeout) => {
                panic!("{} did not end within {TEST_HANG_GUARD:?}", what())
            }
        }
    }
}

pub struct Steps(std::sync::mpsc::Sender<String>);

impl Steps {
    pub fn at(&self, step: impl std::fmt::Display) {
        let _ = self.0.send(step.to_string());
    }
}

#[cfg(windows)]
pub fn qpc_frequency() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    }
    let mut frequency = 0;
    unsafe { QueryPerformanceFrequency(&raw mut frequency) };
    frequency.max(1) as u64
}
