//! thread may hold the process heap's lock with nobody left to release it.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicIsize, Ordering};

type Handle = isize;

#[link(name = "kernel32")]
extern "system" {
    fn HeapCreate(options: u32, initial_size: usize, maximum_size: usize) -> Handle;
    fn HeapDestroy(heap: Handle) -> i32;
    fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut u8;
    fn HeapReAlloc(heap: Handle, flags: u32, block: *mut u8, bytes: usize) -> *mut u8;
    fn HeapFree(heap: Handle, flags: u32, block: *mut u8) -> i32;
}

const HEAP_ZERO_MEMORY: u32 = 0x0000_0008;

const MIN_ALIGN: usize = 16;

type Header = *mut u8;

pub struct PrivateHeap;

static HEAP: AtomicIsize = AtomicIsize::new(0);

#[inline]
fn heap() -> Handle {
    let existing = HEAP.load(Ordering::Acquire);
    if existing != 0 {
        return existing;
    }
    // SAFETY: plain Win32 call with documented arguments (growable, default
    let created = unsafe { HeapCreate(0, 0, 0) };
    if created == 0 {
        return 0;
    }
    match HEAP.compare_exchange(0, created, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => created,
        Err(winner) => {
            // SAFETY: `created` was created by this call, nothing has been
            unsafe {
                HeapDestroy(created);
            }
            winner
        }
    }
}

#[inline]
fn direct(layout: &Layout) -> bool {
    layout.align() <= MIN_ALIGN && layout.align() <= layout.size()
}

#[inline]
unsafe fn allocate_over_aligned(heap: Handle, layout: Layout, flags: u32) -> *mut u8 {
    let Some(size) = layout.size().checked_add(layout.align()) else {
        return core::ptr::null_mut();
    };
    // SAFETY: valid heap handle, documented flags.
    let block = unsafe { HeapAlloc(heap, flags, size) };
    if block.is_null() {
        return block;
    }
    let header_size = core::mem::size_of::<Header>();
    let start = block as usize + header_size;
    let aligned = (start + layout.align() - 1) & !(layout.align() - 1);
    // SAFETY: `aligned - header_size >= block` and `aligned + layout.size()
    unsafe {
        let header = (aligned - header_size) as *mut Header;
        header.write_unaligned(block);
    }
    aligned as *mut u8
}

#[inline]
unsafe fn block_of(ptr: *mut u8, layout: &Layout) -> *mut u8 {
    if direct(layout) {
        ptr
    } else {
        // SAFETY: `ptr` came from `allocate_over_aligned`, which wrote the
        unsafe { (ptr as *mut Header).sub(1).read_unaligned() }
    }
}

// SAFETY: `GlobalAlloc`'s contract — a valid pointer or null from `alloc`,
unsafe impl GlobalAlloc for PrivateHeap {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap = heap();
        if heap == 0 {
            return core::ptr::null_mut();
        }
        if direct(&layout) {
            // SAFETY: valid heap handle.
            unsafe { HeapAlloc(heap, 0, layout.size()) }
        } else {
            // SAFETY: as documented on the helper.
            unsafe { allocate_over_aligned(heap, layout, 0) }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let heap = heap();
        if heap == 0 {
            return core::ptr::null_mut();
        }
        if direct(&layout) {
            // SAFETY: valid heap handle; the flag zero-fills the block.
            unsafe { HeapAlloc(heap, HEAP_ZERO_MEMORY, layout.size()) }
        } else {
            // SAFETY: as documented on the helper; the whole block (padding
            unsafe { allocate_over_aligned(heap, layout, HEAP_ZERO_MEMORY) }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let heap = HEAP.load(Ordering::Acquire);
        // SAFETY: `ptr` was returned by `alloc`/`realloc` of this allocator
        unsafe {
            let block = block_of(ptr, &layout);
            HeapFree(heap, 0, block);
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_is_direct = layout.align() <= MIN_ALIGN && layout.align() <= new_size;
        if direct(&layout) && new_is_direct {
            let heap = HEAP.load(Ordering::Acquire);
            // SAFETY: `ptr` is a direct block of this heap; `HeapReAlloc`
            unsafe { HeapReAlloc(heap, 0, ptr, new_size) }
        } else {
            // SAFETY: the trait's default `realloc` (alloc, copy, dealloc) is
            let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
            let new_ptr = unsafe { self.alloc(new_layout) };
            if !new_ptr.is_null() {
                let copy = core::cmp::min(layout.size(), new_size);
                // SAFETY: both blocks are valid for `copy` bytes and do not
                unsafe {
                    core::ptr::copy_nonoverlapping(ptr, new_ptr, copy);
                    self.dealloc(ptr, layout);
                }
            }
            new_ptr
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_heap_serves_every_layout_shape_and_round_trips() {
        let heap = PrivateHeap;
        for &(size, align) in &[
            (1usize, 1usize),
            (7, 1),
            (16, 16),
            (17, 16),
            (8, 16),
            (100, 32),
            (4096, 4096),
            (10, 64),
            (65_536, 8),
            (3, 2),
        ] {
            let layout = Layout::from_size_align(size, align).unwrap();
            // SAFETY: valid layouts; every pointer is written within its
            unsafe {
                let ptr = heap.alloc(layout);
                assert!(!ptr.is_null(), "alloc {size}/{align}");
                assert_eq!(ptr as usize % align, 0, "alignment {size}/{align}");
                for i in 0..size {
                    ptr.add(i).write((i as u8).wrapping_mul(31));
                }
                for i in 0..size {
                    assert_eq!(ptr.add(i).read(), (i as u8).wrapping_mul(31));
                }
                let grown = heap.realloc(ptr, layout, size * 3 + 5);
                assert!(!grown.is_null());
                assert_eq!(
                    grown as usize % align,
                    0,
                    "realloc alignment {size}/{align}"
                );
                for i in 0..size {
                    assert_eq!(grown.add(i).read(), (i as u8).wrapping_mul(31));
                }
                let grown_layout = Layout::from_size_align(size * 3 + 5, align).unwrap();
                let shrunk = heap.realloc(grown, grown_layout, size.max(1));
                assert!(!shrunk.is_null());
                assert_eq!(shrunk.read(), 0);
                heap.dealloc(shrunk, Layout::from_size_align(size.max(1), align).unwrap());

                let zeroed = heap.alloc_zeroed(layout);
                assert!(!zeroed.is_null());
                assert!(
                    (0..size).all(|i| zeroed.add(i).read() == 0),
                    "zeroed {size}/{align}"
                );
                heap.dealloc(zeroed, layout);
            }
        }
        let first = HEAP.load(Ordering::Acquire);
        assert_ne!(first, 0);
        assert_eq!(super::heap(), first);
    }

    #[test]
    fn direct_rule_matches_std() {
        assert!(direct(&Layout::from_size_align(16, 16).unwrap()));
        assert!(direct(&Layout::from_size_align(1, 1).unwrap()));
        assert!(!direct(&Layout::from_size_align(8, 16).unwrap()));
        assert!(!direct(&Layout::from_size_align(64, 32).unwrap()));
    }
}
