use super::*;
use engine_native::{cgame_state, memory::SliceMemory, msvc_string};
use replay_data::rows::TableSchema;

const BASE: usize = 0x1000;
const MANAGER: usize = 0x2000;
const SOURCE_ARRAY: usize = 0x2100;
const AGGREGATES: usize = 0x2200;
const FIRST: usize = 0x2400;
const SECOND: usize = 0x2500;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
    put(bytes, header, &(data as u64).to_le_bytes());
    put(bytes, header + 8, &count.to_le_bytes());
    put(bytes, header + 12, &count.to_le_bytes());
}

fn source(bytes: &mut [u8], at: usize, current: i64, reason: &[u8]) {
    put(
        bytes,
        at,
        &(cthreat_source::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(bytes, at + cthreat_source::COUNTRY, &u32::MAX.to_le_bytes());
    put(
        bytes,
        at + cthreat_source::SECOND_COUNTRY,
        &7_u32.to_le_bytes(),
    );
    put(bytes, at + cthreat_source::CURRENT, &current.to_le_bytes());
    put(bytes, at + cthreat_source::FINAL, &i64::MAX.to_le_bytes());
    put(bytes, at + cthreat_source::DAILY, &(-1_i64).to_le_bytes());
    put(bytes, at + cthreat_source::DATE, &i32::MIN.to_le_bytes());
    put(bytes, at + cthreat_source::REASON, reason);
    put(
        bytes,
        at + cthreat_source::REASON + msvc_string::LENGTH,
        &(reason.len() as u64 - 1).to_le_bytes(),
    );
    put(
        bytes,
        at + cthreat_source::REASON + msvc_string::CAPACITY,
        &15_u64.to_le_bytes(),
    );
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x2600];
    put(
        &mut bytes,
        BASE + cgame_state::WORLD_THREAT,
        &(MANAGER as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER,
        &(cworld_threat::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER + cworld_threat::TOTAL,
        &(-125_682_i64).to_le_bytes(),
    );
    array(
        &mut bytes,
        MANAGER + cworld_threat::SOURCES,
        SOURCE_ARRAY,
        2,
    );
    array(
        &mut bytes,
        MANAGER + cworld_threat::COUNTRY_AGGREGATES,
        AGGREGATES,
        2,
    );
    put(&mut bytes, SOURCE_ARRAY, &(FIRST as u64).to_le_bytes());
    put(&mut bytes, SOURCE_ARRAY + 8, &(SECOND as u64).to_le_bytes());
    put(&mut bytes, AGGREGATES, &i64::MIN.to_le_bytes());
    put(&mut bytes, AGGREGATES + 8, &0_i64.to_le_bytes());
    source(&mut bytes, FIRST, i64::MIN, b"Peace\0");
    source(&mut bytes, SECOND, 0, b"\0");
    bytes
}

fn small() -> (Capacities, Captured) {
    let capacities = Capacities {
        rows: [1, 2, 2],
        reason_bytes: 8,
    };
    let captured = Captured::with_capacities(&capacities);
    (capacities, captured)
}

fn fixed_job() -> SessionJob {
    SessionJob {
        log: RawLog::new(256, 16 * 1024),
        ..Default::default()
    }
}

fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("expected complete rows, got {view:?}")
    };
    rows.rows::<S>().unwrap()
}

/// Every hour reads the manager: a declaration's tension reaches the next
/// hour, where the daily slot 18 showed it up to a day late.
#[test]
fn session_is_read_every_hour() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (capacities, mut captured) = small();
    let mut job = fixed_job();
    for force_full in [false, true] {
        for hour in 0..24 {
            let ctx = SessionCopyContext {
                base: 0,
                game_state: BASE,
                hour,
                force_full,
            };
            captured.prepare_hour(hour, force_full);
            let report = copy_session_refresh(&memory, ctx, &mut job);
            assert!(report.raw_ok && !job.log.spans.is_empty(), "hour {hour}");
            captured.decode_session_payload(0, &job, &mut [0; N]);
            assert_matches_direct(&captured, &direct(&memory, &capacities));
        }
    }
}

