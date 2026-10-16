//! Synthetic allocation denial and per-call timing shared by collector tests.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    ptr,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering},
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

pub struct FailingAllocator;

// Only the prepared-buffer child interval denies allocations. Fixed controls,
// paths, thread creation and recovery run outside that accepted boundary.
static OWNER: AtomicU32 = AtomicU32::new(0);
static ORDINAL: AtomicUsize = AtomicUsize::new(usize::MAX);
pub static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
pub static DENIALS: AtomicUsize = AtomicUsize::new(0);
pub fn with_allocations_denied<T>(run: impl FnOnce() -> T) -> T {
    let guard = NullGuard::new(1);
    let output = run();
    drop(guard);
    output
}
fn deny() -> bool {
    if OWNER.load(Ordering::Relaxed) != unsafe { GetCurrentThreadId() } {
        return false;
    }
    let attempt = ATTEMPTS.fetch_add(1, Ordering::Relaxed) + 1;
    if attempt >= ORDINAL.load(Ordering::Relaxed) {
        DENIALS.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
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
        ORDINAL.store(ordinal, Ordering::Relaxed);
        ATTEMPTS.store(0, Ordering::Relaxed);
        DENIALS.store(0, Ordering::Relaxed);
        OWNER.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
        Self
    }
}
impl Drop for NullGuard {
    fn drop(&mut self) {
        OWNER.store(0, Ordering::Relaxed);
    }
}

/// Release benches: the median nanoseconds per call over seven timed rounds,
/// after one warm-up round. Each `round` makes `calls` calls.
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
        // SAFETY: CRT initialisers run before main, while this is the only thread.
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
