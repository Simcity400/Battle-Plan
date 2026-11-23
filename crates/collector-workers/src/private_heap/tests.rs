use super::*;
use std::sync::atomic::AtomicBool;
use windows_sys::Win32::System::Memory::{GetProcessHeaps, MEMORY_BASIC_INFORMATION, VirtualQuery};

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
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn large_growth_moves_with_its_payload_and_a_failed_move_keeps_the_old_block() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(64 * 1024, 16).unwrap();
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
    let grown_size = 64 * 1024 * 1024;
    heap.0.failures.store(ALLOCATE, Ordering::Relaxed);
    assert!(unsafe { heap.0.realloc(pointer, layout, grown_size) }.is_null());
    assert_eq!(heap.0.failures.load(Ordering::Relaxed), 0);
    assert!(intact(pointer));
    let grown = unsafe { heap.0.realloc(pointer, layout, grown_size) };
    assert!(!grown.is_null());
    let committed = region(grown);
    assert_eq!(committed.State, MEM_COMMIT);
    assert!(committed.RegionSize >= grown_size);
    assert!(intact(grown));
    unsafe { grown.add(grown_size - 1).write(0x5a) };
    let grown_layout = Layout::from_size_align(grown_size, 16).unwrap();
    unsafe { heap.0.dealloc(grown, grown_layout) };
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn creation_failure_is_cached_without_fallback() {
    let heap = Heap::new();
    heap.0.failures.store(CREATE, Ordering::Relaxed);
    let layout = Layout::from_size_align(23, 16).unwrap();
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
    assert_eq!(heap.0.failures.load(Ordering::Relaxed), 0);
    assert!(unsafe { heap.0.alloc_zeroed(layout) }.is_null());
    assert!(!collector_memory::terminal::failed());
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
    assert!(!collector_memory::terminal::failed());
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
    let mut count = unsafe { GetProcessHeaps(0, ptr::null_mut()) };
    loop {
        let mut heaps = vec![ptr::null_mut(); count as usize];
        let returned = unsafe { GetProcessHeaps(count, heaps.as_mut_ptr()) };
        assert!(returned > 0);
        if returned <= count {
            heaps.truncate(returned as usize);
            return heaps;
        }
        count = returned;
    }
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
    assert!(unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    assert!(!process_heaps().contains(&handle));
    assert!(heap.0.pages.made.load(Ordering::Relaxed).is_null());
    assert!(!unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
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
    collector_test_support::with_allocations_denied(|| assert!(heap.0.ready()));
    assert!(
        heap.0
            .heap
            .get()
            .is_some_and(|domain| domain.handle.load(Ordering::Relaxed) != 0)
    );
    let failed = Heap::new();
    failed.0.failures.store(CREATE, Ordering::Relaxed);
    collector_test_support::with_allocations_denied(|| assert!(!failed.0.ready()));
    assert!(!failed.0.ready());
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
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn reallocation_stays_within_a_class_and_moves_across_classes() {
    let heap = Heap::new();
    let layout = |size| Layout::from_size_align(size, 8).unwrap();
    let pointer = unsafe { heap.0.alloc(layout(40)) };
    assert!(!pointer.is_null());
    unsafe { pointer.write_bytes(0x11, 40) };
    assert_eq!(unsafe { heap.0.realloc(pointer, layout(40), 48) }, pointer);
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
    assert!(!collector_memory::terminal::failed());
}

fn heap_blocks(heap: &PrivateHeap, class: usize, count: usize) -> Vec<*mut u8> {
    let blocks: Vec<*mut u8> = (0..count)
        .map(|_| unsafe { HeapAlloc(heap.heap(), 0, class_size(class)) }.cast())
        .collect();
    assert!(blocks.iter().all(|block| !block.is_null()));
    blocks
}

#[test]
fn a_full_cache_passes_a_batch_through_the_depot_to_another_thread() {
    let heap = Heap::new();
    let class = super::class(SMALL_LIMIT).unwrap();
    assert_eq!(batch(class), 1);
    let blocks = heap_blocks(&heap.0, class, 3);
    let (mut freeing, mut allocating) = (Cache::EMPTY, Cache::EMPTY);
    for &block in &blocks {
        freeing.keep(class, block, &heap.0.depot);
    }
    let list = freeing.lists[class];
    assert_eq!((list.head, list.count, list.spare), (blocks[2], 1, blocks[1]));
    assert_eq!(allocating.take(class, &heap.0.depot), blocks[0]);
    assert!(allocating.take(class, &heap.0.depot).is_null());
    assert_eq!(freeing.take(class, &heap.0.depot), blocks[2]);
    assert_eq!(freeing.take(class, &heap.0.depot), blocks[1]);
    for block in blocks {
        assert_ne!(unsafe { HeapFree(heap.0.heap(), 0, block.cast()) }, 0);
    }
}

#[test]
fn a_small_class_batch_keeps_every_block_in_order() {
    let heap = Heap::new();
    let class = super::class(16).unwrap();
    let batch = batch(class) as usize;
    let blocks = heap_blocks(&heap.0, class, 3 * batch);
    let (mut freeing, mut allocating) = (Cache::EMPTY, Cache::EMPTY);
    for &block in &blocks {
        freeing.keep(class, block, &heap.0.depot);
    }
    let taken: Vec<*mut u8> = (0..batch)
        .map(|_| allocating.take(class, &heap.0.depot))
        .collect();
    assert!(
        taken
            .iter()
            .copied()
            .eq(blocks[..batch].iter().rev().copied())
    );
    assert!(allocating.take(class, &heap.0.depot).is_null());
    for block in blocks {
        assert_ne!(unsafe { HeapFree(heap.0.heap(), 0, block.cast()) }, 0);
    }
}

#[test]
fn slab_backed_small_blocks_never_enter_the_private_heap_lock() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(SMALL_LIMIT, 16).unwrap();
    let blocks: Vec<*mut u8> = (0..64)
        .map(|_| unsafe { heap.0.alloc(layout) })
        .collect();
    assert!(blocks.iter().all(|block| !block.is_null()));
    for &block in &blocks {
        unsafe { heap.0.dealloc(block, layout) };
    }
    assert_eq!(heap.0.hold().heap.calls, 0);
    assert_eq!(heap.0.other().heap.calls, 0);
    assert!(heap.0.hold().pages.calls > 0);
}

fn churn(heap: &PrivateHeap, shape: &[(usize, usize)]) {
    let mut blocks = Vec::new();
    for &(size, count) in shape {
        let layout = Layout::from_size_align(size, 16).unwrap();
        for _ in 0..count {
            let block = unsafe { heap.alloc(layout) };
            assert!(!block.is_null());
            unsafe { block.write_bytes(size as u8, 16) };
            blocks.push((block, layout));
        }
    }
    for (block, layout) in blocks {
        assert!(
            unsafe { std::slice::from_raw_parts(block, 16) }
                .iter()
                .all(|&byte| byte == layout.size() as u8)
        );
        unsafe { heap.dealloc(block, layout) };
    }
}

#[test]
fn the_hold_path_takes_no_lock_while_other_threads_hammer_the_heap() {
    const HOUR: [(usize, usize); 4] = [(16, 64), (112, 64), (2048, 8), (100 * 1024, 1)];
    const ROUND: [(usize, usize); 4] = [(16, 4096), (112, 1024), (2048, 512), (100 * 1024, 64)];
    let heap = Heap::new();
    let heap = &heap.0;
    let stop = AtomicBool::new(false);
    let rounds = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                off_hold_path();
                while !stop.load(Ordering::Relaxed) {
                    churn(heap, &ROUND);
                    rounds.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        churn(heap, &HOUR);
        while rounds.load(Ordering::Relaxed) < 4 {
            std::thread::yield_now();
        }
        let before = heap.hold();
        let rounds_before = rounds.load(Ordering::Relaxed);
        let mut hours = 0;
        while hours < 100 || rounds.load(Ordering::Relaxed) < rounds_before + 8 {
            churn(heap, &HOUR);
            hours += 1;
        }
        let held = heap.hold().since(before);
        stop.store(true, Ordering::Relaxed);
        assert_eq!((held.heap.calls, held.heap.ticks), (0, 0));
    });
    assert_eq!(heap.other().heap.calls, 0);
    assert!(!collector_memory::terminal::failed());
}

fn region(pointer: *const u8) -> MEMORY_BASIC_INFORMATION {
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let size = size_of::<MEMORY_BASIC_INFORMATION>();
    assert_ne!(unsafe { VirtualQuery(pointer.cast(), &mut info, size) }, 0);
    info
}

#[test]
fn a_large_block_grows_and_shrinks_in_place_and_its_reservation_is_reused_zeroed() {
    let heap = Heap::new();
    let layout = |size| Layout::from_size_align(size, 16).unwrap();
    let pointer = unsafe { heap.0.alloc(layout(70_000)) };
    assert!(!pointer.is_null());
    unsafe { pointer.write_bytes(0x3c, 70_000) };
    assert_eq!(unsafe { heap.0.realloc(pointer, layout(70_000), 120_000) }, pointer);
    let bytes = |size| unsafe { std::slice::from_raw_parts(pointer, size) };
    assert!(bytes(70_000).iter().all(|&byte| byte == 0x3c));
    unsafe { pointer.add(70_000).write_bytes(0x3c, 50_000) };
    let pages_before_shrink = heap.0.hold().pages.calls;
    assert_eq!(unsafe { heap.0.realloc(pointer, layout(120_000), 66_000) }, pointer);
    assert_eq!(heap.0.hold().pages.calls, pages_before_shrink);
    assert!(bytes(66_000).iter().all(|&byte| byte == 0x3c));
    unsafe { heap.0.dealloc(pointer, layout(66_000)) };
    let reused = unsafe { heap.0.alloc_zeroed(layout(100_000)) };
    assert_eq!(reused, pointer);
    assert!(bytes(100_000).iter().all(|&byte| byte == 0));
    let moved = unsafe { heap.0.realloc(reused, layout(100_000), 140_000) };
    assert!(!moved.is_null());
    assert_ne!(moved, reused);
    assert!(
        unsafe { std::slice::from_raw_parts(moved, 100_000) }
            .iter()
            .all(|&byte| byte == 0)
    );
    unsafe { heap.0.dealloc(moved, layout(140_000)) };
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn every_refused_page_call_returns_null_and_keeps_what_it_had() {
    let heap = Heap::new();
    let heap = &heap.0;
    let layout = |size| Layout::from_size_align(size, 16).unwrap();
    let refuse = |ahead| heap.page_refusal.store(ahead, Ordering::Relaxed);
    let refused = || heap.page_refusal.load(Ordering::Relaxed) == 0;
    for ahead in [1, 2] {
        refuse(ahead);
        assert!(unsafe { heap.alloc(layout(70_000)) }.is_null());
        assert!(refused());
        assert!(heap.pages.made.load(Ordering::Relaxed).is_null());
        assert_eq!(heap.pages.kept.load(Ordering::Relaxed), 0);
    }
    let pointer = unsafe { heap.alloc(layout(70_000)) };
    assert!(!pointer.is_null());
    unsafe { pointer.write_bytes(0x6d, 70_000) };
    let record = PrivateHeap::record(pointer);
    let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
    refuse(1);
    assert!(unsafe { heap.realloc(pointer, layout(70_000), 120_000) }.is_null());
    assert!(refused());
    assert_eq!(
        unsafe { (*record).committed.load(Ordering::Relaxed) },
        committed
    );
    assert!(
        unsafe { std::slice::from_raw_parts(pointer, 70_000) }
            .iter()
            .all(|&byte| byte == 0x6d)
    );
    unsafe { heap.dealloc(pointer, layout(70_000)) };
    let kept = heap.pages.kept.load(Ordering::Relaxed);
    assert_ne!(kept, 0);
    refuse(1);
    assert!(unsafe { heap.alloc(layout(120_000)) }.is_null());
    assert!(refused());
    assert_eq!(heap.pages.kept.load(Ordering::Relaxed), kept);
    let reused = unsafe { heap.alloc(layout(120_000)) };
    assert_eq!(reused, pointer);
    assert_eq!(heap.pages.kept.load(Ordering::Relaxed), 0);
    unsafe { heap.dealloc(reused, layout(120_000)) };
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn a_freed_large_body_past_the_bound_is_decommitted_off_the_hold_path_and_kept_on_it() {
    let heap = Heap::new();
    let heap = &heap.0;
    let layout = Layout::from_size_align(1024 * 1024, 16).unwrap();
    let pointer = unsafe { heap.alloc(layout) };
    assert!(!pointer.is_null());
    unsafe { pointer.write_bytes(0x51, layout.size()) };
    heap.pages.kept.store(KEPT_BYTES, Ordering::Relaxed);
    let before = heap.hold();
    unsafe { heap.dealloc(pointer, layout) };
    assert_eq!(heap.hold().pages.calls, before.pages.calls);
    assert_eq!(region(pointer).State, MEM_COMMIT);
    let address = pointer as usize;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            off_hold_path();
            let reused = unsafe { heap.alloc(layout) };
            assert_eq!(reused as usize, address);
            heap.pages.kept.store(KEPT_BYTES, Ordering::Relaxed);
            unsafe { heap.dealloc(reused, layout) };
            let body = (reused as usize / GRANULARITY * GRANULARITY + PAGE) as *const u8;
            assert_eq!(region(body).State, MEM_RESERVE);
            assert_eq!(heap.pages.kept.load(Ordering::Relaxed), KEPT_BYTES);
        });
    });
    let reused = unsafe { heap.alloc_zeroed(layout) };
    assert_eq!(reused, pointer);
    assert!(
        unsafe { std::slice::from_raw_parts(reused, layout.size()) }
            .iter()
            .all(|&byte| byte == 0)
    );
    let warm = unsafe { heap.alloc(layout) };
    assert!(!warm.is_null());
    heap.pages.kept.store(0, Ordering::Relaxed);
    unsafe { heap.dealloc(warm, layout) };
    let warm_address = warm as usize;
    let cold_address = reused as usize;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            off_hold_path();
            heap.pages.kept.store(KEPT_BYTES, Ordering::Relaxed);
            unsafe { heap.dealloc(cold_address as *mut u8, layout) };
        });
    });
    let before_warm = heap.hold().pages.calls;
    let picked = unsafe { heap.alloc(layout) };
    assert_eq!(picked as usize, warm_address);
    assert_eq!(heap.hold().pages.calls, before_warm);
    unsafe { heap.dealloc(picked, layout) };
}

