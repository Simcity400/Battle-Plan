use super::*;
use crate::cgame_state;
use engine_native_memory::memory::SliceMemory;

const MODULE: usize = 0x140000000;
const MEMORY: usize = 0x1000;
const COUNTRY_POINTER: usize = 0x3000;
const LAND: usize = 0x5000;
const LAND_GROUP: usize = 0x5300;
const ORDERS: usize = 0x5600;
const AIR: usize = 0x5900;
const AIR_GROUP: usize = 0x5c00;

fn put<const N: usize>(bytes: &mut [u8], address: usize, value: [u8; N]) {
    bytes[address - MEMORY..address - MEMORY + N].copy_from_slice(&value);
}

fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, (data as u64).to_le_bytes());
    put(bytes, address + 8, count.to_le_bytes());
    put(bytes, address + 12, count.to_le_bytes());
}

fn handle(kind: u32) -> u64 {
    (u64::from(u32::MAX) << 32) | u64::from(kind)
}

fn object(bytes: &mut [u8], address: usize, vtable: Rva, kind: u32) {
    put(bytes, address, ((MODULE + vtable.0) as u64).to_le_bytes());
    put(bytes, address + 8, handle(kind).to_le_bytes());
}

fn string(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - MEMORY..address - MEMORY + value.len()].copy_from_slice(value);
    put(
        bytes,
        address + msvc_string::LENGTH,
        (value.len() as u64).to_le_bytes(),
    );
    put(bytes, address + msvc_string::CAPACITY, 15_u64.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x6000];
    array(&mut bytes, MEMORY + cgame_state::COUNTRIES, 0x2000, 2);
    put(&mut bytes, 0x2008, (COUNTRY_POINTER as u64).to_le_bytes());
    array(&mut bytes, MEMORY + cgame_state::COUNTRY_LINKS, 0x2100, 3);
    put(&mut bytes, 0x2104, 1_u32.to_le_bytes());
    put(&mut bytes, 0x2108, 1_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY_POINTER,
        ((MODULE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::ID,
        2_u32.to_le_bytes(),
    );
    array(&mut bytes, COUNTRY_POINTER + ccountry::THEATRES, 0x2200, 1);
    put(&mut bytes, 0x2200, (LAND as u64).to_le_bytes());
    array(
        &mut bytes,
        COUNTRY_POINTER + ccountry::AIR_THEATRES,
        0x2220,
        1,
    );
    put(&mut bytes, 0x2220, (AIR as u64).to_le_bytes());
    object(&mut bytes, LAND, VTABLE_RVA, REFERENCE_TYPE);
    put(&mut bytes, LAND + OWNER, 1_u32.to_le_bytes());
    put(&mut bytes, LAND + VOLUNTEERS_THEATRE, 0_i32.to_le_bytes());
    array(&mut bytes, LAND + THEATER_GROUP_DATA, 0x2240, 1);
    put(&mut bytes, 0x2240, (LAND_GROUP as u64).to_le_bytes());
    object(
        &mut bytes,
        LAND_GROUP,
        ctheater_group::VTABLE_RVA,
        ctheater_group::REFERENCE_TYPE,
    );
    put(
        &mut bytes,
        LAND_GROUP + ctheater_group::THEATRE,
        (LAND as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LAND_GROUP + ctheater_group::PRIORITY,
        u32::MAX.to_le_bytes(),
    );
    string(
        &mut bytes,
        LAND_GROUP + ctheater_group::NAME,
        &[0xff, 0, 0x80],
    );
    array(
        &mut bytes,
        LAND_GROUP + ctheater_group::ORDERS_GROUP_DATA,
        0x2260,
        1,
    );
    put(&mut bytes, 0x2260, (ORDERS as u64).to_le_bytes());
    object(
        &mut bytes,
        ORDERS,
        corders_group::VTABLE_RVA,
        corders_group::REFERENCE_TYPE,
    );
    put(
        &mut bytes,
        ORDERS + corders_group::THEATRE,
        (LAND as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ORDERS + corders_group::THEATER_GROUP,
        (LAND_GROUP as u64).to_le_bytes(),
    );
    object(
        &mut bytes,
        AIR,
        cair_theatre::VTABLE_RVA,
        cair_theatre::TYPE,
    );
    put(&mut bytes, AIR + cair_theatre::COUNTRY, 1_u32.to_le_bytes());
    string(&mut bytes, AIR + cair_theatre::NAME, b"air-name");
    array(&mut bytes, AIR + cair_theatre::GROUPS, 0x2280, 1);
    put(&mut bytes, 0x2280, (AIR_GROUP as u64).to_le_bytes());
    object(
        &mut bytes,
        AIR_GROUP,
        cair_group::VTABLE_RVA,
        cair_group::TYPE,
    );
    string(&mut bytes, AIR_GROUP + cair_group::NAME, b"");
    put(
        &mut bytes,
        AIR_GROUP + cair_group::THEATRE_HANDLE,
        handle(cair_theatre::TYPE).to_le_bytes(),
    );
    array(&mut bytes, AIR_GROUP + cair_group::WINGS, 0x22a0, 1);
    put(&mut bytes, 0x22a0, handle(engine_native_parts::cair_wing_layout::TYPE).to_le_bytes());
    bytes
}

#[test]
fn reserved_header_failure_keeps_first_observation_without_a_gather_retry() {
    struct FirstFault<'a> {
        memory: SliceMemory<'a>,
        attempts: std::cell::Cell<usize>,
    }
    impl Memory for FirstFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == LAND + OWNER {
                self.attempts.set(self.attempts.get() + 1);
                if self.attempts.get() == 1 {
                    return false;
                }
            }
            self.memory.copy(address, output)
        }
    }
    let mut bytes = fixture();
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::ID,
        0_u32.to_le_bytes(),
    );
    put(&mut bytes, LAND + OWNER, 0_u32.to_le_bytes());
    let memory = FirstFault {
        memory: SliceMemory {
            base: MEMORY,
            bytes: &bytes,
        },
        attempts: std::cell::Cell::new(0),
    };
    let root = engine_native_memory::country_identity::Country {
        pointer: COUNTRY_POINTER,
        canonical_slot: 0,
        id: 0,
    };
    let mut output = vec![Row::default(); 1];
    let mut callbacks = 0;
    let walk = walk_reserved_land_headers(&memory, MODULE, Ok(root), &mut output, |_, _| {
        callbacks += 1;
        Ok(())
    });
    assert_eq!(walk.len, Err("reserved land theatre header unreadable"));
    assert_eq!(walk.native_count, Some(1));
    assert_eq!((memory.attempts.get(), callbacks), (1, 0));
    assert_eq!(output, [Row::default(); 1]);
    array(&mut bytes, COUNTRY_POINTER + ccountry::THEATRES, 0, 0);
    let memory = SliceMemory {
        base: MEMORY,
        bytes: &bytes,
    };
    let empty = walk_reserved_land_headers(&memory, MODULE, Ok(root), &mut Vec::new(), |_, _| {
        panic!("fresh empty theatre vector must not demand unrelated OOB")
    });
    assert_eq!((empty.len, empty.native_count), (Ok(0), Some(0)));
}

