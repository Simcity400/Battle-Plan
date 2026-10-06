//! Hourly game-thread sampling with an independent recoverable spool writer.

// Borrowed status objects expand one field per captured family.
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
mod allocation;
#[cfg(test)]
mod allocation_failure_tests;
#[cfg(feature = "callback_profile")]
mod callback_profile;
mod combat_log_prune;
mod construction_ic;
mod country_scale;
mod decode;
mod division_census;
mod division_detail;
mod equipment_destruction;
mod extended;
mod fixed_layout;
mod history;
mod history_changes;
mod hook;
mod line_output;
mod map_capture;
mod map_spool;
mod opinion_changes;
mod output_hooks;
mod output_log;
mod portrait_changes;
mod projection;
mod raw;
mod raw_memory;
mod raw_plan;
mod read_accounting;
mod relation_changes;
mod roster;
mod runtime;
mod sampling;
mod slicing;
mod snapshot_plan;
mod spooling;
mod status;
mod status_writer;
mod table_capture;
#[cfg(test)]
mod unit_scale_tests;
mod watch_registry;

/// How long a test waits on another thread before failing instead of hanging.
/// Only a hang guard, never a timing check: a full build saturates every core
/// and can starve a test thread for many seconds.
#[cfg(test)]
const TEST_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(120);

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
