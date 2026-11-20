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
static TEST_THREAD_STACK: unsafe extern "C" fn() = crate::test_support::reserve_test_stack;

// Tests retain the allocation-denial allocator and exercise PrivateHeap directly.
#[cfg_attr(not(test), global_allocator)]
static ALLOCATOR: private_heap::PrivateHeap = private_heap::PrivateHeap::new();

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: crate::test_support::FailingAllocator =
    crate::test_support::FailingAllocator;

mod acquisition;
mod acquisition_countries;
mod acquisition_factions;
mod acquisition_gather;
mod acquisition_jobs;
mod acquisition_states;
mod acquisition_wars;
mod advisor_changes;
mod columnar;
mod crew;
mod faction_accounts;
mod faction_advisors;
mod faction_companion_changes;
mod faction_companions;
mod faction_goal_changes;
mod faction_goals;
mod faction_pools;
mod tick_gate;
mod capture_point;
mod command_batch;
pub(crate) use collector_observers::advisor_pool_changes;
pub(crate) use collector_observers::air_base_carry_changes;
mod air_base_changes;
pub(crate) use collector_air_records::air_record_changes;
pub(crate) use collector_observers::air_losses;
mod allocation;
#[cfg(test)]
mod allocation_failure_tests;
mod private_heap;
pub(crate) use collector_construction::building_changes;
pub(crate) use collector_division_parts::army_stat_changes;
pub(crate) use collector_observers::building_bombing;
#[cfg(feature = "callback_profile")]
mod callback_profile;
pub(crate) use collector_combat_log::combat_log_prune;
pub(crate) use collector_observers::census_changes;
pub(crate) use collector_observers::character_changes;
pub(crate) use collector_observers::country_input_changes;
pub(crate) use collector_output::construction_ic;
pub(crate) use collector_primitives::clock;
mod country_scale;
pub(crate) use collector_primitives::country_slots;
mod decode;
pub(crate) use collector_certificates::dirty_set;
mod division_census;
mod division_detail;
mod equipment_destruction;
pub(crate) use collector_experience_gains::experience_gains;
pub(crate) use collector_observers::event_feed_history;
pub(crate) use collector_observers::event_feed_observer;
#[cfg(test)]
use collector_observers::event_ledger;
mod extended;
mod frame_capacities;
mod frame_fence;
mod frame_pool;
pub(crate) use collector_intel::flag_changes;
pub(crate) use collector_metadata::fixed_layout;
#[cfg(all(test, feature = "tap-foreign-probe"))]
#[path = "../../../tools/tap-foreign-unwind/runtime.rs"]
mod foreign_unwind_probe;
pub(crate) use collector_fuel_awards::fuel_awards;
pub(crate) use collector_map_events as map_events;
mod history;
mod hour_stages;
mod hook_failures;
mod module_threads;
mod hook_inventory;
pub(crate) use collector_air_histories::history_changes;
pub(crate) use collector_hooks::hook;
#[cfg(test)]
mod hook_sites_tests;
pub(crate) use collector_timing::hook_timing;
#[cfg(test)]
mod land_census_scale;
mod land_events;
pub(crate) use collector_logistics::logistics_balance;
pub(crate) use collector_model_inputs::local_subunit_changes;
pub(crate) use collector_output::line_output;
mod air_detection_changes;
mod ground_crew_changes;
mod map_capture;
mod map_spool;
mod mission_changes;
mod naval_accidents;
mod naval_map_changes;
mod naval_mines;
mod naval_result_sources;
mod radar_changes;
pub(crate) use collector_observers::naval_tree_changes;
mod navy_ledger_changes;
pub(crate) use collector_observers::market_changes;
pub(crate) use collector_observers::nuclear_strikes;
mod operation_outcomes;
pub(crate) use collector_observers::opinion_changes;
pub(crate) use collector_observers::orders_group_changes;
pub(crate) use collector_observers::order_membership_changes;
mod orders_parts;
mod pack_conversion;
pub(crate) use collector_governance::party_changes;
pub(crate) use collector_governance::party_leader_changes;
#[cfg(test)]
use collector_observers::output_log;
pub(crate) use collector_observers::political_status_changes;
pub(crate) use collector_peace::peace_settlements;
pub(crate) use collector_politics::politics_changes;
mod player_commands;
mod portrait_changes;
mod progress;
pub(crate) use collector_observers::province_changes;
pub(crate) use collector_projection::projection;
mod province_unit_changes;
pub(crate) use collector_memory::read_accounting;
pub(crate) use collector_observers::raid_changes;
pub(crate) use collector_observers::rail_changes;
pub(crate) use collector_primitives::raw_memory;
pub(crate) use collector_raw::raw;
#[cfg(test)]
mod read_accounting_tests;
mod site_withholding;
mod worker_panic;
pub(crate) use collector_air_relations::relation_changes;
pub(crate) use collector_metadata::roster;
pub(crate) use collector_observers::retreat_ban_changes;
pub(crate) use collector_observers::roster_changes;
mod runtime;
mod sampling;
mod scripted_maps;
pub(crate) use collector_certificates::slicing;
pub(crate) use collector_primitives::shared_lane;
pub(crate) use collector_ship_models::ship_model_changes;
mod slow_hours;
pub(crate) use collector_snapshots::snapshot_plan;
#[cfg(test)]
mod snapshot_integration_tests;
mod spool_compare;
mod spool_helper;
mod spool_phases;
mod spool_segments;
mod spool_writer;
mod spooling;
pub(crate) use collector_status::status;
#[cfg(test)]
mod status_tests;
mod status_writer;
pub(crate) use collector_primitives::table_capture;
pub(crate) use collector_primitives::table_hash;
pub(crate) use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as test_support;
mod template_changes;
pub(crate) use collector_memory::terminal;
pub(crate) use collector_observers::territory_changes;
pub(crate) use collector_observers::province_combat_changes;
pub(crate) use collector_production::production::design_changes;
mod frame_priority;
mod thread_cpu;
pub(crate) use collector_observers::unit_leader_changes;
pub(crate) use collector_observers::unit_leader_trait_changes;
#[cfg(test)]
mod unit_scale_tests;
pub(crate) use collector_certificates::watch_registry;
pub(crate) use collector_intel::variable_changes;
pub(crate) use collector_observers::war_changes;
pub(crate) use collector_observers::wing_changes;
#[cfg(feature = "worker_sampler")]
mod worker_sampler;
pub(crate) use collector_lifetime::world_lifetime;
pub(crate) use collector_observers::world_input_changes;

