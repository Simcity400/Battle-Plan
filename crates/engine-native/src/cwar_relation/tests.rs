use super::*;
use crate::{cgame_state, memory::SliceMemory};
use std::cell::{Cell, RefCell};

#[test]
fn checked_owned_snapshot_capacity_overflow_returns_oom() {
    let error = match OwnedSnapshotScratch::try_with_capacity(usize::MAX) {
        Ok(_) => panic!("impossible snapshot capacity accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::OutOfMemory);
    let mut scratch = OwnedSnapshotScratch::try_with_capacity(2).unwrap();
    let before = scratch.capacity();
    assert_eq!(
        scratch.grow(usize::MAX).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory
    );
    assert_eq!(scratch.capacity(), before);
}

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x3000;
const LINKS: usize = 0x3100;
const COUNTRY: [usize; 3] = [0x4000, 0x6000, 0x8000];
const DIPLOMACY: [usize; 3] = [0xa000, 0xb000, 0xc000];
const WAR1: usize = 0x1b000;
const WAR2: usize = 0x1c000;
const OTHER: usize = 0x1d000;

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
    for (index, &value) in values.iter().enumerate() {
        p64(bytes, data + index * 8, value);
    }
}
fn status(owner: usize, target: usize) -> usize {
    0xe000 + owner * 0x4000 + target * 0xa00
}
fn owned(owner: usize) -> usize {
    0xd100 + owner * 0x100
}
fn matrix(owner: usize) -> usize {
    0xd000 + owner * 0x40
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x20000];
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        COUNTRIES,
        &[
            u64::MAX,
            COUNTRY[0] as u64,
            COUNTRY[1] as u64,
            COUNTRY[2] as u64,
            COUNTRY[0] as u64,
        ],
    );
    p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS as u64);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 5);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 5);
    for (index, id) in [0, 1, 2, 3, 1].into_iter().enumerate() {
        p32(&mut bytes, LINKS + index * 4, id);
    }
    for owner in 0..3 {
        p64(&mut bytes, COUNTRY[owner], ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, COUNTRY[owner] + ccountry::ID, [4, 2, 3][owner]);
        p64(
            &mut bytes,
            COUNTRY[owner] + ccountry::DIPLOMACY_STATUS,
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
            COUNTRY[owner] as u64,
        );
        let entries: Vec<_> = (0..5).map(|target| status(owner, target) as u64).collect();
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::RELATIONS,
            matrix(owner),
            &entries,
        );
        for target in 0..5 {
            let at = status(owner, target);
            p64(&mut bytes, at, crelation_status::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                at + crelation_status::DIPLOMACY_STATUS,
                DIPLOMACY[owner] as u64,
            );
            // Matrix slot one deliberately uses alias4 as its raw target.
            p32(
                &mut bytes,
                at + crelation_status::TARGET_COUNTRY,
                [0, 4, 2, 3, 0][target],
            );
        }
    }
    array(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS,
        owned(0),
        &[WAR2 as u64, OTHER as u64, WAR1 as u64],
    );
    array(
        &mut bytes,
        DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS,
        owned(1),
        &[WAR1 as u64],
    );
    array(
        &mut bytes,
        DIPLOMACY[2] + cdiplomacy_status::OWNED_RELATIONS,
        owned(2),
        &[WAR2 as u64],
    );
    p32(&mut bytes, OTHER + crelation::TYPE_ID, 0x28da);
    for (war, second) in [(WAR1, 2), (WAR2, 3)] {
        p64(&mut bytes, war, VTABLE_RVA as u64);
        p32(&mut bytes, war + crelation::TYPE_ID, TYPE);
        p32(&mut bytes, war + crelation::FIRST_COUNTRY, 4);
        p32(&mut bytes, war + crelation::SECOND_COUNTRY, second as u32);
        p32(&mut bytes, war + crelation::START_HOUR, i32::MIN as u32);
        p32(&mut bytes, war + crelation::END_HOUR, i32::MAX as u32);
        bytes[war + crelation::CANCEL] = 7;
        bytes[war + FIRST_WAS_INSTIGATOR] = 255;
        p64(&mut bytes, war + FIRST_CASUALTIES, i64::MAX as u64);
        p64(&mut bytes, war + SECOND_CASUALTIES, i64::MIN as u64);
        p64(&mut bytes, war + FIRST_UNKNOWN_CASUALTIES, 0x1_0000_0001);
        p64(
            &mut bytes,
            war + SECOND_UNKNOWN_CASUALTIES,
            (-0x1_0000_0001_i64) as u64,
        );
        for (owner, target, serializable) in [(0, second, true), (second - 1, 1, false)] {
            let at = status(owner, target);
            p64(&mut bytes, at + crelation_status::WAR_RELATION, war as u64);
            array(
                &mut bytes,
                at + crelation_status::SUBRELATIONS,
                at + 0x400,
                &[war as u64],
            );
            if serializable {
                array(
                    &mut bytes,
                    at + crelation_status::SERIALIZABLE_RELATIONS,
                    at + 0x420,
                    &[war as u64],
                );
            }
        }
    }
    bytes
}

fn capture(bytes: &[u8], output: &mut [Row]) -> Outcome {
    walk(&SliceMemory { base: 0, bytes }, 0, GS, output)
}

#[test]
fn war_rows_grow_from_owned_headers_on_five_thousand_country_world() {
    use crate::ccountry::synthetic::World;
    let mut output = Vec::new();
    let mut scratch = OwnedSnapshotScratch::try_with_capacity(0).unwrap();
    let mut wars = Vec::new();
    for pairs in [2_000_u32, 2_500] {
        let mut created = Vec::new();
        let mut world = World::new(0, 5_000, 0x1100);
        let mut diplomacies = vec![0; 5_001];
        for slot in 1..=5_000 {
            let country = world.object(slot);
            let diplomacy = world.alloc(0x300);
            diplomacies[slot as usize] = diplomacy;
            world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            world.u64(diplomacy, cdiplomacy_status::VTABLE_RVA as u64);
            world.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        }
        for pair in 0..pairs {
            let first = pair * 2 + 1;
            let second = first + 1;
            let war = world.alloc(0x200);
            let owned = world.alloc(8);
            world.u64(war, VTABLE_RVA as u64);
            world.u32(war + crelation::TYPE_ID, TYPE);
            world.u32(war + crelation::FIRST_COUNTRY, first);
            world.u32(war + crelation::SECOND_COUNTRY, second);
            world.u64(war + SECOND_UNKNOWN_CASUALTIES, u64::from(pair));
            world.u64(owned, war as u64);
            created.push(war);
            for (owner, target, serializable) in [(first, second, true), (second, first, false)] {
                let diplomacy = diplomacies[owner as usize];
                let matrix = world.alloc((target as usize + 1) * 8);
                let status = world.alloc(0x300);
                world.array(diplomacy + cdiplomacy_status::OWNED_RELATIONS, owned, 1);
                world.array(diplomacy + cdiplomacy_status::RELATIONS, matrix, target + 1);
                world.u64(matrix + target as usize * 8, status as u64);
                world.u64(status, crelation_status::VTABLE_RVA as u64);
                world.u64(
                    status + crelation_status::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                world.u32(status + crelation_status::TARGET_COUNTRY, target);
                world.u64(status + crelation_status::WAR_RELATION, war as u64);
                world.array(status + crelation_status::SUBRELATIONS, owned, 1);
                if serializable {
                    world.array(status + crelation_status::SERIALIZABLE_RELATIONS, owned, 1);
                }
            }
        }
        let memory = world.memory();
        assert_eq!(
            row_demand(&memory, 0, world.game_state()),
            Ok(pairs as usize * 2)
        );
        assert_eq!(
            walk_owned_growing(
                &memory,
                &memory,
                0,
                world.game_state(),
                &mut output,
                &mut scratch,
                Some(&mut wars)
            ),
            Outcome::Rows {
                len: pairs as usize
            }
        );
        assert_eq!(output[pairs as usize - 1].second_country, pairs * 2);
        // Each row's war, in row order, for the casualty reads of later hours.
        assert_eq!(wars, created);
        let mut rows = output[..pairs as usize].to_vec();
        for (row, war) in rows.iter_mut().zip(&wars) {
            row.second_unknown_casualties = -1;
            world.u64(war + FIRST_CASUALTIES, 7);
        }
        assert!(read_casualties(&world.memory(), &wars, &mut rows));
        for (pair, row) in rows.iter().enumerate() {
            assert_eq!(row.first_casualties, 7);
            assert_eq!(row.second_unknown_casualties, pair as i64);
            assert_eq!(row.first_country, output[pair].first_country);
        }
        assert!(!read_casualties(&world.memory(), &wars[1..], &mut rows));
        assert!(!read_casualties(
            &world.memory(),
            &[usize::MAX - 0x10],
            &mut rows[..1]
        ));
    }
}

struct CountedMemory<'a> {
    bytes: &'a [u8],
    guards: Cell<usize>,
    blocked: Cell<usize>,
}

impl CountedMemory<'_> {
    fn copy_field(&self, address: usize, destination: &mut [u8]) -> bool {
        let blocked = self.blocked.get();
        if address <= blocked && address.saturating_add(destination.len()) > blocked {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

impl Memory for CountedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.copy_field(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !destination.is_empty()
                && (address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !self.copy_field(address, destination))
            {
                return index;
            }
        }
        N
    }
}

fn reader<'a, M: Memory>(memory: &'a M) -> Reader<'a, M> {
    let countries = ccountry::Reader::new(memory, 0, GS).unwrap();
    // Test-only storage; production readers borrow the frame's resolver.
    let resolver = Box::leak(Box::new(crelation_status::Resolver::default()));
    resolver.begin(&countries);
    resolver.grow().unwrap();
    Reader {
        memory,
        base: 0,
        countries,
        resolver,
        current_matrix: (0, None),
        ownership: None,
    }
}

struct ObservedMemory<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(usize, usize)>>,
    gathers: RefCell<Vec<usize>>,
    blocked: std::ops::Range<usize>,
}

impl<'a> ObservedMemory<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            reads: RefCell::new(Vec::new()),
            gathers: RefCell::new(Vec::new()),
            blocked: 0..0,
        }
    }
}