#[test]
fn an_exiting_thread_returns_its_batches_and_partial_blocks_to_the_depot() {
    let heap = Heap::new();
    let whole = Layout::from_size_align(SMALL_LIMIT, 16).unwrap();
    let partial = Layout::from_size_align(16, 16).unwrap();
    let whole_class = super::class(SMALL_LIMIT).unwrap();
    let partial_class = super::class(16).unwrap();
    let (freed_whole, freed_partial) = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let heap = &heap.0;
                let blocks: Vec<*mut u8> = (0..3).map(|_| unsafe { heap.alloc(whole) }).collect();
                let smalls: Vec<*mut u8> =
                    (0..4).map(|_| unsafe { heap.alloc(partial) }).collect();
                assert!(blocks.iter().chain(&smalls).all(|block| !block.is_null()));
                for &block in &blocks {
                    unsafe { heap.dealloc(block, whole) };
                }
                for &block in &smalls[..3] {
                    unsafe { heap.dealloc(block, partial) };
                }
                // SAFETY: This DLL thread-detach notification follows the thread's collector work.
                unsafe { heap.thread_exit() };
                assert_eq!(heap.hold().heap.calls, 0);
                unsafe { heap.dealloc(smalls[3], partial) };
                let late = unsafe { heap.alloc(partial) };
                assert!(!late.is_null());
                unsafe { heap.dealloc(late, partial) };
                assert_eq!(heap.hold().heap.calls, 0);
                (
                    blocks.into_iter().map(|block| block as usize).collect::<Vec<_>>(),
                    smalls.into_iter().map(|block| block as usize).collect::<Vec<_>>(),
                )
            })
            .join()
            .unwrap()
    });
    let mut taken_whole = Vec::new();
    loop {
        let block = heap.0.depot.take(whole_class);
        if block.is_null() {
            break;
        }
        taken_whole.push(block as usize);
    }
    for block in freed_whole {
        assert!(taken_whole.contains(&block));
    }
    let mut taken_partial = Vec::new();
    loop {
        let block = heap.0.depot.take_single(partial_class);
        if block.is_null() {
            break;
        }
        taken_partial.push(block as usize);
    }
    for block in freed_partial {
        assert!(taken_partial.contains(&block));
    }
}