#[derive(Default)]
struct Captured {
    land: Vec<Row>,
    groups: Vec<ctheater_group::Row>,
    orders: Vec<ctheater_group::MemberRow>,
    air: Vec<cair_theatre::Row>,
    air_groups: Vec<cair_group::Row>,
    wings: Vec<cair_group::MemberRow>,
    names: Vec<u8>,
}

impl Captured {
    fn read(&mut self, bytes: &[u8]) -> Walk {
        walk(
            &SliceMemory {
                base: MEMORY,
                bytes,
            },
            MODULE,
            MEMORY,
            true,
            Buffers {
                land: &mut self.land,
                land_groups: &mut self.groups,
                land_members: &mut self.orders,
                air: &mut self.air,
                air_groups: &mut self.air_groups,
                air_members: &mut self.wings,
                names: &mut self.names,
            },
        )
    }
}

fn count(bytes: &[u8]) -> Walk {
    walk_buffers(
        &SliceMemory {
            base: MEMORY,
            bytes,
        },
        MODULE,
        MEMORY,
        true,
        Storage {
            land: Buffer::Count,
            land_groups: Buffer::Count,
            land_members: Buffer::Count,
            air: Buffer::Count,
            air_groups: Buffer::Count,
            air_members: Buffer::Count,
            names: Buffer::Count,
        },
    )
}