// Use the trait's scalar record implementation while retaining each exact
// observation and the caller's gather boundaries in these pure fixtures.
struct ObservedFields<'a, 'b>(&'a ObservedMemory<'b>);

impl Memory for ObservedFields<'_, '_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.copy(address, destination)
    }
}

impl Memory for ObservedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if address < self.blocked.end && address + destination.len() > self.blocked.start {
            destination[0] = 0xff;
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.gathers.borrow_mut().push(layout.field_count());
        ObservedFields(self).copy_records(sources, layout, destination)
    }
}

// Frozen pre-snapshot membership logic. Keep semantic validation independent
// of scan_occurrence, including its complete batch-prefix observation order.
fn original_owned_occurrence(
    memory: &impl Memory,
    root: usize,
    war: usize,
) -> Result<(), &'static str> {
    let array = root
        .checked_add(cdiplomacy_status::OWNED_RELATIONS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("war ownership vector unreadable or invalid")?;
    if array.count <= 1 {
        let found = if array.count == 0 {
            0
        } else {
            let pointer = array
                .element(0, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("war ownership pointer unreadable")?;
            if pointer == 0 {
                return Err("null war ownership entry");
            }
            u32::from(pointer == war)
        };
        return if found == 1 {
            Ok(())
        } else {
            Err("war ownership cross-link count mismatch")
        };
    }
    let mut found = 0;
    let mut batch = PointerBatch::default();
    for index in 0..array.count {
        let offset = index as usize % BATCH_SIZE;
        if offset == 0 {
            batch.read(memory, array, index);
        }
        let pointer = batch.get(offset, "war ownership pointer unreadable")?;
        if pointer == 0 {
            return Err("null war ownership entry");
        }
        if pointer == war {
            found += 1;
        }
    }
    if found == 1 {
        Ok(())
    } else {
        Err("war ownership cross-link count mismatch")
    }
}

fn prepared_snapshot(countries: usize, words: usize) -> OwnedSnapshotScratch {
    let mut scratch = OwnedSnapshotScratch::with_capacity(countries);
    scratch.stats.required_words = words;
    scratch.stats.required_cells = words.saturating_mul(4);
    scratch.grow(countries).unwrap();
    scratch
}

fn native_id_alias_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 8);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 8);
    p32(&mut bytes, LINKS + 7 * 4, 1);
    p32(&mut bytes, COUNTRY[0] + ccountry::ID, 7);
    for war in [WAR1, WAR2] {
        p32(&mut bytes, war + crelation::FIRST_COUNTRY, 7);
    }
    for owner in 0..3 {
        p32(
            &mut bytes,
            status(owner, 1) + crelation_status::TARGET_COUNTRY,
            7,
        );
    }
    bytes
}

#[test]
fn owned_split_keeps_native_id_alias_cached_and_routes_only_payload_to_source() {
    let bytes = native_id_alias_fixture();
    let seed = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut cache = crate::country_memory::Cache::with_capacity(1024);
    assert_eq!(
        cache.prepare(&seed, 0, GS).seed_status(),
        crate::country_memory::SeedStatus::Ready
    );
    let alias = LINKS + 7 * 4;
    let country_source = ObservedMemory {
        blocked: alias..alias + 4,
        ..ObservedMemory::new(&bytes)
    };
    // Raw 7 is outside the five-slot roster. Its seeded byte observation,
    // not authenticated_countries alone, makes the old resolution succeed.
    let roster_only = cache.roster_view(&country_source);
    assert!(
        ccountry::Reader::new(&roster_only, 0, GS)
            .unwrap()
            .resolve(7)
            .is_err()
    );
    assert_eq!(*country_source.reads.borrow(), [(alias, 4)]);
    country_source.reads.borrow_mut().clear();

    let cached = cache.view(&country_source);
    let payload = ObservedMemory::new(&bytes);
    let mut rows = [Row::default(); 4];
    let mut scratch = prepared_snapshot(5, 64);
    assert_eq!(
        walk_owned(&payload, &cached, 0, GS, &mut rows, &mut scratch),
        Outcome::Rows { len: 2 }
    );
    assert!(rows[..2].iter().all(|row| row.first_country == 7));
    // If payload reads accidentally pass through the country view, this
    // source sees their misses. Country resolution itself needs no native
    // read, including the raw-id alias that roster_view cannot supply.
    assert!(country_source.reads.borrow().is_empty());
    assert!(!payload.reads.borrow().is_empty());
    assert!(
        payload
            .reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == WAR1 + FIRST_CASUALTIES)
    );

    let original = ObservedMemory::new(&bytes);
    let original_cached = cache.view(&original);
    let mut expected = [Row::default(); 4];
    let mut original_scratch = prepared_snapshot(5, 64);
    assert_eq!(
        walk_owned(
            &original_cached,
            &original_cached,
            0,
            GS,
            &mut expected,
            &mut original_scratch,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[..2], expected[..2]);
    assert_eq!(*payload.reads.borrow(), *original.reads.borrow());
    assert_eq!(*payload.gathers.borrow(), *original.gathers.borrow());
}

#[test]
fn owned_split_preserves_alias_and_payload_first_faults_with_or_without_snapshot() {
    let bytes = native_id_alias_fixture();
    let seed = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    for cells in [0, 1024] {
        let mut cache = crate::country_memory::Cache::with_capacity(cells);
        cache.prepare(&seed, 0, GS);
        for blocked in [
            LINKS + 7 * 4..LINKS + 7 * 4 + 4,
            WAR2 + FIRST_CASUALTIES..WAR2 + FIRST_CASUALTIES + 1,
            DIPLOMACY[0]..DIPLOMACY[0] + 1,
        ] {
            let previous = ObservedMemory {
                blocked: blocked.clone(),
                ..ObservedMemory::new(&bytes)
            };
            let previous_cached = cache.view(&previous);
            let mut expected = [Row::default(); 4];
            let expected_outcome = walk_owned(
                &previous_cached,
                &previous_cached,
                0,
                GS,
                &mut expected,
                &mut prepared_snapshot(5, 64),
            );
            let current = ObservedMemory {
                blocked,
                ..ObservedMemory::new(&bytes)
            };
            let current_cached = cache.view(&current);
            let mut actual = [Row::default(); 4];
            let outcome = walk_owned(
                &current,
                &current_cached,
                0,
                GS,
                &mut actual,
                &mut prepared_snapshot(5, 64),
            );
            assert_eq!(outcome, expected_outcome);
            assert_eq!(actual[..outcome.len()], expected[..expected_outcome.len()]);
            assert_eq!(*current.reads.borrow(), *previous.reads.borrow());
            assert_eq!(*current.gathers.borrow(), *previous.gathers.borrow());
        }
    }
}

fn isolated_owned(values: &[u64]) -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    array(
        &mut bytes,
        0x100 + cdiplomacy_status::OWNED_RELATIONS,
        0x1000,
        values,
    );
    bytes
}

#[test]
fn owned_snapshot_default_and_take_preserve_storage_without_allocation() {
    let empty = OwnedSnapshotScratch::default();
    assert_eq!(empty.capacity(), OwnedSnapshotCapacity::default());
    assert_eq!(empty.stats(), OwnedSnapshotStats::default());
    assert_eq!(empty.used_cells, 0);

    let bytes = isolated_owned(&[0x8000, 0x8008]);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut scratch = prepared_snapshot(1, 2);
    assert_eq!(scratch.occurrence(&memory, 0x100, 0x8000), Ok(()));
    assert_eq!(scratch.occurrence(&memory, 0x100, 0x8008), Ok(()));
    let capacity = scratch.capacity();
    let stats = scratch.stats();
    let pointers = |scratch: &OwnedSnapshotScratch| {
        [
            scratch.vectors.as_ptr() as usize,
            scratch.roots.as_ptr() as usize,
            scratch.words.as_ptr() as usize,
            scratch.counts.as_ptr() as usize,
            scratch.status_graphs.as_ptr() as usize,
        ]
    };
    let allocated = pointers(&scratch);
    assert!(capacity.allocated_bytes > 0 && stats.hits > 0);
    for _ in 0..3 {
        let owned = std::mem::take(&mut scratch);
        assert_eq!(scratch.capacity(), OwnedSnapshotCapacity::default());
        assert_eq!(scratch.stats(), OwnedSnapshotStats::default());
        assert_eq!(scratch.used_cells, 0);
        assert_eq!(owned.capacity(), capacity);
        assert_eq!(owned.stats(), stats);
        assert_eq!(pointers(&owned), allocated);
        scratch = owned;
    }
}

