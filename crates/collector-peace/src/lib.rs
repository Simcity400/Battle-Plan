//! Peace capture: the peace settlement observer, its hourly table pack (`tables`) and the peace bid pack. The host installs the hooks, drains at the hourly boundary and assembles the tables.

use collector_extended_support::outcomes::elapsed;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_primitives::clock::qpc;
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;
use collector_timing::hook_timing;

pub mod peace_bids;
#[cfg(test)]
mod peace_settlement_tests;
pub mod peace_settlements;
pub mod tables;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