#[test]
fn gathered_land_headers_match_per_field_reads() {
    struct Holed<'a> {
        memory: SliceMemory<'a>,
        hole: Option<usize>,
        per_field: bool,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Holed<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self
                .hole
                .is_some_and(|hole| (address..address + destination.len()).contains(&hole))
            {
                return false;
            }
            self.memory.copy(address, destination)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            if self.per_field {
                return 0;
            }
            let Some(count) = layout.count(sources, destination.len()) else {
                return 0;
            };
            self.calls.borrow_mut().push((usize::MAX, count));
            let mut completed = 0;
            for row in 0..sources.len() {
                for field in layout.fields() {
                    let address = sources.pointer(row).unwrap() + field.source_offset;
                    let at = row * layout.stride() + field.destination_offset;
                    if self
                        .hole
                        .is_some_and(|hole| (address..address + field.len).contains(&hole))
                        || !self
                            .memory
                            .copy(address, &mut destination[at..at + field.len])
                    {
                        return completed;
                    }
                    completed += 1;
                }
            }
            completed
        }
    }
    let run = |bytes: &[u8], hole, per_field| {
        let memory = Holed {
            memory: SliceMemory {
                base: MEMORY,
                bytes,
            },
            hole,
            per_field,
            calls: Default::default(),
        };
        let mut captured = Captured::default();
        let walk = walk(
            &memory,
            MODULE,
            MEMORY,
            true,
            Buffers {
                land: &mut captured.land,
                land_groups: &mut captured.groups,
                land_members: &mut captured.orders,
                air: &mut captured.air,
                air_groups: &mut captured.air_groups,
                air_members: &mut captured.wings,
                names: &mut captured.names,
            },
        );
        let land = captured.land[..walk.land.len()].to_vec();
        (walk, land, memory.calls.into_inner())
    };
    let bytes = fixture();
    let (clean, rows, calls) = run(&bytes, None, false);
    assert_eq!(clean.land, Outcome::Rows { len: 1 });
    assert_eq!((clean, rows.clone()), {
        let (walk, rows, _) = run(&bytes, None, true);
        (walk, rows)
    });
    for (offset, len) in HEADER_SPANS {
        assert!(!calls.contains(&(LAND + offset, len)));
    }
    let mut failures = std::collections::BTreeSet::new();
    for (offset, len) in HEADER_SPANS {
        for at in LAND + offset..LAND + offset + len {
            let (gathered, rows, _) = run(&bytes, Some(at), false);
            let (per_field, per_field_rows, _) = run(&bytes, Some(at), true);
            assert_eq!(
                (gathered, &rows),
                (per_field, &per_field_rows),
                "hole {at:#x}"
            );
            if let Outcome::Unavailable { reason } = gathered.land {
                failures.insert(reason);
            }
            let mut changed = bytes.clone();
            changed[at - MEMORY] ^= 0x81;
            let (gathered, rows, _) = run(&changed, None, false);
            let (per_field, per_field_rows, _) = run(&changed, None, true);
            assert_eq!(
                (gathered, &rows),
                (per_field, &per_field_rows),
                "byte {at:#x}"
            );
            if let Outcome::Unavailable { reason } = gathered.land {
                failures.insert(reason);
            }
        }
    }
    for reason in [
        "theatre identity unreadable",
        "theatre object vtable mismatch",
        "theatre object reference type mismatch",
        "theatre field unreadable or address overflow",
    ] {
        assert!(failures.contains(reason), "{reason}: {failures:?}");
    }
}

#[test]
fn persistent_handles_native_aliases_priorities_and_literal_bytes_are_lossless() {
    let bytes = fixture();
    let mut captured = Captured::default();
    let result = captured.read(&bytes);
    for table in [
        result.land,
        result.land_groups,
        result.land_members,
        result.air,
        result.air_groups,
        result.air_members,
    ] {
        assert_eq!(table, Outcome::Rows { len: 1 });
    }
    assert_eq!(
        captured.land[0],
        Row {
            id: handle(REFERENCE_TYPE),
            owner: 1,
            volunteers_theatre: 0,
            ordinal: 0
        }
    );
    assert_eq!(captured.groups[0].priority, -1);
    assert_eq!(captured.air[0].owner, 1);
    assert_eq!(
        captured.orders[0].orders_group,
        handle(corders_group::REFERENCE_TYPE)
    );
    assert_eq!(captured.wings[0].wing, handle(engine_native_parts::cair_wing_layout::TYPE));
    assert_eq!(captured.wings[0].group, handle(cair_group::TYPE));
    assert_eq!(
        &captured.names[..result.names_len],
        &[
            0xff, 0, 0x80, b'a', b'i', b'r', b'-', b'n', b'a', b'm', b'e'
        ]
    );
    assert_eq!(result.names_required, 11);
    assert_eq!(captured.air_groups[0].name_len, 0);
}

