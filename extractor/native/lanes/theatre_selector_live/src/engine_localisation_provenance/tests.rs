use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

const IMAGE_BASE: u64 = 0x0000_0001_4000_0000;
const HEAP_BASE: u64 = 0x0000_0002_0000_0000;

#[test]
fn full_provenance_distinguishes_language_observation_failure_from_database_failure() {
    let image = Fixture::new("l_english", vec![]).build();
    let mismatch = read_engine_localisation_provenance_once(&image, IMAGE_BASE, "l_french")
        .unwrap_err();
    assert!(matches!(mismatch, EngineLocalisationReadError::ActiveLanguage(_)));
    let language_unreadable = ScriptedImage(|address, length| {
        if address == IMAGE_BASE + LOCALISATION_ROOT_RVA {
            Err("unreadable active language root".into())
        } else {
            image.read(address, length)
        }
    });
    assert!(matches!(read_engine_localisation_provenance_once(
        &language_unreadable, IMAGE_BASE, "l_english").unwrap_err(),
        EngineLocalisationReadError::ActiveLanguage(_)));
    let root = read_u64(&image, IMAGE_BASE + LOCALISATION_ROOT_RVA, "fixture root").unwrap();
    let database_unreadable = ScriptedImage(|address, length| {
        if address == root + ROOT_TABLE_VECTOR {
            Err("unreadable localisation database".into())
        } else {
            image.read(address, length)
        }
    });
    assert!(matches!(read_engine_localisation_provenance_once(
        &database_unreadable, IMAGE_BASE, "l_english").unwrap_err(),
        EngineLocalisationReadError::Database(_)));
}

#[derive(Default, Clone)]
struct FakeImage {
    regions: BTreeMap<u64, Vec<u8>>,
}

impl FakeImage {
    fn place(&mut self, address: u64, bytes: Vec<u8>) {
        self.regions.insert(address, bytes);
    }
}

impl TargetImageMemory for FakeImage {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        for (start, data) in self.regions.range(..=address).rev() {
            let end = start + data.len() as u64;
            if address >= end {
                continue;
            }
            let offset = (address - start) as usize;
            let stop = offset
                .checked_add(length)
                .ok_or("fake image read overflow")?;
            if stop > data.len() {
                return Err(format!(
                    "fake image read at 0x{address:x} length {length} leaves the mapped region"
                ));
            }
            return Ok(data[offset..stop].to_vec());
        }
        Err(format!("fake image read at 0x{address:x} is unmapped"))
    }
}

struct FlippingImage {
    first: FakeImage,
    second: FakeImage,
    root_reads: Cell<u32>,
}

impl TargetImageMemory for FlippingImage {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if address == IMAGE_BASE + LOCALISATION_ROOT_RVA {
            self.root_reads.set(self.root_reads.get() + 1);
        }
        if self.root_reads.get() <= 1 {
            self.first.read(address, length)
        } else {
            self.second.read(address, length)
        }
    }
}

struct ScriptedImage<F>(F)
where
    F: Fn(u64, usize) -> Result<Vec<u8>, String>;

impl<F> TargetImageMemory for ScriptedImage<F>
where
    F: Fn(u64, usize) -> Result<Vec<u8>, String>,
{
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (self.0)(address, length)
    }
}

#[derive(Clone)]
struct Record {
    key: &'static str,
    value: &'static str,
    source: Option<&'static str>,
}

fn record(key: &'static str, value: &'static str, source: &'static str) -> Record {
    Record {
        key,
        value,
        source: Some(source),
    }
}

struct Fixture {
    language: &'static str,
    records: Vec<Record>,
    decoy_language: Option<&'static str>,
    duplicate_language_table: bool,
    override_entry_count: Option<u32>,
    override_source_index: Option<i32>,
    override_key_offset: Option<i32>,
    root_language_override: Option<&'static str>,
    exact_page_arena: bool,
    override_table_count: Option<u32>,
    override_source_count: Option<u32>,
    null_table_slot: bool,
    corrupt_source_string: Option<CorruptString>,
    arena_capacity: Option<u32>,
    arena_length: Option<u32>,
    table_capacity: Option<u32>,
    entry_capacity: Option<u32>,
    source_capacity: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CorruptString {
    SizeExceedsCapacity,
    SizeBeyondMaximum,
}

impl Fixture {
    fn new(language: &'static str, records: Vec<Record>) -> Self {
        Self {
            language,
            records,
            decoy_language: None,
            duplicate_language_table: false,
            override_entry_count: None,
            override_source_index: None,
            override_key_offset: None,
            root_language_override: None,
            exact_page_arena: false,
            override_table_count: None,
            override_source_count: None,
            null_table_slot: false,
            corrupt_source_string: None,
            arena_capacity: None,
            arena_length: None,
            table_capacity: None,
            entry_capacity: None,
            source_capacity: None,
        }
    }

