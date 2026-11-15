//! The collector's allocator: one private heap and no process-heap fallback.
//! Each thread keeps its own free blocks per size class, so steady allocation
//! and freeing take no lock. Acquisition workers allocate what the frame
//! pipeline frees on other threads, so whole batches pass between threads
//! through a lock-free depot. A block over the small limit is a page
//! reservation of its own, kept for reuse in lock-free lists. Only a refill
//! the depot cannot serve, a free into a full depot and an over-aligned small
//! block reach the heap's lock, and a thread that holds the game never frees
//! into it. Detach destroys the heap, the depot and every reservation.
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{
        Kernel::SLIST_HEADER,
        Memory::{
            HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy, HeapFree, MEM_COMMIT,
            MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VIRTUAL_ALLOCATION_TYPE,
            VirtualAlloc, VirtualFree,
        },
        Threading::{InterlockedFlushSList, InterlockedPopEntrySList, InterlockedPushEntrySList},
    },
};

// Collector requires Windows x64; ordinary heap blocks are 16-byte aligned.
const HEAP_ALIGNMENT: usize = 16;
const HEADER: usize = size_of::<*mut u8>();
const DLL_PROCESS_DETACH: u32 = 0;
/// Small classes: 16-byte steps to 1 KiB, then powers of two to `SMALL_LIMIT`.
const FINE_CLASSES: usize = 64;
const SMALL_LIMIT: usize = 32 * 1024;
const CLASSES: usize = FINE_CLASSES + 5;
/// Free bytes one thread keeps per class: its list and one spare batch.
const CACHE_CLASS_BYTES: usize = 64 * 1024;
/// Free bytes the depot keeps over every class; beyond it a thread that does
/// not hold the game frees blocks back to the heap.
const DEPOT_BYTES: usize = 32 * 1024 * 1024;
const _: () = assert!(SMALL_LIMIT <= CACHE_CLASS_BYTES / 2);
const PAGE: usize = 4096;
/// Windows reserves address space in these steps, so every reservation's base is aligned to it.
const GRANULARITY: usize = 64 * 1024;
/// A reservation's record, then the body's base pointer in the word below the body.
const RECORD_BYTES: usize = 64;
const _: () = assert!(size_of::<Record>() + HEADER <= RECORD_BYTES);
/// Power-of-two reservation sizes from `GRANULARITY` up.
const RESERVATIONS: usize = (usize::BITS - GRANULARITY.trailing_zeros()) as usize;
/// Committed bytes free reservations keep; beyond it a thread that does not hold
/// the game decommits a freed body.
const KEPT_BYTES: usize = 64 * 1024 * 1024;

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

/// The blocks of `class` a thread passes to or takes from the depot at once:
/// half its bound.
const fn batch(class: usize) -> u32 {
    (CACHE_CLASS_BYTES / 2 / class_size(class)) as u32
}

const fn batch_bytes(class: usize) -> usize {
    batch(class) as usize * class_size(class)
}

/// The next block of a list or batch: each free block's first word.
fn next(block: *mut u8) -> *mut *mut u8 {
    block.cast()
}

/// Where a batch's first block keeps its successor while the depot's list
/// link holds its first word: its second word, which every class's 16 bytes hold.
fn parked_next(block: *mut u8) -> *mut *mut u8 {
    block.cast::<*mut u8>().wrapping_add(1)
}

/// The reservation size of a large block, or `None` when it cannot exist.
fn reservation(size: usize, align: usize) -> Option<usize> {
    size.checked_add(RECORD_BYTES + align)?
        .checked_next_power_of_two()
        .map(|bytes| bytes.max(GRANULARITY))
}

fn reservation_list(reserved: usize) -> usize {
    (reserved.trailing_zeros() - GRANULARITY.trailing_zeros()) as usize
}

const fn page_end(bytes: usize) -> usize {
    bytes.div_ceil(PAGE) * PAGE
}

/// A Windows interlocked list: lock-free push and pop of 16-byte aligned entries,
/// safe against reuse of a popped entry.
#[repr(C, align(16))]
struct Stack(UnsafeCell<SLIST_HEADER>);

// SAFETY: the header is only touched by the interlocked list functions.
unsafe impl Sync for Stack {}

impl Stack {
    const fn new() -> Self {
        // An all-zero header is an empty list.
        Self(UnsafeCell::new(unsafe { std::mem::zeroed() }))
    }

