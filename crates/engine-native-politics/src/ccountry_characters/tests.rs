use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const C1: usize = 0x3000;
const C2: usize = 0x3800;
const OBJ1: usize = 0x4000;
const OBJ2: usize = 0x4400;
const ARRAY1: usize = 0x4800;
const ARRAY2: usize = 0x4900;
const ADVISORS: [usize; 3] = [0x5000, 0x5400, 0x5800];
const CHARACTERS: [(usize, u32); 3] = [(0x6000, 1482), (0x6100, 1451), (0x6200, 4365)];
const HEAP: usize = 0x7000;
const LONG_IDEA: &[u8] = b"GER_hjalmar_schacht_idea";

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}
fn string(bytes: &mut [u8], at: usize, value: &[u8], heap: usize) {
    if value.len() <= 15 {
        bytes[at..at + value.len()].copy_from_slice(value);
        bytes[at + value.len()] = 0;
        p64(bytes, at + 24, 15);
    } else {
        p64(bytes, at, heap as u64);
        bytes[heap..heap + value.len()].copy_from_slice(value);
        bytes[heap + value.len()] = 0;
        p64(bytes, at + 24, 31);
    }
    p64(bytes, at + 16, value.len() as u64);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x8000];
    vector(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 3);
    vector(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 3);
    p64(&mut bytes, COUNTRIES, u64::MAX);
    for (slot, (country, object)) in [(C1, OBJ1), (C2, OBJ2)].into_iter().enumerate() {
        let slot = slot + 1;
        p64(&mut bytes, COUNTRIES + slot * 8, country as u64);
        p32(&mut bytes, LINKS + slot * 4, slot as u32);
        p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, country + ccountry::ID, slot as u32);
        p64(&mut bytes, country + ccountry::CHARACTERS, object as u64);
        p64(&mut bytes, object, VTABLE_RVA.0 as u64);
        p64(&mut bytes, object + OWNER, country as u64);
    }
    vector(&mut bytes, OBJ1 + APPOINTED_ADVISORS, ARRAY1, 2);
    vector(&mut bytes, OBJ2 + APPOINTED_ADVISORS, ARRAY2, 1);
    p64(&mut bytes, ARRAY1, ADVISORS[0] as u64);
    p64(&mut bytes, ARRAY1 + 8, ADVISORS[1] as u64);
    p64(&mut bytes, ARRAY2, ADVISORS[2] as u64);
    let ideas: [&[u8]; 3] = [LONG_IDEA, b"GER_theorist", b"army_chief_x"];
    let slots: [&[u8]; 3] = [b"political_advisor", b"theorist", b"army_chief"];
    let mut heap = HEAP;
    for ((advisor, (character, id)), (idea, slot)) in ADVISORS
        .into_iter()
        .zip(CHARACTERS)
        .zip(ideas.into_iter().zip(slots))
    {
        p64(&mut bytes, advisor, cadvisor::VTABLE_RVA.0 as u64);
        p64(&mut bytes, advisor + cadvisor::CHARACTER, character as u64);
        string(&mut bytes, advisor + cadvisor::IDEA_TOKEN, idea, heap);
        heap += 0x40;
        string(&mut bytes, advisor + cadvisor::SLOT, slot, heap);
        heap += 0x40;
        p64(&mut bytes, character, ccharacter::VTABLE_RVA.0 as u64);
        p32(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE,
        );
        p32(&mut bytes, character + ccharacter::ID, id);
    }
    bytes
}

fn run(bytes: &[u8], rows: usize, text: usize) -> (NameWalk, Vec<Row>, Vec<u8>) {
    let mut output = vec![Row::default(); rows];
    let mut arena = vec![0; text];
    let result = walk(
        &SliceMemory { base: 0, bytes },
        0,
        GS,
        &mut output,
        &mut arena,
    );
    (result, output, arena)
}