    fn build(&self) -> FakeImage {
        let mut image = FakeImage::default();
        let mut next = HEAP_BASE;
        let mut alloc = |image: &mut FakeImage, bytes: Vec<u8>| -> u64 {
            let address = (next + 0xfff) & !0xfffu64;
            let mut padded = bytes;
            let pad = (padded.len() + 0xfff) & !0xfffusize;
            padded.resize(pad.max(0x1000), 0);
            next = address + padded.len() as u64;
            image.place(address, padded);
            address
        };

        let mut records = self.records.clone();
        let mut arena = Vec::new();
        let mut offsets: Vec<(i32, i32)> = Vec::new();
        let push_text =
            |arena: &mut Vec<u8>, offsets: &mut Vec<(i32, i32)>, record: &Record| {
                let key_offset = arena.len() as i32;
                arena.extend_from_slice(record.key.as_bytes());
                arena.push(0);
                let value_offset = arena.len() as i32;
                arena.extend_from_slice(record.value.as_bytes());
                arena.push(0);
                offsets.push((key_offset, value_offset));
            };
        for record in &records {
            push_text(&mut arena, &mut offsets, record);
        }
        if self.exact_page_arena {
            const PAGE: usize = 0x1000;
            let key = "ZZ_PAGE_EDGE";
            let after_key = arena.len() + key.len() + 1;
            let target = after_key.div_ceil(PAGE) * PAGE;
            let value_len = target - after_key - 1;
            let filler = Record {
                key,
                value: Box::leak("x".repeat(value_len).into_boxed_str()),
                source: Some("edge.yml"),
            };
            push_text(&mut arena, &mut offsets, &filler);
            records.push(filler);
            assert_eq!(
                arena.len() % PAGE,
                0,
                "the arena must end exactly on a page boundary for this fixture to bite"
            );
        }
        let arena_len = arena.len() as u32;
        let arena_address = alloc(&mut image, arena);

        let mut source_names: Vec<&str> = Vec::new();
        for record in &records {
            if let Some(source) = record.source {
                if !source_names.contains(&source) {
                    source_names.push(source);
                }
            }
        }
        let mut source_blob = Vec::new();
        let mut source_heaps: Vec<(usize, &str)> = Vec::new();
        for (index, name) in source_names.iter().enumerate() {
            source_blob.extend_from_slice(&std_string_placeholder(name));
            if name.len() > STD_STRING_SSO_CAPACITY as usize {
                source_heaps.push((index, name));
            }
        }
        let source_address = alloc(&mut image, source_blob);
        if let Some(kind) = self.corrupt_source_string {
            let mut header = image
                .read(source_address, STD_STRING_STRIDE as usize)
                .expect("source slot is mapped");
            let (size, capacity) = match kind {
                CorruptString::SizeExceedsCapacity => (64u64, 15u64),
                CorruptString::SizeBeyondMaximum => (u64::from(u32::MAX), u64::from(u32::MAX)),
            };
            header[STD_STRING_SIZE_OFFSET..STD_STRING_SIZE_OFFSET + 8]
                .copy_from_slice(&size.to_le_bytes());
            header[STD_STRING_CAPACITY_OFFSET..STD_STRING_CAPACITY_OFFSET + 8]
                .copy_from_slice(&capacity.to_le_bytes());
            patch(&mut image, source_address, &header);
        }
        for (index, name) in source_heaps {
            let heap = alloc(&mut image, name.as_bytes().to_vec());
            let slot = source_address + index as u64 * STD_STRING_STRIDE;
            let mut header = image
                .read(slot, STD_STRING_STRIDE as usize)
                .expect("source slot is mapped");
            header[0..8].copy_from_slice(&heap.to_le_bytes());
            patch(&mut image, slot, &header);
        }

        let mut entries = Vec::new();
        for (index, record) in records.iter().enumerate() {
            let (key_offset, value_offset) = offsets[index];
            let key_offset = if index == 0 {
                self.override_key_offset.unwrap_or(key_offset)
            } else {
                key_offset
            };
            let source_index = match record.source {
                None => -1i32,
                Some(source) => source_names
                    .iter()
                    .position(|candidate| candidate == &source)
                    .expect("source is registered") as i32,
            };
            let source_index = if index == 0 {
                self.override_source_index.unwrap_or(source_index)
            } else {
                source_index
            };
            entries.extend_from_slice(&0xdead_beef_dead_beefu64.to_le_bytes());
            entries.extend_from_slice(&key_offset.to_le_bytes());
            entries.extend_from_slice(&value_offset.to_le_bytes());
            entries.extend_from_slice(&source_index.to_le_bytes());
            entries.extend_from_slice(&0i32.to_le_bytes());
        }
        let entry_address = alloc(&mut image, entries);

        let build_table = |image: &mut FakeImage,
                           alloc: &mut dyn FnMut(&mut FakeImage, Vec<u8>) -> u64,
                           language: &str,
                           entry_address: u64,
                           entry_count: u32,
                           entry_capacity: u32,
                           source_address: u64,
                           source_count: u32,
                           source_capacity: u32|
         -> u64 {
            let mut table = vec![0u8; 0x60];
            table[0x00..0x20].copy_from_slice(&std_string_placeholder(language));
            table[0x38..0x40].copy_from_slice(&entry_address.to_le_bytes());
            table[0x40..0x44].copy_from_slice(&entry_capacity.to_le_bytes());
            table[0x44..0x48].copy_from_slice(&entry_count.to_le_bytes());
            table[0x50..0x58].copy_from_slice(&source_address.to_le_bytes());
            table[0x58..0x5c].copy_from_slice(&source_capacity.to_le_bytes());
            table[0x5c..0x60].copy_from_slice(&source_count.to_le_bytes());
            alloc(image, table)
        };

        let entry_count = self.override_entry_count.unwrap_or(records.len() as u32);
        let mut tables: Vec<u64> = Vec::new();
        if let Some(decoy) = self.decoy_language {
            let table = build_table(
                &mut image,
                &mut alloc,
                decoy,
                entry_address,
                1,
                self.entry_capacity.unwrap_or(1),
                source_address,
                source_names.len() as u32,
                self.source_capacity.unwrap_or(source_names.len() as u32),
            );
            tables.push(table);
        }
        tables.push(build_table(
            &mut image,
            &mut alloc,
            self.language,
            entry_address,
            entry_count,
            self.entry_capacity.unwrap_or(entry_count),
            source_address,
            self.override_source_count
                .unwrap_or(source_names.len() as u32),
            self.source_capacity.unwrap_or(
                self.override_source_count
                    .unwrap_or(source_names.len() as u32),
            ),
        ));
        if self.duplicate_language_table {
            tables.push(build_table(
                &mut image,
                &mut alloc,
                self.language,
                entry_address,
                entry_count,
                self.entry_capacity.unwrap_or(entry_count),
                source_address,
                source_names.len() as u32,
                self.source_capacity.unwrap_or(source_names.len() as u32),
            ));
        }

        if self.null_table_slot {
            tables.insert(0, 0);
        }
        let mut table_array = Vec::new();
        for table in &tables {
            table_array.extend_from_slice(&table.to_le_bytes());
        }
        let table_array_address = alloc(&mut image, table_array);
        let declared_table_count = self.override_table_count.unwrap_or(tables.len() as u32);

        let root_language = self.root_language_override.unwrap_or(self.language);
        let language_string = alloc(&mut image, std_string_placeholder(root_language).to_vec());

        let mut root = vec![0u8; 0x30];
        root[0x00..0x08].copy_from_slice(&language_string.to_le_bytes());
        root[0x08..0x10].copy_from_slice(&table_array_address.to_le_bytes());
        root[0x10..0x14].copy_from_slice(
            &self
                .table_capacity
                .unwrap_or(declared_table_count)
                .to_le_bytes(),
        );
        root[0x14..0x18].copy_from_slice(&declared_table_count.to_le_bytes());
        root[0x20..0x28].copy_from_slice(&arena_address.to_le_bytes());
        root[0x28..0x2c].copy_from_slice(&self.arena_capacity.unwrap_or(arena_len).to_le_bytes());
        root[0x2c..0x30].copy_from_slice(&self.arena_length.unwrap_or(arena_len).to_le_bytes());
        let root_address = alloc(&mut image, root);

        let mut root_slot = vec![0u8; 0x1000];
        root_slot[0..8].copy_from_slice(&root_address.to_le_bytes());
        image.place(IMAGE_BASE + LOCALISATION_ROOT_RVA, root_slot);
        image
    }
}

fn std_string_placeholder(text: &str) -> [u8; 0x20] {
    let mut bytes = [0u8; 0x20];
    bytes[STD_STRING_SIZE_OFFSET..STD_STRING_SIZE_OFFSET + 8]
        .copy_from_slice(&(text.len() as u64).to_le_bytes());
    if text.len() <= STD_STRING_SSO_CAPACITY as usize {
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        bytes[STD_STRING_CAPACITY_OFFSET..STD_STRING_CAPACITY_OFFSET + 8]
            .copy_from_slice(&STD_STRING_SSO_CAPACITY.to_le_bytes());
    } else {
        bytes[STD_STRING_CAPACITY_OFFSET..STD_STRING_CAPACITY_OFFSET + 8]
            .copy_from_slice(&(text.len() as u64).to_le_bytes());
    }
    bytes
}

fn patch(image: &mut FakeImage, address: u64, bytes: &[u8]) {
    let key = *image
        .regions
        .range(..=address)
        .rev()
        .find(|(start, data)| address < *start + data.len() as u64)
        .expect("patched address is mapped")
        .0;
    let data = image.regions.get_mut(&key).expect("region exists");
    let offset = (address - key) as usize;
    data[offset..offset + bytes.len()].copy_from_slice(bytes);
}

fn base_records() -> Vec<Record> {
    vec![
        record(
            "THEATRE_SELECTOR_TITLE",
            "Theatres",
            "localisation/english/theatre_l_english.yml",
        ),
        record(
            "AIR_REGION_NAME",
            "Air Region",
            "localisation/english/air_l_english.yml",
        ),
        record("SHORT", "s", "a.yml"),
    ]
}

fn provenance_of(image: &FakeImage, language: &str) -> EngineLocalisationProvenance {
    read_engine_localisation_provenance(image, IMAGE_BASE, language)
        .unwrap_or_else(|error| panic!("provenance read failed: {error}"))
}

fn error_of(image: &FakeImage, language: &str) -> String {
    read_engine_localisation_provenance(image, IMAGE_BASE, language)
        .expect_err("expected a refusal")
}

#[test]
fn a_synthetic_engine_table_yields_its_keys_sources_and_language() {
    let image = Fixture::new("l_english", base_records()).build();
    let provenance = provenance_of(&image, "l_english");
    assert_eq!(provenance.language, "l_english");
    assert_eq!(provenance.key_count, 3);
    assert_eq!(
        provenance.sources,
        vec![
            "a.yml".to_string(),
            "localisation/english/air_l_english.yml".to_string(),
            "localisation/english/theatre_l_english.yml".to_string(),
        ],
        "long source names must come back from their heap buffers, short ones from the \
         small-string buffer"
    );
    assert_eq!(provenance.provenance_sha256.len(), 64);
}

#[test]
fn active_language_observation_does_not_require_a_launch_token_or_table_walk() {
    let image = Fixture::new("l_english", base_records()).build();
    let prefix_only = ScriptedImage(|address, length| {
        let root = u64_at(&image.read(IMAGE_BASE + LOCALISATION_ROOT_RVA, 8)?, 0)?;
        if address >= root + ROOT_TABLE_VECTOR && address < root + ROOT_TEXT_ARENA_VECTOR + 24 {
            return Err("the language-only observer must not read tables or text arena".into());
        }
        image.read(address, length)
    });
    assert_eq!(read_engine_active_language(&prefix_only, IMAGE_BASE).unwrap(), "l_english");
}

#[test]
fn active_language_observation_refuses_changed_or_unbounded_tokens() {
    let changed = FlippingImage {
        first: Fixture::new("l_english", base_records()).build(),
        second: Fixture::new("l_french", base_records()).build(),
        root_reads: Cell::new(0),
    };
    assert!(read_engine_active_language(&changed, IMAGE_BASE).unwrap_err().contains("changed"));
    let too_long = "x".repeat(65);
    let image = Fixture::new(Box::leak(too_long.into_boxed_str()), base_records()).build();
    assert!(read_engine_active_language(&image, IMAGE_BASE).unwrap_err().contains("unbounded"));
}

#[test]
fn the_digest_is_invariant_to_the_engines_physical_entry_order() {
    let forward = Fixture::new("l_english", base_records()).build();
    let mut reversed_records = base_records();
    reversed_records.reverse();
    let reversed = Fixture::new("l_english", reversed_records).build();
    assert_eq!(
        provenance_of(&forward, "l_english").provenance_sha256,
        provenance_of(&reversed, "l_english").provenance_sha256,
        "entry order in the engine's array is an allocation artefact and must not reach the digest"
    );
}

#[test]
fn a_changed_value_moves_the_digest() {
    let before = Fixture::new("l_english", base_records()).build();
    let mut records = base_records();
    records[0].value = "Theatres of War";
    let after = Fixture::new("l_english", records).build();
    assert_ne!(
        provenance_of(&before, "l_english").provenance_sha256,
        provenance_of(&after, "l_english").provenance_sha256,
    );
}

#[test]
fn a_changed_key_moves_the_digest() {
    let before = Fixture::new("l_english", base_records()).build();
    let mut records = base_records();
    records[0].key = "THEATRE_SELECTOR_HEADER";
    let after = Fixture::new("l_english", records).build();
    assert_ne!(
        provenance_of(&before, "l_english").provenance_sha256,
        provenance_of(&after, "l_english").provenance_sha256,
    );
}

#[test]
fn a_mod_overriding_a_base_key_from_its_own_file_moves_the_digest_even_with_identical_text() {
    let base = Fixture::new("l_english", base_records()).build();
    let mut overridden = base_records();
    overridden[0].source = Some("mod/my_mod/localisation/theatre_l_english.yml");
    let modded = Fixture::new("l_english", overridden).build();
    let before = provenance_of(&base, "l_english");
    let after = provenance_of(&modded, "l_english");
    assert_eq!(
        before.key_count, after.key_count,
        "the override replaces a key rather than adding one"
    );
    assert_ne!(
        before.provenance_sha256, after.provenance_sha256,
        "the winning source file is part of the provenance and must move the digest"
    );
}

#[test]
fn a_key_with_no_recorded_source_is_distinct_from_one_sourced_from_an_empty_name() {
    let mut unattributed = base_records();
    unattributed[0].source = None;
    let unattributed = Fixture::new("l_english", unattributed).build();
    let mut empty_named = base_records();
    empty_named[0].source = Some("");
    let empty_named = Fixture::new("l_english", empty_named).build();
    assert_ne!(
        provenance_of(&unattributed, "l_english").provenance_sha256,
        provenance_of(&empty_named, "l_english").provenance_sha256,
        "the engine's -1 sentinel is not the same fact as a source named \"\""
    );
}

#[test]
fn framing_prevents_a_key_value_boundary_shift_from_colliding() {
    let left = Fixture::new("l_english", vec![record("AB", "C", "f.yml")]).build();
    let right = Fixture::new("l_english", vec![record("A", "BC", "f.yml")]).build();
    assert_ne!(
        provenance_of(&left, "l_english").provenance_sha256,
        provenance_of(&right, "l_english").provenance_sha256,
    );
}

#[test]
fn a_duplicate_key_is_refused_as_a_mid_load_table() {
    let mut records = base_records();
    records[1].key = records[0].key;
    let image = Fixture::new("l_english", records).build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("duplicate key"),
        "duplicate keys must be named as such, got: {error}"
    );
}