    fn push(&self, entry: *mut u8) {
        unsafe { InterlockedPushEntrySList(self.0.get(), entry.cast()) };
    }

    fn pop(&self) -> *mut u8 {
        unsafe { InterlockedPopEntrySList(self.0.get()) }.cast()
    }

    fn clear(&self) {
        unsafe { InterlockedFlushSList(self.0.get()) };
    }
}

/// Calls into one lock and their QPC ticks since attach.
struct Counter {
    calls: AtomicU64,
    ticks: AtomicU64,
    max_ticks: AtomicU64,
}

impl Counter {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
            max_ticks: AtomicU64::new(0),
        }
    }

    fn add(&self, ticks: i64) {
        let ticks = ticks.max(0) as u64;
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.ticks.fetch_add(ticks, Ordering::Relaxed);
        self.max_ticks.fetch_max(ticks, Ordering::Relaxed);
    }

    fn read(&self) -> Calls {
        Calls {
            calls: self.calls.load(Ordering::Relaxed),
            ticks: self.ticks.load(Ordering::Relaxed),
            max_ticks: self.max_ticks.load(Ordering::Relaxed),
        }
    }
}

/// Calls that can wait on a lock another thread holds, their QPC ticks and the longest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Calls {
    pub calls: u64,
    pub ticks: u64,
    pub max_ticks: u64,
}

impl Calls {
    /// Calls and ticks since `earlier`; the longest stays the later total's.
    pub(crate) fn since(self, earlier: Self) -> Self {
        Self {
            calls: self.calls - earlier.calls,
            ticks: self.ticks - earlier.ticks,
            max_ticks: self.max_ticks,
        }
    }
}

/// One side's calls into the private heap's lock and into the address space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Waits {
    pub heap: Calls,
    pub pages: Calls,
}

impl Waits {
    pub(crate) fn since(self, earlier: Self) -> Self {
        Self {
            heap: self.heap.since(earlier.heap),
            pages: self.pages.since(earlier.pages),
        }
    }
}

struct Side {
    heap: Counter,
    pages: Counter,
}

impl Side {
    const fn new() -> Self {
        Self {
            heap: Counter::new(),
            pages: Counter::new(),
        }
    }

    fn read(&self) -> Waits {
        Waits {
            heap: self.heap.read(),
            pages: self.pages.read(),
        }
    }
}

const HOLD: usize = 0;
const OTHER: usize = 1;

/// One class's free blocks on one thread: up to a batch of them linked from
/// `head`, and a full batch aside in `spare`.
#[derive(Clone, Copy)]
struct List {
    head: *mut u8,
    count: u32,
    spare: *mut u8,
}

impl List {
    const EMPTY: Self = Self {
        head: ptr::null_mut(),
        count: 0,
        spare: ptr::null_mut(),
    };
}

struct Cache {
    lists: [List; CLASSES],
}

impl Cache {
    const EMPTY: Self = Self {
        lists: [List::EMPTY; CLASSES],
    };

    /// A free block of `class`, or null when neither this cache nor the depot keeps one.
    fn take(&mut self, class: usize, depot: &Depot) -> *mut u8 {
        let list = &mut self.lists[class];
        if list.count == 0 {
            let refill = if list.spare.is_null() {
                depot.take(class)
            } else {
                std::mem::replace(&mut list.spare, ptr::null_mut())
            };
            if refill.is_null() {
                return ptr::null_mut();
            }
            list.head = refill;
            list.count = batch(class);
        }
        let head = list.head;
        list.head = unsafe { next(head).read() };
        list.count -= 1;
        head
    }

    /// Keep a freed block of `class`; false only when `bounded` and the depot is full.
    fn keep(&mut self, class: usize, block: *mut u8, depot: &Depot, bounded: bool) -> bool {
        let list = &mut self.lists[class];
        if list.count == batch(class) {
            // The full list becomes the spare, passing the old spare on.
            if !list.spare.is_null() && !depot.keep(class, list.spare, bounded) {
                return false;
            }
            list.spare = list.head;
            list.head = ptr::null_mut();
            list.count = 0;
        }
        unsafe { next(block).write(list.head) };
        list.head = block;
        list.count += 1;
        true
    }
}

