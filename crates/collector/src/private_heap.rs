//! The collector's allocator: one private heap and no process-heap fallback.
//! Each hour reallocates much the same small blocks, so they recycle through
//! per-processor size-class lists: the hot path takes an uncontended lock
//! instead of the heap's shared one. Acquisition workers allocate what the
//! frame pipeline frees on other processors, so a shard passes whole batches
//! to a shared depot that refills the others. Detach destroys the heap, the
//! lists and the depot.
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{
        Memory::{
            HEAP_REALLOC_IN_PLACE_ONLY, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy,
            HeapFree, HeapReAlloc,
        },
        Performance::QueryPerformanceFrequency,
        Threading::{
            AcquireSRWLockExclusive, GetCurrentProcessorNumber, ReleaseSRWLockExclusive, SRWLOCK,
            SRWLOCK_INIT, TryAcquireSRWLockExclusive,
        },
    },
};

// Collector requires Windows x64; ordinary heap blocks are 16-byte aligned.
const HEAP_ALIGNMENT: usize = 16;
/// Growth to this size copies outside the heap lock, which a moving HeapReAlloc holds.
const UNLOCKED_GROWTH: usize = 64 * 1024;
const HEADER: usize = size_of::<*mut u8>();
const DLL_PROCESS_DETACH: u32 = 0;
/// Small classes: 16-byte steps to 1 KiB, then powers of two to `SMALL_LIMIT`.
const FINE_CLASSES: usize = 64;
const SMALL_LIMIT: usize = 32 * 1024;
const CLASSES: usize = FINE_CLASSES + 5;
/// Processors share a list set only modulo this power of two.
const SHARDS: usize = 64;
/// Free bytes one shard keeps per class: its list and one spare batch.
const SHARD_CLASS_BYTES: usize = 64 * 1024;
/// Free bytes the depot keeps over every class; beyond it blocks go back to the heap.
const DEPOT_BYTES: usize = 32 * 1024 * 1024;
const _: () = assert!(SMALL_LIMIT <= SHARD_CLASS_BYTES / 2);

/// Blocked lock acquisitions or slow heap calls: how many, and their QPC ticks.
struct Contention {
    count: AtomicU64,
    ticks: AtomicU64,
    max: AtomicU64,
}

impl Contention {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
            max: AtomicU64::new(0),
        }
    }

    fn record(&self, ticks: i64) {
        let ticks = ticks.max(0) as u64;
        self.count.fetch_add(1, Ordering::Relaxed);
        self.ticks.fetch_add(ticks, Ordering::Relaxed);
        self.max.fetch_max(ticks, Ordering::Relaxed);
    }

    fn status(&self, frequency: u64) -> impl serde::Serialize {
        let ns = |ticks: u64| (u128::from(ticks) * 1_000_000_000 / u128::from(frequency.max(1))) as u64;
        crate::status::status!({
            "count": self.count.load(Ordering::Relaxed),
            "total_ns": ns(self.ticks.load(Ordering::Relaxed)),
            "max_ns": ns(self.max.load(Ordering::Relaxed)),
        })
    }
}

static SHARD_WAITS: Contention = Contention::new();
static DEPOT_WAITS: Contention = Contention::new();
static SLOW_HEAP_CALLS: Contention = Contention::new();
/// A heap call at least this long counts as slow: a millisecond of QPC ticks, set on first use.
static SLOW_HEAP_TICKS: AtomicU64 = AtomicU64::new(0);

fn qpc_frequency() -> u64 {
    let mut frequency = 0;
    unsafe { QueryPerformanceFrequency(&mut frequency) };
    frequency.max(1) as u64
}

/// Takes `lock`, counting the wait when another thread holds it.
fn acquire(lock: *mut SRWLOCK, waits: &Contention) {
    if unsafe { TryAcquireSRWLockExclusive(lock) } {
        return;
    }
    let begin = crate::clock::qpc();
    unsafe { AcquireSRWLockExclusive(lock) };
    waits.record(crate::clock::qpc() - begin);
}

