//! Large blocks' own pages: a reservation with room to grow in place, committed to the page.
//!
//! The blocks freed and kept for reuse are held in one list per size class, each behind its own
//! lock that is only ever tried, never waited for. The thread that draws the map allocates on its
//! frame path, and a lock it waits for is a lock a descheduled thread can hold: a call that finds
//! its class held takes fresh pages, or releases at once, instead of waiting.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_READWRITE,
    VirtualAlloc, VirtualFree, VirtualQuery,
};

/// A larger block has pages of its own: 13-year preparations measured 16 KiB to 1 MiB.
pub(crate) const LARGE: usize = 64 << 10;
const PAGE: usize = 4 << 10;
/// The system reserves address space in these, so no block is aligned past them.
const GRANULE: usize = 64 << 10;

/// What large blocks hold committed, kept ones included.
static COMMITTED: AtomicUsize = AtomicUsize::new(0);

/// Large-block calls, those that found another in flight (what one list behind one lock would
/// have made wait), and those that found their own class held and so kept nothing.
static CALLS: AtomicU64 = AtomicU64::new(0);
static OVERLAPPED: AtomicU64 = AtomicU64::new(0);
static CONTENDED: AtomicU64 = AtomicU64::new(0);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub(crate) fn own(size: usize, align: usize) -> bool {
    size > LARGE && align <= GRANULE
}

pub(crate) fn rounded(size: usize) -> usize {
    size.next_multiple_of(PAGE)
}

pub(crate) fn committed() -> usize {
    COMMITTED.load(Relaxed)
}

/// Large-block calls, those that overlapped another and those that found their class held.
pub(crate) fn contention() -> (u64, u64, u64) {
    (
        CALLS.load(Relaxed),
        OVERLAPPED.load(Relaxed),
        CONTENDED.load(Relaxed),
    )
}

/// Counts one large-block call while it runs, and whether it overlapped another.
struct InFlight;

impl InFlight {
    fn enter() -> Self {
        CALLS.fetch_add(1, Relaxed);
        if IN_FLIGHT.fetch_add(1, Relaxed) > 0 {
            OVERLAPPED.fetch_add(1, Relaxed);
        }
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Relaxed);
    }
}

/// Freed blocks stay committed this long, as mimalloc keeps its pages, so the next block of
/// about their size reuses their pages instead of faulting fresh ones in: half the bytes
/// committed over a 13-year preparation.
const DELAY: Duration = Duration::from_millis(crate::PURGE_DELAY_MS as u64);

/// A kept block's header, written at its base while it waits: its own pages hold the list.
#[repr(C)]
struct Node {
    next: usize,
    bytes: usize,
    freed: Instant,
}

/// The kept blocks of one size class, linked oldest first; 0 is none.
struct Kept {
    head: usize,
    tail: usize,
}

/// One list per octave of block size from [`LARGE`] up: class `k` holds the blocks of
/// `2^(16+k)` bytes up to but not including twice that, so every block of a higher class fits
/// any request of a lower one.
const CLASSES: usize = 32;

static KEPT: [Mutex<Kept>; CLASSES] =
    [const { Mutex::new(Kept { head: 0, tail: 0 }) }; CLASSES];

/// A bit per class that may hold a block, so a request skips the empty classes above its own.
static FILLED: AtomicU32 = AtomicU32::new(0);

/// The class swept for expired blocks next, so classes nothing asks for still give their pages
/// back after the delay.
static SWEEP: AtomicUsize = AtomicUsize::new(0);

/// The class of a rounded block size, which is always more than [`LARGE`].
fn class_of(bytes: usize) -> usize {
    (bytes.max(LARGE).ilog2() as usize - LARGE.ilog2() as usize).min(CLASSES - 1)
}

fn node(base: usize) -> *mut Node {
    base as *mut Node
}

impl Kept {
    fn push(&mut self, base: usize, bytes: usize, freed: Instant) {
        // SAFETY: a freed block's committed pages, larger than a header and page aligned.
        unsafe {
            node(base).write(Node {
                next: 0,
                bytes,
                freed,
            })
        };
        match self.tail {
            0 => self.head = base,
            tail => unsafe { (*node(tail)).next = base },
        }
        self.tail = base;
    }

    /// Unlinks the blocks past the delay, oldest first, returning their chain.
    fn expire(&mut self, now: Instant) -> usize {
        let first = self.head;
        let mut last = 0;
        let mut at = self.head;
        // SAFETY: every linked base holds a header [`Self::push`] wrote.
        while at != 0 && now.duration_since(unsafe { (*node(at)).freed }) >= DELAY {
            last = at;
            at = unsafe { (*node(at)).next };
        }
        if last == 0 {
            return 0;
        }
        unsafe { (*node(last)).next = 0 };
        self.head = at;
        if at == 0 {
            self.tail = 0;
        }
        first
    }

