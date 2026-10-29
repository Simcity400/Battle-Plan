use super::*;
use windows_sys::Win32::System::Memory::{GetProcessHeaps, HeapSize};

pub(super) const CREATE: usize = 1;
pub(super) const ALLOCATE: usize = 2;
pub(super) const REALLOCATE: usize = 4;

struct Heap(PrivateHeap);
impl Heap {
    fn new() -> Self {
        Self(PrivateHeap::new())
    }
}
impl Drop for Heap {
    fn drop(&mut self) {
        // Every test joins its workers and frees its ordinary live blocks first.
        // The detach test deliberately leaves a retained block for HeapDestroy.
        unsafe { self.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) };
    }
}

#[test]
fn alignment_and_zeroing() {
    let heap = Heap::new();
    for align in [1, 2, 4, 8, 16, 32, 64, 256, 4096, 65536] {
        for size in [1, 3, 16, 17, 100, 4097, 65539] {
            let layout = Layout::from_size_align(size, align).unwrap();
            for zeroed in [false, true] {
                let pointer = unsafe {
                    if zeroed {
                        heap.0.alloc_zeroed(layout)
                    } else {
                        heap.0.alloc(layout)
                    }
                };
                assert!(!pointer.is_null());
                assert_eq!(pointer as usize % align, 0);
                let bytes = unsafe { std::slice::from_raw_parts_mut(pointer, size) };
                if zeroed {
                    assert!(bytes.iter().all(|&byte| byte == 0));
                }
                bytes.fill(0x5a);
                unsafe { heap.0.dealloc(pointer, layout) };
            }
        }
    }
}

#[test]
fn realloc_growth_and_shrink_preserve_payload_and_alignment() {
    let heap = Heap::new();
    for align in [1, 16, 32, 256, 4096, 65536] {
        let mut layout = Layout::from_size_align(257, align).unwrap();
        let mut pointer = unsafe { heap.0.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe { pointer.write_bytes(0xa5, layout.size()) };
        for new_size in [8193, 31, 32769, 1] {
            let next = unsafe { heap.0.realloc(pointer, layout, new_size) };
            assert!(!next.is_null());
            assert_eq!(next as usize % align, 0);
            assert!(
                unsafe { std::slice::from_raw_parts(next, layout.size().min(new_size)) }
                    .iter()
                    .all(|&byte| byte == 0xa5)
            );
            layout = Layout::from_size_align(new_size, align).unwrap();
            pointer = next;
            unsafe { pointer.write_bytes(0xa5, new_size) };
        }
        unsafe { heap.0.dealloc(pointer, layout) };
    }
}

#[test]
fn failed_realloc_preserves_the_old_allocation() {
    let heap = Heap::new();
    for align in [16, 32, 4096, 65536] {
        let layout = Layout::from_size_align(257, align).unwrap();
        let pointer = unsafe { heap.0.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe { pointer.write_bytes(0x47, layout.size()) };
        for new_size in [1025, 13] {
            heap.0.failures.store(
                if align <= HEAP_ALIGNMENT {
                    REALLOCATE
                } else {
                    ALLOCATE
                },
                Ordering::Relaxed,
            );
            assert!(unsafe { heap.0.realloc(pointer, layout, new_size) }.is_null());
            assert!(
                unsafe { std::slice::from_raw_parts(pointer, layout.size()) }
                    .iter()
                    .all(|&byte| byte == 0x47)
            );
        }
        unsafe { heap.0.dealloc(pointer, layout) };
    }
    // The allocator itself never records a capture failure; checked callers do.
    assert!(!crate::allocation::failed());
}

#[test]
fn large_growth_moves_with_its_payload_and_a_failed_move_keeps_the_old_block() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(UNLOCKED_GROWTH, 16).unwrap();
    let pointer = unsafe { heap.0.alloc(layout) };
    assert!(!pointer.is_null());
    let payload = unsafe { std::slice::from_raw_parts_mut(pointer, layout.size()) };
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte = index as u8;
    }
    let bytes = |pointer: *mut u8| unsafe { std::slice::from_raw_parts(pointer, layout.size()) };
    let intact = |pointer| {
        bytes(pointer)
            .iter()
            .enumerate()
            .all(|(i, &b)| b == i as u8)
    };
    // Far beyond any in-place extension, so growth must move the block.
    let grown_size = 64 * 1024 * 1024;
    heap.0.failures.store(ALLOCATE, Ordering::Relaxed);
    assert!(unsafe { heap.0.realloc(pointer, layout, grown_size) }.is_null());
    assert_eq!(heap.0.failures.load(Ordering::Relaxed), 0);
    assert!(intact(pointer));
    let grown = unsafe { heap.0.realloc(pointer, layout, grown_size) };
    assert!(!grown.is_null());
    let size = unsafe { HeapSize(heap.0.heap(), 0, grown.cast()) };
    assert!(size >= grown_size);
    assert!(intact(grown));
    unsafe { grown.add(grown_size - 1).write(0x5a) };
    let grown_layout = Layout::from_size_align(grown_size, 16).unwrap();
    unsafe { heap.0.dealloc(grown, grown_layout) };
    assert!(!crate::allocation::failed());
}

