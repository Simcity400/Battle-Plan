use super::*;
use crate::memory::SliceMemory;
use std::cell::Cell;

const COUNTRY_POINTER: usize = 0x1000;
const STATUS_POINTER: usize = 0x3000;
const TECHNOLOGY_A: usize = 0x4000;
const TECHNOLOGY_B: usize = 0x5000;
const TEMPLATE_A: usize = 0x6000;
const TEMPLATE_B: usize = 0x7000;
const SLOT_A: usize = 0x8000;
const SLOT_B: usize = 0x8100;
const ALL: usize = 0x9000;
const COMPLETED: usize = 0x9100;
const SLOTS: usize = 0x9200;
const MODULE: usize = 0x140000000;

fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
    bytes[at - COUNTRY_POINTER..at - COUNTRY_POINTER + N].copy_from_slice(&value);
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, (data as u64).to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xa000];
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::TECHNOLOGY_STATUS,
        (STATUS_POINTER as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_POINTER,
        ((MODULE + VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS_POINTER + COUNTRY,
        (COUNTRY_POINTER as u64).to_le_bytes(),
    );
    array(&mut bytes, STATUS_POINTER + TECHNOLOGIES_DATA, ALL, 3);
    array(&mut bytes, STATUS_POINTER + RESEARCHED_DATA, COMPLETED, 1);
    array(&mut bytes, STATUS_POINTER + RESEARCH_SLOTS_DATA, SLOTS, 2);
    put(&mut bytes, ALL, (TECHNOLOGY_B as u64).to_le_bytes());
    put(&mut bytes, ALL + 16, (TECHNOLOGY_A as u64).to_le_bytes());
    put(&mut bytes, COMPLETED, (TECHNOLOGY_B as u64).to_le_bytes());
    put(&mut bytes, SLOTS, (SLOT_B as u64).to_le_bytes());
    put(&mut bytes, SLOTS + 8, (SLOT_A as u64).to_le_bytes());
    for (pointer, template, handle, classifier, level, progress) in [
        (TECHNOLOGY_A, TEMPLATE_A, 3_u32, 4_u32, -2_i32, i64::MIN + 9),
        (TECHNOLOGY_B, TEMPLATE_B, 17, 0x100, 9, i64::MAX - 8),
    ] {
        put(
            &mut bytes,
            pointer,
            ((MODULE + ctechnology::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::DEFINITION_INDEX,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::STATUS,
            (STATUS_POINTER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::TEMPLATE,
            (template as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::CLASSIFIER,
            classifier.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::LEVEL,
            level.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::LIVE_PROGRESS,
            progress.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::AHEAD_REDUCTION_YEARS,
            (-8_123_456_789_i64).to_le_bytes(),
        );
        put(&mut bytes, template + ctechnology_template::ID, *b"tech");
        put(
            &mut bytes,
            template + ctechnology_template::ID + 16,
            4_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::ID + 24,
            15_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::DATABASE_INDEX,
            (if template == TEMPLATE_A { 3_i32 } else { 1_i32 }).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::TECHNOLOGY_HANDLE,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::YEAR,
            1940_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::MAX_LEVEL,
            12_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::RESEARCH_COST,
            7_654_321_234_i64.to_le_bytes(),
        );
    }
    for (pointer, technology, handle, index) in [
        (SLOT_A, TECHNOLOGY_A, 3_u32, 0_u32),
        (SLOT_B, 0, cresearch_slot::IDLE_HANDLE, 1),
    ] {
        put(
            &mut bytes,
            pointer,
            ((MODULE + cresearch_slot::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::STATUS,
            (STATUS_POINTER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::TECHNOLOGY_HANDLE,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::TECHNOLOGY,
            (technology as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::INDEX,
            index.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::RESEARCH_SPEED,
            5_123_456_789_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::BANKED_DAYS,
            (-5_234_567_890_i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cresearch_slot::APPLIED_DAYS,
            6_345_678_901_i64.to_le_bytes(),
        );
    }
    bytes
}

#[test]
fn full_census_preserves_native_widths_holes_and_independent_inventory() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![usize::MAX; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(source_pointers, [TECHNOLOGY_B, TECHNOLOGY_A, usize::MAX]);
    assert_eq!(
        (rows[0].technology_handle, rows[1].technology_handle),
        (17, 3)
    );
    rows[..2].sort_unstable_by_key(|row| row.technology_handle);
    assert_eq!(
        (
            rows[0].technology_handle,
            rows[0].classifier,
            rows[0].researched
        ),
        (3, 4, 0)
    );
    assert_eq!(
        (
            rows[1].technology_handle,
            rows[1].classifier,
            rows[1].researched
        ),
        (17, 0x100, 1)
    );
    assert_eq!(rows[0].progress_raw, i64::MIN + 9);
    assert_eq!(rows[1].progress_raw, i64::MAX - 8);
    assert_eq!(rows[0].level, -2);
    assert_eq!(rows[0].ahead_reduction_years_raw, -8_123_456_789);
    assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(definitions.required_bytes, 8);
    assert_eq!(definitions.entry(3).unwrap().row.database_index, 3);
    assert_eq!(
        definitions.entry(1).unwrap().row.research_cost_raw,
        7_654_321_234
    );
    let mut slots = vec![cresearch_slot::Row::default(); 2];
    assert_eq!(
        walk_slots(&memory, MODULE, COUNTRY_POINTER, 7, &mut slots),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        (
            slots[0].index,
            slots[0].technology_present,
            slots[1].technology_present
        ),
        (0, 1, 0)
    );
    assert_eq!(slots[1].technology_handle, cresearch_slot::IDLE_HANDLE);
    assert_eq!((slots[0].database_index, slots[1].database_index), (3, 0));
    assert_eq!(slots[0].research_speed_raw, 5_123_456_789);
    assert_eq!(slots[0].banked_days_raw, -5_234_567_890);
    assert_eq!(slots[1].applied_days_raw, 6_345_678_901);
}

#[test]
fn first_roster_grows_rows_and_names_without_repeating_native_reads() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let mut rows = vec![ctechnology::Row::default(); 1];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 3];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut vec![0; 1]),
            0,
            &mut definitions,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].researched, 1);
    assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });
    assert_eq!((definitions.required_bytes, definitions.bytes_len), (8, 8));
    let mut slots = vec![cresearch_slot::Row::default(); 1];
    assert_eq!(
        walk_slots(&memory, MODULE, COUNTRY_POINTER, 7, &mut slots),
        Outcome::Rows { len: 2 }
    );
}

#[test]
fn dictionary_resets_each_callback_and_refuses_conflicting_templates() {
    let mut bytes = fixture();
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![0; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    {
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        let memory = SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        };
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        );
        assert_eq!(definitions.instance_pointer_row(TECHNOLOGY_B), Some(0));
        assert_eq!(definitions.instance_pointer_row(TECHNOLOGY_A), Some(1));
        definitions.begin_roster();
        assert_eq!(definitions.instance_pointer_row(TECHNOLOGY_B), None);
        assert_eq!(definitions.instance_pointer_row(TECHNOLOGY_A), None);
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        );
        assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });
        assert_eq!(definitions.required_bytes, 8);
        definitions.admit(&memory, TEMPLATE_B, 3, 3);
        assert!(matches!(definitions.outcome(), Outcome::Unavailable { .. }));
    }
    put(
        &mut bytes,
        TEMPLATE_A + ctechnology_template::YEAR,
        2000_i32.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    walk_technologies(
        &memory,
        MODULE,
        COUNTRY_POINTER,
        7,
        (&mut rows, &mut source_pointers),
        0,
        &mut definitions,
    );
    assert_eq!(definitions.entry(3).unwrap().row.year, 2000);
    assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });
}

