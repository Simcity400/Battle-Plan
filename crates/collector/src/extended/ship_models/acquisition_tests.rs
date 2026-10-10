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
        strike_modifiers: SnapshotCapacity::default(),
        strike_metadata: 0,
    }
}

fn transport_context(tick_busy: u8) -> CaptureContext {
    let (base, game_state, hour) = (0, 0x9000, 77);
    let clock = engine::cgame_state::Clock {
        game_state: game_state as u64,
        hour,
        tick_busy,
        play_time: 0,
        play_session_started_at: 7,
        seed: 0,
        campaign_id: engine::capture_text::Owned::default(),
        speed: 0,
    };
    let _scope = crate::transport_admission::Scope::enter_fixture(base, game_state, Some(clock));
    CaptureContext::current(base, game_state, hour)
}

/// The old model selects this nonpositive entry, then refuses its sprite.
/// Its flags and carrier base are never demanded by the old reader.
fn transport_model_source() -> (Source, cship::Row) {
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

#[test]
fn refused_transport_model_admission_preserves_every_legacy_read_and_has_no_journal() {
    let admitted = transport_context(0);
    // The copied verdict remains usable after its callback TLS scope retires.
    assert_eq!(admitted.check_transport(), Ok(()));
    let mut contexts = vec![
        CaptureContext::legacy(0),
        transport_context(1),
        CaptureContext::current(admitted.base, admitted.game_state, admitted.hour),
    ];
    for field in 0..3 {
        let mut context = admitted;
        match field {
            0 => context.base += 1,
            1 => context.game_state += 1,
            2 => context.hour += 1,
            _ => unreachable!(),
        }
        contexts.push(context);
    }
    for context in contexts {
        assert!(context.check_transport().is_err());
        let (reference, row) = transport_model_source();
        let reason = cship::read_model_with_selection(
            &reference,
            context.base,
            BASE,
            &row,
            &mut cship::ModelMemo::default(),
        )
        .err()
        .expect("old model prefix must refuse");
        let (source, row) = transport_model_source();
        let mut capture = Captured::with_capacities(&capacities(0, 0));
        capture.begin_sources();
        capture.visit_source(BASE, &row);
        capture.finish_sources(Outcome::Rows { len: 1 });
        capture.capture_owned(&source, context);
        assert_eq!(capture.outcomes(), [Outcome::Unavailable { reason }]);
        assert_eq!(*source.reads.borrow(), *reference.reads.borrow());
        assert!(!source.reads.borrow().iter().any(|&(at, _)| at
            == 0x3000 + engine::cequipment_variant::CLASSIFIER_FLAGS
            || at == BASE + cship::CARRIER_AIR_BASE));
        assert_eq!(capture.transport_models.len, 0);
        assert_eq!(capture.transport_models.required, 0);
        let mut risk = super::super::transport_risk_ships::Captured::new(0, 1);
        risk.begin_models(context.hour);
        assert_eq!(
            capture.lend_transport_models(
                &mut risk,
                context.base,
                context.game_state,
                context.hour
            ),
            Err("transport original ship-model callback journal not admitted")
        );
    }
    let (source, row) = transport_model_source();
    let mut capture = Captured::with_capacities(&capacities(0, 0));
    capture.begin_sources();
    capture.visit_source(BASE, &row);
    capture.finish_sources(Outcome::Rows { len: 1 });
    capture.capture_owned(&source, admitted);
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "ship model sprite exceeds owned capacity"
        }]
    );
    assert!(
        source
            .reads
            .borrow()
            .contains(&(0x3000 + engine::cequipment_variant::CLASSIFIER_FLAGS, 8))
    );
    assert_eq!(capture.transport_models.len, 1);
    assert_eq!(
        capture.transport_models.first[0]
            .observation
            .positive_equipment_flags,
        Some(Ok(0))
    );
}