#[test]
fn blocks_freed_on_other_threads_come_back_intact() {
    let heap = Heap::new();
    let layouts = [16, 112, 2048].map(|size| Layout::from_size_align(size, 16).unwrap());
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel::<(usize, usize, u8)>();
        let heap = &heap.0;
        for producer in 1..=4_u8 {
            let sender = sender.clone();
            scope.spawn(move || {
                for round in 0..10_000 {
                    let index = round % layouts.len();
                    let block = unsafe { heap.alloc(layouts[index]) };
                    assert!(!block.is_null());
                    unsafe { block.write_bytes(producer, layouts[index].size()) };
                    sender.send((block as usize, index, producer)).unwrap();
                }
            });
        }
        drop(sender);
        for (address, index, fill) in receiver {
            let block = address as *mut u8;
            let bytes = unsafe { std::slice::from_raw_parts(block, layouts[index].size()) };
            assert!(bytes.iter().all(|&byte| byte == fill));
            unsafe { heap.dealloc(block, layouts[index]) };
        }
    });
    assert!(!collector_memory::terminal::failed());
}

#[test]
fn detach_forgets_cached_blocks_and_releases_every_reservation() {
    let heap = Heap::new();
    let layout = Layout::from_size_align(64, 16).unwrap();
    let pointer = unsafe { heap.0.alloc(layout) };
    assert!(!pointer.is_null());
    unsafe { heap.0.dealloc(pointer, layout) };
    let class = super::class(SMALL_LIMIT).unwrap();
    let mut other = Cache::EMPTY;
    for block in heap_blocks(&heap.0, class, 3) {
        other.keep(class, block, &heap.0.depot);
    }
    let large = Layout::from_size_align(1024 * 1024, 16).unwrap();
    let kept = unsafe { heap.0.alloc(large) };
    assert!(!kept.is_null());
    unsafe { heap.0.dealloc(kept, large) };
    assert_eq!(
        heap.0.pages.made.load(Ordering::Relaxed),
        PrivateHeap::record(kept)
    );
    let handle = heap.0.heap();
    assert!(unsafe { heap.0.detach(DLL_PROCESS_DETACH, ptr::null_mut()) });
    assert!(!process_heaps().contains(&handle));
    assert!((0..CLASSES).all(|class| heap.0.depot.take(class).is_null()));
    assert!((0..CLASSES).all(|class| heap.0.depot.take_single(class).is_null()));
    assert!(heap.0.pages.free.iter().all(|list| list.pop().is_null()));
    assert!(heap.0.pages.made.load(Ordering::Relaxed).is_null());
    assert_eq!(heap.0.pages.kept.load(Ordering::Relaxed), 0);
    let cache = unsafe { &(*thread()).cache };
    assert!(
        cache
            .lists
            .iter()
            .all(|list| list.head.is_null() && list.spare.is_null())
    );
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
    assert!(unsafe { heap.0.alloc(large) }.is_null());
}

