use super::*;
use engine::{cbuilding_template, cprovince, memory::SliceMemory};
use std::cell::RefCell;

const BASE: usize = 0x1000;

struct Fixture {
    bytes: Vec<u8>,
    first_building: usize,
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
            &(BASE as u64 + ccountry::VTABLE_RVA as u64).to_le_bytes(),
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
            &(BASE as u64 + cbuilding_template::VTABLE_RVA as u64).to_le_bytes(),
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
                &(BASE as u64 + vtable as u64).to_le_bytes(),
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
                &(BASE as u64 + cbuilding_status::VTABLE_RVA as u64).to_le_bytes(),
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
                    &(BASE as u64 + cbuilding::VTABLE_RVA as u64).to_le_bytes(),
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
                // Active buildings keep positive signed levels in order.
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
fn cold_building_worker_keeps_5000_countries_and_later_mod_sized_location_vectors() {
    const COUNTRIES: usize = 5_000;
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
            engine::cpdx_array::Array::read(&memory, BASE + cgame_state::COUNTRIES, 8)
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
        // The existing reader grows even a one-row destination; sufficient
        // storage and short storage consume exactly the same original fields.
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
