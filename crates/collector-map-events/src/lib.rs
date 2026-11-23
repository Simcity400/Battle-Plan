
use collector_hooks::hook;
use collector_lifetime::world_lifetime;
use collector_primitives::shared_lane;
use collector_status::status;
use collector_timing::hook_timing;

pub mod hooks;
pub mod interval;
pub mod lanes;

pub fn largest_event(countries: usize) -> usize {
    lanes::HEADER + 17 + countries.saturating_mul(5)
}

/// # Safety
/// Run only during attach or the active session's serial sizing pass, while map-event lanes remain
/// live and teardown (`lanes::release`) is excluded.
pub unsafe fn presize(countries: usize) -> Result<bool, &'static str> {
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
