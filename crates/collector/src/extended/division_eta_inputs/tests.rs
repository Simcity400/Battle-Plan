use super::*;
use engine::memory::SliceMemory;

#[test]
fn cadence_loan_survives_worker_panic_and_callback_reset_keeps_row_storage() {
    struct Panics;
    impl Memory for Panics {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("witness fault");
        }
    }
    let mut range = RangeCaptured::vacant();
    let row = Arc::new(DivisionEtaInputsRow::default());
    range.carried = Some(vec![Arc::clone(&row)]);
    range.reset();
    assert_eq!(Arc::strong_count(&row), 2);
    range.fail_owned_capture("admission failed");
    assert_eq!(Arc::strong_count(&row), 2);
    range.cadence = Some(Box::default());
    let mut route = [0; 16];
    route[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
    route[8..12].copy_from_slice(&1_u32.to_le_bytes());
    route[12..].copy_from_slice(&1_u32.to_le_bytes());
    let seed = DivisionSeed {
        pointer: 0x2000,
        division: 1,
        handle: Some(1),
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        ordinary_route: Some(route),
        movement_target: Some(0),
        movement_source: Some(0),
        location: Some(0),
        retreat: Some(0),
        strategic_redeployment: Some(0),
        naval_transfer_membership: Some(0),
        ..Default::default()
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        range.capture_owned(
            &Panics,
            CaptureContext {
                base: 0,
                game_state: 0,
                hour: 1,
                force_full: false,
                span: (0, 1),
                partition: (0, 1),
            },
            Sources {
                rows: &[seed],
                outcome: Outcome::Rows { len: 1 },
            },
            None,
        );
    }));
    assert!(result.is_err());
    assert!(range.cadence.is_some());
    assert_eq!(Arc::strong_count(&row), 1);
}

#[test]
fn air_defense_memo_keeps_exact_signed_arguments_through_collisions_and_resets() {
    // Every input collides in the sole slot, including sign changes whose
    // pure results happen to be equal. Only identical triples can hit.
    let mut memo = AirDefenseMemo::with_capacity(1);
    for (index, arguments) in [
        [100_000, 100_000, 50_000],
        [-100_000, 100_000, 50_000],
        [100_000, -100_000, 50_000],
        [100_000, 100_000, -50_000],
        [100_000, 0, 50_000],
        [100_000, i64::MIN, i64::MAX],
        [100_000, i64::MAX, i64::MIN],
        [0, i64::MIN, i64::MAX],
        [100_000, 100_000, 50_000],
    ]
    .into_iter()
    .enumerate()
    {
        let [defense, denominator, reduction] = arguments;
        let expected = engine::csub_unit_definition::movement_air_defense_value(
            defense,
            denominator,
            reduction,
        );
        assert!(expected.is_ok());
        assert_eq!(memo.value(defense, denominator, reduction), expected);
        assert_eq!(memo.requests, 2 * index as u64 + 1);
        assert_eq!(memo.hits, index as u64);
        assert_eq!(memo.value(defense, denominator, reduction), expected);
        assert_eq!(memo.requests, 2 * index as u64 + 2);
        assert_eq!(memo.hits, index as u64 + 1);
    }
    memo.reset_counts();
    assert_eq!((memo.requests, memo.hits), (0, 0));
    assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
    assert_eq!((memo.requests, memo.hits), (1, 1));
}