#[test]
fn owned_snapshot_first_scan_and_capacity_fallback_keep_frozen_fault_traces() {
    for count in [0, 1, 2, BATCH_SIZE, BATCH_SIZE + 1] {
        let values: Vec<_> = (0..count)
            .map(|index| (0x8000 + index * 8) as u64)
            .collect();
        let bytes = isolated_owned(&values);
        let mut faults = vec![0..0, 0x120..0x121, 0x12f..0x130];
        for index in 0..count {
            for byte in [0, 7] {
                let at = 0x1000 + index * 8 + byte;
                faults.push(at..at + 1);
            }
        }
        for fault in faults {
            for capacity in 0..4 {
                let mut scratch = match capacity {
                    0 => prepared_snapshot(0, count),
                    1 => OwnedSnapshotScratch::with_capacity(1),
                    3 => {
                        let mut scratch = prepared_snapshot(1, count);
                        scratch.counts.clear();
                        scratch
                    }
                    _ => prepared_snapshot(1, count),
                };
                scratch.begin();
                let allocated = scratch.allocated_bytes();
                let mut original = ObservedMemory::new(&bytes);
                original.blocked = fault.clone();
                let mut candidate = ObservedMemory::new(&bytes);
                candidate.blocked = fault.clone();
                let expected = original_owned_occurrence(&original, 0x100, 0x8000);
                assert_eq!(scratch.occurrence(&candidate, 0x100, 0x8000), expected);
                assert_eq!(*candidate.reads.borrow(), *original.reads.borrow());
                assert_eq!(scratch.allocated_bytes(), allocated);
                if expected.is_err() {
                    assert!(scratch.words.is_empty());
                    assert_eq!(scratch.stats.installed, 0);
                }
            }
        }
    }
    let variants = [
        vec![],
        vec![0],
        vec![0x8008],
        vec![0x8000, 0],
        vec![0x8000, 0x8000],
        vec![0x8008, 0x8010],
    ];
    for values in variants {
        let bytes = isolated_owned(&values);
        let original = ObservedMemory::new(&bytes);
        let candidate = ObservedMemory::new(&bytes);
        let mut scratch = prepared_snapshot(1, values.len());
        let expected = original_owned_occurrence(&original, 0x100, 0x8000);
        assert_eq!(scratch.occurrence(&candidate, 0x100, 0x8000), expected);
        assert_eq!(*candidate.reads.borrow(), *original.reads.borrow());
    }
    for (offset, value) in [
        (crate::cpdx_array::COUNT, u32::MAX),
        (crate::cpdx_array::CAPACITY, 1),
    ] {
        let mut bytes = isolated_owned(&[0x8000, 0x8008]);
        p32(&mut bytes, 0x120 + offset, value);
        let original = ObservedMemory::new(&bytes);
        let candidate = ObservedMemory::new(&bytes);
        let mut scratch = prepared_snapshot(1, 4);
        assert_eq!(
            scratch.occurrence(&candidate, 0x100, 0x8000),
            original_owned_occurrence(&original, 0x100, 0x8000)
        );
        assert_eq!(*candidate.reads.borrow(), *original.reads.borrow());
    }
    for root in [
        usize::MAX,
        usize::MAX - cdiplomacy_status::OWNED_RELATIONS + 1,
    ] {
        let bytes = isolated_owned(&[]);
        let original = ObservedMemory::new(&bytes);
        let candidate = ObservedMemory::new(&bytes);
        let mut scratch = prepared_snapshot(1, 4);
        assert_eq!(
            scratch.occurrence(&candidate, root, 0x8000),
            original_owned_occurrence(&original, root, 0x8000)
        );
        assert!(candidate.reads.borrow().is_empty());
    }
}

#[test]
fn owned_snapshot_complete_walk_matches_serial_rows_errors_aliases_and_output_capacity() {
    let stable = fixture();
    let mut variants = vec![stable.clone()];
    for (at, value) in [
        (owned(0) + 8, 0),
        (owned(0) + 8, WAR1 as u64),
        (owned(1), OTHER as u64),
        (status(0, 2) + 0x400, OTHER as u64),
    ] {
        let mut bytes = stable.clone();
        p64(&mut bytes, at, value);
        variants.push(bytes);
    }
    let mut bytes = stable.clone();
    p32(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS + 12,
        2,
    );
    variants.push(bytes);
    for bytes in variants {
        for capacity in 0..4 {
            let mut scratch = OwnedSnapshotScratch::with_capacity(5);
            // Initial demand is discovered with no payload allocation.
            let memory = SliceMemory {
                base: 0,
                bytes: &stable,
            };
            let _ = walk_owned(
                &memory,
                &memory,
                0,
                GS,
                &mut [Row::default(); 2],
                &mut scratch,
            );
            assert_eq!(scratch.stats.hits, 0);
            scratch.grow(5).unwrap();
            let allocated = scratch.allocated_bytes();
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut expected = vec![Row::default(); capacity];
            let mut actual = expected.clone();
            let outcome = walk(&memory, 0, GS, &mut expected);
            let result = walk_owned(&memory, &memory, 0, GS, &mut actual, &mut scratch);
            assert_eq!(result, outcome);
            assert_eq!(actual, expected);
            assert_eq!(scratch.allocated_bytes(), allocated);
            if bytes == stable {
                assert_eq!(scratch.stats.hits, 3);
                assert_eq!(
                    actual.first().map(|row| row.first_country),
                    (capacity != 0).then_some(4)
                );
            }
        }
    }
}

#[test]
fn owned_snapshot_collisions_and_duplicate_counts_use_full_keys_and_preserve_absence() {
    let root_mask = 3;
    let first_root = 0x100;
    let second_root = (0x140..0x800)
        .step_by(0x20)
        .find(|&root| {
            OwnedSnapshotScratch::hash(root) & root_mask
                == OwnedSnapshotScratch::hash(first_root) & root_mask
        })
        .unwrap();
    let first = 0x8000;
    let collision = (first + 8..0xa000)
        .step_by(8)
        .find(|&pointer| {
            OwnedSnapshotScratch::hash(pointer) & 7 == OwnedSnapshotScratch::hash(first) & 7
        })
        .unwrap();
    let mut bytes = isolated_owned(&[first as u64, collision as u64, collision as u64]);
    array(
        &mut bytes,
        second_root + cdiplomacy_status::OWNED_RELATIONS,
        0x2000,
        &[collision as u64, first as u64],
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut scratch = prepared_snapshot(2, 8);
    assert_eq!(scratch.occurrence(&memory, first_root, first), Ok(()));
    assert_eq!(scratch.occurrence(&memory, second_root, collision), Ok(()));
    for (root, pointer) in [
        (first_root, collision),
        (first_root, 0xdead),
        (first_root, first),
        (second_root, first),
        (second_root, collision),
    ] {
        assert_eq!(
            scratch.occurrence(&memory, root, pointer),
            original_owned_occurrence(&memory, root, pointer)
        );
    }
    assert_eq!(scratch.stats.installed, 2);
    assert_eq!(scratch.stats.hits, 5);
}

#[test]
fn owned_snapshot_resets_and_growth_keep_root_pressure_and_unreused_storage_honest() {
    let mut bytes = isolated_owned(&[0x8000, 0x8008]);
    array(
        &mut bytes,
        0x200 + cdiplomacy_status::OWNED_RELATIONS,
        0x2000,
        &[0x9000, 0x9008],
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut scratch = prepared_snapshot(1, 8);
    assert_eq!(scratch.occurrence(&memory, 0x100, 0x8000), Ok(()));
    let old = ObservedMemory::new(&bytes);
    let candidate = ObservedMemory::new(&bytes);
    assert_eq!(
        scratch.occurrence(&candidate, 0x200, 0x9000),
        original_owned_occurrence(&old, 0x200, 0x9000)
    );
    assert_eq!(*candidate.reads.borrow(), *old.reads.borrow());
    assert_eq!(scratch.stats.fallbacks, 1);
    scratch.grow(2).unwrap();
    scratch.begin();
    assert_eq!(scratch.stats, OwnedSnapshotStats::default());
    assert!(scratch.words.is_empty());
    let allocated = scratch.allocated_bytes();
    for root in [0x100, 0x200] {
        let pointer = if root == 0x100 { 0x8000 } else { 0x9000 };
        assert_eq!(scratch.occurrence(&memory, root, pointer), Ok(()));
        assert_eq!(scratch.occurrence(&memory, root, pointer), Ok(()));
    }
    assert_eq!(scratch.stats.installed, 2);
    assert_eq!(scratch.stats.hits, 2);
    assert_eq!(scratch.allocated_bytes(), allocated);
    let capacity = scratch.capacity();
    assert_eq!(capacity.allocated_bytes, allocated);
    assert_eq!(capacity.vectors, scratch.vectors.capacity());
    assert_eq!(capacity.root_slots, scratch.roots.len());
    assert_eq!(capacity.root_capacity, scratch.roots.capacity());
    assert_eq!(capacity.words, scratch.words.capacity());
    assert_eq!(capacity.initialized_count_cells, scratch.counts.len());
    assert_eq!(capacity.count_capacity, scratch.counts.capacity());
    scratch.begin();
    let mut changed = bytes.clone();
    p64(&mut changed, 0x1000, 0x8008);
    let memory = SliceMemory {
        base: 0,
        bytes: &changed,
    };
    assert_eq!(
        scratch.occurrence(&memory, 0x100, 0x8000),
        Err("war ownership cross-link count mismatch")
    );
    assert_eq!(scratch.stats.hits, 0);
    assert_eq!(scratch.stats.installed, 0);
    assert!(scratch.words.is_empty());
    // Demand only controls off-thread storage. It cannot impose a source
    // cap, and a demand no allocation can satisfy is refused as out of
    // memory while the existing (still correct) storage is kept.
    let before = scratch.capacity();
    scratch.stats.required_words = usize::MAX;
    scratch.stats.required_cells = usize::MAX;
    assert_eq!(
        scratch.grow(2).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory
    );
    assert_eq!(scratch.capacity(), before);
    // Observed demand beyond any former fixed budget is honoured exactly.
    scratch.stats.required_words = 70_000;
    scratch.stats.required_cells = 280_000;
    scratch.grow(2).unwrap();
    assert_eq!(scratch.counts.len(), 280_000);
    assert!(scratch.words.capacity() >= 70_000);
    assert_eq!(
        scratch.allocated_bytes(),
        scratch.vectors.capacity() * std::mem::size_of::<OwnedVector>()
            + scratch.roots.capacity() * std::mem::size_of::<usize>()
            + scratch.words.capacity() * std::mem::size_of::<usize>()
            + scratch.counts.capacity() * std::mem::size_of::<PointerCount>()
            + scratch.status_graphs.capacity() * std::mem::size_of::<StatusGraph>()
            + scratch.resolver.allocated_bytes()
    );
}

#[test]
fn owned_snapshot_dense_membership_reads_scale_as_one_capture() {
    let degree = BATCH_SIZE + 1;
    let values: Vec<_> = (0..degree)
        .map(|index| (0x8000 + index * 8) as u64)
        .collect();
    let bytes = isolated_owned(&values);
    let original = ObservedMemory::new(&bytes);
    let candidate = ObservedMemory::new(&bytes);
    let mut scratch = prepared_snapshot(1, degree);
    let allocated = scratch.allocated_bytes();
    for &pointer in &values {
        assert_eq!(
            original_owned_occurrence(&original, 0x100, pointer as usize),
            Ok(())
        );
        assert_eq!(
            scratch.occurrence(&candidate, 0x100, pointer as usize),
            Ok(())
        );
    }
    let body = |memory: &ObservedMemory<'_>| {
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| len == 8 && (0x1000..0x1000 + degree * 8).contains(&address))
            .count()
    };
    assert_eq!(body(&original), degree * degree);
    assert_eq!(body(&candidate), degree);
    assert_eq!(
        candidate
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, len)| at == 0x120 && len == 16)
            .count(),
        degree
    );
    assert_eq!(scratch.stats.hits, degree - 1);
    assert_eq!(scratch.stats.acquisition_words, degree);
    assert_eq!(scratch.allocated_bytes(), allocated);
}

