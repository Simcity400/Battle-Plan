use super::*;
use windows_sys::Win32::System::Memory::GetProcessHeaps;

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
fn creation_failure_is_cached_without_fallback() {
    let heap = Heap::new();
    heap.0.failures.store(CREATE, Ordering::Relaxed);
    let layout = Layout::from_size_align(23, 16).unwrap();
    assert!(unsafe { heap.0.alloc(layout) }.is_null());
    // The one-shot failure has cleared. A retry would create a heap if this
    // had not cached the failed creation, but still returns null.
    assert_eq!(heap.0.failures.load(Ordering::Relaxed), 0);
    assert!(unsafe { heap.0.alloc_zeroed(layout) }.is_null());
    assert_eq!(heap.0.committed_bytes(), Some(0));
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
    assert_eq!(heap.0.committed_bytes(), Some(0));
    assert!(heap.0.heap.get().is_none());
    let layout = Layout::from_size_align(1024 * 1024, 64).unwrap();
    let pointer = unsafe { heap.0.alloc_zeroed(layout) };
    assert!(!pointer.is_null());
    let handle = heap.0.heap();
    assert!(process_heaps().contains(&handle));
    if let Some(committed) = heap.0.committed_bytes() {
        assert!(committed >= layout.size());
    }
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
    assert_eq!(heap.0.committed_bytes(), Some(0));
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
    // Process attach checks readiness before it can start the allocating bootstrap.
    let lib = include_str!("../lib.rs");
    let attach = &lib[lib.find("fn DllMain(").unwrap()..];
    let ready = attach.find("if !ALLOCATOR.ready()").unwrap();
    assert!(ready < attach.find("CreateThread(").unwrap());
    assert!(ready < attach.find("return 0;").unwrap());
}