#[test]
fn an_uninitialised_localisation_system_is_refused_by_name() {
    let mut image = Fixture::new("l_english", base_records()).build();
    image.place(IMAGE_BASE + LOCALISATION_ROOT_RVA, vec![0u8; 0x1000]);
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("not initialised"),
        "a null root is the engine's own not-yet-loaded signal and must be reported as that, \
         not as a stray unmapped read; got: {error}"
    );
}

#[test]
fn an_engine_language_differing_from_the_launch_binding_is_refused() {
    let image = Fixture::new("l_french", base_records()).build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("differs from the authenticated language"),
        "got: {error}"
    );
}

#[test]
fn the_language_table_is_selected_by_name_not_by_position() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.decoy_language = Some("l_german");
    let image = fixture.build();
    let provenance = provenance_of(&image, "l_english");
    assert_eq!(
        provenance.key_count, 3,
        "the decoy table sits at index 0 and declares one entry; selecting by position would \
         yield 1 key"
    );
}

#[test]
fn two_tables_claiming_one_language_are_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.duplicate_language_table = true;
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("more than one table"), "got: {error}");
}

#[test]
fn an_active_language_with_no_table_of_its_own_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.root_language_override = Some("l_spanish");
    let image = fixture.build();
    let error = error_of(&image, "l_spanish");
    assert!(
        error.contains("no table for the active language"),
        "got: {error}"
    );
}

