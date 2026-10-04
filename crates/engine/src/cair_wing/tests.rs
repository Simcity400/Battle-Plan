use super::*;
use crate::memory::SliceMemory;

const GS: usize = 0x1000;
const COUNTRY: usize = 0x2000;
const COUNTRIES: usize = 0x4000;
const LINKS: usize = 0x4100;
const MANAGER: usize = 0x5000;
const OWNERS: usize = 0x6000;
const OWNER: usize = 0x7000;
const POOLS: usize = 0x8000;
const POOL_OBJECT: usize = 0x9000;
const WINGS: usize = 0xa000;
const WING_A: usize = 0xb000;
const WING_B: usize = 0xc000;
const REGION: usize = 0xd000;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xe000];
    array(&mut bytes, GS + 0x310, COUNTRIES, 4);
    array(&mut bytes, GS + 0x340, LINKS, 4);
    p64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    p64(&mut bytes, COUNTRIES + 24, COUNTRY as u64);
    p32(&mut bytes, LINKS + 4, 1);
    p32(&mut bytes, LINKS + 12, 1);
    p64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
    p32(&mut bytes, COUNTRY + 8, 3);
    p64(&mut bytes, GS + 0x690, MANAGER as u64);
    p64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA as u64,
    );
    array(&mut bytes, MANAGER + 0x30, OWNERS, 4);
    p64(&mut bytes, OWNERS + 8, OWNER as u64);
    p64(&mut bytes, OWNER, cstrategic_air::VTABLE_RVA as u64);
    p32(&mut bytes, OWNER + 0x90, 3);
    array(&mut bytes, OWNER + 0x58, POOLS, 1);
    p64(&mut bytes, POOLS, POOL_OBJECT as u64);
    p64(&mut bytes, POOL_OBJECT, cair_wing_pool::VTABLE_RVA as u64);
    p64(&mut bytes, POOL_OBJECT + 0x18, 0xfeed_beef_0000_0041);
    p64(&mut bytes, POOL_OBJECT + 0x40, OWNER as u64);
    array(&mut bytes, POOL_OBJECT + 0x28, WINGS, 2);
    p64(&mut bytes, WINGS, WING_A as u64);
    p64(&mut bytes, WINGS + 8, WING_B as u64);
    for (wing, id) in [(WING_A, u64::MAX), (WING_B, 69)] {
        p64(&mut bytes, wing, VTABLE_RVA as u64);
        p64(&mut bytes, wing + 0x10, SECONDARY_VTABLE_RVA as u64);
        p64(&mut bytes, wing + 0x18, id);
        p64(&mut bytes, wing + 0x38, POOL_OBJECT as u64);
        p32(&mut bytes, wing + 0x7c, i32::MIN as u32);
        p32(&mut bytes, wing + 0xa0, 0x8000_0800);
        p32(&mut bytes, wing + 0x9c4, i32::MIN as u32);
    }
    p64(&mut bytes, WING_A + 0x48, 0xffff_ffff_0000_0041);
    p64(&mut bytes, WING_A + 0xc0, REGION as u64);
    p64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA as u64);
    p64(
        &mut bytes,
        REGION + 8,
        cstrategic_region::SECONDARY_VTABLE_RVA as u64,
    );
    p32(&mut bytes, REGION + 0x58, u32::MAX);
    bytes
}
fn capture(bytes: &[u8], rows: &mut [Row]) -> Outcome {
    walk(&SliceMemory { base: 0, bytes }, 0, GS, rows)
}

struct Traced<'a> {
    bytes: &'a [u8],
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}

impl Memory for Traced<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