#[test]
fn reusing_a_large_reservation_off_hold_trims_excess_committed_tail() {
    let heap = Heap::new();
    let heap = &heap.0;
    let big = Layout::from_size_align(1024 * 1024, 16).unwrap();
    let small = Layout::from_size_align(600 * 1024, 16).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            off_hold_path();
            let first = unsafe { heap.alloc(big) };
            assert!(!first.is_null());
            unsafe { first.write_bytes(0x4a, big.size()) };
            unsafe { heap.dealloc(first, big) };
            let reused = unsafe { heap.alloc(small) };
            assert_eq!(reused, first);
            let record = PrivateHeap::record(reused);
            let committed = unsafe { (*record).committed.load(Ordering::Relaxed) };
            assert!(committed < big.size());
            let tail = unsafe { record.cast::<u8>().add(committed) };
            assert_eq!(region(tail).State, MEM_RESERVE);
            unsafe { heap.dealloc(reused, small) };
        });
    });
}

#[test]
fn reclaim_empty_slabs_decommits_idle_slabs_and_reuses_reservations() {
    let heap = Heap::new();
    let heap = &heap.0;
    let layout = Layout::from_size_align(SMALL_LIMIT, 16).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            off_hold_path();
            let blocks: Vec<*mut u8> = (0..128)
                .map(|_| {
                    let block = unsafe { heap.alloc(layout) };
                    assert!(!block.is_null());
                    unsafe { block.write_bytes(0x5a, layout.size()) };
                    block
                })
                .collect();
            for block in blocks {
                unsafe { heap.dealloc(block, layout) };
            }
            unsafe { heap.thread_exit() };
        });
    });
    std::thread::scope(|scope| {
        scope.spawn(move || {
            off_hold_path();
            let reclaimed = heap.reclaim_empty_slabs();
            assert!(reclaimed >= 12 * (GRANULARITY - PAGE));
            let blocks: Vec<*mut u8> = (0..32)
                .map(|_| {
                    let block = unsafe { heap.alloc(layout) };
                    assert!(!block.is_null());
                    unsafe { block.write_bytes(0x3c, layout.size()) };
                    block
                })
                .collect();
            for block in blocks {
                unsafe { heap.dealloc(block, layout) };
            }
            unsafe { heap.thread_exit() };
        });
    });
}

