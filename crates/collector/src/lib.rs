//! Hourly game-thread sampling with an independent recoverable spool writer.

// The decode-worker calibration snapshot grows one small object per moved
// family; keep the `json!` muncher comfortably below its expansion limit.
#![recursion_limit = "512"]

#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("collector requires Windows x64 MSVC");

mod decode;
mod extended;
mod history;
mod hook;
mod map_capture;
mod map_spool;
mod raw;
mod roster;
mod runtime;
mod sampling;
mod spooling;
mod table_capture;

use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HINSTANCE},
    System::Threading::CreateThread,
};

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> i32 {
    if reason == 1 {
        // Loader work is deferred until this thread runs outside the loader lock.
        let thread = unsafe {
            CreateThread(
                std::ptr::null(),
                0,
                Some(runtime::worker),
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
    1
}
