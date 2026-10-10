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

#[test]
fn strategic_bombing_stat_uses_the_signed_mission_index_and_default_only_for_minus_one() {
    const RECORDS: usize = 0xe000;
    const COLLECTION: usize = WING_A + 0x968;
    const INDEX: usize = COLLECTION + 0x18 + 3;
    const DEFAULT: usize = WING_A + 0x520;
    let mut bytes = fixture();
    bytes.resize(RECORDS + 3 * 0x270, 0);
    array(&mut bytes, COLLECTION, RECORDS, 3);
    p64(&mut bytes, DEFAULT, 765_432);
    p64(&mut bytes, RECORDS + 2 * 0x270 + 0x32 * 8, i64::MIN as u64);
    for (index, expected) in [
        (2_i8, Ok(i64::MIN)),
        (-1, Ok(765_432)),
        (-2, Err("strategic bombing mission index outside vector")),
        (3, Err("strategic bombing mission index outside vector")),
    ] {
        bytes[INDEX] = index as u8;
        assert_eq!(
            strategic_bombing_stat(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                WING_A
            ),
            expected
        );
    }
    bytes[INDEX] = 2;
    for (fault, expected) in [
        (INDEX, "strategic bombing mission index unreadable"),
        (COLLECTION, "strategic bombing mission-stat vector invalid"),
        (
            RECORDS + 2 * 0x270 + 0x32 * 8,
            "strategic bombing selected stat unreadable",
        ),
    ] {
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(fault),
        };
        assert_eq!(strategic_bombing_stat(&memory, WING_A), Err(expected));
        assert!(!memory.reads.borrow().iter().any(|&(at, _)| at == DEFAULT));
    }
    bytes[INDEX] = u8::MAX;
    let memory = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: Some(COLLECTION),
    };
    assert_eq!(strategic_bombing_stat(&memory, WING_A), Ok(765_432));
}

#[test]
fn strategic_bombing_optional_fault_preserves_the_wing_and_observed_zero() {
    let mut bytes = fixture();
    p32(&mut bytes, WING_A + MISSION + air_mission::ENABLED_MASK, 8);
    bytes[WING_A + 0x968 + 0x18 + 3] = u8::MAX;
    let mut rows = [Row::default(); 2];
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].strategic_bombing_raw, Some(0));
    assert_eq!(rows[1].strategic_bombing_raw, None);
    let memory = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: Some(WING_A + 0x520),
    };
    assert_eq!(walk(&memory, 0, GS, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].strategic_bombing_raw, None);
    assert_eq!(rows[0].id, u64::MAX);
}

#[test]
fn stored_wing_names_keep_sso_heap_empty_bytes_and_field_local_failures() {
    for text in [
        Vec::new(),
        b"Player %d".to_vec(),
        vec![0xff, 0, b'X'],
        vec![b'x'; 4097],
    ] {
        let mut bytes = fixture();
        bytes.resize(0xe000 + text.len() + 1, 0);
        let source = if text.len() <= 15 {
            WING_A + NAME
        } else {
            p64(&mut bytes, WING_A + NAME, 0xe000);
            0xe000
        };
        bytes[source..source + text.len()].copy_from_slice(&text);
        p64(
            &mut bytes,
            WING_A + NAME + crate::msvc_string::LENGTH,
            text.len() as u64,
        );
        p64(
            &mut bytes,
            WING_A + NAME + crate::msvc_string::CAPACITY,
            text.len().max(15) as u64,
        );
        p64(&mut bytes, WING_B + NAME + crate::msvc_string::CAPACITY, 15);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(name_demand(&memory, 0, GS), Ok(text.len()));
        let mut storage = vec![0; text.len()];
        let mut arena = NameArena {
            bytes: &mut storage,
            used: 0,
            required: 0,
            failures: 0,
        };
        let mut rows = [Row::default(); 2];
        assert_eq!(
            walk_with_names(&memory, 0, GS, &mut rows, &mut arena, |_, _, _| {}),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (arena.used, arena.required, arena.failures),
            (text.len(), text.len(), 0)
        );
        assert_eq!(rows[0].name, Some((0, text.len())));
        assert_eq!(rows[1].name, Some((text.len(), 0)));
        assert_eq!(storage, text);
        if !text.is_empty() {
            let mut arena = NameArena {
                bytes: &mut [],
                used: 0,
                required: 0,
                failures: 0,
            };
            assert_eq!(
                walk_with_names(&memory, 0, GS, &mut rows, &mut arena, |_, _, _| {}),
                Outcome::Rows { len: 2 }
            );
            assert_eq!((arena.required, arena.failures), (text.len(), 1));
            assert_eq!(rows[0].name, None);
            assert_eq!(rows[1].name, Some((0, 0)));
            assert_eq!(rows[0].planes, i32::MIN);
        }
        let fault = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(WING_A + NAME + crate::msvc_string::LENGTH),
        };
        let mut arena = NameArena {
            bytes: &mut storage,
            used: 0,
            required: 0,
            failures: 0,
        };
        assert_eq!(
            walk_with_names(&fault, 0, GS, &mut rows, &mut arena, |_, _, _| {}),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].name, None);
        assert_eq!(rows[1].name, Some((0, 0)));
        assert_eq!(arena.failures, 1);
    }
}