#[test]
fn air_defense_hits_preserve_fresh_operands_and_first_native_failure() {
    use engine::csub_unit_definition as stats;
    use std::cell::Cell;
    const BASE: usize = 0x140_000_000;
    const STATS: usize = 0x1000;
    struct Source {
        arguments: Cell<[i64; 3]>,
        fault: Cell<Option<usize>>,
        identity: Cell<bool>,
        reads: RefCell<Vec<usize>>,
    }
    impl Memory for Source {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if self.fault.get() == Some(address) {
                return false;
            }
            let arguments = self.arguments.get();
            let value = if address == STATS {
                if self.identity.get() {
                    (BASE + stats::VTABLE_RVA) as i64
                } else {
                    0
                }
            } else if address == STATS + stats::MOVEMENT_AIR_DEFENSE {
                arguments[0]
            } else if address == BASE + stats::AIR_DEFENSE_DENOMINATOR_RVA {
                arguments[1]
            } else if address == BASE + stats::AIR_DEFENSE_REDUCTION_RVA {
                arguments[2]
            } else {
                return false;
            };
            output.copy_from_slice(&value.to_le_bytes());
            true
        }
    }
    let source = Source {
        arguments: Cell::new([100_000, 100_000, 50_000]),
        fault: Cell::new(None),
        identity: Cell::new(true),
        reads: RefCell::new(Vec::new()),
    };
    let arena = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let air_defense = RefCell::new(AirDefenseMemo::with_capacity(1));
    let memory = ModifierMemory {
        inner: &source,
        arena: &arena,
        definitions: &PassDefinitions::default(),
        air_defense: &air_defense,
        countries: None,
    };
    let addresses = [
        STATS,
        STATS + stats::MOVEMENT_AIR_DEFENSE,
        BASE + stats::AIR_DEFENSE_DENOMINATOR_RVA,
        BASE + stats::AIR_DEFENSE_REDUCTION_RVA,
    ];
    for arguments in [
        [100_000, 100_000, 50_000],
        [50_000, 100_000, 50_000],
        [50_000, 200_000, 50_000],
        [50_000, 200_000, -50_000],
        [100_000, 100_000, 50_000],
    ] {
        source.arguments.set(arguments);
        let expected = stats::movement_air_defense(&source, BASE, STATS);
        air_defense.borrow_mut().reset_counts();
        for hits in [0, 1] {
            source.reads.borrow_mut().clear();
            assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
            assert_eq!(&*source.reads.borrow(), &addresses);
            let memo = air_defense.borrow();
            assert_eq!((memo.requests, memo.hits), (hits + 1, hits));
        }
    }
    // The matching successful entry stays present while each fresh read
    // fails. The pure hook must not run until every operand is available.
    for (index, address) in addresses.into_iter().enumerate() {
        source.fault.set(Some(address));
        let expected = stats::movement_air_defense(&source, BASE, STATS);
        assert!(expected.is_err());
        source.reads.borrow_mut().clear();
        assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
        assert_eq!(&*source.reads.borrow(), &addresses[..=index]);
        let memo = air_defense.borrow();
        assert_eq!((memo.requests, memo.hits), (2, 1));
    }
    source.fault.set(None);
    source.identity.set(false);
    let expected = stats::movement_air_defense(&source, BASE, STATS);
    assert!(expected.is_err());
    source.reads.borrow_mut().clear();
    assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
    assert_eq!(&*source.reads.borrow(), &addresses[..1]);
    source.identity.set(true);
    source.arguments.set([0, i64::MIN, i64::MAX]);
    source.fault.set(Some(addresses[2]));
    source.reads.borrow_mut().clear();
    assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), Ok(0));
    assert_eq!(&*source.reads.borrow(), &addresses[..2]);
    let memo = air_defense.borrow();
    assert_eq!((memo.requests, memo.hits), (2, 1));
}

#[test]
fn air_defense_storage_grows_with_rows_off_capture_and_accounts_for_capacity() {
    let mut range = RangeCaptured::vacant();
    assert_eq!(range.allocated_bytes(), 0);
    let memo = range.air_defense.get_mut();
    for _ in 0..2 {
        assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
    }
    assert_eq!((memo.requests, memo.hits), (2, 0));
    assert_eq!(memo.allocated_bytes(), 0);
    let mut capacities = Capacities {
        rows: 1,
        terrain: 0,
        legs: 0,
        geometry: cprovince::GeometryCapacities {
            provinces: 0,
            neighbour_entries: 0,
        },
        modifiers: SnapshotCapacity::default(),
        ranges: 1,
    };
    for rows in [1, 3] {
        capacities.rows = rows;
        range.reserve(&capacities);
        range.reset();
        let memo = range.air_defense.get_mut();
        assert_eq!(memo.slots.len(), rows);
        assert_eq!(
            memo.allocated_bytes(),
            memo.slots.capacity() * std::mem::size_of::<Option<AirDefenseEntry>>()
        );
        assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
        assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
        assert_eq!((memo.requests, memo.hits), (2, 1));
        assert_eq!(
            range.allocated_bytes(),
            range.packet.allocated_bytes() + range.air_defense.borrow().allocated_bytes()
        );
    }
    range.reset();
    assert_eq!(
        range.air_defense.get_mut().value(100_000, 100_000, 50_000),
        Ok(25_000)
    );
    assert_eq!(range.air_defense.borrow().hits, 1);
    range.fail_owned_capture("worker stopped");
    let memo = range.air_defense.borrow();
    assert_eq!((memo.requests, memo.hits), (0, 0));
    assert_eq!(range.counters.air_defense_requests, 0);
    assert_eq!(range.counters.air_defense_hits, 0);
}

