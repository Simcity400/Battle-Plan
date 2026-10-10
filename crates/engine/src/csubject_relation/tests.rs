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
        p64(&mut bytes, COUNTRIES[owner], ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, COUNTRIES[owner] + ccountry::ID, [3, 2][owner]);
        p64(
            &mut bytes,
            COUNTRIES[owner] + ccountry::DIPLOMACY_STATUS,
            DIPLOMACY[owner] as u64,
        );
        p64(
            &mut bytes,
            DIPLOMACY[owner],
            cdiplomacy_status::VTABLE_RVA as u64,
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
            p64(&mut bytes, entry, crelation_status::VTABLE_RVA as u64);
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
    p64(&mut bytes, RELATION, VTABLE_RVA as u64);
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
    p64(&mut bytes, RETIRED_RELATION, VTABLE_RVA as u64);
    p32(&mut bytes, RETIRED_RELATION + TYPE_ID, TYPE_VALUE);
    bytes[RETIRED_RELATION + crelation::RETIRED] = 7;
    p32(&mut bytes, 0x1a000 + TYPE_ID, 0x1234);
    bytes
}

fn capture(bytes: &[u8], matrix: bool, output: &mut [Row], names: &mut [u8]) -> NameWalk {
    let memory = SliceMemory { base: 0, bytes };
    if matrix {
        walk_matrix(&memory, 0, GS, output, names)
    } else {
        walk(&memory, 0, GS, output, names)
    }
}

#[test]
fn growing_subject_rows_names_and_membership_snapshots_capture_the_first_5000_country_pass() {
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let mut scratch = OwnedSnapshotScratch::default();
    for countries in [5_000_usize, 10_000] {
        let (bytes, expected) = overlord_census(countries);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let captured = walk_owned_growing(&memory, 0, GS, &mut rows, &mut names, &mut scratch);
        assert_eq!(
            captured,
            NameWalk {
                outcome: Outcome::Rows { len: countries - 1 },
                bytes_len: (countries - 1) * 6,
                required_bytes: (countries - 1) * 6
            }
        );
        assert_eq!(&rows[..countries - 1], expected.as_slice());
        assert!(
            names[..captured.bytes_len]
                .chunks(6)
                .all(|value| value == b"puppet")
        );
        assert!(scratch.stats().installed > 0);
    }
}