#[test]
fn worker_grows_mod_sized_advisor_records_and_both_character_arrays() {
    let mut rows = Vec::new();
    let mut text = Vec::new();
    let mut active = Vec::new();
    let mut retired = Vec::new();
    for count in [50_u32, 123] {
        let mut bytes = fixture();
        let data = bytes.len();
        bytes.resize(data + count as usize * 8, 0);
        for ordinal in 0..count {
            p64(&mut bytes, data + ordinal as usize * 8, ADVISORS[0] as u64);
        }
        for object in [OBJ1, OBJ2] {
            for offset in [APPOINTED_ADVISORS, AVAILABLE_ADVISORS] {
                vector(&mut bytes, object + offset, data, count);
            }
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut rows, &mut text).outcome,
            Outcome::Rows {
                len: 2 * count as usize
            }
        );
        assert_eq!(rows[2 * count as usize - 1].ordinal, count - 1);
        assert_eq!(
            walk_available(&memory, 0, GS, &mut rows, &mut text).outcome,
            Outcome::Rows {
                len: 2 * count as usize
            }
        );
        assert!(
            rows[..2 * count as usize]
                .iter()
                .all(|row| row.slot_len == 0)
        );
        let data = bytes.len();
        bytes.resize(data + count as usize * ACTIVE_RECORD_STRIDE, 0);
        for ordinal in 0..count {
            p64(
                &mut bytes,
                data + ordinal as usize * ACTIVE_RECORD_STRIDE,
                CHARACTERS[0].0 as u64,
            );
        }
        for object in [OBJ1, OBJ2] {
            for offset in [ACTIVE_CHARACTERS, RETIRED_CHARACTERS] {
                vector(&mut bytes, object + offset, data, count);
            }
        }
        assert_eq!(
            walk_records(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut active,
                &mut retired
            ),
            [Outcome::Rows {
                len: 2 * count as usize
            }; 2]
        );
        assert_eq!(active[2 * count as usize - 1].ordinal, count - 1);
        assert_eq!(
            retired[2 * count as usize - 1].character,
            Some(CHARACTERS[0].1)
        );
    }
}

#[test]
fn available_advisors_keep_their_own_order_without_reading_slot_bytes() {
    let mut bytes = fixture();
    vector(&mut bytes, OBJ1 + AVAILABLE_ADVISORS, ARRAY1, 2);
    vector(&mut bytes, OBJ2 + AVAILABLE_ADVISORS, ARRAY2, 1);
    p64(&mut bytes, ARRAY1, ADVISORS[1] as u64);
    p64(&mut bytes, ARRAY1 + 8, ADVISORS[0] as u64);
    for advisor in ADVISORS {
        p64(&mut bytes, advisor + cadvisor::SLOT + 16, u64::MAX);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 3];
    let mut arena = vec![0; 128];
    let result = walk_available(&memory, 0, GS, &mut rows, &mut arena);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        rows.iter().map(|row| (row.country, row.ordinal, row.character)).collect::<Vec<_>>(),
        [(1, 0, 1451), (1, 1, 1482), (2, 0, 4365)]
    );
    assert!(rows.iter().all(|row| row.slot_len == 0));
    let demand = demand_available(&memory, 0, GS).unwrap();
    assert_eq!(demand, (3, result.bytes_len));
    p64(&mut bytes, ARRAY1, 0);
    assert!(matches!(
        walk_available(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut rows,
            &mut arena,
        )
        .outcome,
        Outcome::Unavailable {
            reason: "available advisor pointer null",
        }
    ));
}

