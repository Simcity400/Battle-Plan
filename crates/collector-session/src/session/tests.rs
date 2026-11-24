use super::*;
use engine_native::cgame_state;
use engine_native_memory::{memory::SliceMemory, msvc_string};
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
        &(cthreat_source::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        &(cworld_threat::VTABLE_RVA.0 as u64).to_le_bytes(),
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

const SOURCE_STRIDE: usize = 0x100;

fn source_current(index: usize) -> i64 {
    index as i64 * 1_000 - 7
}

fn aggregate_total(index: usize) -> i64 {
    -(index as i64) * 3 - 1
}

fn many_sources(count: usize) -> Vec<u8> {
    let sources = SOURCE_ARRAY;
    let aggregates = (sources + 8 * count).next_multiple_of(0x100);
    let first = (aggregates + 8 * count).next_multiple_of(0x100);
    let mut bytes = vec![0; first + count * SOURCE_STRIDE - BASE];
    put(&mut bytes, BASE + cgame_state::WORLD_THREAT, &(MANAGER as u64).to_le_bytes());
    put(&mut bytes, MANAGER, &(cworld_threat::VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, MANAGER + cworld_threat::TOTAL, &(-125_682_i64).to_le_bytes());
    array(&mut bytes, MANAGER + cworld_threat::SOURCES, sources, count as u32);
    array(&mut bytes, MANAGER + cworld_threat::COUNTRY_AGGREGATES, aggregates, count as u32);
    for index in 0..count {
        let at = first + index * SOURCE_STRIDE;
        put(&mut bytes, sources + 8 * index, &(at as u64).to_le_bytes());
        put(&mut bytes, aggregates + 8 * index, &aggregate_total(index).to_le_bytes());
        source(&mut bytes, at, source_current(index), format!("r{}\0", index % 1_000).as_bytes());
    }
    bytes
}

fn job_for(sources: usize) -> SessionJob {
    SessionJob {
        log: RawLog::new(64 + 8 * sources, 16 * 1024 + 256 * sources),
        ..Default::default()
    }
}

fn small() -> (Capacities, Captured) {
    let capacities = Capacities {
        rows: [1, 2, 2],
        reason_bytes: 8,
    };
    let captured = Captured::with_capacities(&capacities);
    (capacities, captured)
}

fn presized_job() -> SessionJob {
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

#[test]
fn session_is_read_every_hour() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (capacities, mut captured) = small();
    let mut job = presized_job();
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
        ..presized_job()
    };
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
fn short_reason_storage_grows_and_a_broken_root_stays_distinct() {
    let (mut capacities, mut captured) = small();
    capacities.reason_bytes = 2;
    let mut captured_small = Captured::with_capacities(&capacities);
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let _ticks = captured_small.capture(&memory, 0, BASE, 0, true);
    assert_eq!(captured_small.outcomes()[1], Outcome::Rows { len: 2 });
    assert!(matches!(captured_small.tables()[1], TableView::Complete(_)));
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
    let mut first = presized_job();
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
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
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
    expected[anchor.index()] = ReadCounts {
        copy_calls: 9,
        spans: 9,
        bytes: 272,
        ..ReadCounts::ZERO
    };
    assert_eq!(counts, expected);
}

#[test]
fn session_truncation_matrix_matches_direct_with_same_caps() {
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
        let mut first = presized_job();
        let report = copy_session_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok, "rows={rows:?}");
        split.raw_mode = true;
        let mut ticks = [0u64; 3];
        split.decode_session_payload(0, &first, &mut ticks);
        assert_matches_direct(&split, &direct);
    }
    for count in [5, 1_500] {
        let bytes = many_sources(count);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        for cap in [0, 1, count - 1, count, count + 1, 4 * count] {
            let reasons = 8 * cap.min(count);
            let capacities = Capacities {
                rows: [1, cap, cap],
                reason_bytes: reasons,
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
            let mut first = job_for(count);
            let report = copy_session_refresh(&memory, ctx, &mut first);
            assert!(report.raw_ok, "count={count} cap={cap}");
            split.raw_mode = true;
            let mut ticks = [0u64; 3];
            split.decode_session_payload(0, &first, &mut ticks);
            assert_matches_direct(&split, &direct);
            let expected = Outcome::Rows { len: count };
            assert_eq!((split.sources.outcome, split.aggregates.outcome), (expected, expected), "every row of {count} is kept from an initial capacity of {cap}");
            let len = expected.len();
            let currents: Vec<i64> = split.sources.rows[..len].iter().map(|row| row.current_raw).collect();
            assert_eq!(currents, (0..len).map(source_current).collect::<Vec<_>>(), "sources keep their order");
            let totals: Vec<i64> = split.aggregates.rows[..len].iter().map(|row| row.total_raw).collect();
            assert_eq!(totals, (0..len).map(aggregate_total).collect::<Vec<_>>(), "aggregates keep their order");
        }
    }
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
fn session_bulk_header_serves_enclosed_length_word() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut job = presized_job();
    copy_pass(&memory, 0, BASE, &mut job.log);
    assert!(job.log.has_bulk);
    let recorded = crate::raw::Recorded::build(&job.log).unwrap();
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
fn session_source_fault_is_not_retried_and_preserves_aggregate_sibling() {
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
        copy_session_refresh(
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
