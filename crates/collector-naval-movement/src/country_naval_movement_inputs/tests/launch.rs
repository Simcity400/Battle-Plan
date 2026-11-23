use super::*;
use std::cell::Cell;

const ZERO_NAVY: usize = 0x18000;
const LAUNCH_HEADER: usize = 0x18;
const LAUNCH_POINTERS: usize = 0x17200;

fn launch_world() -> TestMemory {
    let mut memory = world();
    pointer(&mut memory.bytes, 0xa100, ZERO_NAVY);
    pointer(&mut memory.bytes, ZERO_NAVY, cstrategic_navy::VTABLE_RVA.0);
    pointer(
        &mut memory.bytes,
        ZERO_NAVY + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA.0,
    );
    pointer(&mut memory.bytes, ZERO_NAVY + native::MANAGER, MANAGER);
    word(
        &mut memory.bytes,
        ZERO_NAVY + engine_native_parts::cnaval_base_layout::HOLDER_COUNTRY,
        999,
    );
    header(
        &mut memory.bytes,
        ZERO_NAVY + LAUNCH_HEADER,
        LAUNCH_POINTERS,
        3,
    );
    for (object, id) in [(0x17080, (-1_i32) as u32), (0x17100, 0)] {
        pointer(&mut memory.bytes, object, engine_native_parts::cnaval_base_layout::VTABLE_RVA.0);
        pointer(&mut memory.bytes, object + engine_native_parts::cnaval_base_layout::HOLDER, ZERO_NAVY);
        word(&mut memory.bytes, object + engine_native_parts::cnaval_base_layout::PROVINCE_ID, id);
    }
    for (ordinal, object) in [0x17080, 0x17100, 0x17080].into_iter().enumerate() {
        pointer(&mut memory.bytes, LAUNCH_POINTERS + ordinal * 8, object);
    }
    memory
}

fn words(launch: &Launch) -> Option<Vec<(i32, u32)>> {
    match launch
        .projection_launch(Some(vec![9, 3, 9]))
        .unwrap()
        .unwrap()
        .naval_bases
    {
        replay_data::rows::LaunchNavalBases::Bases(rows) => Some(
            rows.into_iter()
                .map(|row| (row.base_province as i32, row.country))
                .collect(),
        ),
        replay_data::rows::LaunchNavalBases::Unavailable => None,
        replay_data::rows::LaunchNavalBases::NativeNull => {
            panic!("expected a list or read refusal")
        }
    }
}

fn launch_capacity() -> (Capacities, LaunchCapacities) {
    let (capacity, mut launch_capacity) = capacity();
    launch_capacity.launch_bases = 3;
    (capacity, launch_capacity)
}

#[test]
fn unreadable_countries_short_physical_arenas_or_area_faults_cannot_suppress_launch_zero() {
    for scenario in 0..4 {
        let mut memory = launch_world();
        let (mut capacity, launch_capacity) = launch_capacity();
        match scenario {
            1 => {
                for table in [cgame_state::COUNTRIES, cgame_state::COUNTRY_LINKS] {
                    pointer(&mut memory.bytes, STATE + table, usize::MAX);
                }
            }
            2 => capacity = Capacities::default(),
            3 => memory.fault = Some(PROVINCES[0]),
            _ => {}
        }
        let mut captured = Captured::new(capacity);
        let mut launch = Launch::new(launch_capacity);
        pass(&mut captured, &mut launch, &memory, Some(71));
        assert_eq!(words(&launch), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
        assert_eq!(
            launch
                .projection_launch(Some(vec![9, 3, 9]))
                .unwrap()
                .unwrap()
                .owned_states,
            Some(vec![9, 3, 9])
        );
        assert_eq!(launch.launch_sample_hour(), Some(71));
        assert_eq!(memory.count(0xa100), 1);
    }
}

#[test]
fn launch_fault_is_independent_of_the_physical_pass() {
    let mut memory = launch_world();
    memory.fault = Some(ZERO_NAVY + LAUNCH_HEADER);
    let (capacity, launch_capacity) = launch_capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(71), &mut launch);
    assert!(words(&launch).is_none());
    assert_eq!(
        captured.borrow_navy(0).unwrap().ledger_navy(),
        Ok(ZERO_NAVY)
    );
    assert_eq!(captured.borrow_navy(1).unwrap().ledger_navy(), Ok(NAVY));
    captured.finish();
    reader.clear();
    assert_eq!(
        memory.count(0xa100),
        1,
        "every borrower reuses the already failed launch observation"
    );
    assert_eq!(memory.count(ZERO_NAVY + LAUNCH_HEADER), 1);
}