#[test]
fn a_source_index_past_the_source_array_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_source_index = Some(9_999);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("source index is out of range"),
        "an out-of-range index must be refused rather than read out of bounds; got: {error}"
    );
}

#[test]
fn an_unbounded_entry_count_is_refused_before_any_allocation() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_entry_count = Some(MAX_ENTRIES + 1);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("entry count is unbounded"), "got: {error}");
}

#[test]
fn a_negative_arena_offset_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_key_offset = Some(-4);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("offset is negative"), "got: {error}");
}

#[test]
fn an_arena_offset_past_the_span_bound_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_key_offset = Some(i32::MAX);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("offset is out of range"), "got: {error}");
}

#[test]
fn an_empty_language_binding_is_refused() {
    let image = Fixture::new("l_english", base_records()).build();
    let error = error_of(&image, "");
    assert!(
        error.contains("requires an authenticated language"),
        "got: {error}"
    );
}

#[test]
fn a_table_that_changes_between_the_two_reads_is_refused() {
    let first = Fixture::new("l_english", base_records()).build();
    let mut moved = base_records();
    moved[0].value = "Theatres of War";
    let second = Fixture::new("l_english", moved).build();
    let flipping = FlippingImage {
        first,
        second,
        root_reads: Cell::new(0),
    };
    let error = read_engine_localisation_provenance(&flipping, IMAGE_BASE, "l_english")
        .expect_err("a table that moved under us must be refused");
    assert!(
        error.contains("changed between two consecutive reads"),
        "got: {error}"
    );
}

