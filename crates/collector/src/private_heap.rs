//! One collector-owned, growable serialized heap; no process-heap fallback.
use std::{
    alloc::{GlobalAlloc, Layout},
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Memory::{
        HEAP_REALLOC_IN_PLACE_ONLY, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy, HeapFree,
        HeapReAlloc,
    },
};

// Collector requires Windows x64; ordinary heap blocks are 16-byte aligned.
const HEAP_ALIGNMENT: usize = 16;
/// Growth to this size copies outside the heap lock, which a moving HeapReAlloc holds.
const UNLOCKED_GROWTH: usize = 64 * 1024;
const HEADER: usize = size_of::<*mut u8>();
const DLL_PROCESS_DETACH: u32 = 0;

pub(super) struct PrivateHeap {
    // OnceLock initialization allocates no Rust memory. A failed creation is
    // cached too; zero after detach is terminal, never a new allocation domain.
    heap: OnceLock<AtomicUsize>,
    #[cfg(test)]
    failures: AtomicUsize,
}

impl PrivateHeap {
    pub(super) const fn new() -> Self {
        Self {
            heap: OnceLock::new(),
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
        let raw = unsafe { HeapAlloc(heap, if zeroed { HEAP_ZERO_MEMORY } else { 0 }, bytes) }
            .cast::<u8>();
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
            let replacement = unsafe { self.allocate(new_size, layout.align(), false) };
            if !replacement.is_null() {
                unsafe {
                    ptr::copy_nonoverlapping(pointer, replacement, layout.size());
                    HeapFree(self.heap(), 0, pointer.cast());
                }
            }
            return replacement;
        }
        // Moving a raw over-aligned block changes its padding, and shrinking
        // could discard its payload offset. Copy/free only after success, with
        // no second capture allocation_attempt; failure preserves the old block.
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
