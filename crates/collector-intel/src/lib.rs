//! Intel capture: country intel, agencies, operations and operatives, script flags and variables, the shared daily CountryIntel buffer and the flag and variable writer observers. The host owns dispatch, loans, the spool conversion caches and table assembly.

use collector_certificates::sampled_verify;
use collector_certificates::slicing;
use collector_certificates::watch_registry;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_metadata::roster;
use collector_observers::country_input_changes;
use collector_observers::opinion_changes;
use collector_observers::roster_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;
use collector_timing::hook_timing;

pub mod daily_intel;
pub mod flag_changes;
pub mod intel;
pub mod variable_changes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
