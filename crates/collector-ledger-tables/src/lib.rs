//! Event store tables: the hourly packs that drain the event feed history and the air loss, building bombing and nuclear strike event ledgers (collector-observers) into owned rows. The host owns dispatch and table assembly.

use collector_lifetime::world_lifetime;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;

pub mod air_losses;
pub mod building_bombing;
pub mod event_feed;
pub mod market;
pub mod nuclear_strikes;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