/// Runs one heap call, counting it when it took a millisecond or more.
fn heap_call<T>(call: impl FnOnce() -> T) -> T {
    let begin = crate::clock::qpc();
    let result = call();
    let ticks = crate::clock::qpc() - begin;
    let mut slow = SLOW_HEAP_TICKS.load(Ordering::Relaxed);
    if slow == 0 {
        slow = qpc_frequency() / 1000;
        SLOW_HEAP_TICKS.store(slow, Ordering::Relaxed);
    }
    if ticks >= slow as i64 {
        SLOW_HEAP_CALLS.record(ticks);
    }
    result
}

/// Every tick any thread has spent blocked on the allocator's locks or in a slow heap call.
pub(crate) fn contention_ticks() -> u64 {
    SHARD_WAITS.ticks.load(Ordering::Relaxed)
        + DEPOT_WAITS.ticks.load(Ordering::Relaxed)
        + SLOW_HEAP_CALLS.ticks.load(Ordering::Relaxed)
}

/// The class of a block of `size` bytes, or `None` for a large block.
fn class(size: usize) -> Option<usize> {
    if size <= FINE_CLASSES * 16 {
        Some(size.saturating_sub(1) / 16)
    } else if size <= SMALL_LIMIT {
        Some(FINE_CLASSES + (size.next_power_of_two().trailing_zeros() - 11) as usize)
    } else {
        None
    }
}

/// The bytes every block of `class` holds.
const fn class_size(class: usize) -> usize {
    if class < FINE_CLASSES {
        (class + 1) * 16
    } else {
        2048 << (class - FINE_CLASSES)
    }
}

/// The blocks of `class` a shard passes to or takes from the depot at once:
/// half the shard's bound.
const fn batch(class: usize) -> u32 {
    (SHARD_CLASS_BYTES / 2 / class_size(class)) as u32
}

/// The next block of a list or batch: each free block's first word.
fn next(block: *mut u8) -> *mut *mut u8 {
    block.cast()
}

/// The next batch on the depot's stack: a batch's first block's second word,
/// which every class's 16 bytes hold.
fn next_batch(block: *mut u8) -> *mut *mut u8 {
    block.cast::<*mut u8>().wrapping_add(1)
}

/// One class's free blocks in a shard: up to a batch of them linked from
/// `head`, and a full batch aside in `spare`.
#[derive(Clone, Copy)]
struct List {
    head: *mut u8,
    count: u32,
    spare: *mut u8,
    hits: u64,
    misses: u64,
    kept: u64,
    returned: u64,
}

impl List {
    const EMPTY: Self = Self {
        head: ptr::null_mut(),
        count: 0,
        spare: ptr::null_mut(),
        hits: 0,
        misses: 0,
        kept: 0,
        returned: 0,
    };
}

/// One class's full batches that shards passed on.
struct Stack {
    top: *mut u8,
    batches: u32,
}

#[repr(align(64))]
struct Depot {
    lock: UnsafeCell<SRWLOCK>,
    stack: UnsafeCell<Stack>,
}

// SAFETY: `stack` is only touched under the exclusive SRW lock.
unsafe impl Sync for Depot {}

impl Depot {
    const fn new() -> Self {
        Self {
            lock: UnsafeCell::new(SRWLOCK_INIT),
            stack: UnsafeCell::new(Stack {
                top: ptr::null_mut(),
                batches: 0,
            }),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Stack) -> R) -> R {
        acquire(self.lock.get(), &DEPOT_WAITS);
        let result = f(unsafe { &mut *self.stack.get() });
        unsafe { ReleaseSRWLockExclusive(self.lock.get()) };
        result
    }
}

/// Every class's depot and the bytes they hold together.
struct Depots {
    classes: [Depot; CLASSES],
    bytes: AtomicUsize,
}

impl Depots {
    const fn new() -> Self {
        Self {
            classes: [const { Depot::new() }; CLASSES],
            bytes: AtomicUsize::new(0),
        }
    }