#[test]
fn owned_snapshot_current_header_guard_rejects_changed_or_malformed_geometry() {
    let bytes = isolated_owned(&[0x8000, 0x8008]);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut changed = bytes.clone();
    // A valid capacity-only change must reach exact-header comparison,
    // unlike count > capacity, which retains the original parse error.
    p32(&mut changed, 0x120 + crate::cpdx_array::CAPACITY, 3);
    let changed_memory = SliceMemory {
        base: 0,
        bytes: &changed,
    };
    assert_eq!(
        Array::read(&changed_memory, 0x120, 8),
        Some(Array {
            data: 0x1000,
            count: 2
        })
    );
    assert_eq!(
        original_owned_occurrence(&changed_memory, 0x100, 0x8000),
        Ok(())
    );
    let mut scratch = prepared_snapshot(1, 2);
    assert_eq!(scratch.occurrence(&memory, 0x100, 0x8000), Ok(()));
    assert_eq!(
        scratch.occurrence(&changed_memory, 0x100, 0x8000),
        Err("war ownership snapshot changed or became unreadable")
    );

    scratch.begin();
    assert_eq!(scratch.occurrence(&memory, 0x100, 0x8000), Ok(()));
    let mut invalid = bytes.clone();
    p32(&mut invalid, 0x120 + crate::cpdx_array::COUNT, 3);
    let invalid_memory = SliceMemory {
        base: 0,
        bytes: &invalid,
    };
    let expected = original_owned_occurrence(&invalid_memory, 0x100, 0x8000);
    assert_eq!(expected, Err("war ownership vector unreadable or invalid"));
    assert_eq!(scratch.occurrence(&invalid_memory, 0x100, 0x8000), expected);
}

fn expected_warmed_war1_reads() -> Vec<(usize, usize)> {
    let mut reads = vec![
        (WAR1, 8),
        (WAR1 + crelation::TYPE_ID, 4),
        (WAR1 + crelation::RETIRED, 1),
        (WAR1 + crelation::FIRST_COUNTRY, 4),
        (WAR1 + crelation::SECOND_COUNTRY, 4),
        (DIPLOMACY[0] + cdiplomacy_status::RELATIONS, 16),
        (matrix(0) + 2 * 8, 8),
        (status(0, 2), 8),
        (status(0, 2) + crelation_status::DIPLOMACY_STATUS, 8),
        (status(0, 2) + crelation_status::TARGET_COUNTRY, 4),
        (status(0, 2) + crelation_status::WAR_RELATION, 8),
        (DIPLOMACY[1] + cdiplomacy_status::RELATIONS, 16),
        (matrix(1) + 8, 8),
        (status(1, 1), 8),
        (status(1, 1) + crelation_status::DIPLOMACY_STATUS, 8),
        (status(1, 1) + crelation_status::TARGET_COUNTRY, 4),
        (status(1, 1) + crelation_status::WAR_RELATION, 8),
    ];
    reads.extend([
        (DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS, 16),
        (owned(0), 8),
        (owned(0) + 8, 8),
        (owned(0) + 16, 8),
        (DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS, 16),
        (owned(1), 8),
        (status(0, 2) + crelation_status::SUBRELATIONS, 16),
        (status(0, 2) + 0x400, 8),
        (status(1, 1) + crelation_status::SUBRELATIONS, 16),
        (status(1, 1) + 0x400, 8),
        (status(0, 2) + crelation_status::SERIALIZABLE_RELATIONS, 16),
        (status(0, 2) + 0x420, 8),
        (status(1, 1) + crelation_status::SERIALIZABLE_RELATIONS, 16),
    ]);
    reads.extend([
        (WAR1 + crelation::START_HOUR, 4),
        (WAR1 + crelation::END_HOUR, 4),
        (WAR1 + crelation::CANCEL, 1),
        (WAR1 + FIRST_CASUALTIES, 8),
        (WAR1 + SECOND_CASUALTIES, 8),
        (WAR1 + FIRST_UNKNOWN_CASUALTIES, 8),
        (WAR1 + SECOND_UNKNOWN_CASUALTIES, 8),
        (WAR1 + FIRST_WAS_INSTIGATOR, 1),
    ]);
    reads
}

fn warmed_war1_reader<'a, M: Memory>(memory: &'a M) -> (Reader<'a, M>, ccountry::Country) {
    let mut walker = reader(memory);
    let first = walker.resolve(4).unwrap();
    let second = walker.resolve(2).unwrap();
    walker.diplomacy(first).unwrap();
    walker.diplomacy(second).unwrap();
    walker.current_matrix = (DIPLOMACY[0], None);
    (walker, first)
}

#[test]
fn status_header_hints_keep_primary_scalar_observation_order() {
    let bytes = fixture();
    let memory = ObservedMemory::new(&bytes);
    let (mut walker, first) = warmed_war1_reader(&memory);
    memory.reads.borrow_mut().clear();
    assert!(walker.relation(WAR1, first).unwrap().is_some());
    assert_eq!(*memory.reads.borrow(), expected_warmed_war1_reads());
}

#[test]
fn current_matrix_header_reuse_keeps_every_other_observation() {
    let bytes = fixture();
    let memory = ObservedMemory::new(&bytes);
    let (mut walker, first) = warmed_war1_reader(&memory);
    let expected_row = walker.relation(WAR1, first).unwrap();
    memory.reads.borrow_mut().clear();
    assert_eq!(walker.relation(WAR1, first), Ok(expected_row));
    let mut expected = expected_warmed_war1_reads();
    expected.retain(|&span| span != (DIPLOMACY[0] + cdiplomacy_status::RELATIONS, 16));
    assert_eq!(*memory.reads.borrow(), expected);
}

#[test]
fn owned_status_reuse_keeps_both_owner_orders_and_exact_remaining_reads() {
    let bytes = fixture();
    let baseline = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let (mut original, first) = warmed_war1_reader(&baseline);
    let row = original.relation(WAR1, first).unwrap();
    let full = expected_warmed_war1_reads();
    for emitter_first in [false, true] {
        let memory = ObservedMemory::new(&bytes);
        let mut scratch = prepared_snapshot(5, 8);
        let allocated = scratch.allocated_bytes();
        let (mut walker, first) = warmed_war1_reader(&memory);
        let second = walker.resolve(2).unwrap();
        walker.ownership = Some(&mut scratch);
        let owners = if emitter_first {
            [first, second]
        } else {
            [second, first]
        };
        walker.current_matrix = (DIPLOMACY[usize::from(!emitter_first)], None);
        memory.reads.borrow_mut().clear();
        assert_eq!(
            walker.relation(WAR1, owners[0]),
            Ok(if emitter_first { row } else { None })
        );
        assert_eq!(
            &*memory.reads.borrow(),
            &full[..full.len() - if emitter_first { 0 } else { 8 }]
        );
        walker.current_matrix = (
            if emitter_first {
                DIPLOMACY[1]
            } else {
                DIPLOMACY[0]
            },
            None,
        );
        memory.reads.borrow_mut().clear();
        assert_eq!(
            walker.relation(WAR1, owners[1]),
            Ok(if emitter_first { None } else { row })
        );
        let mut expected = full[..5].to_vec();
        expected.extend([
            (DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS, 16),
            (DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS, 16),
            (owned(1), 8),
        ]);
        if !emitter_first {
            expected.extend_from_slice(&full[full.len() - 8..]);
        }
        assert_eq!(*memory.reads.borrow(), expected);
        let third = walker.resolve(3).unwrap();
        assert_eq!(
            walker.relation(WAR1, third),
            Err("war relation does not include owning country")
        );
        assert_eq!(scratch.allocated_bytes(), allocated);
        assert_eq!(
            scratch
                .status_graphs
                .iter()
                .filter(|graph| graph.war != 0)
                .count(),
            1
        );

        // A new walk must reauthenticate statuses even with the same keys.
        let mut changed = bytes.clone();
        p64(&mut changed, status(0, 2), 0);
        let memory = SliceMemory {
            base: 0,
            bytes: &changed,
        };
        let mut expected = [Row::default(); 2];
        let mut actual = expected;
        assert_eq!(
            walk_owned(&memory, &memory, 0, GS, &mut actual, &mut scratch),
            walk(&memory, 0, GS, &mut expected)
        );
        assert_eq!(actual, expected);
        assert!(!scratch.status_graphs.iter().any(|graph| graph.war == WAR1));
    }
}

#[test]
fn owned_status_reuse_preserves_first_faults_and_fresh_owner_faults() {
    let bytes = fixture();
    let full = expected_warmed_war1_reads();
    for &(address, width) in &full[..full.len() - 8] {
        for byte in [0, width - 1] {
            let mut original = ObservedMemory::new(&bytes);
            original.blocked = address + byte..address + byte + 1;
            let mut candidate = ObservedMemory::new(&bytes);
            candidate.blocked = original.blocked.clone();
            let (mut original_reader, first) = warmed_war1_reader(&original);
            let (mut candidate_reader, _) = warmed_war1_reader(&candidate);
            let mut scratch = prepared_snapshot(5, 8);
            candidate_reader.ownership = Some(&mut scratch);
            original.reads.borrow_mut().clear();
            candidate.reads.borrow_mut().clear();
            let expected = original_reader.relation(WAR1, first);
            assert!(expected.is_err());
            assert_eq!(candidate_reader.relation(WAR1, first), expected);
            assert_eq!(*candidate.reads.borrow(), *original.reads.borrow());
            assert!(scratch.status_graphs.iter().all(|graph| graph.war == 0));
        }
    }
    let field_error = "war relation field unreadable or address overflow";
    for (address, reason) in [
        (WAR1, "war relation vtable mismatch"),
        (WAR1 + crelation::TYPE_ID, field_error),
        (WAR1 + crelation::RETIRED, field_error),
        (WAR1 + crelation::FIRST_COUNTRY, field_error),
        (WAR1 + crelation::SECOND_COUNTRY, field_error),
        (
            DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS,
            "war ownership vector unreadable or invalid",
        ),
        (
            DIPLOMACY[1] + cdiplomacy_status::OWNED_RELATIONS,
            "war ownership vector unreadable or invalid",
        ),
        (owned(1), "war ownership pointer unreadable"),
        (WAR1 + FIRST_CASUALTIES, field_error),
    ] {
        let memory = CountedMemory {
            bytes: &bytes,
            guards: Cell::new(0),
            blocked: Cell::new(usize::MAX),
        };
        let mut scratch = prepared_snapshot(5, 8);
        let (mut walker, first) = warmed_war1_reader(&memory);
        walker.ownership = Some(&mut scratch);
        assert!(walker.relation(WAR1, first).unwrap().is_some());
        memory.blocked.set(address);
        assert_eq!(walker.relation(WAR1, first), Err(reason));
    }
}

