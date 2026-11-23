use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;
use super::*;
use crate::{cgame_state, memory::SliceMemory};

const GS: usize = 0x1000;
const COUNTRIES: [usize; 2] = [0x4000, 0x6000];
const DIPLOMACY: [usize; 2] = [0x8000, 0x9000];
const RELATION: usize = 0x18000;
const RETIRED_RELATION: usize = 0x19000;
const DEFINITION: usize = 0x1b000;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, values.len() as u32);
    p32(bytes, at + 12, values.len() as u32);
    for (index, value) in values.iter().enumerate() {
        p64(bytes, data + index * 8, *value);
    }
}
fn status(owner: usize, target: usize) -> usize {
    0xb000 + owner * 0x5000 + target * 0xa00
}
fn owned(owner: usize) -> usize {
    0xa100 + owner * 0x100
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x20000];
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        0x3000,
        &[
            u64::MAX,
            COUNTRIES[0] as u64,
            COUNTRIES[1] as u64,
            COUNTRIES[0] as u64,
        ],
    );
    p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
    for (index, value) in [0, 1, 2, 1].into_iter().enumerate() {
        p32(&mut bytes, 0x3100 + index * 4, value);
    }
    for (owner, diplomacy) in DIPLOMACY.into_iter().enumerate() {
        p64(&mut bytes, COUNTRIES[owner], ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, COUNTRIES[owner] + ccountry::ID, [3, 2][owner]);
        p64(
            &mut bytes,
            COUNTRIES[owner] + ccountry::DIPLOMACY_STATUS,
            DIPLOMACY[owner] as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY[owner],
            cdiplomacy_status::VTABLE_RVA.0 as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::COUNTRY,
            COUNTRIES[owner] as u64,
        );
        let entries: Vec<_> = (0..4).map(|target| status(owner, target) as u64).collect();
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::RELATIONS,
            0xa000 + owner * 0x40,
            &entries,
        );
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            owned(owner),
            &[RELATION as u64, RETIRED_RELATION as u64, 0x1a000],
        );
        for target in 0..4 {
            let entry = status(owner, target);
            p64(&mut bytes, entry, crelation_status::VTABLE_RVA.0 as u64);
            p64(
                &mut bytes,
                entry + crelation_status::DIPLOMACY_STATUS,
                DIPLOMACY[owner] as u64,
            );
            p32(
                &mut bytes,
                entry + crelation_status::TARGET_COUNTRY,
                [0, 3, 2, 0][target],
            );
        }
        let entry = status(owner, [2, 1][owner]);
        p64(
            &mut bytes,
            entry + crelation_status::SUBJECT_RELATION,
            RELATION as u64,
        );
        array(
            &mut bytes,
            entry + crelation_status::SUBRELATIONS,
            entry + 0x400,
            &[RELATION as u64],
        );
        if owner == 0 {
            array(
                &mut bytes,
                entry + crelation_status::SERIALIZABLE_RELATIONS,
                entry + 0x420,
                &[RELATION as u64],
            );
        }
    }
    p64(&mut bytes, RELATION, VTABLE_RVA.0 as u64);
    p32(&mut bytes, RELATION + TYPE_ID, TYPE_VALUE);
    p32(&mut bytes, RELATION + OVERLORD, 3);
    p32(&mut bytes, RELATION + SUBJECT, 2);
    p32(&mut bytes, RELATION + START_HOUR, i32::MIN as u32);
    p32(&mut bytes, RELATION + crelation::END_HOUR, i32::MAX as u32);
    bytes[RELATION + crelation::CANCEL] = 255;
    p64(&mut bytes, RELATION + AUTONOMY_RATIO, i64::MIN as u64);
    p64(&mut bytes, RELATION + AUTONOMY_STATE, DEFINITION as u64);
    let name = DEFINITION + autonomy_state_definition::NAME;
    bytes[name..name + 4].copy_from_slice(&[b'a', 255, 0, b'z']);
    p64(&mut bytes, name + msvc_string::LENGTH, 4);
    p64(&mut bytes, name + msvc_string::CAPACITY, 15);
    p64(&mut bytes, RETIRED_RELATION, VTABLE_RVA.0 as u64);
    p32(&mut bytes, RETIRED_RELATION + TYPE_ID, TYPE_VALUE);
    bytes[RETIRED_RELATION + crelation::RETIRED] = 7;
    p32(&mut bytes, 0x1a000 + TYPE_ID, 0x1234);
    bytes
}

