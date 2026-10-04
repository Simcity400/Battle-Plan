use super::*;
use crate::{cgame_state, memory::SliceMemory};

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

fn object(bytes: &mut [u8], address: usize, vtable: usize, kind: u32) {
    put(bytes, address, ((MODULE + vtable) as u64).to_le_bytes());
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
        ((MODULE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
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
    // Raw reference 1 and the current country reference 2 resolve to one
    // object. Preserve the native theatre owner rather than replacing it.
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
    put(&mut bytes, 0x22a0, handle(cair_wing::TYPE).to_le_bytes());
    bytes
}

#[derive(Default)]
struct Captured {
    land: [Row; 2],
    groups: [ctheater_group::Row; 2],
    orders: [ctheater_group::MemberRow; 2],
    air: [cair_theatre::Row; 2],
    air_groups: [cair_group::Row; 2],
    wings: [cair_group::MemberRow; 2],
    names: [u8; 32],
}

impl Captured {
    fn read(&mut self, bytes: &[u8], row_capacity: usize, name_capacity: usize) -> Walk {
        walk(
            &SliceMemory {
                base: MEMORY,
                bytes,
            },
            MODULE,
            MEMORY,
            true,
            Buffers {
                land: &mut self.land[..row_capacity],
                land_groups: &mut self.groups[..row_capacity],
                land_members: &mut self.orders[..row_capacity],
                air: &mut self.air[..row_capacity],
                air_groups: &mut self.air_groups[..row_capacity],
                air_members: &mut self.wings[..row_capacity],
                names: &mut self.names[..name_capacity],
            },
        )
    }
}

/// The gathered theatre header gives the per-field walk's rows, arenas and
/// first failure: for a hole at every byte of the header spans, and for a
/// changed byte at every one of them. One guarded record replaces the
/// three field reads when it completes.
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
            // One call for the whole record, as the guarded copy makes.
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
    // The header's three spans are one record request, not three reads.
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
    let result = captured.read(&bytes, 2, 32);
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
    assert_eq!(captured.wings[0].wing, handle(cair_wing::TYPE));
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
        let result = captured.read(&bytes, 2, 32);
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
        captured.read(&bytes, 2, 32).land,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn capacities_omit_whole_rows_and_report_nested_rows_and_full_name_demand() {
    let bytes = fixture();
    let mut captured = Captured::default();
    let result = captured.read(&bytes, 0, 0);
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
    let result = captured.read(&bytes, 2, 2);
    assert_eq!(
        result.land_groups,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(
        result.air,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(result.air_groups, Outcome::Rows { len: 1 });
    assert_eq!(result.air_members, Outcome::Rows { len: 1 });
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
    let result = captured.read(&bytes, 2, 32);
    assert!(matches!(result.land_groups, Outcome::Unavailable { .. }));
    assert_eq!(result.air_members, Outcome::Rows { len: 1 });
    let mut bytes = fixture();
    put(
        &mut bytes,
        AIR_GROUP + cair_group::THEATRE_HANDLE,
        0_u64.to_le_bytes(),
    );
    let result = captured.read(&bytes, 2, 32);
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
    assert!(matches!(result.air_groups, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::AIR_THEATRES + 8,
        0_u32.to_le_bytes(),
    );
    let result = captured.read(&bytes, 2, 32);
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
    assert!(matches!(result.air, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(&mut bytes, AIR + cair_theatre::COUNTRY, 0_u32.to_le_bytes());
    let result = captured.read(&bytes, 2, 32);
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
    let result = captured.read(&bytes, 2, 32);
    assert_eq!(result.air, Outcome::Rows { len: 1 });
    assert_eq!(&captured.names[3..12], b"arbitrary");
    put(&mut bytes, 0x6209, [1]);
    let result = captured.read(&bytes, 2, 32);
    assert!(matches!(result.air, Outcome::Unavailable { .. }));
    let mut bytes = fixture();
    put(&mut bytes, 0x22a0, handle(cair_group::TYPE).to_le_bytes());
    let result = captured.read(&bytes, 2, 32);
    assert!(matches!(result.air_members, Outcome::Unavailable { .. }));
    assert_eq!(result.land_members, Outcome::Rows { len: 1 });
}

const AREA_A: usize = 0x4000;
const AREA_B: usize = 0x4200;
const PROVINCE_A: usize = 0x4400;
const PROVINCE_B: usize = 0x4600;
const UNIT_A: usize = 0x4800;
const UNIT_B: usize = 0x4a00;

/// The single land theatre becomes a volunteer container with two areas and
/// two stationed units; the second area is empty and has no representative.
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
            ((MODULE + ccontroller_area::VTABLE_RVA) as u64).to_le_bytes(),
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
            ((MODULE + cprovince::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, province + cprovince::ID, id.to_le_bytes());
    }
    array(&mut bytes, LAND + STATIONED_UNIT_DATA, 0x2440, 2);
    put(&mut bytes, 0x2440, (UNIT_A as u64).to_le_bytes());
    put(&mut bytes, 0x2448, (UNIT_B as u64).to_le_bytes());
    put(
        &mut bytes,
        UNIT_A,
        ((MODULE + carmy::VTABLE_RVA) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT_A + cunit::HANDLE,
        ((3_u64 << 32) | 5).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT_B,
        ((MODULE + ctask_force::VTABLE_RVA) as u64).to_le_bytes(),
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

/// The single area holds thirty-three provinces, one past the copy batch.
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
            ((MODULE + cprovince::VTABLE_RVA) as u64).to_le_bytes(),
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
    // The thirty-third element is the one a loop stopping at its first
    // window would silently drop.
    assert_eq!(provinces[32], 33);
    // A fault inside the second window still refuses the whole table.
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
fn stationed_capacity_omits_whole_rows_and_keeps_faults_independent() {
    let (walk, areas, _, _) = stationed(&stationed_fixture(), 4, 1, 1);
    assert_eq!(
        walk.areas,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(areas[0].ordinal, 1);
    assert_eq!((walk.provinces_len, walk.required_provinces), (0, 2));
    assert_eq!(
        walk.units,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
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
    // A count above its own capacity is a torn vector header.
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