#[test]
fn dictionary_growth_uses_full_sparse_handles_and_preserves_signed_inventory_order() {
    for dictionary_capacity in [0, 1, 4] {
        let mut bytes = fixture();
        let memory = SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        };
        let mut rows = vec![ctechnology::Row::default(); 3];
        let mut source_pointers = vec![0; 3];
        let mut entries = vec![ctechnology_template::Entry::default(); dictionary_capacity];
        let mut names = vec![0; 64];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Rows { len: 2 }
        );
        assert!(definitions.entries.len() <= 32);
        assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });

        let high_handle = 0x8000_0001_u32;
        put(
            &mut bytes,
            TECHNOLOGY_B + ctechnology::DEFINITION_INDEX,
            high_handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE_B + ctechnology_template::TECHNOLOGY_HANDLE,
            high_handle.to_le_bytes(),
        );
        array(&mut bytes, STATUS_POINTER + RESEARCHED_DATA, COMPLETED, 2);
        put(
            &mut bytes,
            COMPLETED + 8,
            (TECHNOLOGY_A as u64).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        };
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (rows[0].technology_handle, rows[1].technology_handle),
            (high_handle, 3)
        );
        assert_eq!((rows[0].researched, rows[1].researched), (1, 1));
        assert!(definitions.entries.len() <= 32);
        assert_eq!(definitions.outcome(), Outcome::Rows { len: 2 });

        for order in [[TECHNOLOGY_A, TECHNOLOGY_B], [TECHNOLOGY_B, TECHNOLOGY_B]] {
            put(&mut bytes, COMPLETED, (order[0] as u64).to_le_bytes());
            put(&mut bytes, COMPLETED + 8, (order[1] as u64).to_le_bytes());
            let memory = SliceMemory {
                base: COUNTRY_POINTER,
                bytes: &bytes,
            };
            let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
            assert_eq!(
                walk_technologies(
                    &memory,
                    MODULE,
                    COUNTRY_POINTER,
                    7,
                    (&mut rows, &mut source_pointers),
                    0,
                    &mut definitions,
                ),
                Outcome::Unavailable {
                    reason: "researched technology roster is not strictly ordered"
                }
            );
        }
    }
}

#[test]
fn corrupt_identity_and_duplicate_keys_are_unavailable_not_empty() {
    for (address, value) in [
        (STATUS_POINTER + COUNTRY, 0_u64),
        (ALL + 16, TECHNOLOGY_B as u64),
        (TECHNOLOGY_A + ctechnology::STATUS, 0),
        (COMPLETED, TECHNOLOGY_A as u64 + 1),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, value.to_le_bytes());
        let memory = SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        };
        let mut rows = vec![ctechnology::Row::default(); 3];
        let mut entries = vec![ctechnology_template::Entry::default(); 32];
        let mut names = vec![0; 64];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert!(matches!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut vec![0; 3]),
                0,
                &mut definitions,
            ),
            Outcome::Unavailable { .. }
        ));
    }
}

const BULK_COUNT: usize = 128;
const BULK_COMPLETED_COUNT: usize = 97;
const BULK_ALL: usize = 0xa000;
const BULK_COMPLETED: usize = 0xb000;
const BULK_INSTANCES: usize = 0x10000;
const BULK_TEMPLATES: usize = 0x30000;

fn bulk_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    bytes.resize(0x50000, 0);
    array(
        &mut bytes,
        STATUS_POINTER + TECHNOLOGIES_DATA,
        BULK_ALL,
        BULK_COUNT as u32,
    );
    array(
        &mut bytes,
        STATUS_POINTER + RESEARCHED_DATA,
        BULK_COMPLETED,
        BULK_COMPLETED_COUNT as u32,
    );
    for index in 0..BULK_COUNT {
        let pointer = BULK_INSTANCES + index * 0x200;
        let template = BULK_TEMPLATES + index * 0x400;
        let handle = (index + 1) as u32;
        bytes.copy_within(
            TECHNOLOGY_A - COUNTRY_POINTER..TECHNOLOGY_A - COUNTRY_POINTER + 0x200,
            pointer - COUNTRY_POINTER,
        );
        bytes.copy_within(
            TEMPLATE_A - COUNTRY_POINTER..TEMPLATE_A - COUNTRY_POINTER + 0x400,
            template - COUNTRY_POINTER,
        );
        put(
            &mut bytes,
            BULK_ALL + index * 8,
            (pointer as u64).to_le_bytes(),
        );
        if index < BULK_COMPLETED_COUNT {
            put(
                &mut bytes,
                BULK_COMPLETED + index * 8,
                (pointer as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            pointer + ctechnology::DEFINITION_INDEX,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::TEMPLATE,
            (template as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + ctechnology::LIVE_PROGRESS,
            (i64::MAX - index as i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::DATABASE_INDEX,
            (handle as i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + ctechnology_template::TECHNOLOGY_HANDLE,
            handle.to_le_bytes(),
        );
    }
    bytes
}

struct ObservedMemory<'a> {
    inner: SliceMemory<'a>,
    instance_batches: Cell<usize>,
    instance_spans: Cell<usize>,
    instance_bytes: Cell<usize>,
    identity_batches: Cell<usize>,
    identity_spans: Cell<usize>,
    identity_bytes: Cell<usize>,
    definition_batches: Cell<usize>,
    guards: Cell<usize>,
    spans: Cell<usize>,
    bytes: Cell<usize>,
    fail: Option<usize>,
}

impl<'a> ObservedMemory<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            inner: SliceMemory {
                base: COUNTRY_POINTER,
                bytes,
            },
            instance_batches: Cell::new(0),
            instance_spans: Cell::new(0),
            instance_bytes: Cell::new(0),
            identity_batches: Cell::new(0),
            identity_spans: Cell::new(0),
            identity_bytes: Cell::new(0),
            definition_batches: Cell::new(0),
            guards: Cell::new(0),
            spans: Cell::new(0),
            bytes: Cell::new(0),
            fail: None,
        }
    }
}