/// What the allocator keeps per thread. Constant-initialised without a
/// destructor, so it never allocates and is reachable for the thread's life.
struct Thread {
    /// The heap whose blocks `cache` holds; zero for none.
    heap: usize,
    /// Set by collector threads that never hold the game.
    off_hold: bool,
    /// Set when the thread flushed its cache on exit; later blocks bypass it.
    closed: bool,
    cache: Cache,
}

impl Thread {
    /// This thread's cache for heap `id`, emptied when it held another heap's
    /// blocks; `None` after the thread flushed it on exit.
    fn cache(&mut self, id: usize) -> Option<&mut Cache> {
        if self.closed {
            return None;
        }
        if self.heap != id {
            self.heap = id;
            self.cache = Cache::EMPTY;
        }
        Some(&mut self.cache)
    }
}

thread_local! {
    static THREAD: UnsafeCell<Thread> = const {
        UnsafeCell::new(Thread { heap: 0, off_hold: false, closed: false, cache: Cache::EMPTY })
    };
}

/// The calling thread's state. Valid for the thread's life; never held across
/// a call that can allocate.
fn thread() -> *mut Thread {
    THREAD.with(UnsafeCell::get)
}

/// Marks the calling collector thread as one that never holds the game: its
/// waits count apart from the hold path's, and it frees into the heap rather
/// than past the depot's bound. Every other thread, the game's own and the
/// acquisition workers, is on the hold path.
pub(crate) fn off_hold_path() {
    unsafe { (*thread()).off_hold = true };
}

fn side() -> usize {
    if unsafe { (*thread()).off_hold } { OTHER } else { HOLD }
}

/// Every class's full batches that threads passed on, and the bytes they hold.
struct Depot {
    classes: [Stack; CLASSES],
    bytes: AtomicUsize,
}

impl Depot {
    const fn new() -> Self {
        Self {
            classes: [const { Stack::new() }; CLASSES],
            bytes: AtomicUsize::new(0),
        }
    }

    /// Keep a full batch of `class`; false when `bounded` and the depot holds its bound.
    fn keep(&self, class: usize, batch_head: *mut u8, bounded: bool) -> bool {
        let bytes = batch_bytes(class);
        if bounded {
            let reserved = self
                .bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                    (held + bytes <= DEPOT_BYTES).then_some(held + bytes)
                });
            if reserved.is_err() {
                return false;
            }
        } else {
            self.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        unsafe { parked_next(batch_head).write(next(batch_head).read()) };
        self.classes[class].push(batch_head);
        true
    }

    /// A full batch of `class`, or null when none is kept.
    fn take(&self, class: usize) -> *mut u8 {
        let head = self.classes[class].pop();
        if !head.is_null() {
            unsafe { next(head).write(parked_next(head).read()) };
            self.bytes.fetch_sub(batch_bytes(class), Ordering::Relaxed);
        }
        head
    }

    fn clear(&self) {
        self.classes.iter().for_each(Stack::clear);
        self.bytes.store(0, Ordering::Relaxed);
    }
}

/// The start of a large block's reservation. Its first 16 bytes are the free
/// list's link; the pages to `committed` are committed.
#[repr(C)]
struct Record {
    link: [usize; 2],
    /// The next of every reservation made since attach.
    made: *mut Record,
    reserved: usize,
    committed: usize,
}

/// Every reservation, the free ones by size, and the bytes the free ones keep committed.
struct Pages {
    free: [Stack; RESERVATIONS],
    kept: AtomicUsize,
    made: AtomicPtr<Record>,
}

