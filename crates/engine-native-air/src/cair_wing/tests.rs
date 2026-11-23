use super::*;
use engine_native_memory::memory::SliceMemory;

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
    p64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    p32(&mut bytes, COUNTRY + 8, 3);
    p64(&mut bytes, GS + 0x690, MANAGER as u64);
    p64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA.0 as u64,
    );
    array(&mut bytes, MANAGER + 0x30, OWNERS, 4);
    p64(&mut bytes, OWNERS + 8, OWNER as u64);
    p64(&mut bytes, OWNER, cstrategic_air::VTABLE_RVA.0 as u64);
    p32(&mut bytes, OWNER + 0x90, 3);
    array(&mut bytes, OWNER + 0x58, POOLS, 1);
    p64(&mut bytes, POOLS, POOL_OBJECT as u64);
    p64(&mut bytes, POOL_OBJECT, cair_wing_pool::VTABLE_RVA.0 as u64);
    p64(&mut bytes, POOL_OBJECT + 0x18, 0xfeed_beef_0000_0041);
    p64(&mut bytes, POOL_OBJECT + 0x40, OWNER as u64);
    array(&mut bytes, POOL_OBJECT + 0x28, WINGS, 2);
    p64(&mut bytes, WINGS, WING_A as u64);
    p64(&mut bytes, WINGS + 8, WING_B as u64);
    for (wing, id) in [(WING_A, u64::MAX), (WING_B, 69)] {
        p64(&mut bytes, wing, VTABLE_RVA.0 as u64);
        p64(&mut bytes, wing + 0x10, SECONDARY_VTABLE_RVA.0 as u64);
        p64(&mut bytes, wing + 0x18, id);
        p64(&mut bytes, wing + 0x38, POOL_OBJECT as u64);
        p32(&mut bytes, wing + 0x7c, i32::MIN as u32);
        p32(&mut bytes, wing + 0xa0, 0x8000_0800);
        p32(&mut bytes, wing + 0x9c4, i32::MIN as u32);
    }
    p64(&mut bytes, WING_A + 0x48, 0xffff_ffff_0000_0041);
    p64(&mut bytes, WING_A + 0xc0, REGION as u64);
    p64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        REGION + 8,
        cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
    );
    p32(&mut bytes, REGION + 0x58, u32::MAX);
    bytes
}
fn name_header(bytes: &[u8], wing: usize) -> Option<[u8; engine_native_memory::msvc_string::STRIDE]> {
    Some(
        bytes[wing + NAME..wing + NAME + engine_native_memory::msvc_string::STRIDE]
            .try_into()
            .unwrap(),
    )
}
fn rows_or_count<T>(rows: &mut Vec<T>, capacity: usize) -> engine_native_memory::walk::Buffer<'_, T> {
    if capacity == 0 {
        engine_native_memory::walk::Buffer::Count
    } else {
        engine_native_memory::walk::Buffer::Growing(rows)
    }
}

fn independent(memory: &impl Memory, capacity: usize) -> Outcome {
    if capacity == 0 {
        count(memory, 0, GS)
    } else {
        walk(memory, 0, GS, &mut Vec::new())
    }
}