struct RecordSource<'a, 'b>(&'a ObservedMemory<'b>);

impl Memory for RecordSource<'_, '_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.span(address, destination)
    }
}

impl Memory for ObservedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.span(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if (BULK_TEMPLATES..BULK_TEMPLATES + BULK_COUNT * 0x400).contains(&addresses[0]) {
            self.definition_batches
                .set(self.definition_batches.get() + 1);
        }
        if destinations
            .iter()
            .any(|destination| !destination.is_empty())
        {
            self.guards.set(self.guards.get() + 1);
        }
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty() && !self.span(address, destination) {
                return completed;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &crate::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let (batches, spans, bytes) = if layout.stride() == ctechnology::IDENTITY_BYTES {
            assert_eq!(layout.field_count(), ctechnology::IDENTITY_FIELDS.len());
            (
                &self.identity_batches,
                &self.identity_spans,
                &self.identity_bytes,
            )
        } else {
            assert_eq!(layout.stride(), ctechnology::INSTANCE_BYTES);
            assert_eq!(layout.field_count(), ctechnology::FIELDS.len());
            (
                &self.instance_batches,
                &self.instance_spans,
                &self.instance_bytes,
            )
        };
        batches.set(batches.get() + 1);
        self.guards.set(self.guards.get() + 1);
        spans.set(spans.get() + sources.len() * layout.field_count());
        bytes.set(bytes.get() + layout.completed_bytes(sources.len() * layout.field_count()));
        RecordSource(self).copy_records(sources, layout, destination)
    }
}

impl ObservedMemory<'_> {
    fn span(&self, address: usize, destination: &mut [u8]) -> bool {
        self.spans.set(self.spans.get() + 1);
        self.bytes.set(self.bytes.get() + destination.len());
        self.source(address, destination)
    }

    fn source(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .fail
            .is_some_and(|fail| address <= fail && fail < address.saturating_add(destination.len()))
        {
            destination.fill(0xfe);
            return false;
        }
        self.inner.copy(address, destination)
    }
}

#[test]
fn whole_rosters_batch_reads_and_reuse_definitions_only_within_one_pass() {
    let mut bytes = bulk_fixture();
    let mut rows = vec![ctechnology::Row::default(); BULK_COUNT];
    let mut source_pointers = vec![0; BULK_COUNT];
    let mut entries = vec![ctechnology_template::Entry::default(); BULK_COUNT + 1];
    let mut names = vec![0; BULK_COUNT * 4];
    {
        let memory = ObservedMemory::new(&bytes);
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        for pass in 1..=2 {
            assert_eq!(
                walk_technologies(
                    &memory,
                    MODULE,
                    COUNTRY_POINTER,
                    7,
                    (&mut rows, &mut source_pointers),
                    0,
                    &mut definitions,
                ),
                Outcome::Rows { len: BULK_COUNT }
            );
            let batches = BULK_COUNT.div_ceil(ctechnology::BATCH);
            assert_eq!(memory.instance_batches.get(), pass * batches);
            assert_eq!(
                memory.instance_spans.get(),
                pass * BULK_COUNT * ctechnology::FIELDS.len()
            );
            assert_eq!(
                memory.instance_bytes.get(),
                pass * BULK_COUNT * ctechnology::INSTANCE_BYTES
            );
            assert_eq!(memory.identity_batches.get(), 0);
            assert_eq!(memory.identity_spans.get(), 0);
            assert_eq!(memory.identity_bytes.get(), 0);
            assert_eq!(memory.definition_batches.get(), BULK_COUNT);
            let pages = BULK_COUNT.div_ceil(ctechnology::BATCH)
                + BULK_COMPLETED_COUNT.div_ceil(ctechnology::BATCH);
            assert_eq!(
                memory.guards.get(),
                pass * (4 + pages + batches) + 3 * BULK_COUNT
            );
            assert_eq!(
                memory.spans.get(),
                pass * (5 + pages + ctechnology::FIELDS.len() * BULK_COUNT) + 5 * BULK_COUNT
            );
            assert_eq!(
                memory.bytes.get(),
                pass * (56
                    + 8 * (BULK_COUNT + BULK_COMPLETED_COUNT)
                    + ctechnology::INSTANCE_BYTES * BULK_COUNT)
                    + 61 * BULK_COUNT
            );
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row.technology_handle, (index + 1) as u32);
                assert_eq!(row.researched, u8::from(index < BULK_COMPLETED_COUNT));
                assert_eq!(row.progress_raw, i64::MAX - index as i64);
            }
        }
    }
    put(
        &mut bytes,
        BULK_TEMPLATES + ctechnology_template::YEAR,
        2007_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BULK_INSTANCES + ctechnology::LIVE_PROGRESS,
        i64::MIN.to_le_bytes(),
    );
    let memory = ObservedMemory::new(&bytes);
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        ),
        Outcome::Rows { len: BULK_COUNT }
    );
    assert_eq!(memory.definition_batches.get(), BULK_COUNT);
    assert_eq!(definitions.entry(1).unwrap().row.year, 2007);
    assert_eq!(rows[0].progress_raw, i64::MIN);
}

#[test]
fn failed_definition_is_attempted_once_without_losing_native_instance_rows() {
    let bytes = bulk_fixture();
    let mut memory = ObservedMemory::new(&bytes);
    memory.fail = Some(BULK_TEMPLATES + ctechnology_template::ID);
    let mut rows = vec![ctechnology::Row::default(); BULK_COUNT];
    let mut source_pointers = vec![0; BULK_COUNT];
    let mut entries = vec![ctechnology_template::Entry::default(); BULK_COUNT + 1];
    let mut names = vec![0; BULK_COUNT * 4];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    for _ in 0..2 {
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Rows { len: BULK_COUNT }
        );
        assert_eq!(
            definitions.outcome(),
            Outcome::Unavailable {
                reason: "technology template fields unreadable"
            }
        );
        assert_eq!(memory.definition_batches.get(), BULK_COUNT);
    }
}

