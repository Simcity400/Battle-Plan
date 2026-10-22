use crate::peace_settlements::{Capacities, Storage};
#[path = "../../engine/tests/support/peace_world.rs"]
pub mod fixture;
use fixture::Fixture;

#[test]
fn owned_log_copies_multiple_conferences_and_retains_failures() {
    let mut f = Fixture::new(2, 3);
    assert_eq!(
        f.copy(
            &mut [0; 2],
            &mut [engine_peace::peace_settlement::Action::default(); 3]
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
    let memory = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &f.bytes,
    };
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 0, true));
    assert!(storage.observe(&memory, 0, 0x100, Some(8), 1, true));
    assert_eq!(storage.events[1].snapshot.omitted, 0);
    assert_eq!(storage.events[0].snapshot.actions, 3);
    f.u64(0x20000, 99);
    assert!(storage.observe(
        &engine_native::memory::SliceMemory {
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
        &engine_native::memory::SliceMemory {
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
