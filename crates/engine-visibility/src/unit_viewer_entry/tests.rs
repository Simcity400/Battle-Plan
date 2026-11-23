use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x1000;
const COUNTRY: usize = 0x3000;
const DIVISION: usize = 0x5000;
const FLEET: usize = 0x5800;
const TASK_FORCE: usize = 0x6000;
const GUN_ONE: usize = 0x6400;
const GUN_TWO: usize = 0x6600;
const GUN_THREE: usize = 0x6800;
const ENTRIES: usize = 0x7000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRIES,
        &0x2000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRIES_CAPACITY,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRIES_COUNT,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS,
        &0x2100_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS_CAPACITY,
        &3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS_COUNT,
        &3_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x2008, &(COUNTRY as u64).to_le_bytes());
    put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
    put(&mut bytes, 0x2108, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        COUNTRY + ccountry::DIVISIONS,
        &0x4000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::DIVISIONS_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::DIVISIONS_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x4000, &(DIVISION as u64).to_le_bytes());
    put(
        &mut bytes,
        COUNTRY + ccountry::FLEETS,
        &0x4100_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::FLEETS_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::FLEETS_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x4100, &(FLEET as u64).to_le_bytes());
    put(
        &mut bytes,
        FLEET,
        &(cfleet::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        FLEET + cfleet::TASK_FORCES,
        &0x4200_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        FLEET + cfleet::TASK_FORCE_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        FLEET + cfleet::TASK_FORCE_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x4200, &(TASK_FORCE as u64).to_le_bytes());
    put(
        &mut bytes,
        DIVISION,
        &(carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        DIVISION + cunit::HANDLE,
        &((7_u64 << 32) | 3).to_le_bytes(),
    );
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_DATA,
        &(ENTRIES as u64).to_le_bytes(),
    );
    put(&mut bytes, DIVISION + 0x280, &4_u32.to_le_bytes());
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_COUNT,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TASK_FORCE,
        &(ctask_force::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TASK_FORCE + cunit::HANDLE,
        &u64::MAX.to_le_bytes(),
    );
    for (index, (viewer, level)) in [(4_u32, 255_u8), (u32::MAX, 4)].into_iter().enumerate() {
        put(
            &mut bytes,
            ENTRIES + index * STRIDE + VIEWER,
            &viewer.to_le_bytes(),
        );
        put(
            &mut bytes,
            ENTRIES + index * STRIDE + TIMER,
            &i64::MIN.to_le_bytes(),
        );
        put(&mut bytes, ENTRIES + index * STRIDE + LEVEL, &[level]);
    }
    bytes
}

fn walked(bytes: &[u8], rows: usize) -> (Outcome, Vec<Row>) {
    let memory = SliceMemory { base: BASE, bytes };
    let mut output = vec![Row::default(); rows];
    let outcome = walk(&memory, 0, BASE, &mut output);
    (outcome, output)
}

fn railway_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    let second_country = 0x3600_usize;
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRIES_CAPACITY,
        &3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRIES_COUNT,
        &3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS_CAPACITY,
        &4_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        BASE + cgame_state::COUNTRY_LINKS_COUNT,
        &4_u32.to_le_bytes(),
    );
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    put(&mut bytes, 0x2010, &(second_country as u64).to_le_bytes());
    put(&mut bytes, 0x2108, &2_u32.to_le_bytes());
    put(&mut bytes, 0x210c, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        second_country,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        second_country + ccountry::ID,
        &3_u32.to_le_bytes(),
    );

    put(
        &mut bytes,
        COUNTRY + ccountry::RAILWAY_UNITS,
        &0x4300_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::RAILWAY_UNITS_CAPACITY,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY + ccountry::RAILWAY_UNITS_COUNT,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x4300, &(GUN_ONE as u64).to_le_bytes());
    put(&mut bytes, 0x4308, &(GUN_TWO as u64).to_le_bytes());
    put(
        &mut bytes,
        second_country + ccountry::RAILWAY_UNITS,
        &0x4380_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        second_country + ccountry::RAILWAY_UNITS_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        second_country + ccountry::RAILWAY_UNITS_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x4380, &(GUN_THREE as u64).to_le_bytes());

    put(
        &mut bytes,
        TASK_FORCE + cunit::VIEWER_DATA,
        &0x7300_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        TASK_FORCE + cunit::VIEWER_COUNT - 4,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        TASK_FORCE + cunit::VIEWER_COUNT,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x7300 + VIEWER, &2_u32.to_le_bytes());
    put(&mut bytes, 0x7300 + LEVEL, &[3]);
    for (gun, id, entries, count) in [
        (GUN_ONE, 17_u64, 0x7400_u64, 2_u32),
        (GUN_TWO, 18, 0x7500, 1),
        (GUN_THREE, 19, 0x7600, 1),
    ] {
        put(
            &mut bytes,
            gun,
            &(crailway_gun::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            gun + cunit::HANDLE,
            &((id << 32) | 13).to_le_bytes(),
        );
        put(&mut bytes, gun + cunit::VIEWER_DATA, &entries.to_le_bytes());
        put(
            &mut bytes,
            gun + cunit::VIEWER_COUNT - 4,
            &count.to_le_bytes(),
        );
        put(&mut bytes, gun + cunit::VIEWER_COUNT, &count.to_le_bytes());
        for ordinal in 0..count as usize {
            let entry = entries as usize + ordinal * STRIDE;
            put(
                &mut bytes,
                entry + VIEWER,
                &(ordinal as u32 + 1).to_le_bytes(),
            );
            put(&mut bytes, entry + LEVEL, &[id as u8 + ordinal as u8]);
        }
    }
    bytes
}