#[test]
fn first_capture_growth_keeps_all_rows_indexes_and_exact_native_work() {
    let bytes = bulk_fixture();
    for capacity in [0, 1, 64, 70, BULK_COUNT] {
        for dictionary_capacity in [0, 1, 4, 64, BULK_COUNT + 1] {
            let memory = ObservedMemory::new(&bytes);
            let mut rows = vec![ctechnology::Row::default(); capacity];
            let mut source_pointers = vec![0; capacity];
            let mut entries = vec![ctechnology_template::Entry::default(); dictionary_capacity];
            let mut names = Vec::new();
            let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
            assert_eq!(
                walk_technologies(
                    &memory,
                    MODULE,
                    COUNTRY_POINTER,
                    7,
                    (&mut rows, &mut source_pointers),
                    0,
                    &mut definitions,
                ),
                Outcome::Rows { len: BULK_COUNT }
            );
            assert_eq!(definitions.outcome(), Outcome::Rows { len: BULK_COUNT });
            assert_eq!(
                memory.instance_batches.get(),
                BULK_COUNT.div_ceil(ctechnology::BATCH)
            );
            assert_eq!(
                memory.instance_spans.get(),
                BULK_COUNT * ctechnology::FIELDS.len()
            );
            assert_eq!(
                memory.instance_bytes.get(),
                BULK_COUNT * ctechnology::INSTANCE_BYTES
            );
            assert_eq!(memory.identity_batches.get(), 0);
            assert_eq!(memory.identity_spans.get(), 0);
            assert_eq!(memory.identity_bytes.get(), 0);
            assert_eq!(memory.definition_batches.get(), BULK_COUNT);
            let pages = BULK_COUNT.div_ceil(ctechnology::BATCH)
                + BULK_COMPLETED_COUNT.div_ceil(ctechnology::BATCH);
            assert_eq!(
                memory.guards.get(),
                4 + pages + BULK_COUNT.div_ceil(ctechnology::BATCH) + 3 * BULK_COUNT
            );
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row.technology_handle, (index + 1) as u32);
                assert_eq!(row.researched, u8::from(index < BULK_COMPLETED_COUNT));
                assert_eq!(row.progress_raw, i64::MAX - index as i64);
                assert_eq!(row.level, -2);
                assert_eq!((row.country, row.classifier), (7, 4));
                assert_eq!(row.ahead_reduction_years_raw, -8_123_456_789);
                assert_eq!(source_pointers[index], BULK_INSTANCES + index * 0x200);
                assert_eq!(
                    definitions.instance_pointer_row(BULK_INSTANCES + index * 0x200),
                    Some(index)
                );
            }
            assert_eq!(definitions.bytes_len, BULK_COUNT * 4);
        }
    }
}

#[test]
fn oversized_roster_headers_only_grow_for_readable_instance_batches() {
    let mut bytes = bulk_fixture();
    array(
        &mut bytes,
        STATUS_POINTER + TECHNOLOGIES_DATA,
        BULK_ALL,
        32_768,
    );
    for (fault, kept, reason) in [
        (BULK_ALL, 0, "technology roster pointers unreadable"),
        (BULK_INSTANCES, 0, "technology fields unreadable"),
        (
            BULK_ALL + ctechnology::BATCH * POINTER_STRIDE,
            ctechnology::BATCH,
            "technology roster pointers unreadable",
        ),
    ] {
        let mut memory = ObservedMemory::new(&bytes);
        memory.fail = Some(fault);
        let previous = ctechnology::Row {
            country: 99,
            ..Default::default()
        };
        let mut rows = vec![previous];
        let mut source_pointers = vec![usize::MAX];
        let capacities = (rows.capacity(), source_pointers.capacity());
        let mut entries = Vec::new();
        let mut names = Vec::new();
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                1,
                &mut definitions,
            ),
            Outcome::Unavailable { reason }
        );
        assert_eq!(rows.len(), 1 + kept);
        assert_eq!(source_pointers.len(), rows.len());
        assert_eq!(rows[0], previous);
        assert_eq!(source_pointers[0], usize::MAX);
        if kept == 0 {
            assert_eq!((rows.capacity(), source_pointers.capacity()), capacities);
            assert_eq!(definitions.outcome(), Outcome::Rows { len: 0 });
        }
        for index in 0..kept {
            assert_eq!(rows[1 + index].technology_handle, (index + 1) as u32);
            assert_eq!(source_pointers[1 + index], BULK_INSTANCES + index * 0x200);
            assert_eq!(
                definitions.instance_pointer_row(BULK_INSTANCES + index * 0x200),
                Some(index)
            );
        }
    }
}

#[test]
fn oversized_definition_name_does_not_allocate_before_its_payload_read() {
    const HEAP: usize = 0x100000;
    const LENGTH: usize = 32768;
    struct UnreadableName<'a> {
        inner: SliceMemory<'a>,
        payload_reads: Cell<usize>,
    }
    impl Memory for UnreadableName<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == HEAP + LENGTH && output.len() == 1 {
                output[0] = 0;
                return true;
            }
            if address == HEAP {
                self.payload_reads.set(self.payload_reads.get() + 1);
                assert!(output.len() <= 256);
                return false;
            }
            self.inner.copy(address, output)
        }
    }
    let mut bytes = fixture();
    let string = TEMPLATE_B + ctechnology_template::ID;
    put(&mut bytes, string, (HEAP as u64).to_le_bytes());
    put(&mut bytes, string + 16, (LENGTH as u64).to_le_bytes());
    put(&mut bytes, string + 24, (LENGTH as u64).to_le_bytes());
    let memory = UnreadableName {
        inner: SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        },
        payload_reads: Cell::new(0),
    };
    let mut entries = Vec::new();
    let mut names = Vec::new();
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    definitions.admit(&memory, TEMPLATE_B, 17, 1);
    assert_eq!(
        definitions.outcome(),
        Outcome::Unavailable {
            reason: "technology id bytes unreadable"
        }
    );
    assert_eq!(definitions.bytes_len, 0);
    assert_eq!(definitions.required_bytes, 0);
    assert_eq!(definitions.names.capacity(), 0);
    assert_eq!(memory.payload_reads.get(), 1);
}