#[test]
fn owned_status_reuse_checks_raw_alias_keys_and_storage_falls_back() {
    let bytes = fixture();
    let mut scratch = prepared_snapshot(5, 8);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let (mut walker, first) = warmed_war1_reader(&memory);
    walker.ownership = Some(&mut scratch);
    assert!(walker.relation(WAR1, first).unwrap().is_some());

    // Both raw references resolve to the same canonical country. Changing
    // the source word must still miss the previously authenticated graph.
    let mut changed = bytes.clone();
    p32(&mut changed, WAR1 + crelation::FIRST_COUNTRY, 1);
    let mut memory = ObservedMemory::new(&changed);
    memory.blocked = status(0, 2)..status(0, 2) + 1;
    let (mut walker, first) = warmed_war1_reader(&memory);
    walker.ownership = Some(&mut scratch);
    assert_eq!(
        walker.relation(WAR1, first),
        Err("relation status vtable mismatch")
    );

    // A matching status result cannot bypass the existing exact owned
    // header guard, including a geometrically valid capacity-only change.
    let mut changed = bytes.clone();
    p32(
        &mut changed,
        DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS + crate::cpdx_array::CAPACITY,
        4,
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &changed,
    };
    let (mut walker, first) = warmed_war1_reader(&memory);
    walker.ownership = Some(&mut scratch);
    assert_eq!(
        walker.relation(WAR1, first),
        Err("war ownership snapshot changed or became unreadable")
    );

    for full in [false, true] {
        let memory = ObservedMemory::new(&bytes);
        let mut scratch = OwnedSnapshotScratch::with_capacity(5);
        if full {
            scratch.status_graphs.fill(StatusGraph {
                war: usize::MAX,
                ..StatusGraph::default()
            });
        } else {
            scratch.status_graphs.clear();
        }
        let allocated = scratch.allocated_bytes();
        let (mut walker, first) = warmed_war1_reader(&memory);
        walker.ownership = Some(&mut scratch);
        let expected = walker.relation(WAR1, first).unwrap();
        memory.reads.borrow_mut().clear();
        assert_eq!(walker.relation(WAR1, first), Ok(expected));
        let mut reads = expected_warmed_war1_reads();
        reads.retain(|&span| span != (DIPLOMACY[0] + cdiplomacy_status::RELATIONS, 16));
        assert_eq!(*memory.reads.borrow(), reads);
        assert_eq!(scratch.allocated_bytes(), allocated);
    }
}

#[test]
fn matrix_reuse_and_status_hints_preserve_each_first_header_fault() {
    let bytes = fixture();
    let expected = expected_warmed_war1_reads();
    for (roots, offsets, reason) in [
        (
            [DIPLOMACY[0], DIPLOMACY[1]],
            &[cdiplomacy_status::RELATIONS][..],
            "war matrix unreadable or invalid",
        ),
        (
            [status(0, 2), status(1, 1)],
            &[
                crelation_status::SUBRELATIONS,
                crelation_status::SERIALIZABLE_RELATIONS,
            ][..],
            "war ownership vector unreadable or invalid",
        ),
    ] {
        for address in roots
            .into_iter()
            .flat_map(|root| offsets.iter().map(move |offset| root + offset))
        {
            let failed = expected
                .iter()
                .position(|&span| span == (address, 16))
                .unwrap();
            for byte in 0..16 {
                let mut memory = ObservedMemory::new(&bytes);
                memory.blocked = address + byte..address + byte + 1;
                let (mut walker, first) = warmed_war1_reader(&memory);
                memory.reads.borrow_mut().clear();
                assert_eq!(walker.relation(WAR1, first), Err(reason));
                assert_eq!(&*memory.reads.borrow(), &expected[..=failed]);
            }
        }
    }
    for pointer in [0, usize::MAX, usize::MAX - 0xd0, usize::MAX - 0xe8 - 15] {
        hint_owned_status_headers(pointer);
    }
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[test]
fn status_header_hints_preserve_owned_page_protection_after_later_auth_failure() {
    use std::ffi::c_void;

    const PAGE: usize = 4096;
    const READWRITE: u32 = 4;
    const NOACCESS: u32 = 1;
    const GUARD: u32 = 0x100;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }

    struct OwnedStatus {
        page: *mut c_void,
        offset: usize,
    }

    impl OwnedStatus {
        fn new(bytes: &[u8], offset: usize) -> Self {
            assert!(offset + 0x300 <= PAGE);
            // SAFETY: this fixture owns one writable, non-executable page.
            let page = unsafe { VirtualAlloc(std::ptr::null_mut(), PAGE, 0x3000, READWRITE) };
            assert!(!page.is_null());
            let owned_status = Self { page, offset };
            let source = &bytes[status(0, 2)..status(0, 2) + 0x300];
            // SAFETY: source is a complete immutable fixture prefix and the
            // destination is disjoint, writable owned storage.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr(),
                    owned_status.pointer() as *mut u8,
                    source.len(),
                );
            }
            owned_status
        }

        fn pointer(&self) -> usize {
            self.page as usize + self.offset
        }

        fn protect(&self, protection: u32) -> u32 {
            let mut previous = 0;
            // SAFETY: changes only this fixture's one owned page.
            assert_ne!(
                unsafe { VirtualProtect(self.page, PAGE, protection, &mut previous) },
                0
            );
            previous
        }
    }

    impl Drop for OwnedStatus {
        fn drop(&mut self) {
            // SAFETY: releases only this fixture's complete allocation.
            unsafe { VirtualFree(self.page, 0, 0x8000) };
        }
    }

    struct ArmedStatusMemory<'a> {
        bytes: &'a [u8],
        status: &'a OwnedStatus,
        live: crate::memory::LiveMemory,
        protection: u32,
        armed: Cell<bool>,
    }

    impl Memory for ArmedStatusMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address
                .checked_sub(self.status.pointer())
                .and_then(|offset| offset.checked_add(destination.len()))
                .is_some_and(|end| end <= 0x300)
            {
                let copied = self.live.copy(address, destination);
                if copied
                    && address == self.status.pointer() + crelation_status::WAR_RELATION
                    && destination.len() == 8
                {
                    assert!(!self.armed.replace(true));
                    assert_eq!(self.status.protect(self.protection), READWRITE);
                }
                return copied;
            }
            // The first status authenticated completely. Fail the second
            // status before any original ownership-header read is reached.
            if address == status(1, 1) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    // Both 16-byte headers are covered, including each one's possible
    // cache-line crossing. Hinting must not consume even the future guard.
    for offset in [0, 0x10, 0x28] {
        for protection in [NOACCESS, READWRITE | GUARD] {
            let mut bytes = fixture();
            let owned_status = OwnedStatus::new(&bytes, offset);
            p64(&mut bytes, matrix(0) + 2 * 8, owned_status.pointer() as u64);
            let memory = ArmedStatusMemory {
                bytes: &bytes,
                status: &owned_status,
                // SAFETY: foreign requests here go only to this fixture's
                // owned readable or deliberately protected status storage.
                live: unsafe { crate::memory::LiveMemory::new() },
                protection,
                armed: Cell::new(false),
            };
            let (mut walker, first) = warmed_war1_reader(&memory);
            assert_eq!(
                walker.relation(WAR1, first),
                Err("relation status vtable mismatch")
            );
            assert!(memory.armed.get());
            assert_eq!(owned_status.protect(READWRITE), protection);
            owned_status.protect(protection);
            let mut destination = [0xa5; 16];
            assert!(!memory.live.copy(
                owned_status.pointer() + crelation_status::SUBRELATIONS,
                &mut destination
            ));
            assert_eq!(destination, [0xa5; 16]);
            assert_eq!(
                owned_status.protect(READWRITE),
                if protection & GUARD != 0 {
                    READWRITE
                } else {
                    NOACCESS
                }
            );
        }
    }
}

#[test]
fn fixed_gathers_preserve_exact_widths_and_repeated_observations() {
    let bytes = fixture();
    let memory = ObservedMemory::new(&bytes);
    let mut reader = reader(&memory);
    let first = reader.countries.at(1).unwrap().unwrap();
    let second = reader.countries.at(2).unwrap().unwrap();
    memory.reads.borrow_mut().clear();
    assert!(reader.relation(WAR1, first).unwrap().is_some());
    assert_eq!(reader.relation(WAR1, second), Ok(None));
    let reads = memory.reads.borrow();
    let fields = [
        (0, 8),
        (crelation::TYPE_ID, 4),
        (crelation::RETIRED, 1),
        (crelation::FIRST_COUNTRY, 4),
        (crelation::SECOND_COUNTRY, 4),
        (crelation::START_HOUR, 4),
        (crelation::END_HOUR, 4),
        (crelation::CANCEL, 1),
        (FIRST_CASUALTIES, 8),
        (SECOND_CASUALTIES, 8),
        (FIRST_UNKNOWN_CASUALTIES, 8),
        (SECOND_UNKNOWN_CASUALTIES, 8),
        (FIRST_WAS_INSTIGATOR, 1),
    ];
    let expected: Vec<_> = fields
        .iter()
        .chain(&fields[..5])
        .map(|&(offset, width)| (WAR1 + offset, width))
        .collect();
    assert_eq!(
        reads
            .iter()
            .copied()
            .filter(|&(at, _)| (WAR1..WAR1 + 0x200).contains(&at))
            .collect::<Vec<_>>(),
        expected
    );
    for pointer in [status(0, 2), status(1, 1)] {
        let expected: Vec<_> = [
            (pointer, 8),
            (pointer + crelation_status::DIPLOMACY_STATUS, 8),
            (pointer + crelation_status::TARGET_COUNTRY, 4),
            (pointer + crelation_status::WAR_RELATION, 8),
        ]
        .into_iter()
        .cycle()
        .take(8)
        .collect();
        assert_eq!(
            reads
                .iter()
                .copied()
                .filter(|&(at, _)| {
                    [
                        pointer,
                        pointer + crelation_status::DIPLOMACY_STATUS,
                        pointer + crelation_status::TARGET_COUNTRY,
                        pointer + crelation_status::WAR_RELATION,
                    ]
                    .contains(&at)
                })
                .collect::<Vec<_>>(),
            expected
        );
    }
    for pointer in [owned(0), owned(0) + 8, owned(0) + 16, owned(1)] {
        assert_eq!(
            reads.iter().filter(|&&span| span == (pointer, 8)).count(),
            2
        );
    }
    assert_eq!(*memory.gathers.borrow(), [2, 3, 3, 1, 8, 2, 3, 3, 1]);
    assert_eq!(
        memory
            .gathers
            .borrow()
            .iter()
            .map(|fields| fields - 1)
            .sum::<usize>(),
        17
    );
}

