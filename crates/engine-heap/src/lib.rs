//! The process heap: mimalloc, with pages of their own from the system for large blocks.

mod pages;

use std::alloc::{GlobalAlloc, Layout};
use std::ffi::{c_int, c_long, c_void};
// Links mimalloc's library; the functions it provides are declared below.
use libmimalloc_sys as _;

/// The global allocator of every binary and test target. A block past [`pages::LARGE`] bytes
/// has pages of its own, committed to the page it uses, where mimalloc would round it to its
/// size class and then to whole 64 KiB slices.
pub struct Heap;

// SAFETY: a block goes back the way it came, which its layout alone decides.
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
                // mimalloc's own rule, without mi_realloc: v3's reads a thread heap still null.
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
        // SAFETY: the caller's alignment with a size it vouches for.
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

/// How long, in milliseconds, a freed page stays committed for its thread to reuse. Returning
/// and recommitting pages at once made a 13-year preparation spend about a tenth more CPU;
/// longer delays saved little more for gigabytes of commit.
const PURGE_DELAY_MS: c_long = 10;

// Indices into the bundled mimalloc v3's `mi_option_t`.
const ARENA_EAGER_COMMIT: c_int = 4;
const PURGE_DECOMMITS: c_int = 5;
const PURGE_DELAY: c_int = 15;
const ARENA_RESERVE: c_int = 23;
const ARENA_PURGE_MULT: c_int = 24;
const PAGE_RECLAIM_ON_FREE: c_int = 35;
const PAGE_FULL_RETAIN: c_int = 36;
const PAGE_COMMIT_ON_DEMAND: c_int = 40;

/// The options [`return_freed_memory_promptly`] sets, in the order it sets them. A page
/// commits as its blocks are carved: committed whole, its blocks never used stayed committed.
const SET: [(c_int, c_long); 3] = [
    (PURGE_DELAY, PURGE_DELAY_MS),
    (ARENA_PURGE_MULT, 1),
    (PAGE_COMMIT_ON_DEMAND, 1),
];

/// The options that decide what mimalloc keeps committed, by name.
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

/// The bundled mimalloc's `mi_stats_t`, in 8-byte words, and its `MI_STAT_VERSION`.
const STATS_WORDS: usize = 546;
const STATS_VERSION: u64 = 5;
/// The word of `mi_stats_t` holding `committed.current`.
const COMMITTED: usize = 10;

// The mimalloc static library provides these.
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

/// The data service frees gigabytes as preparation and scans finish, on worker
/// threads that may then idle. mimalloc returns a thread's freed pages after a
/// delay, but only when that thread next allocates, so the memory stayed committed.
/// Call first in `main`, before any other thread runs: mimalloc's options are not
/// synchronized. A thread's freed pages now go back [`PURGE_DELAY_MS`] after it frees
/// them, at its next allocation or [`return_freed`], which the executor's workers
/// call between tasks.
pub fn return_freed_memory_promptly() {
    for (option, value) in SET {
        // SAFETY: setting an option has no preconditions; mimalloc reads them as it purges.
        unsafe { mi_option_set(option, value) };
    }
    engine_allocation::set_collect(Some(return_freed));
}

/// mimalloc's version, as it numbers it (major, two digits of minor, two of patch), and the
/// value of each option deciding what it keeps committed, as it reads them now.
pub fn retention() -> (i32, Vec<(&'static str, i64)>) {
    // SAFETY: reading the version and options has no preconditions.
    let version = unsafe { mi_version() };
    let options = RETENTION
        .iter()
        .map(|&(name, option)| (name, i64::from(unsafe { mi_option_get(option) })))
        .collect();
    (version, options)
}

/// The bytes the heap holds committed from the system: mimalloc's, as it counts them, and
/// large blocks' own pages; None when mimalloc's statistics are laid out otherwise. It
/// allocates nothing, so the allocator may call it.
pub fn committed() -> Option<usize> {
    let mut stats = [0u64; STATS_WORDS];
    stats[0] = (STATS_WORDS * 8) as u64;
    stats[1] = STATS_VERSION;
    // SAFETY: a block of the size the first word gives; mimalloc refuses any other size or
    // version.
    let read = unsafe { mi_stats_get(stats.as_mut_ptr()) };
    read.then(|| stats[COMMITTED] as usize + pages::committed())
}

/// Large blocks' own pages: the calls made, those that overlapped another call (what one list
/// behind one lock would have made wait), and those that found their size class held and so took
/// fresh pages or released at once instead of waiting.
pub fn large_blocks() -> (u64, u64, u64) {
    pages::contention()
}

/// The bytes a request of `size` takes of the heap: its own pages past [`pages::LARGE`], else
/// mimalloc's size class. It allocates nothing, so the allocator may call it.
pub fn size_class(size: usize) -> usize {
    if pages::own(size, 1) {
        return pages::rounded(size);
    }
    // SAFETY: a pure function of the size.
    unsafe { mi_good_size(size) }
}

/// Returns the pages this thread freed and is holding for reuse, those finished threads left,
/// and the large blocks' pages kept past their delay.
pub fn return_freed() {
    // SAFETY: collecting has no preconditions and leaves mimalloc's options alone.
    unsafe { mi_collect(true) };
    pages::release_expired();
}

/// Calls [`return_freed`] as it is dropped: a preparation holds one, so every way out of it
/// returns what it freed.
pub struct ReturnFreed;

impl Drop for ReturnFreed {
    fn drop(&mut self) {
        return_freed();
    }
}