#[test]
fn empty_model_journal_requires_copied_admission_in_every_split_and_exact_loan_key() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("empty source has no native reads")
        }
    }
    let context = transport_context(0);
    let mut capture = Captured::with_capacities(&capacities(0, 0));
    capture.begin_sources();
    capture.finish_sources(Outcome::Rows { len: 0 });
    capture.set_context(context);
    let mut risk = super::super::transport_risk_ships::Captured::new(0, 0);
    risk.begin_models(context.hour);
    assert!(
        capture
            .lend_transport_models(&mut risk, context.base, context.game_state, context.hour)
            .is_err(),
        "a census context is not a completed detail journal"
    );
    assert_eq!(capture.split_details(MAX_PARTS), MAX_PARTS);
    let mut parts: Vec<_> = (0..MAX_PARTS - 1)
        .map(|index| capture.take_part(index))
        .collect();
    capture.capture_owned(&NoReads, capture.capture_context(context.base));
    for part in &mut parts {
        let copied = part.capture_context(context.base);
        assert_eq!(
            (copied.base, copied.game_state, copied.hour),
            (context.base, context.game_state, context.hour)
        );
        assert_eq!(copied.check_transport(), Ok(()));
        part.capture_owned(&NoReads, copied);
    }
    for (index, part) in parts.into_iter().enumerate() {
        capture.restore_part(index, part);
    }
    capture.finish_split();
    assert_eq!(capture.outcomes(), [Outcome::Rows { len: 0 }]);
    assert_eq!(
        capture.lend_transport_models(&mut risk, context.base, context.game_state, context.hour),
        Ok(())
    );
    for key in [
        (1, context.game_state, context.hour),
        (0, context.game_state + 1, context.hour),
        (0, context.game_state, context.hour + 1),
    ] {
        assert_eq!(
            capture.lend_transport_models(&mut risk, key.0, key.1, key.2),
            Err("transport original ship-model callback journal base world or hour mismatch")
        );
    }
    capture.extra[1].transport_models_admitted = false;
    assert_eq!(
        capture.lend_transport_models(&mut risk, context.base, context.game_state, context.hour),
        Err("transport original ship-model callback journal not admitted"),
        "every part must admit even an empty journal"
    );
    capture.reset();
    assert!(capture.context.is_none());
    assert!(
        capture
            .lend_transport_models(&mut risk, context.base, context.game_state, context.hour)
            .is_err()
    );
    capture.begin_sources();
    capture.finish_sources(Outcome::Rows { len: 0 });
    capture.capture_owned(&NoReads, CaptureContext::legacy(0));
    assert_eq!(capture.outcomes(), [Outcome::Rows { len: 0 }]);
    assert_eq!(
        capture.lend_transport_models(&mut risk, 0, 0, 0),
        Err("transport original ship-model callback journal not admitted")
    );
}

#[test]
fn original_model_journal_loans_five_thousand_attempts_to_empty_risk_storage_without_allocation() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("ownership loan must not acquire any native input")
        }
    }
    let context = transport_context(0);
    let mut capacity = capacities(0, 0);
    capacity.rows = 5000;
    let mut capture = Captured::with_capacities(&capacity);
    capture.context = Some(context);
    capture.transport_models_admitted = true;
    for index in 0..5000 {
        capture.transport_models.first[index] = TransportModel {
            ship: ((index as u64 + 1) << 32) | u64::from(cship::HANDLE_TYPE),
            observation: cship::TransportModelObservation {
                identity: Some(Err("original identity failed")),
                ..Default::default()
            },
        };
    }
    capture.transport_models.len = 5000;
    capture.transport_models.required = 5000;
    let mut risk = super::super::transport_risk_ships::Captured::new(0, 0);
    risk.begin_models(context.hour);
    crate::allocation_failure_tests::with_allocations_denied(|| {
        assert_eq!(
            capture.lend_transport_models(
                &mut risk,
                context.base,
                context.game_state,
                context.hour
            ),
            Ok(())
        );
    });
    assert_eq!(capture.transport_models.len, 0);
    assert!(risk.observer(&NoReads, context.base, context.hour).is_ok());
    assert_eq!(
        risk.packet(context.hour).unwrap().unwrap().receivers,
        Some(vec![])
    );
    capture.begin_sources();
    capture.finish_sources(Outcome::Rows { len: 0 });
    assert!(capture.transport_models.first.len() >= capacity.rows);
}

