//! Per-division input parts: attack stats and the army stat writer observer, dig-in factors, land inputs, supply inputs and unit combat references, which division presentation, railway guns and naval counters read. The host owns the presentation jobs, loans and table assembly.

use collector_certificates::watch_registry;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod army_stat_changes;
pub mod attack_stats;
pub mod dig_in;
mod division_experience;
pub mod division_land;
pub mod supply_inputs;
pub mod unit_combat_references;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

pub mod division_history;

pub mod panel_sources;

mod panel_operand_hooks;