#[test]
fn country_roster_reuse_keeps_ordinary_eta_reads_fresh_and_scoped() {
    use engine::{ccountry, cgame_state, country_memory::Cache};
    use std::cell::Cell;
    let mut bytes = vec![0; 0x4000];
    let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    for (offset, pointer, count) in [
        (cgame_state::COUNTRIES, 0x1500_u64, 4_u32),
        (cgame_state::COUNTRY_LINKS, 0x1700, 8),
    ] {
        put(0x1000 + offset, &pointer.to_le_bytes());
        put(0x1000 + offset + 8, &count.to_le_bytes());
        put(0x1000 + offset + 12, &count.to_le_bytes());
    }
    for slot in [1, 2] {
        put(0x1500 + slot * 8, &0x2000_u64.to_le_bytes());
    }
    for link in [1, 2, 7] {
        put(0x1700 + link * 4, &1_u32.to_le_bytes());
    }
    put(0x2000, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
    put(0x2000 + ccountry::ID, &7_u32.to_le_bytes());
    put(0x2000 + ccountry::COMMAND_POWER, &11_i64.to_le_bytes());
    struct Source {
        bytes: Vec<u8>,
        reads: Cell<usize>,
        scalars: Cell<usize>,
        spans: RefCell<Vec<(usize, usize)>>,
        fault: Cell<Option<usize>>,
    }
    impl Memory for Source {
        fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
            self.scalars.set(self.scalars.get() + 1);
            self.reads.set(self.reads.get() + 1);
            self.spans
                .borrow_mut()
                .push((address, std::mem::size_of::<T>()));
            if self.fault.get() == Some(address) {
                return None;
            }
            SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .read_scalar::<T>(address)
        }

        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            self.spans.borrow_mut().push((address, output.len()));
            self.fault.get() != Some(address)
                && SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .copy(address, output)
        }
    }
    let source = Source {
        bytes,
        reads: Cell::new(0),
        scalars: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        fault: Cell::new(None),
    };
    let mut cache = Cache::try_with_capacity(64).unwrap();
    let cached = cache.prepare(&source, 0, 0x1000);
    let roster = cached.authenticated_countries(0, 0x1000).unwrap();
    let expected = ccountry::Reader::new(&cached, 0, 0x1000)
        .unwrap()
        .resolve(7)
        .unwrap();
    let arena = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let air_defense = RefCell::new(AirDefenseMemo::with_capacity(0));
    let memory = ModifierMemory {
        inner: &source,
        arena: &arena,
        definitions: &PassDefinitions::default(),
        air_defense: &air_defense,
        countries: Some((0, 0x1000, roster)),
    };
    source.reads.set(0);
    source.scalars.set(0);
    source.spans.borrow_mut().clear();
    let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(reader.resolve(7).unwrap(), expected);
    assert!(reader.at(2).unwrap().is_none());
    // The complete callback census owns both headers and the authenticated
    // objects. The requested link still reads its exact four native bytes.
    assert_eq!(source.reads.get(), 1);
    assert_eq!(source.scalars.get(), 1);
    assert_eq!(*source.spans.borrow(), [(0x1700 + 7 * 4, 4)]);
    assert!(memory.authenticated_countries(1, 0x1000).is_none());
    assert!(memory.authenticated_countries(0, 0x1001).is_none());
    assert_eq!(engine::read::<u32>(&memory, 0x2000 + ccountry::ID), Some(7));
    assert_eq!(source.reads.get(), 2);
    assert_eq!(source.scalars.get(), 2);
    assert_eq!(
        engine::read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
        Some(11)
    );
    source.fault.set(Some(0x2000 + ccountry::COMMAND_POWER));
    assert_eq!(
        engine::read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
        None
    );
    assert_eq!(source.scalars.get(), 2);
    assert_eq!(
        &source.spans.borrow()[1..],
        [
            (0x2000 + ccountry::ID, 4),
            (0x2000 + ccountry::COMMAND_POWER, 8),
            (0x2000 + ccountry::COMMAND_POWER, 8),
        ]
    );
    source.fault.set(Some(0x1700 + 7 * 4));
    source.spans.borrow_mut().clear();
    assert!(reader.resolve(7).is_err());
    assert_eq!(*source.spans.borrow(), [(0x1700 + 7 * 4, 4)]);
    let direct = ModifierMemory {
        countries: None,
        ..memory
    };
    source.fault.set(None);
    source.spans.borrow_mut().clear();
    assert_eq!(
        ccountry::Reader::new(&direct, 0, 0x1000)
            .unwrap()
            .resolve(7)
            .unwrap(),
        expected
    );
    assert_eq!(
        *source.spans.borrow(),
        [
            (0x1000 + cgame_state::COUNTRIES, 16),
            (0x1000 + cgame_state::COUNTRY_LINKS, 16),
            (0x1700 + 7 * 4, 4),
            (0x1500 + 8, 8),
            (0x2000, 8),
            (0x2000 + ccountry::ID, 4),
            (0x1700 + 7 * 4, 4),
            (0x1700 + 4, 4),
            (0x1700 + 4, 4),
            (0x1500 + 8, 8),
        ]
    );
    // Without the scoped census, each header and its original failure
    // remain mandatory. The second header cannot conceal the first fault.
    let headers = [
        (0x1000 + cgame_state::COUNTRIES, 16),
        (0x1000 + cgame_state::COUNTRY_LINKS, 16),
    ];
    for (index, &(address, _)) in headers.iter().enumerate() {
        source.fault.set(Some(address));
        source.spans.borrow_mut().clear();
        assert!(ccountry::Reader::new(&direct, 0, 0x1000).is_err());
        assert_eq!(&*source.spans.borrow(), &headers[..=index]);
    }
}

