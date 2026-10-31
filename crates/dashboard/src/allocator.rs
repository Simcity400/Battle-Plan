//! How the processes hosting the data service return freed memory.

use std::ffi::{c_int, c_long};
// Links mimalloc's library into binaries that keep the system allocator too.
use mimalloc as _;

/// How long, in milliseconds, a freed page stays committed for its thread to reuse. Returning
/// and recommitting pages at once made a 13-year preparation spend about a tenth more CPU;
/// longer delays saved little more for gigabytes of commit.
const PURGE_DELAY_MS: c_long = 10;

// The mimalloc static library provides these.
unsafe extern "C" {
    fn mi_option_set(option: c_int, value: c_long);
    fn mi_collect(force: bool);
}

/// The data service frees gigabytes as preparation and scans finish, on worker
/// threads that may then idle. mimalloc returns a thread's freed pages after a
/// delay, but only when that thread next allocates, and never those of a thread
/// that ended, so the memory stayed committed. Call first in `main`, before any
/// other thread runs: mimalloc's options are not synchronized. A finished thread's
/// pages are now returned at once, and a running thread's [`PURGE_DELAY_MS`] after
/// it frees them, at its next allocation or [`return_freed`].
pub fn return_freed_memory_promptly() {
    // Indices into the bundled mimalloc v2's `mi_option_t`.
    const ABANDONED_PAGE_PURGE: c_int = 12;
    const PURGE_DELAY: c_int = 15;
    const ARENA_PURGE_MULT: c_int = 24;
    // SAFETY: setting an option has no preconditions; mimalloc reads them as it purges.
    unsafe {
        mi_option_set(PURGE_DELAY, PURGE_DELAY_MS);
        mi_option_set(ARENA_PURGE_MULT, 1);
        mi_option_set(ABANDONED_PAGE_PURGE, 1);
    }
}

/// Returns the pages this thread freed and is holding for reuse, and those finished threads
/// left.
pub fn return_freed() {
    // SAFETY: collecting has no preconditions and leaves mimalloc's options alone.
    unsafe { mi_collect(true) }
}

/// Calls [`return_freed`] as it is dropped: a preparation holds one, so every way out of it
/// returns what it freed.
pub struct ReturnFreed;

impl Drop for ReturnFreed {
    fn drop(&mut self) {
        return_freed();
    }
}