#[test]
fn worker_wing_name_growth_uses_one_original_large_header_and_named_native_faults() {
    use crate::{msvc_string, raid_tooltip_operands::NameReadError};
    let text = vec![b'x'; 131_073];
    let mut bytes = fixture();
    bytes.resize(0xe000 + text.len() + 1, 0);
    bytes[0xe000..0xe000 + text.len()].copy_from_slice(&text);
    p64(&mut bytes, WING_A + NAME, 0xe000);
    p64(
        &mut bytes,
        WING_A + NAME + msvc_string::LENGTH,
        text.len() as u64,
    );
    p64(
        &mut bytes,
        WING_A + NAME + msvc_string::CAPACITY,
        text.len() as u64,
    );
    let memory = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut storage = vec![b'p'];
    let (mut used, mut required) = (1, 1);
    assert_eq!(
        capture_name_growing(&memory, WING_A, &mut storage, &mut used, &mut required),
        Ok((1, text.len()))
    );
    assert_eq!(used, text.len() + 1);
    assert_eq!(required, used);
    assert_eq!(storage[0], b'p');
    assert_eq!(storage[1..used], text);
    assert_eq!(
        *memory.reads.borrow(),
        [
            (WING_A + NAME + msvc_string::LENGTH, 8),
            (WING_A + NAME + msvc_string::CAPACITY, 8),
            (WING_A + NAME, 8),
            (0xe000 + text.len(), 1),
            (0xe000, text.len()),
        ]
    );
    let failed = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: Some(0xe000),
    };
    assert_eq!(
        capture_name_growing(&failed, WING_A, &mut storage, &mut used, &mut required),
        Err(NameFailure::Native(NameReadError::Unreadable))
    );
    assert_eq!(used, text.len() + 1);
    assert_eq!(storage[1..used], text);
    assert_eq!(
        failed
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0xe000)
            .count(),
        1
    );
    p64(&mut bytes, WING_B + NAME + msvc_string::LENGTH, 16);
    p64(&mut bytes, WING_B + NAME + msvc_string::CAPACITY, 15);
    let failed = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    assert_eq!(
        capture_name_growing(&failed, WING_B, &mut storage, &mut used, &mut required),
        Err(NameFailure::Native(NameReadError::Header))
    );
    assert_eq!(failed.reads.borrow().len(), 2);
    assert_eq!(used, text.len() + 1);
}