#[test]
fn pair_gather_resolves_uncached_alias_before_observing_war_pointer() {
    let bytes = fixture();
    let memory = ObservedMemory::new(&bytes);
    let mut walker = reader(&memory);
    let first = walker.countries.at(1).unwrap().unwrap();
    memory.reads.borrow_mut().clear();
    assert_eq!(
        walker.pair_status(DIPLOMACY[1], first, WAR1),
        Ok(status(1, 1))
    );
    let reads = memory.reads.borrow();
    let target = reads
        .iter()
        .position(|&(at, _)| at == status(1, 1) + crelation_status::TARGET_COUNTRY)
        .unwrap();
    let war = reads
        .iter()
        .position(|&(at, _)| at == status(1, 1) + crelation_status::WAR_RELATION)
        .unwrap();
    assert_eq!(
        &reads[target + 1..war],
        &[
            (LINKS + 4 * 4, 4),
            (COUNTRIES + 8, 8),
            (COUNTRY[0], 8),
            (COUNTRY[0] + ccountry::ID, 4),
            (LINKS + 4 * 4, 4),
            (LINKS + 4, 4),
            (LINKS + 4, 4),
            (COUNTRIES + 8, 8),
        ]
    );
    drop(reads);
    let mut memory = ObservedMemory::new(&bytes);
    memory.blocked = COUNTRY[0] + ccountry::ID..COUNTRY[0] + ccountry::ID + 1;
    assert_eq!(
        reader(&memory).pair_status(DIPLOMACY[1], first, WAR1),
        Err("country id unreadable")
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| { at == status(1, 1) + crelation_status::WAR_RELATION })
    );
}

#[test]
fn fixed_gather_faults_preserve_reasons_capacity_and_earlier_rows() {
    let bytes = fixture();
    let mut expected = [Row::default(); 4];
    assert_eq!(capture(&bytes, &mut expected), Outcome::Rows { len: 2 });
    let field_error = "war relation field unreadable or address overflow";
    for (at, width, reason) in [
        (WAR1 + crelation::FIRST_COUNTRY, 4, field_error),
        (WAR1 + crelation::SECOND_COUNTRY, 4, field_error),
        (WAR1 + crelation::START_HOUR, 4, field_error),
        (WAR1 + crelation::END_HOUR, 4, field_error),
        (WAR1 + crelation::CANCEL, 1, field_error),
        (WAR1 + FIRST_CASUALTIES, 8, field_error),
        (WAR1 + SECOND_CASUALTIES, 8, field_error),
        (WAR1 + FIRST_UNKNOWN_CASUALTIES, 8, field_error),
        (WAR1 + SECOND_UNKNOWN_CASUALTIES, 8, field_error),
        (WAR1 + FIRST_WAS_INSTIGATOR, 1, field_error),
        (status(0, 2), 8, "relation status vtable mismatch"),
        (
            status(0, 2) + crelation_status::DIPLOMACY_STATUS,
            8,
            field_error,
        ),
        (
            status(0, 2) + crelation_status::TARGET_COUNTRY,
            4,
            field_error,
        ),
    ] {
        for byte in 0..width {
            let memory = FaultMemory {
                bytes: &bytes,
                blocked: at + byte..at + byte + 1,
            };
            for capacity in [0, 4] {
                let mut output = [Row::default(); 4];
                assert_eq!(
                    walk(&memory, 0, GS, &mut output[..capacity]),
                    Outcome::Unavailable { reason },
                    "field {at:x}+{byte}, capacity={capacity}"
                );
                if capacity != 0 {
                    assert_eq!(output[0], expected[0]);
                }
            }
        }
    }
}

#[test]
fn pair_gather_keeps_semantic_errors_ahead_of_later_faults() {
    for (invalid, replacement, unreadable, reason) in [
        (
            status(0, 3),
            0,
            status(0, 3) + crelation_status::DIPLOMACY_STATUS,
            "relation status vtable mismatch",
        ),
        (
            status(0, 3) + crelation_status::DIPLOMACY_STATUS,
            DIPLOMACY[1] as u64,
            status(0, 3) + crelation_status::TARGET_COUNTRY,
            "relation status diplomacy mismatch",
        ),
        (
            status(0, 3) + crelation_status::TARGET_COUNTRY,
            0,
            status(0, 3) + crelation_status::WAR_RELATION,
            "country id is zero",
        ),
        (
            status(0, 3) + crelation_status::TARGET_COUNTRY,
            2,
            status(0, 3) + crelation_status::WAR_RELATION,
            "relation status target differs from matrix slot",
        ),
        (
            WAR2 + crelation::FIRST_COUNTRY,
            0,
            WAR2 + crelation::SECOND_COUNTRY,
            "war relation field unreadable or address overflow",
        ),
        (
            status(0, 3) + 0x400,
            0,
            WAR2 + crelation::START_HOUR,
            "null war ownership entry",
        ),
    ] {
        let mut bytes = fixture();
        // Four-byte replacements must not also change their adjacent field.
        if invalid == WAR2 + crelation::FIRST_COUNTRY
            || invalid == status(0, 3) + crelation_status::TARGET_COUNTRY
        {
            p32(&mut bytes, invalid, replacement as u32);
        } else {
            p64(&mut bytes, invalid, replacement);
        }
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: unreadable..unreadable + 1,
        };
        for capacity in [0, 4] {
            assert_eq!(
                walk(&memory, 0, GS, &mut [Row::default(); 4][..capacity]),
                Outcome::Unavailable { reason }
            );
        }
    }
}

#[test]
fn fixed_gathers_leave_gaps_and_nonemitting_rows_unread() {
    let bytes = fixture();
    for gap in [
        WAR1 + crelation::CANCEL + 2,
        WAR1 + SECOND_UNKNOWN_CASUALTIES + 8,
        WAR1 + FIRST_WAS_INSTIGATOR + 1,
        status(0, 2) + 8,
        status(0, 2) + crelation_status::DIPLOMACY_STATUS + 8,
        status(0, 2) + crelation_status::TARGET_COUNTRY + 4,
    ] {
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: gap..gap + 1,
        };
        assert_eq!(
            walk(&memory, 0, GS, &mut [Row::default(); 4]),
            Outcome::Rows { len: 2 }
        );
    }
    let mut memory = ObservedMemory::new(&bytes);
    memory.blocked = WAR1 + crelation::START_HOUR..WAR1 + crelation::START_HOUR + 1;
    let mut walker = reader(&memory);
    let second = walker.countries.at(2).unwrap().unwrap();
    assert_eq!(walker.relation(WAR1, second), Ok(None));
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| { at == WAR1 + crelation::START_HOUR })
    );
    let mut bytes = fixture();
    bytes[WAR1 + crelation::RETIRED] = 1;
    let memory = FaultMemory {
        bytes: &bytes,
        blocked: WAR1 + crelation::FIRST_COUNTRY..WAR1 + crelation::FIRST_COUNTRY + 4,
    };
    let mut reader = reader(&memory);
    let first = reader.countries.at(1).unwrap().unwrap();
    assert_eq!(reader.relation(WAR1, first), Ok(None));
}