#[test]
fn creation_failure_is_cached_without_fallback() {
    let heap = Heap::new();
    heap.0.failures.store(CREATE, Ordering::Relaxed);
    let layout = Layout::from_size_align(23, 16).unwrap();
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
    // The one-shot failure has cleared. A retry would create a heap if this
    // had not cached the failed creation, but still returns null.
    assert_eq!(heap.0.failures.load(Ordering::Relaxed), 0);
    assert!(unsafe { heap.0.alloc_zeroed(layout) }.is_null());
    assert!(!crate::allocation::failed());
}

#[test]
fn allocation_failure_and_size_overflow_return_null() {
    let heap = Heap::new();
    assert!(unsafe { heap.0.allocate(usize::MAX, 64, false) }.is_null());
    assert!(heap.0.heap.get().is_none());
    let layout = Layout::from_size_align(23, 64).unwrap();
    for zeroed in [false, true] {
        heap.0.failures.store(ALLOCATE, Ordering::Relaxed);
        assert!(unsafe { heap.0.allocate(layout.size(), layout.align(), zeroed) }.is_null());
    }
    let pointer = unsafe { heap.0.alloc(layout) };
    assert!(!pointer.is_null());
    unsafe { heap.0.dealloc(pointer, layout) };
    assert!(!crate::allocation::failed());
}

#[test]
fn cross_thread_free_uses_the_same_serialized_heap() {
    let heap = Heap::new();
    std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for align in [16, 64, 4096, 65536] {
            let layout = Layout::from_size_align(517, align).unwrap();
            let pointer = unsafe { heap.0.alloc(layout) };
            assert!(!pointer.is_null());
            unsafe { pointer.write_bytes(0x29, layout.size()) };
            let address = pointer as usize;
            let heap = &heap.0;
            threads.push(scope.spawn(move || {
                let pointer = address as *mut u8;
                assert!(
                    unsafe { std::slice::from_raw_parts(pointer, layout.size()) }
                        .iter()
                        .all(|&v| v == 0x29)
                );
                unsafe { heap.dealloc(pointer, layout) };
                let replacement = unsafe { heap.alloc_zeroed(layout) };
                assert!(!replacement.is_null());
                (replacement as usize, layout)
            }));
        }
        for thread in threads {
            let (address, layout) = thread.join().unwrap();
            let pointer = address as *mut u8;
            assert!(
                unsafe { std::slice::from_raw_parts(pointer, layout.size()) }
                    .iter()
                    .all(|&v| v == 0)
            );
            unsafe { heap.0.dealloc(pointer, layout) };
        }
    });
}

fn process_heaps() -> Vec<HANDLE> {
    let count = unsafe { GetProcessHeaps(0, ptr::null_mut()) };
    let mut heaps = vec![ptr::null_mut(); count as usize];
    let returned = unsafe { GetProcessHeaps(count, heaps.as_mut_ptr()) };
    assert!(returned > 0 && returned <= count);
    heaps.truncate(returned as usize);
    heaps
}

#[test]
fn destroy_runs_only_for_free_library_process_detach() {
    let heap = Heap::new();
    assert!(heap.0.heap.get().is_none());
    let layout = Layout::from_size_align(1024 * 1024, 64).unwrap();
    let pointer = unsafe { heap.0.alloc_zeroed(layout) };
    assert!(!pointer.is_null());
    let handle = heap.0.heap();
    assert!(process_heaps().contains(&handle));
    for (reason, reserved) in [
        (1, ptr::null_mut()),
        (2, ptr::null_mut()),
        (3, ptr::null_mut()),
        (DLL_PROCESS_DETACH, ptr::dangling_mut::<c_void>()),
    ] {
        assert!(!unsafe { heap.0.detach(reason, reserved) });
        assert_eq!(heap.0.heap(), handle);
        assert!(process_heaps().contains(&handle));
        unsafe { pointer.write(37) };
    }
    // Reclaims the intentionally retained block as well as the empty heap.
    assert!(unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    assert!(!process_heaps().contains(&handle));
    assert!(!unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    // Cached terminal zero prevents allocating into a second heap after destroy.
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
}

#[test]
fn detach_does_not_create_an_unused_heap() {
    let heap = Heap::new();
    assert!(!unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    assert!(heap.0.heap.get().is_none());
}

#[test]
fn attach_creates_the_heap_allocation_free_and_refuses_a_failed_creation() {
    let heap = Heap::new();
    crate::test_support::with_allocations_denied(|| assert!(heap.0.ready()));
    assert!(heap.0.heap.get().is_some_and(|handle| handle.load(Ordering::Relaxed) != 0));
    let failed = Heap::new();
    failed.0.failures.store(CREATE, Ordering::Relaxed);
    crate::test_support::with_allocations_denied(|| assert!(!failed.0.ready()));
    // The failure is cached: a later attach check still refuses, creating nothing.
    assert!(!failed.0.ready());
}

/// Runs `f` with this thread held on one processor, so every list it touches is
/// the same shard's.
fn on_one_processor<R>(f: impl FnOnce() -> R) -> R {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};
    let processor = unsafe { GetCurrentProcessorNumber() } as usize;
    let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1 << (processor % 64)) };
    assert_ne!(previous, 0);
    let result = f();
    unsafe { SetThreadAffinityMask(GetCurrentThread(), previous) };
    result
}