#[test]
fn shared_walk_three_capacities_match_independent_walks_with_one_read_sequence() {
    let bytes = fixture();
    for map_capacity in 0..=3 {
        for source_capacity in 0..=3 {
            for naval_capacity in 0..=3 {
                let mut expected_map = [Row::default(); 3];
                let map = capture(&bytes, &mut expected_map[..map_capacity]);
                let mut expected_sources = [Row::default(); 3];
                let sources = capture(&bytes, &mut expected_sources[..source_capacity]);
                let mut expected_naval = [Row::default(); 3];
                let naval = capture(&bytes, &mut expected_naval[..naval_capacity]);
                let traced = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: None,
                };
                let mut map_rows = [Row::default(); 3];
                let mut source_rows = [Source::default(); 3];
                let mut visited = Vec::new();
                let result = walk_shared(
                    &traced,
                    0,
                    GS,
                    &mut map_rows[..map_capacity],
                    &mut source_rows[..source_capacity],
                    naval_capacity,
                    |pointer, pool, row, admitted| {
                        visited.push((
                            Source {
                                pointer,
                                pool,
                                row: *row,
                            },
                            admitted,
                        ));
                    },
                );
                assert_eq!(
                    result,
                    SharedWalk {
                        map,
                        sources,
                        naval
                    }
                );
                assert_eq!(map_rows[..map.len()], expected_map[..map.len()]);
                for (index, source) in source_rows[..sources.len()].iter().enumerate() {
                    assert_eq!(source.row, expected_sources[index]);
                    assert_eq!(source.pool, POOL_OBJECT);
                    assert_eq!(source.pointer, [WING_A, WING_B][index]);
                }
                let maximum = map_capacity.max(source_capacity).max(naval_capacity);
                assert_eq!(visited.len(), maximum.min(2));
                for (index, (source, admitted)) in visited.iter().enumerate() {
                    assert_eq!(source.pointer, [WING_A, WING_B][index]);
                    assert_eq!(
                        *admitted,
                        Admitted {
                            map: index < map_capacity,
                            sources: index < source_capacity,
                            naval: index < naval_capacity
                        }
                    );
                    if admitted.naval {
                        assert_eq!(source.row, expected_naval[index]);
                    }
                    if admitted.sources {
                        assert_eq!(*source, source_rows[index]);
                    }
                }
                let reference = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: None,
                };
                walk(&reference, 0, GS, &mut [Row::default(); 3][..maximum]);
                assert_eq!(
                    traced.reads.borrow().as_slice(),
                    reference.reads.borrow().as_slice(),
                    "capacities map {map_capacity}, source {source_capacity}, naval {naval_capacity}"
                );
            }
        }
    }
}