#[test]
fn a_value_is_read_past_the_first_chunk_not_truncated_at_it() {
    let prefix = "a".repeat(600);
    let one: &'static str = Box::leak(format!("{prefix}TAIL_ONE").into_boxed_str());
    let two: &'static str = Box::leak(format!("{prefix}TAIL_TWO").into_boxed_str());
    let left = Fixture::new("l_english", vec![record("K", one, "f.yml")]).build();
    let right = Fixture::new("l_english", vec![record("K", two, "f.yml")]).build();
    assert_ne!(
        provenance_of(&left, "l_english").provenance_sha256,
        provenance_of(&right, "l_english").provenance_sha256,
        "values differing only past the first chunk must still be distinguished"
    );
}

#[test]
fn a_value_ending_flush_with_the_last_mapped_page_is_still_read() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.exact_page_arena = true;
    let image = fixture.build();
    let provenance = provenance_of(&image, "l_english");
    assert_eq!(
        provenance.key_count, 4,
        "the page-edge record must be read, not refused"
    );
    assert!(provenance.sources.contains(&"edge.yml".to_string()));
}

#[test]
fn the_active_language_is_part_of_the_digest() {
    let english = Fixture::new("l_english", base_records()).build();
    let french = Fixture::new("l_french", base_records()).build();
    assert_ne!(
        provenance_of(&english, "l_english").provenance_sha256,
        provenance_of(&french, "l_french").provenance_sha256,
        "identical content under a different language must not share a digest"
    );
}

