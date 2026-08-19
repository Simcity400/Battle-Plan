//! The collector's OWN Win32 heap (lane LO, 2026-08-16).
//!
//! # Why
//!
//! Rust's default `System` allocator on Windows is `HeapAlloc` on the PROCESS
//! DEFAULT HEAP — the heap the game's own threads allocate from through
//! ordinary Win32/CRT calls. Every collector allocation therefore contends
//! with the engine's on the same heap locks (the main heap lock for anything
//! the low-fragmentation heap does not serve, the LFH bucket locks otherwise),
//! which is one way an engine thread can be made to wait on the collector. It is
//! also the hazard behind `reserve_confirmed_span_cache`: a read
//! taken inside a held freeze must not allocate because a SUSPENDED game
//! thread may hold the process heap's lock with nobody left to release it.
//! And it is the reason the below-normal worker-priority policy cannot ship:
//! a low-priority collector thread preempted while holding a lock the engine
//! needs is a priority inversion the engine pays for.
//!
//! A private heap removes all three at once: the engine never touches it, so
//! no engine thread can wait on a collector allocation, no suspended engine
//! thread can hold its lock, and a lowered collector thread holding it can
//! only delay other collector threads.
//!
//! # What it is
//!
//! `HeapCreate(0, 0, 0)`: growable, serialised (its own lock, LFH enabled by
//! the OS as for any growable heap), created on the first allocation. The
//! layout rules are the standard library's own Windows `System` allocator
//! rules, reproduced: requests aligned to at most [`MIN_ALIGN`] whose size is
//! at least the alignment are served straight from `HeapAlloc` (which
//! guarantees 16-byte alignment on x64); anything over-aligned is served from
//! an over-allocation with the block's real start stored in the eight bytes
//! before the returned pointer, exactly as `std` does, so `dealloc`/`realloc`
//! can find the block again.
//!
//! # What it is not
//!
//! Not `mimalloc` (recorder_host's choice, a separate process with a fold
//! workload) — the collector needs isolation from the game's heap first and a
//! faster allocator second, and a Win32 heap adds no code the game does not
//! already run. Not destroyed at DLL unload: the statics the collector leaks
//! today (`OnceLock`s, thread-locals) would dangle into a destroyed heap, and
//! a leaked heap object is small; the game process ends with the campaign.
//!
//! # Where it applies
//!
//! Installed as the crate's `#[global_allocator]` for the shipped cdylib only:
//! `cargo test` for this crate (`cfg(test)`) and consumers that link the rlib
//! with `test-support` (recorder_host's dev-dependency, which installs
//! mimalloc itself) get the type but not the installation, so no binary ends
//! up with two global allocators.

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

/// The alignment `HeapAlloc` guarantees on x64 (and the standard library's
/// own `MIN_ALIGN` for this target).
const MIN_ALIGN: usize = 16;

/// The over-aligned block header: the block's real start, stored immediately
/// before the aligned pointer handed out.
type Header = *mut u8;

/// The collector's private heap allocator. Zero-sized; the heap handle lives
/// in a process-global atomic so the type can be a `static`.
pub struct PrivateHeap;

static HEAP: AtomicIsize = AtomicIsize::new(0);

