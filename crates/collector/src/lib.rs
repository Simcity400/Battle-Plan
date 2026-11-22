#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("collector requires Windows x64 MSVC");

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

struct Allocator;

unsafe impl std::alloc::GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe { collector_workers::private_heap::PRIVATE_HEAP.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe { collector_workers::private_heap::PRIVATE_HEAP.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { collector_workers::private_heap::PRIVATE_HEAP.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        unsafe { collector_workers::private_heap::PRIVATE_HEAP.realloc(ptr, layout, size) }
    }
}

pub use collector_operation_outcomes::operation_outcomes::BATTLEPLAN_OPERATION_OUTCOME_CALLBACKS;

use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HINSTANCE},
    System::Threading::CreateThread,
};

const DLL_THREAD_DETACH: u32 = 3;

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, reserved: *mut c_void) -> i32 {
    if reason == 1 {
        if !collector_workers::private_heap::PRIVATE_HEAP.ready() {
            return 0;
        }
        let thread = unsafe {
            CreateThread(
                std::ptr::null(),
                0,
                Some(collector_runtime::runtime::worker),
                module,
                0,
                std::ptr::null_mut(),
            )
        };
        if thread.is_null() {
            return 0;
        }
        unsafe {
            CloseHandle(thread);
        }
    }
    if reason == DLL_THREAD_DETACH {
        // SAFETY: Windows is detaching the current thread after its collector work has ended.
        unsafe { collector_workers::private_heap::PRIVATE_HEAP.thread_exit() };
    }
    unsafe { collector_workers::private_heap::PRIVATE_HEAP.detach(reason, reserved) };
    1
}
