//! Slow job country and state tables: country factories, country tactic unlocks, faction theatres, logistics availability, modifier definitions and state building inputs. The host owns the Slow job's dispatch, loans and table assembly.

use collector_extended_support::outcomes::failed_capture;
use collector_logistics::logistics_balance;
use collector_memory::read_accounting;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod country_factories;
pub mod country_tactic_unlocks;
pub mod doctrines;
pub mod faction_theatres;
pub mod logistics;
pub mod modifier_definitions;
pub mod state_building_inputs;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