fn acquire(capture: &mut Captured, source: &Source) -> Result<(), &'static str> {
    capture.begin_details(0);
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
            + capture.transport_models.allocated_bytes()
            + capture.seeds.capacity() * std::mem::size_of::<Seed>()
            + capture.modifiers.borrow().allocated_bytes()
            + capture.strike_modifiers.borrow().allocated_bytes()
            + capture.strike_metadata.allocated_bytes()
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

#[test]
fn strike_reuses_core_success_without_mutating_its_observation() {
    let source = Source::new(2);
    let core = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 1,
            entries: 2,
        })
        .unwrap(),
    );
    core.borrow_mut().capture_at(&source, BASE, 1).unwrap();
    let reads = source.reads.borrow().len();
    let required = core.borrow().required_capacity();
    let suffix = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let used = Cell::new((0, 0));
    let bounded = BoundedMemory {
        inner: &source,
        used: &used,
    };
    let memory = StrikeMemory {
        inner: &bounded,
        core: &core,
        suffix: &suffix,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 2)
            .unwrap()
            .raw,
        -13
    );
    assert_eq!(source.reads.borrow().len(), reads);
    assert_eq!(used.get(), (0, 0));
    assert_eq!(core.borrow().required_capacity(), required);
    assert_eq!(
        suffix.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
}

#[test]
fn strike_cannot_retry_core_native_failure_but_may_use_its_own_storage() {
    let source = Source::new(2);
    let core = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 1,
            entries: 2,
        })
        .unwrap(),
    );
    source.fault.set(Some(ENTRIES));
    assert!(core.borrow_mut().capture_at(&source, BASE, 1).is_err());
    source.fault.set(None);
    let reads = source.reads.borrow().len();
    let suffix = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 1,
            entries: 2,
        })
        .unwrap(),
    );
    let used = Cell::new((0, 0));
    let bounded = BoundedMemory {
        inner: &source,
        used: &used,
    };
    let memory = StrikeMemory {
        inner: &bounded,
        core: &core,
        suffix: &suffix,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(source.reads.borrow().len(), reads);
    assert_eq!(used.get(), (0, 0));
    assert_eq!(
        suffix.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
    // A core storage admission refusal is not a native source failure.
    let short_core = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    assert_eq!(
        short_core.borrow_mut().capture_at(&source, BASE, 1).err(),
        Some(SNAPSHOT_NODE_CAPACITY)
    );
    let memory = StrikeMemory {
        inner: &bounded,
        core: &short_core,
        suffix: &suffix,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 2)
            .unwrap()
            .raw,
        -13
    );
    assert_eq!(short_core.borrow().capacity(), SnapshotCapacity::default());
    assert_eq!(
        short_core.borrow().required_capacity(),
        SnapshotCapacity {
            nodes: 1,
            entries: 0
        }
    );
}

#[test]
fn fixed_strike_storage_refusal_never_enters_direct_fallback() {
    let source = Source::new(2);
    let core = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let suffix = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 1,
            entries: 1,
        })
        .unwrap(),
    );
    let used = Cell::new((0, 0));
    let bounded = BoundedMemory {
        inner: &source,
        used: &used,
    };
    let memory = StrikeMemory {
        inner: &bounded,
        core: &core,
        suffix: &suffix,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err(SNAPSHOT_ENTRY_CAPACITY)
    );
    assert_eq!(*source.reads.borrow(), [(BASE, 16)]);
    // Native repair cannot heal this pass's stored suffix refusal.
    source.array(BASE, ENTRIES, 1);
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err(SNAPSHOT_ENTRY_CAPACITY)
    );
    assert_eq!(*source.reads.borrow(), [(BASE, 16)]);
    assert_eq!(
        core.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
}

