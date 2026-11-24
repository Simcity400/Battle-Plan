use std::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
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

const HEAP_ALIGNMENT: usize = 16;
const HEADER: usize = size_of::<*mut u8>();
const DLL_PROCESS_DETACH: u32 = 0;
const FINE_CLASSES: usize = 64;
const SMALL_LIMIT: usize = 8 * 1024;
const CLASSES: usize = FINE_CLASSES + 3;
const PAGE_CLASS_FIRST: usize = FINE_CLASSES + 1;
const PAGE_CLASSES: usize = CLASSES - PAGE_CLASS_FIRST;
const CACHE_CLASS_BYTES: usize = 16 * 1024;
const MAX_BATCH: usize = 64;
const _: () = assert!(SMALL_LIMIT <= CACHE_CLASS_BYTES / 2);
const PAGE: usize = 4096;
const GRANULARITY: usize = 64 * 1024;
const RECORD_BYTES: usize = 64;
const COLD_OFFSET: usize = 256;
const COLD_STRIDE: usize = 16;
const SUBPAGE_START: usize = 512;
const WARM_PAGE_BLOCKS: usize = 8;
const WARM_SUBPAGE_PAGES: usize = 4;
const _: () = assert!(size_of::<Record>() + HEADER <= RECORD_BYTES);
const _: () = assert!(RECORD_BYTES <= COLD_OFFSET);
const _: () = assert!(SUBPAGE_START % HEAP_ALIGNMENT == 0);
const _: () = assert!(COLD_OFFSET + ((GRANULARITY - PAGE) / PAGE) * COLD_STRIDE <= SUBPAGE_START);
const _: () = assert!(SUBPAGE_START + 2048 <= PAGE);
const RESERVATIONS: usize = (usize::BITS - GRANULARITY.trailing_zeros()) as usize;
const KEPT_BYTES: usize = 8 * 1024 * 1024;
const TRIM_SLACK: usize = 256 * 1024;
const _: () = assert!(slab_bytes(CLASSES - 1) == GRANULARITY);

const fn is_page_class(class: usize) -> bool {
    class >= PAGE_CLASS_FIRST
}

const fn subpage_page0_count(size: usize) -> usize {
    (PAGE - SUBPAGE_START) / size
}

const fn subpage_per_page(size: usize) -> usize {
    PAGE / size
}

const fn slab_count(class: usize) -> usize {
    let size = class_size(class);
    if is_page_class(class) {
        (slab_bytes(class) - PAGE) / size
    } else {
        subpage_page0_count(size) + ((GRANULARITY - PAGE) / PAGE) * subpage_per_page(size)
    }
}

fn slab_block(base: *mut u8, class: usize, index: usize) -> *mut u8 {
    let size = class_size(class);
    if is_page_class(class) {
        // SAFETY: `index < slab_count(class)` keeps the pointer inside the slab.
        unsafe { base.add(PAGE + index * size) }
    } else {
        let p0 = subpage_page0_count(size);
        if index < p0 {
            // SAFETY: `SUBPAGE_START + index * size < PAGE` for `index < p0`.
            unsafe { base.add(SUBPAGE_START + index * size) }
        } else {
            let rem = index - p0;
            let pp = subpage_per_page(size);
            let page = 1 + rem / pp;
            let slot = rem % pp;
            // SAFETY: `page < GRANULARITY / PAGE` and `slot * size < PAGE`.
            unsafe { base.add(page * PAGE + slot * size) }
        }
    }
}

fn cold_node(block: *mut u8, class: usize) -> *mut u8 {
    let slab = (block as usize) & !(GRANULARITY - 1);
    let slot = (block as usize - slab - PAGE) / class_size(class);
    (slab + COLD_OFFSET + slot * COLD_STRIDE) as *mut u8
}

fn cold_block(node: *mut u8, class: usize) -> *mut u8 {
    let slab = (node as usize) & !(GRANULARITY - 1);
    let slot = (node as usize - slab - COLD_OFFSET) / COLD_STRIDE;
    (slab + PAGE + slot * class_size(class)) as *mut u8
}

fn cold_subpage_node(page: *mut u8) -> *mut u8 {
    let slab = (page as usize) & !(GRANULARITY - 1);
    let slot = (page as usize - slab - PAGE) / PAGE;
    (slab + COLD_OFFSET + slot * COLD_STRIDE) as *mut u8
}

fn cold_subpage_page(node: *mut u8) -> *mut u8 {
    let slab = (node as usize) & !(GRANULARITY - 1);
    let slot = (node as usize - slab - COLD_OFFSET) / COLD_STRIDE;
    (slab + PAGE + slot * PAGE) as *mut u8
}

fn class(size: usize) -> Option<usize> {
    if size <= FINE_CLASSES * 16 {
        Some(size.saturating_sub(1) / 16)
    } else if size <= SMALL_LIMIT {
        Some(FINE_CLASSES + (size.next_power_of_two().trailing_zeros() - 11) as usize)
    } else {
        None
    }
}

const fn class_size(class: usize) -> usize {
    if class < FINE_CLASSES {
        (class + 1) * 16
    } else {
        2048 << (class - FINE_CLASSES)
    }
}

const fn batch(class: usize) -> u32 {
    let count = CACHE_CLASS_BYTES / 2 / class_size(class);
    if count > MAX_BATCH {
        MAX_BATCH as u32
    } else {
        count as u32
    }
}

const fn slab_bytes(class: usize) -> usize {
    let size = class_size(class);
    let min_blocks = if (batch(class) as usize) < 4 {
        4
    } else {
        batch(class) as usize
    };
    let want = min_blocks * size + RECORD_BYTES;
    let mut bytes = GRANULARITY;
    while bytes < want {
        bytes *= 2;
    }
    bytes
}

fn next(block: *mut u8) -> *mut *mut u8 {
    block.cast()
}