#[test]
fn available_advisor_walk_header_demand_and_selected_slices_follow_countries() {
    const COUNT: usize = 30;
    const COUNTRY_ARRAY: usize = 0x1_0000;
    const LINK_ARRAY: usize = 0x2_0000;
    const OBJECTS: usize = 0x3_0000;
    const STRIDE: usize = 0x1800;
    let mut bytes = vec![0; OBJECTS + COUNT * STRIDE];
    vector(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        COUNTRY_ARRAY,
        COUNT as u32 + 1,
    );
    vector(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        LINK_ARRAY,
        COUNT as u32 + 1,
    );
    for index in 0..COUNT {
        let country = OBJECTS + index * STRIDE;
        let object = country + 0x1000;
        let array = country + 0x1100;
        let advisor = country + 0x1200;
        let character = country + 0x1400;
        let id = index as u32 + 1;
        p64(&mut bytes, COUNTRY_ARRAY + (index + 1) * 8, country as u64);
        p32(&mut bytes, LINK_ARRAY + (index + 1) * 4, id);
        p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, country + ccountry::ID, id);
        p64(&mut bytes, country + ccountry::CHARACTERS, object as u64);
        p64(&mut bytes, object, VTABLE_RVA.0 as u64);
        p64(&mut bytes, object + OWNER, country as u64);
        vector(&mut bytes, object + AVAILABLE_ADVISORS, array, 1);
        p64(&mut bytes, array, advisor as u64);
        p64(&mut bytes, advisor, cadvisor::VTABLE_RVA.0 as u64);
        p64(&mut bytes, advisor + cadvisor::CHARACTER, character as u64);
        string(&mut bytes, advisor + cadvisor::IDEA_TOKEN, b"idea", 0);
        p64(&mut bytes, character, ccharacter::VTABLE_RVA.0 as u64);
        p32(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE,
        );
        p32(&mut bytes, character + ccharacter::ID, id + COUNT as u32);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        demand_available(&memory, 0, GS).unwrap(),
        (COUNT, COUNT * 4)
    );
    let mut rows = vec![Row::default(); COUNT];
    let mut arena = vec![0; COUNT * 4];
    let result = walk_available(&memory, 0, GS, &mut rows, &mut arena);
    assert_eq!(result.outcome, Outcome::Rows { len: COUNT });
    assert_eq!(rows[COUNT - 1].country, COUNT as u32);
    assert_eq!(rows[COUNT - 1].character, COUNT as u32 * 2);
    assert_eq!(result.bytes_len, COUNT * 4);
    let identity = |row: &Row, text: &[u8]| {
        (
            row.country,
            row.ordinal,
            row.character,
            text[row.idea_start..row.idea_start + row.idea_len].to_vec(),
        )
    };
    let whole: Vec<_> = rows.iter().map(|row| identity(row, &arena)).collect();
    let mut covered = 0;
    for slice in 0..24_u32 {
        let select = |country: u32| country % 24 == slice;
        let mut selected = Vec::new();
        let mut text = Vec::new();
        let walk = walk_available_carried(
            &memory,
            0,
            GS,
            &mut selected,
            &mut text,
            &mut Fresh,
            &select,
        );
        let expected: Vec<_> = whole.iter().filter(|row| select(row.0)).cloned().collect();
        assert_eq!(
            walk.outcome,
            Outcome::Rows {
                len: expected.len()
            }
        );
        let actual: Vec<_> = selected[..expected.len()]
            .iter()
            .map(|row| identity(row, &text))
            .collect();
        assert_eq!(actual, expected, "slice {slice}");
        covered += expected.len();
    }
    assert_eq!(covered, COUNT);
}

#[test]
fn appointed_order_identities_and_exact_strings_survive() {
    let bytes = fixture();
    let (result, rows, arena) = run(&bytes, 4, 256);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(result.bytes_len, result.required_bytes);
    let text = |start: usize, len: usize| &arena[start..start + len];
    assert_eq!(
        rows[..3]
            .iter()
            .map(|row| (row.country, row.ordinal, row.character))
            .collect::<Vec<_>>(),
        [(1, 0, 1482), (1, 1, 1451), (2, 0, 4365)]
    );
    assert_eq!(text(rows[0].idea_start, rows[0].idea_len), LONG_IDEA);
    assert_eq!(
        text(rows[0].slot_start, rows[0].slot_len),
        b"political_advisor"
    );
    assert_eq!(text(rows[1].idea_start, rows[1].idea_len), b"GER_theorist");
    assert_eq!(text(rows[2].slot_start, rows[2].slot_len), b"army_chief");
}

#[test]
fn a_missing_character_object_or_empty_list_has_no_rows() {
    let mut bytes = fixture();
    p64(&mut bytes, C1 + ccountry::CHARACTERS, 0);
    vector(&mut bytes, OBJ2 + APPOINTED_ADVISORS, 0, 0);
    let (result, ..) = run(&bytes, 4, 256);
    assert_eq!(result.outcome, Outcome::Rows { len: 0 });
}