    /// Stack a full batch of `class`; false when the depot holds its bound.
    fn keep(&self, class: usize, batch_head: *mut u8) -> bool {
        let bytes = batch(class) as usize * class_size(class);
        let reserved = self
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                (held + bytes <= DEPOT_BYTES).then_some(held + bytes)
            });
        if reserved.is_err() {
            return false;
        }
        self.classes[class].with(|stack| {
            unsafe { next_batch(batch_head).write(stack.top) };
            stack.top = batch_head;
            stack.batches += 1;
        });
        true
    }

    /// A full batch of `class`, or null when none is stacked.
    fn take(&self, class: usize) -> *mut u8 {
        let top = self.classes[class].with(|stack| {
            let top = stack.top;
            if !top.is_null() {
                stack.top = unsafe { next_batch(top).read() };
                stack.batches -= 1;
            }
            top
        });
        if !top.is_null() {
            let bytes = batch(class) as usize * class_size(class);
            self.bytes.fetch_sub(bytes, Ordering::Relaxed);
        }
        top
    }

    fn clear(&self) {
        for depot in &self.classes {
            depot.with(|stack| {
                stack.top = ptr::null_mut();
                stack.batches = 0;
            });
        }
        self.bytes.store(0, Ordering::Relaxed);
    }
}

/// The small-block lists of the processors that map to one shard.
#[repr(align(64))]
struct Shard {
    lock: UnsafeCell<SRWLOCK>,
    lists: UnsafeCell<[List; CLASSES]>,
}

// SAFETY: `lists` is only touched under the exclusive SRW lock.
unsafe impl Sync for Shard {}

impl Shard {
    const fn new() -> Self {
        Self {
            lock: UnsafeCell::new(SRWLOCK_INIT),
            lists: UnsafeCell::new([List::EMPTY; CLASSES]),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut [List; CLASSES]) -> R) -> R {
        acquire(self.lock.get(), &SHARD_WAITS);
        let result = f(unsafe { &mut *self.lists.get() });
        unsafe { ReleaseSRWLockExclusive(self.lock.get()) };
        result
    }

    /// A free block of `class`, or null when neither the shard nor the depot
    /// keeps one.
    fn take(&self, class: usize, depots: &Depots) -> *mut u8 {
        self.with(|lists| {
            let list = &mut lists[class];
            if list.count == 0 {
                let refill = if list.spare.is_null() {
                    depots.take(class)
                } else {
                    std::mem::replace(&mut list.spare, ptr::null_mut())
                };
                if refill.is_null() {
                    list.misses += 1;
                    return ptr::null_mut();
                }
                list.head = refill;
                list.count = batch(class);
            }
            let head = list.head;
            list.head = unsafe { next(head).read() };
            list.count -= 1;
            list.hits += 1;
            head
        })
    }

    /// Keep a freed block of `class`; false when the shard and the depot are full.
    fn keep(&self, class: usize, block: *mut u8, depots: &Depots) -> bool {
        self.with(|lists| {
            let list = &mut lists[class];
            if list.count == batch(class) {
                // The full list becomes the spare, passing the old spare on.
                if !list.spare.is_null() && !depots.keep(class, list.spare) {
                    list.returned += 1;
                    return false;
                }
                list.spare = list.head;
                list.head = ptr::null_mut();
                list.count = 0;
            }
            unsafe { next(block).write(list.head) };
            list.head = block;
            list.count += 1;
            list.kept += 1;
            true
        })
    }
}

pub(super) struct PrivateHeap {
    // OnceLock initialization allocates no Rust memory. A failed creation is
    // cached too; zero after detach is terminal, never a new allocation domain.
    heap: OnceLock<AtomicUsize>,
    shards: [Shard; SHARDS],
    depots: Depots,
    #[cfg(test)]
    failures: AtomicUsize,
}

impl PrivateHeap {
    pub(super) const fn new() -> Self {
        Self {
            heap: OnceLock::new(),
            shards: [const { Shard::new() }; SHARDS],
            depots: Depots::new(),
            #[cfg(test)]
            failures: AtomicUsize::new(0),
        }
    }

    fn heap(&self) -> HANDLE {
        self.heap
            .get_or_init(|| {
                #[cfg(test)]
                if self.refuse(tests::CREATE) {
                    return AtomicUsize::new(0);
                }
                AtomicUsize::new(unsafe { HeapCreate(0, 0, 0) } as usize)
            })
            .load(Ordering::Relaxed) as HANDLE
    }

