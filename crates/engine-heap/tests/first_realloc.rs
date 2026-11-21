
use engine_heap::Heap;
use std::alloc::{GlobalAlloc, Layout};

fn resized_on_a_new_thread(from: usize, to: usize) {
    let layout = Layout::from_size_align(from, 8).unwrap();
    let block = unsafe { Heap.alloc(layout) };
    assert!(!block.is_null());
    unsafe { block.write_bytes(0x5a, from) };
    let address = block as usize;
    let resized = std::thread::spawn(move || {
        let resized = unsafe { Heap.realloc(address as *mut u8, layout, to) };
        assert!(!resized.is_null());
        let kept = unsafe { std::slice::from_raw_parts(resized, from.min(to)) };
        assert!(kept.iter().all(|&byte| byte == 0x5a));
        resized as usize
    })
    .join()
    .unwrap();
    unsafe { Heap.dealloc(resized as *mut u8, Layout::from_size_align(to, 8).unwrap()) };
}

#[test]
fn a_new_threads_first_heap_call_may_be_a_realloc() {
    for (from, to) in [
        (100, 104),
        (100, 64),
        (100, 1000),
        (40 << 10, 48 << 10),
        (60 << 10, 200 << 10),
        (200 << 10, 210 << 10),
        (200 << 10, 1 << 20),
        (1 << 20, 32 << 10),
    ] {
        resized_on_a_new_thread(from, to);
    }
}