#[test]
fn reclaim_empty_slabs_decommits_free_page_blocks_in_partially_live_slabs() {
    let heap = Heap::new();
    let heap = &heap.0;
    for size in [4096, SMALL_LIMIT] {
        let layout = Layout::from_size_align(size, 16).unwrap();
        let per_slab = slab_count(super::class(size).unwrap());
        let total = per_slab * 6;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                off_hold_path();
                let blocks: Vec<*mut u8> = (0..total)
                    .map(|_| {
                        let block = unsafe { heap.alloc(layout) };
                        assert!(!block.is_null());
                        assert_eq!(block as usize % PAGE, 0);
                        unsafe { block.write_bytes(0x6b, size) };
                        block
                    })
                    .collect();
                let mut pinned = Vec::new();
                let mut freed = Vec::new();
                for (index, &block) in blocks.iter().enumerate() {
                    if index % per_slab == 0 {
                        pinned.push(block);
                    } else {
                        freed.push(block);
                        unsafe { heap.dealloc(block, layout) };
                    }
                }
                let reclaimed = heap.reclaim_empty_slabs();
                assert!(reclaimed >= (freed.len() - WARM_PAGE_BLOCKS) * size);
                for &block in &pinned {
                    assert!(
                        unsafe { std::slice::from_raw_parts(block, size) }
                            .iter()
                            .all(|&b| b == 0x6b)
                    );
                }
                assert!(
                    freed
                        .iter()
                        .any(|&block| region(block).State == MEM_RESERVE)
                );
                let reused: Vec<*mut u8> = (0..freed.len())
                    .map(|_| {
                        let block = unsafe { heap.alloc_zeroed(layout) };
                        assert!(!block.is_null());
                        assert!(
                            unsafe { std::slice::from_raw_parts(block, size) }
                                .iter()
                                .all(|&b| b == 0)
                        );
                        unsafe { block.write_bytes(0x2d, size) };
                        block
                    })
                    .collect();
                for block in pinned.into_iter().chain(reused) {
                    unsafe { heap.dealloc(block, layout) };
                }
                let full_reclaimed = heap.reclaim_empty_slabs();
                assert!(full_reclaimed >= 4 * (GRANULARITY - PAGE));
                unsafe { heap.thread_exit() };
            });
        });
    }
}