/// How long a test waits on another thread before failing instead of hanging.
/// Only a hang guard, never a timing check: a full build saturates every core
/// and can starve a test thread for many seconds.
#[cfg(test)]
const TEST_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(120);

/// The machine's performance-counter frequency, so a test's walls read in real ns.
#[cfg(test)]
fn test_qpc_frequency() -> u64 {
    let mut frequency = 0;
    // SAFETY: a valid local output.
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) };
    frequency.max(1) as u64
}

/// Runs a threaded test's `scenario` on a thread of its own while the test thread watches it.
/// Every step it reports restarts [`TEST_HANG_GUARD`]; a scenario silent that long fails the
/// test with its last step and `state()`, wherever it is blocked, and is left behind. Its own
/// panic fails the test as itself.
#[cfg(test)]
fn watched(state: impl Fn() -> String, scenario: impl FnOnce(&Steps) + Send + 'static) {
    use std::sync::mpsc::RecvTimeoutError;
    let (steps, seen) = std::sync::mpsc::channel();
    let scenario = std::thread::spawn(move || scenario(&Steps(steps)));
    let mut last = String::from("starting");
    loop {
        match seen.recv_timeout(TEST_HANG_GUARD) {
            Ok(step) => last = step,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!("blocked for {TEST_HANG_GUARD:?} at: {last}; {}", state())
            }
        }
    }
    // Its steps are dropped only once the scenario returned or unwound: this join waits for
    // nothing but the thread's exit.
    if let Err(panic) = scenario.join() {
        std::panic::resume_unwind(panic);
    }
}

/// A test's thread that says when it is done, so the test joins it only once it has finished.
#[cfg(test)]
struct Ending<T> {
    thread: std::thread::JoinHandle<()>,
    done: std::sync::mpsc::Receiver<T>,
}

#[cfg(test)]
fn spawn_ending<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> Ending<T> {
    let (send, done) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = send.send(run());
    });
    Ending { thread, done }
}

#[cfg(test)]
impl<T> Ending<T> {
    fn thread(&self) -> &std::thread::Thread {
        self.thread.thread()
    }

    fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// What it returned once it finishes. A thread still running at [`TEST_HANG_GUARD`] fails
    /// the test, named by `what`, and is left behind; its own panic fails the test as itself.
    fn end(self, what: impl FnOnce() -> String) -> T {
        use std::sync::mpsc::RecvTimeoutError;
        match self.done.recv_timeout(TEST_HANG_GUARD) {
            // Its result is its last act: this join waits only for the thread's exit.
            Ok(result) => {
                let _ = self.thread.join();
                result
            }
            Err(RecvTimeoutError::Disconnected) => match self.thread.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => unreachable!("a thread that returned sent its result"),
            },
            Err(RecvTimeoutError::Timeout) => {
                panic!("{} did not end within {TEST_HANG_GUARD:?}", what())
            }
        }
    }
}

/// Where a [`watched`] scenario is.
#[cfg(test)]
struct Steps(std::sync::mpsc::Sender<String>);

#[cfg(test)]
impl Steps {
    fn at(&self, step: impl std::fmt::Display) {
        let _ = self.0.send(step.to_string());
    }
}

use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HINSTANCE},
    System::Threading::CreateThread,
};

const DLL_THREAD_DETACH: u32 = 3;

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, reserved: *mut c_void) -> i32 {
    if reason == 1 {
        // Every Rust allocation, including the bootstrap's infallible ones, uses
        // the private heap: refuse the load, allocation-free, if it cannot exist.
        if !ALLOCATOR.ready() {
            return 0;
        }
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
    if reason == DLL_THREAD_DETACH {
        ALLOCATOR.thread_exit();
    }
    // FreeLibrary is reached only after runtime's joined/quiescent teardown.
    // Windows has already run this thread's Rust TLS callbacks. Rust statics
    // have no automatic destructors; the remaining CRT cleanup owns no Rust
    // blocks. Process termination deliberately leaves reclamation to Windows.
    unsafe { ALLOCATOR.detach(reason, reserved) };
    1
}

mod division_panel_forwarder;
pub(crate) use collector_war_overview::participation as war_participation;

pub(crate) use collector_intel::{
    network_auxiliary, propaganda_changes, source_changes as intel_source_changes,
};
pub(crate) use collector_naval_management::changes as naval_management_changes;

/// A hook site's live address and the 16 bytes its hook replaces. A withheld site is refused
/// the way a differing sentinel was, by the withheld text that names it.
pub(crate) fn located_hook(
    base: usize,
    key: engine_sites::Rva,
) -> Result<(usize, [u8; 16]), &'static str> {
    engine_sites::hook(base, key)
        .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
        .inspect_err(|cause| {
            hook::installation::refused(hook::installation::Kind::Sentinel, None, cause)
        })
}