#[test]
fn shared_walk_leaf_faults_preserve_each_independently_capped_consumer() {
    for faulty_field in [
        WING_B,
        WING_B + MISSION + air_mission::ENABLED_MASK,
        WING_B + COUNTRY_OVERRIDE,
    ] {
        let bytes = fixture();
        for map_capacity in 0..=2 {
            for source_capacity in 0..=2 {
                for naval_capacity in 0..=2 {
                    let memory = Traced {
                        bytes: &bytes,
                        reads: Default::default(),
                        fault: Some(faulty_field),
                    };
                    let mut visits = Vec::new();
                    let shared = walk_shared(
                        &memory,
                        0,
                        GS,
                        &mut [Row::default(); 2][..map_capacity],
                        &mut [Source::default(); 2][..source_capacity],
                        naval_capacity,
                        |pointer, _, row, admitted| visits.push((pointer, *row, admitted)),
                    );
                    let reference = Traced {
                        bytes: &bytes,
                        reads: Default::default(),
                        fault: Some(faulty_field),
                    };
                    let map = walk(&reference, 0, GS, &mut [Row::default(); 2][..map_capacity]);
                    let sources = walk(
                        &reference,
                        0,
                        GS,
                        &mut [Row::default(); 2][..source_capacity],
                    );
                    let naval = walk(
                        &reference,
                        0,
                        GS,
                        &mut [Row::default(); 2][..naval_capacity],
                    );
                    assert_eq!(
                        shared,
                        SharedWalk {
                            map,
                            sources,
                            naval
                        }
                    );
                    assert_eq!(
                        visits.len(),
                        usize::from(map_capacity.max(source_capacity).max(naval_capacity) != 0)
                    );
                    if let Some((pointer, _, admitted)) = visits.first() {
                        assert_eq!(*pointer, WING_A);
                        assert_eq!(
                            *admitted,
                            Admitted {
                                map: map_capacity != 0,
                                sources: source_capacity != 0,
                                naval: naval_capacity != 0
                            }
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn shared_walk_naval_only_survivor_keeps_later_mandatory_roster_failure_precedence() {
    for later_fault in 0..3 {
        let mut bytes = fixture();
        p64(&mut bytes, WING_B, 0);
        match later_fault {
            1 => {
                array(&mut bytes, POOL_OBJECT + cair_wing_pool::WINGS, WINGS, 3);
                p64(&mut bytes, WINGS + 16, 0);
            }
            2 => {
                // A second pool exists only after the bad wing. Its null
                // identity is a mandatory header failure for capped naval.
                array(&mut bytes, OWNER + cstrategic_air::POOLS, POOLS, 2);
                p64(&mut bytes, POOLS + 8, 0);
            }
            _ => {}
        }
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: None,
        };
        let mut visits = Vec::new();
        let result = walk_shared(
            &memory,
            0,
            GS,
            &mut [Row::default(); 2],
            &mut [Source::default(); 3],
            1,
            |pointer, _, _, admitted| visits.push((pointer, admitted)),
        );
        let leaf_failure = Outcome::Unavailable {
            reason: "air wing primary vtable mismatch",
        };
        assert_eq!(result.map, leaf_failure);
        assert_eq!(result.sources, leaf_failure);
        let naval = match later_fault {
            1 => Outcome::Unavailable {
                reason: "air wing roster contains null wing",
            },
            2 => Outcome::Unavailable {
                reason: "air wing pool vtable mismatch",
            },
            _ => Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
        };
        assert_eq!(result.naval, naval);
        assert_eq!(
            visits,
            [(
                WING_A,
                Admitted {
                    map: true,
                    sources: true,
                    naval: true
                }
            )]
        );
        assert!(memory.reads.borrow().contains(&(WINGS + 8, 8)));
        if later_fault == 1 {
            assert!(memory.reads.borrow().contains(&(WINGS + 16, 8)));
        }
        if later_fault == 2 {
            assert!(memory.reads.borrow().contains(&(POOLS + 8, 8)));
        }
    }
}

#[test]
fn shared_walk_full_naval_lane_does_not_hide_map_or_source_only_admissions() {
    let bytes = fixture();
    for (map_capacity, source_capacity, naval_capacity) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
        let mut visited = Vec::new();
        let result = walk_shared(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut [Row::default(); 2][..map_capacity],
            &mut [Source::default(); 2][..source_capacity],
            naval_capacity,
            |pointer, _, row, admitted| visited.push((pointer, row.id, admitted)),
        );
        assert_eq!(visited.len(), 2);
        assert_eq!(
            visited[1],
            (
                WING_B,
                69,
                Admitted {
                    map: map_capacity == 2,
                    sources: source_capacity == 2,
                    naval: naval_capacity == 2
                }
            )
        );
        assert_eq!(result.naval.len(), naval_capacity);
    }
}

#[test]
fn shared_walk_mandatory_roster_and_header_faults_affect_all_capped_consumers() {
    for fault in [
        WINGS + 8,
        POOL_OBJECT + cair_wing_pool::WINGS + 12,
        POOL_OBJECT + cair_wing_pool::OWNER,
        OWNERS + 16,
    ] {
        let bytes = fixture();
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(fault),
        };
        let result = walk_shared(&memory, 0, GS, &mut [], &mut [], 0, |_, _, _, _| {
            panic!("no capacity")
        });
        assert!(matches!(result.map, Outcome::Unavailable { .. }));
        assert_eq!(result.map, result.sources);
        assert_eq!(result.map, result.naval);
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|(address, _)| *address == WING_A || *address == WING_B)
        );
    }
}

#[test]
fn shared_walk_each_pass_is_fresh_and_source_visits_do_not_depend_on_map_capacity() {
    let mut bytes = fixture();
    let mut first = [Source::default(); 2];
    let a = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut [],
        &mut first,
        1,
        |_, _, _, _| {},
    );
    assert_eq!(a.sources, Outcome::Rows { len: 2 });
    assert_eq!(
        a.map,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    p32(&mut bytes, WING_B + PLANE_COUNT, 17);
    let mut second = [Source::default(); 2];
    let mut visited = Vec::new();
    let b = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut [],
        &mut second,
        1,
        |pointer, pool, row, admitted| {
            if admitted.sources {
                visited.push(Source {
                    pointer,
                    pool,
                    row: *row,
                });
            }
        },
    );
    assert_eq!(a, b);
    assert_eq!(visited, second);
    assert_eq!(first[0], second[0]);
    assert_ne!(first[1], second[1]);
    assert_eq!(second[1].row.planes, 17);
}

#[test]
fn shared_walk_projection_overflow_and_later_fault_preserve_first_failure() {
    let mut projection = Projection::new(0);
    projection.unread_rows = u32::MAX;
    projection.omit();
    projection.fail("later roster fault");
    assert_eq!(
        projection.outcome(),
        Outcome::Unavailable {
            reason: "air wing unread row count overflow"
        }
    );
    assert!(!projection.needs_row());
    projection.omit();
    assert_eq!(projection.unread_rows, u32::MAX);
}

/// The ordinal follows the roster, pools first, not the handles, and a
/// wing a full consumer omits still takes its place.
#[test]
fn pool_ordinals_follow_pools_then_wings() {
    const POOL_B: usize = 0x9800;
    const WINGS_B: usize = 0xa800;
    const WING_C: usize = 0xe000;
    let mut bytes = fixture();
    bytes.resize(0xf000, 0);
    // The second pool is listed first and holds wing C, handle 5.
    array(&mut bytes, OWNER + cstrategic_air::POOLS, POOLS, 2);
    p64(&mut bytes, POOLS, POOL_B as u64);
    p64(&mut bytes, POOLS + 8, POOL_OBJECT as u64);
    p64(&mut bytes, POOL_B, cair_wing_pool::VTABLE_RVA as u64);
    p64(&mut bytes, POOL_B + 0x18, 0xfeed_beef_0000_0042);
    p64(&mut bytes, POOL_B + 0x40, OWNER as u64);
    array(&mut bytes, POOL_B + 0x28, WINGS_B, 1);
    p64(&mut bytes, WINGS_B, WING_C as u64);
    p64(&mut bytes, WING_C, VTABLE_RVA as u64);
    p64(&mut bytes, WING_C + 0x10, SECONDARY_VTABLE_RVA as u64);
    p64(&mut bytes, WING_C + 0x18, 5);
    p64(&mut bytes, WING_C + 0x38, POOL_B as u64);
    let mut rows = [Row::default(); 3];
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 3 });
    assert_eq!(
        rows.map(|row| (row.id, row.air_base, row.pool_ordinal)),
        [
            (5, 0xfeed_beef_0000_0042, 0),
            (u64::MAX, 0xfeed_beef_0000_0041, 1),
            (69, 0xfeed_beef_0000_0041, 2),
        ]
    );
    // A map with room for one row: the source census still numbers the
    // wings after it by the roster.
    let mut map = [Row::default(); 1];
    let mut sources = [Source::default(); 3];
    let walked = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut map,
        &mut sources,
        0,
        |_, _, _, _| {},
    );
    assert_eq!(
        walked.map,
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    assert_eq!(sources.map(|source| source.row.pool_ordinal), [0, 1, 2]);
}

