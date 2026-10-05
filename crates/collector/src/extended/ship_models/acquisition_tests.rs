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