/// Every division of a country asks its accumulator for the same movement
/// keys, and each lookup scans the whole array. The snapshot reads it once.
#[test]
fn the_modifier_snapshot_reads_each_accumulator_once_a_walk() {
    const ACCUMULATOR: usize = 0x1000;
    const DATA: usize = 0x2000;
    const ENTRIES: u32 = 200;
    let mut bytes = vec![0_u8; 0x4000];
    bytes[ACCUMULATOR..ACCUMULATOR + 8].copy_from_slice(&(DATA as u64).to_le_bytes());
    bytes[ACCUMULATOR + 8..ACCUMULATOR + 12].copy_from_slice(&ENTRIES.to_le_bytes());
    bytes[ACCUMULATOR + 12..ACCUMULATOR + 16].copy_from_slice(&ENTRIES.to_le_bytes());
    for index in 0..ENTRIES {
        let at = DATA + index as usize * 16;
        bytes[at..at + 4].copy_from_slice(&(index + 1).to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&i64::from(index).to_le_bytes());
    }
    struct Counted<'a> {
        bytes: &'a [u8],
        reads: std::cell::Cell<usize>,
        fault: Option<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.fault == Some(address) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    let keys = [0x2b_u32, 0x5e, 0x17f, 0xc8];
    let source = Counted {
        bytes: &bytes,
        reads: std::cell::Cell::new(0),
        fault: None,
    };
    let expected =
        keys.map(|key| engine::modifier_accumulator::lookup_at(&source, ACCUMULATOR, key).unwrap());
    assert_eq!(
        expected,
        [
            Value {
                raw: 42,
                present: true
            },
            Value {
                raw: 93,
                present: true
            },
            Value::default(),
            Value {
                raw: 199,
                present: true
            },
        ]
    );
    let direct = source.reads.replace(0);
    let arena = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 4,
            entries: ENTRIES as usize,
        })
        .unwrap(),
    );
    let air_defense = RefCell::new(AirDefenseMemo::with_capacity(0));
    let memory = ModifierMemory {
        inner: &source,
        arena: &arena,
        definitions: &PassDefinitions::default(),
        air_defense: &air_defense,
        countries: None,
    };
    let values =
        keys.map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
    assert_eq!(values, expected);
    // One scan for the four keys, and the scan is the direct reader's own.
    assert_eq!(source.reads.get() * keys.len(), direct);
    // A new capture clears the arena and acquires current bytes again.
    arena.borrow_mut().clear();
    source.reads.set(0);
    let values =
        keys.map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
    assert_eq!(values, expected);
    assert_eq!(source.reads.get() * keys.len(), direct);
    // Too small an arena refuses the snapshot after its header read and
    // the direct reader answers every key.
    let small = RefCell::new(
        SnapshotArena::try_new(SnapshotCapacity {
            nodes: 4,
            entries: 8,
        })
        .unwrap(),
    );
    let memory = ModifierMemory {
        inner: &source,
        arena: &small,
        definitions: &PassDefinitions::default(),
        air_defense: &air_defense,
        countries: None,
    };
    source.reads.set(0);
    assert_eq!(
        memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
        None
    );
    assert_eq!(source.reads.get(), 1);
    let values =
        keys.map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
    assert_eq!(values, expected);
    assert_eq!(source.reads.get(), direct + 1);
    assert!(small.borrow().required_capacity().entries >= ENTRIES as usize);

    let empty = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let memory = ModifierMemory {
        inner: &source,
        arena: &empty,
        definitions: &PassDefinitions::default(),
        air_defense: &air_defense,
        countries: None,
    };
    source.reads.set(0);
    assert_eq!(
        memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
        None
    );
    assert_eq!(source.reads.get(), 0);
    let values =
        keys.map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
    assert_eq!(values, expected);
    assert_eq!(source.reads.get(), direct);
    assert_eq!(empty.borrow().required_capacity().nodes, 1);
    assert_eq!(empty.borrow().allocated_bytes(), 0);

    // Source failures remain owned errors; lookup must not retry them through
    // the direct-reader fallback, even when another movement key is requested.
    for (fault, reads, reason) in [
        (
            ACCUMULATOR,
            1,
            "modifier accumulator header unreadable or invalid",
        ),
        (DATA + 64 * 16, 66, "modifier accumulator entry unreadable"),
    ] {
        let source = Counted {
            bytes: &bytes,
            reads: std::cell::Cell::new(0),
            fault: Some(fault),
        };
        let arena = RefCell::new(
            SnapshotArena::try_new(SnapshotCapacity {
                nodes: 4,
                entries: ENTRIES as usize,
            })
            .unwrap(),
        );
        let memory = ModifierMemory {
            inner: &source,
            arena: &arena,
            definitions: &PassDefinitions::default(),
            air_defense: &air_defense,
            countries: None,
        };
        assert_eq!(
            memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
            Some(Err(reason))
        );
        assert_eq!(source.reads.get(), reads);
        for key in keys {
            assert_eq!(
                engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key),
                Err(reason)
            );
        }
        assert_eq!(source.reads.get(), reads);
    }
}