#[test]
fn slot_zero_identity_failure_is_not_retried_for_launch_or_borrowers() {
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
    let (capacity, launch_capacity) = launch_capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&failing, 0, STATE, Some(71), &mut launch);
    assert!(words(&launch).is_none());
    let reason = captured.borrow_navy(0).err().expect("slot zero refused");
    captured.finish();
    reader.clear();
    assert_eq!(
        memory.count(ZERO_NAVY),
        1,
        "a second identity read would recover and hide the first refusal"
    );
    assert_eq!(memory.count(0xa100), 1);
    assert!(
        captured.navies.first[..captured.navies.len]
            .iter()
            .any(|row| row.slot == 0 && row.identity_reason == Some(reason))
    );
}

#[test]
fn short_physical_arena_still_transfers_the_fixed_slot_zero_attempt_once() {
    let memory = launch_world();
    let (mut capacity, launch_capacity) = launch_capacity();
    capacity.navies = 1;
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    pass(&mut captured, &mut launch, &memory, Some(71));
    assert_eq!(words(&launch), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    assert_eq!(memory.count(0xa100), 1);
    assert_eq!(launch.launch_bases.required, 3);
    assert_eq!(captured.required().navies, 4);
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
}

#[test]
fn short_launch_storage_grows_both_arenas_and_later_hours_reuse_it() {
    fn capture_fixed(memory: &TestMemory, launch: &mut Launch, hour: i32) {
        let reader = native::Reader::new(memory, 0, STATE);
        let mut observed = reader.observe_navy(0).unwrap();
        let captured = observed.capture_launch(
            memory,
            0,
            &mut launch.launch_scratch.first,
            &mut launch.launch_bases.first,
        );
        launch.sample_hour = Some(hour);
        launch.launch_bases.required = captured.required;
        launch.launch_scratch.required = captured.required;
        launch.launch_result = captured.provider;
        launch.launch_bases.len = match captured.provider {
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => len,
            _ => 0,
        };
    }
    let memory = launch_world();
    let (_, mut launch_capacity) = capacity();
    let mut launch = Launch::new(launch_capacity);
    capture_fixed(&memory, &mut launch, 71);
    assert_eq!(launch.required().launch_bases, 3);
    assert_eq!(words(&launch), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    launch.grow(&mut launch_capacity);
    assert!(launch.launch_bases.first.len() >= 3 && launch.launch_scratch.first.len() >= 3);
    let allocated = launch.allocated_bytes();
    memory.calls.borrow_mut().clear();
    capture_fixed(&memory, &mut launch, 72);
    assert_eq!(launch.allocated_bytes(), allocated);
    assert_eq!(words(&launch), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    launch.reset();
    assert!(launch.projection_launch(None).unwrap().is_none());
}

#[test]
fn worker_launch_grows_empty_arenas_before_first_publication() {
    let memory = launch_world();
    let mut captured = Captured::new(Capacities::default());
    let mut launch = Launch::new(LaunchCapacities::default());
    assert_eq!(launch.allocated_bytes(), 0);
    pass(&mut captured, &mut launch, &memory, Some(71));
    assert_eq!(words(&launch), Some(vec![(-1, 999), (0, 999), (-1, 999)]));
    assert_eq!(launch.required().launch_bases, 3);
    assert!(launch.launch_bases.first.len() >= 3 && launch.launch_scratch.first.len() >= 3);
    assert_eq!(
        memory.count(ZERO_NAVY + LAUNCH_HEADER),
        1,
        "growth uses the original launch header"
    );
}

#[test]
fn native_null_projection_is_fresh_and_distinct_from_lists_and_read_faults() {
    use replay_data::rows::LaunchNavalBases;

    let mut memory = launch_world();
    let (capacity, launch_capacity) = launch_capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
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
        pass(&mut captured, &mut launch, &memory, Some(hour));
        let packet = launch
            .projection_launch(Some(vec![9, 3, 9]))
            .unwrap()
            .unwrap();
        assert_eq!(packet.sample_hour, replay_data::rows::PacketHour::Host);
        assert_eq!(packet.owned_states, Some(vec![9, 3, 9]));
        match hour {
            71 => assert!(
                matches!(packet.naval_bases, LaunchNavalBases::Bases(bases) if bases.len() == 3)
            ),
            72 => {
                assert_eq!(packet.naval_bases, LaunchNavalBases::NativeNull);
                assert_eq!(launch.required().launch_bases, 0);
                assert_eq!(launch.launch_bases.len, 0);
                assert_eq!(memory.count(ZERO_NAVY + LAUNCH_HEADER), 0);
            }
            73 => assert_eq!(packet.naval_bases, LaunchNavalBases::Unavailable),
            74 => assert_eq!(packet.naval_bases, LaunchNavalBases::Bases(vec![])),
            _ => unreachable!(),
        }
        assert_eq!(memory.count(0xa100), 1);
    }
    launch.reset();
    assert!(launch.projection_launch(None).unwrap().is_none());
}
