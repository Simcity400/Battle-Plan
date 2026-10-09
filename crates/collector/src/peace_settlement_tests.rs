use crate::peace_settlements::{Capacities, Storage};
#[path = "../../engine/tests/support/peace_world.rs"]
pub(crate) mod fixture;
use fixture::Fixture;

#[test]
fn owned_log_copies_multiple_conferences_and_retains_failures() {
    let mut f = Fixture::new(2, 3);
    assert_eq!(
        f.copy(
            &mut [0; 2],
            &mut [engine::peace_settlement::Action::default(); 3]
        )
        .actions,
        3
    );
    let mut storage = Storage::new(Capacities {
        events: 3,
        countries: 6,
        actions: 7,
        transfers: 1,
        name_bytes: 0,
    })
    .unwrap();
    let bytes = storage.allocated_bytes();
    let memory = engine::memory::SliceMemory {
        base: 0,
        bytes: &f.bytes,
    };
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 0, true));
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 1, true));
    assert_eq!(storage.events[1].snapshot.omitted, 0);
    assert_eq!(storage.events[0].snapshot.actions, 3);
    f.u64(0x20000, 99);
    assert!(storage.observe(
        &engine::memory::SliceMemory {
            base: 0,
            bytes: &f.bytes
        },
        0,
        0x100,
        Some(8),
        2,
        true
    ));
    assert_eq!(
        storage.events[2].snapshot.failure,
        Some("peace action vtable unsupported")
    );
    assert!(!storage.observe(
        &engine::memory::SliceMemory {
            base: 0,
            bytes: &f.bytes
        },
        0,
        0x100,
        Some(8),
        3,
        false
    ));
    assert_eq!(storage.allocated_bytes(), bytes);
    assert_eq!(storage.used[0], 3);
}

#[test]
fn bounded_storage_follows_design_and_stress_populations() {
    for count in [1500, 5000] {
        let mut f = Fixture::new(count, count);
        let pointer = f.rebase();
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let mut storage = Storage::new(Capacities::for_counts(count, count)).unwrap();
        assert!(storage.observe(&memory, 0, pointer, Some(91), 0, true));
        assert_eq!(storage.events[0].snapshot.failure, None);
        assert_eq!(storage.events[0].snapshot.omitted, 0);
        assert_eq!(storage.events[0].snapshot.actions, count);
    }
}

#[test]
fn small_world_work_capacity_follows_live_counts() {
    let capacity = Capacities::for_counts(50, 100);
    assert_eq!(capacity.countries, 200);
    assert_eq!(capacity.actions, 500);
    assert_eq!(capacity.transfers, 400);
}
