use super::*;
use engine::{memory::SliceMemory, modifier_accumulator};

const BASE: usize = 0x1000;
const ENTRIES: usize = 0x1100;

struct Source {
    base: usize,
    bytes: RefCell<Vec<u8>>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Cell<Option<usize>>,
    scalars: Cell<usize>,
    scatters: Cell<usize>,
    records: Cell<usize>,
}
impl Source {
    fn from_bytes(base: usize, bytes: Vec<u8>) -> Self {
        Self {
            base,
            bytes: RefCell::new(bytes),
            reads: RefCell::new(Vec::new()),
            fault: Cell::new(None),
            scalars: Cell::new(0),
            scatters: Cell::new(0),
            records: Cell::new(0),
        }
    }
    fn new(count: u32) -> Self {
        let source = Self::from_bytes(BASE, vec![0; 0x400 + count as usize * 16]);
        source.array(BASE, ENTRIES, count);
        for index in 0..count {
            source.put(ENTRIES + index as usize * 16, &(index + 1).to_le_bytes());
            source.put(
                ENTRIES + index as usize * 16 + 8,
                &(i64::from(index) * -13).to_le_bytes(),
            );
        }
        source
    }
    fn put(&self, address: usize, value: &[u8]) {
        let offset = address - self.base;
        self.bytes.borrow_mut()[offset..offset + value.len()].copy_from_slice(value);
    }
    fn array(&self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }
}

// Force Memory's original scalar batch defaults without forwarding any
// capability. This supplies an independent budget/failure-prefix reference.
struct Scalar<'a, M>(&'a M);
impl<M: Memory> Memory for Scalar<'_, M> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.0.copy(address, out)
    }
}
impl Memory for Source {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        self.scalars.set(self.scalars.get() + 1);
        Scalar(self).read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, out.len()));
        if self.fault.get() == Some(address) {
            for byte in out.iter_mut().take(2) {
                *byte = 0xe1;
            }
            return false;
        }
        SliceMemory {
            base: self.base,
            bytes: &self.bytes.borrow(),
        }
        .copy(address, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.set(self.scatters.get() + 1);
        Scalar(self).copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.records.set(self.records.get() + 1);
        Scalar(self).copy_records(sources, layout, destination)
    }
}

fn capacities(nodes: usize, entries: usize) -> Capacities {
    Capacities {
        rows: 1,
        modifiers: SnapshotCapacity { nodes, entries },
    }
}

/// The model selects this nonpositive entry, then refuses its sprite.
fn model_source() -> (Source, cship::Row) {
    use engine::{cequipment_variant, cequipment_variant_pool, ctask_force, cunit, msvc_string};
    let source = Source::from_bytes(BASE, vec![0; 0x5000]);
    let (parent, variant, bank) = (0x2000, 0x3000, 0x4000);
    let ship = (42_u64 << 32) | u64::from(cship::HANDLE_TYPE);
    let task_force = (17_u64 << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
    source.put(BASE, &(cship::VTABLE_RVA as u64).to_le_bytes());
    source.put(BASE + cship::HANDLE, &ship.to_le_bytes());
    source.put(BASE + cship::TASK_FORCE, &(parent as u64).to_le_bytes());
    source.put(parent, &(ctask_force::VTABLE_RVA as u64).to_le_bytes());
    source.put(parent + cunit::HANDLE, &task_force.to_le_bytes());
    source.put(
        BASE + cship::EQUIPMENT_POOL,
        &(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
    );
    source.array(
        BASE + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        bank,
        1,
    );
    source.put(bank, &(variant as u64).to_le_bytes());
    source.put(bank + 8, &i64::MIN.to_le_bytes());
    source.put(
        variant,
        &(cequipment_variant::VTABLE_RVA as u64).to_le_bytes(),
    );
    source.put(
        variant + 8,
        &((1_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
    );
    let len = cship::MODEL_TEXT_CAPACITY as u64 + 1;
    source.put(
        BASE + cship::FALLBACK_SPRITE,
        &(variant as u64).to_le_bytes(),
    );
    source.put(
        BASE + cship::FALLBACK_SPRITE + msvc_string::LENGTH,
        &len.to_le_bytes(),
    );
    source.put(
        BASE + cship::FALLBACK_SPRITE + msvc_string::CAPACITY,
        &len.to_le_bytes(),
    );
    (
        source,
        cship::Row {
            id: ship,
            task_force,
            ..Default::default()
        },
    )
}

fn acquire(capture: &mut Captured, source: &Source) -> Result<(), &'static str> {
    capture.begin_details();
    let result = {
        let bounded = BoundedMemory {
            inner: source,
            used: &capture.copied,
        };
        let memory = ModifierMemory {
            inner: &bounded,
            arena: &capture.modifiers,
        };
        modifier_accumulator::lookup_at(&memory, BASE, 1)
    };
    match result {
        Ok(value) => {
            capture.rows.rows[0].maximum_organisation = value.raw;
            capture.len = 1;
        }
        Err(reason) => capture.failure = Some(acquisition_failure(capture.copied.get(), reason)),
    }
    capture.finish_details(Outcome::Rows { len: 1 });
    result.map(|_| ())
}

#[test]
fn modifier_memo_reuses_current_bytes_and_release_performs_no_reads() {
    let source = Source::new(2);
    let mut capture = Captured::with_capacities(&capacities(2, 4));
    let allocated = capture.allocated_bytes();
    acquire(&mut capture, &source).unwrap();
    let before = source.reads.borrow().len();
    let observation = capture
        .modifiers
        .borrow_mut()
        .capture_at(&source, BASE, 2)
        .unwrap();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(
        capture.modifiers.borrow().value(observation).unwrap().raw,
        -13
    );
    let copied = capture.copied.get();
    assert_eq!(copied.0, before);
    assert_eq!(
        copied.1,
        source
            .reads
            .borrow()
            .iter()
            .map(|&(_, len)| len)
            .sum::<usize>()
    );
    capture.release_sources();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(capture.copied.get(), copied);
    assert_eq!(
        capture.required_modifiers,
        SnapshotCapacity {
            nodes: 1,
            entries: 2
        }
    );
    assert_eq!(
        capture.modifiers.borrow().value(observation),
        Err("modifier snapshot observation expired")
    );
    assert_eq!(
        capture.modifiers.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
    assert_eq!(capture.allocated_bytes(), allocated);
}

#[test]
fn failed_snapshot_prefix_keeps_first_fault_and_releases_without_retry() {
    let source = Source::new(70);
    let failed = ENTRIES + 66 * 16;
    source.fault.set(Some(failed));
    let mut capture = Captured::with_capacities(&capacities(2, 72));
    assert_eq!(
        acquire(&mut capture, &source),
        Err("modifier accumulator entry unreadable")
    );
    let before = source.reads.borrow().len();
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|&&(p, _)| p == failed)
            .count(),
        1
    );
    capture.release_sources();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(
        capture.failure,
        Some("modifier accumulator entry unreadable")
    );
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "modifier accumulator entry unreadable"
        }]
    );
    assert_eq!(capture.required_modifiers.entries, 70);
}

