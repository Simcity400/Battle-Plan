use super::{SegmentHeap, pages, windows_heap};
use std::alloc::{GlobalAlloc, Layout};
use std::ptr::with_exposed_provenance_mut;

#[test]
fn reallocates_from_the_private_heap_to_pages_and_back() {
    let small = Layout::from_size_align(23, 16).unwrap();
    let large = Layout::from_size_align(pages::LARGE + 1, small.align()).unwrap();
    let expected = [0xf3; 23];
    assert!(windows_heap::for_layout(small).is_some());
    assert!(pages::own(large.size(), large.align()));

    // SAFETY: both layouts are nonzero and the blocks are accessed only while live.
    unsafe {
        let block = SegmentHeap.alloc(small);
        assert!(!block.is_null());
        block.copy_from_nonoverlapping(expected.as_ptr(), expected.len());

        let grown = SegmentHeap.realloc(block, small, large.size());
        assert!(!grown.is_null());
        assert_eq!(std::slice::from_raw_parts(grown, expected.len()), expected);

        let shrunk = SegmentHeap.realloc(grown, large, small.size());
        assert!(!shrunk.is_null());
        assert_eq!(std::slice::from_raw_parts(shrunk, expected.len()), expected);
        SegmentHeap.dealloc(shrunk, small);
    }
}

#[test]
fn zeroes_and_aligns_allocations_in_each_backend() {
    for (size, alignment) in [(19, 8), (33, 16), (33, 64), (2 << 20, 4096)] {
        let layout = Layout::from_size_align(size, alignment).unwrap();
        // SAFETY: allocation uses a nonzero layout. Its initialized bytes are read before it is freed.
        unsafe {
            let block = SegmentHeap.alloc_zeroed(layout);
            assert!(!block.is_null());
            assert_eq!(block.addr() % alignment, 0);
            assert!(std::slice::from_raw_parts(block, size).iter().all(|&byte| byte == 0));
            SegmentHeap.dealloc(block, layout);
        }
    }
}

#[test]
fn the_private_heap_can_free_a_zero_byte_request() {
    let heap = windows_heap::for_layout(Layout::from_size_align(0, 16).unwrap()).unwrap();
    // SAFETY: HeapAlloc accepts zero bytes. The returned pointer is freed without being dereferenced.
    unsafe {
        let block = heap.alloc(0, true);
        assert!(!block.is_null());
        heap.free(block);
    }
}

#[test]
fn frees_a_private_heap_allocation_on_another_thread() {
    let layout = Layout::from_size_align(29, 16).unwrap();
    assert!(windows_heap::for_layout(layout).is_some());
    // SAFETY: the layout is valid and nonzero. The new thread takes ownership of the block.
    let address = unsafe { SegmentHeap.alloc(layout) }.expose_provenance();
    assert_ne!(address, 0);
    std::thread::spawn(move || {
        // SAFETY: the allocation is live and has exposed provenance. Only this thread frees it.
        unsafe { SegmentHeap.dealloc(with_exposed_provenance_mut(address), layout) };
    })
    .join()
    .unwrap();
}