    /// The running processor's shard.
    fn shard(&self) -> &Shard {
        &self.shards[unsafe { GetCurrentProcessorNumber() } as usize & (SHARDS - 1)]
    }

    unsafe fn allocate(&self, size: usize, align: usize, zeroed: bool) -> *mut u8 {
        let bytes = if align <= HEAP_ALIGNMENT {
            size
        } else {
            let Some(bytes) = size
                .checked_add(align - 1)
                .and_then(|n| n.checked_add(HEADER))
            else {
                return ptr::null_mut();
            };
            bytes
        };
        let heap = self.heap();
        if heap.is_null() {
            return ptr::null_mut();
        }
        #[cfg(test)]
        if self.refuse(tests::ALLOCATE) {
            return ptr::null_mut();
        }
        let flags = if zeroed { HEAP_ZERO_MEMORY } else { 0 };
        if align <= HEAP_ALIGNMENT
            && let Some(class) = class(bytes)
        {
            let kept = self.shard().take(class, &self.depots);
            if kept.is_null() {
                return heap_call(|| unsafe { HeapAlloc(heap, flags, class_size(class)) }).cast();
            }
            if zeroed {
                unsafe { kept.write_bytes(0, bytes) };
            }
            return kept;
        }
        let raw = heap_call(|| unsafe { HeapAlloc(heap, flags, bytes) }).cast::<u8>();
        if raw.is_null() || align <= HEAP_ALIGNMENT {
            return raw;
        }
        // Layout alignment is a power of two. The checked reserve covers the
        // pointer-aligned header and every possible byte of alignment padding.
        let start = unsafe { raw.add(HEADER) };
        let aligned = unsafe { start.add(start.align_offset(align)) };
        unsafe { aligned.cast::<*mut u8>().sub(1).write(raw) };
        aligned
    }

    /// Create the heap if needed, allocation-free. Process attach refuses the
    /// load when this fails, so no infallible allocation can meet a null heap.
    pub(super) fn ready(&self) -> bool {
        !self.heap().is_null()
    }

    /// # Safety
    /// On FreeLibrary detach no caller may still own an allocation it will use,
    /// free or reallocate. Runtime must first restore/quiesce hooks and join all
    /// workers; the loader has run the unloading thread's TLS destructors before
    /// DllMain, and removes this module's callbacks before other threads exit.
    /// No Rust static has an automatic destructor. Do not allocate/log here.
    pub(super) unsafe fn detach(&self, reason: u32, reserved: *mut c_void) -> bool {
        if reason != DLL_PROCESS_DETACH || !reserved.is_null() {
            return false;
        }
        let Some(heap) = self.heap.get() else {
            return false;
        };
        let heap = heap.swap(0, Ordering::Relaxed) as HANDLE;
        for shard in &self.shards {
            shard.with(|lists| lists.iter_mut().for_each(|list| *list = List::EMPTY));
        }
        self.depots.clear();
        !heap.is_null() && unsafe { HeapDestroy(heap) } != 0
    }

    #[cfg(test)]
    fn refuse(&self, operation: usize) -> bool {
        self.failures.fetch_and(!operation, Ordering::Relaxed) & operation != 0
    }
}

