//! Shared synthetic allocation-denial support.
use super::*;

pub struct DeniedAllocator;
thread_local! {
    static DENY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
unsafe impl std::alloc::GlobalAlloc for DeniedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if DENY.try_with(|deny| deny.get()).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { std::alloc::System.alloc(layout) }
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if DENY.try_with(|deny| deny.get()).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { std::alloc::System.realloc(pointer, layout, size) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { std::alloc::System.dealloc(pointer, layout) };
    }
}
#[cfg(feature = "alloc-fault-injection")]
pub fn with_allocations_denied<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            DENY.set(self.0);
        }
    }
    let _reset = Reset(DENY.replace(true));
    run()
}