    /// The kept block holding the fewest bytes of at least `bytes`, unlinked, with its size.
    fn take(&mut self, bytes: usize) -> Option<(usize, usize)> {
        let (mut best, mut best_before, mut best_bytes) = (0, 0, usize::MAX);
        let (mut before, mut at) = (0, self.head);
        while at != 0 {
            // SAFETY: every linked base holds a header [`Self::push`] wrote.
            let held = unsafe { (*node(at)).bytes };
            if held >= bytes && held < best_bytes {
                (best, best_before, best_bytes) = (at, before, held);
                if held == bytes {
                    break;
                }
            }
            before = at;
            at = unsafe { (*node(at)).next };
        }
        if best == 0 {
            return None;
        }
        let next = unsafe { (*node(best)).next };
        match best_before {
            0 => self.head = next,
            before => unsafe { (*node(before)).next = next },
        }
        if self.tail == best {
            self.tail = best_before;
        }
        Some((best, best_bytes))
    }

    /// The oldest kept block, unlinked, with its size: every block of a class above the
    /// request's holds more bytes than it asks for, so the first one fits.
    fn take_oldest(&mut self) -> Option<(usize, usize)> {
        let base = match self.head {
            0 => return None,
            base => base,
        };
        // SAFETY: a linked base holds a header [`Self::push`] wrote.
        let Node { next, bytes, .. } = unsafe { node(base).read() };
        self.head = next;
        if next == 0 {
            self.tail = 0;
        }
        Some((base, bytes))
    }

    fn is_empty(&self) -> bool {
        self.head == 0
    }
}

/// Releases a chain [`Kept::expire`] unlinked, once its class's lock is let go.
fn release_chain(mut at: usize) {
    while at != 0 {
        // SAFETY: a header [`Kept::push`] wrote, read before its block goes.
        let Node { next, bytes, .. } = unsafe { node(at).read() };
        release(at as *mut u8, bytes);
        at = next;
    }
}

/// Class `class`'s list, or none while another thread holds it: no caller waits here.
fn kept(class: usize) -> Option<MutexGuard<'static, Kept>> {
    match KEPT[class].try_lock() {
        Ok(held) => Some(held),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => {
            CONTENDED.fetch_add(1, Relaxed);
            None
        }
    }
}

/// Marks `class` by what its list now holds, so an empty one is skipped without its lock.
fn filled(class: usize, kept: &Kept) {
    let bit = 1 << class;
    if kept.is_empty() {
        FILLED.fetch_and(!bit, Relaxed);
    } else {
        FILLED.fetch_or(bit, Relaxed);
    }
}

/// Expires one class other than `touched`, so a class nothing asks for still gives its pages
/// back; a class held by another thread is left to the next call.
fn sweep(touched: usize, now: Instant) {
    let next = SWEEP.fetch_add(1, Relaxed) % CLASSES;
    if next == touched || FILLED.load(Relaxed) & (1 << next) == 0 {
        return;
    }
    let expired = match kept(next) {
        Some(mut kept) => {
            let expired = kept.expire(now);
            filled(next, &kept);
            expired
        }
        None => 0,
    };
    release_chain(expired);
}

/// A kept block of at least `bytes`, unlinked, with the bytes it holds.
fn reuse(bytes: usize, class: usize, now: Instant) -> Option<(usize, usize)> {
    let mut expired = 0;
    let mut reused = None;
    if let Some(mut kept) = kept(class) {
        reused = kept.take(bytes);
        expired = kept.expire(now);
        filled(class, &kept);
    }
    release_chain(expired);
    if reused.is_some() {
        return reused;
    }
    // Every block of a higher class holds more than this request asks for.
    // The classes above this one; the highest class has none above it.
    let mut above = FILLED.load(Relaxed) & u32::MAX.checked_shl(class as u32 + 1).unwrap_or(0);
    while above != 0 {
        let higher = above.trailing_zeros() as usize;
        above &= above - 1;
        if let Some(mut kept) = kept(higher) {
            reused = kept.take_oldest();
            filled(higher, &kept);
            if reused.is_some() {
                return reused;
            }
        }
    }
    None
}

fn release(base: *mut u8, bytes: usize) {
    // SAFETY: the base of a reservation [`alloc`] made.
    unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    COMMITTED.fetch_sub(bytes, Relaxed);
}