/// A quiet hour copies nothing and publishes the held read; an hour after a
/// source writer ended in `update_total` copies and decodes afresh.
#[test]
fn quiet_tension_hours_publish_the_held_read_without_copying() {
    use crate::world_input_changes::{Domain, Held, Isolated};
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            panic!("quiet session read at {address:#x}");
        }
    }
    let changes: &'static Isolated = Box::leak(Box::new(Isolated::new()));
    let held: &'static Mutex<Held<Tension>> =
        Box::leak(Box::new(Mutex::new(Held::new(Tension::new()))));
    let mut bytes = fixture();
    let (capacities, mut captured) = small();
    let mut job = SessionJob {
        observed: Observed { changes, held },
        ..fixed_job()
    };
    /// One hour's copy and decode; whether it copied nothing.
    fn hour_at(
        memory: &impl Memory,
        hour: i32,
        job: &mut SessionJob,
        captured: &mut Captured,
    ) -> bool {
        let ctx = SessionCopyContext {
            base: 0,
            game_state: BASE,
            hour,
            force_full: false,
        };
        captured.prepare_hour(hour, false);
        assert!(copy_session_refresh(memory, ctx, job).raw_ok);
        captured.decode_session_payload(0, job, &mut [0; N]);
        job.log.spans.is_empty()
    }
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert!(!hour_at(&memory, 1, &mut job, &mut captured));
    assert_matches_direct(&captured, &direct(&memory, &capacities));
    let expected = direct(&memory, &capacities);
    assert!(hour_at(&NoReads, 3, &mut job, &mut captured));
    assert_matches_direct(&captured, &expected);
    // A source writer ran: the next hour copies the new total.
    put(
        &mut bytes,
        MANAGER + cworld_threat::TOTAL,
        &7_i64.to_le_bytes(),
    );
    changes.write(Domain::Tension);
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert!(!hour_at(&memory, 4, &mut job, &mut captured));
    assert_matches_direct(&captured, &direct(&memory, &capacities));
    assert!(hour_at(&NoReads, 5, &mut job, &mut captured));
    assert_matches_direct(&captured, &direct(&memory, &capacities));
}

#[test]
fn manager_sources_and_aggregates_keep_raw_operands_and_literal_reasons() {
    let (_, mut captured) = small();
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured.capture(&memory, 0, BASE, 0, true);
    let tables = captured.tables();
    assert_eq!(
        complete::<tables::WorldTension>(&tables[0]),
        &[rows::WorldTensionRow {
            total_raw: -125_682,
            source_count: 2,
            country_aggregate_count: 2,
        }]
    );
    let sources = complete::<tables::ThreatSources>(&tables[1]);
    assert_eq!(sources.len(), 2);
    assert_eq!(
        (
            sources[0].ordinal,
            sources[0].country,
            sources[0].second_country,
            sources[0].current_raw,
            sources[0].final_raw,
            sources[0].daily_raw,
            sources[0].date,
        ),
        (0, u32::MAX, 7, i64::MIN, i64::MAX, -1, i32::MIN)
    );
    assert_eq!(sources[0].reason, b"Peace");
    assert_eq!(sources[1].reason, b"");
    assert_eq!(
        complete::<tables::ThreatCountryAggregates>(&tables[2]),
        &[
            rows::ThreatCountryAggregateRow {
                country_slot: 0,
                total_raw: i64::MIN,
            },
            rows::ThreatCountryAggregateRow {
                country_slot: 1,
                total_raw: 0,
            },
        ]
    );
}

