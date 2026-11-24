
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_READWRITE,
    VirtualAlloc, VirtualFree, VirtualQuery,
};

pub(crate) const LARGE: usize = 64 << 10;
const PAGE: usize = 4 << 10;
const GRANULE: usize = 64 << 10;

static COMMITTED: AtomicUsize = AtomicUsize::new(0);

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

pub(crate) fn contention() -> (u64, u64, u64) {
    (
        CALLS.load(Relaxed),
        OVERLAPPED.load(Relaxed),
        CONTENDED.load(Relaxed),
    )
}

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

const DELAY: Duration = Duration::from_millis(crate::PURGE_DELAY_MS as u64);

#[repr(C)]
struct Node {
    next: usize,
    bytes: usize,
    freed: Instant,
}

struct Kept {
    head: usize,
    tail: usize,
}

const CLASSES: usize = 32;

static KEPT: [Mutex<Kept>; CLASSES] =
    [const { Mutex::new(Kept { head: 0, tail: 0 }) }; CLASSES];

static FILLED: AtomicU32 = AtomicU32::new(0);

static SWEEP: AtomicUsize = AtomicUsize::new(0);

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

    fn take(&mut self, bytes: usize) -> Option<(usize, usize)> {
        let (mut best, mut best_before, mut best_bytes) = (0, 0, usize::MAX);
        let (mut before, mut at) = (0, self.head);
        while at != 0 {
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

fn release_chain(mut at: usize) {
    while at != 0 {
        // SAFETY: a header [`Kept::push`] wrote, read before its block goes.
        let Node { next, bytes, .. } = unsafe { node(at).read() };
        release(at as *mut u8, bytes);
        at = next;
    }
}

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

fn filled(class: usize, kept: &Kept) {
    let bit = 1 << class;
    if kept.is_empty() {
        FILLED.fetch_and(!bit, Relaxed);
    } else {
        FILLED.fetch_or(bit, Relaxed);
    }
}

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

/// # Safety
/// [`own`] accepts the layout `size` comes from.
pub(crate) unsafe fn alloc(size: usize, zeroed: bool) -> *mut u8 {
    unsafe { alloc_at(size, zeroed, Instant::now()) }
}

/// # Safety
/// As [`alloc`].
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
    let reserved = (bytes * 2).next_multiple_of(GRANULE);
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

/// # Safety
/// `pointer` is a live block [`alloc`] returned, `size` its current size after any [`resize`],
/// and the block is never used again.
pub(crate) unsafe fn free(pointer: *mut u8, size: usize) {
    unsafe { free_at(pointer, size, Instant::now()) }
}

/// # Safety
/// As [`free`].
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

/// # Safety
/// `pointer` is a live block [`alloc`] returned, `old` its current size, and nothing else uses
/// the block during the call. [`own`] accepts the layout `new` comes from. On `true` the block
/// holds `new` bytes and that is its current size; on `false` it is unchanged.
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
    let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let read = unsafe {
        VirtualQuery(
            tail.cast(),
            &raw mut region,
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

pub(crate) fn release_expired() {
    release_expired_at(Instant::now());
}

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

    #[test]
    fn a_class_holds_one_octave_and_every_higher_class_fits_a_lower_request() {
        assert_eq!(class_of(rounded(LARGE + 1)), 0);
        assert_eq!(class_of(2 * LARGE), 1);
        assert_eq!(class_of(2 * LARGE - PAGE), 0);
        assert_eq!(class_of(usize::MAX / 2), CLASSES - 1);
        assert!(reuse(usize::MAX / 2, CLASSES - 1, Instant::now()).is_none());
        for class in 0..8 {
            let smallest = LARGE << class;
            let largest = (LARGE << (class + 1)) - PAGE;
            assert_eq!(class_of(smallest), class);
            assert_eq!(class_of(largest), class);
            assert!(LARGE << (class + 1) > largest);
        }
    }

    #[test]
    fn large_blocks_are_their_own_pages_reused_by_size_and_given_back() {
        use std::collections::BTreeSet;
        use std::sync::Mutex;
        let started = Instant::now();
        let before = committed();
        let size = 500 * LARGE + PAGE;
        unsafe {
            let first = alloc_at(size, false, started);
            let once = committed();
            assert_eq!(once, before + rounded(size));
            free_at(first, size, started);
            assert_eq!(committed(), once, "a kept block stays committed");
            let again = alloc_at(size, false, started);
            assert_eq!(again, first, "the kept block's own pages were reused");
            assert_eq!(committed(), once);
            free_at(again, size, started);
            let smaller = size / 4;
            let shrunk = alloc_at(smaller, false, started);
            assert_eq!(shrunk, first, "a higher class serves a smaller request");
            assert_eq!(committed(), once - (rounded(size) - rounded(smaller)));
            free_at(shrunk, smaller, started);
        }
        release_expired_at(started + DELAY);
        assert_eq!(committed(), before, "the kept block went back past its delay");

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
                            let base = unsafe { alloc(size, at % 2 == 0) };
                            assert!(!base.is_null(), "a large block was refused");
                            assert_eq!(base as usize % PAGE, 0, "a block starts on a page");
                            if at % 2 == 0 {
                                let zeroed = unsafe { std::slice::from_raw_parts(base, size) };
                                assert!(zeroed.iter().all(|byte| *byte == 0), "zeroed as asked");
                            }
                            unsafe { base.write_bytes(thread, size) };
                            assert!(
                                live.lock().unwrap().insert(base as usize),
                                "no live block is handed out twice"
                            );
                            held.push((base, size));
                        }
                        for (base, size) in held {
                            let mine = unsafe { std::slice::from_raw_parts(base, size) };
                            assert!(
                                mine.iter().all(|byte| *byte == thread),
                                "a live block is nobody else's"
                            );
                            live.lock().unwrap().remove(&(base as usize));
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
