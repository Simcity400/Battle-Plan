//! How the processes hosting the data service return freed memory.

use std::ffi::{c_int, c_long};
// Links mimalloc's library into binaries that keep the system allocator too.
use mimalloc as _;

// The mimalloc static library provides these.
unsafe extern "C" {
    fn mi_option_set(option: c_int, value: c_long);
}

/// The data service frees gigabytes as preparation and scans finish, on worker
/// threads that may then idle. mimalloc returns a thread's freed pages after a
/// delay, but only when that thread next allocates, and never those of a thread
/// that ended, so the memory stayed committed. Call first in `main`: freed pages
/// are now returned at once, as are a finished thread's.
pub fn return_freed_memory_promptly() {
    // Indices into the bundled mimalloc v2's `mi_option_t`.
    const ABANDONED_PAGE_PURGE: c_int = 12;
    const PURGE_DELAY: c_int = 15;
    const ARENA_PURGE_MULT: c_int = 24;
    // SAFETY: setting an option has no preconditions; mimalloc reads them as it purges.
    unsafe {
        mi_option_set(PURGE_DELAY, 0);
        mi_option_set(ARENA_PURGE_MULT, 1);
        mi_option_set(ABANDONED_PAGE_PURGE, 1);
    }
}