#[test]
fn volunteer_tag_is_independent_of_owner_and_backlinks_keep_complete_references() {
    let mut bytes = fixture();
    let mut captured = Captured::default();
    for tag in [0, 2, i32::MIN] {
        put(&mut bytes, LAND + VOLUNTEERS_THEATRE, tag.to_le_bytes());
        let result = captured.read(&bytes);
        assert_eq!(result.land, Outcome::Rows { len: 1 });
        assert_eq!(captured.land[0].owner, 1);
        assert_eq!(captured.land[0].volunteers_theatre, tag);
    }
    let root = corders_group::Root {
        pointer: ORDERS,
        handle: handle(corders_group::REFERENCE_TYPE),
        country: 1,
    };
    assert_eq!(
        read_orders_membership(
            &SliceMemory {
                base: MEMORY,
                bytes: &bytes
            },
            MODULE,
            &root
        ),
        Ok(OrdersMembership {
            orders_group: root.handle,
            theater_group: handle(ctheater_group::REFERENCE_TYPE)
        })
    );
    put(
        &mut bytes,
        ORDERS + corders_group::THEATER_GROUP,
        0_u64.to_le_bytes(),
    );
    assert_eq!(
        read_orders_membership(
            &SliceMemory {
                base: MEMORY,
                bytes: &bytes
            },
            MODULE,
            &root
        ),
        Ok(OrdersMembership {
            orders_group: root.handle,
            theater_group: 0
        })
    );
    put(&mut bytes, LAND + OWNER, 0_u32.to_le_bytes());
    assert!(matches!(
        captured.read(&bytes).land,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn the_count_probe_reports_nested_rows_and_full_name_demand() {
    let bytes = fixture();
    let result = count(&bytes);
    for table in [
        result.land,
        result.land_groups,
        result.land_members,
        result.air,
        result.air_groups,
        result.air_members,
    ] {
        assert_eq!(
            table,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }
    assert_eq!(result.names_len, 0);
    assert_eq!(result.names_required, 11);
}

#[test]
fn land_and_air_faults_are_independent_and_parent_mismatches_are_not_empty_tables() {
    let mut bytes = fixture();
    let mut captured = Captured::default();
    put(
        &mut bytes,
        LAND_GROUP + ctheater_group::THEATRE,
        0_u64.to_le_bytes(),
    );
    let result = captured.read(&bytes);
    assert!(matches!(result.land_groups, Outcome::Unavailable { .. }));
    assert_eq!(result.air_members, Outcome::Rows { len: 1 });
    let mut bytes = fixture();
    put(
        &mut bytes,
        AIR_GROUP + cair_group::THEATRE_HANDLE,
        0_u64.to_le_bytes(),
    );
    let result = captured.read(&bytes);
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
    assert!(matches!(result.air_groups, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::AIR_THEATRES + 8,
        0_u32.to_le_bytes(),
    );
    let result = captured.read(&bytes);
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
    assert!(matches!(result.air, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(&mut bytes, AIR + cair_theatre::COUNTRY, 0_u32.to_le_bytes());
    let result = captured.read(&bytes);
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
    assert!(matches!(result.air, Outcome::Unavailable { .. }));
}

#[test]
fn heap_names_are_exact_and_wrong_terminators_or_reference_types_fail() {
    let mut bytes = fixture();
    let mut captured = Captured::default();
    let name = AIR + cair_theatre::NAME;
    put(&mut bytes, name, 0x6200_u64.to_le_bytes());
    put(
        &mut bytes,
        name + msvc_string::CAPACITY,
        31_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x6200, *b"arbitrary");
    put(&mut bytes, name + msvc_string::LENGTH, 9_u64.to_le_bytes());
    let result = captured.read(&bytes);
    assert_eq!(result.air, Outcome::Rows { len: 1 });
    assert_eq!(&captured.names[3..12], b"arbitrary");
    put(&mut bytes, 0x6209, [1]);
    let result = captured.read(&bytes);
    assert!(matches!(result.air, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(&mut bytes, 0x22a0, handle(cair_group::TYPE).to_le_bytes());
    let result = captured.read(&bytes);
    assert!(matches!(result.air_members, Outcome::Unavailable { .. }));
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
}

const AREA_A: usize = 0x4000;
const AREA_B: usize = 0x4200;
const PROVINCE_A: usize = 0x4400;
const PROVINCE_B: usize = 0x4600;
const UNIT_A: usize = 0x4800;
const UNIT_B: usize = 0x4a00;

fn stationed_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    put(&mut bytes, LAND + VOLUNTEERS_THEATRE, 7_i32.to_le_bytes());
    array(&mut bytes, LAND + CONTROLLER_AREA_DATA, 0x2400, 2);
    put(&mut bytes, 0x2400, (AREA_A as u64).to_le_bytes());
    put(&mut bytes, 0x2408, (AREA_B as u64).to_le_bytes());
    for area in [AREA_A, AREA_B] {
        put(
            &mut bytes,
            area,
            ((MODULE + ccontroller_area::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        AREA_A + ccontroller_area::REPRESENTATIVE_PROVINCE,
        (PROVINCE_A as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        AREA_A + ccontroller_area::PROVINCE_DATA,
        0x2420,
        2,
    );
    put(&mut bytes, 0x2420, (PROVINCE_A as u64).to_le_bytes());
    put(&mut bytes, 0x2428, (PROVINCE_B as u64).to_le_bytes());
    for (province, id) in [(PROVINCE_A, 1_u32), (PROVINCE_B, 13_413)] {
        put(
            &mut bytes,
            province,
            ((MODULE + cprovince::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, province + cprovince::ID, id.to_le_bytes());
    }
    array(&mut bytes, LAND + STATIONED_UNIT_DATA, 0x2440, 2);
    put(&mut bytes, 0x2440, (UNIT_A as u64).to_le_bytes());
    put(&mut bytes, 0x2448, (UNIT_B as u64).to_le_bytes());
    put(
        &mut bytes,
        UNIT_A,
        ((MODULE + carmy::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT_A + cunit::HANDLE,
        ((3_u64 << 32) | 5).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT_B,
        ((MODULE + ctask_force::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(&mut bytes, UNIT_B + cunit::HANDLE, u64::MAX.to_le_bytes());
    bytes
}

fn stationed(
    bytes: &[u8],
    areas: usize,
    provinces: usize,
    units: usize,
) -> (
    StationedWalk,
    Vec<ccontroller_area::Row>,
    Vec<u32>,
    Vec<StationedUnitRow>,
) {
    let memory = SliceMemory {
        base: MEMORY,
        bytes,
    };
    let mut area_rows = vec![ccontroller_area::Row::default(); areas];
    let mut area_provinces = vec![0; provinces];
    let mut unit_rows = vec![StationedUnitRow::default(); units];
    let walk = walk_stationed(
        &memory,
        MODULE,
        MEMORY,
        StationedBuffers {
            areas_active: true,
            areas: &mut area_rows,
            area_provinces: &mut area_provinces,
            units: &mut unit_rows,
        },
    );
    (walk, area_rows, area_provinces, unit_rows)
}

#[test]
fn volunteer_theatres_keep_native_area_and_unit_order() {
    let (walk, areas, provinces, units) = stationed(&stationed_fixture(), 4, 8, 4);
    assert_eq!(walk.areas, Outcome::Rows { len: 2 });
    assert_eq!(walk.units, Outcome::Rows { len: 2 });
    assert_eq!((walk.provinces_len, walk.required_provinces), (2, 2));
    assert_eq!(
        areas[0],
        ccontroller_area::Row {
            theatre: handle(REFERENCE_TYPE),
            ordinal: 0,
            representative: Some(1),
            provinces_start: 0,
            provinces_len: 2,
        }
    );
    assert_eq!(
        areas[1],
        ccontroller_area::Row {
            theatre: handle(REFERENCE_TYPE),
            ordinal: 1,
            representative: None,
            provinces_start: 2,
            provinces_len: 0,
        }
    );
    assert_eq!(&provinces[..2], [1, 13_413]);
    assert_eq!(
        units[0],
        StationedUnitRow {
            theatre: handle(REFERENCE_TYPE),
            ordinal: 0,
            unit: (3 << 32) | 5,
        }
    );
    assert_eq!(units[1].unit, u64::MAX);
}

fn wide_area_fixture() -> Vec<u8> {
    const PROVINCES: usize = 0x8000;
    let mut bytes = stationed_fixture();
    bytes.resize(0x11000, 0);
    array(
        &mut bytes,
        AREA_A + ccontroller_area::PROVINCE_DATA,
        0x7000,
        33,
    );
    for index in 0..33_usize {
        let province = PROVINCES + index * 0x100;
        put(
            &mut bytes,
            0x7000 + index * 8,
            (province as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province,
            ((MODULE + cprovince::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province + cprovince::ID,
            (index as u32 + 1).to_le_bytes(),
        );
    }
    bytes
}

#[test]
fn area_provinces_survive_the_copy_batch_boundary_in_native_order() {
    let (walk, areas, provinces, _) = stationed(&wide_area_fixture(), 4, 64, 4);
    assert_eq!(walk.areas, Outcome::Rows { len: 2 });
    assert_eq!((walk.provinces_len, walk.required_provinces), (33, 33));
    assert_eq!(areas[0].provinces_len, 33);
    assert_eq!(&provinces[..33], (1..=33).collect::<Vec<u32>>().as_slice());
    assert_eq!(provinces[32], 33);
    let mut bytes = wide_area_fixture();
    put(
        &mut bytes,
        0x8000 + 32 * 0x100 + cprovince::ID,
        0_u32.to_le_bytes(),
    );
    let (walk, _, _, _) = stationed(&bytes, 4, 64, 4);
    assert_eq!(
        walk.areas,
        Outcome::Unavailable {
            reason: "theatre province id is zero"
        }
    );
}

#[test]
fn a_nonpositive_volunteer_key_leaves_a_theatre_out_of_scope() {
    let mut bytes = stationed_fixture();
    put(&mut bytes, LAND + VOLUNTEERS_THEATRE, 0_i32.to_le_bytes());
    let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
    assert_eq!(walk.areas, Outcome::Rows { len: 0 });
    assert_eq!(walk.units, Outcome::Rows { len: 0 });
    put(
        &mut bytes,
        LAND + VOLUNTEERS_THEATRE,
        (-1_i32).to_le_bytes(),
    );
    let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
    assert_eq!(walk.areas, Outcome::Rows { len: 0 });
}

#[test]
fn stationed_unit_faults_stay_independent_of_areas() {
    let mut bytes = stationed_fixture();
    put(&mut bytes, 0x2440, 0_u64.to_le_bytes());
    let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
    assert_eq!(walk.areas, Outcome::Rows { len: 2 });
    assert_eq!(
        walk.units,
        Outcome::Unavailable {
            reason: "stationed unit pointer unreadable or null"
        }
    );
}

#[test]
fn stationed_identity_and_header_faults_refuse_by_name() {
    for (address, value, areas, units) in [
        (
            0x2400,
            0_u64,
            Some("theatre controller area pointer unreadable or null"),
            None,
        ),
        (
            0x2428,
            0,
            Some("controller area province pointer unreadable or null"),
            None,
        ),
        (
            AREA_B,
            0x1234,
            Some("theatre controller area vtable mismatch"),
            None,
        ),
        (
            PROVINCE_A,
            0x1234,
            Some("theatre province vtable mismatch"),
            None,
        ),
        (
            UNIT_A,
            0x1234,
            None,
            Some("stationed unit is neither a division nor a task force"),
        ),
        (
            UNIT_A + cunit::HANDLE,
            0,
            None,
            Some("stationed unit has a null reference handle"),
        ),
    ] {
        let mut bytes = stationed_fixture();
        put(&mut bytes, address, value.to_le_bytes());
        let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
        if let Some(reason) = areas {
            assert_eq!(walk.areas, Outcome::Unavailable { reason }, "{address:#x}");
        }
        if let Some(reason) = units {
            assert_eq!(walk.units, Outcome::Unavailable { reason }, "{address:#x}");
        }
    }
    let mut bytes = stationed_fixture();
    put(&mut bytes, PROVINCE_B + cprovince::ID, 0_u32.to_le_bytes());
    let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
    assert_eq!(
        walk.areas,
        Outcome::Unavailable {
            reason: "theatre province id is zero"
        }
    );
    let mut bytes = stationed_fixture();
    put(
        &mut bytes,
        AREA_A + ccontroller_area::PROVINCE_COUNT,
        9_u32.to_le_bytes(),
    );
    let (walk, _, _, _) = stationed(&bytes, 4, 8, 4);
    assert_eq!(
        walk.areas,
        Outcome::Unavailable {
            reason: "controller area province vector unreadable or invalid"
        }
    );
}

#[derive(Default)]
struct GrowingTables {
    land: Vec<Row>,
    land_groups: Vec<ctheater_group::Row>,
    land_members: Vec<ctheater_group::MemberRow>,
    air: Vec<cair_theatre::Row>,
    air_groups: Vec<cair_group::Row>,
    air_members: Vec<cair_group::MemberRow>,
    names: Vec<u8>,
}
impl GrowingTables {
    fn capture(&mut self, memory: &impl Memory) -> Walk {
        walk_buffers(
            memory,
            MODULE,
            MEMORY,
            true,
            Storage {
                land: Buffer::Growing(&mut self.land),
                land_groups: Buffer::Growing(&mut self.land_groups),
                land_members: Buffer::Growing(&mut self.land_members),
                air: Buffer::Growing(&mut self.air),
                air_groups: Buffer::Growing(&mut self.air_groups),
                air_members: Buffer::Growing(&mut self.air_members),
                names: Buffer::Growing(&mut self.names),
            },
        )
    }
}
fn many_theatres(countries: usize, members: usize) -> Vec<u8> {
    let first = 0x30000;
    let mut bytes = vec![0; first + countries * 0x1000];
    array(
        &mut bytes,
        MEMORY + cgame_state::COUNTRIES,
        0x2000,
        countries as u32 + 1,
    );
    array(
        &mut bytes,
        MEMORY + cgame_state::COUNTRY_LINKS,
        0x12000,
        countries as u32 + 1,
    );
    for index in 0..countries {
        let slot = index + 1;
        let country = first + index * 0x1000;
        let land = country + 0x200;
        let group = country + 0x400;
        let orders = country + 0x500;
        let air = country + 0x700;
        let air_group = country + 0x800;
        put(
            &mut bytes,
            0x2000 + slot * 8,
            (country as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x12000 + slot * 4, (slot as u32).to_le_bytes());
        put(
            &mut bytes,
            country,
            ((MODULE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            (slot as u32).to_le_bytes(),
        );
        array(&mut bytes, country + ccountry::THEATRES, country + 0x900, 1);
        put(&mut bytes, country + 0x900, (land as u64).to_le_bytes());
        array(
            &mut bytes,
            country + ccountry::AIR_THEATRES,
            country + 0x908,
            1,
        );
        put(&mut bytes, country + 0x908, (air as u64).to_le_bytes());
        object(&mut bytes, land, VTABLE_RVA, REFERENCE_TYPE);
        put(&mut bytes, land + OWNER, (slot as u32).to_le_bytes());
        array(&mut bytes, land + THEATER_GROUP_DATA, country + 0x910, 1);
        put(&mut bytes, country + 0x910, (group as u64).to_le_bytes());
        object(
            &mut bytes,
            group,
            ctheater_group::VTABLE_RVA,
            ctheater_group::REFERENCE_TYPE,
        );
        put(
            &mut bytes,
            group + ctheater_group::THEATRE,
            (land as u64).to_le_bytes(),
        );
        string(&mut bytes, group + ctheater_group::NAME, b"land");
        array(
            &mut bytes,
            group + ctheater_group::ORDERS_GROUP_DATA,
            country + 0x920,
            members as u32,
        );
        object(
            &mut bytes,
            orders,
            corders_group::VTABLE_RVA,
            corders_group::REFERENCE_TYPE,
        );
        put(
            &mut bytes,
            orders + corders_group::THEATRE,
            (land as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            orders + corders_group::THEATER_GROUP,
            (group as u64).to_le_bytes(),
        );
        object(
            &mut bytes,
            air,
            cair_theatre::VTABLE_RVA,
            cair_theatre::TYPE,
        );
        put(
            &mut bytes,
            air + cair_theatre::COUNTRY,
            (slot as u32).to_le_bytes(),
        );
        string(&mut bytes, air + cair_theatre::NAME, b"air");
        array(&mut bytes, air + cair_theatre::GROUPS, country + 0x918, 1);
        put(
            &mut bytes,
            country + 0x918,
            (air_group as u64).to_le_bytes(),
        );
        object(
            &mut bytes,
            air_group,
            cair_group::VTABLE_RVA,
            cair_group::TYPE,
        );
        put(
            &mut bytes,
            air_group + cair_group::THEATRE_HANDLE,
            handle(cair_theatre::TYPE).to_le_bytes(),
        );
        string(&mut bytes, air_group + cair_group::NAME, b"group");
        array(
            &mut bytes,
            air_group + cair_group::WINGS,
            country + 0xc00,
            members as u32,
        );
        for member in 0..members {
            put(
                &mut bytes,
                country + 0x920 + member * 8,
                (orders as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + 0xc00 + member * 8,
                handle(engine_native_parts::cair_wing_layout::TYPE).to_le_bytes(),
            );
        }
    }
    bytes
}
#[test]
fn worker_theatres_grow_for_every_country_and_later_nested_demand_without_rereads() {
    struct Once<'a> {
        memory: SliceMemory<'a>,
        headers: std::cell::RefCell<std::collections::BTreeSet<usize>>,
    }
    impl Memory for Once<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if output.len() == 16
                && address >= 0x30000
                && matches!(
                    (address - 0x30000) % 0x1000,
                    0x168 | 0x180 | 0x2c8 | 0x448 | 0x740 | 0x878
                )
            {
                assert!(
                    self.headers.borrow_mut().insert(address),
                    "header reread at {address:x}"
                );
            }
            self.memory.copy(address, output)
        }
    }
    let mut captured = GrowingTables::default();
    for members in [3, 40] {
        let bytes = many_theatres(12, members);
        let memory = Once {
            memory: SliceMemory {
                base: MEMORY,
                bytes: &bytes,
            },
            headers: Default::default(),
        };
        let walked = captured.capture(&memory);
        for outcome in [
            walked.land,
            walked.land_groups,
            walked.air,
            walked.air_groups,
        ] {
            assert_eq!(outcome, Outcome::Rows { len: 12 });
        }
        for outcome in [walked.land_members, walked.air_members] {
            assert_eq!(outcome, Outcome::Rows { len: 12 * members });
        }
        assert_eq!(walked.names_len, 12 * 12);
        assert_eq!(walked.names_required, walked.names_len);
        assert_eq!(
            captured.land_members[12 * members - 1].ordinal,
            members as u32 - 1
        );
    }
}

#[test]
fn unit_theatres_resolve_only_their_roster_country_theatres() {
    let mut bytes = fixture();
    let memory = SliceMemory {
        base: MEMORY,
        bytes: &bytes,
    };
    let mut theatres = UnitTheatres::default();
    assert_eq!(theatres.handle(&memory, MODULE, COUNTRY_POINTER, 0), Ok(0));
    assert_eq!(
        theatres.handle(&memory, MODULE, COUNTRY_POINTER, LAND),
        Ok(handle(REFERENCE_TYPE))
    );
    assert_eq!(
        theatres.handle(&memory, MODULE, COUNTRY_POINTER, LAND_GROUP),
        Err("unit theatre is not one of its roster country's theatres")
    );
    for theatre in [0, LAND, LAND_GROUP] {
        assert_eq!(
            unit_theatre(&memory, MODULE, COUNTRY_POINTER, theatre),
            theatres.handle(&memory, MODULE, COUNTRY_POINTER, theatre)
        );
    }
    put(&mut bytes, LAND, 0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: MEMORY,
        bytes: &bytes,
    };
    theatres.clear();
    assert_eq!(
        theatres.handle(&memory, MODULE, COUNTRY_POINTER, LAND),
        Err("theatre object vtable mismatch")
    );
    assert_eq!(
        unit_theatre(&memory, MODULE, COUNTRY_POINTER, LAND),
        Err("theatre object vtable mismatch")
    );
    assert_eq!(theatres.handle(&memory, MODULE, COUNTRY_POINTER, 0), Ok(0));
}