#[test]
fn railway_gun_viewer_entries_follow_country_and_unit_roster_order() {
    let bytes = railway_fixture();
    let (outcome, rows) = walked(&bytes, 7);
    assert_eq!(outcome, Outcome::Rows { len: 7 });
    assert_eq!(
        rows.iter()
            .map(|row| (row.unit, row.ordinal, row.level))
            .collect::<Vec<_>>(),
        vec![
            ((7_u64 << 32) | 3, 0, -1),
            ((7_u64 << 32) | 3, 1, 4),
            (u64::MAX, 0, 3),
            ((17_u64 << 32) | 13, 0, 17),
            ((17_u64 << 32) | 13, 1, 18),
            ((18_u64 << 32) | 13, 0, 18),
            ((19_u64 << 32) | 13, 0, 19),
        ]
    );
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let sources = [DivisionSource {
        pointer: DIVISION,
        canonical: 1,
    }];
    let mut shared = vec![Row::default(); 7];
    assert_eq!(
        walk_with_divisions_buffer(&memory, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut shared), Some(DivisionSources {
                entries: &sources,
                outcome: Outcome::Rows { len: 1 }
            })),
        outcome
    );
    assert_eq!(shared, rows);
}

#[test]
fn railway_gun_roster_and_viewer_faults_refuse_complete_intel() {
    for (address, value, reason) in [
        (
            COUNTRY + ccountry::RAILWAY_UNITS_COUNT,
            3_u64,
            "railway gun roster unreadable or invalid",
        ),
        (0x4308, 0, "railway gun roster pointer unreadable or null"),
        (
            GUN_ONE,
            0x1234,
            "unit vtable mismatch or identity unreadable",
        ),
        (
            GUN_ONE + cunit::HANDLE,
            0,
            "nonnull unit has a null reference handle",
        ),
        (
            GUN_ONE + cunit::VIEWER_DATA,
            0,
            "unit viewer vector header invalid",
        ),
    ] {
        let mut bytes = railway_fixture();
        if address == COUNTRY + ccountry::RAILWAY_UNITS_COUNT {
            put(&mut bytes, address, &(value as u32).to_le_bytes());
        } else {
            put(&mut bytes, address, &value.to_le_bytes());
        }
        assert_eq!(walked(&bytes, 3).0, Outcome::Unavailable { reason });
    }
    let mut bytes = railway_fixture();
    put(
        &mut bytes,
        GUN_TWO + cunit::VIEWER_DATA,
        &0x9ff0_u64.to_le_bytes(),
    );
    assert_eq!(
        walked(&bytes, 3).0,
        Outcome::Unavailable {
            reason: "unit viewer entry block unreadable"
        }
    );
}

#[test]
fn native_order_nonpositive_levels_and_full_handles_survive() {
    let (outcome, rows) = walked(&fixture(), 4);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        rows[0],
        Row {
            unit: (7 << 32) | 3,
            ordinal: 0,
            viewer: 4,
            level: -1,
        }
    );
    assert_eq!(
        rows[1],
        Row {
            unit: (7 << 32) | 3,
            ordinal: 1,
            viewer: u32::MAX,
            level: 4,
        }
    );
}