#[test]
fn raw_handles_signed_counts_alias_owner_and_nullable_region_survive() {
    let mut bytes = fixture();
    p32(&mut bytes, WING_A + ELIGIBILITY_COUNTER, i32::MIN as u32);
    p32(&mut bytes, WING_A + ELIGIBILITY_LIMIT, i32::MAX as u32);
    bytes[WING_A + MISSION + air_mission::ACTIVE] = 1;
    bytes[WING_B + ELIGIBILITY_VETO] = 1;
    let mut rows = [Row::default(); 2];
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(
        rows[0],
        Row {
            id: u64::MAX,
            owner: 3,
            pool_ordinal: 0,
            country_override: i32::MIN,
            air_base: 0xfeed_beef_0000_0041,
            base_override: 0xffff_ffff_0000_0041,
            planes: i32::MIN,
            mission_mask: 0x8000_0800,
            mission_region: Some(u32::MAX),
            base_range_raw: 0,
            ace: 0,
            mission_active: 1,
            eligibility_veto: 0,
            eligibility_counter: i32::MIN,
            eligibility_limit: i32::MAX,
            attached_orders_group: None,
        }
    );
    assert_eq!(rows[1].id, 69); // One zero handle word is not a null handle.
    assert_eq!(rows[1].pool_ordinal, 1);
    assert_eq!((rows[1].mission_active, rows[1].eligibility_veto), (0, 1));
    assert_eq!(rows[1].mission_region, None);
    p32(&mut bytes, REGION + 0x58, 0);
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].mission_region, Some(0));
    // A tactical wing records a null group as zero, and a group that does
    // not authenticate as unknown without failing the census.
    p32(
        &mut bytes,
        WING_A + MISSION + air_mission::ENABLED_MASK,
        TACTICAL_MISSION,
    );
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].attached_orders_group, Some(0));
    p64(&mut bytes, WING_A + ATTACHED_ORDERS_GROUP, 0xd000);
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].attached_orders_group, None);
}

