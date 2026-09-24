//! Hourly game-thread sampling with an independent recoverable spool writer.

// The decode-worker calibration snapshot grows one small object per moved
// family; keep the `json!` muncher comfortably below its expansion limit.
#![recursion_limit = "512"]

#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("collector requires Windows x64 MSVC");

mod acquisition;
mod acquisition_countries;
mod acquisition_factions;
mod acquisition_gather;
mod acquisition_jobs;
mod acquisition_states;
mod acquisition_wars;
mod advisor_changes;
#[cfg(feature = "callback_profile")]
mod callback_profile;
mod country_scale;
mod decode;
mod division_census;
mod division_detail;
mod extended;
mod history;
mod hook;
mod map_capture;
mod map_spool;
mod portrait_changes;
mod projection;
mod raw;
mod raw_memory;
mod raw_plan;
mod read_accounting;
mod roster;
mod runtime;
mod sampling;
mod snapshot_plan;
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
