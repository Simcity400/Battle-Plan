use crate::peace_settlements::{Capacities, Storage};
#[path = "../../engine/tests/native/support/peace_world.rs"]
pub mod fixture;
use fixture::Fixture;

#[test]
fn owned_log_copies_multiple_conferences_and_retains_failures() {
    let mut f = Fixture::new(2, 3);
    assert_eq!(
        f.copy(
            engine_native_memory::walk::Buffer::Growing(&mut vec![0; 2]),
            engine_native_memory::walk::Buffer::Growing(&mut vec![
                engine_peace::peace_settlement::Action::default();
                3
            ])
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
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &f.bytes,
    };
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 0, true));
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 1, true));
    assert_eq!(storage.events[1].snapshot.omitted, 0);
    assert_eq!(storage.events[0].snapshot.actions, 3);
    f.u64(0x20000, 99);
    assert!(storage.observe(
        &engine_native_memory::memory::SliceMemory {
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
    assert!(storage.allocated_bytes() > bytes);
    assert!(storage.observe(
        &engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &f.bytes
        },
        0,
        0x100,
        Some(8),
        3,
        true
    ));
    assert_eq!(storage.used[0], 4);
    assert_eq!(storage.events[3].sequence, 3);
}

#[test]
fn operands_past_the_prepared_arenas_are_kept_whole() {
    let f = Fixture::new(2, 3);
    let mut storage = Storage::new(Capacities {
        events: 1,
        countries: 1,
        actions: 1,
        transfers: 0,
        name_bytes: 0,
    })
    .unwrap();
    assert!(storage.observe(&f.memory(), 0, 0x100, Some(8), 0, true));
    let snapshot = storage.events[0].snapshot;
    assert_eq!(snapshot.failure, None);
    assert_eq!(snapshot.omitted, 0);
    assert_eq!((snapshot.winners, snapshot.actions), (2, 3));
    assert_eq!(&storage.winners[..2], &[0xa0000000, 0xa0000001]);
    assert_eq!(storage.used[1], 2);
    assert_eq!(storage.used[3], 3);
}

#[test]
fn a_header_nothing_backs_is_refused_before_any_growth() {
    let mut f = Fixture::new(1, 0);
    f.array(
        0x100 + engine_peace::peace_conference::WINNERS,
        0x20000,
        i32::MAX as usize,
        4,
    );
    let mut storage = Storage::new(Capacities {
        events: 1,
        countries: 1,
        ..Capacities::default()
    })
    .unwrap();
    let bytes = storage.allocated_bytes();
    assert!(storage.observe(&f.memory(), 0, 0x100, Some(8), 0, true));
    assert_eq!(
        storage.events[0].snapshot.failure,
        Some("peace settlement array element unreadable")
    );
    assert_eq!(storage.allocated_bytes(), bytes);
}