#[test]
fn reclaim_empty_slabs_decommits_subpage_4k_pages_and_zeroes_reused_allocations() {
    let heap = Heap::new();
    let heap = &heap.0;
    for size in [32, 48, 2048] {
        let layout = Layout::from_size_align(size, 16).unwrap();
        let per_slab = slab_count(super::class(size).unwrap());
        let total = per_slab * 3;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                off_hold_path();
                let blocks: Vec<*mut u8> = (0..total)
                    .map(|_| {
                        let block = unsafe { heap.alloc(layout) };
                        assert!(!block.is_null());
                        unsafe { block.write_bytes(0x5d, size) };
                        block
                    })
                    .collect();
                let mut pinned = Vec::new();
                let mut freed = Vec::new();
                for (index, &block) in blocks.iter().enumerate() {
                    let offset = block as usize & (GRANULARITY - 1);
                    if offset < PAGE && index % per_slab == 0 {
                        pinned.push(block);
                    } else {
                        freed.push(block);
                        unsafe { heap.dealloc(block, layout) };
                    }
                }
                let reclaimed = heap.reclaim_empty_slabs();
                assert!(reclaimed >= 10 * PAGE, "size={size} reclaimed={reclaimed}");
                for &block in &pinned {
                    assert!(
                        unsafe { std::slice::from_raw_parts(block, size) }
                            .iter()
                            .all(|&b| b == 0x5d)
                    );
                }
                assert!(
                    freed
                        .iter()
                        .any(|&block| region(block).State == MEM_RESERVE)
                );
                let reused: Vec<*mut u8> = (0..freed.len())
                    .map(|_| {
                        let block = unsafe { heap.alloc_zeroed(layout) };
                        assert!(!block.is_null());
                        assert!(
                            unsafe { std::slice::from_raw_parts(block, size) }
                                .iter()
                                .all(|&b| b == 0)
                        );
                        unsafe { block.write_bytes(0x3a, size) };
                        block
                    })
                    .collect();
                for block in pinned.into_iter().chain(reused) {
                    unsafe { heap.dealloc(block, layout) };
                }
                let full_reclaimed = heap.reclaim_empty_slabs();
                assert!(full_reclaimed >= GRANULARITY - PAGE);
                let fresh_zeroed = unsafe { heap.alloc_zeroed(layout) };
                assert!(!fresh_zeroed.is_null());
                assert!(
                    unsafe { std::slice::from_raw_parts(fresh_zeroed, size) }
                        .iter()
                        .all(|&b| b == 0)
                );
                unsafe { heap.dealloc(fresh_zeroed, layout) };
                unsafe { heap.thread_exit() };
            });
        });
    }
}