#[test]
fn worker_strike_growth_preserves_original_faults_bases_and_core_budget() {
    let source = Source::new(130);
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details(0);
    source.fault.set(Some(ENTRIES + 64 * 16));
    let memory = StrikeMemory {
        inner: &source,
        core: &capture.modifiers,
        suffix: &capture.strike_modifiers,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err("modifier accumulator entry unreadable")
    );
    let reads = source.reads.borrow().len();
    source.fault.set(None);
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 130),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(source.reads.borrow().len(), reads);
    source.array(BASE + 0x20, ENTRIES, 130);
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE + 0x20, 130)
            .unwrap()
            .raw,
        129 * -13
    );
    // Node-index growth must keep the first failed source's observation.
    let reads = source.reads.borrow().len();
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(source.reads.borrow().len(), reads);
    assert_eq!(capture.copied.get(), (0, 0));
    assert_eq!(
        capture.modifiers.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
    for (offset, value) in [
        (0x1e8, i64::MIN),
        (0x1f8, 0),
        (0x210, i64::MAX),
        (0x208, -13),
    ] {
        source.put(BASE + offset, &value.to_le_bytes());
    }
    assert_eq!(
        cship::read_strike_bases(&source, BASE),
        [Some(i64::MIN), Some(0), Some(i64::MAX), Some(-13)]
    );
    assert_eq!(
        source.reads.borrow()[reads..],
        [
            (BASE + 0x1e8, 8),
            (BASE + 0x1f8, 8),
            (BASE + 0x210, 8),
            (BASE + 0x208, 8)
        ]
    );
    let bounded = BoundedMemory {
        inner: &source,
        used: &capture.copied,
    };
    assert_eq!(engine::read::<i64>(&bounded, BASE + 0x208), Some(-13));
    assert_eq!(capture.copied.get(), (1, 8));
}

#[test]
fn worker_strike_native_demand_above_former_entry_and_read_ceilings_is_complete() {
    struct LargeAccumulator {
        count: u32,
        headers: Cell<usize>,
        entries: Cell<usize>,
    }
    impl Memory for LargeAccumulator {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert_eq!(
                (address, output.len()),
                (BASE, 16),
                "no direct retry census"
            );
            self.headers.set(self.headers.get() + 1);
            output[..8].copy_from_slice(&(ENTRIES as u64).to_le_bytes());
            output[8..12].copy_from_slice(&self.count.to_le_bytes());
            output[12..].copy_from_slice(&self.count.to_le_bytes());
            true
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            output: &mut [u8],
        ) -> usize {
            let RecordSources::Strided {
                base,
                stride,
                count,
            } = sources
            else {
                panic!("native accumulator has a strided extent")
            };
            assert_eq!(stride, 16);
            assert_eq!(layout.stride(), 16);
            assert_eq!(
                layout.fields(),
                &[RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 16
                }]
            );
            assert_eq!(output.len(), count * 16);
            let first = (base - ENTRIES) / 16;
            assert!(first + count <= self.count as usize);
            for (index, entry) in output.chunks_exact_mut(16).enumerate() {
                let key = u32::try_from(first + index + 1).unwrap();
                entry.fill(0);
                entry[..4].copy_from_slice(&key.to_le_bytes());
                entry[8..].copy_from_slice(&(i64::from(key) * -13).to_le_bytes());
            }
            self.entries.set(self.entries.get() + count);
            count
        }
    }
    let count = u32::try_from(SNAPSHOT_LIMITS.entries + 1).unwrap();
    assert!(count as usize > READ_LIMIT);
    let source = LargeAccumulator {
        count,
        headers: Cell::new(0),
        entries: Cell::new(0),
    };
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details(0);
    let memory = StrikeMemory {
        inner: &source,
        core: &capture.modifiers,
        suffix: &capture.strike_modifiers,
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, count).unwrap(),
        Value {
            raw: i64::from(count) * -13,
            present: true
        }
    );
    assert_eq!(
        (source.headers.get(), source.entries.get()),
        (1, count as usize)
    );
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1)
            .unwrap()
            .raw,
        -13
    );
    assert_eq!(
        (source.headers.get(), source.entries.get()),
        (1, count as usize)
    );
    assert_eq!(capture.copied.get(), (0, 0));
    capture.release_sources();
    assert_eq!(
        capture.required_strike_modifiers,
        SnapshotCapacity {
            nodes: 1,
            entries: count as usize
        }
    );
    let mut capacity = capacities(0, 0);
    capture.grow(&mut capacity);
    assert!(capacity.strike_modifiers.entries >= count as usize);
    assert!(capture.strike_modifiers.borrow().capacity().entries >= count as usize);
    // Construction must also retain a previously observed demand above the old clamp.
    let prepared = Captured::part(&capacity);
    assert!(prepared.strike_modifiers.borrow().capacity().entries >= count as usize);
}