#[test]
fn researched_pointer_aliases_require_authentication_and_exact_owned_membership() {
    const ALIAS: usize = TECHNOLOGY_B + 0x200;
    for dictionary_capacity in [0, 1, 32] {
        for pointer in [TECHNOLOGY_B, TECHNOLOGY_A] {
            for (offset, reason) in [
                (
                    None,
                    "researched technology pointer disagrees with country roster",
                ),
                (Some(0), "technology vtable mismatch"),
                (
                    Some(ctechnology::STATUS),
                    "technology status backlink mismatch",
                ),
                (Some(ctechnology::TEMPLATE), "technology template is null"),
            ] {
                let mut bytes = fixture();
                bytes.copy_within(
                    pointer - COUNTRY_POINTER..pointer - COUNTRY_POINTER + 0x200,
                    ALIAS - COUNTRY_POINTER,
                );
                put(&mut bytes, COMPLETED, (ALIAS as u64).to_le_bytes());
                if let Some(offset) = offset {
                    put(&mut bytes, ALIAS + offset, 0_u64.to_le_bytes());
                }
                let memory = ObservedMemory::new(&bytes);
                let mut rows = vec![ctechnology::Row::default(); 3];
                let mut source_pointers = vec![0; 3];
                let mut entries = vec![ctechnology_template::Entry::default(); dictionary_capacity];
                let mut names = vec![0; 64];
                let mut definitions =
                    ctechnology_template::Dictionary::new(&mut entries, &mut names);
                assert_eq!(
                    walk_technologies(
                        &memory,
                        MODULE,
                        COUNTRY_POINTER,
                        7,
                        (&mut rows, &mut source_pointers),
                        0,
                        &mut definitions,
                    ),
                    Outcome::Unavailable { reason }
                );
                assert_eq!(source_pointers, [TECHNOLOGY_B, TECHNOLOGY_A, 0]);
                assert_eq!(rows[0].progress_raw, i64::MAX - 8);
                assert_eq!(rows[1].progress_raw, i64::MIN + 9);
                assert_eq!((rows[0].researched, rows[1].researched), (0, 0));
                assert_eq!(memory.instance_bytes.get(), 2 * ctechnology::INSTANCE_BYTES);
                assert_eq!(
                    (memory.identity_spans.get(), memory.identity_bytes.get()),
                    (2, 28)
                );
            }
        }
    }

    let mut bytes = fixture();
    put(&mut bytes, ALL, 0_u64.to_le_bytes());
    let memory = ObservedMemory::new(&bytes);
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut vec![0; 3]),
            0,
            &mut definitions,
        ),
        Outcome::Unavailable {
            reason: "researched technology missing from country roster"
        }
    );
    assert_eq!(memory.instance_bytes.get(), ctechnology::INSTANCE_BYTES);
    assert_eq!(memory.identity_bytes.get(), 28);
}

#[test]
fn pointer_scratch_reuse_ignores_previous_roster_tail() {
    for dictionary_capacity in [0, 1, 32] {
        let mut bytes = fixture();
        let mut rows = vec![ctechnology::Row::default(); 3];
        let mut source_pointers = vec![usize::MAX; 3];
        let mut entries = vec![ctechnology_template::Entry::default(); dictionary_capacity];
        let mut names = vec![0; 64];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert_eq!(
            walk_technologies(
                &ObservedMemory::new(&bytes),
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(source_pointers, [TECHNOLOGY_B, TECHNOLOGY_A, usize::MAX]);

        put(&mut bytes, ALL + 16, 0_u64.to_le_bytes());
        put(&mut bytes, COMPLETED, (TECHNOLOGY_A as u64).to_le_bytes());
        let memory = ObservedMemory::new(&bytes);
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                8,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Unavailable {
                reason: "researched technology missing from country roster"
            }
        );
        assert_eq!(source_pointers, [TECHNOLOGY_B, TECHNOLOGY_A, usize::MAX]);
        assert_eq!((rows[0].country, rows[0].researched), (8, 0));
        assert_eq!(memory.instance_bytes.get(), ctechnology::INSTANCE_BYTES);
        assert_eq!(memory.identity_bytes.get(), 28);
    }
}

#[test]
fn growth_preserves_batch_fault_precedence_and_prior_page_flags() {
    for capacity in [0, 1, BULK_COUNT] {
        for (fail, kept, flagged, reason) in [
            (
                BULK_INSTANCES + 31 * 0x200,
                0,
                0,
                "technology fields unreadable",
            ),
            (
                BULK_INSTANCES + 70 * 0x200 + ctechnology::STATUS,
                64,
                0,
                "technology fields unreadable",
            ),
            (
                BULK_COMPLETED + 64 * 8,
                BULK_COUNT,
                64,
                "researched technology pointers unreadable",
            ),
        ] {
            let mut bytes = bulk_fixture();
            if kept == 0 {
                put(&mut bytes, BULK_INSTANCES + 2 * 0x200, 0_u64.to_le_bytes());
            }
            let mut memory = ObservedMemory::new(&bytes);
            memory.fail = Some(fail);
            let mut rows = vec![ctechnology::Row::default(); capacity];
            let mut source_pointers = vec![0; capacity];
            let mut entries = Vec::new();
            let mut names = Vec::new();
            let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
            assert_eq!(
                walk_technologies(
                    &memory,
                    MODULE,
                    COUNTRY_POINTER,
                    7,
                    (&mut rows, &mut source_pointers),
                    0,
                    &mut definitions,
                ),
                Outcome::Unavailable { reason }
            );
            for (index, row) in rows[..kept].iter().enumerate() {
                assert_eq!(row.progress_raw, i64::MAX - index as i64);
                assert_eq!(row.researched, u8::from(index < flagged));
                assert_eq!(source_pointers[index], BULK_INSTANCES + index * 0x200);
            }
            assert!(
                rows[kept..]
                    .iter()
                    .all(|row| *row == ctechnology::Row::default())
            );
            assert!(source_pointers[kept..].iter().all(|&pointer| pointer == 0));
            assert_eq!(memory.identity_batches.get(), 0);
        }
    }
}

#[test]
fn primary_batch_faults_invalidate_the_table_without_admitting_partial_rows() {
    let bytes = bulk_fixture();
    for (fail, kept) in [
        (BULK_INSTANCES + 31 * 0x200, 0),
        (BULK_INSTANCES + 64 * 0x200 + ctechnology::LIVE_PROGRESS, 64),
    ] {
        let mut memory = ObservedMemory::new(&bytes);
        memory.fail = Some(fail);
        let mut rows = vec![ctechnology::Row::default(); BULK_COUNT];
        let mut source_pointers = vec![usize::MAX; BULK_COUNT];
        let mut entries = vec![ctechnology_template::Entry::default(); BULK_COUNT + 1];
        let mut names = vec![0; BULK_COUNT * 4];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                7,
                (&mut rows, &mut source_pointers),
                0,
                &mut definitions,
            ),
            Outcome::Unavailable {
                reason: "technology fields unreadable"
            }
        );
        for (index, row) in rows[..kept].iter().enumerate() {
            assert_eq!(row.technology_handle, (index + 1) as u32);
            assert_eq!(row.progress_raw, i64::MAX - index as i64);
            assert_eq!(row.researched, 0);
            assert_eq!(source_pointers[index], BULK_INSTANCES + index * 0x200);
        }
        assert!(
            rows[kept..]
                .iter()
                .all(|row| *row == ctechnology::Row::default())
        );
        assert_eq!(memory.identity_batches.get(), 0);
        assert!(
            source_pointers[kept..]
                .iter()
                .all(|&pointer| pointer == usize::MAX)
        );
    }
}