#[test]
fn the_count_probe_counts_every_unit() {
    assert_eq!(
        count(
            &SliceMemory {
                base: BASE,
                bytes: &fixture()
            },
            0,
            BASE
        ),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
}

#[test]
fn null_members_bad_vtables_zero_handles_and_torn_headers_refuse() {
    for (address, value, reason) in [
        (0x4000, 0_u64, "division roster pointer unreadable or null"),
        (0x4200, 0, "task force roster pointer unreadable or null"),
        (
            DIVISION,
            0x1234,
            "unit vtable mismatch or identity unreadable",
        ),
        (FLEET, 0x1234, "fleet vtable mismatch"),
        (
            DIVISION + cunit::HANDLE,
            0,
            "nonnull unit has a null reference handle",
        ),
        (
            DIVISION + cunit::VIEWER_DATA,
            0,
            "unit viewer vector header invalid",
        ),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, &value.to_le_bytes());
        assert_eq!(walked(&bytes, 4).0, Outcome::Unavailable { reason });
    }
    let mut bytes = fixture();
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_COUNT,
        &9_u32.to_le_bytes(),
    );
    assert_eq!(
        walked(&bytes, 4).0,
        Outcome::Unavailable {
            reason: "unit viewer vector header invalid"
        }
    );
    let mut bytes = fixture();
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_DATA,
        &(BASE as u64 + 0x8ff0).to_le_bytes(),
    );
    put(&mut bytes, DIVISION + 0x280, &9_u32.to_le_bytes());
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_COUNT,
        &9_u32.to_le_bytes(),
    );
    assert_eq!(
        walked(&bytes, 32).0,
        Outcome::Unavailable {
            reason: "unit viewer entry block unreadable"
        }
    );
}

#[test]
fn a_world_with_no_viewer_entries_is_an_honest_empty_table() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        DIVISION + cunit::VIEWER_COUNT,
        &0_u32.to_le_bytes(),
    );
    assert_eq!(walked(&bytes, 4).0, Outcome::Rows { len: 0 });
}
#[test]
fn shared_division_census_avoids_roster_reads_and_keeps_bounds_and_faults() {
    struct NoRoster<'a>(SliceMemory<'a>);
    impl Memory for NoRoster<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address == COUNTRY + ccountry::DIVISIONS || address == 0x4000 {
                return false;
            }
            self.0.copy(address, out)
        }
    }
    let bytes = fixture();
    let direct = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let memory = NoRoster(SliceMemory {
        base: BASE,
        bytes: &bytes,
    });
    let entries = [DivisionSource {
        pointer: DIVISION,
        canonical: 1,
    }];
    for capacity in [0, 1, 2, 4] {
        let mut expected = vec![Row::default(); capacity];
        let expected_outcome = walk(&direct, 0, BASE, &mut expected);
        let mut output = vec![Row::default(); capacity];
        let result = walk_with_divisions_buffer(&memory, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut output), Some(DivisionSources {
                entries: &entries,
                outcome: Outcome::Rows { len: 1 },
            }));
        assert_eq!(result, expected_outcome);
        assert_eq!(output, expected);
    }
    for (entry, reason) in [
        (
            DivisionSource {
                pointer: 0,
                canonical: 1,
            },
            "division roster pointer unreadable or null",
        ),
        (
            DivisionSource {
                pointer: DIVISION,
                canonical: 3,
            },
            "shared division census references an absent country",
        ),
    ] {
        assert_eq!(
            walk_with_divisions_buffer(&memory, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 4]), Some(DivisionSources {
                    entries: &[entry],
                    outcome: Outcome::Rows { len: 1 }
                })),
            Outcome::Unavailable { reason }
        );
    }
    assert_eq!(
        walk_with_divisions_buffer(&memory, 0, BASE, engine_native_memory::walk::Buffer::Count, Some(DivisionSources {
                entries: &entries,
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                }
            })),
        Outcome::Unavailable {
            reason: "shared division census incomplete"
        }
    );
}

