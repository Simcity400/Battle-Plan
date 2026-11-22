#![recursion_limit = "512"]

#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("collector requires Windows x64 MSVC");

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = crate::test_support::reserve_test_stack;

#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[cfg(not(test))]
struct Allocator;

#[cfg(not(test))]
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

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: crate::test_support::FailingAllocator =
    crate::test_support::FailingAllocator;

mod acquisition_countries;
mod acquisition_factions;
mod acquisition_jobs;
mod capture_point;
mod columnar;
mod command_batch;
mod tick_gate;
pub(crate) use collector_air_records::air_record_changes;
pub(crate) use collector_observers::advisor_pool_changes;
pub(crate) use collector_observers::air_base_carry_changes;
pub(crate) use collector_observers::air_losses;
mod allocation;
#[cfg(test)]
mod allocation_failure_tests;
pub(crate) use collector_construction::building_changes;
pub(crate) use collector_division_parts::army_stat_changes;
pub(crate) use collector_observers::building_bombing;
#[cfg(feature = "callback_profile")]
mod callback_profile;
pub(crate) use collector_combat_log::combat_log_prune;
pub(crate) use collector_observers::census_changes;
pub(crate) use collector_observers::combat_engagement_refreshes;
pub(crate) use collector_observers::character_changes;
pub(crate) use collector_observers::country_input_changes;
pub(crate) use collector_output::construction_ic;
pub(crate) use collector_primitives::clock;
mod country_scale;
pub(crate) use collector_primitives::country_slots;
mod decode;
mod division_detail;
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
pub(crate) use collector_fuel_awards::fuel_awards;
pub(crate) use collector_map_events as map_events;
mod history;
mod hook_failures;
mod module_threads;
pub(crate) use collector_air_histories::history_changes;
pub(crate) use collector_hooks::hook;
#[cfg(test)]
mod hook_sites_tests;
pub(crate) use collector_timing::hook_timing;
#[cfg(test)]
mod land_census_scale;
pub(crate) use collector_logistics::logistics_balance;
pub(crate) use collector_model_inputs::local_subunit_changes;
pub(crate) use collector_output::line_output;
mod map_capture;
mod map_spool;
pub(crate) use collector_observers::market_changes;
pub(crate) use collector_observers::naval_tree_changes;
pub(crate) use collector_observers::nuclear_strikes;
pub(crate) use collector_observers::opinion_changes;
pub(crate) use collector_observers::order_membership_changes;
pub(crate) use collector_observers::orders_group_changes;
mod pack_conversion;
pub(crate) use collector_governance::party_changes;
pub(crate) use collector_governance::party_leader_changes;
#[cfg(test)]
use collector_observers::output_log;
pub(crate) use collector_observers::political_status_changes;
pub(crate) use collector_peace::peace_settlements;
pub(crate) use collector_politics::politics_changes;
mod player_commands;
mod progress;
pub(crate) use collector_memory::read_accounting;
pub(crate) use collector_observers::province_changes;
pub(crate) use collector_observers::raid_changes;
pub(crate) use collector_observers::rail_changes;
pub(crate) use collector_primitives::raw_memory;
pub(crate) use collector_projection::projection;
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
pub(crate) use collector_certificates::slicing;
pub(crate) use collector_primitives::shared_lane;
pub(crate) use collector_ship_models::ship_model_changes;
mod slow_hours;
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
mod status_file;
mod status_writer;
pub(crate) use collector_memory::terminal;
pub(crate) use collector_observers::province_combat_changes;
pub(crate) use collector_observers::territory_changes;
pub(crate) use collector_primitives::table_capture;
pub(crate) use collector_primitives::table_hash;
pub(crate) use collector_production::production::design_changes;
pub(crate) use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as test_support;
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

#[cfg(test)]
fn test_qpc_frequency() -> u64 {
    let mut frequency = 0;
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) };
    frequency.max(1) as u64
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
        collector_workers::private_heap::PRIVATE_HEAP.thread_exit();
    }
    unsafe { collector_workers::private_heap::PRIVATE_HEAP.detach(reason, reserved) };
    1
}

mod division_panel_forwarder;
pub(crate) use collector_war_overview::participation as war_participation;

pub(crate) use collector_intel::{
    network_auxiliary, propaganda_changes, source_changes as intel_source_changes,
};
pub(crate) use collector_naval_management::changes as naval_management_changes;

#[cfg(test)]
mod air_registration_contract_tests;

#[cfg(test)]
mod air_map_capture_tests;

#[cfg(test)]
mod air_carrier_retirement_tests;

#[cfg(test)]
mod state_flags_publication_tests;

#[cfg(test)]
mod states_worker_growth_tests;

#[cfg(test)]
mod template_worker_tests;

#[cfg(test)]
mod operation_retry_tests;

#[cfg(test)]
mod land_detach_tests;

#[cfg(test)]
mod advisor_sizing_tests;

#[cfg(test)]
mod worker_job_tests;
