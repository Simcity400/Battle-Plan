
mod pages;
mod windows_heap;

#[cfg(test)]
mod tests;

use std::alloc::{GlobalAlloc, Layout};
use std::ffi::{c_int, c_long, c_void};
use libmimalloc_sys as _;

pub struct Heap;
pub struct SegmentHeap;

// SAFETY: a block goes back the way it came, which its layout alone decides.
unsafe impl GlobalAlloc for Heap {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if pages::own(layout.size(), layout.align()) {
            // SAFETY: the nonzero layout fits the page backend's size and alignment.
            unsafe { pages::alloc(layout.size(), false) }
        } else {
            // SAFETY: the caller supplied a valid nonzero layout to allocate.
            unsafe { mi_malloc_aligned(layout.size(), layout.align()).cast() }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if pages::own(layout.size(), layout.align()) {
            // SAFETY: the nonzero layout fits the page backend's size and alignment.
            unsafe { pages::alloc(layout.size(), true) }
        } else {
            // SAFETY: the caller supplied a valid nonzero layout to allocate.
            unsafe { mi_zalloc_aligned(layout.size(), layout.align()).cast() }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pages::own(layout.size(), layout.align()) {
            // SAFETY: pointer and layout identify a live page allocation.
            unsafe { pages::free(pointer, layout.size()) }
        } else {
            // SAFETY: the layout identifies a live mimalloc allocation.
            unsafe { mi_free(pointer.cast()) }
        }
    }

    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let align = layout.align();
        match (pages::own(layout.size(), align), pages::own(size, align)) {
            (false, false) => {
                // SAFETY: the original layout selects mimalloc and the block is still live.
                let usable = unsafe { mi_usable_size(pointer.cast()) };
                if size <= usable && size >= usable / 2 {
                    return pointer;
                }
            }
            (true, true) => {
                // SAFETY: pointer is a live page allocation and both sizes fit the page backend.
                if unsafe { pages::resize(pointer, layout.size(), size) } {
                    return pointer;
                }
            }
            _ => {}
        }
        // SAFETY: GlobalAlloc requires the new size to fit a Layout with the old alignment.
        let next = unsafe { Layout::from_size_align_unchecked(size, align) };
        // SAFETY: the caller supplied the live block and its original layout.
        unsafe { move_block(self, pointer, layout, next) }
    }
}

// SAFETY: the layout and the process's fixed private heap select the same backend at allocation and free.
unsafe impl GlobalAlloc for SegmentHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match windows_heap::for_layout(layout) {
            // SAFETY: the private heap supports the alignment checked by for_layout.
            Some(heap) => unsafe { heap.alloc(layout.size(), false) },
            // SAFETY: the caller supplied a nonzero layout suitable for GlobalAlloc.
            None => unsafe { Heap.alloc(layout) },
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match windows_heap::for_layout(layout) {
            // SAFETY: the private heap supports the alignment checked by for_layout.
            Some(heap) => unsafe { heap.alloc(layout.size(), true) },
            // SAFETY: the caller supplied a nonzero layout suitable for GlobalAlloc.
            None => unsafe { Heap.alloc_zeroed(layout) },
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        match windows_heap::for_layout(layout) {
            // SAFETY: the caller supplied a live allocation from the private heap.
            Some(heap) => unsafe { heap.free(pointer) },
            // SAFETY: the caller supplied a live allocation from Heap with its original layout.
            None => unsafe { Heap.dealloc(pointer, layout) },
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc requires the new size to fit a Layout with the old alignment.
        let next = unsafe { Layout::from_size_align_unchecked(size, layout.align()) };
        match (windows_heap::for_layout(layout), windows_heap::for_layout(next)) {
            // SAFETY: both layouts belong to the same private heap, and pointer is its live block.
            (Some(heap), Some(_)) => unsafe { heap.resize(pointer, size) },
            // SAFETY: both layouts use Heap and pointer is its live block with the original layout.
            (None, None) => unsafe { Heap.realloc(pointer, layout, size) },
            // SAFETY: pointer is a live allocation from this allocator and next is a valid layout.
            _ => unsafe { move_block(self, pointer, layout, next) },
        }
    }
}

unsafe fn move_block(
    allocator: &impl GlobalAlloc,
    pointer: *mut u8,
    layout: Layout,
    next: Layout,
) -> *mut u8 {
    // SAFETY: the caller supplied the valid nonzero layout for the next allocation.
    let moved = unsafe { allocator.alloc(next) };
    if !moved.is_null() {
        // SAFETY: these are distinct live blocks. The copy fits both and the old layout frees pointer.
        unsafe {
            std::ptr::copy_nonoverlapping(pointer, moved, layout.size().min(next.size()));
            allocator.dealloc(pointer, layout);
        }
    }
    moved
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
        // SAFETY: SET carries option IDs and values for the pinned mimalloc version.
        unsafe { mi_option_set(option, value) };
    }
    engine_allocation::set_collect(Some(return_freed));
}

pub fn retention() -> (i32, Vec<(&'static str, i64)>) {
    // SAFETY: mi_version takes no arguments and does not read caller state.
    let version = unsafe { mi_version() };
    let options = RETENTION
        .iter()
        .map(|&(name, option)| {
            // SAFETY: each ID in RETENTION names an option in the pinned mimalloc version.
            (name, i64::from(unsafe { mi_option_get(option) }))
        })
        .collect();
    (version, options)
}

pub fn committed() -> Option<usize> {
    let mut stats = [0u64; STATS_WORDS];
    stats[0] = (STATS_WORDS * 8) as u64;
    stats[1] = STATS_VERSION;
    // SAFETY: stats supplies the size and version of the pinned mimalloc stats structure.
    let read = unsafe { mi_stats_get(stats.as_mut_ptr()) };
    read.then_some(stats[COMMITTED] as usize + pages::committed())
        .zip(windows_heap::committed())
        .map(|(mimalloc_and_pages, windows)| mimalloc_and_pages + windows)
}

pub fn large_blocks() -> (u64, u64, u64) {
    pages::contention()
}

pub fn size_class(size: usize) -> usize {
    if pages::own(size, 1) {
        return pages::rounded(size);
    }
    // SAFETY: mi_good_size accepts a byte count without reading any caller memory.
    unsafe { mi_good_size(size) }
}

pub fn return_freed() {
    // SAFETY: mimalloc allows its collection routine to be called on the current thread.
    unsafe { mi_collect(true) };
    pages::release_expired();
}

pub struct ReturnFreed;

impl Drop for ReturnFreed {
    fn drop(&mut self) {
        return_freed();
    }
}