#[test]
fn modifier_capacity_fallback_and_primary_growth_preserve_current_values() {
    let source = Source::new(3);
    let mut capacity = capacities(0, 0);
    let mut capture = Captured::with_capacities(&capacity);
    for expected in [
        SnapshotCapacity {
            nodes: 1,
            entries: 0,
        },
        SnapshotCapacity {
            nodes: 1,
            entries: 3,
        },
    ] {
        let allocated = capture.allocated_bytes();
        acquire(&mut capture, &source).unwrap();
        assert_eq!(capture.rows.rows[0].maximum_organisation, 0);
        let reads = source.reads.borrow().len();
        capture.release_sources();
        assert_eq!(source.reads.borrow().len(), reads);
        assert_eq!(capture.required_modifiers, expected);
        assert_eq!(capture.allocated_bytes(), allocated);
        capture.grow(&mut capacity);
        assert!(
            capacity.modifiers.nodes >= expected.nodes
                && capacity.modifiers.entries >= expected.entries
        );
        assert_eq!(capture.modifiers.borrow().capacity(), capacity.modifiers);
        capture.reset();
    }
    acquire(&mut capture, &source).unwrap();
    capture.release_sources();
    assert_eq!(
        capture.allocated_bytes(),
        capture.rows.allocated_bytes()
            + capture.seeds.capacity() * std::mem::size_of::<Seed>()
            + capture.modifiers.borrow().allocated_bytes()
            + capture
                .extra
                .iter()
                .map(Captured::allocated_bytes)
                .sum::<usize>()
    );
}

#[test]
fn huge_cache_demand_is_bounded_without_allocating_or_clamping_reported_demand() {
    let required = SnapshotCapacity {
        nodes: usize::MAX,
        entries: usize::MAX,
    };
    let mut capacity = SnapshotCapacity {
        nodes: 1,
        entries: 1,
    };
    grow_snapshot_capacity(&mut capacity, required);
    assert_eq!(capacity, SNAPSHOT_LIMITS);
    assert_eq!(required.nodes, usize::MAX);
    assert_eq!(required.entries, usize::MAX);
    grow_snapshot_capacity(&mut capacity, SnapshotCapacity::default());
    assert_eq!(capacity, SNAPSHOT_LIMITS);
}

#[test]
fn only_current_country_authentication_is_forwarded_for_matching_context() {
    const STATE: usize = 0x1000;
    let source = Source::from_bytes(0, vec![0; 0x8000]);
    source.array(STATE + engine::cgame_state::COUNTRIES, 0x5000, 2);
    source.array(STATE + engine::cgame_state::COUNTRY_LINKS, 0x5100, 2);
    let mut cache = engine::country_memory::Cache::try_with_capacity(64).unwrap();
    let cached = cache.prepare(&source, 0, STATE);
    assert_eq!(
        cached.seed_status(),
        engine::country_memory::SeedStatus::Ready
    );
    let used = Cell::new((0, 0));
    let bounded = BoundedMemory {
        inner: &cached,
        used: &used,
    };
    let arena = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let memory = ModifierMemory {
        inner: &bounded,
        arena: &arena,
    };
    assert!(memory.authenticated_countries(0, STATE).is_some());
    assert!(memory.authenticated_countries(1, STATE).is_none());
    assert!(memory.authenticated_countries(0, STATE + 1).is_none());
    assert_eq!(
        used.get(),
        (0, 0),
        "forwarding supplies no new observations"
    );
    assert!(
        BoundedMemory {
            inner: &source,
            used: &used
        }
        .authenticated_countries(0, STATE)
        .is_none()
    );
}

fn scattered(memory: &impl Memory, addresses: [usize; 4]) -> (usize, [[u8; 8]; 4]) {
    let mut output = [[0xa5; 8]; 4];
    let [a, b, c, d] = &mut output;
    let completed = memory.copy_scattered(
        addresses,
        &mut [&mut a[..4], &mut b[..0], &mut c[..8], &mut d[..3]],
    );
    (completed, output)
}

#[test]
fn scatter_forwarding_matches_scalar_prefixes_bytes_and_budgets_at_every_edge() {
    for initial in [
        (0, 0),
        (READ_LIMIT - 1, 0),
        (0, COPY_LIMIT - 5),
        (READ_LIMIT, 0),
        (0, COPY_LIMIT),
    ] {
        for failed in [None, Some(ENTRIES), Some(ENTRIES + 16), Some(ENTRIES + 32)] {
            for invalid in [None, Some(0), Some(usize::MAX)] {
                let source = Source::new(4);
                source.fault.set(failed);
                let used = Cell::new(initial);
                let bounded = BoundedMemory {
                    inner: &source,
                    used: &used,
                };
                let addresses = [
                    ENTRIES,
                    usize::MAX,
                    invalid.unwrap_or(ENTRIES + 16),
                    ENTRIES + 32,
                ];
                let expected = scattered(&Scalar(&bounded), addresses);
                let expected_used = used.replace(initial);
                let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
                assert_eq!(scattered(&bounded, addresses), expected);
                assert_eq!(used.get(), expected_used);
                assert_eq!(*source.reads.borrow(), expected_reads);
                if initial == (0, 0) {
                    assert_eq!(source.scatters.get(), 1);
                }
            }
        }
    }
}

#[test]
fn record_forwarding_matches_scalar_prefixes_partial_rows_and_budget_precedence() {
    let fields = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 4,
            len: 0,
        },
        RecordField {
            source_offset: 8,
            destination_offset: 8,
            len: 8,
        },
    ];
    let layout = RecordLayout::new(&fields, 16).unwrap();
    for initial in [
        (0, 0),
        (READ_LIMIT - 2, 0),
        (0, COPY_LIMIT - 5),
        (READ_LIMIT, 0),
        (0, COPY_LIMIT),
    ] {
        for failed in [None, Some(ENTRIES), Some(ENTRIES + 8), Some(ENTRIES + 32)] {
            for kind in 0..5 {
                let source = Source::new(4);
                source.fault.set(failed);
                let used = Cell::new(initial);
                let bounded = BoundedMemory {
                    inner: &source,
                    used: &used,
                };
                let pointers = [
                    ENTRIES,
                    match kind {
                        2 => 0,
                        3 => usize::MAX,
                        _ => ENTRIES + 32,
                    },
                ];
                let sources = match kind {
                    1 => RecordSources::Strided {
                        base: ENTRIES,
                        stride: 32,
                        count: 2,
                    },
                    4 => RecordSources::Strided {
                        base: usize::MAX - 16,
                        stride: 32,
                        count: 2,
                    },
                    _ => RecordSources::Pointers(&pointers),
                };
                let mut expected = [0xa5; 32];
                let expected_count = Scalar(&bounded).copy_records(sources, &layout, &mut expected);
                let expected_used = used.replace(initial);
                let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
                let mut actual = [0xa5; 32];
                assert_eq!(
                    bounded.copy_records(sources, &layout, &mut actual),
                    expected_count
                );
                assert_eq!(actual, expected);
                assert_eq!(used.get(), expected_used);
                assert_eq!(*source.reads.borrow(), expected_reads);
                if initial == (0, 0) {
                    assert_eq!(source.records.get(), 1);
                }
            }
        }
    }
}

