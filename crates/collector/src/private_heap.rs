//! The collector's allocator: one private heap and no process-heap fallback.
//! Each hour reallocates much the same small blocks, so they recycle through
//! per-processor size-class lists: the hot path takes an uncontended lock
//! instead of the heap's shared one. Detach destroys the heap and the lists.
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{
        Memory::{
            HEAP_REALLOC_IN_PLACE_ONLY, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy,
            HeapFree, HeapReAlloc,
        },
        Threading::{
            AcquireSRWLockExclusive, GetCurrentProcessorNumber, ReleaseSRWLockExclusive, SRWLOCK,
            SRWLOCK_INIT,
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
/// Free bytes one shard keeps per class; beyond it blocks go back to the heap.
const SHARD_CLASS_BYTES: usize = 64 * 1024;

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

/// One class's free blocks, linked through each block's first word.
#[derive(Clone, Copy)]
struct List {
    head: *mut u8,
    count: u32,
    hits: u64,
    misses: u64,
    kept: u64,
    returned: u64,
}

impl List {
    const EMPTY: Self = Self {
        head: ptr::null_mut(),
        count: 0,
        hits: 0,
        misses: 0,
        kept: 0,
        returned: 0,
    };
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
        unsafe { AcquireSRWLockExclusive(self.lock.get()) };
        let result = f(unsafe { &mut *self.lists.get() });
        unsafe { ReleaseSRWLockExclusive(self.lock.get()) };
        result
    }

    /// A free block of `class`, or null when none is kept.
    fn take(&self, class: usize) -> *mut u8 {
        self.with(|lists| {
            let list = &mut lists[class];
            let head = list.head;
            if head.is_null() {
                list.misses += 1;
            } else {
                list.head = unsafe { head.cast::<*mut u8>().read() };
                list.count -= 1;
                list.hits += 1;
            }
            head
        })
    }

    /// Keep a freed block of `class`; false when the class is full.
    fn keep(&self, class: usize, block: *mut u8) -> bool {
        self.with(|lists| {
            let list = &mut lists[class];
            if (list.count as usize + 1) * class_size(class) > SHARD_CLASS_BYTES {
                list.returned += 1;
                return false;
            }
            unsafe { block.cast::<*mut u8>().write(list.head) };
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
    #[cfg(test)]
    failures: AtomicUsize,
}

impl PrivateHeap {
    pub(super) const fn new() -> Self {
        Self {
            heap: OnceLock::new(),
            shards: [const { Shard::new() }; SHARDS],
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
            let kept = self.shard().take(class);
            if kept.is_null() {
                return unsafe { HeapAlloc(heap, flags, class_size(class)) }.cast();
            }
            if zeroed {
                unsafe { kept.write_bytes(0, bytes) };
            }
            return kept;
        }
        let raw = unsafe { HeapAlloc(heap, flags, bytes) }.cast::<u8>();
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
                && self.shard().keep(class, pointer)
            {
                return;
            }
            pointer
        } else {
            unsafe { pointer.cast::<*mut u8>().sub(1).read() }
        };
        unsafe { HeapFree(self.heap(), 0, raw.cast()) };
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
                return unsafe { HeapReAlloc(self.heap(), 0, pointer.cast(), new_size) }.cast();
            }
            let grown = unsafe {
                HeapReAlloc(
                    self.heap(),
                    HEAP_REALLOC_IN_PLACE_ONLY,
                    pointer.cast(),
                    new_size,
                )
            };
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
        let mut totals = [List::EMPTY; CLASSES];
        for shard in &self.shards {
            shard.with(|lists| {
                for (total, list) in totals.iter_mut().zip(lists.iter()) {
                    total.count += list.count;
                    total.hits += list.hits;
                    total.misses += list.misses;
                    total.kept += list.kept;
                    total.returned += list.returned;
                }
            });
        }
        Snapshot(totals)
    }
}

/// Per class: allocations served from a list or by the heap, frees kept on a
/// list or returned to the heap, and blocks on the lists now.
pub(super) struct Snapshot([List; CLASSES]);

impl serde::Serialize for Snapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let classes = (self.0.iter().enumerate())
            .filter(|(_, list)| list.hits + list.misses + list.kept + list.returned != 0)
            .map(|(class, list)| {
                crate::status::status!({
                    "bytes": class_size(class),
                    "hits": list.hits,
                    "misses": list.misses,
                    "kept": list.kept,
                    "returned": list.returned,
                    "cached": list.count,
                })
            });
        crate::status::status!({
            "classes": crate::status::sequence(classes),
            "scope": "cumulative since attach, per small-block class summed over processor shards: allocations served from a shard's free list (hits) or by the heap (misses), frees kept on a list (kept) or returned to the heap because the class held its bound (returned), and blocks on the lists now (cached)",
        })
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
