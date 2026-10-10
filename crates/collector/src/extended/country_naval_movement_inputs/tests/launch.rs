use super::*;
use std::cell::Cell;

const ZERO_NAVY: usize = 0x18000;
const LAUNCH_HEADER: usize = 0x18;
const LAUNCH_POINTERS: usize = 0x17200;

fn launch_world() -> TestMemory {
    let mut memory = world();
    pointer(&mut memory.bytes, 0xa100, ZERO_NAVY);
    pointer(&mut memory.bytes, ZERO_NAVY, cstrategic_navy::VTABLE_RVA);
    pointer(
        &mut memory.bytes,
        ZERO_NAVY + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA,
    );
    pointer(&mut memory.bytes, ZERO_NAVY + native::MANAGER, MANAGER);
    word(
        &mut memory.bytes,
        ZERO_NAVY + cnaval_base::HOLDER_COUNTRY,
        999,
    );
    header(
        &mut memory.bytes,
        ZERO_NAVY + LAUNCH_HEADER,
        LAUNCH_POINTERS,
        3,
    );
    for (object, id) in [(0x17080, (-1_i32) as u32), (0x17100, 0)] {
        pointer(&mut memory.bytes, object, cnaval_base::VTABLE_RVA);
        pointer(&mut memory.bytes, object + cnaval_base::HOLDER, ZERO_NAVY);
        word(&mut memory.bytes, object + cnaval_base::PROVINCE_ID, id);
    }
    for (ordinal, object) in [0x17080, 0x17100, 0x17080].into_iter().enumerate() {
        pointer(&mut memory.bytes, LAUNCH_POINTERS + ordinal * 8, object);
    }
    memory
}

fn words(shared: &Shared) -> Option<Vec<(i32, u32)>> {
    match shared
        .projection_launch(Some(vec![9, 3, 9]))
        .unwrap()
        .unwrap()
        .naval_bases
    {
        replay::rows::LaunchNavalBases::Bases(rows) => Some(
            rows.into_iter()
                .map(|row| (row.base_province as i32, row.country))
                .collect(),
        ),
        replay::rows::LaunchNavalBases::Unavailable => None,
        replay::rows::LaunchNavalBases::NativeNull => panic!("expected a list or read refusal"),
    }
}

#[test]
fn no_moving_owners_or_failed_owner_arenas_cannot_suppress_launch_zero() {
    for scenario in 0..3 {
        let memory = launch_world();
        let (mut capacity, mut shared_capacity) = capacity();
        shared_capacity.launch_bases = 3;
        if scenario == 2 {
            capacity = Capacities::default();
        }
        let mut captured = Captured::new(capacity);
        let mut shared = Shared::new(shared_capacity);
        let owners = if scenario == 0 {
            vec![]
        } else {
            vec![Ok(owner(&memory))]
        };
        captured.capture(
            &memory,
            0,
            STATE,
            Some(71),
            OwnerCensus {
                owners,
                failure: (scenario == 1).then_some("test owner census refusal"),
            },
            &mut shared,
        );
        assert_eq!(words(&shared), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
        assert_eq!(
            shared
                .projection_launch(Some(vec![9, 3, 9]))
                .unwrap()
                .unwrap()
                .owned_states,
            Some(vec![9, 3, 9])
        );
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == 0xa100)
                .count(),
            1
        );
    }
}

#[test]
fn launch_fault_is_independent_of_held_movement_headers_and_theatre_truth() {
    let mut memory = launch_world();
    memory.fault = Some(ZERO_NAVY + LAUNCH_HEADER);
    let (capacity, mut shared_capacity) = capacity();
    shared_capacity.launch_bases = 3;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
    );
    assert!(words(&shared).is_none());
    let movement = shared.projection().unwrap().unwrap();
    assert!(
        movement
            .nearest_caches
            .unwrap()
            .iter()
            .all(|row| row.nearest_bases.is_some())
    );
    assert_eq!(movement.reserved_theatre.unwrap().provinces, vec![3]);
    assert!(captured.for_country(1).unwrap().0.is_some());
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0xa100)
            .count(),
        1,
        "foreign movement reuses the already failed launch observation"
    );
}

#[test]
fn shared_identity_failure_cannot_retry_when_slot_zero_is_later_foreign() {
    struct FailOnce<'a> {
        memory: &'a TestMemory,
        failed: Cell<bool>,
    }
    impl Memory for FailOnce<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == ZERO_NAVY && !self.failed.replace(true) {
                self.memory.calls.borrow_mut().push((address, output.len()));
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    let memory = launch_world();
    let failing = FailOnce {
        memory: &memory,
        failed: Cell::new(false),
    };
    let (capacity, mut shared_capacity) = capacity();
    shared_capacity.launch_bases = 3;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &failing,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
    );
    assert!(words(&shared).is_none());
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == ZERO_NAVY)
            .count(),
        1,
        "a second identity read would recover and hide the first refusal"
    );
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0xa100)
            .count(),
        1
    );
    assert!(
        shared
            .projection()
            .unwrap()
            .unwrap()
            .nearest_caches
            .is_none()
    );
    assert!(
        captured.navies.first[..captured.navies.len]
            .iter()
            .any(|row| row.slot == 0 && row.identity_reason.is_some())
    );
}