#[test]
fn game_score_shared_walk_preserves_stored_names_in_map_and_source_rows() {
    let mut bytes = fixture();
    let text = b"Literal mod player wing";
    bytes.resize(0xe000 + text.len() + 1, 0);
    bytes[0xe000..0xe000 + text.len()].copy_from_slice(text);
    p64(&mut bytes, WING_A + NAME, 0xe000);
    p64(
        &mut bytes,
        WING_A + NAME + crate::msvc_string::LENGTH,
        text.len() as u64,
    );
    p64(
        &mut bytes,
        WING_A + NAME + crate::msvc_string::CAPACITY,
        text.len() as u64,
    );
    p64(&mut bytes, WING_B + NAME + crate::msvc_string::CAPACITY, 15);
    for fault in [None, Some(0xe000)] {
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault,
        };
        let mut names = vec![0; text.len()];
        let mut arena = NameArena {
            bytes: &mut names,
            used: 0,
            required: 0,
            failures: 0,
        };
        let mut map = [Row::default(); 2];
        let mut sources = [Source::default(); 2];
        let mut leaves = Vec::new();
        let result = walk_shared_game_score_with_owners(
            &memory,
            0,
            GS,
            &mut map,
            &mut sources,
            2,
            Some(&mut arena),
            Some(2),
            |_, _, _, _| {},
            |_, row| leaves.push(*row),
            |_, _| {},
        );
        assert_eq!(result.shared.map, Outcome::Rows { len: 2 });
        assert_eq!(result.shared.sources, Outcome::Rows { len: 2 });
        assert_eq!(result.score, Outcome::Rows { len: 2 });
        assert_eq!(sources.map(|source| source.row), map);
        assert_eq!(map[0].name, fault.is_none().then_some((0, text.len())));
        assert_eq!(map[1].name, Some((arena.used, 0)));
        assert_eq!(
            (arena.required, arena.failures),
            (text.len(), usize::from(fault.is_some()))
        );
        assert_eq!(
            leaves.iter().map(|row| row.planes).collect::<Vec<_>>(),
            [Ok(i32::MIN); 2]
        );
        if fault.is_none() {
            assert_eq!(names, text);
        }
    }
}