#[test]
fn dense_authentication_cells_key_full_ids_and_fall_back_beyond_storage() {
    // A raw alias far above the former 512-cell direct map.
    const FAR: usize = 20_000;
    let mut bytes = fixture();
    const EXPANDED_LINKS: usize = 0x20000;
    bytes.resize(EXPANDED_LINKS + (FAR + 1) * 4, 0);
    p64(
        &mut bytes,
        GS + cgame_state::COUNTRY_LINKS,
        EXPANDED_LINKS as u64,
    );
    for offset in [8, 12] {
        p32(
            &mut bytes,
            GS + cgame_state::COUNTRY_LINKS + offset,
            (FAR + 1) as u32,
        );
    }
    for (raw, canonical) in [(1, 1), (2, 2), (3, 3), (4, 1), (FAR, 1)] {
        p32(&mut bytes, EXPANDED_LINKS + raw * 4, canonical);
    }
    let memory = CountedMemory {
        bytes: &bytes,
        guards: Cell::new(0),
        blocked: Cell::new(usize::MAX),
    };
    let mut dense = reader(&memory);
    assert_eq!(dense.resolver.capacity(), [FAR + 1, 5]);
    let start = memory.guards.get();
    let first = dense.resolve(1).unwrap();
    assert_eq!(memory.guards.get() - start, 8);
    assert_eq!(dense.resolve(1), Ok(first));
    assert_eq!(memory.guards.get() - start, 8);
    // The alias has its own cell: it is validated once and never evicts 1.
    assert_eq!(dense.resolve(FAR as u32), Ok(first));
    assert_eq!(memory.guards.get() - start, 16);
    assert_eq!(dense.resolve(1), Ok(first));
    assert_eq!(dense.resolve(FAR as u32), Ok(first));
    assert_eq!(memory.guards.get() - start, 16);
    assert_eq!(
        dense.resolve(0x8000_0001),
        Err("country link unreadable or outside country table")
    );
    assert_eq!(
        dense.resolve(u32::MAX),
        Err("country link unreadable or outside country table")
    );
    assert_eq!(dense.resolve(0), Err("country id is zero"));
    assert_eq!(dense.resolve(1), Ok(first));
    assert_eq!(memory.guards.get() - start, 16);
    assert_eq!(first.id, 4); // Authentication never substitutes the queried alias.
    let start = memory.guards.get();
    assert_eq!(dense.diplomacy(first), Ok(DIPLOMACY[0]));
    assert_eq!(dense.diplomacy(first), Ok(DIPLOMACY[0]));
    assert_eq!(memory.guards.get() - start, 3);

    // Storage shorter than the live headers is never a refusal: uncovered
    // ids and slots keep the original uncached validators, and the walk
    // reports the observed headers for off-thread growth.
    let countries = ccountry::Reader::new(&memory, 0, GS).unwrap();
    let resolver = Box::leak(Box::new(crelation_status::Resolver::with_capacity(2)));
    resolver.begin(&countries);
    let mut short = Reader {
        resolver,
        ..reader(&memory)
    };
    let start = memory.guards.get();
    for round in 1..=2 {
        assert_eq!(short.resolve(FAR as u32), Ok(first));
        assert_eq!(memory.guards.get() - start, 8 * round);
    }
    assert_eq!(short.resolve(1), Ok(first));
    let start = memory.guards.get();
    assert_eq!(short.resolve(1), Ok(first));
    assert_eq!(memory.guards.get() - start, 0);
    for _ in 0..2 {
        assert_eq!(short.diplomacy(first), Ok(DIPLOMACY[0]));
    }
    assert_eq!(memory.guards.get() - start, 3);
    let third = short.resolve(3).unwrap();
    let start = memory.guards.get();
    for _ in 0..2 {
        assert_eq!(short.diplomacy(third), Ok(DIPLOMACY[2]));
    }
    assert_eq!(memory.guards.get() - start, 6);
    assert_eq!(short.resolver.required(), [FAR + 1, 5]);
    short.resolver.grow().unwrap();
    assert_eq!(short.resolver.capacity(), [FAR + 1, 5]);
}

#[test]
fn successful_authentication_is_local_and_failed_authentication_is_never_cached() {
    let bytes = fixture();
    let memory = CountedMemory {
        bytes: &bytes,
        guards: Cell::new(0),
        blocked: Cell::new(usize::MAX),
    };
    let mut first_reader = reader(&memory);
    let country = first_reader.resolve(4).unwrap();
    memory.blocked.set(COUNTRY[0] + ccountry::ID);
    // A changing address space is outside this cache's stability contract.
    assert_eq!(first_reader.resolve(4), Ok(country));
    assert_eq!(reader(&memory).resolve(4), Err("country id unreadable"));
    for _ in 0..2 {
        assert_eq!(
            walk(&memory, 0, GS, &mut []),
            Outcome::Unavailable {
                reason: "country id unreadable"
            }
        );
    }
    memory.blocked.set(COUNTRY[1]);
    assert_eq!(first_reader.resolve(2), Err("country vtable mismatch"));
    memory.blocked.set(usize::MAX);
    assert_eq!(first_reader.resolve(2).unwrap().id, 2);

    let mut matrix_reader = reader(&memory);
    matrix_reader.current_matrix = (DIPLOMACY[0], None);
    let second = matrix_reader.resolve(2).unwrap();
    memory
        .blocked
        .set(DIPLOMACY[0] + cdiplomacy_status::RELATIONS);
    let start = memory.guards.get();
    for _ in 0..2 {
        assert_eq!(
            matrix_reader.pair_status(DIPLOMACY[0], second, WAR1),
            Err("war matrix unreadable or invalid")
        );
    }
    assert_eq!(memory.guards.get() - start, 2);
    memory.blocked.set(usize::MAX);
    assert_eq!(
        matrix_reader.pair_status(DIPLOMACY[0], second, WAR1),
        Ok(status(0, 2))
    );
}

#[test]
fn pointer_batches_use_exact_strided_reads_and_preserve_failure_prefixes() {
    struct Routed<'a> {
        memory: ObservedMemory<'a>,
        batches: RefCell<Vec<Vec<(usize, usize)>>>,
        records: RefCell<Vec<(usize, usize, usize)>>,
    }
    impl Memory for Routed<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.memory.copy(address, destination)
        }

        fn copy_scattered<const N: usize>(&self, _: [usize; N], _: &mut [&mut [u8]; N]) -> usize {
            panic!("these war batches must opt into the prefetched route");
        }

        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.batches.borrow_mut().push(
                addresses
                    .iter()
                    .zip(destinations.iter())
                    .map(|(&address, destination)| (address, destination.len()))
                    .collect(),
            );
            self.memory
                .copy_scattered_prefetched(addresses, destinations)
        }

        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            let RecordSources::Strided {
                base,
                stride,
                count,
            } = sources
            else {
                panic!("pointer batches must use the strided route");
            };
            assert_eq!(layout.stride(), 8);
            assert_eq!(layout.field_count(), 1);
            let field = layout.fields()[0];
            assert_eq!(
                (field.source_offset, field.destination_offset, field.len),
                (0, 0, 8)
            );
            assert_eq!(destination.len(), count * 8);
            self.records.borrow_mut().push((base, stride, count));
            self.memory.copy_records(sources, layout, destination)
        }
    }

    let bytes = fixture();
    let routed = || Routed {
        memory: ObservedMemory::new(&bytes),
        batches: RefCell::new(Vec::new()),
        records: RefCell::new(Vec::new()),
    };
    for count in [0, 1, 2, 3, 9, 17, BATCH_SIZE] {
        let memory = routed();
        let mut batch = PointerBatch::default();
        batch.read(
            &memory,
            Array {
                data: matrix(0),
                count: count as u32 + 1,
            },
            1,
        );
        let expected: Vec<_> = (1..=count)
            .map(|index| (matrix(0) + index * 8, 8))
            .collect();
        assert_eq!(batch.copied, count);
        assert_eq!(*memory.memory.reads.borrow(), expected);
        assert!(memory.batches.borrow().is_empty());
        if count < 2 {
            assert!(memory.records.borrow().is_empty());
        } else {
            assert_eq!(*memory.records.borrow(), [(matrix(0) + 8, 8, count)]);
        }
        for (failed, &(address, _)) in expected.iter().enumerate() {
            let mut memory = routed();
            memory.memory.blocked = address..address + 1;
            let mut batch = PointerBatch {
                values: [[0xa5; 8]; BATCH_SIZE],
                ..PointerBatch::default()
            };
            batch.read(
                &memory,
                Array {
                    data: matrix(0),
                    count: count as u32 + 1,
                },
                1,
            );
            assert_eq!(batch.copied, failed);
            assert_eq!(*memory.memory.reads.borrow(), expected[..=failed]);
            assert_eq!(batch.values[failed][0], 0xff);
            assert!(
                batch.values[failed + 1..]
                    .iter()
                    .all(|row| *row == [0xa5; 8])
            );
        }
    }

    let memory = routed();
    let walker = reader(&memory);
    memory.memory.reads.borrow_mut().clear();
    assert_eq!(walker.active(WAR1), Ok(true));
    let expected = vec![
        (WAR1, 8),
        (WAR1 + crelation::TYPE_ID, 4),
        (WAR1 + crelation::RETIRED, 1),
    ];
    assert_eq!(*memory.memory.reads.borrow(), expected);
    assert_eq!(*memory.batches.borrow(), [expected]);
}

#[test]
fn owned_census_preserves_aliases_native_values_and_truncation() {
    let bytes = fixture();
    let mut rows = [Row::default(); 4];
    let memory = ObservedMemory::new(&bytes);
    assert_eq!(walk(&memory, 0, GS, &mut rows), Outcome::Rows { len: 2 });
    let mut scratch = prepared_snapshot(3, 8);
    for (pass, matrix_counts) in [[3, 2, 2], [4, 3, 3]].into_iter().enumerate() {
        // The direct walk reads seven matrix headers. The owned walk adds
        // three: each war's status graph is validated only on its first
        // visit, and the shared first owner's matrix header is reused.
        let reads = memory.reads.borrow();
        for (owner, count) in matrix_counts.into_iter().enumerate() {
            assert_eq!(
                reads
                    .iter()
                    .filter(|&&span| {
                        span == (DIPLOMACY[owner] + cdiplomacy_status::RELATIONS, 16)
                    })
                    .count(),
                count
            );
            // Both walks retain the outer census header and every
            // diplomacy-owned membership header on both war visits.
            assert_eq!(
                reads
                    .iter()
                    .filter(|&&span| {
                        span == (DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS, 16)
                    })
                    .count(),
                [5, 3, 3][owner] * (pass + 1)
            );
        }
        drop(reads);
        if pass == 0 {
            let mut owned_rows = [Row::default(); 4];
            assert_eq!(
                walk_owned(&memory, &memory, 0, GS, &mut owned_rows, &mut scratch),
                Outcome::Rows { len: 2 }
            );
            assert_eq!(owned_rows, rows);
        }
    }
    assert_eq!(rows[0].second_country, 3);
    assert_eq!(
        rows[1],
        Row {
            first_country: 4,
            second_country: 2,
            start_hour: i32::MIN,
            end_hour: i32::MAX,
            cancel: 7,
            first_casualties: i64::MAX,
            second_casualties: i64::MIN,
            first_unknown_casualties: 0x1_0000_0001,
            second_unknown_casualties: -0x1_0000_0001,
            first_was_instigator: 255,
        }
    );
    for capacity in [0, 1] {
        assert_eq!(
            capture(&bytes, &mut rows[..capacity]),
            Outcome::Truncated {
                len: capacity,
                unread_rows: 2 - capacity as u32
            }
        );
    }
}

#[test]
fn inactive_installed_relations_and_readable_empty_vectors_are_not_wars() {
    let mut bytes = fixture();
    bytes[WAR1 + crelation::RETIRED] = 1;
    bytes[WAR2 + crelation::RETIRED] = 255;
    assert_eq!(capture(&bytes, &mut []), Outcome::Rows { len: 0 });
    for (owner, diplomacy) in DIPLOMACY.into_iter().enumerate() {
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::OWNED_RELATIONS,
            0,
            &[],
        );
        for target in 0..5 {
            p64(
                &mut bytes,
                status(owner, target) + crelation_status::WAR_RELATION,
                0,
            );
        }
    }
    assert_eq!(capture(&bytes, &mut []), Outcome::Rows { len: 0 });
}