#[test]
fn unauthenticated_objects_and_short_reads_fail_the_table() {
    for (address, value) in [
        (OBJ1, 1_u64),
        (OBJ2 + OWNER, C1 as u64),
        (ARRAY1 + 8, 0),
        (ADVISORS[1], 7),
        (ADVISORS[1] + cadvisor::CHARACTER, 0),
        (CHARACTERS[2].0, 7),
        (OBJ1 + APPOINTED_ADVISORS, 0x9_0000),
        (ARRAY2, 0x9_0000),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, address, value);
        let (result, ..) = run(&bytes, 4, 256);
        assert!(
            matches!(result.outcome, Outcome::Unavailable { .. }),
            "{address:#x}: {result:?}"
        );
        assert_eq!((result.bytes_len, result.required_bytes), (0, 0));
    }
    let mut bytes = fixture();
    p32(&mut bytes, CHARACTERS[0].0 + ccharacter::ID, 0);
    assert!(matches!(
        run(&bytes, 4, 256).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut bytes = fixture();
    bytes[HEAP + LONG_IDEA.len()] = b'!';
    assert!(matches!(
        run(&bytes, 4, 256).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut bytes = fixture();
    p32(&mut bytes, OBJ2 + APPOINTED_ADVISORS + 12, 5);
    assert!(matches!(
        run(&bytes, 4, 256).0.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn a_declared_string_length_alone_never_sizes_growth() {
    struct FaultMemory<'a> {
        memory: SliceMemory<'a>,
        hole: std::ops::Range<usize>,
    }
    impl Memory for FaultMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            if address < self.hole.end && end > self.hole.start {
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    let declared = 1 << 20;
    let far = 0x10000;
    let mut bytes = fixture();
    bytes.resize(far + declared + 1, 0);
    let address = ADVISORS[0] + cadvisor::IDEA_TOKEN;
    p64(&mut bytes, address, far as u64);
    p64(&mut bytes, address + 16, declared as u64);
    p64(&mut bytes, address + 24, declared as u64 + 15);
    let run = |hole: std::ops::Range<usize>| {
        let memory = FaultMemory {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            hole,
        };
        let mut output = vec![Row::default(); 4];
        let mut arena = vec![0; 8];
        walk(&memory, 0, GS, &mut output, &mut arena)
    };
    let result = run(far..far + declared);
    assert!(
        matches!(result.outcome, Outcome::Unavailable { .. }),
        "{result:?}"
    );
}

const ACTIVE1: usize = 0x4a00;
fn active_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    vector(&mut bytes, OBJ1 + ACTIVE_CHARACTERS, ACTIVE1, 3);
    p64(&mut bytes, ACTIVE1, CHARACTERS[1].0 as u64);
    p64(&mut bytes, ACTIVE1 + ACTIVE_RECORD_STRIDE, 0);
    p64(
        &mut bytes,
        ACTIVE1 + 2 * ACTIVE_RECORD_STRIDE,
        CHARACTERS[0].0 as u64,
    );
    bytes[ACTIVE1 + 8..ACTIVE1 + 12].copy_from_slice(&[1, 0, 1, 0]);
    bytes
}

fn run_active(bytes: &[u8], rows: usize) -> (Outcome, Vec<ActiveRow>) {
    let mut output = vec![ActiveRow::default(); rows];
    let [outcome, _] = walk_records(&SliceMemory { base: 0, bytes }, 0, GS, &mut output, &mut Vec::new());
    (outcome, output)
}

const RETIRED2: usize = 0x4b00;
fn retired_fixture() -> Vec<u8> {
    let mut bytes = active_fixture();
    vector(&mut bytes, OBJ2 + RETIRED_CHARACTERS, RETIRED2, 1);
    p64(&mut bytes, RETIRED2, CHARACTERS[0].0 as u64);
    bytes
}

#[test]
fn retired_records_walk_beside_the_active_ones() {
    let bytes = retired_fixture();
    let (mut active, mut retired) = (vec![ActiveRow::default(); 4], vec![ActiveRow::default(); 4]);
    let outcomes = walk_records(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut active,
        &mut retired,
    );
    assert_eq!(
        outcomes,
        [Outcome::Rows { len: 3 }, Outcome::Rows { len: 1 }]
    );
    assert_eq!(
        retired[0],
        ActiveRow {
            country: 2,
            ordinal: 0,
            character: Some(1482)
        }
    );
    let outcomes = count_records(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
    );
    assert_eq!(
        outcomes,
        [
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            },
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        ]
    );
}

#[test]
fn an_unreadable_retired_array_fails_only_its_table() {
    let mut bytes = retired_fixture();
    p32(&mut bytes, OBJ2 + RETIRED_CHARACTERS + 12, 5);
    let mut active = vec![ActiveRow::default(); 4];
    let [active_outcome, retired_outcome] = walk_records(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut active,
        &mut vec![ActiveRow::default(); 4],
    );
    assert_eq!(active_outcome, Outcome::Rows { len: 3 });
    assert!(matches!(retired_outcome, Outcome::Unavailable { .. }));
    let mut bytes = retired_fixture();
    p64(&mut bytes, OBJ2, 1);
    let outcomes = walk_records(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut active,
        &mut vec![ActiveRow::default(); 4],
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
    );
}

#[test]
fn a_table_keeps_its_own_first_failure_before_a_shared_one() {
    let mut bytes = retired_fixture();
    vector(&mut bytes, OBJ1 + RETIRED_CHARACTERS, RETIRED2, 1);
    p32(&mut bytes, OBJ1 + RETIRED_CHARACTERS + 12, 5);
    p64(&mut bytes, OBJ2, 1);
    let outcomes = walk_records(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        GS,
        &mut vec![ActiveRow::default(); 4],
        &mut vec![ActiveRow::default(); 4],
    );
    assert_eq!(
        outcomes,
        [
            Outcome::Unavailable {
                reason: "country characters vtable mismatch"
            },
            Outcome::Unavailable {
                reason: "character record array unreadable or invalid"
            }
        ]
    );
}

#[test]
fn active_records_keep_native_order_and_null_records() {
    let (outcome, rows) = run_active(&active_fixture(), 4);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        rows[..3],
        [
            ActiveRow {
                country: 1,
                ordinal: 0,
                character: Some(1451)
            },
            ActiveRow {
                country: 1,
                ordinal: 1,
                character: None
            },
            ActiveRow {
                country: 1,
                ordinal: 2,
                character: Some(1482)
            },
        ]
    );
}

#[test]
fn a_missing_object_or_empty_active_array_has_no_rows() {
    let mut bytes = active_fixture();
    p64(&mut bytes, C1 + ccountry::CHARACTERS, 0);
    assert_eq!(run_active(&bytes, 4).0, Outcome::Rows { len: 0 });
    let mut bytes = active_fixture();
    vector(&mut bytes, OBJ1 + ACTIVE_CHARACTERS, 0, 0);
    assert_eq!(run_active(&bytes, 4).0, Outcome::Rows { len: 0 });
}

#[test]
fn active_rows_count_the_live_total_from_the_headers_alone() {
    let mut bytes = active_fixture();
    vector(&mut bytes, OBJ2 + ACTIVE_CHARACTERS, 0x9_0000, 4);
    assert_eq!(
        count_records(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
        )[0],
        Outcome::Truncated {
            len: 0,
            unread_rows: 7
        }
    );
}

#[test]
fn unauthenticated_active_records_fail_the_table() {
    for (address, value) in [
        (OBJ1, 1_u64),
        (OBJ2 + OWNER, C1 as u64),
        (OBJ1 + ACTIVE_CHARACTERS, 0x9_0000),
        (CHARACTERS[0].0, 7),
        (CHARACTERS[1].0 + ccharacter::TYPE, 74),
    ] {
        let mut bytes = active_fixture();
        p64(&mut bytes, address, value);
        let (outcome, _) = run_active(&bytes, 4);
        assert!(
            matches!(outcome, Outcome::Unavailable { .. }),
            "{address:#x}: {outcome:?}"
        );
    }
    let mut bytes = active_fixture();
    p32(&mut bytes, OBJ1 + ACTIVE_CHARACTERS + 12, 5);
    assert!(matches!(
        run_active(&bytes, 4).0,
        Outcome::Unavailable { .. }
    ));
}

struct Guarded<'a> {
    bytes: &'a [u8],
    guards: std::cell::Cell<usize>,
}

impl Memory for Guarded<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        let memory = SliceMemory {
            base: 0,
            bytes: self.bytes,
        };
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !destination.is_empty() && (address == 0 || !memory.copy(address, destination)) {
                return index;
            }
        }
        N
    }
}

fn many_active_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    vector(&mut bytes, OBJ1 + ACTIVE_CHARACTERS, ACTIVE1, 30);
    for record in 0..30 {
        p64(
            &mut bytes,
            ACTIVE1 + record * ACTIVE_RECORD_STRIDE,
            CHARACTERS[record % 3].0 as u64,
        );
    }
    bytes
}

#[test]
fn the_first_faulty_active_record_decides_the_reason() {
    let unreadable = 0x9_0000_u64;
    for (records, reason) in [
        (
            [unreadable, CHARACTERS[1].0 as u64 + 1],
            "character identity unreadable",
        ),
        (
            [CHARACTERS[1].0 as u64 + 1, unreadable],
            "character vtable or reference type mismatch",
        ),
    ] {
        let mut bytes = active_fixture();
        p64(&mut bytes, ACTIVE1, records[0]);
        p64(&mut bytes, ACTIVE1 + ACTIVE_RECORD_STRIDE, records[1]);
        assert_eq!(
            run_active(&bytes, 4).0,
            Outcome::Unavailable { reason },
            "{records:x?}"
        );
    }
}