#[test]
fn malformed_snapshot_prefix_precedes_a_later_batch_budget_refusal() {
    let source = Source::new(3);
    source.put(ENTRIES, &0u32.to_le_bytes());
    let used = Cell::new((READ_LIMIT - 2, 0));
    let bounded = BoundedMemory {
        inner: &source,
        used: &used,
    };
    let arena = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 1,
            entries: 3,
        })
        .unwrap(),
    );
    let memory = ModifierMemory {
        inner: &bounded,
        arena: &arena,
    };
    let reason = modifier_accumulator::lookup_at(&memory, BASE, 1).unwrap_err();
    assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
    assert_eq!(
        acquisition_failure(used.get(), reason),
        "modifier accumulator keys not positive and increasing"
    );
    assert_eq!(
        source.reads.borrow().len(),
        2,
        "no source access after the admitted prefix"
    );
}

#[test]
fn empty_fields_and_invalid_destination_geometry_never_spend_budget() {
    let source = Source::new(0);
    let used = Cell::new((READ_LIMIT, COPY_LIMIT));
    let bounded = BoundedMemory {
        inner: &source,
        used: &used,
    };
    assert_eq!(
        bounded.copy_scattered([0, usize::MAX], &mut [&mut [], &mut []]),
        2
    );
    let empty = [RecordField {
        source_offset: usize::MAX,
        destination_offset: 0,
        len: 0,
    }];
    let layout = RecordLayout::new(&empty, 0).unwrap();
    assert_eq!(
        bounded.copy_records(RecordSources::Pointers(&[0, usize::MAX]), &layout, &mut []),
        2
    );
    let nonempty = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 4,
    }];
    let layout = RecordLayout::new(&nonempty, 4).unwrap();
    assert_eq!(
        bounded.copy_records(RecordSources::Pointers(&[ENTRIES]), &layout, &mut [0; 3]),
        0
    );
    assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
    assert!(source.reads.borrow().is_empty());
}

struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("copy budget must stop before inner memory access")
    }
}
#[test]
fn whole_census_budget_refuses_bytes_and_calls_before_memory_access() {
    for value in [(READ_LIMIT, 0), (0, COPY_LIMIT)] {
        let used = Cell::new(value);
        let memory = BoundedMemory {
            inner: &NoReads,
            used: &used,
        };
        assert!(!memory.copy(1, &mut [0]));
        assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        used.set(value);
        assert_eq!(memory.read_scalar::<u32>(1), None);
        assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
    }
    for initial in [(0, 0), (READ_LIMIT - 1, 0), (0, COPY_LIMIT - 4)] {
        for failed in [None, Some(ENTRIES)] {
            let source = Source::new(1);
            source.fault.set(failed);
            let used = Cell::new(initial);
            let bounded = BoundedMemory {
                inner: &source,
                used: &used,
            };
            let arena = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
            let memory = ModifierMemory {
                inner: &bounded,
                arena: &arena,
            };
            let expected = engine::read::<u32>(&Scalar(&memory), ENTRIES);
            let expected_used = used.replace(initial);
            let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
            assert_eq!(source.scalars.get(), 0);
            assert_eq!(engine::read::<u32>(&memory, ENTRIES), expected);
            assert_eq!(used.get(), expected_used);
            assert_eq!(*source.reads.borrow(), expected_reads);
            assert_eq!(source.scalars.get(), 1);
            assert_eq!(used.get(), (initial.0 + 1, initial.1 + 4));
            if initial != (0, 0) {
                assert_eq!(engine::read::<u32>(&memory, ENTRIES), None);
                assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
                assert_eq!(source.scalars.get(), 1);
                assert_eq!(*source.reads.borrow(), expected_reads);
            } else {
                assert_eq!(engine::read::<u64>(&memory, BASE), Some(ENTRIES as u64));
                assert_eq!(used.get(), (2, 12));
                assert_eq!(source.scalars.get(), 1);
            }
        }
    }
    let source = Source::new(1);
    let used = Cell::new((0, COPY_LIMIT));
    let memory = BoundedMemory {
        inner: &source,
        used: &used,
    };
    assert_eq!(engine::read::<[u8; 0]>(&memory, 0), Some([]));
    assert_eq!(used.get(), (0, COPY_LIMIT));
    assert_eq!(source.scalars.get(), 0);
    assert_eq!(memory.read_scalar::<[u8; 0]>(BASE), Some([]));
    assert_eq!(used.get(), (1, COPY_LIMIT));
    assert_eq!(source.scalars.get(), 1);
    assert_eq!(*source.reads.borrow(), [(BASE, 0)]);
}