struct OptionalModelSource(Source);
impl OptionalModelSource {
    fn new() -> (Self, cship::Row) {
        use engine::{cequipment_database, cequipment_type, cequipment_variant, msvc_string};
        let (source, row) = transport_model_source();
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
        capture.begin_details(0);
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
        let status = serde_json::to_value(strike_data()).unwrap();
        assert!(
            status["optional_operands"][OPTIONAL_REFUSAL_REASONS[index]]
                .as_u64()
                .unwrap()
                > 0
        );
    }
}

#[test]
fn current_hull_packet_and_field_refusals_are_named_and_preserve_independent_operands() {
    let (healthy, census) = OptionalModelSource::new();
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details(0);
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
        capture.begin_details(0);
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
    capture.begin_details(0);
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
        capture.begin_details(0);
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
fn parts_keep_full_metadata_floor_and_idle_growth_accounts_every_suffix() {
    let mut capacity = Capacities {
        rows: 8,
        modifiers: SnapshotCapacity::default(),
        strike_modifiers: SnapshotCapacity {
            nodes: 2,
            entries: 4,
        },
        strike_metadata: 0x300,
    };
    let mut capture = Captured::with_capacities(&capacity);
    let full = capture.strike_metadata.allocated_bytes();
    assert!(full > 0);
    assert!(
        capture
            .extra
            .iter()
            .all(|part| part.strike_metadata.allocated_bytes() == full)
    );
    let before = capture.allocated_bytes();
    // No child pointer or payload is consulted while raising the storage floor.
    capacity.raise_for_strike_metadata(0x301);
    capacity.raise_for_strike_metadata(0);
    assert_eq!(capacity.strike_metadata, 0x301);
    capture.grow(&mut capacity);
    assert!(capture.allocated_bytes() > before);
    assert!(
        capture
            .extra
            .iter()
            .all(|part| part.strike_metadata.allocated_bytes()
                == capture.strike_metadata.allocated_bytes())
    );
    let before = capture.allocated_bytes();
    capture.begin_details(0x140000000);
    capture.release_sources();
    assert_eq!(capture.allocated_bytes(), before);
    assert_eq!(
        capture.strike_modifiers.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
}

#[test]
fn cold_ship_model_modifier_arenas_grow_from_original_mod_sized_headers() {
    let mut capture = Captured::part(&Capacities::default());
    assert_eq!(capture.modifiers.borrow().allocated_bytes(), 0);
    assert_eq!(capture.strike_modifiers.borrow().allocated_bytes(), 0);
    for count in [5_000_u32, 15_000] {
        let source = Source::new(count);
        source.array(BASE + 0x20, ENTRIES, count);
        capture.worker_growth = true;
        capture.begin_details(0);
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
        let suffix = StrikeMemory {
            inner: &source,
            core: &capture.modifiers,
            suffix: &capture.strike_modifiers,
        };
        assert_eq!(
            modifier_accumulator::lookup_at(&suffix, BASE + 0x20, count)
                .unwrap()
                .raw,
            i64::from(count - 1) * -13
        );
        assert!(capture.modifiers.borrow().capacity().entries >= count as usize);
        assert!(capture.strike_modifiers.borrow().capacity().entries >= count as usize);
        let reads = source.reads.borrow().len();
        assert_eq!(
            modifier_accumulator::lookup_at(&core, BASE, 1).unwrap().raw,
            0
        );
        assert_eq!(
            modifier_accumulator::lookup_at(&suffix, BASE + 0x20, 1)
                .unwrap()
                .raw,
            0
        );
        assert_eq!(
            source.reads.borrow().len(),
            reads,
            "owned cache must retain each original acquisition"
        );
    }
}