#[test]
fn capacity_counts_omitted_wings_but_does_not_hide_a_broken_roster() {
    let mut bytes = fixture();
    let mut rows = [Row::default(); 1];
    p64(&mut bytes, WING_B + 0xc0, u64::MAX);
    assert_eq!(
        capture(&bytes, &mut rows),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    let mut all = [Row::default(); 2];
    assert!(matches!(
        capture(&bytes, &mut all),
        Outcome::Unavailable { .. }
    ));
    p64(&mut bytes, WINGS + 8, 0);
    assert_eq!(
        capture(&bytes, &mut rows),
        Outcome::Unavailable {
            reason: "air wing roster contains null wing"
        }
    );
    p64(&mut bytes, WINGS + 8, WING_B as u64);
    p32(&mut bytes, POOL_OBJECT + 0x34, 3);
    assert_eq!(
        capture(&bytes, &mut rows),
        Outcome::Unavailable {
            reason: "air wing roster unreadable or invalid"
        }
    );
}

#[test]
fn bad_identity_links_and_missing_roots_never_become_empty_air() {
    for (at, value, reason) in [
        (WING_A, 0, "air wing primary vtable mismatch"),
        (WING_A + 0x18, 0, "non-null air wing has null handle"),
        (WING_A + 0x38, 1, "air wing pool differs from roster"),
        (
            POOL_OBJECT + 0x40,
            1,
            "air wing pool owner differs from roster",
        ),
        (
            REGION + 8,
            0,
            "air wing mission region secondary vtable mismatch",
        ),
        (GS + 0x690, 0, "strategic air manager vtable mismatch"),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, at, value);
        assert_eq!(
            capture(&bytes, &mut [Row::default(); 2]),
            Outcome::Unavailable { reason }
        );
    }
    let mut bytes = fixture();
    p64(&mut bytes, OWNERS + 8, 0);
    p64(&mut bytes, OWNERS + 24, OWNER as u64);
    assert_eq!(
        capture(&bytes, &mut [Row::default(); 2]),
        Outcome::Unavailable {
            reason: "strategic air owner differs from canonical country slot"
        }
    );
    p64(&mut bytes, OWNERS + 24, 0);
    assert_eq!(
        capture(&bytes, &mut [Row::default(); 2]),
        Outcome::Rows { len: 0 }
    );
    p32(&mut bytes, MANAGER + 0x3c, 0);
    assert_eq!(
        capture(&bytes, &mut [Row::default(); 2]),
        Outcome::Unavailable {
            reason: "strategic air manager has no country slots"
        }
    );
}

