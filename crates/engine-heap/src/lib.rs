
mod pages;

use std::alloc::{GlobalAlloc, Layout};
use std::ffi::{c_int, c_long, c_void};
use libmimalloc_sys as _;

pub struct Heap;

unsafe impl GlobalAlloc for Heap {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if pages::own(layout.size(), layout.align()) {
            unsafe { pages::alloc(layout.size(), false) }
        } else {
            unsafe { mi_malloc_aligned(layout.size(), layout.align()).cast() }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if pages::own(layout.size(), layout.align()) {
            unsafe { pages::alloc(layout.size(), true) }
        } else {
            unsafe { mi_zalloc_aligned(layout.size(), layout.align()).cast() }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pages::own(layout.size(), layout.align()) {
            unsafe { pages::free(pointer, layout.size()) }
        } else {
            unsafe { mi_free(pointer.cast()) }
        }
    }

    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let align = layout.align();
        match (pages::own(layout.size(), align), pages::own(size, align)) {
            (false, false) => {
                let usable = unsafe { mi_usable_size(pointer.cast()) };
                if size <= usable && size >= usable / 2 {
                    return pointer;
                }
            }
            (true, true) => {
                if unsafe { pages::resize(pointer, layout.size(), size) } {
                    return pointer;
                }
            }
            _ => {}
        }
        let moved = unsafe { self.alloc(Layout::from_size_align_unchecked(size, align)) };
        if !moved.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(pointer, moved, layout.size().min(size));
                self.dealloc(pointer, layout);
            }
        }
        moved
    }
}

const PURGE_DELAY_MS: c_long = 10;

const ARENA_EAGER_COMMIT: c_int = 4;
const PURGE_DECOMMITS: c_int = 5;
const PURGE_DELAY: c_int = 15;
const ARENA_RESERVE: c_int = 23;
const ARENA_PURGE_MULT: c_int = 24;
const PAGE_RECLAIM_ON_FREE: c_int = 35;
const PAGE_FULL_RETAIN: c_int = 36;
const PAGE_COMMIT_ON_DEMAND: c_int = 40;

const SET: [(c_int, c_long); 3] = [
    (PURGE_DELAY, PURGE_DELAY_MS),
    (ARENA_PURGE_MULT, 1),
    (PAGE_COMMIT_ON_DEMAND, 1),
];

const RETENTION: [(&str, c_int); 8] = [
    ("purge_delay", PURGE_DELAY),
    ("arena_purge_mult", ARENA_PURGE_MULT),
    ("purge_decommits", PURGE_DECOMMITS),
    ("page_commit_on_demand", PAGE_COMMIT_ON_DEMAND),
    ("page_full_retain", PAGE_FULL_RETAIN),
    ("page_reclaim_on_free", PAGE_RECLAIM_ON_FREE),
    ("arena_eager_commit", ARENA_EAGER_COMMIT),
    ("arena_reserve_kib", ARENA_RESERVE),
];

const STATS_WORDS: usize = 546;
const STATS_VERSION: u64 = 5;
const COMMITTED: usize = 10;

unsafe extern "C" {
    fn mi_malloc_aligned(size: usize, alignment: usize) -> *mut c_void;
    fn mi_zalloc_aligned(size: usize, alignment: usize) -> *mut c_void;
    fn mi_usable_size(pointer: *const c_void) -> usize;
    fn mi_free(pointer: *mut c_void);
    fn mi_option_set(option: c_int, value: c_long);
    fn mi_option_get(option: c_int) -> c_long;
    fn mi_collect(force: bool);
    fn mi_version() -> c_int;
    fn mi_stats_get(stats: *mut u64) -> bool;
    fn mi_good_size(size: usize) -> usize;
}

pub fn return_freed_memory_promptly() {
    for (option, value) in SET {
        unsafe { mi_option_set(option, value) };
    }
    engine_allocation::set_collect(Some(return_freed));
}

pub fn retention() -> (i32, Vec<(&'static str, i64)>) {
    let version = unsafe { mi_version() };
    let options = RETENTION
        .iter()
        .map(|&(name, option)| (name, i64::from(unsafe { mi_option_get(option) })))
        .collect();
    (version, options)
}

pub fn committed() -> Option<usize> {
    let mut stats = [0u64; STATS_WORDS];
    stats[0] = (STATS_WORDS * 8) as u64;
    stats[1] = STATS_VERSION;
    let read = unsafe { mi_stats_get(stats.as_mut_ptr()) };
    read.then(|| stats[COMMITTED] as usize + pages::committed())
}

pub fn large_blocks() -> (u64, u64, u64) {
    pages::contention()
}

pub fn size_class(size: usize) -> usize {
    if pages::own(size, 1) {
        return pages::rounded(size);
    }
    unsafe { mi_good_size(size) }
}

pub fn return_freed() {
    unsafe { mi_collect(true) };
    pages::release_expired();
}

pub struct ReturnFreed;

impl Drop for ReturnFreed {
    fn drop(&mut self) {
        return_freed();
    }
}