#[test]
fn size_classes_fit_every_small_size_in_16_byte_blocks() {
    let mut previous = 0;
    for class in 0..CLASSES {
        let size = class_size(class);
        assert!(size > previous && size % HEAP_ALIGNMENT == 0);
        assert_eq!(super::class(size), Some(class));
        assert_eq!(super::class(previous + 1), Some(class));
        previous = size;
    }
    assert_eq!(previous, SMALL_LIMIT);
    assert_eq!(super::class(0), Some(0));
    assert_eq!(super::class(SMALL_LIMIT + 1), None);
    for size in 1..=SMALL_LIMIT {
        assert!(class_size(super::class(size).unwrap()) >= size);
    }
}

#[test]
fn freed_small_blocks_are_reused_and_cleared_on_request() {
    let heap = Heap::new();
    on_one_processor(|| {
        for size in [1, 100, 1024, 1025, SMALL_LIMIT] {
            let layout = Layout::from_size_align(size, 16).unwrap();
            let pointer = unsafe { heap.0.alloc(layout) };
            assert!(!pointer.is_null());
            unsafe { pointer.write_bytes(0x7c, size) };
            unsafe { heap.0.dealloc(pointer, layout) };
            let reused = unsafe { heap.0.alloc_zeroed(layout) };
            assert_eq!(reused, pointer);
            let bytes = unsafe { std::slice::from_raw_parts(reused, size) };
            assert!(bytes.iter().all(|&byte| byte == 0));
            unsafe { heap.0.dealloc(reused, layout) };
        }
    });
    assert!(!crate::allocation::failed());
}

#[test]
fn reallocation_stays_within_a_class_and_moves_across_classes() {
    let heap = Heap::new();
    let layout = |size| Layout::from_size_align(size, 8).unwrap();
    let pointer = unsafe { heap.0.alloc(layout(40)) };
    assert!(!pointer.is_null());
    unsafe { pointer.write_bytes(0x11, 40) };
    assert_eq!(unsafe { heap.0.realloc(pointer, layout(40), 48) }, pointer);
    // Through a larger class, a large block and back to a small one.
    let mut current = (pointer, 48);
    for size in [49, 40_000, 100] {
        let moved = unsafe { heap.0.realloc(current.0, layout(current.1), size) };
        assert!(!moved.is_null());
        assert_ne!(moved, current.0);
        let kept = current.1.min(size).min(40);
        assert!(
            unsafe { std::slice::from_raw_parts(moved, kept) }
                .iter()
                .all(|&byte| byte == 0x11)
        );
        current = (moved, size);
    }
    unsafe { heap.0.dealloc(current.0, layout(current.1)) };
    assert!(!crate::allocation::failed());
}

#[test]
fn a_full_class_returns_further_frees_to_the_heap() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(SMALL_LIMIT, 16).unwrap();
    let class = super::class(SMALL_LIMIT).unwrap();
    on_one_processor(|| {
        let blocks = [(); 3].map(|()| unsafe { heap.0.alloc(layout) });
        assert!(blocks.iter().all(|block| !block.is_null()));
        for block in blocks {
            unsafe { heap.0.dealloc(block, layout) };
        }
        heap.0.shard().with(|lists| {
            let list = lists[class];
            assert_eq!(list.count as usize, SHARD_CLASS_BYTES / SMALL_LIMIT);
            assert_eq!(list.kept as usize, SHARD_CLASS_BYTES / SMALL_LIMIT);
            assert_eq!(list.returned as usize, 3 - SHARD_CLASS_BYTES / SMALL_LIMIT);
        });
    });
    // The status itself never allocates; only this test's JSON value does.
    let snapshot = heap.0.status();
    crate::test_support::with_allocations_denied(|| {
        serde_json::to_writer(std::io::sink(), &snapshot).unwrap()
    });
    let status = serde_json::to_value(&snapshot).unwrap();
    let classes = status["classes"].as_array().unwrap();
    let row = classes
        .iter()
        .find(|row| row["bytes"] == SMALL_LIMIT)
        .unwrap();
    assert_eq!(row["misses"], 3);
}

#[test]
fn detach_forgets_cached_small_blocks() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(64, 16).unwrap();
    let pointer = unsafe { heap.0.alloc(layout) };
    assert!(!pointer.is_null());
    unsafe { heap.0.dealloc(pointer, layout) };
    let handle = heap.0.heap();
    assert!(unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    assert!(!process_heaps().contains(&handle));
    for shard in &heap.0.shards {
        shard.with(|lists| assert!(lists.iter().all(|list| list.head.is_null())));
    }
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
}