#[test]
fn short_movement_rows_still_transfer_the_fixed_slot_zero_attempt_once() {
    let memory = launch_world();
    let (mut capacity, mut shared_capacity) = capacity();
    // Own slot1 fits and exposes its foreign list, but slot0's movement row does not.
    capacity.navies = 1;
    shared_capacity.launch_bases = 3;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
    );
    assert_eq!(words(&shared), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0xa100)
            .count(),
        1
    );
    assert_eq!(shared.launch_bases.required, 3);
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
}

#[test]
fn fixed_launch_capacity_grows_both_arenas_without_body_reads_or_hot_growth() {
    fn capture_fixed(memory: &TestMemory, shared: &mut Shared, hour: i32) {
        let reader = native::Reader::new(memory, 0, STATE);
        let mut observed = reader.observe_navy(0).unwrap();
        let captured = observed.capture_launch(
            memory,
            0,
            &mut shared.launch_scratch.first,
            &mut shared.launch_bases.first,
        );
        shared.sample_hour = Some(hour);
        shared.launch_bases.required = captured.required;
        shared.launch_scratch.required = captured.required;
        shared.launch_result = captured.provider;
        shared.launch_bases.len = match captured.provider {
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => len,
            _ => 0,
        };
    }
    let memory = launch_world();
    let (_, mut shared_capacity) = capacity();
    let mut shared = Shared::new(shared_capacity);
    capture_fixed(&memory, &mut shared, 71);
    assert!(words(&shared).is_none());
    assert_eq!(shared.required().launch_bases, 3);
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| at == LAUNCH_POINTERS || at == 0x17080)
    );
    shared.grow(&mut shared_capacity);
    assert!(shared.launch_bases.first.len() >= 3 && shared.launch_scratch.first.len() >= 3);
    let allocated = shared.allocated_bytes();
    memory.calls.borrow_mut().clear();
    capture_fixed(&memory, &mut shared, 72);
    assert_eq!(shared.allocated_bytes(), allocated);
    assert_eq!(words(&shared), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    shared.reset();
    assert!(shared.projection_launch(None).unwrap().is_none());
}

#[test]
fn worker_launch_grows_empty_arenas_before_first_publication() {
    let memory = launch_world();
    let mut captured = Captured::new(Capacities::default());
    let mut shared = Shared::new(SharedCapacities::default());
    assert_eq!(shared.allocated_bytes(), 0);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [],
            failure: None,
        },
        &mut shared,
    );
    assert_eq!(words(&shared), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    assert_eq!(shared.required().launch_bases, 3);
    assert!(shared.launch_bases.first.len() >= 3 && shared.launch_scratch.first.len() >= 3);
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == ZERO_NAVY + LAUNCH_HEADER)
            .count(),
        1,
        "growth uses the original launch header"
    );
}

#[test]
fn native_null_projection_is_fresh_and_distinct_from_lists_and_read_faults() {
    use replay::rows::LaunchNavalBases;

    let mut memory = launch_world();
    let (capacity, mut shared_capacity) = capacity();
    shared_capacity.launch_bases = 3;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let mut work = Work::default();
    for hour in 71..=74 {
        if hour == 72 {
            pointer(&mut memory.bytes, 0xa100, 0);
        } else if hour == 73 {
            memory.fault = Some(0xa100);
        } else if hour == 74 {
            memory.fault = None;
            pointer(&mut memory.bytes, 0xa100, ZERO_NAVY);
            header(&mut memory.bytes, ZERO_NAVY + LAUNCH_HEADER, 0, 0);
        }
        memory.calls.borrow_mut().clear();
        captured.capture(
            &memory,
            0,
            STATE,
            Some(hour),
            OwnerCensus {
                owners: [],
                failure: None,
            },
            &mut shared,
        );
        let packet = shared
            .projection_launch(Some(vec![9, 3, 9]))
            .unwrap()
            .unwrap();
        assert_eq!(packet.sample_hour, hour);
        assert_eq!(packet.owned_states, Some(vec![9, 3, 9]));
        match hour {
            71 => assert!(
                matches!(packet.naval_bases, LaunchNavalBases::Bases(bases) if bases.len() == 3)
            ),
            72 => {
                assert_eq!(packet.naval_bases, LaunchNavalBases::NativeNull);
                assert_eq!(shared.required().launch_bases, 0);
                assert_eq!(shared.launch_bases.len, 0);
                assert!(
                    !memory
                        .calls
                        .borrow()
                        .iter()
                        .any(|&(at, _)| at == ZERO_NAVY + LAUNCH_HEADER)
                );
            }
            73 => assert_eq!(packet.naval_bases, LaunchNavalBases::Unavailable),
            74 => assert_eq!(packet.naval_bases, LaunchNavalBases::Bases(vec![])),
            _ => unreachable!(),
        }
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == 0xa100)
                .count(),
            1
        );
        work.observe(&captured, &shared).unwrap();
    }
    assert_eq!(work.launch_available, 3);
    assert_eq!(work.launch_native_null, 1);
    assert_eq!(work.launch_refused, 1);
    shared.reset();
    assert!(shared.projection_launch(None).unwrap().is_none());
}