#[test]
fn shared_extensions_visit_only_authenticated_emitted_rows() {
    let bytes = fixture();
    let mut count = 0;
    let outcome = walk_with(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut [Row::default(); 1],
        |wing, pool, row| {
            count += 1;
            assert_eq!((wing, pool, row.id), (WING_A, POOL_OBJECT, u64::MAX));
        },
    );
    assert_eq!(count, 1);
    assert_eq!(
        outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    let mut corrupt = bytes;
    p64(&mut corrupt, WING_A + 0x38, 0);
    let outcome = walk_with(
        &SliceMemory {
            base: 0,
            bytes: &corrupt,
        },
        0,
        GS,
        &mut [Row::default(); 2],
        |_, _, _| panic!("invalid wing must not reach an extension"),
    );
    assert!(matches!(outcome, Outcome::Unavailable { .. }));
}

#[test]
fn cumulative_extension_preserves_native_width_and_reports_faults() {
    let mut bytes = fixture();
    p32(&mut bytes, WING_A + AIR_ACCIDENTS, i32::MIN as u32);
    p64(&mut bytes, WING_A + EXPERIENCE, i64::MAX as u64);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let row = Row {
        id: u64::MAX,
        ..Row::default()
    };
    assert_eq!(
        read_cumulative(&memory, WING_A, &row),
        Ok(CumulativeRow {
            id: u64::MAX,
            accidents: i32::MIN,
            experience: i64::MAX,
        })
    );
    let broken = SliceMemory {
        base: 0,
        bytes: &bytes[..WING_A + EXPERIENCE + 7],
    };
    assert!(read_cumulative(&broken, WING_A, &row).is_err());
    assert!(read_cumulative(&memory, usize::MAX - AIR_ACCIDENTS, &row).is_err());
}

#[test]
fn equipment_extension_preserves_duplicate_variants_zero_and_signed_amounts() {
    let mut bytes = fixture();
    const ENTRIES: usize = 0xd000;
    const VARIANT: usize = 0xd100;
    let variant = (u64::from(u32::MAX) << 32) | 70;
    p64(
        &mut bytes,
        WING_A + EQUIPMENT_POOL,
        crate::cequipment_variant_pool::VTABLE_RVA as u64,
    );
    array(&mut bytes, WING_A + EQUIPMENT_BANK, ENTRIES, 3);
    p64(
        &mut bytes,
        VARIANT,
        crate::cequipment_variant::VTABLE_RVA as u64,
    );
    p64(&mut bytes, VARIANT + 8, variant);
    for (index, amount) in [0_i64, i64::MIN, i64::MAX].into_iter().enumerate() {
        p64(&mut bytes, ENTRIES + index * 16, VARIANT as u64);
        p64(&mut bytes, ENTRIES + index * 16 + 8, amount as u64);
    }
    let wing = Row {
        id: u64::MAX,
        ..Row::default()
    };
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut output = [EquipmentRow::default(); 3];
    assert_eq!(
        read_equipment(&memory, 0, WING_A, &wing, &mut output),
        Outcome::Rows { len: 3 }
    );
    for (ordinal, amount) in [0_i64, i64::MIN, i64::MAX].into_iter().enumerate() {
        assert_eq!(
            output[ordinal],
            EquipmentRow {
                wing: wing.id,
                ordinal: ordinal as u32,
                variant,
                amount_raw: amount,
                variant_pointer: VARIANT,
            }
        );
    }
    assert_eq!(
        read_equipment(&memory, 0, WING_A, &wing, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        }
    );
    p64(&mut bytes, WING_A + EQUIPMENT_POOL, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(matches!(
        read_equipment(&memory, 0, WING_A, &wing, &mut output),
        Outcome::Unavailable { .. }
    ));
}