#[test]
fn a_failed_census_never_becomes_an_empty_eta_input_table() {
    let mut capture = Captured::with_capacities(&Capacities::with_ranges(1));
    let sources = Sources {
        rows: &[],
        outcome: Outcome::Unavailable {
            reason: "census failed",
        },
    };
    assert_eq!(capture.plan(&sources), 1);
    let (mut range, span, partition) = capture.take_range(0);
    range.cadence = Some(Box::default());
    range.capture_owned(
        &SliceMemory {
            base: 0,
            bytes: &[],
        },
        CaptureContext {
            hour: 0,
            force_full: false,
            base: 0,
            game_state: 0,
            span,
            partition,
        },
        sources,
        None,
    );
    assert!(range.cadence.is_some());
    capture.restore_range(0, range);
    assert!(matches!(
        capture.outcomes()[0],
        Outcome::Unavailable {
            reason: "census failed"
        }
    ));
    assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
}

#[test]
fn stable_partitions_cover_the_census_and_a_failed_range_denies_the_table() {
    let mut capture = Captured::with_capacities(&Capacities::with_ranges(3));
    let mut route = [0; 16];
    route[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
    route[8..12].copy_from_slice(&64_u32.to_le_bytes());
    route[12..].copy_from_slice(&64_u32.to_le_bytes());
    let seeds: Vec<_> = (0..12)
        .map(|index| DivisionSeed {
            pointer: 0x1000 + index,
            division: index as u64 + 1,
            ordinary_route: Some(route),
            ..Default::default()
        })
        .collect();
    let outcome = Outcome::Rows { len: seeds.len() };
    let sources = Sources {
        rows: &seeds,
        outcome,
    };
    assert_eq!(capture.plan(&sources), 3);
    // Every worker sees the same immutable census and a distinct stable partition.
    for index in 0..3 {
        let (captured, span, partition) = capture.take_range(index);
        assert_eq!(span, (0, seeds.len()));
        assert_eq!(partition, (index, 3));
        capture.restore_range(index, captured);
    }
    // Empty and small censuses preserve ownership too.
    assert_eq!(
        capture.plan(&Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 }
        }),
        3
    );
    // A worker failure on one range denies the table with its own reason.
    assert_eq!(capture.plan(&sources), 3);
    let (mut range, _, _) = capture.take_range(0);
    assert_eq!(range.fail_owned_capture("worker stopped").ticks, [0]);
    capture.restore_range(0, range);
    assert!(matches!(
        capture.outcomes()[0],
        Outcome::Unavailable {
            reason: "worker stopped"
        }
    ));
}

