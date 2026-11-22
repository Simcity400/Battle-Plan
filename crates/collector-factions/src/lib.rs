#![recursion_limit = "512"]

pub mod faction_accounts;
pub mod faction_advisors;
pub mod faction_companion_changes;
pub mod faction_companions;
pub mod faction_goal_changes;
pub mod faction_goals;
pub mod faction_pools;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