fn capture(bytes: &[u8], rows: &mut Vec<Row>) -> Outcome {
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
    let mut rows = vec![Row::default(); 2];
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
            WING_A + NAME + engine_native_memory::msvc_string::LENGTH,
            text.len() as u64,
        );
        p64(
            &mut bytes,
            WING_A + NAME + engine_native_memory::msvc_string::CAPACITY,
            text.len().max(15) as u64,
        );
        p64(&mut bytes, WING_B + NAME + engine_native_memory::msvc_string::CAPACITY, 15);
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
        let mut rows = vec![Row::default(); 2];
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
        let fault = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(WING_A + NAME + engine_native_memory::msvc_string::LENGTH),
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
    use engine_native_memory::msvc_string;
    use crate::raid_tooltip_operands::NameReadError;
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
        capture_name_growing(
            &memory,
            WING_A,
            None,
            &mut storage,
            &mut used,
            &mut required
        ),
        Ok((1, text.len()))
    );
    assert_eq!(used, text.len() + 1);
    assert_eq!(required, used);
    assert_eq!(storage[0], b'p');
    assert_eq!(storage[1..used], text);
    assert_eq!(
        *memory.reads.borrow(),
        [
            (WING_A + NAME, msvc_string::STRIDE),
            (0xe000, text.len()),
            (0xe000 + text.len(), 1),
        ]
    );
    let failed = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: Some(0xe000),
    };
    assert_eq!(
        capture_name_growing(
            &failed,
            WING_A,
            None,
            &mut storage,
            &mut used,
            &mut required
        ),
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
        capture_name_growing(
            &failed,
            WING_B,
            None,
            &mut storage,
            &mut used,
            &mut required
        ),
        Err(NameFailure::Native(NameReadError::Header))
    );
    assert_eq!(failed.reads.borrow().len(), 1);
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
        WING_A + NAME + engine_native_memory::msvc_string::LENGTH,
        text.len() as u64,
    );
    p64(
        &mut bytes,
        WING_A + NAME + engine_native_memory::msvc_string::CAPACITY,
        text.len() as u64,
    );
    p64(&mut bytes, WING_B + NAME + engine_native_memory::msvc_string::CAPACITY, 15);
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
        let mut map = vec![Row::default(); 2];
        let mut sources = vec![Source::default(); 2];
        let mut leaves = Vec::new();
        let result = walk_shared_game_score_with_owners(
            &memory,
            0,
            GS,
            &mut map,
            &mut sources,
            Some(&mut arena),
            true,
            |_, _, _, _| {},
            |_, row| leaves.push(*row),
            |_, _| {},
        );
        assert_eq!(result.shared.map, Outcome::Rows { len: 2 });
        assert_eq!(result.shared.sources, Outcome::Rows { len: 2 });
        assert_eq!(result.score, Outcome::Rows { len: 2 });
        assert_eq!(sources.iter().map(|source| source.row).collect::<Vec<_>>(), map);
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
    let mut expected = vec![Row::default(); 2];
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
    let mut rows = vec![Row::default(); 2];
    let mut owners = Vec::new();
    assert_eq!(
        walk_game_score_with_owners(
            &shared,
            0,
            GS,
            &mut rows,
            None,
            false,
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
            false,
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
            false,
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
            false,
            |_, _, _| {},
            |_, _| {},
            |slot, pointer| owners.push((slot, pointer))
        )
        .shared
        .map,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(owners, [(0, None), (1, None), (2, None), (3, None)]);
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
            false,
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
    p64(&mut bytes, COUNTRIES, COUNTRY as u64);
    p64(&mut bytes, COUNTRIES + 8, 0);
    p64(&mut bytes, COUNTRIES + 24, 0);
    p32(&mut bytes, COUNTRY + ccountry::ID, 0);
    p64(&mut bytes, OWNERS, OWNER as u64);
    p64(&mut bytes, OWNERS + 8, 0);
    p32(&mut bytes, OWNER + cstrategic_air::COUNTRY, 0);
    let mut rows = vec![Row::default(); 2];
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
        false,
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
        true,
        |_, _, _| {},
        |_, _| {},
        |slot, pointer| owners.push((slot, pointer)),
    );
    assert_eq!(result.score, Outcome::Rows { len: 2 });
    assert_eq!(rows.iter().map(|row| row.owner).collect::<Vec<_>>(), [0, 0]);
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
        true,
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
        let mut rows = vec![Row::default(); 2];
        let mut scores = Vec::new();
        let result = walk_game_score_with_owners(
            &memory,
            0,
            GS,
            &mut rows,
            None,
            true,
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
        for wing in [WING_A, WING_B] {
            let planes = reads
                .iter()
                .filter(|&&(at, _)| at == wing + PLANE_COUNT)
                .count();
            assert!((1..=2).contains(&planes), "{fault:x} {wing:x} {planes}");
        }
    }
    let legacy = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut expected = vec![Row::default(); 2];
    let expected_outcome = walk(&legacy, 0, GS, &mut expected);
    let disabled = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut rows = vec![Row::default(); 2];
    let result = walk_game_score_with_owners(
        &disabled,
        0,
        GS,
        &mut rows,
        None,
        false,
        |_, _, _| {},
        |_, _| panic!("disabled score visitor"),
        |_, _| {},
    );
    assert_eq!((result.shared.map, rows), (expected_outcome, expected));
    assert_eq!(*disabled.reads.borrow(), *legacy.reads.borrow());
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
    let mut map = vec![Row::default(); 2];
    let mut sources = vec![Source::default(); 2];
    let mut name_bytes = vec![0; 16];
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
    let mut full = Vec::new();
    assert_eq!(capture(&bytes, &mut full), Outcome::Rows { len: 2 });
    let independent = |capacity: usize| {
        if capacity == 0 {
            count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS)
        } else {
            Outcome::Rows { len: 2 }
        }
    };
    for map_capacity in [0, 3] {
        for source_capacity in [0, 3] {
            {
                let map = independent(map_capacity);
                let sources = independent(source_capacity);
                let naval = Outcome::Rows { len: 2 };
                let traced = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: None,
                };
                let mut map_rows = Vec::new();
                let mut source_rows = Vec::new();
                let mut visited = Vec::new();
                let result = walk_shared(
                    &traced,
                    0,
                    GS,
                    rows_or_count(&mut map_rows, map_capacity),
                    rows_or_count(&mut source_rows, source_capacity),
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
                assert_eq!(map_rows[..map.len()], full[..map.len()]);
                for (index, source) in source_rows[..sources.len()].iter().enumerate() {
                    assert_eq!(source.row, full[index]);
                    assert_eq!(source.pool, POOL_OBJECT);
                    assert_eq!(source.pointer, [WING_A, WING_B][index]);
                }
                assert_eq!(visited.len(), 2);
                for (index, (source, admitted)) in visited.iter().enumerate() {
                    assert_eq!(source.pointer, [WING_A, WING_B][index]);
                    assert_eq!(
                        *admitted,
                        Admitted {
                            map: index < map_capacity,
                            sources: index < source_capacity,
                            naval: true,
                            name_header: name_header(&bytes, [WING_A, WING_B][index]),
                        }
                    );
                    assert_eq!(source.row, full[index]);
                    if admitted.sources {
                        assert_eq!(*source, source_rows[index]);
                    }
                }
                let reference = Traced {
                    bytes: &bytes,
                    reads: Default::default(),
                    fault: None,
                };
                walk(&reference, 0, GS, &mut Vec::new());
                assert_eq!(
                    traced.reads.borrow().as_slice(),
                    reference.reads.borrow().as_slice(),
                    "capacities map {map_capacity}, source {source_capacity}"
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
            engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 2]),
            engine_native_memory::walk::Buffer::Growing(&mut vec![Source::default(); 2]),
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
            EVERY_OWNER,
            Buffer::Growing(&mut map),
            Buffer::Growing(&mut sources),
            None,
            false,
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

struct Part {
    walked: GameScoreWalk,
    rows: Vec<Row>,
    owners: Vec<(u32, Option<usize>)>,
    scores: Vec<u64>,
}

fn part(bytes: &[u8], slots: std::ops::Range<u32>) -> Part {
    let mut rows = Vec::new();
    let mut owners = Vec::new();
    let mut scores = Vec::new();
    let walked = walk_shared_game_score_buffers_with_owners(
        &SliceMemory { base: 0, bytes },
        0,
        GS,
        slots,
        Buffer::Growing(&mut rows),
        Buffer::Growing(&mut Vec::new()),
        None,
        true,
        |_, _, _, _| {},
        |_, leaf| scores.push(leaf.id),
        |slot, entry| owners.push((slot, entry)),
    );
    rows.truncate(walked.shared.map.len());
    Part {
        walked,
        rows,
        owners,
        scores,
    }
}

#[test]
fn owner_slot_parts_walk_only_their_slots_and_end_where_the_serial_walk_ends() {
    let mut bytes = fixture();
    let serial = part(&bytes, EVERY_OWNER);
    assert!(!serial.walked.ended);
    assert_eq!(serial.walked.shared.map, Outcome::Rows { len: 2 });
    let parts = [0..1, 1..2, 2..u32::MAX].map(|slots| part(&bytes, slots));
    assert_eq!(
        parts
            .each_ref()
            .map(|part| (part.walked.shared.map, part.walked.ended)),
        [
            (Outcome::Rows { len: 0 }, false),
            (Outcome::Rows { len: 2 }, false),
            (Outcome::Rows { len: 0 }, false),
        ]
    );
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.rows.clone())
            .collect::<Vec<_>>(),
        serial.rows
    );
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.owners.clone())
            .collect::<Vec<_>>(),
        serial.owners
    );
    assert_eq!(
        parts
            .iter()
            .flat_map(|part| part.scores.clone())
            .collect::<Vec<_>>(),
        serial.scores
    );
    p64(&mut bytes, POOL_OBJECT, 0);
    let serial = part(&bytes, EVERY_OWNER).walked;
    let faulted = part(&bytes, 1..2).walked;
    assert!(serial.ended && faulted.ended);
    assert_eq!(faulted.shared, serial.shared);
    assert!(!part(&bytes, 0..1).walked.ended);
    p64(&mut bytes, GS + cgame_state::STRATEGIC_AIR_MANAGER, 0);
    let first = part(&bytes, 0..1);
    let later = part(&bytes, 1..u32::MAX);
    assert!(first.walked.ended && later.walked.ended);
    assert_eq!(first.owners, [(0, None), (1, None), (2, None), (3, None)]);
    assert!(later.owners.is_empty());
}

