
use collector_certificates::dirty_set;
use collector_certificates::watch_registry;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_observers::raid_changes;
use collector_observers::rail_changes;
use collector_observers::territory_changes;
use collector_primitives::clock;
use collector_primitives::country_slots as country_scale;
use collector_primitives::raw_memory;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_raw::raw;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod building_changes;
pub mod construction;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

fn located(base: usize, key: engine_sites::Rva) -> Result<(), &'static str> {
    engine_sites::hook(base, key)
        .and_then(|code| code.sentinel::<16>().map(drop))
        .inspect_err(|cause| {
            hook::installation::refused(hook::installation::Kind::Sentinel, None, cause);
        })
}
