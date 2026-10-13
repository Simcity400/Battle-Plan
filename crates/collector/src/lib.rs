//! Hourly game-thread sampling with an independent recoverable spool writer.

// Borrowed status objects expand one field per captured family.
#![recursion_limit = "512"]

#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("collector requires Windows x64 MSVC");

/// The test profile builds collector unoptimised, where frames are large: `sampling::acquire`
/// alone needs 8 MiB and the spool's status writer almost 3 MiB, beyond the 2 MiB that test and
/// spawned threads get by default. Before `main`, this test binary gives threads without an
/// explicit size 32 MiB of reserved stack unless `RUST_MIN_STACK` is already set. Child processes
/// inherit the variable; the DLL and its explicit worker stacks are unaffected.
#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = {
    unsafe extern "C" fn reserve() {
        if std::env::var_os("RUST_MIN_STACK").is_none() {
            // SAFETY: CRT initialisers run before main, while this is the only thread.
            unsafe { std::env::set_var("RUST_MIN_STACK", "33554432") };
        }
    }
    reserve
};

mod acquisition;
mod acquisition_countries;
mod acquisition_factions;
mod acquisition_gather;
mod acquisition_jobs;
mod acquisition_states;
mod acquisition_wars;
mod advisor_changes;
mod advisor_pool_changes;
mod air_base_carry_changes;
mod air_base_changes;
mod air_losses;
mod air_record_changes;
mod allocation;
#[cfg(test)]
mod allocation_failure_tests;
mod area_changes;
mod army_stat_changes;
mod building_bombing;
mod building_changes;
#[cfg(feature = "callback_profile")]
mod callback_profile;
mod census_changes;
mod character_changes;
mod combat_log_prune;
mod construction_ic;
mod country_input_changes;
mod country_scale;
mod decode;
mod dirty_set;
mod division_census;
mod division_detail;
mod equipment_destruction;
mod event_feed_history;
mod event_feed_observer;
mod event_ledger;
mod experience_gains;
mod extended;
mod fixed_layout;
mod flag_changes;
#[cfg(all(test, feature = "tap-foreign-probe"))]
#[path = "../../../tools/tap-foreign-unwind/runtime.rs"]
mod foreign_unwind_probe;
mod fuel_awards;
mod history;
mod history_changes;
mod hook;
#[cfg(test)]
mod hook_sites_tests;
mod hook_timing;
#[cfg(test)]
mod land_census_scale;
mod land_events;
mod line_output;
mod local_subunit_changes;
mod logistics_balance;
mod map_capture;
mod map_spool;
mod mission_changes;
mod naval_result_sources;
mod naval_tree_changes;
mod navy_ledger_changes;
mod nuclear_strikes;
mod operation_outcomes;
mod opinion_changes;
mod orders_parts;
mod output_hooks;
mod output_log;
mod party_changes;
mod party_leader_changes;
#[cfg(test)]
mod peace_settlement_tests;
mod peace_settlements;
mod political_status_changes;
mod politics_changes;
mod portrait_changes;
mod progress;
mod projection;
mod province_changes;
mod province_unit_changes;
mod raid_changes;
mod rail_changes;
mod raw;
mod raw_memory;
mod raw_plan;
mod read_accounting;
mod relation_changes;
mod retreat_ban_changes;
mod roster;
mod roster_changes;
mod runtime;
mod sampled_verify;
mod sampling;
mod shared_lane;
mod ship_model_changes;
mod slicing;
mod slow_hours;
mod snapshot_plan;
mod spool_compare;
mod spool_helper;
mod spool_phases;
mod spool_segments;
mod spooling;
mod status;
mod status_writer;
mod table_capture;
mod table_hash;
mod template_changes;
mod territory_changes;
mod thread_cpu;
mod unit_leader_changes;
mod unit_leader_trait_changes;
#[cfg(test)]
mod unit_scale_tests;
mod variable_changes;
mod war_changes;
mod war_provenance;
mod watch_registry;
mod wing_changes;
mod world_input_changes;
mod world_lifetime;

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
