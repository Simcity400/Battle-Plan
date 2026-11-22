use collector_primitives::clock;
use collector_status::status;
#[cfg(test)]
use collector_test_support as test_support;

pub mod hook;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

pub fn located_hook(
    base: usize,
    key: engine_sites::Rva,
) -> Result<(usize, [u8; 16]), &'static str> {
    engine_sites::hook(base, key)
        .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
        .inspect_err(|cause| {
            hook::installation::refused(hook::installation::Kind::Sentinel, None, cause)
        })
}