#[test]
fn source_tags_stop_a_source_being_reassociated_onto_a_neighbouring_key() {
    let left = Fixture::new(
        "l_english",
        vec![
            Record {
                key: "A",
                value: "V1",
                source: Some("B"),
            },
            Record {
                key: "C",
                value: "V2",
                source: None,
            },
        ],
    )
    .build();
    let right = Fixture::new(
        "l_english",
        vec![
            Record {
                key: "A",
                value: "V1",
                source: None,
            },
            Record {
                key: "B",
                value: "C",
                source: Some("V2"),
            },
        ],
    )
    .build();
    assert_ne!(
        provenance_of(&left, "l_english").provenance_sha256,
        provenance_of(&right, "l_english").provenance_sha256,
    );
}

#[test]
fn a_null_pointer_inside_the_counted_table_array_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.null_table_slot = true;
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("table slot is null"),
        "a null in a counted array is corruption, not a sentinel; got: {error}"
    );
}

#[test]
fn a_table_count_larger_than_the_array_is_refused_not_read_past() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_table_count = Some(MAX_TABLES + 1);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("table count is unbounded"), "got: {error}");
}

#[test]
fn an_unbounded_source_count_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.override_source_count = Some(MAX_SOURCES + 1);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("source count is unbounded"), "got: {error}");
}

#[test]
fn a_std_string_whose_size_exceeds_its_capacity_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.corrupt_source_string = Some(CorruptString::SizeExceedsCapacity);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("size exceeds capacity"),
        "no valid MSVC string has size > capacity; got: {error}"
    );
}

#[test]
fn a_std_string_claiming_an_enormous_size_is_refused_before_the_read() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.corrupt_source_string = Some(CorruptString::SizeBeyondMaximum);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("string is unbounded"),
        "a 4 GB length must be refused rather than allocated; got: {error}"
    );
}

#[test]
fn an_empty_key_is_refused() {
    let image = Fixture::new("l_english", vec![record("", "value", "f.yml")]).build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("empty key"), "got: {error}");
}

#[test]
fn an_arena_string_that_never_terminates_within_the_bound_is_refused() {
    let huge: &'static str = Box::leak("x".repeat(MAX_TEXT_BYTES + 5_000).into_boxed_str());
    let image = Fixture::new("l_english", vec![record("K", huge, "f.yml")]).build();
    let error = error_of(&image, "l_english");
    assert!(error.contains("is unterminated"), "got: {error}");
}