// SAFETY: every block belongs to this serialized heap. Over-aligned blocks keep
// their original pointer in a header. Failure returns null without unwinding or
// calling capture accounting: engine_allocation retains that existing path.
unsafe impl GlobalAlloc for PrivateHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { self.allocate(layout.size(), layout.align(), false) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { self.allocate(layout.size(), layout.align(), true) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let raw = if layout.align() <= HEAP_ALIGNMENT {
            if let Some(class) = class(layout.size())
                && self.shard().keep(class, pointer, &self.depots)
            {
                return;
            }
            pointer
        } else {
            unsafe { pointer.cast::<*mut u8>().sub(1).read() }
        };
        heap_call(|| unsafe { HeapFree(self.heap(), 0, raw.cast()) });
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= HEAP_ALIGNMENT {
            #[cfg(test)]
            if self.refuse(tests::REALLOCATE) {
                return ptr::null_mut();
            }
            match (class(layout.size()), class(new_size)) {
                (Some(old), Some(new)) if old == new => return pointer,
                (None, None) => {}
                // A class block must keep its class's whole size, so it moves.
                _ => return unsafe { self.moved(pointer, layout, new_size) },
            }
            if new_size <= layout.size() || new_size < UNLOCKED_GROWTH {
                return heap_call(|| unsafe { HeapReAlloc(self.heap(), 0, pointer.cast(), new_size) })
                    .cast();
            }
            let grown = heap_call(|| unsafe {
                HeapReAlloc(
                    self.heap(),
                    HEAP_REALLOC_IN_PLACE_ONLY,
                    pointer.cast(),
                    new_size,
                )
            });
            if !grown.is_null() {
                return grown.cast();
            }
            // A failed in-place attempt leaves the old block intact.
            return unsafe { self.moved(pointer, layout, new_size) };
        }
        // Moving a raw over-aligned block changes its padding, and shrinking
        // could discard its payload offset.
        unsafe { self.moved(pointer, layout, new_size) }
    }
}

impl PrivateHeap {
    /// Copy into a new block and free the old one only after success, with no
    /// second capture allocation attempt; failure preserves the old block.
    unsafe fn moved(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let replacement = unsafe { self.allocate(new_size, layout.align(), false) };
        if !replacement.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(pointer, replacement, layout.size().min(new_size));
                self.dealloc(pointer, layout);
            }
        }
        replacement
    }

    /// The small-block counters summed over shards, copied without allocating.
    pub(super) fn status(&self) -> Snapshot {
        let mut totals = [Totals::default(); CLASSES];
        for shard in &self.shards {
            shard.with(|lists| {
                for (class, (total, list)) in totals.iter_mut().zip(lists.iter()).enumerate() {
                    let spare = if list.spare.is_null() { 0 } else { batch(class) };
                    total.cached += u64::from(list.count + spare);
                    total.hits += list.hits;
                    total.misses += list.misses;
                    total.kept += list.kept;
                    total.returned += list.returned;
                }
            });
        }
        for (class, (total, depot)) in totals.iter_mut().zip(&self.depots.classes).enumerate() {
            total.depot = depot.with(|stack| u64::from(stack.batches) * u64::from(batch(class)));
            total.cached += total.depot;
        }
        Snapshot(totals)
    }
}

#[derive(Clone, Copy, Default)]
struct Totals {
    hits: u64,
    misses: u64,
    kept: u64,
    returned: u64,
    cached: u64,
    depot: u64,
}

/// Per class: allocations served from a list or by the heap, frees kept or
/// returned to the heap, and blocks kept now, in the depot among them.
pub(super) struct Snapshot([Totals; CLASSES]);

impl serde::Serialize for Snapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let classes = (self.0.iter().enumerate())
            .filter(|(_, total)| total.hits + total.misses + total.kept + total.returned != 0)
            .map(|(class, total)| {
                crate::status::status!({
                    "bytes": class_size(class),
                    "hits": total.hits,
                    "misses": total.misses,
                    "kept": total.kept,
                    "returned": total.returned,
                    "cached": total.cached,
                    "depot": total.depot,
                })
            });
        let frequency = qpc_frequency();
        crate::status::status!({
            "classes": crate::status::sequence(classes),
            "shard_lock_waits": SHARD_WAITS.status(frequency),
            "depot_lock_waits": DEPOT_WAITS.status(frequency),
            "slow_heap_calls": SLOW_HEAP_CALLS.status(frequency),
            "scope": "cumulative since attach, per small-block class summed over processor shards: allocations served from a shard's lists or the depot (hits) or by the heap (misses), frees kept (kept) or returned to the heap because the shard and the depot held their bounds (returned), and blocks kept now (cached), those in the depot among them (depot); lock waits count acquisitions another thread held, slow heap calls those of a millisecond or more, on every collector thread",
        })
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