#[test]
fn shared_division_viewer_entries_are_hinted_before_their_reads() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Touch {
        Copy(usize, usize),
        Hint(usize, usize),
    }
    struct Traced<'a> {
        memory: SliceMemory<'a>,
        log: std::cell::RefCell<Vec<Touch>>,
        hints: bool,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.log.borrow_mut().push(Touch::Copy(address, out.len()));
            self.memory.copy(address, out)
        }
        fn prefetch(&self, address: usize, len: usize) {
            if self.hints {
                self.log.borrow_mut().push(Touch::Hint(address, len));
            }
        }
    }
    let bytes = fixture();
    let header: [u8; 16] = bytes
        [DIVISION + cunit::VIEWER_DATA - BASE..DIVISION + cunit::VIEWER_DATA - BASE + 16]
        .try_into()
        .unwrap();
    let viewers = Array::from_header(header, STRIDE).unwrap();
    assert!(
        viewers.count != 0,
        "the fixture division has viewer entries"
    );
    let span = viewers.count.min(ENTRY_BATCH as u32) as usize * STRIDE;
    let run = |hints: bool| {
        let memory = Traced {
            memory: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            log: Default::default(),
            hints,
        };
        let mut output = vec![Row::default(); 8];
        let outcome = walk_with_divisions_buffer(&memory, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut output), Some(DivisionSources {
                entries: &[DivisionSource {
                    pointer: DIVISION,
                    canonical: 1,
                }],
                outcome: Outcome::Rows { len: 1 },
            }));
        (outcome, output, memory.log.into_inner())
    };
    let (outcome, rows, log) = run(true);
    let (plain_outcome, plain_rows, plain_log) = run(false);
    assert_eq!((outcome, &rows), (plain_outcome, &plain_rows));
    let copies: Vec<_> = log
        .iter()
        .filter(|touch| matches!(touch, Touch::Copy(..)))
        .copied()
        .collect();
    assert_eq!(copies, plain_log);
    let hint = log
        .iter()
        .position(|&touch| touch == Touch::Hint(viewers.data, span))
        .expect("the division's viewer entries are hinted");
    let read = log
        .iter()
        .position(|&touch| touch == Touch::Copy(viewers.data, span))
        .expect("the division's viewer entries are read");
    assert!(hint < read);
}

#[test]
fn shared_division_batches_preserve_entry_fault_after_growth_and_partial_headers() {
    struct Fault<'a> {
        bytes: &'a [u8],
        address: usize,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address <= self.address && self.address < address.saturating_add(out.len()) {
                return false;
            }
            SliceMemory {
                base: BASE,
                bytes: self.bytes,
            }
            .copy(address, out)
        }
    }
    let bytes = fixture();
    let entries = [DivisionSource {
        pointer: DIVISION,
        canonical: 1,
    }; 65];
    let sources = DivisionSources {
        entries: &entries,
        outcome: Outcome::Rows { len: entries.len() },
    };
    let mut output = vec![Row::default(); 1];
    assert_eq!(
        walk_with_divisions_buffer(&SliceMemory {
                base: BASE,
                bytes: &bytes
            }, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut output), Some(sources)),
        Outcome::Rows { len: 130 }
    );
    for (address, reason) in [
        (DIVISION, "unit vtable mismatch or identity unreadable"),
        (
            DIVISION + cunit::HANDLE,
            "unit handle or viewer vector header unreadable",
        ),
        (
            DIVISION + cunit::VIEWER_DATA,
            "unit handle or viewer vector header unreadable",
        ),
        (ENTRIES + STRIDE, "unit viewer entry block unreadable"),
    ] {
        assert_eq!(
            walk_with_divisions_buffer(&Fault {
                    bytes: &bytes,
                    address
                }, 0, BASE, engine_native_memory::walk::Buffer::Count, Some(sources)),
            Outcome::Unavailable { reason }
        );
    }
}