#[test]
fn the_text_bound_holds_even_when_the_terminator_lands_inside_a_later_chunk() {
    const REGION: u64 = 0x0000_0003_0000_0000;
    const NUL_AT: usize = 65_720;
    let mut bytes = vec![b'x'; 17 * 4096];
    bytes[1 + NUL_AT] = 0;
    let mut image = FakeImage::default();
    image.place(REGION, bytes);

    let result = read_c_string(&image, REGION + 1, usize::MAX, "value");
    match result {
        Err(error) => assert!(error.contains("is unterminated"), "got: {error}"),
        Ok(text) => panic!(
            "a string terminating at {NUL_AT} bytes must not be returned under a \
             {MAX_TEXT_BYTES}-byte bound, but {} bytes came back",
            text.len()
        ),
    }
}

#[test]
fn an_image_base_that_overflows_the_root_slot_address_is_refused() {
    let image = Fixture::new("l_english", base_records()).build();
    let error = read_engine_localisation_provenance(&image, u64::MAX, "l_english")
        .expect_err("an overflowing base must be refused");
    assert!(error.contains("overflow"), "got: {error}");
}

#[test]
fn a_table_vector_whose_size_exceeds_its_capacity_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.table_capacity = Some(0);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("language table vector size exceeds its capacity"),
        "got: {error}"
    );
}

#[test]
fn an_entry_vector_whose_size_exceeds_its_capacity_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.entry_capacity = Some(0);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("entry vector size exceeds its capacity"),
        "got: {error}"
    );
}

#[test]
fn a_source_vector_whose_size_exceeds_its_capacity_is_refused() {
    let mut fixture = Fixture::new("l_english", base_records());
    fixture.source_capacity = Some(0);
    let image = fixture.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("source vector size exceeds its capacity"),
        "got: {error}"
    );
}

fn vector_header(data: u64, capacity: i32, size: i32) -> (FakeImage, u64) {
    const AT: u64 = 0x0000_0004_0000_0000;
    let mut bytes = vec![0u8; 0x1000];
    bytes[0..8].copy_from_slice(&data.to_le_bytes());
    bytes[8..12].copy_from_slice(&capacity.to_le_bytes());
    bytes[12..16].copy_from_slice(&size.to_le_bytes());
    let mut image = FakeImage::default();
    image.place(AT, bytes);
    (image, AT)
}

#[test]
fn a_vector_with_capacity_but_a_null_data_pointer_is_refused() {
    let (image, at) = vector_header(0, 1, 0);
    let error = read_pdx_vector(&image, at, "source").expect_err("must refuse");
    assert!(
        error.contains("has capacity without storage"),
        "got: {error}"
    );
}

#[test]
fn an_empty_never_allocated_vector_is_still_accepted() {
    let (image, at) = vector_header(0, 0, 0);
    let (data, size) = read_pdx_vector(&image, at, "source").expect("must be accepted");
    assert_eq!((data, size), (0, 0));
}

#[test]
fn a_vector_declaring_a_negative_capacity_is_refused() {
    let (image, at) = vector_header(0x1000, -1, 1);
    let error = read_pdx_vector(&image, at, "entry").expect_err("must refuse");
    assert!(error.contains("capacity is negative"), "got: {error}");
}

#[test]
fn a_vector_declaring_a_negative_size_is_refused() {
    let (image, at) = vector_header(0x1000, 8, -1);
    let error = read_pdx_vector(&image, at, "entry").expect_err("must refuse");
    assert!(error.contains("count is negative"), "got: {error}");
}

#[test]
fn a_field_offset_that_overflows_the_structure_address_is_refused() {
    let error = field(u64::MAX - 3, ROOT_TABLE_VECTOR).expect_err("must refuse");
    assert!(error.contains("field address overflow"), "got: {error}");
}

#[test]
fn a_table_slot_index_that_overflows_the_array_address_is_refused() {
    let memory = ScriptedImage(|_address, length| match length {
        8 => Ok(0x1000u64.to_le_bytes().to_vec()),
        0x20 => Ok(std_string_placeholder("l_german").to_vec()),
        _ => Err("unexpected read".to_string()),
    });
    let error = select_language_table(&memory, u64::MAX - 4, 2, "l_english").expect_err("refuse");
    assert!(
        error.contains("table slot address overflow"),
        "got: {error}"
    );
}

#[test]
fn a_source_slot_index_that_overflows_the_array_address_is_refused() {
    let memory = ScriptedImage(|_address, _length| Ok(std_string_placeholder("a.yml").to_vec()));
    let error = read_sources(&memory, u64::MAX - 4, 2).expect_err("must refuse");
    assert!(
        error.contains("source slot address overflow"),
        "got: {error}"
    );
}

