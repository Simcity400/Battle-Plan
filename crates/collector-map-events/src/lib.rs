//! Map event capture: the seven simulation edges behind the game's floating
//! map text (`hooks`), their per-thread publication lanes (`lanes`) and the
//! hourly interval that becomes the MapEvents table (`interval`). The host
//! installs the hooks in the shared attach round, seals each hourly cut,
//! assembles and commits the interval on the spool thread and grows the
//! lanes on the sizing thread.

use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_primitives::shared_lane;
use collector_status::status;
use collector_timing::hook_timing;

pub mod hooks;
pub mod interval;
pub mod lanes;

/// The largest event a country population can produce: a unit's intel
/// array holds at most one entry per country.
pub fn largest_event(countries: usize) -> usize {
    lanes::HEADER + 17 + countries.saturating_mul(5)
}

/// Sizing thread: lanes for every hardware thread, each holding several of
/// the largest events the live country population allows, raised by any
/// demand the lanes refused.
pub fn presize(countries: usize) -> Result<bool, &'static str> {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    lanes::grow(threads, largest_event(countries).saturating_mul(8))
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

#[cfg(test)]
mod tests;
