
use collector_certificates::watch_registry;
use collector_hooks::hook;
use collector_memory::read_accounting;
use collector_memory::terminal as allocation;
use collector_primitives::clock;
use collector_primitives::table_capture;
use collector_projection::table_conversion;
use collector_status::status;
#[cfg(test)]
use collector_test_support as _;
use collector_timing::hook_timing;

pub mod army_stat_changes;
pub mod attack_stats;
pub mod dig_in;
mod division_experience;
pub mod division_land;
pub mod observations;
pub mod supply_inputs;
pub mod unit_combat_references;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;

pub mod division_history;

pub mod panel_sources;

mod panel_operand_hooks;

#[cfg(test)]
mod allocation_denial_tests {
    #[test]
    fn a_denied_interval_belongs_to_its_thread() {
        use std::sync::{Arc, Barrier};
        let entered = Arc::new(Barrier::new(2));
        let left = Arc::new(Barrier::new(2));
        let other = {
            let (entered, left) = (Arc::clone(&entered), Arc::clone(&left));
            std::thread::spawn(move || {
                entered.wait();
                drop(collector_test_support::NullGuard::new(usize::MAX));
                left.wait();
            })
        };
        let denied = collector_test_support::with_allocations_denied(|| {
            entered.wait();
            left.wait();
            Vec::<u8>::new().try_reserve(64).is_err()
        });
        other.join().unwrap();
        assert!(denied, "another thread's guard ended this thread's denial");
        assert_eq!(
            collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