#[test]
fn capacity_truncation_and_a_broken_root_stay_distinct_and_grow_off_thread() {
    let (mut capacities, mut captured) = small();
    capacities.reason_bytes = 2;
    let mut captured_small = Captured::with_capacities(&capacities);
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured_small.capture(&memory, 0, BASE, 0, true);
    // The five-byte reason does not fit; its row is omitted and counted.
    assert_eq!(
        captured_small.outcomes()[1],
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert!(matches!(
        captured_small.tables()[1],
        TableView::Truncated { missing: 1, .. }
    ));
    captured_small.grow(&mut capacities);
    assert!(capacities.reason_bytes >= 5);

    let mut broken = fixture();
    put(&mut broken, MANAGER, &0_u64.to_le_bytes());
    let _ticks = captured.capture(
        &SliceMemory {
            base: BASE,
            bytes: &broken,
        },
        0,
        BASE,
        0,
        true,
    );
    for outcome in captured.outcomes() {
        assert_eq!(
            outcome,
            Outcome::Unavailable {
                reason: "world threat vtable mismatch"
            }
        );
    }
}

fn split_capacities() -> Capacities {
    Capacities {
        rows: [2, 8, 8],
        reason_bytes: 64,
    }
}

fn direct(memory: &SliceMemory<'_>, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let _ticks = captured.capture(memory, 0, BASE, 0, true);
    captured
}

fn split(memory: &impl Memory, capacities: &Capacities) -> (Captured, SessionCopyReport) {
    let mut captured = Captured::with_capacities(capacities);
    let ctx = SessionCopyContext {
        hour: 0,
        force_full: true,
        base: 0,
        game_state: BASE,
    };
    let mut first = fixed_job();
    let report = copy_session_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    captured.raw_mode = report.raw_ok;
    let mut ticks = [0u64; 3];
    captured.decode_session_payload(0, &first, &mut ticks);
    (captured, report)
}

fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_eq!(
        split.manager.outcome, direct.manager.outcome,
        "manager outcome"
    );
    assert_eq!(
        split.sources.outcome, direct.sources.outcome,
        "sources outcome"
    );
    assert_eq!(
        split.aggregates.outcome, direct.aggregates.outcome,
        "aggregates outcome"
    );
    let len = split.manager.outcome.len();
    assert_eq!(
        &split.manager.rows[..len],
        &direct.manager.rows[..len],
        "manager rows"
    );
    let len = split.sources.outcome.len();
    assert_eq!(
        &split.sources.rows[..len],
        &direct.sources.rows[..len],
        "source rows"
    );
    assert_eq!(
        &split.reason_bytes.first[..split.reason_bytes.len],
        &direct.reason_bytes.first[..direct.reason_bytes.len],
        "reason arena"
    );
    assert_eq!(
        split.reason_bytes.required, direct.reason_bytes.required,
        "required"
    );
    let len = split.aggregates.outcome.len();
    assert_eq!(
        &split.aggregates.rows[..len],
        &direct.aggregates.rows[..len],
        "aggregate rows"
    );
}

#[test]
fn session_split_copy_matches_direct_walk() {
    use engine_native::{memory::CountedMemory, memory::ReadCounts};
    let capacities = split_capacities();
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let direct = direct(&memory, &capacities);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, _report) = split(&memory, &capacities);
    let counts = capture.finish();
    assert_matches_direct(&split, &direct);
    assert_eq!(split.manager.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.sources.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.aggregates.outcome, Outcome::Rows { len: 2 });
    let anchor = TableId::WorldTension;
    let mut expected = read_accounting::ZERO;
    // Manager word/header, two source pointers/52-byte heads/32-byte
    // inline reasons, the two-slot aggregate cache in one span: nine
    // requests, 272 bytes at one anchor.
    expected[anchor.index()] = ReadCounts {
        copy_calls: 9,
        spans: 9,
        bytes: 272,
        ..ReadCounts::ZERO
    };
    // Every hour reads (session_is_read_every_hour) unless its tension is
    // quiet (quiet_tension_hours_publish_the_held_read_without_copying).
    assert_eq!(counts, expected);
}

#[test]
fn session_staging_overflow_falls_back_to_typed_walk() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let ctx = SessionCopyContext {
        hour: 0,
        force_full: true,
        base: 0,
        game_state: BASE,
    };
    let mut tiny = SessionJob {
        log: crate::raw::RawLog::new(8, 8),
        game_state: 0,
        ..Default::default()
    };
    let report = copy_session_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
    assert!(tiny.log.overflowed);
    assert!(tiny.grow_for_next());
}

