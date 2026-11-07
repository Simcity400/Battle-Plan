//! Large blocks' own pages: a reservation with room to grow in place, committed to the page.

use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};
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

/// The kept blocks, linked oldest first; 0 is none.
struct Kept {
    head: usize,
    tail: usize,
}

static KEPT: Mutex<Kept> = Mutex::new(Kept { head: 0, tail: 0 });

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
}

/// Releases a chain [`Kept::expire`] unlinked, once [`KEPT`]'s lock is let go.
fn release_chain(mut at: usize) {
    while at != 0 {
        // SAFETY: a header [`Kept::push`] wrote, read before its block goes.
        let Node { next, bytes, .. } = unsafe { node(at).read() };
        release(at as *mut u8, bytes);
        at = next;
    }
}

fn kept() -> std::sync::MutexGuard<'static, Kept> {
    KEPT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn release(base: *mut u8, bytes: usize) {
    // SAFETY: the base of a reservation [`alloc`] made.
    unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    COMMITTED.fetch_sub(bytes, Relaxed);
}

/// A block of `size` bytes, zeroed when asked.
pub(crate) unsafe fn alloc(size: usize, zeroed: bool) -> *mut u8 {
    let bytes = rounded(size);
    let (reused, expired) = {
        let mut kept = kept();
        let reused = kept.take(bytes);
        (reused, kept.expire(Instant::now()))
    };
    release_chain(expired);
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

/// Frees a block of `size` bytes: kept for reuse until the delay passes.
pub(crate) unsafe fn free(pointer: *mut u8, size: usize) {
    let now = Instant::now();
    let expired = {
        let mut kept = kept();
        kept.push(pointer as usize, rounded(size), now);
        kept.expire(now)
    };
    release_chain(expired);
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

/// Releases the kept blocks past their delay.
pub(crate) fn release_expired() {
    let expired = kept().expire(Instant::now());
    release_chain(expired);
}