#[test]
fn slots_refuse_contradictory_idle_identity_and_duplicate_ordinals() {
    for (address, value) in [
        (SLOT_B + cresearch_slot::TECHNOLOGY_HANDLE, 7_u32),
        (SLOT_B + cresearch_slot::INDEX, 0),
        (SLOT_A + cresearch_slot::TECHNOLOGY_HANDLE, 9),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, value.to_le_bytes());
        let memory = SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &bytes,
        };
        let mut slots = vec![cresearch_slot::Row::default(); 2];
        assert!(matches!(
            walk_slots(&memory, MODULE, COUNTRY_POINTER, 7, &mut slots),
            Outcome::Unavailable { .. }
        ));
    }
}

#[test]
fn repeated_handles_keep_native_occurrences_and_exact_researched_membership_at_country_scale() {
    let mut bytes = fixture();
    let handle = 0xffff_fffe_u32;
    for pointer in [TECHNOLOGY_A, TECHNOLOGY_B] {
        put(
            &mut bytes,
            pointer + ctechnology::DEFINITION_INDEX,
            handle.to_le_bytes(),
        );
    }
    for template in [TEMPLATE_A, TEMPLATE_B] {
        put(
            &mut bytes,
            template + ctechnology_template::TECHNOLOGY_HANDLE,
            handle.to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        TEMPLATE_A + ctechnology_template::YEAR,
        2001_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE_A + ctechnology_template::MAX_LEVEL,
        2_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TEMPLATE_A + ctechnology_template::RESEARCH_COST,
        17_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        TECHNOLOGY_A + ctechnology::APPLIED_BONUS_IDS + 12,
        3_i32.to_le_bytes(),
    );
    let memory = ObservedMemory::new(&bytes);
    let mut rows = Vec::new();
    let mut pointers = Vec::new();
    let mut entries = Vec::new();
    let mut names = Vec::new();
    let mut dictionary = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    for country in 0..5_000 {
        assert_eq!(
            walk_technologies(
                &memory,
                MODULE,
                COUNTRY_POINTER,
                country,
                (&mut rows, &mut pointers),
                0,
                &mut dictionary
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!((rows[0].database_index, rows[1].database_index), (1, 3));
        assert_eq!(
            (rows[0].technology_handle, rows[1].technology_handle),
            (handle, handle)
        );
        assert_eq!((rows[0].researched, rows[1].researched), (1, 0));
        assert_ne!(rows[0].level, rows[1].level);
        assert_ne!(rows[0].progress_raw, rows[1].progress_raw);
        assert_eq!(rows[1].applied_bonus_count, 3);
    }
    assert_eq!(dictionary.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(dictionary.entry(3).unwrap().row.year, 2001);
    assert_eq!(dictionary.entry(3).unwrap().row.max_level, 2);
    assert_eq!(dictionary.entry(3).unwrap().row.research_cost_raw, 17);
    assert_eq!(
        memory.spans.get() - 5_000 * (5 + 2 + ctechnology::FIELDS.len() * 2),
        10
    );
}

#[test]
fn repeated_handles_survive_dictionary_rehash_without_rereading_templates() {
    for shared in [None, Some(0xffff_fffe_u32)] {
        let mut bytes = bulk_fixture();
        for index in 0..BULK_COUNT {
            let handle = shared.unwrap_or((index / 2 + 1) as u32);
            put(
                &mut bytes,
                BULK_INSTANCES + index * 0x200 + ctechnology::DEFINITION_INDEX,
                handle.to_le_bytes(),
            );
            put(
                &mut bytes,
                BULK_TEMPLATES + index * 0x400 + ctechnology_template::TECHNOLOGY_HANDLE,
                handle.to_le_bytes(),
            );
        }
        let researched_count = if shared.is_some() { 1 } else { BULK_COUNT / 2 };
        array(
            &mut bytes,
            STATUS_POINTER + RESEARCHED_DATA,
            BULK_COMPLETED,
            researched_count as u32,
        );
        for index in 0..researched_count {
            put(
                &mut bytes,
                BULK_COMPLETED + index * 8,
                (BULK_INSTANCES as u64
                    + (if shared.is_some() {
                        BULK_COUNT - 1
                    } else {
                        index * 2
                    }) as u64
                        * 0x200)
                    .to_le_bytes(),
            );
        }
        let memory = ObservedMemory::new(&bytes);
        let mut rows = Vec::new();
        let mut pointers = Vec::new();
        let mut entries = Vec::new();
        let mut names = Vec::new();
        let mut dictionary = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        for country in 0..2 {
            assert_eq!(
                walk_technologies(
                    &memory,
                    MODULE,
                    COUNTRY_POINTER,
                    country,
                    (&mut rows, &mut pointers),
                    0,
                    &mut dictionary,
                ),
                Outcome::Rows { len: BULK_COUNT }
            );
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row.database_index, (index + 1) as i32);
                assert_eq!(
                    row.technology_handle,
                    shared.unwrap_or((index / 2 + 1) as u32)
                );
                assert_eq!(
                    row.researched,
                    u8::from(if shared.is_some() {
                        index == BULK_COUNT - 1
                    } else {
                        index % 2 == 0
                    })
                );
            }
        }
        assert_eq!(dictionary.outcome(), Outcome::Rows { len: BULK_COUNT });
        assert_eq!(memory.identity_batches.get(), 0);
        assert_eq!(
            memory.spans.get(),
            2 * (5
                + BULK_COUNT.div_ceil(ctechnology::BATCH)
                + researched_count.div_ceil(ctechnology::BATCH)
                + BULK_COUNT * ctechnology::FIELDS.len())
                + 5 * BULK_COUNT
        );
    }
}

#[test]
fn governance_batch_slots_serial_and_batched_equality() {
    use engine_governance_tests::{Trace, World};
    let world = World::new(3);
    let before = Trace::new(&world, "ResearchSlots serial");
    let after = Trace::new(&world, "ResearchSlots batch");
    for id in 1..=3 {
        let mut old = vec![cresearch_slot::Row::default(); 3];
        let mut new = old.clone();
        let serial = super::serial_slots::walk_slots_buffer(
            &before,
            0,
            World::country(id),
            id,
            crate::walk::Buffer::Growing(&mut old),
        );
        let batched = walk_slots(&after, 0, World::country(id), id, &mut new);
        assert_eq!(serial, Outcome::Rows { len: 3 });
        assert_eq!(batched, serial);
        assert_eq!(new, old);
        assert_eq!((before.count(), after.count()), (13, 7));
        assert!(after.fallbacks.witnesses().is_empty());
        before.clear();
        after.clear();
    }
}

#[test]
fn governance_batch_slots_persistent_and_one_shot_every_serial_operand() {
    use engine_governance_tests::{
        SLOT, SLOTS, TECH, TEMPLATE, Trace, World, addresses, never_harsher,
    };
    let world = World::new(1);
    let capture = |memory: &Trace<'_, World>, serial: bool| {
        let mut rows = vec![cresearch_slot::Row::default(); 3];
        let outcome = if serial {
            super::serial_slots::walk_slots_buffer(
                memory,
                0,
                World::country(1),
                1,
                crate::walk::Buffer::Growing(&mut rows),
            )
        } else {
            walk_slots(memory, 0, World::country(1), 1, &mut rows)
        };
        (outcome, rows[..outcome.len()].to_vec())
    };
    let country = World::country(1);
    for (offset, reader) in [
        (SLOTS, "slot_pointers"),
        (SLOT + cresearch_slot::STATUS, "slots"),
        (
            TECH + crate::ctechnology::LIVE_PROGRESS,
            "assigned_technologies",
        ),
        (
            TEMPLATE + crate::ctechnology_template::DATABASE_INDEX,
            "assigned_templates",
        ),
    ] {
        for once in [false, true] {
            let before = Trace::fault(&world, "ResearchSlots serial", country + offset, once);
            let after = Trace::fault(&world, "ResearchSlots batch", country + offset, once);
            let old = capture(&before, true);
            let new = capture(&after, false);
            if once {
                never_harsher(old.0, new.0);
            } else {
                assert_eq!(new, old);
            }
            assert_eq!(
                &*after.fallbacks.witnesses(),
                &[reader],
                "{reader}, once={once}"
            );
        }
    }
    let census = Trace::new(&world, "ResearchSlots serial census");
    capture(&census, true);
    for address in addresses(&census) {
        for once in [false, true] {
            let before = Trace::fault(&world, "ResearchSlots serial", address, once);
            let after = Trace::fault(&world, "ResearchSlots batch", address, once);
            let old = capture(&before, true);
            let new = capture(&after, false);
            if once {
                never_harsher(old.0, new.0);
            } else {
                assert_eq!(new, old, "persistent {address:#x}");
            }
            for reader in [
                "slot_pointers",
                "slots",
                "assigned_technologies",
                "assigned_templates",
            ] {
                assert!(
                    after
                        .fallbacks
                        .witnesses()
                        .iter()
                        .filter(|&&r| r == reader)
                        .count()
                        <= 1
                );
            }
        }
    }
}

#[test]
fn full_census_and_researched_identity_batches_retry_once_without_rereading_completed_rows() {
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct CountedFault<'a> {
        inner: SliceMemory<'a>,
        fault: usize,
        once: bool,
        spent: Cell<bool>,
        reads: RefCell<HashMap<usize, usize>>,
    }

    impl<'a> CountedFault<'a> {
        fn new(bytes: &'a [u8], fault: usize, once: bool) -> Self {
            Self {
                inner: SliceMemory {
                    base: COUNTRY_POINTER,
                    bytes,
                },
                fault,
                once,
                spent: Cell::new(false),
                reads: RefCell::new(HashMap::new()),
            }
        }

        fn reads_at(&self, address: usize) -> usize {
            self.reads.borrow().get(&address).copied().unwrap_or(0)
        }
    }

    impl Memory for CountedFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            {
                let mut reads = self.reads.borrow_mut();
                for offset in 0..destination.len() {
                    *reads.entry(address + offset).or_insert(0) += 1;
                }
            }
            if address <= self.fault
                && self.fault < address.saturating_add(destination.len())
                && (!self.once || !self.spent.get())
            {
                if self.once {
                    self.spent.set(true);
                }
                destination.fill(0xfe);
                return false;
            }
            self.inner.copy(address, destination)
        }
    }

    let bytes = bulk_fixture();
    let kept = BULK_INSTANCES + 30 * 0x200 + ctechnology::LIVE_PROGRESS;
    let fault = BULK_INSTANCES + 31 * 0x200 + ctechnology::LIVE_PROGRESS;
    let after = BULK_INSTANCES + 32 * 0x200 + ctechnology::LIVE_PROGRESS;
    for once in [true, false] {
        let memory = CountedFault::new(&bytes, fault, once);
        let mut rows = vec![ctechnology::Row::default(); BULK_COUNT];
        let mut source_pointers = vec![0; BULK_COUNT];
        let mut entries = vec![ctechnology_template::Entry::default(); BULK_COUNT + 1];
        let mut names = vec![0; BULK_COUNT * 4];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        let outcome = walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        );
        if once {
            assert_eq!(outcome, Outcome::Rows { len: BULK_COUNT });
            assert_eq!(rows[31].progress_raw, i64::MAX - 31);
            assert_eq!(memory.reads_at(after), 1);
        } else {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: "technology fields unreadable"
                }
            );
            assert_eq!(memory.reads_at(after), 0);
        }
        assert_eq!(memory.reads_at(kept), 1);
        assert_eq!(memory.reads_at(fault), 2);
    }

    const ALIAS_A: usize = TECHNOLOGY_B + 0x200;
    const ALIAS_B: usize = TECHNOLOGY_B + 0x400;
    let mut bytes = fixture();
    bytes.copy_within(
        TECHNOLOGY_B - COUNTRY_POINTER..TECHNOLOGY_B - COUNTRY_POINTER + 0x200,
        ALIAS_A - COUNTRY_POINTER,
    );
    bytes.copy_within(
        TECHNOLOGY_A - COUNTRY_POINTER..TECHNOLOGY_A - COUNTRY_POINTER + 0x200,
        ALIAS_B - COUNTRY_POINTER,
    );
    array(&mut bytes, STATUS_POINTER + RESEARCHED_DATA, COMPLETED, 2);
    put(&mut bytes, COMPLETED, (ALIAS_A as u64).to_le_bytes());
    put(&mut bytes, COMPLETED + 8, (ALIAS_B as u64).to_le_bytes());
    for once in [true, false] {
        let memory = CountedFault::new(&bytes, ALIAS_A + ctechnology::STATUS, once);
        let mut rows = vec![ctechnology::Row::default(); 3];
        let mut source_pointers = vec![0; 3];
        let mut entries = vec![ctechnology_template::Entry::default(); 32];
        let mut names = vec![0; 64];
        let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
        let outcome = walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        );
        assert_eq!(
            outcome,
            Outcome::Unavailable {
                reason: if once {
                    "researched technology pointer disagrees with country roster"
                } else {
                    "technology fields unreadable"
                }
            }
        );
        assert_eq!(memory.reads_at(ALIAS_A + ctechnology::STATUS), 2);
    }
    let memory = CountedFault::new(&bytes, ALIAS_B + ctechnology::STATUS, false);
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![0; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    let outcome = walk_technologies(
        &memory,
        MODULE,
        COUNTRY_POINTER,
        7,
        (&mut rows, &mut source_pointers),
        0,
        &mut definitions,
    );
    assert_eq!(
        outcome,
        Outcome::Unavailable {
            reason: "technology fields unreadable"
        }
    );
    assert_eq!(memory.reads_at(ALIAS_A + ctechnology::STATUS), 1);
    assert_eq!(memory.reads_at(ALIAS_B + ctechnology::STATUS), 2);

    put(&mut bytes, COMPLETED, (TECHNOLOGY_B as u64).to_le_bytes());
    put(&mut bytes, COMPLETED + 8, (ALIAS_A as u64).to_le_bytes());
    let memory = CountedFault::new(&bytes, ALIAS_A + ctechnology::STATUS, false);
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![0; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        ),
        Outcome::Unavailable {
            reason: "technology fields unreadable"
        }
    );
    assert_eq!(rows[0].researched, 0);
    assert_eq!(memory.reads_at(ALIAS_A + ctechnology::STATUS), 2);

    array(&mut bytes, STATUS_POINTER + RESEARCHED_DATA, COMPLETED, 3);
    put(&mut bytes, COMPLETED, (ALIAS_A as u64).to_le_bytes());
    put(&mut bytes, COMPLETED + 8, (ALIAS_B as u64).to_le_bytes());
    put(&mut bytes, COMPLETED + 16, (ALIAS_A as u64).to_le_bytes());
    let memory = CountedFault::new(&bytes, ALIAS_B + ctechnology::STATUS, true);
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![0; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        ),
        Outcome::Unavailable {
            reason: "researched technology pointer disagrees with country roster"
        }
    );
    assert_eq!(memory.reads_at(ALIAS_A + ctechnology::STATUS), 1);
    assert_eq!(memory.reads_at(ALIAS_B + ctechnology::STATUS), 2);

    struct DuplicateGuard<'a> {
        inner: SliceMemory<'a>,
        one_shot: usize,
        second_fault: usize,
        reads: RefCell<HashMap<usize, usize>>,
    }
    impl Memory for DuplicateGuard<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let mut hit_one_shot = false;
            let mut hit_second = false;
            {
                let mut reads = self.reads.borrow_mut();
                for offset in 0..destination.len() {
                    let at = address + offset;
                    let count = reads.entry(at).or_insert(0);
                    *count += 1;
                    if at == self.one_shot && *count == 1 {
                        hit_one_shot = true;
                    }
                    if at == self.second_fault && *count >= 2 {
                        hit_second = true;
                    }
                }
            }
            if hit_one_shot || hit_second {
                destination.fill(0xfe);
                return false;
            }
            self.inner.copy(address, destination)
        }
    }

    let mut dup_bytes = fixture();
    put(&mut dup_bytes, ALL, (TECHNOLOGY_B as u64).to_le_bytes());
    put(&mut dup_bytes, ALL + 8, (TECHNOLOGY_A as u64).to_le_bytes());
    put(
        &mut dup_bytes,
        ALL + 16,
        (TECHNOLOGY_B as u64).to_le_bytes(),
    );
    let dup_memory = DuplicateGuard {
        inner: SliceMemory {
            base: COUNTRY_POINTER,
            bytes: &dup_bytes,
        },
        one_shot: TECHNOLOGY_A + ctechnology::LIVE_PROGRESS,
        second_fault: TECHNOLOGY_B + ctechnology::LIVE_PROGRESS,
        reads: RefCell::new(HashMap::new()),
    };
    let mut rows = vec![ctechnology::Row::default(); 3];
    let mut source_pointers = vec![0; 3];
    let mut entries = vec![ctechnology_template::Entry::default(); 32];
    let mut names = vec![0; 64];
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    assert_eq!(
        walk_technologies(
            &dup_memory,
            MODULE,
            COUNTRY_POINTER,
            7,
            (&mut rows, &mut source_pointers),
            0,
            &mut definitions,
        ),
        Outcome::Unavailable {
            reason: "duplicate technology pointer in country roster"
        }
    );
    assert_eq!(
        dup_memory
            .reads
            .borrow()
            .get(&(TECHNOLOGY_B + ctechnology::LIVE_PROGRESS))
            .copied(),
        Some(1)
    );
    assert_eq!(
        dup_memory
            .reads
            .borrow()
            .get(&(TECHNOLOGY_A + ctechnology::LIVE_PROGRESS))
            .copied(),
        Some(2)
    );

    let mut shared_slot_bytes = fixture();
    put(
        &mut shared_slot_bytes,
        SLOT_B + cresearch_slot::TECHNOLOGY,
        (TECHNOLOGY_A as u64).to_le_bytes(),
    );
    put(
        &mut shared_slot_bytes,
        SLOT_B + cresearch_slot::TECHNOLOGY_HANDLE,
        3_u32.to_le_bytes(),
    );
    let shared_fault = TECHNOLOGY_A + ctechnology::LIVE_PROGRESS;
    let shared_memory = CountedFault::new(&shared_slot_bytes, shared_fault, true);
    let mut slots = vec![cresearch_slot::Row::default(); 2];
    assert_eq!(
        walk_slots(&shared_memory, MODULE, COUNTRY_POINTER, 7, &mut slots),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(shared_memory.reads_at(shared_fault), 2);
}