#[test]
fn shared_walk_leaf_faults_preserve_each_independently_capped_consumer() {
    for faulty_field in [
        WING_B,
        WING_B + MISSION + air_mission::ENABLED_MASK,
        WING_B + COUNTRY_OVERRIDE,
    ] {
        let bytes = fixture();
        for map_capacity in [0, 2] {
            for source_capacity in [0, 2] {
                {
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
                        rows_or_count(&mut Vec::new(), map_capacity),
                        rows_or_count(&mut Vec::new(), source_capacity),
                        |pointer, _, row, admitted| visits.push((pointer, *row, admitted)),
                    );
                    let reference = Traced {
                        bytes: &bytes,
                        reads: Default::default(),
                        fault: Some(faulty_field),
                    };
                    let map = independent(&reference, map_capacity);
                    let sources = independent(&reference, source_capacity);
                    let naval = independent(&reference, 2);
                    assert_eq!(
                        shared,
                        SharedWalk {
                            map,
                            sources,
                            naval
                        }
                    );
                    assert_eq!(visits.len(), 1);
                    if let Some((pointer, _, admitted)) = visits.first() {
                        assert_eq!(*pointer, WING_A);
                        assert_eq!(
                            *admitted,
                            Admitted {
                                map: map_capacity != 0,
                                sources: source_capacity != 0,
                                naval: true,
                                name_header: name_header(&bytes, WING_A),
                            }
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn shared_walk_counting_lanes_do_not_hide_map_source_or_naval_admissions() {
    let bytes = fixture();
    for (map_capacity, source_capacity) in [(2, 0), (0, 2), (0, 0)] {
        let mut visited = Vec::new();
        let result = walk_shared(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            rows_or_count(&mut Vec::new(), map_capacity),
            rows_or_count(&mut Vec::new(), source_capacity),
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
                    naval: true,
                    name_header: name_header(&bytes, WING_B),
                }
            )
        );
        assert_eq!(result.naval, Outcome::Rows { len: 2 });
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
        let result = walk_shared(
            &memory,
            0,
            GS,
            engine_native_memory::walk::Buffer::Count,
            engine_native_memory::walk::Buffer::Count,
            |_, _, _, _| {},
        );
        assert!(matches!(result.map, Outcome::Unavailable { .. }));
        assert_eq!(result.map, result.sources);
        assert_eq!(result.map, result.naval);
    }
}

#[test]
fn shared_walk_each_pass_is_fresh_and_source_visits_do_not_depend_on_map_capacity() {
    let mut bytes = fixture();
    let mut first = vec![Source::default(); 2];
    let a = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Growing(&mut first),
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
    let mut second = vec![Source::default(); 2];
    let mut visited = Vec::new();
    let b = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Growing(&mut second),
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

#[test]
fn pool_ordinals_follow_pools_then_wings() {
    const POOL_B: usize = 0x9800;
    const WINGS_B: usize = 0xa800;
    const WING_C: usize = 0xe000;
    let mut bytes = fixture();
    bytes.resize(0xf000, 0);
    array(&mut bytes, OWNER + cstrategic_air::POOLS, POOLS, 2);
    p64(&mut bytes, POOLS, POOL_B as u64);
    p64(&mut bytes, POOLS + 8, POOL_OBJECT as u64);
    p64(&mut bytes, POOL_B, cair_wing_pool::VTABLE_RVA.0 as u64);
    p64(&mut bytes, POOL_B + 0x18, 0xfeed_beef_0000_0042);
    p64(&mut bytes, POOL_B + 0x40, OWNER as u64);
    array(&mut bytes, POOL_B + 0x28, WINGS_B, 1);
    p64(&mut bytes, WINGS_B, WING_C as u64);
    p64(&mut bytes, WING_C, VTABLE_RVA.0 as u64);
    p64(&mut bytes, WING_C + 0x10, SECONDARY_VTABLE_RVA.0 as u64);
    p64(&mut bytes, WING_C + 0x18, 5);
    p64(&mut bytes, WING_C + 0x38, POOL_B as u64);
    let mut rows = vec![Row::default(); 3];
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 3 });
    assert_eq!(
        rows.iter().map(|row| (row.id, row.air_base, row.pool_ordinal)).collect::<Vec<_>>(),
        [
            (5, 0xfeed_beef_0000_0042, 0),
            (u64::MAX, 0xfeed_beef_0000_0041, 1),
            (69, 0xfeed_beef_0000_0041, 2),
        ]
    );
    let mut sources = vec![Source::default(); 3];
    let walked = walk_shared(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Growing(&mut sources),
        |_, _, _, _| {},
    );
    assert_eq!(
        walked.map,
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    assert_eq!(sources.iter().map(|source| source.row.pool_ordinal).collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn raw_handles_signed_counts_alias_owner_and_nullable_region_survive() {
    let mut bytes = fixture();
    p32(&mut bytes, WING_A + ELIGIBILITY_COUNTER, i32::MIN as u32);
    p32(&mut bytes, WING_A + ELIGIBILITY_LIMIT, i32::MAX as u32);
    bytes[WING_A + MISSION + air_mission::ACTIVE] = 1;
    bytes[WING_B + ELIGIBILITY_VETO] = 1;
    let mut rows = vec![Row::default(); 2];
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(
        rows[0],
        Row {
            acquired_manpower: None,
            exile_country: None,
            id: u64::MAX,
            owner: 3,
            owner_pointer: COUNTRY,
            owner_slot: 1,
            pool_ordinal: 0,
            country_override: i32::MIN,
            air_base: 0xfeed_beef_0000_0041,
            base_override: 0xffff_ffff_0000_0041,
            planes: i32::MIN,
            mission_mask: 0x8000_0800,
            mission_region: Some(u32::MAX),
            region_pointer: REGION,
            base_range_raw: 0,
            ace: 0,
            mission_active: 1,
            eligibility_veto: 0,
            eligibility_counter: i32::MIN,
            eligibility_limit: i32::MAX,
            attached_orders_group: None,
            attached_group_pointer: 0,
            name: None,
            pool_subunit: None,
            raid_reference: None,
            strategic_bombing_raw: None,
            replacement_setting: None,
            map_flags: None,
            fuel_inputs: None,
        }
    );
    assert_eq!(rows[1].id, 69);
    assert_eq!(rows[1].pool_ordinal, 1);
    assert_eq!((rows[1].mission_active, rows[1].eligibility_veto), (0, 1));
    assert_eq!(rows[1].mission_region, None);
    p32(&mut bytes, REGION + 0x58, 0);
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows[0].mission_region, Some(0));
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
fn a_broken_roster_or_wing_refuses_the_census() {
    let mut bytes = fixture();
    let mut rows = Vec::new();
    p64(&mut bytes, WING_B + 0xc0, u64::MAX);
    let mut all = Vec::new();
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
fn strategic_air_manager_absence_and_foreign_identity_refuse() {
    let mut bytes = fixture();
    p64(&mut bytes, GS + cgame_state::STRATEGIC_AIR_MANAGER, 0);
    assert_eq!(
        capture(&bytes, &mut Vec::new()),
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
        capture(&bytes, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "strategic air manager vtable unreadable"
        }
    );
    p64(
        &mut bytes,
        GS + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    p64(
        &mut bytes,
        MANAGER,
        (cstrategic_air_manager::VTABLE_RVA.0 + 0x20) as u64,
    );
    assert_eq!(
        capture(&bytes, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "strategic air manager vtable mismatch"
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
        (GS + 0x690, 0, "strategic air manager absent"),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, at, value);
        assert_eq!(
            capture(&bytes, &mut Vec::new()),
            Outcome::Unavailable { reason }
        );
    }
    let mut bytes = fixture();
    p64(&mut bytes, OWNERS + 8, 0);
    p64(&mut bytes, OWNERS + 24, OWNER as u64);
    assert_eq!(
        capture(&bytes, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "strategic air owner differs from canonical country slot"
        }
    );
    p64(&mut bytes, OWNERS + 24, 0);
    assert_eq!(
        capture(&bytes, &mut Vec::new()),
        Outcome::Rows { len: 0 }
    );
    p32(&mut bytes, MANAGER + 0x3c, 0);
    assert_eq!(
        capture(&bytes, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "strategic air manager has no country slots"
        }
    );
}

#[test]
fn shared_extensions_visit_only_authenticated_emitted_rows() {
    let bytes = fixture();
    let mut visits = Vec::new();
    let outcome = walk_with(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut vec![Row::default(); 1],
        |wing, pool, row| visits.push((wing, pool, row.id)),
    );
    assert_eq!(
        visits,
        [(WING_A, POOL_OBJECT, u64::MAX), (WING_B, POOL_OBJECT, 69)]
    );
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    let mut corrupt = bytes;
    p64(&mut corrupt, WING_A + 0x38, 0);
    let outcome = walk_with(
        &SliceMemory {
            base: 0,
            bytes: &corrupt,
        },
        0,
        GS,
        &mut vec![Row::default(); 2],
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
        engine_native::cequipment_variant::pool::VTABLE_RVA.0 as u64,
    );
    array(&mut bytes, WING_A + EQUIPMENT_BANK, ENTRIES, 3);
    p64(
        &mut bytes,
        VARIANT,
        engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
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

#[test]
fn a_pass_authenticates_a_shared_mission_region_once_and_reads_eligibility_in_one_span() {
    let mut bytes = fixture();
    p64(&mut bytes, WING_B + 0xc0, REGION as u64);
    p32(&mut bytes, WING_A + ELIGIBILITY_COUNTER, 7);
    p32(&mut bytes, WING_A + ELIGIBILITY_LIMIT, 9);
    let memory = Traced {
        bytes: &bytes,
        reads: Default::default(),
        fault: None,
    };
    let mut rows = vec![Row::default(); 2];
    assert_eq!(walk(&memory, 0, GS, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(
        rows.iter().map(|row| row.mission_region).collect::<Vec<_>>(),
        [Some(u32::MAX), Some(u32::MAX)]
    );
    assert_eq!(
        (rows[0].eligibility_counter, rows[0].eligibility_limit),
        (7, 9)
    );
    let reads = memory.reads.borrow();
    let region_reads = reads
        .iter()
        .filter(|&&(at, _)| (REGION..REGION + 0x60).contains(&at))
        .count();
    assert_eq!(region_reads, 3, "{reads:x?}");
    for wing in [WING_A, WING_B] {
        assert!(reads.contains(&(wing + ELIGIBILITY_COUNTER, 8)));
        assert!(!reads.iter().any(|&(at, _)| at == wing + ELIGIBILITY_LIMIT));
    }
    drop(reads);
    p32(&mut bytes, REGION + 0x58, 5);
    assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
    assert_eq!(rows.iter().map(|row| row.mission_region).collect::<Vec<_>>(), [Some(5), Some(5)]);
    for fault in [WING_A + ELIGIBILITY_COUNTER, WING_A + ELIGIBILITY_LIMIT] {
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: Some(fault),
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut rows),
            Outcome::Unavailable {
                reason: "air wing row field unreadable or address overflow"
            },
            "{fault:x}"
        );
    }
}

#[test]
fn census_hints_the_pointer_array_and_each_serially_read_wing() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Event {
        Read(usize, usize),
        Hint(usize),
    }
    struct Logged<'a> {
        bytes: &'a [u8],
        events: std::cell::RefCell<Vec<Event>>,
    }
    impl Memory for Logged<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.events
                .borrow_mut()
                .push(Event::Read(address, destination.len()));
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
        fn prefetch(&self, address: usize, _len: usize) {
            self.events.borrow_mut().push(Event::Hint(address));
        }
    }
    let bytes = fixture();
    let mut plain = vec![Row::default(); 2];
    assert_eq!(capture(&bytes, &mut plain), Outcome::Rows { len: 2 });
    for serial in [false, true] {
        let memory = Logged {
            bytes: &bytes,
            events: Default::default(),
        };
        let mut rows = vec![Row::default(); 2];
        SERIAL.set(serial);
        assert_eq!(walk(&memory, 0, GS, &mut rows), Outcome::Rows { len: 2 });
        SERIAL.set(false);
        assert_eq!(rows, plain);
        let events = memory.events.into_inner();
        let at = |event: Event| events.iter().position(|&seen| seen == event);
        let first_pointer = if serial {
            at(Event::Read(WINGS, 8))
        } else {
            at(Event::Read(WINGS, 16))
        }
        .unwrap();
        assert!(at(Event::Hint(WINGS)).unwrap() < first_pointer);
        for (slot, wing) in [WING_A, WING_B].into_iter().enumerate() {
            let field = events
                .iter()
                .position(
                    |event| matches!(*event, Event::Read(at, _) if (wing..wing + 0x1000).contains(&at)),
                )
                .unwrap();
            assert!(first_pointer < field, "{wing:x}");
            match at(Event::Hint(wing)) {
                Some(hint) => {
                    assert!(serial, "{wing:x}");
                    let pointer = at(Event::Read(WINGS + slot * 8, 8)).unwrap();
                    assert!(pointer < hint && hint < field, "{wing:x}");
                }
                None => assert!(!serial, "{wing:x}"),
            }
        }
        let reads: Vec<Event> = events
            .into_iter()
            .filter(|event| matches!(event, Event::Read(..)))
            .collect();
        let traced = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: None,
        };
        SERIAL.set(serial);
        walk(&traced, 0, GS, &mut plain);
        SERIAL.set(false);
        let expected: Vec<Event> = traced
            .reads
            .into_inner()
            .into_iter()
            .map(|(at, len)| Event::Read(at, len))
            .collect();
        assert_eq!(reads, expected);
    }
}

struct Scale {
    bytes: Vec<u8>,
    wings: usize,
    owners: Vec<usize>,
    pools: Vec<usize>,
    wing_pointers: Vec<usize>,
    slots: Vec<usize>,
    region: usize,
    catalogue: Vec<usize>,
    links: Vec<usize>,
    countries: Vec<usize>,
}

impl Scale {
    const WING_SIZE: usize = 0xa00;
    const HEAP_NAME: usize = 20;

    fn new(countries: usize, wings: impl Fn(usize) -> usize) -> Self {
        Self::build(countries, wings, false)
    }

    fn build(countries: usize, wings: impl Fn(usize) -> usize, zero: bool) -> Self {
        let first = usize::from(!zero);
        let total: usize = (first..=countries).map(&wings).sum();
        let countries_at = 0x10000;
        let links_at = countries_at + (countries + 1) * 8;
        let objects_at = (links_at + (countries + 1) * 4 + 15) & !15;
        let owners_at = objects_at + (countries + 1) * 16;
        let owner_objects = owners_at + (countries + 1) * 8;
        let pool_objects = owner_objects + (countries + 1) * 0xa0;
        let pointers_at = pool_objects + (countries + 1) * 0x50;
        let wings_at = (pointers_at + total * 8 + 15) & !15;
        let texts_at = wings_at + total * Self::WING_SIZE;
        let region = texts_at + total * 32;
        let mut scale = Self {
            bytes: vec![0; region + 0x100],
            wings: total,
            owners: Vec::with_capacity(countries),
            pools: Vec::with_capacity(countries),
            wing_pointers: Vec::with_capacity(total),
            slots: Vec::with_capacity(total),
            region,
            catalogue: Vec::new(),
            links: Vec::new(),
            countries: Vec::new(),
        };
        let bytes = &mut scale.bytes;
        p64(bytes, region, cstrategic_region::VTABLE_RVA.0 as u64);
        p64(
            bytes,
            region + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
        );
        p32(bytes, region + cstrategic_region::ID, 77);
        array(
            bytes,
            GS + cgame_state::COUNTRIES,
            countries_at,
            countries as u32 + 1,
        );
        array(
            bytes,
            GS + cgame_state::COUNTRY_LINKS,
            links_at,
            countries as u32 + 1,
        );
        p64(
            bytes,
            GS + cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        p64(bytes, MANAGER, cstrategic_air_manager::VTABLE_RVA.0 as u64);
        array(
            bytes,
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            owners_at,
            countries as u32 + 1,
        );
        let mut wing_at = wings_at;
        let mut pointer_at = pointers_at;
        for country in first..=countries {
            let object = objects_at + country * 16;
            p64(bytes, countries_at + country * 8, object as u64);
            p32(bytes, links_at + country * 4, country as u32);
            p64(bytes, object, ccountry::VTABLE_RVA.0 as u64);
            p32(bytes, object + ccountry::ID, country as u32);
            scale.catalogue.push(countries_at + country * 8);
            scale.links.push(links_at + country * 4);
            scale.countries.push(object);
            let owner = owner_objects + country * 0xa0;
            let pool = pool_objects + country * 0x50;
            p64(bytes, owners_at + country * 8, owner as u64);
            p64(bytes, owner, cstrategic_air::VTABLE_RVA.0 as u64);
            p32(bytes, owner + cstrategic_air::COUNTRY, country as u32);
            array(bytes, owner + cstrategic_air::POOLS, pool + 0x48, 1);
            p64(bytes, pool + 0x48, pool as u64);
            p64(bytes, pool, cair_wing_pool::VTABLE_RVA.0 as u64);
            p64(bytes, pool + cair_wing_pool::OWNER, owner as u64);
            p64(
                bytes,
                pool + cair_wing_pool::AIR_BASE_REFERENCE,
                country as u64,
            );
            let count = wings(country);
            array(
                bytes,
                pool + cair_wing_pool::WINGS,
                pointer_at,
                count as u32,
            );
            scale.owners.push(owner);
            scale.pools.push(pool);
            for _ in 0..count {
                let ordinal = scale.wing_pointers.len();
                p64(bytes, pointer_at, wing_at as u64);
                scale.slots.push(pointer_at);
                pointer_at += 8;
                p64(bytes, wing_at, VTABLE_RVA.0 as u64);
                p64(bytes, wing_at + SECONDARY, SECONDARY_VTABLE_RVA.0 as u64);
                p64(bytes, wing_at + SECONDARY + 8, ordinal as u64 + 1);
                p64(bytes, wing_at + POOL, pool as u64);
                p32(bytes, wing_at + PLANE_COUNT, ordinal as u32);
                p32(bytes, wing_at + COUNTRY_OVERRIDE, country as u32);
                if ordinal % 5 == 1 {
                    p64(
                        bytes,
                        wing_at + MISSION + air_mission::REGION,
                        region as u64,
                    );
                }
                let name = wing_at + NAME;
                if ordinal % 7 == 3 {
                    let text = texts_at + ordinal * 32;
                    bytes[text..text + Self::HEAP_NAME].fill(b'a' + (ordinal % 26) as u8);
                    p64(bytes, name, text as u64);
                    p64(
                        bytes,
                        name + engine_native_memory::msvc_string::LENGTH,
                        Self::HEAP_NAME as u64,
                    );
                    p64(bytes, name + engine_native_memory::msvc_string::CAPACITY, 31);
                } else {
                    let text = format!("w{ordinal}");
                    bytes[name..name + text.len()].copy_from_slice(text.as_bytes());
                    p64(bytes, name + engine_native_memory::msvc_string::LENGTH, text.len() as u64);
                    p64(bytes, name + engine_native_memory::msvc_string::CAPACITY, 15);
                }
                scale.wing_pointers.push(wing_at);
                wing_at += Self::WING_SIZE;
            }
        }
        scale
    }

    fn heap_text(&self, ordinal: usize) -> usize {
        let name = self.wing_pointers[ordinal] + NAME;
        u64::from_le_bytes(self.bytes[name..name + 8].try_into().unwrap()) as usize
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Census {
    shared: SharedWalk,
    score: Outcome,
    ended: bool,
    rows: Vec<Row>,
    sources: Vec<Source>,
    names: Vec<Result<(usize, usize), NameFailure>>,
    name_bytes: Vec<u8>,
    name_required: usize,
    visits: Vec<(usize, [bool; 3])>,
    scores: Vec<(usize, u64, Result<i32, &'static str>)>,
    owners: Vec<(u32, Option<usize>)>,
}

type Capacities = Option<[usize; 2]>;

fn census(memory: &impl Memory, serial: bool, capacities: Capacities) -> (Census, u32) {
    let [map, source] = capacities.unwrap_or([0; 2]);
    let mut rows = vec![Row::default(); map];
    let mut sources = vec![Source::default(); source];
    let (mut name_bytes, mut used, mut name_required) = (Vec::new(), 0, 0);
    let mut names = Vec::new();
    let mut visits = Vec::new();
    let mut scores = Vec::new();
    let mut owners = Vec::new();
    let recorded = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::AirWingCensus);
    SERIAL.set(serial);
    let walked = walk_shared_game_score_buffers_with_owners(
        memory,
        0,
        GS,
        EVERY_OWNER,
        Buffer::Growing(&mut rows),
        Buffer::Growing(&mut sources),
        None,
        true,
        |pointer, _, _, admitted| {
            if admitted.map {
                names.push(capture_name_growing(
                    memory,
                    pointer,
                    admitted.name_header,
                    &mut name_bytes,
                    &mut used,
                    &mut name_required,
                ));
            }
            visits.push((pointer, [admitted.map, admitted.sources, admitted.naval]))
        },
        |pointer, leaf| scores.push((pointer, leaf.id, leaf.planes)),
        |slot, entry| owners.push((slot, entry)),
    );
    SERIAL.set(false);
    assert_eq!(
        engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::AirWingCensus) - recorded,
        u64::from(walked.row_fallbacks),
        "every fallback is counted for status"
    );
    rows.truncate(walked.shared.map.len());
    sources.truncate(walked.shared.sources.len());
    name_bytes.truncate(used);
    (
        Census {
            shared: walked.shared,
            score: walked.score,
            ended: walked.ended,
            rows,
            sources,
            names,
            name_bytes,
            name_required,
            visits,
            scores,
            owners,
        },
        walked.row_fallbacks,
    )
}

struct Faulted<'a> {
    bytes: &'a [u8],
    faults: Vec<usize>,
    once: bool,
    met: std::cell::RefCell<Vec<bool>>,
}

impl<'a> Faulted<'a> {
    fn new(bytes: &'a [u8], faults: &[usize], once: bool) -> Self {
        Self {
            bytes,
            faults: faults.to_vec(),
            once,
            met: std::cell::RefCell::new(vec![false; faults.len()]),
        }
    }
}

impl Memory for Faulted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let mut met = self.met.borrow_mut();
        for (&fault, met) in self.faults.iter().zip(met.iter_mut()) {
            if address <= fault && fault < address + destination.len() && !*met {
                *met = self.once;
                return false;
            }
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

const CAPACITIES: [Capacities; 4] = [None, Some([1, 2]), Some([70, 0]), Some([0, 0])];

#[test]
fn gathered_census_faults_refuse_no_more_than_the_per_object_reads() {
    let scale = Scale::new(100, |country| 1 + country % 2);
    let mut faults = vec![
        scale.region,
        scale.region + cstrategic_region::ID,
        scale.heap_text(3),
        scale.heap_text(66) + Scale::HEAP_NAME,
    ];
    for ordinal in [0, 1, 63, 64, 65, scale.wings - 1] {
        let wing = scale.wing_pointers[ordinal];
        faults.extend([
            scale.slots[ordinal],
            wing,
            wing + SECONDARY + 8,
            wing + PLANE_COUNT,
            wing + MISSION + air_mission::REGION,
            wing + ELIGIBILITY_COUNTER,
            wing + COUNTRY_OVERRIDE,
            wing + NAME,
            wing + NAME + engine_native_memory::msvc_string::LENGTH,
        ]);
    }
    for country in [0, 40, 99] {
        let pool = scale.pools[country];
        faults.extend([
            scale.catalogue[country],
            scale.links[country],
            scale.countries[country],
            scale.countries[country] + ccountry::ID,
            scale.owners[country],
            scale.owners[country] + cstrategic_air::COUNTRY,
            pool,
            pool + cair_wing_pool::AIR_BASE_REFERENCE,
            pool + cair_wing_pool::WINGS,
        ]);
    }
    for capacities in CAPACITIES {
        let healthy = census(&scale.memory(), true, capacities).0;
        for &fault in &faults {
            let run = |serial, once| {
                census(
                    &Faulted::new(&scale.bytes, &[fault], once),
                    serial,
                    capacities,
                )
                .0
            };
            let reference = run(true, false);
            if capacities.is_none() {
                assert_ne!(reference, healthy, "{fault:x}");
            }
            assert_eq!(run(false, false), reference, "{fault:x} {capacities:?}");
            let once = run(false, true);
            assert!(
                once == healthy || once == run(true, true),
                "{fault:x} {capacities:?}: {once:?}"
            );
        }
    }
    let wing = scale.wing_pointers[64];
    let healthy = census(&scale.memory(), true, None).0;
    assert_eq!(
        census(
            &Faulted::new(&scale.bytes, &[wing + NAME], true),
            false,
            None
        ),
        (healthy, 1)
    );
}

#[test]
fn a_pointer_run_fault_at_its_last_pointer_gives_the_parents_prefix() {
    let scale = Scale::new(3, |country| if country == 1 { 64 } else { 1 });
    let faulted = || Faulted::new(&scale.bytes, &[scale.slots[63]], false);
    let (reference, _) = census(&faulted(), true, None);
    assert_eq!(reference.visits.len(), 63);
    assert_eq!(
        reference.shared.map,
        Outcome::Unavailable {
            reason: "air wing roster pointer unreadable or address overflow"
        }
    );
    assert_eq!(census(&faulted(), false, None), (reference, 1));
}

#[test]
fn a_gather_stopped_in_a_name_keeps_the_row_its_refusal_and_the_demand() {
    use crate::raid_tooltip_operands::NameReadError;
    let scale = Scale::new(40, |_| 2);
    for (ordinal, fault, reason) in [
        (
            10,
            scale.wing_pointers[10] + NAME + engine_native_memory::msvc_string::LENGTH,
            NameReadError::Unreadable,
        ),
        (
            31,
            scale.heap_text(31) + Scale::HEAP_NAME,
            NameReadError::Unreadable,
        ),
    ] {
        let faulted = || Faulted::new(&scale.bytes, &[fault], false);
        let (reference, _) = census(&faulted(), true, None);
        assert_eq!(reference.rows.len(), scale.wings);
        assert_eq!(
            reference.names[ordinal],
            Err(NameFailure::Native(reason)),
            "{fault:x}"
        );
        let fallbacks = u32::from(ordinal == 10);
        assert_eq!(
            census(&faulted(), false, None),
            (reference, fallbacks),
            "{fault:x}"
        );
    }
}

#[test]
fn a_repeated_wing_across_a_replay_boundary_keeps_its_occurrences() {
    let mut scale = Scale::new(2, |country| if country == 1 { 66 } else { 1 });
    let repeated = scale.wing_pointers[63];
    let slot = scale.slots[64];
    p64(&mut scale.bytes, slot, repeated as u64);
    let (reference, _) = census(&scale.memory(), true, None);
    assert_eq!(reference.visits[63].0, repeated);
    assert_eq!(reference.visits[64].0, repeated);
    assert_eq!(
        (
            reference.rows[63].pool_ordinal,
            reference.rows[64].pool_ordinal
        ),
        (63, 64)
    );
    assert_eq!(reference.rows[63].id, reference.rows[64].id);
    for capacities in CAPACITIES {
        assert_eq!(
            census(&scale.memory(), false, capacities),
            (census(&scale.memory(), true, capacities).0, 0),
            "{capacities:?}"
        );
    }
}

#[test]
fn a_census_ended_by_a_wing_does_not_publish_a_later_countrys_refusal() {
    let scale = Scale::new(10, |_| 1);
    let faults = [
        scale.wing_pointers[4],
        scale.owners[5] + cstrategic_air::COUNTRY,
    ];
    let faulted = || Faulted::new(&scale.bytes, &faults, false);
    let (reference, _) = census(&faulted(), true, None);
    assert!(reference.ended);
    assert_eq!(
        reference.shared.map,
        Outcome::Unavailable {
            reason: "air wing primary vtable mismatch"
        }
    );
    assert_eq!(reference.owners.last().map(|owner| owner.0), Some(5));
    assert_eq!(census(&faulted(), false, None), (reference, 1));
}

#[test]
fn gathered_name_header_captures_as_the_whole_header_read() {
    use engine_native_memory::msvc_string;
    let text = vec![b'y'; 40];
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
    let capture = |header| {
        let memory = Traced {
            bytes: &bytes,
            reads: Default::default(),
            fault: None,
        };
        let mut storage = Vec::new();
        let (mut used, mut required) = (0, 0);
        let span = capture_name_growing(
            &memory,
            WING_A,
            header,
            &mut storage,
            &mut used,
            &mut required,
        );
        (span, storage, memory.reads.into_inner())
    };
    let read = capture(None);
    let gathered = capture(name_header(&bytes, WING_A));
    assert_eq!(read.0, Ok((0, text.len())));
    assert_eq!((read.0, &read.1), (gathered.0, &gathered.1));
    assert_eq!(read.2[0], (WING_A + NAME, msvc_string::STRIDE));
    assert_eq!(read.2[1..], gathered.2[..]);
}

#[test]
fn a_slot_zero_owner_and_catalogue_faults_refuse_no_more_than_the_parent() {
    let scale = Scale::build(80, |country| 1 + country % 2, true);
    assert_eq!(scale.owners.len(), 81);
    let (healthy, fallbacks) = census(&scale.memory(), false, None);
    assert_eq!(fallbacks, 0);
    assert_eq!(healthy, census(&scale.memory(), true, None).0);
    assert_eq!(healthy.owners[0], (0, Some(scale.owners[0])));
    assert_eq!(healthy.rows[0].owner, 0);
    let planes = scale.wing_pointers[1] + PLANE_COUNT;
    let (faulted, _) = census(
        &Faulted::new(&scale.bytes, &[planes], false),
        false,
        Some([0; 2]),
    );
    assert_eq!(
        faulted.scores[1].2,
        Err("game score air-wing plane count unreadable")
    );
    let mut faults = vec![planes];
    for country in [0, 1, 60] {
        faults.extend([
            scale.catalogue[country],
            scale.links[country],
            scale.countries[country],
            scale.countries[country] + ccountry::ID,
            scale.owners[country] + cstrategic_air::COUNTRY,
        ]);
    }
    for capacities in CAPACITIES {
        let healthy = census(&scale.memory(), true, capacities).0;
        for &fault in &faults {
            let run = |serial, once| {
                census(
                    &Faulted::new(&scale.bytes, &[fault], once),
                    serial,
                    capacities,
                )
                .0
            };
            let reference = run(true, false);
            assert_eq!(run(false, false), reference, "{fault:x} {capacities:?}");
            let once = run(false, true);
            assert!(
                once == healthy || once == run(true, true),
                "{fault:x} {capacities:?}: {once:?}"
            );
        }
    }
    let faults = [scale.wing_pointers[3], scale.catalogue[5]];
    let faulted = || Faulted::new(&scale.bytes, &faults, false);
    let (reference, _) = census(&faulted(), true, None);
    assert!(reference.ended);
    assert_eq!(
        reference.shared.map,
        Outcome::Unavailable {
            reason: "air wing primary vtable mismatch"
        }
    );
    assert_eq!(census(&faulted(), false, None), (reference, 1));
}