#[test]
fn censuses_preserve_native_direction_full_width_and_literal_bytes() {
    let bytes = fixture();
    for matrix in [false, true] {
        let mut rows = [Row::default(); 2];
        let mut names = [0; 20];
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

/// A retired relation still installed at `+0x288` is what the native
/// subject terms read, so it stays a row; the retired relation left only in
/// the owned vector (the fixture's second entry) is skipped.
#[test]
fn a_retired_subject_counts_while_it_is_installed() {
    let mut bytes = fixture();
    // The owned-only retired relation names the same pair but is not the
    // one installed, so it resolves and is still skipped.
    p32(&mut bytes, RETIRED_RELATION + OVERLORD, 3);
    p32(&mut bytes, RETIRED_RELATION + SUBJECT, 2);
    bytes[RELATION + crelation::RETIRED] = 1;
    for matrix in [false, true] {
        let mut rows = [Row::default(); 2];
        let mut names = [0; 20];
        assert_eq!(
            capture(&bytes, matrix, &mut rows, &mut names).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
    }
}

#[test]
fn capacity_omits_whole_rows_and_reports_exact_required_bytes() {
    let bytes = fixture();
    for matrix in [false, true] {
        for (row_capacity, name_capacity) in [(0, 4), (1, 3)] {
            let mut rows = [Row::default(); 1];
            let mut names = [77; 4];
            assert_eq!(
                capture(
                    &bytes,
                    matrix,
                    &mut rows[..row_capacity],
                    &mut names[..name_capacity]
                ),
                NameWalk {
                    outcome: Outcome::Truncated {
                        len: 0,
                        unread_rows: 1
                    },
                    bytes_len: 0,
                    required_bytes: 4
                }
            );
            assert_eq!(names, [77; 4]);
        }
    }
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
    let mut rows = [Row::default(); 1];
    let mut names = [0; 4];
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
    let mut rows = [Row::default(); 1];
    let mut names = [0; 4];
    assert_eq!(
        capture(&bytes, true, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].overlord, rows[0].subject), (3, 2));
    // The 65th cell has no subject, but an invalid owner must still fail.
    p64(&mut bytes, 0x20000 + 64 * 8, 0x22000);
    p64(&mut bytes, 0x22000, crelation_status::VTABLE_RVA as u64);
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
    // Slot 65 is outside the 65-entry array and may contain garbage.
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
                capture(&bytes, false, &mut [Row::default(); 1], &mut [0; 40]).outcome,
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
    let mut names = [0; 128];
    assert_eq!(
        capture(&bytes, false, &mut [Row::default(); 1], &mut names),
        NameWalk {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: 128,
            required_bytes: 128
        }
    );
    assert_eq!(names, [255; 128]);
    bytes[0x1c080] = 1;
    assert!(matches!(
        capture(&bytes, false, &mut [Row::default(); 1], &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
}

/// Bump-allocated synthetic address space for census-scale fixtures.
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

/// `countries` canonical countries; country 1 is the overlord of every
/// other one, and its owned vector holds all those subjects.
fn overlord_census(countries: usize) -> (Vec<u8>, Vec<Row>) {
    let mut space = Space {
        bytes: vec![0; 0x2000],
    };
    let census = space.alloc((countries + 1) * 8);
    let links = space.alloc((countries + 1) * 4);
    // 24-byte cells: vtable and id, then a later country's diplomacy
    // pointer (0xf88 = 165 * 24 + 16), so objects overlap without clashing.
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
        p64(&mut space.bytes, at, ccountry::VTABLE_RVA as u64);
        p32(&mut space.bytes, at + ccountry::ID, slot as u32);
        *root = space.alloc(0x290);
        p64(
            &mut space.bytes,
            at + ccountry::DIPLOMACY_STATUS,
            *root as u64,
        );
        p64(
            &mut space.bytes,
            *root,
            cdiplomacy_status::VTABLE_RVA as u64,
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
        p64(&mut space.bytes, at, crelation_status::VTABLE_RVA as u64);
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
    for subject in 2..=countries {
        let relation = space.alloc(0x68);
        p64(&mut space.bytes, relation, VTABLE_RVA as u64);
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

/// Counts guarded reads of one address range.
struct CountingMemory<'a> {
    bytes: &'a [u8],
    watched: std::ops::Range<usize>,
    hits: std::cell::Cell<usize>,
}

impl Memory for CountingMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.watched.contains(&address) {
            self.hits.set(self.hits.get() + 1);
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
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
        // Empty storage is the plain walk; grown storage installs the
        // overlord's vector once and answers every later visit from it.
        assert_eq!(scratch.stats().installed > 0, grown);
    }
}

#[test]
fn owned_subject_walk_at_census_scale_is_linear_in_relations() {
    // 15,000 countries, one overlord of 14,999 subjects: the plain walk
    // scans the overlord's 14,999-entry vector for every subject from
    // both endpoints (about 4.5e8 element reads).
    const COUNTRIES: usize = 15_000;
    let (bytes, expected) = overlord_census(COUNTRIES);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    // An empty fixed output still records the complete original headers.
    let mut scratch = OwnedSnapshotScratch::default();
    let mut rows = vec![Row::default(); COUNTRIES];
    let mut names = vec![0; COUNTRIES * 6];
    let hub = {
        let reader = Reader::new(&memory, 0, GS).unwrap();
        let country = reader.countries.at(1).unwrap().unwrap();
        let diplomacy = reader.diplomacy(country).unwrap();
        reader
            .array(diplomacy, cdiplomacy_status::OWNED_RELATIONS)
            .unwrap()
    };
    let counting = CountingMemory {
        bytes: &bytes,
        watched: hub.data..hub.data + hub.count as usize * 8,
        hits: std::cell::Cell::new(0),
    };
    // Measure original headers on a fixed empty output, then prepare exactly
    // that demand. Complete cold rescans are covered by the war fixture.
    let cold = walk_owned(&counting, 0, GS, &mut [], &mut [], &mut scratch);
    assert_eq!(
        cold.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: (COUNTRIES - 1) as u32
        }
    );
    scratch.grow(0).unwrap();
    let allocated = scratch.allocated_bytes();
    counting.hits.set(0);
    let result = walk_owned(&counting, 0, GS, &mut rows, &mut names, &mut scratch);
    assert_eq!(
        result,
        NameWalk {
            outcome: Outcome::Rows { len: COUNTRIES - 1 },
            bytes_len: (COUNTRIES - 1) * 6,
            required_bytes: (COUNTRIES - 1) * 6,
        }
    );
    assert_eq!(rows[..COUNTRIES - 1], expected[..]);
    assert!(
        names[..(COUNTRIES - 1) * 6]
            .chunks(6)
            .all(|name| name == b"puppet")
    );
    assert_eq!(scratch.allocated_bytes(), allocated);
    // The outer census reads each hub entry once; the snapshot adds one
    // more complete scan, instead of 2 x 14,999 scans.
    assert!(
        counting.hits.get() <= 2 * hub.count as usize,
        "{}",
        counting.hits.get()
    );
    assert_eq!(scratch.stats().installed, 1);
    // Output capacity still truncates honestly, never by country count.
    let short = walk_owned(&memory, 0, GS, &mut rows[..10], &mut names, &mut scratch);
    assert_eq!(
        short.outcome,
        Outcome::Truncated {
            len: 10,
            unread_rows: (COUNTRIES - 11) as u32
        }
    );
}