struct OptionalModelSource(Source);
impl OptionalModelSource {
    fn new() -> (Self, cship::Row) {
        use engine::{cequipment_database, cequipment_type, cequipment_variant, msvc_string};
        let (source, row) = model_source();
        let (variant, definition, database, list) = (0x3000, 0x5000, 0x5700, 0x5800);
        source.put(BASE + cship::FALLBACK_SPRITE, &[0; 32]);
        source.put(
            BASE + cship::FALLBACK_SPRITE + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        source.put(
            variant + cequipment_variant::EQUIPMENT_TOKEN,
            &123_u32.to_le_bytes(),
        );
        source.put(
            variant + cequipment_variant::DEFINITION,
            &(definition as u64).to_le_bytes(),
        );
        source.put(
            definition,
            &(cequipment_type::VTABLE_RVA as u64).to_le_bytes(),
        );
        source.put(definition + cequipment_type::TOKEN, &123_u32.to_le_bytes());
        source.put(
            definition + cequipment_type::HULL_SPRITE + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        source.put(definition + 0x418, &1_i32.to_le_bytes());
        source.put(
            database,
            &(cequipment_database::VTABLE_RVA as u64).to_le_bytes(),
        );
        source.array(database + 0x68, list, 2);
        source.put(list + 8, &(definition as u64).to_le_bytes());
        source.put(
            BASE + cship::SUBUNIT_DEFINITION,
            &(engine::csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
        );
        source.put(BASE + cship::SUBUNIT_INDEX, &i32::MIN.to_le_bytes());
        for (offset, value) in [
            (0xc4, -1_i32),
            (0x414, i32::MIN),
            (0x418, i32::MAX),
            (0x4b0, 23),
        ] {
            source.put(variant + offset, &value.to_le_bytes());
        }
        source.put(variant + cequipment_variant::VERSION, &[u8::MAX]);
        (Self(source), row)
    }
}
impl Memory for OptionalModelSource {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == engine::cequipment_database::SINGLETON_RVA {
            self.0.reads.borrow_mut().push((address, output.len()));
            if self.0.fault.get() == Some(address) {
                return false;
            }
            assert_eq!(output.len(), 8);
            output.copy_from_slice(&0x5700_u64.to_le_bytes());
            true
        } else {
            self.0.copy(address, output)
        }
    }
}

fn optional_hull_expected() -> cship::HullVariantInputs {
    cship::HullVariantInputs {
        definition_index: Some(1),
        module_count: Some(-1),
        upgrade_level_sum: Some(i32::MIN),
        module_priority_sum: Some(i32::MAX),
        version: Some(u8::MAX),
        organisation_marker: Some(23),
    }
}

#[test]
fn optional_subunit_refusals_are_named_without_replacing_healthy_model_operands() {
    struct RelocatedHeader<'a> {
        inner: &'a OptionalModelSource,
        pointer: usize,
    }
    impl Memory for RelocatedHeader<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.pointer.checked_add(cship::SUBUNIT_DEFINITION) == Some(address) {
                self.inner.copy(BASE + cship::SUBUNIT_DEFINITION, output)
            } else {
                self.inner.copy(address, output)
            }
        }
    }
    for index in 0..6 {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, _) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        row.maximum_organisation = -701;
        let healthy = row.clone();
        let (mut base, mut pointer) = (0, BASE);
        match index {
            0 => pointer = 0,
            1 => source.0.fault.set(Some(BASE + cship::SUBUNIT_DEFINITION)),
            2 => base = usize::MAX,
            3 => source
                .0
                .put(BASE + cship::SUBUNIT_DEFINITION, &0_u64.to_le_bytes()),
            4 => pointer = usize::MAX - cship::SUBUNIT_INDEX + 1,
            5 => source.0.fault.set(Some(BASE + cship::SUBUNIT_INDEX)),
            _ => unreachable!(),
        }
        let memory = RelocatedHeader {
            inner: &source,
            pointer,
        };
        capture.capture_model_suffixes(&memory, base, pointer, &census, None, &mut row);
        assert_eq!(row, healthy, "only optional subunit index is refused");
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[index] = 1;
        assert_eq!(
            capture.optional_refusals, wanted,
            "{}",
            OPTIONAL_REFUSAL_REASONS[index]
        );
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
        let status = serde_json::to_value(optional_data()).unwrap();
        assert!(
            status["optional_operands"][OPTIONAL_REFUSAL_REASONS[index]]
                .as_u64()
                .unwrap()
                > 0
        );
        // The strike operands are no longer read, so they have no refusals.
        assert!(status.get("torpedo_cooldown_modifier_refused").is_none());
    }
}

#[test]
fn current_hull_packet_and_field_refusals_are_named_and_preserve_independent_operands() {
    let (healthy, census) = OptionalModelSource::new();
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details();
    let (mut row, selected) =
        cship::read_model_with_selection(&healthy, 0, BASE, &census, &mut capture.model).unwrap();
    capture.capture_model_suffixes(&healthy, 0, BASE, &census, selected.as_ref(), &mut row);
    assert_eq!(row.subunit_index, Some(i32::MIN));
    assert_eq!(row.hull_variant, Some(optional_hull_expected()));
    assert_eq!(
        capture.optional_refusals,
        [0; OPTIONAL_REFUSAL_REASONS.len()]
    );
    for (index, address) in [
        (6, 0x3000),
        (6, 0x3000 + engine::cequipment_variant::EQUIPMENT_TOKEN),
        (6, 0x3000 + engine::cequipment_variant::DEFINITION),
        (7, 0x5000),
        (7, 0x5000 + 0x418),
        (7, engine::cequipment_database::SINGLETON_RVA),
        (7, 0x5700),
        (7, 0x5700 + 0x68),
        (7, 0x5800 + 8),
        (8, 0x3000 + 0xc4),
        (9, 0x3000 + 0x414),
        (10, 0x3000 + 0x418),
        (11, 0x3000 + engine::cequipment_variant::VERSION),
        (12, 0x3000 + 0x4b0),
    ] {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, selected) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        row.maximum_organisation = -701;
        let healthy = row.clone();
        source.0.fault.set(Some(address));
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[index] = 1;
        assert_eq!(
            capture.optional_refusals, wanted,
            "{} at {address:#x}",
            OPTIONAL_REFUSAL_REASONS[index]
        );
        assert_eq!(row.maximum_organisation, healthy.maximum_organisation);
        assert_eq!(row.ship, healthy.ship);
        assert_eq!(row.variant, healthy.variant);
        assert_eq!(row.fallback_sprite, healthy.fallback_sprite);
        assert_eq!(row.hull_sprite, healthy.hull_sprite);
        assert_eq!(row.subunit_index, Some(i32::MIN));
        let mut packet = optional_hull_expected();
        match index {
            6 => {}
            7 => packet.definition_index = None,
            8 => packet.module_count = None,
            9 => packet.upgrade_level_sum = None,
            10 => packet.module_priority_sum = None,
            11 => packet.version = None,
            12 => packet.organisation_marker = None,
            _ => unreachable!(),
        }
        assert_eq!(row.hull_variant, (index != 6).then_some(packet));
        // Repair cannot retry the current selected-source packet or its fields.
        let reads = source.0.reads.borrow().len();
        source.0.fault.set(None);
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        assert_eq!(
            source.0.reads.borrow().len(),
            reads + 2,
            "only the independent subunit spans repeat"
        );
        assert_eq!(row.hull_variant, (index != 6).then_some(packet));
        assert_eq!(capture.optional_refusals[index], 2);
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
    }
}