#[test]
fn missing_duplicate_and_disagreeing_links_are_unavailable_even_at_capacity() {
    let cases = [
        (owned(1), WAR2 as u64),
        (owned(0) + 8, WAR2 as u64),
        (status(1, 1) + crelation_status::WAR_RELATION, WAR2 as u64),
        (
            status(0, 3) + crelation_status::DIPLOMACY_STATUS,
            DIPLOMACY[1] as u64,
        ),
        (DIPLOMACY[0] + cdiplomacy_status::COUNTRY, COUNTRY[1] as u64),
        (status(0, 3) + 0x400, WAR1 as u64),
        (status(0, 3) + 0x420, WAR1 as u64),
        (WAR2, 0),
    ];
    for (at, replacement) in cases {
        let mut bytes = fixture();
        p64(&mut bytes, at, replacement);
        let mut output = [Row::default(); 4];
        assert!(
            matches!(capture(&bytes, &mut output), Outcome::Unavailable { .. }),
            "address {at:x}"
        );
        assert!(
            matches!(capture(&bytes, &mut []), Outcome::Unavailable { .. }),
            "address {at:x}"
        );
    }
    let mut bytes = fixture();
    p32(&mut bytes, WAR2 + crelation::SECOND_COUNTRY, 1); // Alias of first, not another country.
    assert_eq!(
        capture(&bytes, &mut []),
        Outcome::Unavailable {
            reason: "war relation endpoints resolve to same country"
        }
    );
}

struct FaultMemory<'a> {
    bytes: &'a [u8],
    blocked: std::ops::Range<usize>,
}

impl Memory for FaultMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address < self.blocked.end && address + destination.len() > self.blocked.start {
            destination[0] = 0xff;
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
fn owned_prefetch_keeps_earlier_semantic_errors_before_later_faults() {
    for (invalid, unreadable, reason) in [
        (WAR2, owned(0) + 2 * 8, "war relation vtable mismatch"),
        (owned(0) + 8, owned(0) + 2 * 8, "null war ownership entry"),
    ] {
        let mut bytes = fixture();
        p64(&mut bytes, invalid, 0);
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: unreadable..unreadable + 1,
        };
        for capacity in [0, 4] {
            assert_eq!(
                walk(&memory, 0, GS, &mut [Row::default(); 4][..capacity]),
                Outcome::Unavailable { reason }
            );
        }
    }
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

/// `countries` canonical countries in slots 1..=countries. Country 1 is at
/// war with each of the first `linked` others (country 1 first) and its
/// owned vector is padded with a non-war relation to `countries - 1 +
/// fillers` entries, so its length does not depend on `linked`. Countries
/// without a linked war own nothing.
struct Census {
    bytes: Vec<u8>,
    rows: Vec<Row>,
    hub_words: usize,
}

fn war_census(countries: usize, linked: usize, fillers: usize) -> Census {
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
    let ids: Vec<u64> = (0..=countries as u64).collect();
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
    for (slot, id) in ids.iter().enumerate() {
        p32(&mut space.bytes, links + slot * 4, *id as u32);
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
    let filler = space.alloc(0x50);
    p32(&mut space.bytes, filler + crelation::TYPE_ID, 0x28da);
    let mut hub_matrix = vec![0_u64; countries + 1];
    let mut hub_owned = Vec::new();
    let mut rows = Vec::new();
    let status = |space: &mut Space, owner: usize, target: usize, war: usize, first| {
        let at = space.alloc(0x300);
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
            at + crelation_status::WAR_RELATION,
            war as u64,
        );
        space.vector(at + crelation_status::SUBRELATIONS, &[war as u64]);
        if first {
            space.vector(at + crelation_status::SERIALIZABLE_RELATIONS, &[war as u64]);
        }
        at
    };
    for partner in 2..2 + linked {
        let war = space.alloc(0x168);
        p64(&mut space.bytes, war, VTABLE_RVA as u64);
        p32(&mut space.bytes, war + crelation::TYPE_ID, TYPE);
        p32(&mut space.bytes, war + crelation::FIRST_COUNTRY, 1);
        p32(
            &mut space.bytes,
            war + crelation::SECOND_COUNTRY,
            partner as u32,
        );
        p32(
            &mut space.bytes,
            war + crelation::START_HOUR,
            partner as u32,
        );
        p32(&mut space.bytes, war + crelation::END_HOUR, u32::MAX);
        p64(&mut space.bytes, war + FIRST_CASUALTIES, partner as u64 * 3);
        p64(&mut space.bytes, war + SECOND_UNKNOWN_CASUALTIES, 7);
        space.bytes[war + FIRST_WAS_INSTIGATOR] = 1;
        hub_matrix[partner] = status(&mut space, 1, partner, war, true) as u64;
        let back = status(&mut space, partner, 1, war, false);
        let matrix = space.alloc(16);
        p64(&mut space.bytes, matrix + 8, back as u64);
        p64(
            &mut space.bytes,
            diplomacy[partner] + cdiplomacy_status::RELATIONS,
            matrix as u64,
        );
        for offset in [8, 12] {
            p32(
                &mut space.bytes,
                diplomacy[partner] + cdiplomacy_status::RELATIONS + offset,
                2,
            );
        }
        space.vector(
            diplomacy[partner] + cdiplomacy_status::OWNED_RELATIONS,
            &[war as u64],
        );
        hub_owned.push(war as u64);
        rows.push(Row {
            first_country: 1,
            second_country: partner as u32,
            start_hour: partner as i32,
            end_hour: -1,
            first_casualties: partner as i64 * 3,
            second_unknown_casualties: 7,
            first_was_instigator: 1,
            ..Row::default()
        });
    }
    hub_owned.resize(countries - 1 + fillers, filler as u64);
    space.vector(diplomacy[1] + cdiplomacy_status::RELATIONS, &hub_matrix);
    space.vector(
        diplomacy[1] + cdiplomacy_status::OWNED_RELATIONS,
        &hub_owned,
    );
    Census {
        bytes: space.bytes,
        rows,
        hub_words: hub_owned.len(),
    }
}

#[test]
fn owned_walk_at_census_scale_grows_past_former_budgets_and_stays_linear() {
    // 15,000 countries, far past the former 512-cell resolver, and an owned
    // vector of 74,999 entries, past the former 65,536-word snapshot budget.
    const COUNTRIES: usize = 15_000;
    const FILLERS: usize = 60_000;
    // One pass observes the demand. Its hub vector has the full length, but
    // only one linked war, so the complete fallback scans stay affordable.
    let learning = war_census(COUNTRIES, 1, FILLERS);
    let memory = SliceMemory {
        base: 0,
        bytes: &learning.bytes,
    };
    let mut scratch = OwnedSnapshotScratch::with_capacity(COUNTRIES + 1);
    let mut rows = vec![Row::default(); COUNTRIES];
    assert_eq!(
        walk_owned(&memory, &memory, 0, GS, &mut rows, &mut scratch),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0], learning.rows[0]);
    assert!(scratch.stats().fallbacks > 0);
    assert!(scratch.stats().required_words >= learning.hub_words);
    // Growth happens between passes, as on the spool thread.
    scratch.grow(COUNTRIES + 1).unwrap();
    assert!(scratch.capacity().words >= learning.hub_words);
    assert!(scratch.capacity().initialized_count_cells >= 2 * learning.hub_words);

    let census = war_census(COUNTRIES, COUNTRIES - 1, FILLERS);
    assert_eq!(census.hub_words, learning.hub_words);
    assert!(census.hub_words > 65_536);
    let memory = SliceMemory {
        base: 0,
        bytes: &census.bytes,
    };
    let allocated = scratch.allocated_bytes();
    assert_eq!(
        walk_owned(&memory, &memory, 0, GS, &mut rows, &mut scratch),
        Outcome::Rows { len: COUNTRIES - 1 }
    );
    assert_eq!(rows[..COUNTRIES - 1], census.rows[..]);
    assert_eq!(scratch.allocated_bytes(), allocated);
    let stats = scratch.stats();
    // The hub vector is scanned once and answered from its snapshot for
    // every later war; only one-entry partner vectors take the plain scan.
    // The former fallback would have consumed about 2.2e9 words here.
    assert!(stats.installed >= 1);
    assert!(stats.hits >= 2 * (COUNTRIES - 1) - 1);
    assert!(
        stats.acquisition_words <= census.hub_words + 4 * COUNTRIES,
        "{stats:?}"
    );
    // The status-graph table is sized from the observed wars next time.
    scratch.grow(COUNTRIES + 1).unwrap();
    assert!(scratch.status_graphs.len() >= 2 * (COUNTRIES - 1));
    // Output capacity still truncates honestly, never by country count.
    let mut short = vec![Row::default(); 10];
    assert_eq!(
        walk_owned(&memory, &memory, 0, GS, &mut short, &mut scratch),
        Outcome::Truncated {
            len: 10,
            unread_rows: (COUNTRIES - 11) as u32
        }
    );
}

#[test]
fn fixed_cold_rescans_complete_above_the_former_country_budget() {
    const COUNTRIES: usize = 6_001;
    let census = war_census(COUNTRIES, COUNTRIES - 1, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &census.bytes,
    };
    let mut scratch = OwnedSnapshotScratch::default();
    let mut rows = vec![Row::default(); COUNTRIES];
    assert_eq!(
        walk_owned(&memory, &memory, 0, GS, &mut rows, &mut scratch),
        Outcome::Rows { len: COUNTRIES - 1 }
    );
    assert!(scratch.rescanned_words > 50_020_000);
    assert_eq!(rows[..COUNTRIES - 1], census.rows[..]);
    scratch.grow(0).unwrap();
    assert_eq!(
        walk_owned(&memory, &memory, 0, GS, &mut rows, &mut scratch),
        Outcome::Rows { len: COUNTRIES - 1 }
    );
    assert_eq!(scratch.rescanned_words, 0);
}