/// Prepare outside the allocator-denial window; execute through the same
/// zero-worker pool route that the callback uses when it runs an accepted job.
pub(crate) fn allocation_failure_route_exercise(
    workers: usize,
) -> impl FnOnce() -> bool + Send + 'static {
    struct Fixture {
        bytes: Vec<u8>,
        seed: DivisionSeed,
        range: RangeCaptured,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let global = match address {
                engine::cgame_state::PLAY_SESSION_STARTED_AT_RVA => Some(1_i64),
                carmy::TRANSPORT_SCALE_RVA | carmy::OUTER_SCALE_RVA => Some(100_000),
                carmy::INVASION_SPEED_RVA => Some(50_000),
                engine::cnaval_unit_transfer::BASE_SPEED_RVA => Some(100_000),
                engine::modifier_definition::TRADE_FEATURE_FLAGS_RVA => Some(0),
                _ => None,
            };
            if let Some(value) = global {
                let bytes = value.to_le_bytes();
                if let Some(source) = bytes.get(..output.len()) {
                    output.copy_from_slice(source);
                    return true;
                }
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        word(bytes, at, data as u64);
        word(bytes, at + 8, u64::from(count) | (u64::from(count) << 32));
    }
    const STATE: usize = 0x1000;
    const ROUTE: usize = 0xd000;
    const LEGS: usize = 3;
    let mut bytes = vec![0_u8; 0xe000];
    array(
        &mut bytes,
        STATE + engine::cgame_state::COUNTRIES,
        0x2000,
        2,
    );
    array(
        &mut bytes,
        STATE + engine::cgame_state::COUNTRY_LINKS,
        0x2100,
        2,
    );
    word(&mut bytes, 0x2008, 0x3000);
    bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
    word(&mut bytes, 0x3000, engine::ccountry::VTABLE_RVA as u64);
    bytes[0x3008..0x300c].copy_from_slice(&1_u32.to_le_bytes());
    array(
        &mut bytes,
        STATE + engine::cgame_state::PROVINCES,
        0x5000,
        5,
    );
    array(
        &mut bytes,
        STATE + engine::cgame_state::STRATEGIC_REGIONS,
        0xb100,
        1,
    );
    word(&mut bytes, 0xb100, 0xb000);
    word(
        &mut bytes,
        0xb000,
        engine::cstrategic_region::VTABLE_RVA as u64,
    );
    word(
        &mut bytes,
        STATE + engine::cgame_state::WEATHER_MANAGER,
        0xc000,
    );
    word(
        &mut bytes,
        0xc000,
        engine::cweather_manager::VTABLE_RVA as u64,
    );
    // An empty weather region array and disabled transport composition select
    // the native zero modifiers without introducing unrelated country graphs.
    for id in 1_u32..=4 {
        let pointer = 0x6000 + id as usize * 0x200;
        let definition = 0x8000 + id as usize * 0x200;
        word(&mut bytes, 0x5000 + id as usize * 8, pointer as u64);
        word(&mut bytes, pointer, cprovince::VTABLE_RVA as u64);
        bytes[pointer + cprovince::ID..pointer + cprovince::ID + 4]
            .copy_from_slice(&id.to_le_bytes());
        word(
            &mut bytes,
            pointer + cprovince::DEFINITION,
            definition as u64,
        );
        word(&mut bytes, pointer + cprovince::STRATEGIC_REGION, 0xb000);
        bytes[definition + engine::province_definition::ID
            ..definition + engine::province_definition::ID + 4]
            .copy_from_slice(&id.to_le_bytes());
        word(
            &mut bytes,
            definition + engine::province_definition::TERRAIN,
            0xa000,
        );
        if id > 1 {
            bytes[ROUTE + (id as usize - 2) * 4..ROUTE + (id as usize - 1) * 4]
                .copy_from_slice(&id.to_le_bytes());
        }
    }
    let mut header = [0_u8; 16];
    header[..8].copy_from_slice(&(ROUTE as u64).to_le_bytes());
    header[8..12].copy_from_slice(&(LEGS as u32).to_le_bytes());
    header[12..].copy_from_slice(&(LEGS as u32).to_le_bytes());
    let seed = DivisionSeed {
        pointer: 0x4000,
        division: 1,
        handle: Some(1),
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        ordinary_route: Some(header),
        location: Some(0x6200),
        class: Some(0),
        logical_owner: Some(1),
        naval_transfer_membership: Some(1),
        ..Default::default()
    };
    let capacities = Capacities {
        rows: 1,
        terrain: 0,
        legs: LEGS,
        geometry: cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        },
        modifiers: SnapshotCapacity::default(),
        ranges: 1,
    };
    let mut request = Some(Fixture {
        bytes,
        seed,
        range: RangeCaptured::with_capacities(&capacities),
    });
    let pool = crate::acquisition::Pool::start(workers, 1, |fixture: &mut Fixture| {
        // Move the owned packet out while its memory view is borrowed. The
        // vacancy neither allocates nor provisions another route arena.
        let mut range = std::mem::replace(&mut fixture.range, RangeCaptured::vacant());
        range.capture(
            fixture,
            CaptureContext {
                hour: 7,
                force_full: true,
                base: 0,
                game_state: STATE,
                span: (0, 1),
                partition: (0, 1),
            },
            Sources {
                rows: std::slice::from_ref(&fixture.seed),
                outcome: Outcome::Rows { len: 1 },
            },
            None,
        );
        let result = (range.outcome(), range.counters.legs);
        fixture.range = range;
        result
    })
    .unwrap();
    if workers != 0 {
        pool.reserve_worker_slots(1).unwrap();
    }
    move || {
        // The request owns all synthetic memory until the accepted job returns.
        let Ok(ticket) = (unsafe { pool.submit_with_priority(&mut request, 0, workers != 0) })
        else {
            return false;
        };
        let Some(completion) = pool.finish(ticket) else {
            return false;
        };
        let calibration = pool.calibration();
        let stopped = completion.report == Err(crate::acquisition::Failure::Stopped);
        let complete = completion.report == Ok((Outcome::Rows { len: 1 }, LEGS as u64));
        let executed = if workers == 0 {
            calibration.caller_executed == 1 && calibration.worker_executed == 0
        } else {
            calibration.caller_executed == 0 && calibration.worker_executed == 1
        };
        let healthy_or_stopped = if crate::allocation::failed() {
            stopped
        } else {
            complete
        };
        pool.shutdown().is_ok() && healthy_or_stopped && executed
    }
}

#[test]
fn inline_eta_route_fixture_scans_every_leg() {
    assert!(allocation_failure_route_exercise(0)());
    assert!(allocation_failure_route_exercise(1)());
}