#[test]
fn absent_selected_hull_is_not_counted_and_changed_selection_is_named() {
    let (source, census) = OptionalModelSource::new();
    source.0.array(
        BASE + cship::EQUIPMENT_POOL + engine::cequipment_variant_pool::ITEMS,
        0,
        0,
    );
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details();
    let (mut row, selected) =
        cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model).unwrap();
    assert!(selected.is_none());
    let before = source.0.reads.borrow().len();
    capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
    assert_eq!(source.0.reads.borrow().len(), before + 2);
    assert_eq!(row.subunit_index, Some(i32::MIN));
    assert_eq!(row.hull_variant, None);
    assert_eq!(
        capture.optional_refusals,
        [0; OPTIONAL_REFUSAL_REASONS.len()]
    );
    for (address, replacement) in [
        (0x3000, 0_u64.to_le_bytes().to_vec()),
        (
            0x3000 + 8,
            ((2_u64 << 32) | u64::from(engine::cequipment_variant::REFERENCE_TYPE))
                .to_le_bytes()
                .to_vec(),
        ),
        (
            0x3000 + engine::cequipment_variant::EQUIPMENT_TOKEN,
            124_u32.to_le_bytes().to_vec(),
        ),
        (
            0x3000 + engine::cequipment_variant::DEFINITION,
            0x5008_u64.to_le_bytes().to_vec(),
        ),
    ] {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, selected) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        source.0.put(address, &replacement);
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        assert_eq!(row.subunit_index, Some(i32::MIN));
        assert_eq!(row.hull_variant, None);
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[6] = 1;
        assert_eq!(capture.optional_refusals, wanted);
    }
}

#[test]
fn cold_ship_model_modifier_arenas_grow_from_original_mod_sized_headers() {
    let mut capture = Captured::part(&Capacities::default());
    assert_eq!(capture.modifiers.borrow().allocated_bytes(), 0);
    for count in [5_000_u32, 15_000] {
        let source = Source::new(count);
        capture.worker_growth = true;
        capture.begin_details();
        let core = ModifierMemory {
            inner: &source,
            arena: &capture.modifiers,
        };
        assert_eq!(
            modifier_accumulator::lookup_at(&core, BASE, count)
                .unwrap()
                .raw,
            i64::from(count - 1) * -13
        );
        assert!(capture.modifiers.borrow().capacity().entries >= count as usize);
        let reads = source.reads.borrow().len();
        assert_eq!(
            modifier_accumulator::lookup_at(&core, BASE, 1).unwrap().raw,
            0
        );
        assert_eq!(
            source.reads.borrow().len(),
            reads,
            "owned cache must retain each original acquisition"
        );
    }
}

/// Twelve ships, eight in one task force and four in another, whose model,
/// maximum organisation and suffix reads all succeed: country, doctrine,
/// experience, pool, variant and hull definition, without a location.
mod fleet {
    use engine::{
        ccountry, cequipment_type, cequipment_variant, cequipment_variant_pool, cgame_state, cship,
        csub_unit_definition, ctask_force, cunit,
    };

    pub(super) const IMAGE: usize = 0x1_4000_0000;
    pub(super) const LOW: usize = 0x1_0000;
    /// The game state, which the singleton slot names.
    pub(super) const GAME: usize = LOW;
    /// The singleton's slot, the one module global these reads reach.
    pub(super) const SINGLETON: usize = IMAGE + cgame_state::SINGLETON_RVA;
    const COUNTRIES: usize = LOW + 0x1000;
    const LINKS: usize = LOW + 0x1100;
    pub(super) const COUNTRY: usize = LOW + 0x2000;
    pub(super) const TASKS: [usize; 2] = [LOW + 0x3000, LOW + 0x3800];
    const DOCTRINE: usize = LOW + 0x4000;
    const EXPERIENCE: usize = LOW + 0x4100;
    const VARIANT: usize = LOW + 0x6000;
    pub(super) const DEFINITION: usize = LOW + 0x7000;
    pub(super) const SHIPS: usize = 12;
    const CLASS: i32 = 7;

    pub(super) fn ship(index: usize) -> usize {
        LOW + 0x8000 + index * 0x1000
    }

    /// Ships 0 to 7 are in the first task force, ships 8 to 11 in the second.
    fn task(index: usize) -> usize {
        usize::from(index >= 8)
    }

    fn task_handle(task: usize) -> u64 {
        ((task as u64 + 1) << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE)
    }

    pub(super) fn census(index: usize) -> cship::Row {
        cship::Row {
            id: ((index as u64 + 1) << 32) | u64::from(cship::HANDLE_TYPE),
            task_force: task_handle(task(index)),
            class_id: CLASS,
            roster_ordinal: index as u32,
            ..Default::default()
        }
    }

    /// The census row of ship `index` in task forces of `per_task` ships
    /// ([`bytes_in_tasks`]).
    pub(super) fn census_in(index: usize, per_task: usize) -> cship::Row {
        cship::Row {
            task_force: task_handle(index / per_task),
            ..census(index)
        }
    }

    /// The world with `ships` ships, each with its pool entry in its own
    /// object's slack.
    pub(super) fn bytes_for(ships: usize) -> Vec<u8> {
        bytes_with(ships, &TASKS, task)
    }

    /// The world with `ships` ships in task forces of `per_task`, laid out
    /// after the ships.
    pub(super) fn bytes_in_tasks(ships: usize, per_task: usize) -> Vec<u8> {
        let tasks: Vec<usize> = (0..ships.div_ceil(per_task))
            .map(|task| LOW + 0x8000 + ships * 0x1000 + task * 0x800)
            .collect();
        bytes_with(ships, &tasks, |index| index / per_task)
    }