fn capture(bytes: &[u8], matrix: bool, output: &mut Vec<Row>, names: &mut Vec<u8>) -> NameWalk {
    let memory = SliceMemory { base: 0, bytes };
    if matrix {
        walk_matrix(&memory, 0, GS, output, names)
    } else {
        walk(&memory, 0, GS, output, names)
    }
}

#[test]
fn censuses_preserve_native_direction_full_width_and_literal_bytes() {
    let bytes = fixture();
    for matrix in [false, true] {
        let mut rows = vec![Row::default(); 2];
        let mut names = vec![0; 20];
        assert_eq!(
            capture(&bytes, matrix, &mut rows, &mut names),
            NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 4,
                required_bytes: 4
            }
        );
        assert_eq!(
            rows[0],
            Row {
                overlord: 3,
                subject: 2,
                start_hour: i32::MIN,
                end_hour: i32::MAX,
                cancel: 255,
                autonomy_ratio: i64::MIN,
                state: Name { start: 0, len: 4 }
            }
        );
        assert_eq!(&names[..4], &[b'a', 255, 0, b'z']);
    }
}

#[test]
fn a_retired_subject_counts_while_it_is_installed() {
    let mut bytes = fixture();
    p32(&mut bytes, RETIRED_RELATION + OVERLORD, 3);
    p32(&mut bytes, RETIRED_RELATION + SUBJECT, 2);
    bytes[RELATION + crelation::RETIRED] = 1;
    for matrix in [false, true] {
        let mut rows = vec![Row::default(); 2];
        let mut names = vec![0; 20];
        assert_eq!(
            capture(&bytes, matrix, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
    }
}

#[test]
fn an_unreadable_installation_slot_is_not_an_uninstalled_subject() {
    struct Hole<'a> {
        bytes: &'a [u8],
        hole: std::ops::Range<usize>,
    }
    impl Memory for Hole<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            !(address < self.hole.end && self.hole.start < address.saturating_add(output.len()))
                && SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
        }
    }
    let mut bytes = fixture();
    bytes[RELATION + crelation::RETIRED] = 1;
    let mut rows = vec![Row::default(); 2];
    let mut names = vec![0; 20];
    assert_eq!(
        capture(&bytes, false, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
    let slot = status(0, 2) + crelation_status::SUBJECT_RELATION;
    let memory = Hole {
        bytes: &bytes,
        hole: slot..slot + 8,
    };
    assert_eq!(
        walk(&memory, 0, GS, &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "subject field unreadable or address overflow"
        }
    );
}

#[test]
fn the_count_probe_reports_exact_required_bytes() {
    let bytes = fixture();
    assert_eq!(
        count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
        NameWalk {
            outcome: Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            bytes_len: 0,
            required_bytes: 4
        }
    );
}

#[test]
fn matrix_census_can_detect_a_relation_missing_from_both_owned_roots() {
    let mut bytes = fixture();
    for diplomacy in DIPLOMACY {
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::OWNED_RELATIONS,
            0,
            &[],
        );
    }
    let mut rows = vec![Row::default(); 1];
    let mut names = vec![0; 4];
    assert_eq!(
        capture(&bytes, false, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
}

#[test]
fn matrix_batches_authenticate_empty_tail_cells_and_do_not_read_past_count() {
    let mut bytes = fixture();
    bytes.resize(0x24000, 0);
    for (owner, diplomacy) in DIPLOMACY.into_iter().enumerate() {
        let mut entries = vec![status(owner, 0) as u64; 65];
        for (index, entry) in entries[..4].iter_mut().enumerate() {
            *entry = status(owner, index) as u64;
        }
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::RELATIONS,
            0x20000 + owner * 0x400,
            &entries,
        );
    }
    let mut rows = vec![Row::default(); 1];
    let mut names = vec![0; 4];
    assert_eq!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
    p64(&mut bytes, 0x20000 + 64 * 8, 0x22000);
    p64(&mut bytes, 0x22000, crelation_status::VTABLE_RVA.0 as u64);
    assert!(matches!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "relation status diplomacy mismatch"
        }
    ));
    p64(
        &mut bytes,
        0x22000 + crelation_status::DIPLOMACY_STATUS,
        DIPLOMACY[0] as u64,
    );
    assert_eq!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
    p64(&mut bytes, 0x20000 + 65 * 8, u64::MAX);
    assert_eq!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
}