/// The heap, created on first use. Two threads racing here both create a
/// heap; the loser destroys its own and adopts the winner's, so exactly one
/// heap ever serves allocations. Returns 0 (null) only if the OS refused to
/// create a heap at all, and every allocation then fails cleanly with null.
#[inline]
fn heap() -> Handle {
    let existing = HEAP.load(Ordering::Acquire);
    if existing != 0 {
        return existing;
    }
    // SAFETY: plain Win32 call with documented arguments (growable, default
    // serialisation, no initial commit).
    let created = unsafe { HeapCreate(0, 0, 0) };
    if created == 0 {
        return 0;
    }
    match HEAP.compare_exchange(0, created, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => created,
        Err(winner) => {
            // SAFETY: `created` was created by this call, nothing has been
            // allocated from it, and it is not published anywhere.
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

/// The standard library's over-aligned path: allocate `size + align`, hand
/// out the first `align`-aligned address that leaves room for a `Header`
/// before it, and store the real block start there.
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
    // The first `align`-aligned address at or after `block + header_size`.
    // `align` is a power of two so the mask arithmetic is exact, and
    // `size >= size + align` cannot overflow into a smaller value.
    let start = block as usize + header_size;
    let aligned = (start + layout.align() - 1) & !(layout.align() - 1);
    // SAFETY: `aligned - header_size >= block` and `aligned + layout.size()
    // <= block + size` by construction, so both the header write and the
    // caller's use stay inside the block.
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
        // block start immediately before it.
        unsafe { (ptr as *mut Header).sub(1).read_unaligned() }
    }
}

// SAFETY: `GlobalAlloc`'s contract — a valid pointer or null from `alloc`,
// `dealloc` only with pointers this allocator returned and the same layout,
// `realloc` preserving contents up to the smaller size — is met exactly as
// the standard library's Windows `System` allocator meets it: same direct /
// over-aligned split, same header, same `HeapReAlloc` use on the direct path
// and the trait's default copy-based `realloc` on the over-aligned path.
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
            // included) is zero-filled by the flag.
            unsafe { allocate_over_aligned(heap, layout, HEAP_ZERO_MEMORY) }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let heap = HEAP.load(Ordering::Acquire);
        // SAFETY: `ptr` was returned by `alloc`/`realloc` of this allocator
        // (the trait's contract), so the heap exists and `block_of` recovers
        // the block `HeapAlloc` returned.
        unsafe {
            let block = block_of(ptr, &layout);
            HeapFree(heap, 0, block);
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Both the OLD and the NEW layout must be direct for `HeapReAlloc`:
        // the caller will `dealloc` with `(new_size, align)`, and a block that
        // is direct under one layout and over-aligned under the other would
        // be freed through the wrong path.
        let new_is_direct = layout.align() <= MIN_ALIGN && layout.align() <= new_size;
        if direct(&layout) && new_is_direct {
            let heap = HEAP.load(Ordering::Acquire);
            // SAFETY: `ptr` is a direct block of this heap; `HeapReAlloc`
            // preserves contents up to the smaller size (the trait's contract).
            unsafe { HeapReAlloc(heap, 0, ptr, new_size) }
        } else {
            // SAFETY: the trait's default `realloc` (alloc, copy, dealloc) is
            // correct for any layout; the over-aligned block header would not
            // survive `HeapReAlloc` moving the block, so it is not used here.
            // The new layout keeps the old alignment, as the trait requires.
            let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
            let new_ptr = unsafe { self.alloc(new_layout) };
            if !new_ptr.is_null() {
                let copy = core::cmp::min(layout.size(), new_size);
                // SAFETY: both blocks are valid for `copy` bytes and do not
                // overlap (distinct heap blocks).
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

    /// kills: a wrong header offset or a lost alignment on the over-aligned
    /// path, `realloc` losing bytes, `alloc_zeroed` not zeroing, and the
    /// direct/over-aligned split disagreeing between `alloc` and `dealloc`
    /// (which would corrupt the heap and, on this OS, fault at free time).
    /// Exercises the type directly — it is not installed in the test binary.
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
            // size, freed with its own layout, and never used after free.
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
                assert_eq!(grown as usize % align, 0, "realloc alignment {size}/{align}");
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
                assert!((0..size).all(|i| zeroed.add(i).read() == 0), "zeroed {size}/{align}");
                heap.dealloc(zeroed, layout);
            }
        }
        // The heap was created exactly once and is process-global.
        let first = HEAP.load(Ordering::Acquire);
        assert_ne!(first, 0);
        assert_eq!(super::heap(), first);
    }

    /// kills: the direct/over-aligned split drifting from the standard
    /// library's rule (`align <= 16 && align <= size`).
    #[test]
    fn direct_rule_matches_std() {
        assert!(direct(&Layout::from_size_align(16, 16).unwrap()));
        assert!(direct(&Layout::from_size_align(1, 1).unwrap()));
        assert!(!direct(&Layout::from_size_align(8, 16).unwrap()));
        assert!(!direct(&Layout::from_size_align(64, 32).unwrap()));
    }
}