/// A block of `size` bytes, zeroed when asked.
pub(crate) unsafe fn alloc(size: usize, zeroed: bool) -> *mut u8 {
    // SAFETY: the caller's.
    unsafe { alloc_at(size, zeroed, Instant::now()) }
}

/// [`alloc`] as the clock stood at `now`, which decides what is kept and what expires.
unsafe fn alloc_at(size: usize, zeroed: bool, now: Instant) -> *mut u8 {
    let _call = InFlight::enter();
    let bytes = rounded(size);
    let class = class_of(bytes);
    let reused = reuse(bytes, class, now);
    sweep(class, now);
    if let Some((base, held)) = reused {
        let base = base as *mut u8;
        if held > bytes {
            // SAFETY: committed pages past the block, in its own reservation.
            unsafe { VirtualFree(base.add(bytes).cast(), held - bytes, MEM_DECOMMIT) };
            COMMITTED.fetch_sub(held - bytes, Relaxed);
        }
        if zeroed {
            unsafe { base.write_bytes(0, size) };
        }
        return base;
    }
    // Reserving costs no commit: twice the block, so it can grow in place.
    let reserved = (bytes * 2).next_multiple_of(GRANULE);
    // SAFETY: a fresh reservation, then its first pages committed; fresh pages are zero.
    let base = unsafe { VirtualAlloc(std::ptr::null(), reserved, MEM_RESERVE, PAGE_READWRITE) };
    if base.is_null() {
        return std::ptr::null_mut();
    }
    if unsafe { VirtualAlloc(base, bytes, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
        release(base.cast(), 0);
        return std::ptr::null_mut();
    }
    COMMITTED.fetch_add(bytes, Relaxed);
    base.cast()
}

/// Frees a block of `size` bytes: kept for reuse until the delay passes, or released here when
/// its class is held by another thread.
pub(crate) unsafe fn free(pointer: *mut u8, size: usize) {
    // SAFETY: the caller's.
    unsafe { free_at(pointer, size, Instant::now()) }
}

/// [`free`] as the clock stood at `now`, which decides what is kept and what expires.
unsafe fn free_at(pointer: *mut u8, size: usize, now: Instant) {
    let _call = InFlight::enter();
    let bytes = rounded(size);
    let class = class_of(bytes);
    let expired = match kept(class) {
        Some(mut kept) => {
            kept.push(pointer as usize, bytes, now);
            let expired = kept.expire(now);
            filled(class, &kept);
            expired
        }
        None => {
            release(pointer, bytes);
            0
        }
    };
    release_chain(expired);
    sweep(class, now);
}

/// Grows or shrinks a block of `old` bytes to `new` in its own reservation; false when it
/// does not fit there.
pub(crate) unsafe fn resize(pointer: *mut u8, old: usize, new: usize) -> bool {
    let (held, wanted) = (rounded(old), rounded(new));
    if wanted <= held {
        if wanted < held {
            // SAFETY: committed pages past the shrunk block, in its own reservation.
            unsafe { VirtualFree(pointer.add(wanted).cast(), held - wanted, MEM_DECOMMIT) };
            COMMITTED.fetch_sub(held - wanted, Relaxed);
        }
        return true;
    }
    let tail = unsafe { pointer.add(held) };
    // SAFETY: zeroed is a valid MEMORY_BASIC_INFORMATION, which VirtualQuery fills.
    let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let read = unsafe {
        VirtualQuery(
            tail.cast(),
            &mut region,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if read == 0
        || region.AllocationBase != pointer.cast()
        || region.State != MEM_RESERVE
        || region.RegionSize < wanted - held
    {
        return false;
    }
    // SAFETY: reserved, uncommitted pages of the block's own reservation.
    if unsafe { VirtualAlloc(tail.cast(), wanted - held, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
        return false;
    }
    COMMITTED.fetch_add(wanted - held, Relaxed);
    true
}

/// Releases the kept blocks past their delay, in every class. Only a class another thread holds
/// keeps its blocks, until the next call.
pub(crate) fn release_expired() {
    release_expired_at(Instant::now());
}

/// [`release_expired`] as the clock stood at `now`.
fn release_expired_at(now: Instant) {
    for class in 0..CLASSES {
        if FILLED.load(Relaxed) & (1 << class) == 0 {
            continue;
        }
        let expired = match kept(class) {
            Some(mut kept) => {
                let expired = kept.expire(now);
                filled(class, &kept);
                expired
            }
            None => 0,
        };
        release_chain(expired);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every class holds one octave, and every higher class fits a lower class's request.
    #[test]
    fn a_class_holds_one_octave_and_every_higher_class_fits_a_lower_request() {
        assert_eq!(class_of(rounded(LARGE + 1)), 0);
        assert_eq!(class_of(2 * LARGE), 1);
        assert_eq!(class_of(2 * LARGE - PAGE), 0);
        assert_eq!(class_of(usize::MAX / 2), CLASSES - 1);
        // The highest class looks at no class above it, whatever is kept.
        assert!(reuse(usize::MAX / 2, CLASSES - 1, Instant::now()).is_none());
        for class in 0..8 {
            let smallest = LARGE << class;
            let largest = (LARGE << (class + 1)) - PAGE;
            assert_eq!(class_of(smallest), class);
            assert_eq!(class_of(largest), class);
            assert!(LARGE << (class + 1) > largest);
        }
    }

    /// The only test that allocates: the clock is given, never waited on, and the committed
    /// bytes are this process's, so the one test owns both.
    #[test]
    fn large_blocks_are_their_own_pages_reused_by_size_and_given_back() {
        use std::collections::BTreeSet;
        use std::sync::Mutex;
        let started = Instant::now();
        let before = committed();
        let size = 500 * LARGE + PAGE;
        // SAFETY: each block below is freed exactly once, at the same clock reading, so
        // nothing it keeps expires meanwhile.
        unsafe {
            let first = alloc_at(size, false, started);
            let once = committed();
            assert_eq!(once, before + rounded(size));
            free_at(first, size, started);
            assert_eq!(committed(), once, "a kept block stays committed");
            let again = alloc_at(size, false, started);
            assert_eq!(again, first, "the kept block's own pages were reused");
            assert_eq!(committed(), once);
            // A smaller request takes a kept block of a higher class and gives back the rest.
            free_at(again, size, started);
            let smaller = size / 4;
            let shrunk = alloc_at(smaller, false, started);
            assert_eq!(shrunk, first, "a higher class serves a smaller request");
            assert_eq!(committed(), once - (rounded(size) - rounded(smaller)));
            free_at(shrunk, smaller, started);
        }
        release_expired_at(started + DELAY);
        assert_eq!(committed(), before, "the kept block went back past its delay");

        // Mixed sizes at once on eight threads: every block is its own committed memory and
        // the writes of one are never another's.
        let sizes = [
            LARGE + 1,
            LARGE + PAGE,
            3 * LARGE,
            7 * LARGE + 13,
            64 * LARGE,
            300 * LARGE + 7,
        ];
        let live: Mutex<BTreeSet<usize>> = Mutex::new(BTreeSet::new());
        std::thread::scope(|scope| {
            for thread in 1..=8u8 {
                let live = &live;
                scope.spawn(move || {
                    for round in 0..64usize {
                        let mut held = Vec::new();
                        for (at, size) in sizes.iter().enumerate() {
                            let size = size + round * PAGE;
                            // SAFETY: a size this module owns, freed below exactly once.
                            let base = unsafe { alloc(size, at % 2 == 0) };
                            assert!(!base.is_null(), "a large block was refused");
                            assert_eq!(base as usize % PAGE, 0, "a block starts on a page");
                            if at % 2 == 0 {
                                // SAFETY: the block just allocated, zeroed as asked.
                                let zeroed = unsafe { std::slice::from_raw_parts(base, size) };
                                assert!(zeroed.iter().all(|byte| *byte == 0), "zeroed as asked");
                            }
                            // SAFETY: this thread alone holds the block until it frees it.
                            unsafe { base.write_bytes(thread, size) };
                            assert!(
                                live.lock().unwrap().insert(base as usize),
                                "no live block is handed out twice"
                            );
                            held.push((base, size));
                        }
                        for (base, size) in held {
                            // SAFETY: written by this thread alone since it was allocated.
                            let mine = unsafe { std::slice::from_raw_parts(base, size) };
                            assert!(
                                mine.iter().all(|byte| *byte == thread),
                                "a live block is nobody else's"
                            );
                            live.lock().unwrap().remove(&(base as usize));
                            // SAFETY: allocated above, freed once.
                            unsafe { free(base, size) };
                        }
                    }
                });
            }
        });
        assert!(live.into_inner().unwrap().is_empty());
        release_expired_at(Instant::now() + DELAY);
        assert_eq!(committed(), before, "every block's pages came back");
        let (calls, _, _) = contention();
        assert!(calls >= 2 * 8 * 64 * sizes.len() as u64);
    }
}