    fn bytes_with(ships: usize, tasks: &[usize], task: impl Fn(usize) -> usize) -> Vec<u8> {
        let end = tasks.iter().max().map_or(0, |&last| last + 0x800 - LOW);
        let mut bytes = vec![0; (0x8000 + ships * 0x1000).max(end)];
        let mut put = |at: usize, value: &[u8]| {
            bytes[at - LOW..at - LOW + value.len()].copy_from_slice(value);
        };
        let word = |value: usize| (value as u64).to_le_bytes();
        let inline = |text: &[u8]| {
            let mut header = [0; 32];
            header[..text.len()].copy_from_slice(text);
            header[16..24].copy_from_slice(&(text.len() as u64).to_le_bytes());
            header[24..].copy_from_slice(&15_u64.to_le_bytes());
            header
        };
        let array = |data: usize, count: u32| {
            let mut header = [0; 16];
            header[..8].copy_from_slice(&(data as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        put(GAME + cgame_state::COUNTRIES, &array(COUNTRIES, 2));
        put(COUNTRIES + 8, &word(COUNTRY));
        put(GAME + cgame_state::COUNTRY_LINKS, &array(LINKS, 2));
        put(LINKS + 4, &1_u32.to_le_bytes());
        put(COUNTRY, &word(IMAGE + ccountry::VTABLE_RVA));
        put(COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        for (index, &task) in tasks.iter().enumerate() {
            put(task, &word(IMAGE + ctask_force::VTABLE_RVA));
            put(task + cunit::HANDLE, &task_handle(index).to_le_bytes());
            put(task + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
            // An empty doctrine tree: its head is its own successor.
            put(task + 0x640, &word(DOCTRINE));
        }
        put(DOCTRINE, &word(DOCTRINE));
        put(DOCTRINE + 0x19, &[1]);
        put(VARIANT, &word(IMAGE + cequipment_variant::VTABLE_RVA));
        put(
            VARIANT + 8,
            &((1_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
        );
        put(
            VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
            &11_u32.to_le_bytes(),
        );
        put(VARIANT + cequipment_variant::DEFINITION, &word(DEFINITION));
        put(
            VARIANT + cequipment_variant::CLASSIFIER_FLAGS,
            &(1_u64 << 6).to_le_bytes(),
        );
        put(VARIANT + 0xc4, &3_i32.to_le_bytes());
        put(DEFINITION, &word(IMAGE + cequipment_type::VTABLE_RVA));
        put(DEFINITION + cequipment_type::TOKEN, &11_u32.to_le_bytes());
        put(DEFINITION + cequipment_type::HULL_SPRITE, &inline(b"hull"));
        for index in 0..ships {
            let ship = ship(index);
            let bank = ship + 0x980;
            put(ship, &word(IMAGE + cship::VTABLE_RVA));
            put(ship + cship::HANDLE, &census(index).id.to_le_bytes());
            put(ship + cship::TASK_FORCE, &word(tasks[task(index)]));
            put(
                ship + cship::EQUIPMENT_POOL,
                &word(IMAGE + cequipment_variant_pool::VTABLE_RVA),
            );
            put(
                ship + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
                &array(bank, 1),
            );
            put(bank, &word(VARIANT));
            put(bank + 8, &5_i64.to_le_bytes());
            put(
                ship + cship::SUBUNIT_DEFINITION,
                &word(IMAGE + csub_unit_definition::VTABLE_RVA),
            );
            put(ship + cship::CLASS_DATABASE_ID, &CLASS.to_le_bytes());
            put(ship + cship::DEFINITION_VALID, &[1]);
            put(ship + cship::FALLBACK_SPRITE, &inline(b"ship"));
            put(
                ship + cship::ANTI_AIR_BASE,
                &(1000 + index as i64).to_le_bytes(),
            );
            put(
                ship + cship::BASE_ORGANISATION,
                &(10_000_000 + index as i64 * 100_000).to_le_bytes(),
            );
            put(ship + cship::MAXIMUM_STRENGTH, &5_000_000_i64.to_le_bytes());
            put(ship + cship::SUBUNIT_INDEX, &(index as i32).to_le_bytes());
            put(ship + cship::MODEL_NUMBER, &(index as i32).to_le_bytes());
            // Ship modifier's experience modifier source.
            put(ship + 0x8f8, &word(EXPERIENCE));
        }
        bytes
    }
}

/// [`fleet`]'s memory: one guarded call per copy, scattered read and record
/// gather.
struct Fleet {
    low: Vec<u8>,
    calls: Cell<usize>,
    /// Reads starting at `watched`, counted in `hits`.
    watched: Cell<usize>,
    hits: Cell<usize>,
}

impl Fleet {
    fn new() -> Self {
        Self::with_ships(fleet::SHIPS)
    }

    fn with_ships(ships: usize) -> Self {
        Self {
            low: fleet::bytes_for(ships),
            calls: Cell::new(0),
            watched: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    fn in_tasks(ships: usize, per_task: usize) -> Self {
        Self {
            low: fleet::bytes_in_tasks(ships, per_task),
            calls: Cell::new(0),
            watched: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    fn read(&self, address: usize, output: &mut [u8]) -> bool {
        if address == self.watched.get() {
            self.hits.set(self.hits.get() + 1);
        }
        if address == fleet::SINGLETON && output.len() == 8 {
            output.copy_from_slice(&(fleet::GAME as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: fleet::LOW,
            bytes: &self.low,
        }
        .copy(address, output)
    }
}

impl Memory for Fleet {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.read(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty() && !self.read(address, destination) {
                return completed;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        // The fields are copied one by one, but the call is one.
        let calls = self.calls.get() + 1;
        let completed = Scalar(self).copy_records(sources, layout, destination);
        self.calls.set(calls);
        completed
    }
}

/// The detail capture reads each ship through its gathered fields
/// (`cship::ShipFields`): its rows are the direct readers' over the same
/// memory, in under half their guarded calls. Reading every field singly, as
/// the readers do directly, makes as many calls as they do.
#[test]
fn ship_model_details_equal_the_direct_readers_in_under_half_their_guarded_calls() {
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    capture.begin_sources();
    for index in 0..fleet::SHIPS {
        capture.visit_source(fleet::ship(index), &fleet::census(index));
    }
    capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
    capture.capture_owned(&world, CaptureContext::new(fleet::IMAGE));
    assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    let gathered = world.calls.replace(0);
    // The direct readers, as the detail capture runs them for each ship.
    let mut direct = Captured::part(&capacities(64, 64));
    direct.begin_details();
    let mut rows = Vec::new();
    for index in 0..fleet::SHIPS {
        let ship = fleet::ship(index);
        let census = fleet::census(index);
        let (mut row, selected) = {
            let memory = ModifierMemory {
                inner: &world,
                arena: &direct.modifiers,
            };
            let (mut row, selected) = cship::read_model_with_selection(
                &memory,
                fleet::IMAGE,
                ship,
                &census,
                &mut direct.model,
            )
            .unwrap();
            row.maximum_organisation = cship::read_maximum_organisation_memo(
                &memory,
                fleet::IMAGE,
                ship,
                &mut direct.doctrine,
            )
            .unwrap();
            (row, selected)
        };
        direct.capture_model_suffixes(
            &world,
            fleet::IMAGE,
            ship,
            &census,
            selected.as_ref(),
            &mut row,
        );
        rows.push(row);
    }
    let read = world.calls.get();
    assert_eq!(capture.rows.rows[..fleet::SHIPS], rows[..]);
    assert_eq!(rows[11].maximum_organisation, 11_100_000);
    assert_eq!(rows[2].subunit_index, Some(2));
    assert!(
        gathered * 2 < read,
        "{gathered} guarded calls through gathered fields, {read} directly"
    );
}

/// A ship whose handle, task force and certificate are those of its held
/// read is served from it: its row is the fresh read's, without reading the
/// ship's own objects. A notified ship is read again and shows its change; an
/// unnotified change stays carried until a verifying hour reads it afresh,
/// publishes it and poisons the family.
#[test]
fn held_ships_are_carried_until_a_writer_touches_them() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured, world: &Fleet, check: Check| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        world.calls.set(0);
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches, check);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        (
            capture.rows.rows[..fleet::SHIPS].to_vec(),
            world.calls.get(),
        )
    };
    let base_organisation = |world: &mut Fleet, index: usize, value: i64| {
        let at = fleet::ship(index) + cship::BASE_ORGANISATION - fleet::LOW;
        world.low[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    let (fresh, read) = hour(&mut capture, &world, Check::Nothing);
    let (carried, served) = hour(&mut capture, &world, Check::Nothing);
    assert_eq!(carried, fresh);
    // A carried ship gathers neither its fields nor its pool entries.
    assert!(
        read >= served + 2 * fleet::SHIPS,
        "{served} guarded calls carried, {read} read"
    );

    // A task force whose doctrine tree writer ran has its doctrine walked
    // again; the other's contribution stays served.
    drop(watches.notify(fleet::TASKS[0], 6));
    let (rows, rewalked) = hour(&mut capture, &world, Check::Nothing);
    assert_eq!(rows, fresh);
    assert!(
        rewalked >= served + 4,
        "{rewalked} guarded calls after the doctrine writer, {served} quiet"
    );

    base_organisation(&mut world, 3, 20_000_000);
    drop(watches.notify(fleet::ship(3), 0));
    let (rows, _) = hour(&mut capture, &world, Check::Nothing);
    assert_ne!(rows[3].maximum_organisation, fresh[3].maximum_organisation);
    assert_eq!(rows[..3], fresh[..3]);

    base_organisation(&mut world, 4, 30_000_000);
    let (carried, _) = hour(&mut capture, &world, Check::Nothing);
    assert_eq!(carried[4], fresh[4], "no writer was observed");
    assert!(!watches.poisoned());
    let (verified, _) = hour(&mut capture, &world, Check::Every);
    assert_ne!(verified[4], fresh[4], "the fresh read is published");
    assert!(watches.poisoned());
}

/// A carried ship keeps its held row's model part even when its task
/// force's terms are read with it (it is the task force's first ship in the
/// range): the model reader and its suffixes read only the ship's certified
/// inputs, its variants and the session's fixed content, so they do not run,
/// and a hull definition changed under them is not read again until a
/// verifying hour reads the ship afresh.
#[test]
fn a_carried_ships_model_part_is_not_read_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let mut hour = |world: &Fleet, check: Check| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches, check);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&world, Check::Nothing);
    assert_eq!(fresh[0].hull_sprite.as_bytes(), b"hull");
    world.low[fleet::DEFINITION + engine::cequipment_type::HULL_SPRITE - fleet::LOW] = b'w';
    assert_eq!(hour(&world, Check::Nothing), fresh);
    let verified = hour(&world, Check::Every);
    assert_eq!(verified[0].hull_sprite.as_bytes(), b"wull");
    assert_eq!(verified[8].hull_sprite.as_bytes(), b"wull");
}

/// A ship a hooked writer touched is gathered again; when its own objects
/// and modifier arrays equal its held read's, it carries again under its new
/// certificate (its model part is not read again), and when a byte changed
/// it is read afresh.
#[test]
fn a_touched_ship_with_unchanged_bytes_carries_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let mut hour = |world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(
            world,
            CaptureContext::new(fleet::IMAGE),
            &watches,
            Check::Nothing,
        );
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&world);
    world.low[fleet::DEFINITION + engine::cequipment_type::HULL_SPRITE - fleet::LOW] = b'w';
    drop(watches.notify(fleet::ship(3), 0));
    assert_eq!(
        hour(&world),
        fresh,
        "the touched ship's bytes are unchanged"
    );
    let at = fleet::ship(3) + cship::BASE_ORGANISATION - fleet::LOW;
    world.low[at..at + 8].copy_from_slice(&20_000_000_i64.to_le_bytes());
    drop(watches.notify(fleet::ship(3), 0));
    let read = hour(&world);
    assert_ne!(read[3].maximum_organisation, fresh[3].maximum_organisation);
    assert_eq!(read[3].hull_sprite.as_bytes(), b"wull");
    assert_eq!(read[..3], fresh[..3]);
}

/// A carried row the capture's buffer already holds at its position, from
/// the hour it held the read, is published without rewriting it (a marker
/// planted in the buffer survives); a ship read afresh is rewritten.
#[test]
fn a_held_row_in_the_buffer_is_not_rewritten() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured, world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(
            world,
            CaptureContext::new(fleet::IMAGE),
            &watches,
            Check::Nothing,
        );
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    };
    hour(&mut capture, &world);
    hour(&mut capture, &world);
    const MARK: i32 = 0x5eed;
    capture.rows.rows[5].model_number = MARK;
    hour(&mut capture, &world);
    assert_eq!(capture.rows.rows[5].model_number, MARK, "carried in place");
    let at = fleet::ship(5) + cship::BASE_ORGANISATION - fleet::LOW;
    world.low[at..at + 8].copy_from_slice(&20_000_000_i64.to_le_bytes());
    drop(watches.notify(fleet::ship(5), 0));
    hour(&mut capture, &world);
    assert_eq!(capture.rows.rows[5].model_number, 5, "read afresh");
}

/// A task force's first ship carries its finished row too: the task force's
/// terms are read without it, so a quiet hour runs no reader for it and its
/// row stays in the buffer (a planted marker survives).
#[test]
fn a_task_forces_first_ship_carries_its_row() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(
            &world,
            CaptureContext::new(fleet::IMAGE),
            &watches,
            Check::Nothing,
        );
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&mut capture);
    assert_eq!(hour(&mut capture), fresh);
    const MARK: i32 = 0x5eed;
    for first in [0, 8] {
        capture.rows.rows[first].model_number = MARK;
    }
    let rows = hour(&mut capture);
    assert_eq!((rows[0].model_number, rows[8].model_number), (MARK, MARK));
    assert_eq!(rows[1..8], fresh[1..8]);
}

/// A task force's country terms (its modifier tree, `CCountry+0x5B8`) are
/// served from the held hour while the country's naval tree certificate
/// holds: a quiet hour reads no country accumulator, and an hour after a
/// tree writer touched the country reads it again.
#[test]
fn held_country_terms_are_not_read_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    world.watched.set(fleet::COUNTRY + 0x5b8);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(
            &world,
            CaptureContext::new(fleet::IMAGE),
            &watches,
            Check::Nothing,
        );
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    };
    hour(&mut capture);
    assert!(world.hits.get() > 0, "the first hour reads the country");
    hour(&mut capture);
    world.hits.set(0);
    hour(&mut capture);
    assert_eq!(world.hits.get(), 0, "a quiet hour serves the held terms");
    drop(watches.notify(fleet::COUNTRY, 0));
    hour(&mut capture);
    assert!(world.hits.get() > 0, "a touched country is read again");
}

/// Compute per ship, in warm memory, over 2,048 ships: the median wall time
/// of a range's hour read afresh (every ship compared) and with finished rows
/// carried (all but each task force's first ship), in two task forces and in
/// task forces of three ships (as at 1943), the latter also with each
/// first ship's model part read again (no model carry: such a ship runs every
/// reader; later ships' rows are still carried). The live cost per
/// ship minus these is game-memory latency. Run with
/// `cargo test --release -p collector ship_model_compute_bench -- --ignored --nocapture`,
/// and under a profiler to see the hot functions.
#[test]
#[ignore]
fn ship_model_compute_bench() {
    use crate::ship_model_changes::Watches;
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    // (label, ships per task force, check, model carry, every ship touched
    // by a writer that changed no byte)
    for (label, per_task, check, model_carry, touched) in [
        ("read afresh", 1024, Check::Every, true, false),
        ("rows carried", 1024, Check::Nothing, true, false),
        (
            "three per task force, read afresh",
            3,
            Check::Every,
            true,
            false,
        ),
        (
            "three per task force, rows carried",
            3,
            Check::Nothing,
            true,
            false,
        ),
        (
            "three per task force, no model carry",
            3,
            Check::Nothing,
            false,
            false,
        ),
        (
            "three per task force, every ship touched",
            3,
            Check::Nothing,
            true,
            true,
        ),
    ] {
        let world = Fleet::in_tasks(SHIPS, per_task);
        let census: Vec<_> = (0..SHIPS)
            .map(|index| fleet::census_in(index, per_task))
            .collect();
        let watches = Watches::enabled(4 * SHIPS);
        let mut capture = Captured::with_capacities(&Capacities {
            rows: SHIPS,
            modifiers: SnapshotCapacity {
                nodes: 4 * SHIPS,
                entries: 4 * SHIPS,
            },
        });
        super::MODEL_CARRY_OFF.set(!model_carry);
        let mut samples: Vec<f64> = (0..HOURS)
            .map(|_| {
                capture.begin_sources();
                for (index, source) in census.iter().enumerate() {
                    capture.visit_source(fleet::ship(index), source);
                }
                capture.finish_sources(Outcome::Rows { len: SHIPS });
                if touched {
                    for index in 0..SHIPS {
                        drop(watches.notify(fleet::ship(index), 0));
                    }
                }
                let started = std::time::Instant::now();
                capture.capture_carrying(
                    &world,
                    CaptureContext::new(fleet::IMAGE),
                    &watches,
                    check,
                );
                assert_eq!(capture.outcomes(), [Outcome::Rows { len: SHIPS }]);
                started.elapsed().as_nanos() as f64 / SHIPS as f64
            })
            .collect();
        super::MODEL_CARRY_OFF.set(false);
        samples.sort_by(f64::total_cmp);
        println!(
            "BENCH ship models, {label}: median {:.0} ns per ship",
            samples[HOURS / 2]
        );
    }
}

/// Where a fresh ship's compute goes, in warm memory over 2,048 ships: the
/// median ns per ship of gathering its fields, then of the model reader and
/// the organisation reader on top, and of the whole detail capture without
/// a store. Run like `ship_model_compute_bench`.
#[test]
#[ignore]
fn ship_model_phase_bench() {
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    let world = Fleet::with_ships(SHIPS);
    let census: Vec<_> = (0..SHIPS).map(fleet::census).collect();
    let median = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let mut direct = Captured::part(&capacities(4 * SHIPS, 4 * SHIPS));
    let mut phase = |depth: u8| {
        median(
            (0..HOURS)
                .map(|_| {
                    direct.begin_details();
                    let mut fields = cship::ShipFields::new(fleet::IMAGE);
                    let started = std::time::Instant::now();
                    for (index, source) in census.iter().enumerate() {
                        let ship = fleet::ship(index);
                        fields.visit(&world, ship);
                        if depth == 0 {
                            continue;
                        }
                        let gathered = fields.memory(&world);
                        let memory = ModifierMemory {
                            inner: &gathered,
                            arena: &direct.modifiers,
                        };
                        let model = cship::read_model_with_selection(
                            &memory,
                            fleet::IMAGE,
                            ship,
                            source,
                            &mut direct.model,
                        );
                        assert!(model.is_ok());
                        if depth == 2 {
                            assert!(
                                cship::read_maximum_organisation_memo(
                                    &memory,
                                    fleet::IMAGE,
                                    ship,
                                    &mut direct.doctrine,
                                )
                                .is_ok()
                            );
                        }
                    }
                    started.elapsed().as_nanos() as f64 / SHIPS as f64
                })
                .collect(),
        )
    };
    let gather = phase(0);
    let model = phase(1);
    let organisation = phase(2);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 4 * SHIPS,
            entries: 4 * SHIPS,
        },
    });
    let whole = median(
        (0..HOURS)
            .map(|_| {
                capture.begin_sources();
                for (index, source) in census.iter().enumerate() {
                    capture.visit_source(fleet::ship(index), source);
                }
                capture.finish_sources(Outcome::Rows { len: SHIPS });
                let started = std::time::Instant::now();
                capture.capture_owned(&world, CaptureContext::new(fleet::IMAGE));
                started.elapsed().as_nanos() as f64 / SHIPS as f64
            })
            .collect(),
    );
    println!(
        "BENCH ship model phases, ns per ship: gather {gather:.0}, + model reader {model:.0}, + organisation reader {organisation:.0}, whole capture without a store {whole:.0}"
    );
}