fn scale_fixture(count: usize) -> (Vec<u8>, Vec<DivisionSource>) {
    const UNIT: usize = cunit::VIEWER_COUNT + 4;
    let mut bytes = fixture();
    let start = BASE + bytes.len();
    let entries = start + count * UNIT;
    let total: usize = (0..count).map(|i| i % 40).sum();
    bytes.resize(bytes.len() + count * UNIT + total * STRIDE, 0);
    let mut next = entries;
    let mut sources = Vec::with_capacity(count);
    for i in 0..count {
        let unit = start + i * UNIT;
        let viewers = i % 40;
        put(
            &mut bytes,
            unit,
            &(carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit::HANDLE,
            &(((i as u64 + 1) << 32) | 3).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit::VIEWER_DATA,
            &(next as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit::VIEWER_DATA + 8,
            &(viewers as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit::VIEWER_COUNT,
            &(viewers as u32).to_le_bytes(),
        );
        for entry in 0..viewers {
            let at = next + entry * STRIDE;
            put(&mut bytes, at + VIEWER, &((i + entry) as u32).to_le_bytes());
            put(&mut bytes, at + LEVEL, &[(entry % 7) as u8]);
        }
        next += viewers * STRIDE;
        sources.push(DivisionSource {
            pointer: unit,
            canonical: 1,
        });
    }
    (bytes, sources)
}

fn batched_and_serial(
    memory: &impl Memory,
    serial_memory: &impl Memory,
    sources: &[DivisionSource],
    rows: usize,
) -> [(Outcome, Vec<Row>); 2] {
    let mut batched = vec![Row::default(); rows];
    let new = walk_with_divisions_buffer(memory, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut batched), Some(DivisionSources {
            entries: sources,
            outcome: Outcome::Rows { len: sources.len() },
        }));
    let mut serial = vec![Row::default(); rows];
    let reader = Reader {
        memory: serial_memory,
        base: 0,
    };
    let mut counter = Counter {
        len: 0,
        unread_rows: 0,
    };
    let old = sources
        .iter()
        .try_for_each(|source| {
            reader.unit(
                source.pointer,
                carmy::VTABLE_RVA,
                &mut Buffer::Growing(&mut serial),
                &mut counter,
            )
        })
        .map_or_else(
            |reason| Outcome::Unavailable { reason },
            |()| counter.outcome(),
        );
    [(new, batched), (old, serial)]
}

#[test]
fn batched_first_blocks_match_the_serial_walk() {
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
    const COUNT: usize = 200;
    let (bytes, sources) = scale_fixture(COUNT);
    let rows: usize = (0..COUNT).map(|i| i % 40).sum();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let [batched, serial] = batched_and_serial(&memory, &memory, &sources, rows);
    assert_eq!(batched.0, Outcome::Rows { len: rows });
    assert_eq!(batched, serial);
    let counted = CountedMemory::new(memory);
    let prior = ReadCounts::take_current();
    let mut output = vec![Row::default(); rows];
    walk_with_divisions_buffer(&counted, 0, BASE, engine_native_memory::walk::Buffer::Growing(&mut output), Some(DivisionSources {
            entries: &sources,
            outcome: Outcome::Rows { len: COUNT },
        }));
    let reads = ReadCounts::take_current();
    prior.add_current();
    let later_blocks = (0..COUNT).filter(|i| i % 40 > ENTRY_BATCH).count() as u64;
    assert!(reads.copy_calls < later_blocks + 64, "{reads:?}");
}

fn scale_fault_sites(bytes: &[u8], sources: &[DivisionSource]) -> [std::ops::Range<usize>; 3] {
    let entries = |i: usize| {
        let at = sources[i].pointer + cunit::VIEWER_DATA - BASE;
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
    };
    let first = entries(37) + 3 * STRIDE;
    let second = entries(75) + (ENTRY_BATCH + 1) * STRIDE;
    let header = sources[90].pointer + cunit::VIEWER_DATA;
    [first..first + 4, second..second + 4, header..header + 16]
}

#[test]
fn a_persistent_viewer_fault_gives_the_serial_result() {
    use engine_native_memory::batch_fallback::{Batch, Faulty, count};
    let (bytes, sources) = scale_fixture(200);
    let rows: usize = (0..200).map(|i| i % 40).sum();
    for (site, (range, counted)) in scale_fault_sites(&bytes, &sources)
        .into_iter()
        .zip([1, 0, 0])
        .enumerate()
    {
        let before = count(Batch::UnitIntel);
        let memory = Faulty::new(
            SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            vec![range],
            false,
        );
        let [batched, serial] = batched_and_serial(&memory, &memory, &sources, rows);
        assert_eq!(batched, serial, "site {site}");
        assert!(
            matches!(batched.0, Outcome::Unavailable { .. }),
            "site {site}"
        );
        assert_eq!(count(Batch::UnitIntel) - before, counted, "site {site}");
    }
}

#[test]
fn a_one_shot_viewer_fault_refuses_no_more_than_the_serial_walk() {
    use engine_native_memory::batch_fallback::Faulty;
    let (bytes, sources) = scale_fixture(200);
    let rows: usize = (0..200).map(|i| i % 40).sum();
    let healthy = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let [expected, _] = batched_and_serial(&healthy, &healthy, &sources, rows);
    for (site, range) in scale_fault_sites(&bytes, &sources).into_iter().enumerate() {
        let once = || {
            Faulty::new(
                SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                vec![range.clone()],
                true,
            )
        };
        let [batched, serial] = batched_and_serial(&once(), &once(), &sources, rows);
        assert!(
            matches!(serial.0, Outcome::Unavailable { .. }),
            "site {site}"
        );
        if site == 0 {
            assert_eq!(batched, expected);
        } else {
            assert_eq!(batched, serial, "site {site}");
        }
    }
}