impl Pages {
    const fn new() -> Self {
        Self {
            free: [const { Stack::new() }; RESERVATIONS],
            kept: AtomicUsize::new(0),
            made: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn made(&self, record: *mut Record) {
        let mut head = self.made.load(Ordering::Relaxed);
        loop {
            unsafe { (*record).made = head };
            match self
                .made
                .compare_exchange_weak(head, record, Ordering::Release, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(current) => head = current,
            }
        }
    }
}

/// The heap's handle, zero once destroyed or when creation failed, and the
/// identity thread caches tag its blocks with.
struct Domain {
    handle: AtomicUsize,
    id: usize,
}

static DOMAINS: AtomicUsize = AtomicUsize::new(1);

pub(super) struct PrivateHeap {
    // OnceLock initialization allocates no Rust memory. A failed creation is
    // cached too; zero after detach is terminal, never a new allocation domain.
    heap: OnceLock<Domain>,
    depot: Depot,
    pages: Pages,
    sides: [Side; 2],
    #[cfg(test)]
    failures: AtomicUsize,
    /// Refuses the page reservation or commit this many calls ahead; zero refuses none.
    #[cfg(test)]
    page_refusal: AtomicUsize,
}

impl PrivateHeap {
    pub(super) const fn new() -> Self {
        Self {
            heap: OnceLock::new(),
            depot: Depot::new(),
            pages: Pages::new(),
            sides: [const { Side::new() }; 2],
            #[cfg(test)]
            failures: AtomicUsize::new(0),
            #[cfg(test)]
            page_refusal: AtomicUsize::new(0),
        }
    }

    fn domain(&self) -> &Domain {
        self.heap.get_or_init(|| {
            let id = DOMAINS.fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            if self.refuse(tests::CREATE) {
                return Domain {
                    handle: AtomicUsize::new(0),
                    id,
                };
            }
            Domain {
                handle: AtomicUsize::new(unsafe { HeapCreate(0, 0, 0) } as usize),
                id,
            }
        })
    }

    fn heap(&self) -> HANDLE {
        self.domain().handle.load(Ordering::Relaxed) as HANDLE
    }

    /// The hold path's calls since attach: the game's threads and the acquisition workers.
    pub(crate) fn hold(&self) -> Waits {
        self.sides[HOLD].read()
    }

    /// Calls since attach by the collector threads that never hold the game.
    pub(crate) fn other(&self) -> Waits {
        self.sides[OTHER].read()
    }

    fn heap_call<R>(&self, call: impl FnOnce() -> R) -> R {
        let begin = crate::clock::qpc();
        let result = call();
        self.sides[side()].heap.add(crate::clock::qpc() - begin);
        result
    }

    fn page_call<R>(&self, call: impl FnOnce() -> R) -> R {
        let begin = crate::clock::qpc();
        let result = call();
        self.sides[side()].pages.add(crate::clock::qpc() - begin);
        result
    }

    /// Reserve or commit pages, counted as a page call; null when the system refuses.
    fn virtual_alloc(
        &self,
        address: *const u8,
        bytes: usize,
        kind: VIRTUAL_ALLOCATION_TYPE,
    ) -> *mut u8 {
        #[cfg(test)]
        if self.refuse_page() {
            return ptr::null_mut();
        }
        self.page_call(|| unsafe { VirtualAlloc(address.cast(), bytes, kind, PAGE_READWRITE) })
            .cast()
    }

    unsafe fn allocate(&self, size: usize, align: usize, zeroed: bool) -> *mut u8 {
        let bytes = if size > SMALL_LIMIT {
            let Some(reserved) = reservation(size, align) else {
                return ptr::null_mut();
            };
            reserved
        } else if align <= HEAP_ALIGNMENT {
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
        let domain = self.domain();
        let heap = domain.handle.load(Ordering::Relaxed) as HANDLE;
        if heap.is_null() {
            return ptr::null_mut();
        }
        #[cfg(test)]
        if self.refuse(tests::ALLOCATE) {
            return ptr::null_mut();
        }
        if size > SMALL_LIMIT {
            return unsafe { self.allocate_large(bytes, size, align, zeroed) };
        }
        let flags = if zeroed { HEAP_ZERO_MEMORY } else { 0 };
        if align <= HEAP_ALIGNMENT
            && let Some(class) = class(bytes)
        {
            if let Some(cache) = unsafe { (*thread()).cache(domain.id) } {
                let kept = cache.take(class, &self.depot);
                if !kept.is_null() {
                    if zeroed {
                        unsafe { kept.write_bytes(0, bytes) };
                    }
                    return kept;
                }
            }
            return self
                .heap_call(|| unsafe { HeapAlloc(heap, flags, class_size(class)) })
                .cast();
        }
        let raw = self
            .heap_call(|| unsafe { HeapAlloc(heap, flags, bytes) })
            .cast::<u8>();
        if raw.is_null() {
            return raw;
        }
        // Layout alignment is a power of two. The checked reserve covers the
        // pointer-aligned header and every possible byte of alignment padding.
        let start = unsafe { raw.add(HEADER) };
        let aligned = unsafe { start.add(start.align_offset(align)) };
        unsafe { aligned.cast::<*mut u8>().sub(1).write(raw) };
        aligned
    }

    /// A large block in a reservation of `reserved` bytes: a free one of that
    /// size when kept, else a new one.
    unsafe fn allocate_large(
        &self,
        reserved: usize,
        size: usize,
        align: usize,
        zeroed: bool,
    ) -> *mut u8 {
        let list = &self.pages.free[reservation_list(reserved)];
        let mut record = list.pop().cast::<Record>();
        let committed = if record.is_null() {
            0
        } else {
            let committed = unsafe { (*record).committed };
            self.pages.kept.fetch_sub(committed, Ordering::Relaxed);
            committed
        };
        let fresh = record.is_null();
        if fresh {
            record = self
                .virtual_alloc(ptr::null(), reserved, MEM_RESERVE)
                .cast();
            if record.is_null() {
                return ptr::null_mut();
            }
        }
        let base = record.cast::<u8>();
        // The base is aligned to the granularity, so for any alignment up to it
        // the body sits at the same offset in every reservation.
        let offset = (base as usize + RECORD_BYTES).next_multiple_of(align) - base as usize;
        let end = page_end(offset + size);
        if end > committed
            && self
                .virtual_alloc(base.wrapping_add(committed), end - committed, MEM_COMMIT)
                .is_null()
        {
            if fresh {
                self.page_call(|| unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) });
            } else {
                self.pages.kept.fetch_add(committed, Ordering::Relaxed);
                list.push(base);
            }
            return ptr::null_mut();
        }
        if fresh {
            unsafe {
                (*record).reserved = reserved;
                (*record).committed = 0;
            }
            self.pages.made(record);
        }
        unsafe { (*record).committed = end.max(committed) };
        let body = unsafe { base.add(offset) };
        if zeroed && committed > offset {
            // Pages committed just now are zero; only reused ones hold old bytes.
            unsafe { body.write_bytes(0, size.min(committed - offset)) };
        }
        unsafe { body.cast::<*mut u8>().sub(1).write(base) };
        body
    }

    fn record(body: *mut u8) -> *mut Record {
        unsafe { body.cast::<*mut Record>().sub(1).read() }
    }

    /// Keep a large block's reservation for reuse. Past the bound a thread off
    /// the hold path decommits the body; the hold path never makes that call.
    unsafe fn free_large(&self, body: *mut u8) {
        let record = Self::record(body);
        let committed = unsafe { (*record).committed };
        let kept = if side() == OTHER {
            self.pages
                .kept
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |kept| {
                    (kept + committed <= KEPT_BYTES).then_some(kept + committed)
                })
                .is_ok()
        } else {
            self.pages.kept.fetch_add(committed, Ordering::Relaxed);
            true
        };
        if !kept {
            let base = record.cast::<u8>();
            if committed > PAGE
                && self.page_call(|| unsafe {
                    VirtualFree(base.add(PAGE).cast(), committed - PAGE, MEM_DECOMMIT)
                }) != 0
            {
                unsafe { (*record).committed = PAGE };
            }
            self.pages
                .kept
                .fetch_add(unsafe { (*record).committed }, Ordering::Relaxed);
        }
        let reserved = unsafe { (*record).reserved };
        self.pages.free[reservation_list(reserved)].push(record.cast());
    }