/// What a quiet hour pays per ship, in warm memory over 2,048 ships in task
/// forces of three with every row carried but each task force's first: the
/// census visit, the three certificate takes, and the whole capture. Run like
/// `ship_model_compute_bench`.
#[test]
#[ignore]
fn ship_model_quiet_bench() {
    use crate::ship_model_changes::{Changes, Watches};
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    let world = Fleet::in_tasks(SHIPS, 3);
    let census: Vec<_> = (0..SHIPS).map(|index| fleet::census_in(index, 3)).collect();
    let watches = Watches::enabled(4 * SHIPS);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 4 * SHIPS,
            entries: 4 * SHIPS,
        },
    });
    let median = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let per_ship = |started: std::time::Instant| started.elapsed().as_nanos() as f64 / SHIPS as f64;
    let (mut census_ns, mut capture_ns, mut certificate_ns) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..HOURS {
        let started = std::time::Instant::now();
        capture.begin_sources();
        for (index, source) in census.iter().enumerate() {
            capture.visit_source(fleet::ship(index), source);
        }
        capture.finish_sources(Outcome::Rows { len: SHIPS });
        census_ns.push(per_ship(started));
        let started = std::time::Instant::now();
        capture.capture_carrying(
            &world,
            CaptureContext::new(fleet::IMAGE),
            &watches,
            Check::Nothing,
        );
        capture_ns.push(per_ship(started));
        let started = std::time::Instant::now();
        for index in 0..SHIPS {
            std::hint::black_box(watches.certificate([fleet::ship(index), 0, 0]));
        }
        certificate_ns.push(per_ship(started));
    }
    println!(
        "BENCH ship models quiet hour, ns per ship: census {:.0}, certificate take {:.0}, capture {:.0}",
        median(census_ns),
        median(certificate_ns),
        median(capture_ns)
    );
}