#[expect(
    clippy::cast_ptr_alignment,
    reason = "small-class blocks are 16-byte multiples from a 64 KiB-aligned slab, so their second word is pointer-aligned"
)]
fn parked_next(block: *mut u8) -> *mut *mut u8 {
    block.cast::<*mut u8>().wrapping_add(1)
}

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

#[repr(C, align(64))]
struct Stack(UnsafeCell<SLIST_HEADER>);

// SAFETY: the header is only touched by the interlocked list functions.
unsafe impl Sync for Stack {}

impl Stack {
    const fn new() -> Self {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calls {
    pub calls: u64,
    pub ticks: u64,
    pub max_ticks: u64,
}

impl Calls {
    pub fn since(self, earlier: Self) -> Self {
        Self {
            calls: self.calls - earlier.calls,
            ticks: self.ticks - earlier.ticks,
            max_ticks: self.max_ticks,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Waits {
    pub heap: Calls,
    pub pages: Calls,
}

impl Waits {
    pub fn since(self, earlier: Self) -> Self {
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

    fn keep(&mut self, class: usize, block: *mut u8, depot: &Depot) -> bool {
        let mut spilled = false;
        let list = &mut self.lists[class];
        if list.count == batch(class) {
            if !list.spare.is_null() {
                depot.keep(class, list.spare);
                spilled = true;
            }
            list.spare = list.head;
            list.head = ptr::null_mut();
            list.count = 0;
        }
        unsafe { next(block).write(list.head) };
        list.head = block;
        list.count += 1;
        spilled
    }

    fn flush(&mut self, depot: &Depot) {
        for (class, list) in std::mem::replace(self, Cache::EMPTY)
            .lists
            .into_iter()
            .enumerate()
        {
            if !list.spare.is_null() {
                depot.keep(class, list.spare);
            }
            if list.head.is_null() {
                continue;
            }
            if list.count == batch(class) {
                depot.keep(class, list.head);
                continue;
            }
            let mut block = list.head;
            while !block.is_null() {
                // SAFETY: blocks in this thread's cache form a valid singly linked list.
                let following = unsafe { next(block).read() };
                depot.keep_single(class, block);
                block = following;
            }
        }
    }
}

struct Thread {
    heap: usize,
    off_hold: bool,
    closed: bool,
    flushed_epoch: u64,
    cache: Cache,
}

impl Thread {
    fn cache(&mut self, id: usize) -> Option<&mut Cache> {
        if self.closed {
            return None;
        }
        if self.heap != id {
            self.heap = id;
            self.flushed_epoch = 0;
            self.cache = Cache::EMPTY;
        }
        Some(&mut self.cache)
    }
}

thread_local! {
    static THREAD: UnsafeCell<Thread> = const {
        UnsafeCell::new(Thread {
            heap: 0,
            off_hold: false,
            closed: false,
            flushed_epoch: 0,
            cache: Cache::EMPTY,
        })
    };
}

fn thread() -> *mut Thread {
    THREAD.with(UnsafeCell::get)
}

pub fn off_hold_path() {
    unsafe { (*thread()).off_hold = true };
}

fn side() -> usize {
    if unsafe { (*thread()).off_hold } {
        OTHER
    } else {
        HOLD
    }
}

struct Depot {
    classes: [Stack; CLASSES],
    singles: [Stack; CLASSES],
    cold: [Stack; PAGE_CLASSES],
    cold_subpage: [Stack; PAGE_CLASS_FIRST],
}

impl Depot {
    const fn new() -> Self {
        Self {
            classes: [const { Stack::new() }; CLASSES],
            singles: [const { Stack::new() }; CLASSES],
            cold: [const { Stack::new() }; PAGE_CLASSES],
            cold_subpage: [const { Stack::new() }; PAGE_CLASS_FIRST],
        }
    }

    fn keep(&self, class: usize, batch_head: *mut u8) {
        unsafe { parked_next(batch_head).write(next(batch_head).read()) };
        self.classes[class].push(batch_head);
    }

    fn take(&self, class: usize) -> *mut u8 {
        let head = self.classes[class].pop();
        if !head.is_null() {
            unsafe { next(head).write(parked_next(head).read()) };
        }
        head
    }

    fn keep_single(&self, class: usize, block: *mut u8) {
        self.singles[class].push(block);
    }

    fn take_single(&self, class: usize) -> *mut u8 {
        self.singles[class].pop()
    }

    fn keep_cold(&self, class: usize, node: *mut u8) {
        self.cold[class - PAGE_CLASS_FIRST].push(node);
    }

    fn take_cold(&self, class: usize) -> *mut u8 {
        self.cold[class - PAGE_CLASS_FIRST].pop()
    }

    fn keep_cold_subpage(&self, class: usize, node: *mut u8) {
        self.cold_subpage[class].push(node);
    }

    fn take_cold_subpage(&self, class: usize) -> *mut u8 {
        self.cold_subpage[class].pop()
    }

    fn clear(&self) {
        self.classes.iter().for_each(Stack::clear);
        self.singles.iter().for_each(Stack::clear);
        self.cold.iter().for_each(Stack::clear);
        self.cold_subpage.iter().for_each(Stack::clear);
    }
}

#[repr(C)]
struct Record {
    link: [usize; 2],
    made: *mut Record,
    reserved: usize,
    committed: AtomicUsize,
}

struct Pages {
    free: [Stack; RESERVATIONS],
    cold: [Stack; RESERVATIONS],
    kept: AtomicUsize,
    made: AtomicPtr<Record>,
}

impl Pages {
    const fn new() -> Self {
        Self {
            free: [const { Stack::new() }; RESERVATIONS],
            cold: [const { Stack::new() }; RESERVATIONS],
            kept: AtomicUsize::new(0),
            made: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn made(&self, record: *mut Record) {
        let mut head = self.made.load(Ordering::Relaxed);
        loop {
            unsafe { (*record).made = head };
            match self.made.compare_exchange_weak(
                head,
                record,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(current) => head = current,
            }
        }
    }
}

struct Domain {
    handle: AtomicUsize,
    id: usize,
}

static DOMAINS: AtomicUsize = AtomicUsize::new(1);

enum Cold {
    Empty,
    Block(*mut u8),
    Failed,
}

pub struct PrivateHeap {
    domain_id: AtomicUsize,
    heap: OnceLock<Domain>,
    depot: Depot,
    pages: Pages,
    reclaiming: AtomicBool,
    flush_epoch: AtomicU64,
    sides: [Side; 2],
    #[cfg(test)]
    failures: AtomicUsize,
    #[cfg(test)]
    page_refusal: AtomicUsize,
}

impl PrivateHeap {
    pub const fn new() -> Self {
        Self {
            domain_id: AtomicUsize::new(0),
            heap: OnceLock::new(),
            depot: Depot::new(),
            pages: Pages::new(),
            reclaiming: AtomicBool::new(false),
            flush_epoch: AtomicU64::new(0),
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
            let handle = unsafe { HeapCreate(0, 0, 0) } as usize;
            if handle != 0 {
                self.domain_id.store(id, Ordering::Release);
            }
            Domain {
                handle: AtomicUsize::new(handle),
                id,
            }
        })
    }

    fn heap(&self) -> HANDLE {
        self.domain().handle.load(Ordering::Relaxed) as HANDLE
    }

    pub fn hold(&self) -> Waits {
        self.sides[HOLD].read()
    }

    pub fn other(&self) -> Waits {
        self.sides[OTHER].read()
    }

    fn heap_call<R>(&self, call: impl FnOnce() -> R) -> R {
        let begin = collector_primitives::clock::qpc();
        let result = call();
        self.sides[side()]
            .heap
            .add(collector_primitives::clock::qpc() - begin);
        result
    }

    fn page_call<R>(&self, call: impl FnOnce() -> R) -> R {
        let begin = collector_primitives::clock::qpc();
        let result = call();
        self.sides[side()]
            .pages
            .add(collector_primitives::clock::qpc() - begin);
        result
    }

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

    /// # Safety
    /// The [`GlobalAlloc::alloc`] contract: `size` and `align` come from a valid layout of non-zero
    /// size.
    unsafe fn allocate(&self, size: usize, align: usize, zeroed: bool) -> *mut u8 {
        if size <= SMALL_LIMIT && align <= HEAP_ALIGNMENT {
            let id = self.domain_id.load(Ordering::Relaxed);
            if id != 0 {
                #[cfg(test)]
                if self.refuse(tests::ALLOCATE) {
                    return ptr::null_mut();
                }
                if let Some(class) = class(size) {
                    if let Some(cache) = unsafe { (*thread()).cache(id) } {
                        let kept = cache.take(class, &self.depot);
                        if !kept.is_null() {
                            if zeroed {
                                unsafe { kept.write_bytes(0, size) };
                            }
                            return kept;
                        }
                        let single = self.depot.take_single(class);
                        if !single.is_null() {
                            if zeroed {
                                unsafe { single.write_bytes(0, size) };
                            }
                            return single;
                        }
                        let cold = if is_page_class(class) {
                            unsafe { self.take_cold(class) }
                        } else {
                            unsafe { self.take_cold_subpage(class, Some(cache)) }
                        };
                        match cold {
                            Cold::Block(block) => return block,
                            Cold::Failed => return ptr::null_mut(),
                            Cold::Empty => {}
                        }
                        return unsafe { self.refill_slab(class, Some(cache), zeroed) };
                    }
                    let reused = self.take_closed(class);
                    if !reused.is_null() {
                        if zeroed {
                            unsafe { reused.write_bytes(0, size) };
                        }
                        return reused;
                    }
                    let cold = if is_page_class(class) {
                        unsafe { self.take_cold(class) }
                    } else {
                        unsafe { self.take_cold_subpage(class, None) }
                    };
                    match cold {
                        Cold::Block(block) => return block,
                        Cold::Failed => return ptr::null_mut(),
                        Cold::Empty => {}
                    }
                    return unsafe { self.refill_slab(class, None, zeroed) };
                }
            }
        }
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
                let single = self.depot.take_single(class);
                if !single.is_null() {
                    if zeroed {
                        unsafe { single.write_bytes(0, bytes) };
                    }
                    return single;
                }
                let cold = if is_page_class(class) {
                    unsafe { self.take_cold(class) }
                } else {
                    unsafe { self.take_cold_subpage(class, Some(cache)) }
                };
                match cold {
                    Cold::Block(block) => return block,
                    Cold::Failed => return ptr::null_mut(),
                    Cold::Empty => {}
                }
                return unsafe { self.refill_slab(class, Some(cache), zeroed) };
            }
            let reused = self.take_closed(class);
            if !reused.is_null() {
                if zeroed {
                    unsafe { reused.write_bytes(0, bytes) };
                }
                return reused;
            }
            let cold = if is_page_class(class) {
                unsafe { self.take_cold(class) }
            } else {
                unsafe { self.take_cold_subpage(class, None) }
            };
            match cold {
                Cold::Block(block) => return block,
                Cold::Failed => return ptr::null_mut(),
                Cold::Empty => {}
            }
            return unsafe { self.refill_slab(class, None, zeroed) };
        }
        let flags = if zeroed { HEAP_ZERO_MEMORY } else { 0 };
        let raw = self
            .heap_call(|| unsafe { HeapAlloc(heap, flags, bytes) })
            .cast::<u8>();
        if raw.is_null() {
            return raw;
        }
        let start = unsafe { raw.add(HEADER) };
        let aligned = unsafe { start.add(start.align_offset(align)) };
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "HeapAlloc blocks are 16-byte aligned, so `aligned`, HEADER past one and rounded up to `align`, is pointer-aligned and so is the word below it"
        )]
        unsafe { aligned.cast::<*mut u8>().sub(1).write(raw) };
        aligned
    }

    fn take_closed(&self, class: usize) -> *mut u8 {
        let single = self.depot.take_single(class);
        if !single.is_null() {
            return single;
        }
        let head = self.depot.take(class);
        if !head.is_null() {
            let mut rest = unsafe { next(head).read() };
            while !rest.is_null() {
                let following = unsafe { next(rest).read() };
                self.depot.keep_single(class, rest);
                rest = following;
            }
        }
        head
    }

    /// # Safety
    /// `class` is a page-aligned small size class (`is_page_class(class)`).
    unsafe fn take_cold(&self, class: usize) -> Cold {
        let node = self.depot.take_cold(class);
        if node.is_null() {
            return Cold::Empty;
        }
        let block = cold_block(node, class);
        let size = class_size(class);
        if self.virtual_alloc(block, size, MEM_COMMIT).is_null() {
            self.depot.keep_cold(class, node);
            return Cold::Failed;
        }
        let record = ((block as usize) & !(GRANULARITY - 1)) as *mut Record;
        // SAFETY: `record` sits in Page 0 of this slab reservation, which stays committed.
        unsafe { (*record).committed.fetch_add(size, Ordering::Relaxed) };
        Cold::Block(block)
    }

    /// # Safety
    /// `class` is a sub-page small size class (`!is_page_class(class)`). If `cache` is `Some`, its
    /// `lists[class]` must currently be empty (`count == 0` and `spare.is_null()`).
    unsafe fn take_cold_subpage(&self, class: usize, cache: Option<&mut Cache>) -> Cold {
        let node = self.depot.take_cold_subpage(class);
        if node.is_null() {
            return Cold::Empty;
        }
        let page = cold_subpage_page(node);
        if self.virtual_alloc(page, PAGE, MEM_COMMIT).is_null() {
            self.depot.keep_cold_subpage(class, node);
            return Cold::Failed;
        }
        let record = ((page as usize) & !(GRANULARITY - 1)) as *mut Record;
        // SAFETY: `record` sits in Page 0 of this slab reservation, which stays committed.
        unsafe { (*record).committed.fetch_add(PAGE, Ordering::Relaxed) };
        let size = class_size(class);
        let count = subpage_per_page(size);
        // SAFETY: `page` is freshly committed and holds `count` disjoint blocks of `size` bytes.
        let first = unsafe {
            self.distribute_blocks(class, count, |index| page.add(index * size), cache)
        };
        Cold::Block(first)
    }

    /// # Safety
    /// `block_at(0..count)` must yield distinct committed blocks for `class`. If `cache` is
    /// `Some`, its `lists[class]` must currently be empty (`count == 0` and `spare.is_null()`).
    unsafe fn distribute_blocks(
        &self,
        class: usize,
        count: usize,
        block_at: impl Fn(usize) -> *mut u8,
        cache: Option<&mut Cache>,
    ) -> *mut u8 {
        let first = block_at(0);
        let batch_len = batch(class) as usize;
        let mut index = 1;
        if let Some(cache) = cache {
            let list = &mut cache.lists[class];
            let head_take = (count - index).min(batch_len - 1);
            if head_take > 0 {
                let mut head = ptr::null_mut();
                for offset in 0..head_take {
                    let block = block_at(index + offset);
                    unsafe { next(block).write(head) };
                    head = block;
                }
                list.head = head;
                list.count = head_take as u32;
                index += head_take;
            }
            if count - index >= batch_len {
                let mut spare = ptr::null_mut();
                for offset in 0..batch_len {
                    let block = block_at(index + offset);
                    unsafe { next(block).write(spare) };
                    spare = block;
                }
                list.spare = spare;
                index += batch_len;
            }
        }
        while count - index >= batch_len {
            let mut batch_head = ptr::null_mut();
            for offset in 0..batch_len {
                let block = block_at(index + offset);
                unsafe { next(block).write(batch_head) };
                batch_head = block;
            }
            self.depot.keep(class, batch_head);
            index += batch_len;
        }
        while index < count {
            let block = block_at(index);
            self.depot.keep_single(class, block);
            index += 1;
        }
        first
    }

    /// # Safety
    /// `class` is a valid small size class `< CLASSES`. If `cache` is `Some`, its `lists[class]`
    /// must currently be empty (`count == 0` and `spare.is_null()`).
    unsafe fn refill_slab(
        &self,
        class: usize,
        cache: Option<&mut Cache>,
        zeroed: bool,
    ) -> *mut u8 {
        let bytes = slab_bytes(class);
        let (reused, from_cold) = if bytes == GRANULARITY {
            #[expect(
                clippy::cast_ptr_alignment,
                reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
            )]
            let warm = self.pages.free[0].pop().cast::<Record>();
            if !warm.is_null() {
                (warm, false)
            } else {
                #[expect(
                    clippy::cast_ptr_alignment,
                    reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
                )]
                let cold = self.pages.cold[0].pop().cast::<Record>();
                (cold, !cold.is_null())
            }
        } else {
            (ptr::null_mut(), false)
        };
        let (base, fresh, committed) = if reused.is_null() {
            let base = self.virtual_alloc(ptr::null(), bytes, MEM_RESERVE | MEM_COMMIT);
            if base.is_null() {
                return ptr::null_mut();
            }
            (base, true, bytes)
        } else {
            let base = reused.cast::<u8>();
            let committed = unsafe { (*reused).committed.load(Ordering::Relaxed) };
            let reclaimable = committed.saturating_sub(PAGE);
            if !from_cold && reclaimable > 0 {
                self.pages.kept.fetch_sub(reclaimable, Ordering::Relaxed);
            }
            if bytes > committed
                && self
                    .virtual_alloc(base.wrapping_add(committed), bytes - committed, MEM_COMMIT)
                    .is_null()
            {
                if from_cold || reclaimable == 0 {
                    self.pages.cold[0].push(base);
                } else {
                    self.pages.kept.fetch_add(reclaimable, Ordering::Relaxed);
                    self.pages.free[0].push(base);
                }
                return ptr::null_mut();
            }
            (base, false, committed)
        };
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
        )]
        let record = base.cast::<Record>();
        unsafe {
            (*record).reserved = bytes;
            (*record).committed.store(bytes, Ordering::Relaxed);
        }
        if fresh {
            self.pages.made(record);
        }
        let count = slab_count(class);
        let first = unsafe {
            self.distribute_blocks(class, count, |index| slab_block(base, class, index), cache)
        };
        if zeroed && !fresh && (!is_page_class(class) || committed > PAGE) {
            // SAFETY: `first` sits in the committed slab and holds `class_size(class)` bytes.
            unsafe { first.write_bytes(0, class_size(class)) };
        }
        first
    }

    /// # Safety
    /// As [`Self::allocate`], with `size` above `SMALL_LIMIT`, so `dealloc` routes the block back here,
    /// and `reserved` the reservation size [`reservation`] gave for `size` and `align`.
    unsafe fn allocate_large(
        &self,
        reserved: usize,
        size: usize,
        align: usize,
        zeroed: bool,
    ) -> *mut u8 {
        let list_index = reservation_list(reserved);
        let warm = &self.pages.free[list_index];
        let cold = &self.pages.cold[list_index];
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
        )]
        let mut record = warm.pop().cast::<Record>();
        let mut from_cold = false;
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
        )]
        if record.is_null() {
            record = cold.pop().cast::<Record>();
            from_cold = !record.is_null();
        }
        let committed = if record.is_null() {
            0
        } else {
            let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
            let reclaimable = committed.saturating_sub(PAGE);
            if !from_cold && reclaimable > 0 {
                self.pages.kept.fetch_sub(reclaimable, Ordering::Relaxed);
            }
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
        let offset = (base as usize + RECORD_BYTES).next_multiple_of(align) - base as usize;
        let end = page_end(offset + size);
        if end > committed
            && self
                .virtual_alloc(base.wrapping_add(committed), end - committed, MEM_COMMIT)
                .is_null()
        {
            if fresh {
                self.page_call(|| unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) });
            } else if from_cold || committed <= PAGE {
                cold.push(base);
            } else {
                self.pages
                    .kept
                    .fetch_add(committed.saturating_sub(PAGE), Ordering::Relaxed);
                warm.push(base);
            }
            return ptr::null_mut();
        }
        if fresh {
            unsafe {
                (*record).reserved = reserved;
                (*record).committed.store(0, Ordering::Relaxed);
            }
            self.pages.made(record);
        }
        let mut final_committed = end.max(committed);
        if !fresh
            && side() == OTHER
            && committed > end.saturating_add(TRIM_SLACK)
            && self.page_call(|| unsafe {
                VirtualFree(base.add(end).cast(), committed - end, MEM_DECOMMIT)
            }) != 0
        {
            final_committed = end;
        }
        unsafe {
            (*record).committed.store(final_committed, Ordering::Relaxed);
        }
        let body = unsafe { base.add(offset) };
        if zeroed && committed > offset {
            unsafe { body.write_bytes(0, size.min(committed - offset)) };
        }
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "`body` is RECORD_BYTES past a 64 KiB-aligned reservation, rounded up to `align`, so the word below it is pointer-aligned"
        )]
        unsafe { body.cast::<*mut u8>().sub(1).write(base) };
        body
    }

    fn record(body: *mut u8) -> *mut Record {
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "large bodies are pointer-aligned as allocate_large places them, so the word below is too"
        )]
        unsafe { body.cast::<*mut Record>().sub(1).read() }
    }

    /// # Safety
    /// `body` is a live large block of this heap, never used again.
    unsafe fn free_large(&self, body: *mut u8) {
        let record = Self::record(body);
        let reserved = unsafe { (*record).reserved };
        let list_index = reservation_list(reserved);
        let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
        let reclaimable = committed.saturating_sub(PAGE);
        if reclaimable == 0 {
            self.pages.cold[list_index].push(record.cast());
            return;
        }
        let previous = self.pages.kept.fetch_add(reclaimable, Ordering::Relaxed);
        if side() == OTHER && previous + reclaimable > KEPT_BYTES {
            let base = record.cast::<u8>();
            if self.page_call(|| unsafe {
                VirtualFree(base.add(PAGE).cast(), reclaimable, MEM_DECOMMIT)
            }) != 0
            {
                unsafe { (*record).committed.store(PAGE, Ordering::Relaxed) };
                self.pages.kept.fetch_sub(reclaimable, Ordering::Relaxed);
                self.pages.cold[list_index].push(record.cast());
                return;
            }
        }
        self.pages.free[list_index].push(record.cast());
    }

    /// # Safety
    /// The [`GlobalAlloc::realloc`] contract, for a large block `body` of this heap, with `new_size`
    /// above `SMALL_LIMIT`: a block kept in place must still be freed as a large one.
    unsafe fn reallocate_large(&self, body: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let record = Self::record(body);
        let base = record.cast::<u8>();
        let (reserved, committed) =
            unsafe { ((*record).reserved, (*record).committed.load(Ordering::Relaxed)) };
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
            unsafe { (*record).committed.store(end, Ordering::Relaxed) };
        }
        body
    }

    pub fn ready(&self) -> bool {
        !self.heap().is_null()
    }

    pub fn flush_thread_cache(&self) {
        let id = self.domain_id.load(Ordering::Relaxed);
        if id == 0 {
            return;
        }
        // SAFETY: `THREAD` is thread-local and only accessed by the current thread.
        let thread = unsafe { &mut *thread() };
        if thread.closed || thread.heap != id {
            return;
        }
        thread.cache.flush(&self.depot);
        thread.flushed_epoch = self.flush_epoch.load(Ordering::Relaxed);
    }

    pub fn maybe_flush_thread_cache(&self) {
        let epoch = self.flush_epoch.load(Ordering::Relaxed);
        // SAFETY: `THREAD` is thread-local and only accessed by the current thread.
        let thread = unsafe { &mut *thread() };
        if thread.flushed_epoch != epoch {
            let id = self.domain_id.load(Ordering::Relaxed);
            if id != 0 && !thread.closed && thread.heap == id {
                thread.cache.flush(&self.depot);
            }
            thread.flushed_epoch = epoch;
        }
    }

    pub fn reclaim_empty_slabs(&self) -> usize {
        if side() != OTHER || self.heap().is_null() {
            return 0;
        }
        if self
            .reclaiming
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return 0;
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(&self.reclaiming);
        self.flush_epoch.fetch_add(1, Ordering::Relaxed);
        self.flush_thread_cache();
        let mut blocks = Vec::<usize>::new();
        let mut runs = Vec::<(usize, usize, usize)>::new();
        if blocks.try_reserve(4096).is_err() {
            return 0;
        }
        let mut reclaimed = 0usize;
        for class in (0..CLASSES).rev() {
            let bytes = slab_bytes(class);
            if bytes != GRANULARITY {
                continue;
            }
            blocks.clear();
            runs.clear();
            let mut oom = false;
            loop {
                let head = self.depot.take(class);
                if head.is_null() {
                    break;
                }
                let mut block = head;
                while !block.is_null() {
                    // SAFETY: blocks popped from the depot form a valid singly linked batch.
                    let following = unsafe { next(block).read() };
                    if !oom && blocks.try_reserve(1).is_ok() {
                        blocks.push(block as usize);
                    } else {
                        oom = true;
                        self.depot.keep_single(class, block);
                    }
                    block = following;
                }
                if oom {
                    break;
                }
            }
            if !oom {
                loop {
                    let block = self.depot.take_single(class);
                    if block.is_null() {
                        break;
                    }
                    if blocks.try_reserve(1).is_ok() {
                        blocks.push(block as usize);
                    } else {
                        self.depot.keep_single(class, block);
                        oom = true;
                        break;
                    }
                }
            }
            let page_class = is_page_class(class);
            if !oom {
                if page_class {
                    loop {
                        let node = self.depot.take_cold(class);
                        if node.is_null() {
                            break;
                        }
                        if blocks.try_reserve(1).is_ok() {
                            blocks.push((cold_block(node, class) as usize) | 1);
                        } else {
                            self.depot.keep_cold(class, node);
                            oom = true;
                            break;
                        }
                    }
                } else {
                    loop {
                        let node = self.depot.take_cold_subpage(class);
                        if node.is_null() {
                            break;
                        }
                        if blocks.try_reserve(1).is_ok() {
                            blocks.push((cold_subpage_page(node) as usize) | 1);
                        } else {
                            self.depot.keep_cold_subpage(class, node);
                            oom = true;
                            break;
                        }
                    }
                }
            }
            if blocks.is_empty() {
                continue;
            }
            blocks.sort_unstable_by_key(|&entry| entry & !1);
            let size = class_size(class);
            let total = slab_count(class);
            let per_page = if page_class { 1 } else { subpage_per_page(size) };
            let batch_len = batch(class) as usize;
            let mut kept_end = 0usize;
            let mut warm_pages_kept = 0usize;
            let mut at = 0usize;
            while at < blocks.len() {
                let slab = (blocks[at] & !1) & !(GRANULARITY - 1);
                let mut end = at + 1;
                while end < blocks.len() && ((blocks[end] & !1) & !(GRANULARITY - 1)) == slab {
                    end += 1;
                }
                let run_len = if page_class {
                    end - at
                } else {
                    blocks[at..end]
                        .iter()
                        .map(|&entry| if entry & 1 != 0 { per_page } else { 1 })
                        .sum()
                };
                let mut freed = false;
                if run_len == total && !oom {
                    let base = slab as *mut u8;
                    #[expect(
                        clippy::cast_ptr_alignment,
                        reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
                    )]
                    let record = base.cast::<Record>();
                    // SAFETY: every block in this 64 KiB slab has been exclusively popped out of
                    // the depot, so no live allocation or thread cache references this slab.
                    let reserved = unsafe { (*record).reserved };
                    if reserved == GRANULARITY
                        && self.page_call(|| unsafe {
                            VirtualFree(base.add(PAGE).cast(), GRANULARITY - PAGE, MEM_DECOMMIT)
                        }) != 0
                    {
                        // SAFETY: page 0 stays committed and we hold exclusive ownership of slab.
                        let previous = unsafe { (*record).committed.swap(PAGE, Ordering::Relaxed) };
                        self.pages.cold[0].push(base);
                        reclaimed += previous.saturating_sub(PAGE);
                        freed = true;
                    }
                }
                if !freed {
                    if page_class {
                        let record = slab as *mut Record;
                        let mut index = at;
                        while index < end {
                            let entry = blocks[index];
                            let block = (entry & !1) as *mut u8;
                            if entry & 1 != 0 {
                                self.depot.keep_cold(class, cold_node(block, class));
                                index += 1;
                            } else if kept_end < WARM_PAGE_BLOCKS {
                                blocks[kept_end] = block as usize;
                                kept_end += 1;
                                index += 1;
                            } else {
                                let run_start = entry & !1;
                                let mut run_end_addr = run_start + size;
                                let mut run_end = index + 1;
                                while run_end < end {
                                    let next_entry = blocks[run_end];
                                    if next_entry & 1 != 0 || next_entry != run_end_addr {
                                        break;
                                    }
                                    run_end_addr += size;
                                    run_end += 1;
                                }
                                let run_bytes = run_end_addr - run_start;
                                if self.page_call(|| unsafe {
                                    VirtualFree(run_start as *mut c_void, run_bytes, MEM_DECOMMIT)
                                }) != 0
                                {
                                    // SAFETY: `record` is Page 0 of the slab and stays committed.
                                    unsafe {
                                        (*record).committed.fetch_sub(run_bytes, Ordering::Relaxed);
                                    }
                                    for item in &blocks[index..run_end] {
                                        self.depot
                                            .keep_cold(class, cold_node(*item as *mut u8, class));
                                    }
                                    reclaimed += run_bytes;
                                } else {
                                    for pos in index..run_end {
                                        blocks[kept_end] = blocks[pos];
                                        kept_end += 1;
                                    }
                                }
                                index = run_end;
                            }
                        }
                    } else {
                        let record = slab as *mut Record;
                        let mut decommit_start = 0usize;
                        let mut decommit_end = 0usize;
                        let mut flush_decommit = |start: &mut usize, end_addr: &mut usize| {
                            if *start == *end_addr {
                                return;
                            }
                            let run_start = *start;
                            let run_bytes = *end_addr - run_start;
                            *start = 0;
                            *end_addr = 0;
                            if self.page_call(|| unsafe {
                                VirtualFree(run_start as *mut c_void, run_bytes, MEM_DECOMMIT)
                            }) != 0
                            {
                                // SAFETY: `record` is Page 0 of the slab and stays committed.
                                unsafe {
                                    (*record).committed.fetch_sub(run_bytes, Ordering::Relaxed);
                                }
                                let mut page = run_start;
                                while page < run_start + run_bytes {
                                    self.depot.keep_cold_subpage(
                                        class,
                                        cold_subpage_node(page as *mut u8),
                                    );
                                    page += PAGE;
                                }
                                reclaimed += run_bytes;
                            } else {
                                let mut page = run_start;
                                while page < run_start + run_bytes {
                                    for offset in 0..per_page {
                                        self.depot.keep_single(
                                            class,
                                            (page + offset * size) as *mut u8,
                                        );
                                    }
                                    page += PAGE;
                                }
                            }
                        };
                        let mut p_start = at;
                        while p_start < end {
                            let page_addr = (blocks[p_start] & !1) & !(PAGE - 1);
                            let mut p_end = p_start + 1;
                            while p_end < end
                                && ((blocks[p_end] & !1) & !(PAGE - 1)) == page_addr
                            {
                                p_end += 1;
                            }
                            if p_end - p_start == 1 && blocks[p_start] & 1 != 0 {
                                flush_decommit(&mut decommit_start, &mut decommit_end);
                                self.depot.keep_cold_subpage(
                                    class,
                                    cold_subpage_node(page_addr as *mut u8),
                                );
                            } else if page_addr > slab
                                && p_end - p_start == per_page
                                && warm_pages_kept >= WARM_SUBPAGE_PAGES
                            {
                                if decommit_start == decommit_end {
                                    decommit_start = page_addr;
                                    decommit_end = page_addr + PAGE;
                                } else if page_addr == decommit_end {
                                    decommit_end += PAGE;
                                } else {
                                    flush_decommit(&mut decommit_start, &mut decommit_end);
                                    decommit_start = page_addr;
                                    decommit_end = page_addr + PAGE;
                                }
                            } else {
                                flush_decommit(&mut decommit_start, &mut decommit_end);
                                if page_addr > slab && p_end - p_start == per_page {
                                    warm_pages_kept += 1;
                                }
                                if runs.try_reserve(1).is_ok() {
                                    runs.push((p_end - p_start, p_start, p_end));
                                } else {
                                    for index in p_start..p_end {
                                        self.depot
                                            .keep_single(class, blocks[index] as *mut u8);
                                    }
                                }
                            }
                            p_start = p_end;
                        }
                        flush_decommit(&mut decommit_start, &mut decommit_end);
                    }
                }
                at = end;
            }
            if page_class {
                let survivors = &blocks[..kept_end];
                let mut index = 0usize;
                while survivors.len() - index >= batch_len {
                    let mut batch_head = ptr::null_mut();
                    for offset in 0..batch_len {
                        let block = survivors[index + offset] as *mut u8;
                        // SAFETY: each survivor is an exclusively owned free block of `class`.
                        unsafe { next(block).write(batch_head) };
                        batch_head = block;
                    }
                    self.depot.keep(class, batch_head);
                    index += batch_len;
                }
                while index < survivors.len() {
                    self.depot.keep_single(class, survivors[index] as *mut u8);
                    index += 1;
                }
            } else {
                runs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
                let mut batch_head = ptr::null_mut();
                let mut batch_count = 0usize;
                for &(_, start, end) in &runs {
                    for &entry in &blocks[start..end] {
                        let block = entry as *mut u8;
                        // SAFETY: each survivor is an exclusively owned free block of `class`.
                        unsafe { next(block).write(batch_head) };
                        batch_head = block;
                        batch_count += 1;
                        if batch_count == batch_len {
                            self.depot.keep(class, batch_head);
                            batch_head = ptr::null_mut();
                            batch_count = 0;
                        }
                    }
                }
                while !batch_head.is_null() {
                    // SAFETY: `batch_head` is a singly linked list of exclusively owned free blocks.
                    let following = unsafe { next(batch_head).read() };
                    self.depot.keep_single(class, batch_head);
                    batch_head = following;
                }
            }
        }
        reclaimed += self.trim_large_free(&mut blocks);
        reclaimed
    }

    fn trim_large_free(&self, held: &mut Vec<usize>) -> usize {
        if self.pages.kept.load(Ordering::Relaxed) <= KEPT_BYTES {
            return 0;
        }
        let mut reclaimed = 0usize;
        for list in (0..RESERVATIONS).rev() {
            if self.pages.kept.load(Ordering::Relaxed) <= KEPT_BYTES {
                break;
            }
            held.clear();
            let limit = if list == 0 { 1024 } else { usize::MAX };
            for _ in 0..limit {
                let base = self.pages.free[list].pop();
                if base.is_null() {
                    break;
                }
                #[expect(
                    clippy::cast_ptr_alignment,
                    reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
                )]
                let record = base.cast::<Record>();
                // SAFETY: `base` was exclusively popped from `pages.free[list]`.
                let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
                if committed <= PAGE {
                    self.pages.cold[list].push(base);
                    continue;
                }
                if self.pages.kept.load(Ordering::Relaxed) > KEPT_BYTES
                    && self.page_call(|| unsafe {
                        VirtualFree(base.add(PAGE).cast(), committed - PAGE, MEM_DECOMMIT)
                    }) != 0
                {
                    // SAFETY: page 0 stays committed and we exclusively own `base`.
                    unsafe { (*record).committed.store(PAGE, Ordering::Relaxed) };
                    let freed = committed - PAGE;
                    self.pages.kept.fetch_sub(freed, Ordering::Relaxed);
                    reclaimed += freed;
                    self.pages.cold[list].push(base);
                    continue;
                }
                if held.try_reserve(1).is_ok() {
                    held.push(base as usize);
                } else {
                    self.pages.free[list].push(base);
                    break;
                }
            }
            for &base in held.iter() {
                self.pages.free[list].push(base as *mut u8);
            }
        }
        reclaimed
    }

    pub fn trim_large_if_needed(&self) -> usize {
        if side() != OTHER
            || self.heap().is_null()
            || self.pages.kept.load(Ordering::Relaxed) <= KEPT_BYTES
        {
            return 0;
        }
        if self
            .reclaiming
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return 0;
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(&self.reclaiming);
        let mut reclaimed = 0usize;
        let mut held = [0usize; 64];
        for list in (0..RESERVATIONS).rev() {
            if self.pages.kept.load(Ordering::Relaxed) <= KEPT_BYTES {
                break;
            }
            let mut count = 0usize;
            while count < held.len() {
                let base = self.pages.free[list].pop();
                if base.is_null() {
                    break;
                }
                #[expect(
                    clippy::cast_ptr_alignment,
                    reason = "reservations and slabs start on 64 KiB allocation-granularity boundaries"
                )]
                let record = base.cast::<Record>();
                // SAFETY: `base` was exclusively popped from `pages.free[list]`.
                let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
                if committed <= PAGE {
                    self.pages.cold[list].push(base);
                    continue;
                }
                if self.pages.kept.load(Ordering::Relaxed) > KEPT_BYTES
                    && self.page_call(|| unsafe {
                        VirtualFree(base.add(PAGE).cast(), committed - PAGE, MEM_DECOMMIT)
                    }) != 0
                {
                    // SAFETY: page 0 stays committed and we exclusively own `base`.
                    unsafe { (*record).committed.store(PAGE, Ordering::Relaxed) };
                    let freed = committed - PAGE;
                    self.pages.kept.fetch_sub(freed, Ordering::Relaxed);
                    reclaimed += freed;
                    self.pages.cold[list].push(base);
                    continue;
                }
                held[count] = base as usize;
                count += 1;
            }
            for &base in &held[..count] {
                self.pages.free[list].push(base as *mut u8);
            }
        }
        reclaimed
    }

    /// # Safety
    /// Only the collector DLL's thread-detach path may flush this thread's heap state, after its
    /// collector work has ended and while the heap is still live.
    pub unsafe fn thread_exit(&self) {
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
        thread.cache.flush(&self.depot);
    }

    /// # Safety
    /// On FreeLibrary detach no caller may still own an allocation it will use,
    /// free or reallocate. Runtime must first restore/quiesce hooks and join all
    /// workers; the loader has run the unloading thread's TLS destructors before
    /// DllMain, and removes this module's callbacks before other threads exit.
    /// No Rust static has an automatic destructor. Do not allocate/log here.
    pub unsafe fn detach(&self, reason: u32, reserved: *mut c_void) -> bool {
        if reason != DLL_PROCESS_DETACH || !reserved.is_null() {
            return false;
        }
        let Some(domain) = self.heap.get() else {
            return false;
        };
        self.domain_id.store(0, Ordering::Release);
        let heap = domain.handle.swap(0, Ordering::Relaxed) as HANDLE;
        self.depot.clear();
        self.pages.free.iter().for_each(Stack::clear);
        self.pages.cold.iter().for_each(Stack::clear);
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
        if layout.align() <= HEAP_ALIGNMENT
            && let Some(class) = class(layout.size())
        {
            let id = self.domain_id.load(Ordering::Relaxed);
            let id = if id != 0 { id } else { self.domain().id };
            let thread = unsafe { &mut *thread() };
            if let Some(cache) = thread.cache(id) {
                let spilled = cache.keep(class, pointer, &self.depot);
                if spilled && thread.off_hold {
                    self.maybe_flush_thread_cache();
                }
            } else {
                self.depot.keep_single(class, pointer);
            }
            return;
        }
        let domain = self.domain();
        let heap = domain.handle.load(Ordering::Relaxed) as HANDLE;
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "blocks from the HeapAlloc path are pointer-aligned as allocate places them, so the word below is too"
        )]
        let raw = unsafe { pointer.cast::<*mut u8>().sub(1).read() };
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
        unsafe { self.moved(pointer, layout, new_size) }
    }
}

impl PrivateHeap {
    /// # Safety
    /// The [`GlobalAlloc::realloc`] contract. On success `pointer` is freed.
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

pub static PRIVATE_HEAP: PrivateHeap = PrivateHeap::new();