#[test]
fn an_arena_offset_that_overflows_the_arena_address_is_refused() {
    let memory = ScriptedImage(|_address, _length| Err("must not be read".to_string()));
    let error = read_arena_text(&memory, u64::MAX - 3, 4096, 100, "key").expect_err("must refuse");
    assert!(error.contains("address overflow"), "got: {error}");
}

#[test]
fn a_c_string_cursor_that_overflows_while_advancing_is_refused() {
    let memory = ScriptedImage(|_address, length| Ok(vec![b'x'; length]));
    let error =
        read_c_string(&memory, u64::MAX - 50, usize::MAX, "value").expect_err("must refuse");
    assert!(error.contains("address overflow"), "got: {error}");
}

#[test]
fn arena_reads_are_bounded_by_the_engines_declared_length_not_by_what_is_mapped() {
    let records = vec![record("KEY", "value", "f.yml")];
    let full = Fixture::new("l_english", records.clone()).build();
    provenance_of(&full, "l_english");

    let mut shortened = Fixture::new("l_english", records);
    shortened.arena_length = Some(2);
    shortened.arena_capacity = Some(2);
    let image = shortened.build();
    let error = error_of(&image, "l_english");
    assert!(
        error.contains("is unterminated"),
        "the declared arena length alone must decide this; got: {error}"
    );
}

struct CachingImage {
    inner: FakeImage,
    fresh_reads: Cell<u32>,
    cache: RefCell<BTreeMap<(u64, usize), Vec<u8>>>,
}

impl CachingImage {
    fn over(inner: FakeImage) -> Self {
        Self {
            inner,
            fresh_reads: Cell::new(0),
            cache: RefCell::new(BTreeMap::new()),
        }
    }
}

impl TargetImageMemory for CachingImage {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if let Some(hit) = self.cache.borrow().get(&(address, length)) {
            return Ok(hit.clone());
        }
        self.fresh_reads.set(self.fresh_reads.get() + 1);
        let value = self.inner.read(address, length)?;
        self.cache
            .borrow_mut()
            .insert((address, length), value.clone());
        Ok(value)
    }
}

#[test]
fn a_second_pass_through_a_caching_reader_touches_the_target_zero_times() {
    let image = CachingImage::over(Fixture::new("l_english", base_records()).build());

    read_engine_localisation_provenance_once(&image, IMAGE_BASE, "l_english").expect("first pass");
    let after_first = image.fresh_reads.get();
    assert!(
        after_first > 0,
        "the first pass must actually read the target"
    );

    read_engine_localisation_provenance_once(&image, IMAGE_BASE, "l_english").expect("second pass");
    assert_eq!(
        image.fresh_reads.get(),
        after_first,
        "a second pass through the same caching reader performed {} fresh reads; it must be 0          for this hazard to be real, and it is why the seam rebuilds the reader per pass",
        image.fresh_reads.get() - after_first
    );
}

#[test]
fn the_root_rva_is_the_documented_absolute_address_minus_the_image_base() {
    assert_eq!(LOCALISATION_ROOT_VA_A729, 0x1_435a_10d8);
    assert_eq!(A729_PREFERRED_IMAGE_BASE, 0x1_4000_0000);
    assert_eq!(LOCALISATION_ROOT_RVA, 0x35a_10d8);
    assert_eq!(
        A729_PREFERRED_IMAGE_BASE + LOCALISATION_ROOT_RVA,
        LOCALISATION_ROOT_VA_A729
    );
}

#[test]
#[ignore]
fn measure_digest_cost_over_a_realistic_table() {
    use std::time::Instant;

    const KEYS: usize = 129_097;
    const SOURCES: usize = 207;
    let mut records: Vec<Record> = Vec::with_capacity(KEYS);
    for index in 0..KEYS {
        let key: &'static str = Box::leak(format!("KEY_NAME_NUMBER_{index:07}").into_boxed_str());
        let value: &'static str = Box::leak(
            format!("A representative localisation value for entry {index}, with some length.")
                .into_boxed_str(),
        );
        let source: &'static str = Box::leak(
            format!(
                "localisation/english/file_{:04}_l_english.yml",
                index % SOURCES
            )
            .into_boxed_str(),
        );
        records.push(Record {
            key,
            value,
            source: Some(source),
        });
    }
    let built = Instant::now();
    let image = Fixture::new("l_english", records).build();
    let build_ms = built.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let provenance = provenance_of(&image, "l_english");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(provenance.key_count as usize, KEYS);
    assert_eq!(provenance.sources.len(), SOURCES);
    println!(
        "MEASURED: {KEYS} keys, {SOURCES} sources -> parse+sort+digest {elapsed_ms:.1} ms \
         (two full passes; fixture build {build_ms:.1} ms)"
    );
}
