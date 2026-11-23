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

const HEAP_ALIGNMENT: usize = 16;
const HEADER: usize = size_of::<*mut u8>();
const DLL_PROCESS_DETACH: u32 = 0;
const FINE_CLASSES: usize = 64;
const SMALL_LIMIT: usize = 64 * 1024;
const CLASSES: usize = FINE_CLASSES + 6;
const CACHE_CLASS_BYTES: usize = 128 * 1024;
const MAX_BATCH: usize = 256;
const _: () = assert!(SMALL_LIMIT <= CACHE_CLASS_BYTES / 2);
const PAGE: usize = 4096;
const GRANULARITY: usize = 64 * 1024;
const RECORD_BYTES: usize = 64;
const _: () = assert!(size_of::<Record>() + HEADER <= RECORD_BYTES);
const RESERVATIONS: usize = (usize::BITS - GRANULARITY.trailing_zeros()) as usize;
const KEPT_BYTES: usize = 1024 * 1024 * 1024;

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

    fn keep(&mut self, class: usize, block: *mut u8, depot: &Depot) {
        let list = &mut self.lists[class];
        if list.count == batch(class) {
            if !list.spare.is_null() {
                depot.keep(class, list.spare);
            }
            list.spare = list.head;
            list.head = ptr::null_mut();
            list.count = 0;
        }
        unsafe { next(block).write(list.head) };
        list.head = block;
        list.count += 1;
    }
}

struct Thread {
    heap: usize,
    off_hold: bool,
    closed: bool,
    cache: Cache,
}

impl Thread {
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
}

impl Depot {
    const fn new() -> Self {
        Self {
            classes: [const { Stack::new() }; CLASSES],
            singles: [const { Stack::new() }; CLASSES],
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

    fn clear(&self) {
        self.classes.iter().for_each(Stack::clear);
        self.singles.iter().for_each(Stack::clear);
    }
}

#[repr(C)]
struct Record {
    link: [usize; 2],
    made: *mut Record,
    reserved: usize,
    committed: usize,
}

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

pub struct PrivateHeap {
    domain_id: AtomicUsize,
    heap: OnceLock<Domain>,
    depot: Depot,
    pages: Pages,
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
                        return unsafe { self.refill_slab(class, Some(cache)) };
                    }
                    let reused = self.take_closed(class);
                    if !reused.is_null() {
                        if zeroed {
                            unsafe { reused.write_bytes(0, size) };
                        }
                        return reused;
                    }
                    return unsafe { self.refill_slab(class, None) };
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
                return unsafe { self.refill_slab(class, Some(cache)) };
            }
            let reused = self.take_closed(class);
            if !reused.is_null() {
                if zeroed {
                    unsafe { reused.write_bytes(0, bytes) };
                }
                return reused;
            }
            return unsafe { self.refill_slab(class, None) };
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
    /// `class` is a valid small size class `< CLASSES`. If `cache` is `Some`, its `lists[class]`
    /// must currently be empty (`count == 0` and `spare.is_null()`).
    unsafe fn refill_slab(&self, class: usize, cache: Option<&mut Cache>) -> *mut u8 {
        let bytes = slab_bytes(class);
        let base = self.virtual_alloc(ptr::null(), bytes, MEM_RESERVE | MEM_COMMIT);
        if base.is_null() {
            return ptr::null_mut();
        }
        let record = base.cast::<Record>();
        unsafe {
            (*record).reserved = bytes;
            (*record).committed = bytes;
        }
        self.pages.made(record);
        let size = class_size(class);
        let count = (bytes - RECORD_BYTES) / size;
        let first = unsafe { base.add(RECORD_BYTES) };
        let batch_len = batch(class) as usize;
        let mut index = 1;
        if let Some(cache) = cache {
            let list = &mut cache.lists[class];
            let head_take = (count - index).min(batch_len - 1);
            if head_take > 0 {
                let mut head = ptr::null_mut();
                for offset in 0..head_take {
                    let block = unsafe { base.add(RECORD_BYTES + (index + offset) * size) };
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
                    let block = unsafe { base.add(RECORD_BYTES + (index + offset) * size) };
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
                let block = unsafe { base.add(RECORD_BYTES + (index + offset) * size) };
                unsafe { next(block).write(batch_head) };
                batch_head = block;
            }
            self.depot.keep(class, batch_head);
            index += batch_len;
        }
        while index < count {
            let block = unsafe { base.add(RECORD_BYTES + index * size) };
            self.depot.keep_single(class, block);
            index += 1;
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
            unsafe { body.write_bytes(0, size.min(committed - offset)) };
        }
        unsafe { body.cast::<*mut u8>().sub(1).write(base) };
        body
    }

    fn record(body: *mut u8) -> *mut Record {
        unsafe { body.cast::<*mut Record>().sub(1).read() }
    }

    /// # Safety
    /// `body` is a live large block of this heap, never used again.
    unsafe fn free_large(&self, body: *mut u8) {
        let record = Self::record(body);
        let committed = unsafe { (*record).committed };
        let previous = self.pages.kept.fetch_add(committed, Ordering::Relaxed);
        if side() == OTHER && previous + committed > KEPT_BYTES {
            let base = record.cast::<u8>();
            if committed > PAGE
                && self.page_call(|| unsafe {
                    VirtualFree(base.add(PAGE).cast(), committed - PAGE, MEM_DECOMMIT)
                }) != 0
            {
                unsafe { (*record).committed = PAGE };
                self.pages
                    .kept
                    .fetch_sub(committed - PAGE, Ordering::Relaxed);
            }
        }
        let reserved = unsafe { (*record).reserved };
        self.pages.free[reservation_list(reserved)].push(record.cast());
    }

    /// # Safety
    /// The [`GlobalAlloc::realloc`] contract, for a large block `body` of this heap, with `new_size`
    /// above `SMALL_LIMIT`: a block kept in place must still be freed as a large one.
    unsafe fn reallocate_large(&self, body: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
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
        }
        body
    }

    pub fn ready(&self) -> bool {
        !self.heap().is_null()
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
        let cache = std::mem::replace(&mut thread.cache, Cache::EMPTY);
        for (class, list) in cache.lists.into_iter().enumerate() {
            if !list.spare.is_null() {
                self.depot.keep(class, list.spare);
            }
            if list.head.is_null() {
                continue;
            }
            if list.count == batch(class) {
                self.depot.keep(class, list.head);
                continue;
            }
            let mut block = list.head;
            while !block.is_null() {
                let following = unsafe { next(block).read() };
                self.depot.keep_single(class, block);
                block = following;
            }
        }
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
                cache.keep(class, pointer, &self.depot);
            } else {
                self.depot.keep_single(class, pointer);
            }
            return;
        }
        let domain = self.domain();
        let heap = domain.handle.load(Ordering::Relaxed) as HANDLE;
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