#[test]
fn mismatched_ownership_target_and_invalid_names_are_unavailable() {
    for corruption in 0..5 {
        let mut bytes = fixture();
        match corruption {
            0 => p64(
                &mut bytes,
                status(1, 1) + crelation_status::SUBJECT_RELATION,
                0,
            ),
            1 => p32(
                &mut bytes,
                status(0, 2) + crelation_status::TARGET_COUNTRY,
                3,
            ),
            2 => array(
                &mut bytes,
                DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS,
                owned(1),
                &[RELATION as u64, RELATION as u64],
            ),
            3 => p64(&mut bytes, RELATION + AUTONOMY_STATE, 0),
            _ => p64(
                &mut bytes,
                DEFINITION + autonomy_state_definition::NAME + msvc_string::LENGTH,
                16,
            ),
        }
        assert!(
            matches!(
                capture(&bytes, false, &mut vec![Row::default(); 1], &mut vec![0; 40]).outcome,
                Outcome::Unavailable { .. }
            ),
            "corruption {corruption}"
        );
    }
}

#[test]
fn heap_names_have_no_fixed_length_limit_or_lossy_conversion() {
    let mut bytes = fixture();
    let name = DEFINITION + autonomy_state_definition::NAME;
    p64(&mut bytes, name, 0x1c000);
    p64(&mut bytes, name + msvc_string::LENGTH, 128);
    p64(&mut bytes, name + msvc_string::CAPACITY, 128);
    bytes[0x1c000..0x1c080].fill(255);
    let mut names = vec![0; 128];
    assert_eq!(
        capture(&bytes, false, &mut vec![Row::default(); 1], &mut names),
        NameWalk {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: 128,
            required_bytes: 128
        }
    );
    assert_eq!(names, [255; 128]);
    bytes[0x1c080] = 1;
    assert!(matches!(
        capture(&bytes, false, &mut vec![Row::default(); 1], &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
}

struct Space {
    bytes: Vec<u8>,
}

impl Space {
    fn alloc(&mut self, size: usize) -> usize {
        let at = (self.bytes.len() + 15) & !15;
        self.bytes.resize(at + size, 0);
        at
    }

    fn vector(&mut self, header: usize, values: &[u64]) {
        let data = if values.is_empty() {
            0
        } else {
            self.alloc(values.len() * 8)
        };
        array(&mut self.bytes, header, data, values);
    }
}

fn overlord_census(countries: usize) -> (Vec<u8>, Vec<Row>) {
    overlord_census_of(countries, |_| true)
}

fn overlord_census_of(countries: usize, is_subject: impl Fn(usize) -> bool) -> (Vec<u8>, Vec<Row>) {
    let mut space = Space {
        bytes: vec![0; 0x2000],
    };
    let census = space.alloc((countries + 1) * 8);
    let links = space.alloc((countries + 1) * 4);
    let objects = space.alloc(24 * (countries + 2 + ccountry::DIPLOMACY_STATUS / 24));
    let country = |slot: usize| objects + 24 * slot;
    let mut pointers = vec![u64::MAX];
    pointers.extend((1..=countries).map(|slot| country(slot) as u64));
    array(
        &mut space.bytes,
        GS + cgame_state::COUNTRIES,
        census,
        &pointers,
    );
    p64(
        &mut space.bytes,
        GS + cgame_state::COUNTRY_LINKS,
        links as u64,
    );
    for offset in [8, 12] {
        p32(
            &mut space.bytes,
            GS + cgame_state::COUNTRY_LINKS + offset,
            countries as u32 + 1,
        );
    }
    for slot in 0..=countries {
        p32(&mut space.bytes, links + slot * 4, slot as u32);
    }
    let mut diplomacy = vec![0; countries + 1];
    for (slot, root) in diplomacy.iter_mut().enumerate().skip(1) {
        let at = country(slot);
        p64(&mut space.bytes, at, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut space.bytes, at + ccountry::ID, slot as u32);
        *root = space.alloc(cdiplomacy_status::COUNTRY + 8);
        p64(
            &mut space.bytes,
            at + ccountry::DIPLOMACY_STATUS,
            *root as u64,
        );
        p64(
            &mut space.bytes,
            *root,
            cdiplomacy_status::VTABLE_RVA.0 as u64,
        );
        p64(
            &mut space.bytes,
            *root + cdiplomacy_status::COUNTRY,
            at as u64,
        );
    }
    let definition = space.alloc(0x40);
    let name = definition + autonomy_state_definition::NAME;
    space.bytes[name..name + 6].copy_from_slice(b"puppet");
    p64(&mut space.bytes, name + msvc_string::LENGTH, 6);
    p64(&mut space.bytes, name + msvc_string::CAPACITY, 15);
    let status = |space: &mut Space, owner: usize, target: usize, relation: usize, first| {
        let at = space.alloc(0x2a0);
        p64(&mut space.bytes, at, crelation_status::VTABLE_RVA.0 as u64);
        p64(
            &mut space.bytes,
            at + crelation_status::DIPLOMACY_STATUS,
            diplomacy[owner] as u64,
        );
        p32(
            &mut space.bytes,
            at + crelation_status::TARGET_COUNTRY,
            target as u32,
        );
        p64(
            &mut space.bytes,
            at + crelation_status::SUBJECT_RELATION,
            relation as u64,
        );
        space.vector(at + crelation_status::SUBRELATIONS, &[relation as u64]);
        if first {
            space.vector(
                at + crelation_status::SERIALIZABLE_RELATIONS,
                &[relation as u64],
            );
        }
        at
    };
    let mut hub_matrix = vec![0_u64; countries + 1];
    let mut hub_owned = Vec::new();
    let mut rows = Vec::new();
    for subject in (2..=countries).filter(|&slot| is_subject(slot)) {
        let relation = space.alloc(0x68);
        p64(&mut space.bytes, relation, VTABLE_RVA.0 as u64);
        p32(&mut space.bytes, relation + TYPE_ID, TYPE_VALUE);
        p32(&mut space.bytes, relation + OVERLORD, 1);
        p32(&mut space.bytes, relation + SUBJECT, subject as u32);
        p32(&mut space.bytes, relation + START_HOUR, subject as u32);
        p32(&mut space.bytes, relation + crelation::END_HOUR, u32::MAX);
        p64(&mut space.bytes, relation + AUTONOMY_RATIO, subject as u64);
        p64(
            &mut space.bytes,
            relation + AUTONOMY_STATE,
            definition as u64,
        );
        hub_matrix[subject] = status(&mut space, 1, subject, relation, true) as u64;
        let back = status(&mut space, subject, 1, relation, false);
        let matrix = space.alloc(16);
        p64(&mut space.bytes, matrix + 8, back as u64);
        p64(
            &mut space.bytes,
            diplomacy[subject] + cdiplomacy_status::RELATIONS,
            matrix as u64,
        );
        for offset in [8, 12] {
            p32(
                &mut space.bytes,
                diplomacy[subject] + cdiplomacy_status::RELATIONS + offset,
                2,
            );
        }
        space.vector(
            diplomacy[subject] + cdiplomacy_status::OWNED_RELATIONS,
            &[relation as u64],
        );
        hub_owned.push(relation as u64);
        rows.push(Row {
            overlord: 1,
            subject: subject as u32,
            start_hour: subject as i32,
            end_hour: -1,
            autonomy_ratio: subject as i64,
            state: Name {
                start: rows.len() * 6,
                len: 6,
            },
            ..Row::default()
        });
    }
    space.vector(diplomacy[1] + cdiplomacy_status::RELATIONS, &hub_matrix);
    space.vector(
        diplomacy[1] + cdiplomacy_status::OWNED_RELATIONS,
        &hub_owned,
    );
    (space.bytes, rows)
}

#[test]
fn owned_subject_walk_matches_the_plain_walk_and_scans_each_owner_once() {
    let (bytes, expected) = overlord_census(600);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut plain = vec![Row::default(); 600];
    let mut plain_names = vec![0; 600 * 6];
    let plain_walk = walk(&memory, 0, GS, &mut plain, &mut plain_names);
    assert_eq!(plain_walk.outcome, Outcome::Rows { len: 599 });
    assert_eq!(plain[..599], expected[..]);
    for grown in [false, true] {
        let mut scratch = OwnedSnapshotScratch::default();
        if grown {
            let mut rows = vec![Row::default(); 600];
            let mut names = vec![0; 600 * 6];
            walk_owned(&memory, 0, GS, &mut rows, &mut names, &mut scratch);
            scratch.grow(0).unwrap();
        }
        let mut rows = vec![Row::default(); 600];
        let mut names = vec![0; 600 * 6];
        let owned = walk_owned(&memory, 0, GS, &mut rows, &mut names, &mut scratch);
        assert_eq!(owned, plain_walk);
        assert_eq!(rows, plain);
        assert_eq!(names, plain_names);
        assert!(scratch.stats().installed > 0, "grown {grown}");
    }
}

fn mixed_census(countries: usize) -> (Vec<u8>, Array, Vec<Row>) {
    mixed_census_of(countries, |_| true)
}

fn mixed_census_of(
    countries: usize,
    is_subject: impl Fn(usize) -> bool,
) -> (Vec<u8>, Array, Vec<Row>) {
    let (bytes, rows) = overlord_census_of(countries, is_subject);
    let hub = |bytes: &[u8]| {
        let memory = SliceMemory { base: 0, bytes };
        let reader = Reader::new(&memory, 0, GS).unwrap();
        let country = reader.countries.at(1).unwrap().unwrap();
        let diplomacy = reader.diplomacy(country).unwrap();
        let owned = reader
            .array(diplomacy, cdiplomacy_status::OWNED_RELATIONS)
            .unwrap();
        let entries: Vec<u64> = (0..owned.count)
            .map(|index| reader.element(owned, index).unwrap() as u64)
            .collect();
        (diplomacy, entries)
    };
    let (diplomacy, subjects) = hub(&bytes);
    let mut space = Space { bytes };
    let mut entries = Vec::new();
    for (index, &relation) in subjects.iter().enumerate() {
        entries.push(relation);
        if index % 10 == 0 {
            space.bytes[relation as usize + crelation::RETIRED] = 1;
        }
        if index % 3 == 2 {
            let other = space.alloc(0x68);
            p32(&mut space.bytes, other + TYPE_ID, 0x30d0);
            entries.push(other as u64);
        }
    }
    space.vector(diplomacy + cdiplomacy_status::OWNED_RELATIONS, &entries);
    let owned = Array {
        data: u64::from_le_bytes(
            space.bytes[diplomacy + cdiplomacy_status::OWNED_RELATIONS
                ..diplomacy + cdiplomacy_status::OWNED_RELATIONS + 8]
                .try_into()
                .unwrap(),
        ) as usize,
        count: entries.len() as u32,
    };
    (space.bytes, owned, rows)
}

fn batched_and_serial(
    memory: &impl Memory,
    serial_memory: &impl Memory,
    rows: usize,
    names: usize,
) -> [(NameWalk, Vec<Row>, Vec<u8>); 2] {
    let mut walks = [(); 2].map(|_| (vec![Row::default(); rows], vec![0; names]));
    let [(new_rows, new_names), (old_rows, old_names)] = &mut walks;
    let new = walk_owned(
        memory,
        0,
        GS,
        new_rows,
        new_names,
        &mut OwnedSnapshotScratch::default(),
    );
    let old = baseline::walk_owned(
        serial_memory,
        0,
        GS,
        old_rows,
        old_names,
        &mut OwnedSnapshotScratch::default(),
    );
    let [(new_rows, new_names), (old_rows, old_names)] = walks;
    [(new, new_rows, new_names), (old, old_rows, old_names)]
}

#[test]
fn batched_subjects_match_the_serial_walk() {
    use crate::memory::{CountedMemory, ReadCounts};
    const COUNTRIES: usize = 200;
    let (bytes, _, expected) = mixed_census(COUNTRIES);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let [batched, serial] = batched_and_serial(&memory, &memory, COUNTRIES, COUNTRIES * 6);
    assert_eq!(batched.0.outcome, Outcome::Rows { len: COUNTRIES - 1 });
    assert_eq!(batched.1[..COUNTRIES - 1], expected[..]);
    assert_eq!(batched, serial);
    let counted = CountedMemory::new(memory);
    let mut counts = [ReadCounts::ZERO; 2];
    for (index, count) in counts.iter_mut().enumerate() {
        let prior = ReadCounts::take_current();
        let mut rows = vec![Row::default(); COUNTRIES];
        let mut names = vec![0; COUNTRIES * 6];
        let mut scratch = OwnedSnapshotScratch::default();
        if index == 0 {
            walk_owned(&counted, 0, GS, &mut rows, &mut names, &mut scratch);
        } else {
            baseline::walk_owned(&counted, 0, GS, &mut rows, &mut names, &mut scratch);
        }
        *count = ReadCounts::take_current();
        prior.add_current();
    }
    let [new, old] = counts;
    assert!(
        new.copy_calls + 8 * (COUNTRIES as u64 - 1) < old.copy_calls,
        "{new:?} {old:?}"
    );
    assert!(new.spans < old.spans, "{new:?} {old:?}");
}

fn subject_fault_sites(bytes: &[u8], owned: Array) -> [std::ops::Range<usize>; 4] {
    let entry = |index: usize| {
        let at = owned.data + index * 8;
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
    };
    let element = owned.data + 10 * 8;
    let other = entry(3) + TYPE_ID;
    let gap = entry(6) + SUBJECT + 4;
    let ratio = entry(9) + AUTONOMY_RATIO;
    [
        element..element + 8,
        other..other + 4,
        gap..gap + 4,
        ratio..ratio + 8,
    ]
}

#[test]
fn a_persistent_subject_fault_gives_the_serial_result() {
    use crate::batch_fallback::{Batch, Faulty, count};
    let (bytes, owned, _) = mixed_census(200);
    for (site, (range, counted)) in subject_fault_sites(&bytes, owned)
        .into_iter()
        .zip([1, 1, 2, 1])
        .enumerate()
    {
        let before = count(Batch::Subjects);
        let memory = Faulty::new(
            SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            vec![range],
            false,
        );
        let [batched, serial] = batched_and_serial(&memory, &memory, 200, 200 * 6);
        assert_eq!(batched, serial, "site {site}");
        assert_eq!(count(Batch::Subjects) - before, counted, "site {site}");
        if site == 2 {
            assert_eq!(batched.0.outcome, Outcome::Rows { len: 199 });
        }
    }
}

#[test]
fn a_one_shot_subject_fault_refuses_no_more_than_the_serial_walk() {
    use crate::batch_fallback::Faulty;
    let (bytes, owned, _) = mixed_census(200);
    let healthy = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let [expected, _] = batched_and_serial(&healthy, &healthy, 200, 200 * 6);
    for (site, range) in subject_fault_sites(&bytes, owned).into_iter().enumerate() {
        let memory = Faulty::new(
            SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            vec![range],
            true,
        );
        let mut rows = vec![Row::default(); 200];
        let mut names = vec![0; 200 * 6];
        let walk = walk_owned(
            &memory,
            0,
            GS,
            &mut rows,
            &mut names,
            &mut OwnedSnapshotScratch::default(),
        );
        assert_eq!((walk, rows, names), expected, "site {site}");
    }
}

fn owned_of(bytes: &[u8], slot: u32) -> (usize, Array) {
    let memory = SliceMemory { base: 0, bytes };
    let reader = Reader::new(&memory, 0, GS).unwrap();
    let country = reader.countries.at(slot).unwrap().unwrap();
    let diplomacy = reader.diplomacy(country).unwrap();
    let owned = reader
        .array(diplomacy, cdiplomacy_status::OWNED_RELATIONS)
        .unwrap();
    (diplomacy, owned)
}

#[test]
fn batched_subjects_stop_where_the_serial_walk_stops() {
    let (healthy, hub, _) = mixed_census(200);
    let mut named = Space {
        bytes: healthy.clone(),
    };
    let long = named.alloc(0x40);
    let empty = named.alloc(0x40);
    let heap = named.alloc(32);
    named.bytes[heap..heap + 22].copy_from_slice(b"a_much_longer_autonomy");
    let name = long + autonomy_state_definition::NAME;
    p64(&mut named.bytes, name, heap as u64);
    p64(&mut named.bytes, name + msvc_string::LENGTH, 22);
    p64(&mut named.bytes, name + msvc_string::CAPACITY, 31);
    let name = empty + autonomy_state_definition::NAME;
    p64(&mut named.bytes, name + msvc_string::CAPACITY, 15);
    let entry = |bytes: &[u8], array: Array, index: usize| {
        let at = array.data + index * 8;
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
    };
    let mut subject = 0;
    for index in 0..hub.count as usize {
        let relation = entry(&healthy, hub, index);
        if u32::from_le_bytes(
            healthy[relation + TYPE_ID..relation + TYPE_ID + 4]
                .try_into()
                .unwrap(),
        ) != TYPE_VALUE
        {
            continue;
        }
        let definition = match subject {
            n if n % 7 == 3 => Some(long),
            n if n % 11 == 5 => Some(empty),
            _ => None,
        };
        if let Some(definition) = definition {
            p64(
                &mut named.bytes,
                relation + AUTONOMY_STATE,
                definition as u64,
            );
        }
        subject += 1;
    }
    let null_at = |bytes: &[u8], array: Array, index: usize| {
        let mut bytes = bytes.to_vec();
        p64(&mut bytes, array.data + index * 8, 0);
        bytes
    };
    let duplicate = {
        let mut bytes = named.bytes.clone();
        let first = entry(&bytes, hub, 0);
        p64(&mut bytes, hub.data + 70 * 8, first as u64);
        bytes
    };
    let (_, last) = owned_of(&named.bytes, 200);
    let images = [
        named.bytes.clone(),
        null_at(&named.bytes, hub, 0),
        null_at(&named.bytes, hub, 63),
        null_at(&named.bytes, hub, 64),
        duplicate,
        null_at(&named.bytes, last, 0),
    ];
    for (image, bytes) in images.iter().enumerate() {
        let memory = SliceMemory { base: 0, bytes };
        for (rows, names) in [
            (200, 200 * 22),
            (200, 50 * 6 + 3),
            (200, 0),
            (40, 200 * 22),
            (63, 63 * 6),
            (0, 0),
        ] {
            let [batched, serial] = batched_and_serial(&memory, &memory, rows, names);
            assert_eq!(batched, serial, "image {image} rows {rows} names {names}");
            if image > 0 {
                assert!(
                    matches!(batched.0.outcome, Outcome::Unavailable { .. }),
                    "image {image} rows {rows} names {names}"
                );
            }
        }
    }
}

#[test]
fn batched_subjects_skip_null_and_alias_slots_and_refuse_late_invalid_names() {
    let (mut bytes, hub, _) = mixed_census_of(200, |slot| slot != 2 && slot != 3);
    assert_ne!(
        hub.count % BATCH as u32,
        0,
        "the overlord's last batch is partial"
    );
    let word = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let census = word(&bytes, GS + cgame_state::COUNTRIES) as usize;
    let links = word(&bytes, GS + cgame_state::COUNTRY_LINKS) as usize;
    let country_four = word(&bytes, census + 4 * 8);
    p64(&mut bytes, census + 2 * 8, 0);
    p64(&mut bytes, census + 3 * 8, country_four);
    p32(&mut bytes, links + 3 * 4, 4);
    let (_, four) = owned_of(&bytes, 4);
    assert_eq!(four.count, 1);
    let mut null_four = bytes.clone();
    p64(&mut null_four, four.data, 0);
    let mut bad_name = Space {
        bytes: bytes.clone(),
    };
    let definition = bad_name.alloc(0x40);
    let name = definition + autonomy_state_definition::NAME;
    bad_name.bytes[name..name + 7].copy_from_slice(b"puppetx");
    p64(&mut bad_name.bytes, name + msvc_string::LENGTH, 6);
    p64(&mut bad_name.bytes, name + msvc_string::CAPACITY, 15);
    let late = (0..hub.count as usize)
        .map(|index| word(&bytes, hub.data + index * 8) as usize)
        .filter(|&relation| {
            u32::from_le_bytes(
                bytes[relation + TYPE_ID..relation + TYPE_ID + 4]
                    .try_into()
                    .unwrap(),
            ) == TYPE_VALUE
        })
        .nth(150)
        .unwrap();
    p64(
        &mut bad_name.bytes,
        late + AUTONOMY_STATE,
        definition as u64,
    );
    for (image, bytes) in [bytes, null_four, bad_name.bytes].iter().enumerate() {
        let memory = SliceMemory { base: 0, bytes };
        for (rows, names) in [
            (200, 200 * 6),
            (200, 50 * 6 + 3),
            (200, 0),
            (40, 200 * 6),
            (0, 0),
        ] {
            let [batched, serial] = batched_and_serial(&memory, &memory, rows, names);
            assert_eq!(batched, serial, "image {image} rows {rows} names {names}");
            let refused = matches!(batched.0.outcome, Outcome::Unavailable { .. });
            assert_eq!(
                refused,
                image > 0,
                "image {image} rows {rows} names {names}"
            );
        }
    }
}