#[test]
fn session_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // Fixture holds 1 manager, 2 sources, 2 aggregates; exact-fit is
    // rows [1,2,2] with 8 reason bytes. u32 unread saturation is not
    // reachable with 2 rows and is covered by the walk's own saturating
    // arithmetic.
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    for (rows, reason) in [
        ([0, 0, 0], 0),
        ([1, 1, 1], 2),
        ([1, 2, 2], 8),
        ([2, 3, 3], 8),
        ([8, 8, 8], 64),
    ] {
        let capacities = Capacities {
            rows,
            reason_bytes: reason,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let _ticks = direct.capture(&memory, 0, BASE, 0, true);
        let mut split = Captured::with_capacities(&capacities);
        let ctx = SessionCopyContext {
            hour: 0,
            force_full: true,
            base: 0,
            game_state: BASE,
        };
        let mut first = fixed_job();
        let report = copy_session_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok, "rows={rows:?}");
        split.raw_mode = true;
        let mut ticks = [0u64; 3];
        split.decode_session_payload(0, &first, &mut ticks);
        assert_matches_direct(&split, &direct);
    }
    // Exact-fit arm pins Rows with all rows present.
    let capacities = Capacities {
        rows: [1, 2, 2],
        reason_bytes: 8,
    };
    let mut direct = Captured::with_capacities(&capacities);
    let _ = direct.capture(&memory, 0, BASE, 0, true);
    assert_eq!(direct.manager.outcome, Outcome::Rows { len: 1 });
    assert_eq!(direct.sources.outcome, Outcome::Rows { len: 2 });
    assert_eq!(direct.aggregates.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn session_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the session copy pass issues, recorded
    // through the real pass. The pass bulk-copies (32 B string headers,
    // 52/64 B manager/aggregate structs, 8 B words) wider than the
    // walk's scattered reads (4 B keys, 8 B words, 1 B flags inside);
    // sub-ranges serve from the enclosing bulk span at replay (proved
    // by session_bulk_header_serves_enclosed_length_word). Alignment
    // follows the engine layout (structs 8-aligned, headers 8-aligned).
    use engine_native::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: SliceMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = fixture();
    let memory = Recorder {
        inner: SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let mut job = fixed_job();
    copy_pass(&memory, 0, BASE, &mut job.log);
    assert!(job.log.raw_bytes.iter().any(|&b| b > 0));
    let sizes = memory.sizes.borrow().clone();
    for required in [8, 32, 52, 64] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn session_cold_job_is_provisioned_without_content_guesses() {
    let job = SessionJob::new();
    assert!(job.is_provisioned());
    assert_eq!(job.log.spans.capacity(), 0);
    assert_eq!(job.log.bytes.capacity(), 0);
    let capacities = Capacities::default();
    assert_eq!(capacities.rows, [1, 0, 0]);
    assert_eq!(capacities.reason_bytes, 0);
    let captured = Captured::vacant();
    assert_eq!(captured.allocated_bytes(), 0);
}

#[test]
fn session_bulk_header_serves_enclosed_length_word() {
    // S3 containment: the 32 B reason-string header is a bulk span, so
    // the walk's enclosed 8 B length/terminator reads inside it are
    // served from the same recorded bytes (raw.rs bulk rule). Proves
    // the sspan containment claim with overlapping/unaligned sub-reads.
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut job = fixed_job();
    copy_pass(&memory, 0, BASE, &mut job.log);
    assert!(job.log.has_bulk);
    let recorded = crate::raw::Recorded::build(&job.log);
    // Find a 32 B bulk header span and read an enclosed 8 B word at an
    // unaligned offset inside it.
    let header = job
        .log
        .spans
        .iter()
        .find(|span| span.ok && span.bulk && span.len == 32)
        .expect("bulk string header");
    let mut word = [0u8; 8];
    assert!(recorded.copy(header.addr + 3, &mut word));
    let mut expected = [0u8; 8];
    expected.copy_from_slice(&bytes[header.addr - BASE + 3..header.addr - BASE + 11]);
    assert_eq!(word, expected);
}

#[test]
fn session_growing_source_fault_is_not_retried_and_preserves_aggregate_sibling() {
    use std::cell::Cell;
    struct FaultOnce<'a> {
        inner: SliceMemory<'a>,
        attempts: Cell<usize>,
    }
    impl Memory for FaultOnce<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == FIRST && output.len() == 0x34 {
                self.attempts.set(self.attempts.get() + 1);
                if self.attempts.get() == 1 {
                    return false;
                }
            }
            self.inner.copy(address, output)
        }
    }
    let bytes = fixture();
    let memory = FaultOnce {
        inner: SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        attempts: Cell::new(0),
    };
    let mut job = SessionJob::default();
    assert!(
        copy_session_refresh_growing(
            &memory,
            SessionCopyContext {
                base: 0,
                game_state: BASE,
                hour: 0,
                force_full: true,
            },
            &mut job
        )
        .raw_ok
    );
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; N],
        reason_bytes: 0,
    });
    captured.decode_session_payload(0, &job, &mut [0; N]);
    assert_eq!(memory.attempts.get(), 1);
    assert_eq!(
        captured.outcomes(),
        [
            Outcome::Rows { len: 1 },
            Outcome::Unavailable {
                reason: "threat source fields unreadable"
            },
            Outcome::Rows { len: 2 },
        ]
    );
}