    /// Resize a large block within its reservation, or move it when it outgrows it.
    unsafe fn reallocate_large(
        &self,
        body: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> *mut u8 {
        let record = Self::record(body);
        let base = record.cast::<u8>();
        let (reserved, committed) = unsafe { ((*record).reserved, (*record).committed) };
        let offset = body as usize - base as usize;
        if offset + new_size > reserved {
            return unsafe { self.moved(body, layout, new_size) };
        }
        let end = page_end(offset + new_size);
        if end > committed {
            let grown =
                self.virtual_alloc(base.wrapping_add(committed), end - committed, MEM_COMMIT);
            if grown.is_null() {
                return ptr::null_mut();
            }
            unsafe { (*record).committed = end };
        } else if end < committed
            && self.page_call(|| unsafe {
                VirtualFree(base.add(end).cast(), committed - end, MEM_DECOMMIT)
            }) != 0
        {
            unsafe { (*record).committed = end };
        }
        body
    }

    /// Create the heap if needed, allocation-free. Process attach refuses the
    /// load when this fails, so no infallible allocation can meet a null heap.
    pub(super) fn ready(&self) -> bool {
        !self.heap().is_null()
    }

    /// Return the exiting thread's cached blocks: whole batches to the depot,
    /// the rest to the heap. Later blocks on this thread bypass its cache.
    pub(super) fn thread_exit(&self) {
        let Some(domain) = self.heap.get() else {
            return;
        };
        let heap = domain.handle.load(Ordering::Relaxed) as HANDLE;
        let thread = unsafe { &mut *thread() };
        let held = !thread.closed && thread.heap == domain.id;
        thread.closed = true;
        if heap.is_null() || !held {
            return;
        }
        let cache = std::mem::replace(&mut thread.cache, Cache::EMPTY);
        for (class, list) in cache.lists.into_iter().enumerate() {
            for (head, whole) in [(list.spare, true), (list.head, list.count == batch(class))] {
                if head.is_null() || (whole && self.depot.keep(class, head, true)) {
                    continue;
                }
                let mut block = head;
                while !block.is_null() {
                    let following = unsafe { next(block).read() };
                    self.heap_call(|| unsafe { HeapFree(heap, 0, block.cast()) });
                    block = following;
                }
            }
        }
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
        let Some(domain) = self.heap.get() else {
            return false;
        };
        let heap = domain.handle.swap(0, Ordering::Relaxed) as HANDLE;
        self.depot.clear();
        self.pages.free.iter().for_each(Stack::clear);
        self.pages.kept.store(0, Ordering::Relaxed);
        let mut record = self.pages.made.swap(ptr::null_mut(), Ordering::Acquire);
        while !record.is_null() {
            let made = unsafe { (*record).made };
            unsafe { VirtualFree(record.cast(), 0, MEM_RELEASE) };
            record = made;
        }
        let thread = unsafe { &mut *thread() };
        if thread.heap == domain.id {
            thread.cache = Cache::EMPTY;
        }
        !heap.is_null() && unsafe { HeapDestroy(heap) } != 0
    }

    #[cfg(test)]
    fn refuse(&self, operation: usize) -> bool {
        self.failures.fetch_and(!operation, Ordering::Relaxed) & operation != 0
    }

    #[cfg(test)]
    fn refuse_page(&self) -> bool {
        self.page_refusal
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |ahead| {
                ahead.checked_sub(1)
            })
            == Ok(1)
    }
}