#[test]
fn game_score_owner_observations_share_reads_and_survive_a_later_pool_failure() {
    let mut bytes = fixture();
    let mut expected = [Row::default(); 2];
    let legacy = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let expected_outcome = walk(&legacy, 0, GS, &mut expected);
    let shared = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut rows = [Row::default(); 2];
    let mut owners = Vec::new();
    assert_eq!(
        walk_game_score_with_owners(
            &shared,
            0,
            GS,
            &mut rows,
            None,
            None,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        expected_outcome
    );
    assert_eq!(rows, expected);
    assert_eq!(*shared.reads.borrow(), *legacy.reads.borrow());
    assert_eq!(owners, [(0, None), (1, Some(OWNER)), (2, None), (3, None)]);
    p64(&mut bytes, POOL_OBJECT, 0);
    owners.clear();
    assert!(matches!(
        walk_game_score_with_owners(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut rows,
            None,
            None,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(owners, [(0, None), (1, Some(OWNER))]);
    p64(&mut bytes, GS + cgame_state::STRATEGIC_AIR_MANAGER, 0);
    owners.clear();
    assert!(matches!(
        walk_game_score_with_owners(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut rows,
            None,
            None,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(owners, [(0, None), (1, None), (2, None), (3, None)]);
    p64(
        &mut bytes,
        GS + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        0,
        0,
    );
    owners.clear();
    assert!(matches!(
        walk_game_score_with_owners(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut rows,
            None,
            None,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(owners, [(0, None), (1, None), (2, None), (3, None)]);
    // The native getter does not bounds-check a nonempty owner roster.
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        OWNERS,
        1,
    );
    owners.clear();
    assert_eq!(
        walk_game_score_with_owners(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            GS,
            &mut rows,
            None,
            None,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        Outcome::Rows { len: 0 }
    );
    assert_eq!(owners, [(0, None)]);
}

#[test]
fn game_score_slot_zero_air_owner_uses_the_catalogue_object() {
    let mut bytes = fixture();
    // Move the authenticated country/air owner to the sentinel slot and
    // retain zero as its raw stored identity, without positive link lookup.
    p64(&mut bytes, COUNTRIES, COUNTRY as u64);
    p64(&mut bytes, COUNTRIES + 8, 0);
    p64(&mut bytes, COUNTRIES + 24, 0);
    p32(&mut bytes, COUNTRY + ccountry::ID, 0);
    p64(&mut bytes, OWNERS, OWNER as u64);
    p64(&mut bytes, OWNERS + 8, 0);
    p32(&mut bytes, OWNER + cstrategic_air::COUNTRY, 0);
    let mut rows = [Row::default(); 2];
    let disabled = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let result = walk_game_score_with_owners(
        &disabled,
        0,
        GS,
        &mut rows,
        None,
        None,
        |_, _, _| {},
        |_, _| panic!("disabled score visitor"),
        |_, _| {},
    );
    assert_eq!(
        result.shared.map,
        Outcome::Unavailable {
            reason: "country id is zero"
        }
    );
    assert!(
        !disabled
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == COUNTRIES)
    );
    assert_eq!(capture(&bytes, &mut rows), result.shared.map);
    let mut owners = Vec::new();
    let result = walk_game_score_with_owners(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut rows,
        None,
        Some(2),
        |_, _, _| {},
        |_, _| {},
        |slot, pointer| owners.push((slot, pointer)),
    );
    assert_eq!(result.score, Outcome::Rows { len: 2 });
    assert_eq!(rows.map(|row| row.owner), [0, 0]);
    assert_eq!(owners[0], (0, Some(OWNER)));
    p64(&mut bytes, COUNTRIES, 0);
    let result = walk_game_score_with_owners(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut rows,
        None,
        Some(2),
        |_, _, _| {},
        |_, _| {},
        |_, _| {},
    );
    assert_eq!(
        result.score,
        Outcome::Unavailable {
            reason: "strategic air slot zero owner has no catalogue object"
        }
    );
}

#[test]
fn game_score_leaf_census_survives_unrelated_details_and_keeps_plane_fault_independent() {
    let mut bytes = fixture();
    for (pointer, stat) in [(WING_A, 73_u64), (WING_B, 74)] {
        bytes[pointer + MISSION_STATS + MISSION_STAT_INDEXES] = 255;
        p64(&mut bytes, pointer + DEFAULT_STATS + 0x33 * 8, stat);
    }
    for fault in [
        REGION,
        WING_A + MISSION + air_mission::REGION,
        WING_A + PLANE_COUNT,
        POOL_OBJECT + cair_wing_pool::AIR_BASE_REFERENCE,
    ] {
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(fault),
        };
        let mut rows = [Row::default(); 2];
        let mut scores = Vec::new();
        let result = walk_game_score_with_owners(
            &memory,
            0,
            GS,
            &mut rows,
            None,
            Some(2),
            |_, _, _| {},
            |pointer, leaf| {
                scores.push(crate::game_score_inputs::read_wing(&memory, pointer, leaf))
            },
            |_, _| {},
        );
        assert!(matches!(result.shared.map, Outcome::Unavailable { .. }));
        assert_eq!(result.score, Outcome::Rows { len: 2 });
        assert_eq!(
            scores.iter().map(|row| row.stat_raw).collect::<Vec<_>>(),
            [Ok(73), Ok(74)]
        );
        assert_eq!(scores[1].planes, Ok(i32::MIN));
        assert_eq!(
            scores[0].planes,
            if fault == WING_A + PLANE_COUNT {
                Err("game score air-wing plane count unreadable")
            } else {
                Ok(i32::MIN)
            }
        );
        let reads = memory.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, _)| at == WING_A + PLANE_COUNT)
                .count(),
            1
        );
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, _)| at == WING_B + PLANE_COUNT)
                .count(),
            1
        );
        assert!(!reads.iter().any(|&(at, _)| at == WING_B + BASE_OVERRIDE));
    }
    let legacy = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut expected = [Row::default(); 2];
    let expected_outcome = walk(&legacy, 0, GS, &mut expected);
    let disabled = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut rows = [Row::default(); 2];
    let result = walk_game_score_with_owners(
        &disabled,
        0,
        GS,
        &mut rows,
        None,
        None,
        |_, _, _| {},
        |_, _| panic!("disabled score visitor"),
        |_, _| {},
    );
    assert_eq!((result.shared.map, rows), (expected_outcome, expected));
    assert_eq!(*disabled.reads.borrow(), *legacy.reads.borrow());
    let omitted = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let result = walk_game_score_with_owners(
        &omitted,
        0,
        GS,
        &mut [],
        None,
        Some(0),
        |_, _, _| {},
        |_, _| panic!("full score visitor"),
        |_, _| {},
    );
    assert_eq!(
        result.score,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert!(
        !omitted
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == WING_A || at == WING_B)
    );
}

struct Traced<'a> {
    bytes: &'a [u8],
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}

#[test]
fn optional_transport_reads_follow_map_admission_and_do_not_withhold_core_rows() {
    let bytes = fixture();
    let memory = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut map = [Row::default(); 2];
    let mut sources = [Source::default(); 2];
    let mut name_bytes = [0; 16];
    let mut names = NameArena {
        bytes: &mut name_bytes,
        used: 0,
        required: 0,
        failures: 0,
    };
    let mut scratch = transport_operands::Scratch::with_capacity(2).unwrap();
    {
        let mut optional = transport_operands::Pass::new(0, map.len(), &mut scratch);
        let result = walk_shared_names_with_operands(
            &memory,
            0,
            GS,
            &mut map,
            &mut sources,
            2,
            &mut names,
            |pointer, pool, row, admitted| {
                if admitted.map {
                    optional.capture(&memory, pointer, pool, row);
                }
            },
        );
        assert_eq!(result.map, Outcome::Rows { len: 2 });
        assert!(map.iter().all(|row| row.pool_subunit.is_none()));
        assert!(map.iter().all(|row| row.planes == i32::MIN));
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|(address, _)| *address == POOL_OBJECT + cair_wing_pool::DEFINITION)
                .count(),
            1,
            "failed pool identity is not retried by a second wing"
        );
    }
    memory.reads.borrow_mut().clear();
    let mut optional = transport_operands::Pass::new(0, 0, &mut scratch);
    let result = walk_shared_names_with_operands(
        &memory,
        0,
        GS,
        &mut [],
        &mut sources,
        2,
        &mut names,
        |pointer, pool, row, admitted| {
            if admitted.map {
                optional.capture(&memory, pointer, pool, row);
            }
        },
    );
    assert_eq!(result.sources, Outcome::Rows { len: 2 });
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == POOL_OBJECT + cair_wing_pool::DEFINITION)
    );
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
fn growing_naval_projection_uses_the_original_pool_range_without_rereads() {
    let bytes = fixture();
    for fault in [None, Some(WING_B), Some(WING_B + COUNTRY_OVERRIDE)] {
        let reference = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault,
        };
        let expected = walk_shared(
            &reference,
            0,
            GS,
            &mut [Row::default(); 2],
            &mut [Source::default(); 2],
            2,
            |_, _, _, _| {},
        );
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault,
        };
        let mut map = Vec::new();
        let mut sources = Vec::new();
        let mut visited = Vec::new();
        let actual = walk_shared_game_score_buffers_with_owners(
            &memory,
            0,
            GS,
            Buffer::Growing(&mut map),
            Buffer::Growing(&mut sources),
            0,
            None,
            None,
            |pointer, _, _, admitted| visited.push((pointer, admitted)),
            |_, _| {},
            |_, _| {},
        );
        assert_eq!(actual.shared, expected);
        if fault.is_none() {
            assert_eq!(actual.shared.naval, Outcome::Rows { len: 2 });
        }
        assert!(visited.iter().all(|(_, admitted)| admitted.naval));
        assert_eq!(
            memory.reads.borrow().as_slice(),
            reference.reads.borrow().as_slice(),
            "growth keeps the original body observations and faults"
        );
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
            name: None,
            pool_subunit: None,
            raid_reference: None,
            strategic_bombing_raw: None,
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
fn strategic_air_manager_absence_is_zero_sizing_but_other_identities_refuse() {
    let mut bytes = fixture();
    p64(&mut bytes, GS + cgame_state::STRATEGIC_AIR_MANAGER, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        crate::transport_air_normalization::wing_entry_demand(&memory, 0, GS),
        Ok(0)
    );
    assert_eq!(
        capture(&bytes, &mut [Row::default(); 2]),
        Outcome::Unavailable {
            reason: "strategic air manager absent"
        }
    );

    let unreadable = bytes.len() as u64;
    p64(
        &mut bytes,
        GS + cgame_state::STRATEGIC_AIR_MANAGER,
        unreadable,
    );
    assert_eq!(
        capture(&bytes, &mut [Row::default(); 2]),
        Outcome::Unavailable {
            reason: "strategic air manager vtable unreadable"
        }
    );
    p64(
        &mut bytes,
        GS + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    // The secondary CPersistent base vtable is not the complete manager.
    p64(
        &mut bytes,
        MANAGER,
        (cstrategic_air_manager::VTABLE_RVA + 0x20) as u64,
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        crate::transport_air_normalization::wing_entry_demand(&memory, 0, GS),
        Err("strategic air manager vtable mismatch")
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
        (GS + 0x690, 0, "strategic air manager absent"),
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
