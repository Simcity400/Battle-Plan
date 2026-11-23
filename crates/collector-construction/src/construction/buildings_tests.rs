use super::*;
use engine_native_parts::cbuilding_template;
use engine_native::cprovince;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const BASE: usize = 0x1000;

struct Fixture {
    bytes: Vec<u8>,
    first_building: usize,
    first_vector: usize,
    first_slots: usize,
    first_location: usize,
}

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}

fn allocate(bytes: &mut Vec<u8>, len: usize) -> usize {
    let start = bytes.len().next_multiple_of(8);
    let address = BASE + start;
    bytes.resize(start + len, 0);
    address
}

fn array(bytes: &mut [u8], address: usize, data: usize, count: usize) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &(count as u32).to_le_bytes());
    put(bytes, address + 12, &(count as u32).to_le_bytes());
}

fn fixture(countries: usize, types: usize) -> Fixture {
    let mut bytes = vec![0; 0x2000];
    let country_slots = allocate(&mut bytes, (countries + 1) * 8);
    let links = allocate(&mut bytes, (countries + 1) * 4);
    let country_objects = allocate(&mut bytes, countries * 16);
    array(
        &mut bytes,
        BASE + cgame_state::COUNTRIES,
        country_slots,
        countries + 1,
    );
    array(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS,
        links,
        countries + 1,
    );
    for id in 1..=countries {
        let country = country_objects + (id - 1) * 16;
        put(
            &mut bytes,
            country_slots + id * 8,
            &(country as u64).to_le_bytes(),
        );
        put(&mut bytes, links + id * 4, &(id as u32).to_le_bytes());
        put(
            &mut bytes,
            country,
            &(BASE as u64 + ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            &(id as u32).to_le_bytes(),
        );
    }
    let templates = allocate(&mut bytes, types * cbuilding_template::SIZE);
    for ordinal in 0..types {
        let template = templates + ordinal * cbuilding_template::SIZE;
        put(
            &mut bytes,
            template,
            &(BASE as u64 + cbuilding_template::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + cbuilding_template::TYPE_ID,
            &(0xffff0000 + ordinal as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + cbuilding_template::DATABASE_ORDINAL,
            &(ordinal as u32).to_le_bytes(),
        );
    }
    let states = allocate(&mut bytes, (countries + 1) * 8);
    let provinces = allocate(&mut bytes, (countries + 1) * 8);
    array(
        &mut bytes,
        BASE + cgame_state::STATES,
        states,
        countries + 1,
    );
    array(
        &mut bytes,
        BASE + cgame_state::PROVINCES,
        provinces,
        countries + 1,
    );
    let stride = cprovince::BUILDING_STATUS + cbuilding_status::COUNTRY + 8;
    let state_objects = allocate(&mut bytes, countries * stride);
    let province_objects = allocate(&mut bytes, countries * stride);
    let mut first_building = 0;
    let mut first_vector = 0;
    for province_scope in [false, true] {
        let (slots, objects, vtable, id_offset, status_offset, flags) = if province_scope {
            (
                provinces,
                province_objects,
                cprovince::VTABLE_RVA,
                cprovince::ID,
                cprovince::BUILDING_STATUS,
                cbuilding_status::PROVINCE_FLAG,
            )
        } else {
            (
                states,
                state_objects,
                cstate::VTABLE_RVA,
                cstate::ID,
                cstate::BUILDING_STATUS,
                cbuilding_status::STATE_FLAG,
            )
        };
        for id in 1..=countries {
            let location = objects + (id - 1) * stride;
            let status = location + status_offset;
            put(&mut bytes, slots + id * 8, &(location as u64).to_le_bytes());
            put(
                &mut bytes,
                location,
                &(BASE as u64 + vtable.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, location + id_offset, &(id as u32).to_le_bytes());
            if province_scope {
                put(
                    &mut bytes,
                    location + cprovince::STATE,
                    &((state_objects + (id - 1) * stride) as u64).to_le_bytes(),
                );
            }
            put(
                &mut bytes,
                status,
                &(BASE as u64 + cbuilding_status::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                status + cbuilding_status::FLAGS,
                &flags.to_le_bytes(),
            );
            put(
                &mut bytes,
                status + cbuilding_status::ID,
                &(id as u32).to_le_bytes(),
            );
            let pointers = allocate(&mut bytes, types * 8);
            if first_vector == 0 {
                first_vector = pointers;
            }
            let indices = allocate(&mut bytes, types * 4);
            let active = allocate(&mut bytes, types / 3 * 8);
            array(
                &mut bytes,
                status + cbuilding_status::BUILDINGS,
                pointers,
                types,
            );
            array(
                &mut bytes,
                status + cbuilding_status::BUILDING_INDICES,
                indices,
                types,
            );
            array(
                &mut bytes,
                status + cbuilding_status::ACTIVE_BUILDINGS,
                active,
                types / 3,
            );
            for ordinal in 0..types {
                let building = allocate(&mut bytes, cbuilding::TEMPLATE + 8);
                if first_building == 0 {
                    first_building = building;
                }
                put(
                    &mut bytes,
                    pointers + ordinal * 8,
                    &(building as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    indices + ordinal * 4,
                    &(ordinal as i32).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building,
                    &(BASE as u64 + cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::TYPE_ID,
                    &(0xffff0000 + ordinal as u32).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::LEVEL,
                    &(ordinal as i16 % 3 - 1).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::HEALTHY_LEVEL,
                    &i16::MIN.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::PARTIAL_HEALTH,
                    &(-5_000_000_000_i64 - id as i64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::BUILDING_STATUS,
                    &(status as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    building + cbuilding::TEMPLATE,
                    &((templates + ordinal * cbuilding_template::SIZE) as u64).to_le_bytes(),
                );
                if ordinal % 3 == 2 {
                    put(
                        &mut bytes,
                        active + ordinal / 3 * 8,
                        &(building as u64).to_le_bytes(),
                    );
                }
            }
        }
    }
    Fixture {
        bytes,
        first_building,
        first_vector,
        first_slots: states,
        first_location: state_objects,
    }
}

struct Trace<'a> {
    source: SliceMemory<'a>,
    blocked: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Trace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if output.is_empty() {
            return true;
        }
        self.reads.borrow_mut().push((address, output.len()));
        if self.blocked == Some(address) {
            return false;
        }
        self.source.copy(address, output)
    }
}

#[test]
fn cold_building_worker_keeps_countries_and_later_mod_sized_location_vectors() {
    const COUNTRIES: usize = 5;
    let mut cold = Buildings::vacant();
    assert_eq!(cold.allocated_bytes(), 0);
    for (hour, types) in [(0, 2), (24, 17)] {
        let fixture = fixture(COUNTRIES, types);
        let memory = Trace {
            source: SliceMemory {
                base: BASE,
                bytes: &fixture.bytes,
            },
            blocked: None,
            reads: RefCell::default(),
        };
        let rows = COUNTRIES * types;
        assert_eq!(
            engine_native_memory::cpdx_array::Array::read(&memory, BASE + cgame_state::COUNTRIES, 8)
                .unwrap()
                .count,
            COUNTRIES as u32 + 1
        );
        memory.reads.borrow_mut().clear();
        cold.capture(&memory, BASE, BASE, hour, true);
        let trace = memory.reads.take();
        assert_eq!(cold.state_buildings.outcome, Outcome::Rows { len: rows });
        assert_eq!(cold.province_buildings.outcome, Outcome::Rows { len: rows });
        for (table, flags) in [
            (&cold.state_buildings, cbuilding_status::STATE_FLAG),
            (&cold.province_buildings, cbuilding_status::PROVINCE_FLAG),
        ] {
            for (index, row) in table.rows[..rows].iter().enumerate() {
                let ordinal = index % types;
                assert_eq!(row.location, (index / types + 1) as u32);
                assert_eq!(row.building_type, 0xffff0000 + ordinal as u32);
                assert_eq!(row.ordinal, ordinal as u32);
                assert_eq!(row.status_flags, flags);
                assert_eq!(row.country, 0);
                assert_eq!(row.level, ordinal as i16 % 3 - 1);
                assert_eq!(row.healthy_level, i16::MIN);
                assert_eq!(row.partial_health, -5_000_000_000 - i64::from(row.location));
                assert_eq!(row.active, ordinal % 3 == 2);
            }
        }
        for available in [1, rows] {
            let mut control = Captured::with_capacities(&Capacities::for_test_buildings(available))
                .take_buildings();
            control.capture(&memory, BASE, BASE, hour, true);
            assert_eq!(memory.reads.take(), trace);
            assert_eq!(
                control.state_buildings.outcome,
                cold.state_buildings.outcome
            );
            assert_eq!(
                control.province_buildings.outcome,
                cold.province_buildings.outcome
            );
            assert_eq!(
                control.state_buildings.rows[..rows],
                cold.state_buildings.rows[..rows]
            );
            assert_eq!(
                control.province_buildings.rows[..rows],
                cold.province_buildings.rows[..rows]
            );
        }
    }
}

#[test]
fn building_worker_growth_preserves_original_fault_order_and_the_independent_scope() {
    let fixture = fixture(2, 3);
    let mut expected_trace = None;
    for available in [0, 1, 6] {
        let memory = Trace {
            source: SliceMemory {
                base: BASE,
                bytes: &fixture.bytes,
            },
            blocked: Some(fixture.first_building),
            reads: RefCell::default(),
        };
        let mut buildings =
            Captured::with_capacities(&Capacities::for_test_buildings(available)).take_buildings();
        buildings.capture(&memory, BASE, BASE, 0, true);
        assert_eq!(
            buildings.state_buildings.outcome,
            Outcome::Unavailable {
                reason: "building vtable unreadable"
            }
        );
        assert_eq!(
            buildings.province_buildings.outcome,
            Outcome::Rows { len: 6 }
        );
        let trace = memory.reads.into_inner();
        assert_eq!(
            trace
                .iter()
                .filter(|&&(address, _)| address == fixture.first_building)
                .count(),
            1
        );
        if let Some(expected) = &expected_trace {
            assert_eq!(&trace, expected);
        } else {
            expected_trace = Some(trace);
        }
    }
}

mod carried {
    use super::*;
    use crate::building_changes::{Changes, Counts, Watches};
    use engine_mutations::building_mutations;
    use engine_native_memory::cpdx_array::Array;
    use std::ops::Range;

    const COUNTRIES: usize = 3;
    const TYPES: usize = 4;
    const STATUSES: u64 = 2 * COUNTRIES as u64;
    const ROWS: usize = COUNTRIES * TYPES;

    struct Hour {
        outcomes: [Outcome; 2],
        states: Vec<cbuilding::Row>,
        provinces: Vec<cbuilding::Row>,
        counts: Counts,
        building_reads: usize,
        location_reads: usize,
        reads: Vec<(usize, usize)>,
    }

    fn capture(fixture: &Fixture, store: &mut BuildingStore, watches: &Watches) -> Hour {
        let memory = Trace {
            source: SliceMemory {
                base: BASE,
                bytes: &fixture.bytes,
            },
            blocked: None,
            reads: RefCell::default(),
        };
        let mut buildings = Buildings::vacant();
        buildings.capture_with(&memory, BASE, BASE, Some((store, watches)));
        let outcomes = [
            buildings.state_buildings.outcome,
            buildings.province_buildings.outcome,
        ];
        let reads = memory.reads.into_inner();
        Hour {
            outcomes,
            states: buildings.state_buildings.rows[..outcomes[0].len()].to_vec(),
            provinces: buildings.province_buildings.rows[..outcomes[1].len()].to_vec(),
            counts: store.counts(),
            building_reads: reads
                .iter()
                .filter(|&&(address, _)| address >= fixture.first_vector)
                .count(),
            location_reads: reads
                .iter()
                .filter(|&&(address, _)| address >= fixture.first_slots)
                .count(),
            reads,
        }
    }

    fn hour(fixture: &Fixture, store: &mut BuildingStore, watches: &Watches) -> Hour {
        let hour = capture(fixture, store, watches);
        assert_eq!(hour.outcomes, [Outcome::Rows { len: ROWS }; 2]);
        hour
    }

    fn memory(fixture: &Fixture) -> SliceMemory<'_> {
        SliceMemory {
            base: BASE,
            bytes: &fixture.bytes,
        }
    }

    fn status(fixture: &Fixture, province: bool, id: usize) -> usize {
        let memory = memory(fixture);
        let slots = if province {
            cgame_state::PROVINCES
        } else {
            cgame_state::STATES
        };
        let locations = Array::read(&memory, BASE + slots, 8).unwrap();
        let location = engine_native_memory::read::<u64>(&memory, locations.element(id as u32, 8).unwrap())
            .unwrap() as usize;
        location
            + if province {
                cprovince::BUILDING_STATUS
            } else {
                cstate::BUILDING_STATUS
            }
    }

    fn building(fixture: &Fixture, province: bool, id: usize, ordinal: usize) -> (usize, usize) {
        let status = status(fixture, province, id);
        let memory = memory(fixture);
        let vector = Array::read(&memory, status + cbuilding_status::BUILDINGS, 8).unwrap();
        let building =
            engine_native_memory::read::<u64>(&memory, vector.element(ordinal as u32, 8).unwrap())
                .unwrap();
        (status, building as usize)
    }

    fn statuses_read(fixture: &Fixture, reads: &[(usize, usize)]) -> Vec<(bool, usize)> {
        let memory = memory(fixture);
        let mut read = Vec::new();
        for province in [false, true] {
            for id in 1..=COUNTRIES {
                let status = status(fixture, province, id);
                let mut spans: Vec<Range<usize>> = Vec::new();
                let mut buildings = Array::default();
                for (offset, stride) in [
                    (cbuilding_status::BUILDINGS, 8),
                    (cbuilding_status::BUILDING_INDICES, 4),
                    (cbuilding_status::ACTIVE_BUILDINGS, 8),
                ] {
                    let vector = Array::read(&memory, status + offset, stride).unwrap();
                    spans.push(vector.data..vector.data + vector.count as usize * stride);
                    if offset == cbuilding_status::BUILDINGS {
                        buildings = vector;
                    }
                }
                for ordinal in 0..buildings.count {
                    let building =
                        engine_native_memory::read::<u64>(&memory, buildings.element(ordinal, 8).unwrap())
                            .unwrap() as usize;
                    spans.push(building..building + cbuilding::TEMPLATE + 8);
                }
                if reads
                    .iter()
                    .any(|&(address, _)| spans.iter().any(|span| span.contains(&address)))
                {
                    read.push((province, id));
                }
            }
        }
        read
    }

    fn no_location_walk(fixture: &Fixture, reads: &[(usize, usize)]) {
        for &(address, _) in reads {
            assert!(
                !(fixture.first_slots..fixture.first_location).contains(&address),
                "slot read at {address:#x}"
            );
            for province in [false, true] {
                for id in 1..=COUNTRIES {
                    let (offset, id_offset) = if province {
                        (cprovince::BUILDING_STATUS, cprovince::ID)
                    } else {
                        (cstate::BUILDING_STATUS, cstate::ID)
                    };
                    let location = status(fixture, province, id) - offset;
                    assert!(
                        address != location && address != location + id_offset,
                        "location read at {address:#x}"
                    );
                }
            }
        }
    }

    fn fresh(fixture: &Fixture) -> (Vec<cbuilding::Row>, Vec<cbuilding::Row>) {
        let memory = memory(fixture);
        let mut buildings = Buildings::vacant();
        buildings.capture(&memory, BASE, BASE, 0, true);
        let [states, provinces] = [&buildings.state_buildings, &buildings.province_buildings]
            .map(|table| table.rows[..table.outcome.len()].to_vec());
        (states, provinces)
    }

    fn first_hour(fixture: &Fixture, store: &mut BuildingStore, watches: &Watches) -> Hour {
        let first = hour(fixture, store, watches);
        assert_eq!((first.counts.read, first.counts.reused), (STATUSES, 0));
        assert!(!first.counts.quiet);
        assert_ne!(first.building_reads, 0);
        watches.boundary(false);
        first
    }

    fn nudge_site() -> usize {
        building_mutations::MUTATIONS
            .iter()
            .position(|&(_, receiver)| receiver == building_mutations::Receiver::Structure)
            .unwrap()
    }

    #[test]
    fn quiet_hours_read_only_the_location_tables_and_publish_identical_rows() {
        let fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = first_hour(&fixture, &mut store, &watches);
        for _ in 0..3 {
            let quiet = hour(&fixture, &mut store, &watches);
            assert_eq!(quiet.states, first.states);
            assert_eq!(quiet.provinces, first.provinces);
            assert!(quiet.counts.quiet);
            assert_eq!((quiet.counts.read, quiet.counts.reused), (0, STATUSES));
            assert_eq!(quiet.counts.reused_rows, 2 * ROWS as u64);
            assert_eq!((quiet.location_reads, quiet.building_reads), (0, 0));
            assert_eq!(
                quiet.reads,
                [
                    (BASE + cgame_state::STATES, 16),
                    (BASE + cgame_state::PROVINCES, 16)
                ]
            );
            watches.boundary(false);
        }
    }

    #[test]
    fn a_hooked_write_rereads_only_its_status() {
        let mut fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = first_hour(&fixture, &mut store, &watches);
        let (status, building) = building(&fixture, false, 2, 1);
        let notification = watches.notify(Some(status), 0);
        put(
            &mut fixture.bytes,
            building + cbuilding::LEVEL,
            &9_i16.to_le_bytes(),
        );
        drop(notification);
        let unwalked = BASE + 8;
        Changes::sequence(&watches, unwalked).unwrap();
        drop(watches.notify(Some(unwalked), 6));
        watches.boundary(false);
        let changed = hour(&fixture, &mut store, &watches);
        assert!(changed.counts.settled && !changed.counts.quiet);
        assert_eq!(changed.counts.marked, 1);
        no_location_walk(&fixture, &changed.reads);
        assert_eq!(
            (changed.counts.read, changed.counts.reused),
            (1, STATUSES - 1)
        );
        assert_eq!(statuses_read(&fixture, &changed.reads), [(false, 2)]);
        let mut expected = first.states.clone();
        expected[TYPES + 1].level = 9;
        assert_eq!(changed.states, expected);
        assert_eq!(changed.provinces, first.provinces);
        assert_eq!((changed.states, changed.provinces), fresh(&fixture));
        watches.boundary(false);
        let quiet = hour(&fixture, &mut store, &watches);
        assert!(quiet.counts.quiet);
        assert_eq!((quiet.counts.read, quiet.counts.reused), (0, STATUSES));
        assert_eq!(quiet.location_reads, 0);
        assert_eq!(quiet.states, expected);
    }

    #[test]
    fn an_invalidating_hour_or_a_forgotten_store_rereads_every_status() {
        let fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = first_hour(&fixture, &mut store, &watches);
        watches.boundary(true);
        let reloaded = hour(&fixture, &mut store, &watches);
        assert_eq!(
            (reloaded.counts.read, reloaded.counts.reused),
            (STATUSES, 0)
        );
        assert!(!reloaded.counts.quiet);
        assert_eq!(reloaded.states, first.states);
        watches.boundary(false);
        store.forget();
        let forgotten = hour(&fixture, &mut store, &watches);
        assert_eq!(
            (forgotten.counts.read, forgotten.counts.reused),
            (STATUSES, 0)
        );
        assert!(!forgotten.counts.quiet);
        watches.boundary(false);
        drop(watches.notify(None, 0));
        let unnamed = hour(&fixture, &mut store, &watches);
        assert_eq!((unnamed.counts.read, unnamed.counts.reused), (STATUSES, 0));
        assert!(!unnamed.counts.quiet);
    }

    #[test]
    fn a_changed_location_table_header_walks_every_location() {
        let mut fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = first_hour(&fixture, &mut store, &watches);
        put(
            &mut fixture.bytes,
            BASE + cgame_state::PROVINCES + 8,
            &(COUNTRIES as u32 + 2).to_le_bytes(),
        );
        let walked = hour(&fixture, &mut store, &watches);
        assert!(!walked.counts.quiet);
        assert_ne!(walked.location_reads, 0);
        assert_eq!(walked.building_reads, 0);
        assert_eq!((walked.counts.read, walked.counts.reused), (0, STATUSES));
        assert_eq!(walked.provinces, first.provinces);
        watches.boundary(false);
        let quiet = hour(&fixture, &mut store, &watches);
        assert!(quiet.counts.quiet);
        assert_eq!(quiet.location_reads, 0);
    }

    #[test]
    fn a_written_status_whose_building_count_changed_moves_the_later_rows() {
        let mut fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        first_hour(&fixture, &mut store, &watches);
        let status = status(&fixture, false, 2);
        let notification = watches.notify(Some(status), 4);
        put(
            &mut fixture.bytes,
            status + cbuilding_status::BUILDINGS + 12,
            &((TYPES - 1) as u32).to_le_bytes(),
        );
        drop(notification);
        watches.boundary(false);
        let shrunk = capture(&fixture, &mut store, &watches);
        assert!(shrunk.counts.settled);
        no_location_walk(&fixture, &shrunk.reads);
        assert_eq!(statuses_read(&fixture, &shrunk.reads), [(false, 2)]);
        assert_eq!(
            shrunk.outcomes,
            [Outcome::Rows { len: ROWS - 1 }, Outcome::Rows { len: ROWS }]
        );
        let expected = fresh(&fixture);
        assert_eq!((shrunk.states, shrunk.provinces), expected);
        for _ in 0..3 {
            watches.boundary(false);
            let carried = capture(&fixture, &mut store, &watches);
            assert!(carried.counts.quiet);
            assert_eq!(carried.location_reads, 0);
            assert_eq!((carried.states, carried.provinces), expected);
        }
    }

    #[test]
    fn entering_nudge_mode_reads_every_status_afterwards() {
        let fixture = fixture(COUNTRIES, TYPES);
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = first_hour(&fixture, &mut store, &watches);
        drop(watches.structure(nudge_site()));
        assert!(watches.poisoned());
        for _ in 0..2 {
            watches.boundary(false);
            let after = hour(&fixture, &mut store, &watches);
            assert!(!after.counts.quiet);
            assert_eq!((after.counts.read, after.counts.reused), (STATUSES, 0));
            assert_eq!(after.states, first.states);
        }
    }

    #[test]
    fn statuses_without_buildings_are_held_and_read_once_written() {
        let mut fixture = fixture(COUNTRIES, TYPES);
        for (province, id) in [(false, 2), (true, 1)] {
            let status = status(&fixture, province, id);
            for vector in [
                cbuilding_status::BUILDINGS,
                cbuilding_status::ACTIVE_BUILDINGS,
            ] {
                put(
                    &mut fixture.bytes,
                    status + vector + 12,
                    &0_u32.to_le_bytes(),
                );
            }
        }
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let first = capture(&fixture, &mut store, &watches);
        assert_eq!(first.outcomes, [Outcome::Rows { len: ROWS - TYPES }; 2]);
        assert_eq!(
            (first.states.clone(), first.provinces.clone()),
            fresh(&fixture)
        );
        assert_eq!(first.counts.empty, 2);
        assert_eq!((first.counts.read, first.counts.reused), (STATUSES - 2, 0));
        assert_eq!(watches.registered(), STATUSES as usize);
        watches.boundary(false);
        let quiet = capture(&fixture, &mut store, &watches);
        assert!(quiet.counts.quiet);
        assert_eq!(quiet.counts.empty, 2);
        assert_eq!(
            (quiet.states, quiet.provinces),
            (first.states, first.provinces)
        );
        assert_eq!(watches.registered(), STATUSES as usize);

        watches.boundary(false);
        let built = status(&fixture, true, 1);
        let notification = watches.notify(Some(built), 4);
        for (vector, count) in [
            (cbuilding_status::BUILDINGS, TYPES),
            (cbuilding_status::ACTIVE_BUILDINGS, TYPES / 3),
        ] {
            put(
                &mut fixture.bytes,
                built + vector + 12,
                &(count as u32).to_le_bytes(),
            );
        }
        drop(notification);
        watches.boundary(false);
        let built = capture(&fixture, &mut store, &watches);
        assert!(built.counts.settled);
        no_location_walk(&fixture, &built.reads);
        assert_eq!(
            built.outcomes,
            [
                Outcome::Rows { len: ROWS - TYPES },
                Outcome::Rows { len: ROWS }
            ]
        );
        assert_eq!((built.states, built.provinces), fresh(&fixture));
        assert_eq!(built.counts.empty, 1);

        let status = status(&fixture, false, 2);
        put(
            &mut fixture.bytes,
            status + cbuilding_status::ACTIVE_BUILDINGS + 12,
            &1_u32.to_le_bytes(),
        );
        let fresh = {
            let memory = memory(&fixture);
            let mut buildings = Buildings::vacant();
            buildings.capture(&memory, BASE, BASE, 0, true);
            [
                buildings.state_buildings.outcome,
                buildings.province_buildings.outcome,
            ]
        };
        assert_eq!(
            fresh[0],
            Outcome::Unavailable {
                reason: "active building vector exceeds building vector"
            }
        );
        let watches = Watches::enabled(64);
        let mut store = BuildingStore::new();
        let failed = capture(&fixture, &mut store, &watches);
        assert_eq!(failed.outcomes, fresh);
    }
}