// SAFETY: small blocks belong to this heap and pass between threads whole;
// large blocks are reservations this allocator made; over-aligned small blocks
// keep their original pointer in a header. Failure returns null without
// unwinding or calling capture accounting: engine_allocation retains that path.
unsafe impl GlobalAlloc for PrivateHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { self.allocate(layout.size(), layout.align(), false) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { self.allocate(layout.size(), layout.align(), true) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if layout.size() > SMALL_LIMIT {
            return unsafe { self.free_large(pointer) };
        }
        let domain = self.domain();
        let heap = domain.handle.load(Ordering::Relaxed) as HANDLE;
        let raw = if layout.align() <= HEAP_ALIGNMENT {
            if let Some(class) = class(layout.size()) {
                let thread = unsafe { &mut *thread() };
                let bounded = thread.off_hold;
                if let Some(cache) = thread.cache(domain.id)
                    && cache.keep(class, pointer, &self.depot, bounded)
                {
                    return;
                }
            }
            pointer
        } else {
            unsafe { pointer.cast::<*mut u8>().sub(1).read() }
        };
        self.heap_call(|| unsafe { HeapFree(heap, 0, raw.cast()) });
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        #[cfg(test)]
        if layout.align() <= HEAP_ALIGNMENT && self.refuse(tests::REALLOCATE) {
            return ptr::null_mut();
        }
        if layout.size() > SMALL_LIMIT && new_size > SMALL_LIMIT {
            return unsafe { self.reallocate_large(pointer, layout, new_size) };
        }
        if layout.align() <= HEAP_ALIGNMENT
            && let (Some(old), Some(new)) = (class(layout.size()), class(new_size))
            && old == new
        {
            return pointer;
        }
        // A class block keeps its class's whole size, and an over-aligned block
        // would change its padding, so either moves.
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
}

#[cfg(test)]
mod tests;
