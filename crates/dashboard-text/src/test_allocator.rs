use std::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
};

pub(crate) struct Allocator;
thread_local! {
    static REMAINING: Cell<Option<usize>> = const { Cell::new(None) };
    static ATTEMPTS: Cell<usize> = const { Cell::new(0) };
}

fn denied() -> bool {
    REMAINING
        .try_with(|remaining| match remaining.get() {
            Some(n) => {
                ATTEMPTS.set(ATTEMPTS.get() + 1);
                remaining.set(Some(n.saturating_sub(1)));
                n == 0
            }
            None => false,
        })
        .unwrap_or(false)
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if denied() {
            std::ptr::null_mut()
        } else {
            unsafe { engine_heap::Heap.alloc(layout) }
        }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if denied() {
            std::ptr::null_mut()
        } else {
            unsafe { engine_heap::Heap.alloc_zeroed(layout) }
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if denied() {
            std::ptr::null_mut()
        } else {
            unsafe { engine_heap::Heap.realloc(pointer, layout, size) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { engine_heap::Heap.dealloc(pointer, layout) }
    }
}

pub(crate) fn deny_after<T>(allowed: usize, run: impl FnOnce() -> T) -> (T, usize) {
    struct Reset(Option<usize>, usize);
    impl Drop for Reset {
        fn drop(&mut self) {
            REMAINING.set(self.0);
            ATTEMPTS.set(self.1);
        }
    }
    let _reset = Reset(REMAINING.replace(Some(allowed)), ATTEMPTS.replace(0));
    let result = run();
    (result, ATTEMPTS.get())
}
