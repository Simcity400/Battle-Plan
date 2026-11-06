//! How the processes hosting the data service return freed memory.

use std::ffi::{c_int, c_long};
// Links mimalloc's library into binaries that keep the system allocator too.
use mimalloc as _;

/// How long, in milliseconds, a freed page stays committed for its thread to reuse. Returning
/// and recommitting pages at once made a 13-year preparation spend about a tenth more CPU;
/// longer delays saved little more for gigabytes of commit.
const PURGE_DELAY_MS: c_long = 10;

// Indices into the bundled mimalloc v2's `mi_option_t`.
const EAGER_COMMIT: c_int = 3;
const ARENA_EAGER_COMMIT: c_int = 4;
const PURGE_DECOMMITS: c_int = 5;
const ABANDONED_PAGE_PURGE: c_int = 12;
const PURGE_DELAY: c_int = 15;
const ARENA_RESERVE: c_int = 23;
const ARENA_PURGE_MULT: c_int = 24;
const PURGE_EXTEND_DELAY: c_int = 25;

/// The options [`return_freed_memory_promptly`] sets, in the order it sets them.
const SET: [(c_int, c_long); 3] = [
    (PURGE_DELAY, PURGE_DELAY_MS),
    (ARENA_PURGE_MULT, 1),
    (ABANDONED_PAGE_PURGE, 1),
];

/// The options that decide what mimalloc keeps committed, by name.
const RETENTION: [(&str, c_int); 8] = [
    ("purge_delay", PURGE_DELAY),
    ("arena_purge_mult", ARENA_PURGE_MULT),
    ("abandoned_page_purge", ABANDONED_PAGE_PURGE),
    ("purge_decommits", PURGE_DECOMMITS),
    ("purge_extend_delay", PURGE_EXTEND_DELAY),
    ("eager_commit", EAGER_COMMIT),
    ("arena_eager_commit", ARENA_EAGER_COMMIT),
    ("arena_reserve_kib", ARENA_RESERVE),
];

/// The bundled mimalloc's `mi_stats_t`, in 8-byte words, and its `MI_STAT_VERSION`.
const STATS_WORDS: usize = 521;
const STATS_VERSION: u64 = 4;
/// The word of `mi_stats_t` holding `committed.current`.
const COMMITTED: usize = 10;

// The mimalloc static library provides these.
unsafe extern "C" {
    fn mi_option_set(option: c_int, value: c_long);
    fn mi_option_get(option: c_int) -> c_long;
    fn mi_collect(force: bool);
    fn mi_version() -> c_int;
    fn mi_stats_get(stats: *mut u64) -> bool;
}

/// The data service frees gigabytes as preparation and scans finish, on worker
/// threads that may then idle. mimalloc returns a thread's freed pages after a
/// delay, but only when that thread next allocates, and never those of a thread
/// that ended, so the memory stayed committed. Call first in `main`, before any
/// other thread runs: mimalloc's options are not synchronized. A finished thread's
/// pages are now returned at once, and a running thread's [`PURGE_DELAY_MS`] after
/// it frees them, at its next allocation or [`return_freed`].
pub fn return_freed_memory_promptly() {
    for (option, value) in SET {
        // SAFETY: setting an option has no preconditions; mimalloc reads them as it purges.
        unsafe { mi_option_set(option, value) };
    }
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

/// The bytes mimalloc holds committed from the system, as it counts them; None when its
/// statistics are laid out otherwise. It allocates nothing, so the allocator may call it.
pub fn committed() -> Option<usize> {
    let mut stats = [0u64; STATS_WORDS];
    stats[0] = (STATS_WORDS * 8) as u64;
    stats[1] = STATS_VERSION;
    // SAFETY: a block of the size the first word gives; mimalloc refuses any other size or
    // version.
    let read = unsafe { mi_stats_get(stats.as_mut_ptr()) };
    read.then(|| stats[COMMITTED] as usize)
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
