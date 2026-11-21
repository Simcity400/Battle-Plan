
use collector_certificates::watch_registry;
use collector_extended_support::outcomes::failed_capture;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod local_subunit_changes;
pub mod model_inputs;

fn located(base: usize, key: engine_sites::Rva) -> Result<(), &'static str> {
    engine_sites::hook(base, key)
        .and_then(|code| code.sentinel::<16>().map(drop))
        .inspect_err(|cause| {
            hook::installation::refused(hook::installation::Kind::Sentinel, None, cause);
        })
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
