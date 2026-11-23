use super::*;
use crate::{columnar_file::HourWriter, fixtures};
use replay_frame::frame::{DecodedFrame, FrameDecoder, FrameEncoder, TableSegment};
use replay_data::rows::{CountryRow, DivisionRow, ResearchInputRow, TableRows, sample_extended_table, tables};
use replay_columnar::{envelope, writer::Work};
use std::sync::Arc;

struct Applied {
    writer: HourWriter<std::io::Cursor<Vec<u8>>>,
    tables: Vec<TableId>,
    mode: CaptureMode,
    running: bool,
}

impl Applied {
    fn new(tables: &[TableId], mode: CaptureMode) -> Self {
        Self { writer: fixtures::writer(recorder_derive::writers(), fixtures::identity(b"native frame", mode.byte()), |_| {}), tables: tables.to_vec(), mode, running: false }
    }

    fn cut(&mut self) {
        self.running = false;
    }

    fn hour(&mut self, input: &FrameInput, prepared: &FrameHour) -> DecodedFrame {
        if !self.running {
            self.writer.start_run(input.branch, envelope::epoch::START, input.hour).unwrap();
            self.running = true;
        }
        let sections: Vec<u8> = prepared.sections().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
        self.writer.hour(&prepared.manifest, &sections).unwrap();
        let mut frame = self.writer.frame(&self.tables, (input.branch, input.hour), self.mode).unwrap();
        let columns: Vec<usize> = replay_recording::schema::schema().unwrap().iter().map(Vec::len).collect();
        let manifest = replay_columnar::manifest::Manifest::read(&prepared.manifest, &columns).unwrap();
        frame.metrics.total_ns = manifest.total_ns;
        frame.metrics.pool_bytes = manifest.pool_bytes;
        frame.metrics.dropped_frames = manifest.dropped_frames;
        for &table in &self.tables {
            let index = table.index();
            if manifest.read.get(index / 64).is_some_and(|word| word >> (index % 64) & 1 != 0) {
                frame.metrics.table_ns[index] = replay_frame::frame::READ_TIME_NOT_RECORDED;
            }
        }
        frame
    }
}

const TABLES: [TableId; 3] = [
    TableId::Countries,
    TableId::Divisions,
    TableId::ResearchInputs,
];

struct Paths {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
    writer: FrameWriter,
    rows: FrameWriter,
    swept: FrameWriter,
    laned: FrameWriter,
    applied: Applied,
    tables: Vec<TableId>,
    mode: CaptureMode,
}

impl Paths {
    fn new() -> Self {
        Self::of(&TABLES, CaptureMode::Full)
    }

    fn of(tables: &[TableId], mode: CaptureMode) -> Self {
        Self {
            encoder: FrameEncoder::new(),
            decoder: FrameDecoder::new(),
            writer: FrameWriter::new().unwrap(),
            rows: FrameWriter::with_staging(native_columns::Staging::Rows).unwrap(),
            swept: FrameWriter::with_sweep(Sweep::Every).unwrap(),
            laned: FrameWriter::new().unwrap(),
            applied: Applied::new(tables, mode),
            tables: tables.to_vec(),
            mode,
        }
    }

    fn restart_rows(&mut self) {
        self.encoder = FrameEncoder::new();
        self.decoder = FrameDecoder::new();
    }

    fn frame(&self, hour: i32, inputs: Vec<(TableInput, bool)>) -> FrameInput {
        frame_of(self.mode, &self.tables, hour, inputs)
    }

    fn check(&mut self, hour: i32, inputs: Vec<(TableInput, bool)>) {
        let input = self.frame(hour, inputs);
        let tables = self.tables.clone();
        let (rows, columnar) = self.hour(input);
        same(&tables, &rows, &columnar);
    }

    fn hour(&mut self, input: FrameInput) -> (DecodedFrame, DecodedFrame) {
        let first = input.hour == 1;
        let mut segments: [Option<TableSegment>; TABLE_COUNT] = std::array::from_fn(|_| None);
        if input.hour % 2 == 0 {
            let context = self.encoder.frame_context(input.branch, input.hour, first);
            for &table in &self.tables {
                let segment = self.encoder.encode_segment(&context, table, &input.tables[table.index()]);
                segments[table.index()] = segment.ok();
            }
        }
        let mut row_frame = self.encoder.prepare_bytes_with(input.clone(), first, &mut segments).unwrap();
        let keyframe = row_frame.frame().keyframe();
        let plain = Some(RowFrame {
            baselines: None,
            keyframe,
            changed: &[],
        });
        let prepared = self.writer.hour_on(&input, &[], plain, &OneThread).unwrap();
        let staged = self.rows.hour_on(&input, &[], plain, &OneThread).unwrap();
        let swept = self.swept.hour_on(&input, &[], plain, &OneThread).unwrap();
        let baselines = Some(RowFrame {
            baselines: Some(&self.encoder),
            keyframe,
            changed: row_frame.changed_rows(),
        });
        let laned = self.laned.hour_on(&input, &[], baselines, &Threads).unwrap();
        for other in [&staged, &swept, &laned] {
            assert_eq!(other.manifest, prepared.manifest, "hour {}", input.hour);
            assert!(other.sections().eq(prepared.sections()), "hour {}", input.hour);
        }
        let columnar = self.applied.hour(&input, &prepared);
        self.writer.commit(prepared).unwrap();
        self.rows.commit(staged).unwrap();
        self.swept.commit(swept).unwrap();
        self.laned.commit_on(laned, &Threads).unwrap();
        same_restarts(&mut [&mut self.writer, &mut self.rows, &mut self.swept, &mut self.laned]);
        let encoded = row_frame.take_frame();
        self.encoder.commit(row_frame).unwrap();
        (self.decoder.apply(&encoded.decode().unwrap()).unwrap(), columnar)
    }
}

struct Threads;

impl Lanes for Threads {
    fn count(&self) -> usize {
        3
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        std::thread::scope(|scope| {
            for _ in 1..self.count() {
                scope.spawn(job);
            }
            job();
        });
    }
}

fn same_restarts(writers: &mut [&mut FrameWriter]) {
    let restarts: Vec<_> = writers.iter_mut().map(|writer| writer.restart().unwrap()).collect();
    let (strings, tables) = &restarts[0];
    for (other_strings, other_tables) in &restarts[1..] {
        assert_eq!(other_strings, strings);
        assert_eq!(other_tables.len(), tables.len());
        for (expected, actual) in tables.iter().zip(other_tables) {
            assert_eq!((&actual.bytes, &actual.ids), (&expected.bytes, &expected.ids));
        }
    }
}

fn frame(hour: i32, tables: [(TableInput, bool); 3]) -> FrameInput {
    frame_of(CaptureMode::Full, &TABLES, hour, tables.into())
}

fn frame_of(
    mode: CaptureMode,
    tables: &[TableId],
    hour: i32,
    given: Vec<(TableInput, bool)>,
) -> FrameInput {
    let mut inputs: [TableInput; TABLE_COUNT] = std::array::from_fn(|index| {
        TableInput::Sampled(if scope::records(mode, TableId::ALL[index]) {
            TableView::Unavailable {
                reason: "not in this test".into(),
            }
        } else {
            TableView::NotRecorded { mode }
        })
    });
    let mut table_ns = [0; TABLE_COUNT];
    for (&table, (input, read)) in tables.iter().zip(given) {
        inputs[table.index()] = input;
        if read {
            table_ns[table.index()] = 100 + hour as u64;
        }
    }
    FrameInput {
        branch: 1,
        hour,
        tables: inputs,
        metrics: Metrics {
            total_ns: 1_000 + hour as u64,
            table_ns,
            pool_bytes: 7,
            dropped_frames: 0,
        },
    }
}

fn country(id: u32, stability: i64) -> CountryRow {
    CountryRow {
        id,
        canonical_slot: 1_000 + id,
        tag: format!("C{id:03}").into_bytes(),
        political_power_raw: (id % 2 == 0).then_some(i64::from(id) * 10),
        stability_raw: stability,
        ..Default::default()
    }
}

fn division(id: u64, hit_points: i64) -> DivisionRow {
    let mut row = DivisionRow {
        id,
        roster_owner: 1,
        logical_owner: 1,
        expeditionary_owner: 1,
        province: 7,
        template: 3,
        hit_points,
        organisation: 1_234_567,
        ..Default::default()
    };
    row.round_to_display();
    row
}

fn input(country: u32, index: u32, progress: i64) -> ResearchInputRow {
    ResearchInputRow {
        country,
        index,
        token: format!("T{country}.{index}").into_bytes(),
        progress_raw: progress,
        ..Default::default()
    }
}

fn countries(rows: Vec<CountryRow>) -> TableInput {
    TableInput::Sampled(TableView::Complete(TableRows::Countries(rows)))
}

fn divisions(rows: Vec<DivisionRow>) -> TableInput {
    TableInput::Sampled(TableView::Complete(TableRows::Divisions(rows)))
}

fn unavailable(reason: &str) -> TableInput {
    TableInput::Sampled(TableView::Unavailable {
        reason: reason.into(),
    })
}

fn same(tables: &[TableId], rows: &DecodedFrame, columnar: &DecodedFrame) {
    for &table in tables {
        let index = table.index();
        assert_eq!(
            *columnar.tables[index], *rows.tables[index],
            "{table:?} at hour {}",
            rows.hour
        );
        assert_eq!(
            columnar.tables[index].sampled_at(),
            rows.tables[index].sampled_at(),
            "{table:?} sample at hour {}",
            rows.hour
        );
        assert_eq!(
            columnar.metrics.table_ns[index] != 0,
            rows.metrics.table_ns[index] != 0,
            "{table:?} read at hour {}",
            rows.hour
        );
    }
    let metrics = |frame: &DecodedFrame| {
        (
            frame.metrics.total_ns,
            frame.metrics.pool_bytes,
            frame.metrics.dropped_frames,
        )
    };
    assert_eq!(metrics(columnar), metrics(rows));
}

#[test]
fn every_conversion_form_walks_to_the_row_recordings_views() {
    let mut paths = Paths::new();
    let research = |rows: Vec<ResearchInputRow>| Arc::new(TableRows::ResearchInputs(rows));
    let first_research = research(vec![input(1, 0, 10), input(1, 1, 11), input(2, 0, 20)]);
    let (rows, columnar) = paths.hour(frame(
        1,
        [
            (countries(vec![country(1, 10), country(2, 20), country(3, 30)]), true),
            (divisions(vec![division(10, 1_500_000), division(11, 2_000_000)]), true),
            (TableInput::SharedComplete(Arc::clone(&first_research)), true),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::Countries>(1, 2, false)
        .unwrap();
    let (changed, _) = paths
        .encoder
        .comparison_baseline::<tables::Divisions>(1, 2, false)
        .unwrap();
    let (sliced, previous) = paths
        .encoder
        .slice_baseline(TableId::ResearchInputs, 1, 2, false)
        .unwrap();
    let update = SliceUpdate::new(
        previous,
        TableRows::ResearchInputs(vec![input(1, 0, 15), input(1, 1, 11)]),
        |owner| owner == 1,
        |_| true,
    )
    .unwrap();
    let (rows, columnar) = paths.hour(frame(
        2,
        [
            (TableInput::ComparedEqual(equal), true),
            (
                TableInput::ComparedChanges {
                    token: changed,
                    indices: vec![0],
                    rows: TableRows::Divisions(vec![division(10, 1_200_000)]),
                },
                true,
            ),
            (
                TableInput::SliceUpdate {
                    token: sliced,
                    update: Arc::new(update),
                },
                true,
            ),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::Divisions>(1, 3, false)
        .unwrap();
    let (sliced, previous) = paths
        .encoder
        .slice_baseline(TableId::ResearchInputs, 1, 3, false)
        .unwrap();
    let update = SliceUpdate::new(
        previous,
        TableRows::ResearchInputs(Vec::new()),
        |_| false,
        |owner| owner != 2,
    )
    .unwrap();
    let (rows, columnar) = paths.hour(frame(
        3,
        [
            (countries(vec![country(1, 10), country(2, 25), country(3, 30)]), false),
            (TableInput::ComparedEqual(equal), false),
            (
                TableInput::SliceUpdate {
                    token: sliced,
                    update: Arc::new(update),
                },
                false,
            ),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::ResearchInputs>(1, 4, false)
        .unwrap();
    let (rows, columnar) = paths.hour(frame(
        4,
        [
            (
                TableInput::Sampled(TableView::Truncated {
                    rows: TableRows::Countries(vec![country(1, 12)]),
                    missing: 2,
                }),
                true,
            ),
            (divisions(vec![division(10, 1_200_000), division(12, 900_000)]), true),
            (TableInput::ComparedEqual(equal), true),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (rows, columnar) = paths.hour(frame(
        5,
        [
            (countries(vec![country(1, 12), country(2, 25), country(3, 30)]), true),
            (unavailable("divisions refused"), true),
            (
                TableInput::SharedComplete(research(vec![input(1, 0, 16), input(3, 0, 30)])),
                false,
            ),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::Countries>(1, 6, false)
        .unwrap();
    let (rows, columnar) = paths.hour(frame(
        6,
        [
            (TableInput::ComparedEqual(equal), false),
            (divisions(vec![division(10, 1_100_000)]), true),
            (
                TableInput::SharedComplete(research(vec![input(1, 0, 16), input(3, 0, 30)])),
                false,
            ),
        ],
    ));
    same(&TABLES, &rows, &columnar);

    let (changed, _) = paths
        .encoder
        .comparison_baseline::<tables::Divisions>(1, 7, false)
        .unwrap();
    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::ResearchInputs>(1, 7, false)
        .unwrap();
    paths.check(
        7,
        vec![
            (countries(vec![country(1, 12), country(2, 25), country(3, 30)]), false),
            (
                TableInput::ComparedChanges {
                    token: changed,
                    indices: vec![0],
                    rows: TableRows::Divisions(vec![division(10, 1_000_000)]),
                },
                false,
            ),
            (TableInput::ComparedEqual(equal), false),
        ],
    );
    let (equal, _) = paths
        .encoder
        .comparison_baseline::<tables::ResearchInputs>(1, 8, false)
        .unwrap();
    paths.check(
        8,
        vec![
            (
                TableInput::Sampled(TableView::Truncated {
                    rows: TableRows::Countries(vec![country(2, 26)]),
                    missing: 2,
                }),
                false,
            ),
            (divisions(vec![division(10, 1_000_000)]), true),
            (TableInput::ComparedEqual(equal), true),
        ],
    );

    let (sliced, _) = paths
        .encoder
        .slice_baseline(TableId::ResearchInputs, 1, 9, false)
        .unwrap();
    let update = SliceUpdate::keyed(
        vec![input(1, 0, 0).key(), input(3, 0, 0).key()],
        TableRows::ResearchInputs(vec![input(1, 0, 17)]),
    );
    paths.check(
        9,
        vec![
            (countries(vec![country(1, 12), country(2, 26), country(3, 30)]), true),
            (divisions(vec![division(10, 1_000_000)]), false),
            (
                TableInput::SliceUpdate {
                    token: sliced,
                    update: Arc::new(update),
                },
                true,
            ),
        ],
    );

    let (sliced, _) = paths
        .encoder
        .slice_baseline(TableId::ResearchInputs, 1, 10, false)
        .unwrap();
    let update = SliceUpdate {
        ranges: vec![input(3, 0, 0).key()..input(4, 0, 0).key()],
        owners: Vec::new(),
        keys: Vec::new(),
        rows: TableRows::ResearchInputs(vec![input(3, 0, 31), input(3, 1, 32)]),
    };
    paths.check(
        10,
        vec![
            (countries(vec![country(1, 12), country(2, 26), country(3, 30)]), false),
            (divisions(vec![division(10, 1_000_000)]), false),
            (
                TableInput::SliceUpdate {
                    token: sliced,
                    update: Arc::new(update),
                },
                false,
            ),
        ],
    );
}

#[test]
fn an_off_slot_refusal_carries_complete_rows_as_the_row_encoder_does() {
    let (table, first, second) = TableId::ALL
        .into_iter()
        .find_map(|table| {
            let first = sample_extended_table(table, 1)?;
            let second = sample_extended_table(table, 2)?;
            (replay_data::cadence::is_staggered(table)
                && scope::records(CaptureMode::Full, table)
                && !first.is_empty()
                && first != second)
                .then_some((table, first, second))
        })
        .expect("a staggered table with fixtures");
    let mut paths = Paths::of(&[table], CaptureMode::Full);
    let skipped = || unavailable(replay_data::cadence::SKIPPED_REASON);
    paths.check(1, vec![(skipped(), false)]);
    paths.check(2, vec![(TableInput::Sampled(TableView::Complete(first)), true)]);
    paths.check(3, vec![(skipped(), false)]);
    paths.check(4, vec![(skipped(), false)]);
    paths.check(5, vec![(TableInput::Sampled(TableView::Complete(second)), true)]);
    paths.check(6, vec![(unavailable("refused this hour"), true)]);
    paths.check(7, vec![(skipped(), false)]);
}

#[test]
fn a_map_recording_walks_its_unrecorded_tables_as_not_recorded() {
    let unrecorded = TableId::ALL
        .into_iter()
        .find(|&table| !scope::records(CaptureMode::Map, table))
        .expect("Map leaves some table out");
    assert!(scope::records(CaptureMode::Map, TableId::Countries));
    let mut paths = Paths::of(&[TableId::Countries, unrecorded], CaptureMode::Map);
    let not_recorded = || {
        TableInput::Sampled(TableView::NotRecorded {
            mode: CaptureMode::Map,
        })
    };
    paths.check(
        1,
        vec![(countries(vec![country(1, 10)]), true), (not_recorded(), false)],
    );
    paths.check(
        2,
        vec![
            (countries(vec![country(1, 11), country(2, 20)]), true),
            (not_recorded(), false),
        ],
    );
}

#[test]
fn omitted_tables_walk_as_decoded_and_keep_no_read_bit() {
    let format = FormatVersion::CURRENT.version();
    let mut tables = vec![TableId::Countries, TableId::DivisionModelInputs];
    assert!(!scope::records(CaptureMode::Full, TableId::DivisionModelInputs));
    let retired = TableId::ALL
        .into_iter()
        .find(|table| !table.last_change().present(format));
    tables.extend(retired);
    let mut paths = Paths::of(&tables, CaptureMode::Full);
    for hour in 1..=2 {
        let mut inputs = vec![
            (countries(vec![country(1, 10 + hour)]), true),
            (
                TableInput::Sampled(TableView::NotRecorded {
                    mode: CaptureMode::Full,
                }),
                true,
            ),
        ];
        if retired.is_some() {
            inputs.push((unavailable("a retired table's producer"), true));
        }
        paths.check(hour as i32, inputs);
    }
}

#[test]
fn a_truncated_first_hour_walks_as_the_keyframe_refusal() {
    let mut paths = Paths::of(&[TableId::Countries], CaptureMode::Full);
    let truncated = |rows: Vec<CountryRow>| {
        TableInput::Sampled(TableView::Truncated {
            rows: TableRows::Countries(rows),
            missing: 3,
        })
    };
    paths.check(1, vec![(truncated(vec![country(1, 10)]), true)]);
    paths.check(2, vec![(countries(vec![country(1, 10), country(2, 20)]), true)]);
    paths.check(3, vec![(truncated(vec![country(2, 21)]), true)]);
}

#[test]
fn a_reread_shared_table_writes_only_its_differing_rows() {
    let mut paths = Paths::new();
    let shared = |rows: Vec<CountryRow>| TableInput::SharedComplete(Arc::new(TableRows::Countries(rows)));
    let others = || {
        (
            divisions(vec![division(1, 1_000)]),
            TableInput::Sampled(TableView::Complete(TableRows::ResearchInputs(vec![input(1, 0, 5)]))),
        )
    };
    let (divisions_1, research_1) = others();
    paths.check(1, vec![(shared(vec![country(1, 10), country(2, 20)]), true), (divisions_1, true), (research_1, true)]);
    let (divisions_2, research_2) = others();
    paths.check(2, vec![(shared(vec![country(1, 10), country(2, 25)]), true), (divisions_2, true), (research_2, true)]);
    let (divisions_3, research_3) = others();
    paths.check(3, vec![(shared(vec![country(1, 10), country(2, 25), country(3, 30)]), true), (divisions_3, true), (research_3, true)]);
}

#[test]
fn complete_tables_in_their_held_order_walk_to_the_row_recordings_views() {
    let mut paths = Paths::of(&[TableId::Countries, TableId::Divisions], CaptureMode::Full);
    let hour = |ids: std::ops::RangeInclusive<u32>, stability: fn(u32) -> i64, hit: i64| {
        vec![
            (countries(ids.clone().map(|id| country(id, stability(id))).collect()), true),
            (
                divisions(ids.map(|id| division(u64::from(id), hit + i64::from(id))).collect()),
                true,
            ),
        ]
    };
    paths.check(1, hour(1..=20, |id| i64::from(id) * 10, 1_000_000));
    paths.check(2, hour(1..=20, |id| i64::from(id) * 10, 1_000_000));
    paths.check(3, hour(1..=20, |_| 500, 900_000));
    paths.check(4, hour(2..=22, |_| 500, 900_000));
    paths.check(5, hour(2..=22, |id| i64::from(id % 3), 900_000));
    paths.check(6, hour(2..=22, |id| i64::from(id % 3), 900_000));
}

fn works(writer: &FrameWriter) -> Vec<Work> {
    writer.tables().iter().map(TableWriter::work).collect()
}

#[test]
fn unchanged_tables_cost_their_writers_no_column() {
    let mut paths = Paths::new();
    let width = |table: TableId| paths.writer.columns()[table.index()] as u64;
    let (c, d, r) = (
        width(TableId::Countries),
        width(TableId::Divisions),
        width(TableId::ResearchInputs),
    );
    let w = |hours, columns| Work { hours, columns };
    let expect = |paths: &Paths, hour: i32, touched: [Work; 3], swept: [Work; 3]| {
        assert_eq!(works(&paths.rows), works(&paths.writer), "hour {hour}");
        for (writer, expected) in [(&paths.writer, touched), (&paths.swept, swept)] {
            for (index, work) in works(writer).into_iter().enumerate() {
                let table = TableId::ALL[index];
                let at = TABLES.iter().position(|&given| given == table);
                let expected = at.map_or(Work::default(), |at| expected[at]);
                assert_eq!(work, expected, "{table:?} at hour {hour}");
            }
        }
    };
    let held = |tag: &[u8]| {
        let mut third = country(3, 30);
        third.tag = tag.to_vec();
        countries(vec![country(1, 10), country(2, 20), third])
    };
    let tokens = |paths: &Paths, hour: i32| {
        let encoder = &paths.encoder;
        let divisions = encoder.comparison_baseline::<tables::Divisions>(1, hour, false);
        let research = encoder.comparison_baseline::<tables::ResearchInputs>(1, hour, false);
        (divisions.unwrap().0, research.unwrap().0)
    };
    let no_change = |token| TableInput::ComparedChanges {
        token,
        indices: Vec::new(),
        rows: TableRows::Divisions(Vec::new()),
    };
    paths.check(
        1,
        vec![
            (held(b"C003"), true),
            (
                divisions(vec![division(10, 1_500_000), division(11, 2_000_000)]),
                true,
            ),
            (
                TableInput::Sampled(TableView::Complete(TableRows::ResearchInputs(vec![
                    input(1, 0, 10),
                    input(1, 1, 11),
                    input(2, 0, 20),
                ]))),
                true,
            ),
        ],
    );
    let swept = [w(1, 2 * c), w(1, 2 * d), w(1, 2 * r)];
    expect(&paths, 1, [w(1, c), w(1, d), w(1, r)], swept);
    let (compared, equal) = tokens(&paths, 2);
    paths.check(
        2,
        vec![
            (held(b"C003"), true),
            (no_change(compared), true),
            (TableInput::ComparedEqual(equal), true),
        ],
    );
    let swept = [w(2, 4 * c), w(2, 4 * d), w(1, 2 * r)];
    expect(&paths, 2, [w(2, 2 * c), w(1, d), w(1, r)], swept);
    let (compared, equal) = tokens(&paths, 3);
    paths.check(
        3,
        vec![
            (held(b"C003"), false),
            (TableInput::ComparedEqual(compared), false),
            (TableInput::ComparedEqual(equal), false),
        ],
    );
    let swept = [w(3, 6 * c), w(2, 4 * d), w(1, 2 * r)];
    expect(&paths, 3, [w(3, 2 * c), w(1, d), w(1, r)], swept);
    let (compared, equal) = tokens(&paths, 4);
    paths.check(
        4,
        vec![
            (held(b"Z003"), true),
            (no_change(compared), false),
            (TableInput::ComparedEqual(equal), true),
        ],
    );
    let swept = [w(4, 8 * c), w(3, 6 * d), w(1, 2 * r)];
    expect(&paths, 4, [w(4, 2 * c + 1), w(1, d), w(1, r)], swept);
    for hour in 5..=6 {
        let (compared, equal) = tokens(&paths, hour);
        paths.check(
            hour,
            vec![
                (held(b"Z003"), true),
                (no_change(compared), true),
                (TableInput::ComparedEqual(equal), true),
            ],
        );
    }
    let swept = [w(6, 12 * c), w(5, 10 * d), w(1, 2 * r)];
    expect(&paths, 6, [w(6, 2 * c + 2), w(1, d), w(1, r)], swept);
}

struct Converters {
    live: FrameWriter,
    converted: FrameWriter,
    rows: FrameWriter,
    swept: FrameWriter,
    laned: FrameWriter,
    encoder: FrameEncoder,
    decoder: FrameDecoder,
}

fn recorded_hour(
    writers: &mut Converters,
    hour: i32,
    tables: [(TableInput, bool); 3],
    deltas: [Option<Changed<'_>>; 3],
) {
    let Converters {
        live,
        converted,
        rows,
        swept,
        laned,
        encoder,
        decoder,
    } = writers;
    let input = frame(hour, tables);
    let encoded = encoder.encode_input(input.clone(), hour == 1).unwrap();
    let decoded = decoder.apply(&encoded).unwrap();
    let mut recorded: Vec<Source<'_>> = decoded
        .tables
        .iter()
        .map(|view| Source {
            sample: view.sampled_at(),
            delta: None,
        })
        .collect();
    for (table, delta) in TABLES.iter().zip(deltas) {
        recorded[table.index()].delta = delta;
    }
    let expected = live.hour(&input).unwrap();
    let actual = converted.hour_with(&input, &recorded).unwrap();
    let staged = rows.hour_with(&input, &recorded).unwrap();
    let every = swept.hour_with(&input, &recorded).unwrap();
    let on_lanes = laned.hour_on(&input, &recorded, None, &Threads).unwrap();
    for other in [&actual, &staged, &every, &on_lanes] {
        assert_eq!(other.manifest, expected.manifest, "hour {hour}");
        assert!(other.sections().eq(expected.sections()), "hour {hour}");
    }
    live.commit(expected).unwrap();
    converted.commit(actual).unwrap();
    rows.commit(staged).unwrap();
    swept.commit(every).unwrap();
    laned.commit_on(on_lanes, &Threads).unwrap();
}

#[test]
fn a_recorded_delta_writes_what_its_complete_rows_write() {
    let mut writers = Converters {
        live: FrameWriter::new().unwrap(),
        converted: FrameWriter::new().unwrap(),
        rows: FrameWriter::with_staging(native_columns::Staging::Rows).unwrap(),
        swept: FrameWriter::with_sweep(Sweep::Every).unwrap(),
        laned: FrameWriter::new().unwrap(),
        encoder: FrameEncoder::new(),
        decoder: FrameDecoder::new(),
    };
    let key = |row: &CountryRow| row.key();
    let division_key = |id| division(id, 0).key();
    let unchanged = Some(Changed {
        keys: &[],
        removed: &[],
    });
    let research = || unavailable("research refused");
    recorded_hour(
        &mut writers,
        1,
        [
            (countries(vec![country(1, 10), country(2, 20), country(3, 30)]), true),
            (divisions(vec![division(10, 1_500_000), division(11, 2_000_000)]), true),
            (research(), true),
        ],
        [unchanged, unchanged, None],
    );
    let changed = [key(&country(2, 0)), key(&country(4, 0))];
    let (kept, removed) = ([division_key(10)], [division_key(11)]);
    recorded_hour(
        &mut writers,
        2,
        [
            (
                countries(vec![country(1, 10), country(2, 25), country(3, 30), country(4, 40)]),
                true,
            ),
            (divisions(vec![division(10, 1_200_000)]), true),
            (research(), true),
        ],
        [
            Some(Changed {
                keys: &changed,
                removed: &[],
            }),
            Some(Changed {
                keys: &kept,
                removed: &removed,
            }),
            None,
        ],
    );
    let truncated = TableInput::Sampled(TableView::Truncated {
        rows: TableRows::Countries(vec![country(1, 12)]),
        missing: 3,
    });
    recorded_hour(
        &mut writers,
        3,
        [
            (truncated, true),
            (divisions(vec![division(10, 1_200_000), division(12, 900_000)]), true),
            (research(), true),
        ],
        [
            None,
            Some(Changed {
                keys: &[division_key(12)],
                removed: &[],
            }),
            None,
        ],
    );
    recorded_hour(
        &mut writers,
        4,
        [
            (
                countries(vec![country(1, 12), country(2, 26), country(3, 30), country(4, 40)]),
                true,
            ),
            (unavailable("divisions refused"), true),
            (research(), true),
        ],
        [unchanged, None, None],
    );
    recorded_hour(
        &mut writers,
        5,
        [
            (
                countries(vec![country(1, 12), country(2, 26), country(3, 30), country(4, 40)]),
                false,
            ),
            (divisions(vec![division(10, 1_100_000), division(12, 900_000)]), true),
            (research(), true),
        ],
        [unchanged, unchanged, None],
    );
    let Converters {
        live,
        converted,
        rows,
        swept,
        laned,
        ..
    } = &mut writers;
    same_restarts(&mut [live, converted, rows, swept, laned]);
}

#[test]
fn a_delta_key_without_its_row_is_refused() {
    let mut writer = FrameWriter::new().unwrap();
    let rows = || countries(vec![country(1, 10)]);
    let tables = || [(rows(), true), (divisions(Vec::new()), true), (unavailable("none"), true)];
    let first = writer.hour(&frame(1, tables())).unwrap();
    writer.commit(first).unwrap();
    let mut recorded = vec![Source::default(); TABLE_COUNT];
    let absent = [country(2, 0).key()];
    recorded[TableId::Countries.index()].delta = Some(Changed {
        keys: &absent,
        removed: &[],
    });
    assert!(writer.hour_with(&frame(2, tables()), &recorded).is_err());
    assert!(writer.hour(&frame(2, tables())).is_err());
}

#[test]
fn new_strings_on_lanes_are_interned_in_table_order() {
    let mut paths = Paths::new();
    let named = |id: u32, hour: i32| CountryRow {
        tag: format!("H{hour}C{id}").into_bytes(),
        ..country(id, 10 + i64::from(id))
    };
    for hour in 1..=6 {
        let mut rows = vec![country(1, 10), named(2, hour), named(3, hour / 2)];
        if hour % 2 == 0 {
            rows.push(named(4, hour));
        }
        let research = if hour == 4 {
            unavailable(&format!("not read at hour {hour}"))
        } else {
            TableInput::Sampled(TableView::Complete(TableRows::ResearchInputs(vec![
                input(1, 0, 5),
                input(u32::try_from(hour).unwrap(), 1, 7),
            ])))
        };
        paths.check(
            hour,
            vec![
                (countries(rows), true),
                (divisions(vec![division(1, 1_000 + i64::from(hour))]), hour % 3 != 0),
                (research, true),
            ],
        );
    }
}

#[test]
fn complete_tables_diffed_with_the_encoders_baselines_write_their_complete_bytes() {
    let mut paths = Paths::new();
    let divisions_at = |hour: i32| {
        let mut ids: Vec<u64> = (1..=20).collect();
        if hour >= 3 {
            ids.retain(|&id| id != 2);
            ids.push(21);
        }
        let changed = (hour as u64 % 20) + 1;
        ids.into_iter()
            .map(|id| {
                let base = if hour >= 4 { 3_000_000 } else { 1_000_000 };
                let moved = (hour >= 2 && id == 5) || (hour >= 3 && id == 7) || (hour > 4 && id == changed);
                division(id, base + id as i64 * 100_000 + if moved { i64::from(hour) * 1_000_000 } else { 0 })
            })
            .collect()
    };
    for hour in 1..=30 {
        let mut members = vec![country(1, 10), country(2, 20 + i64::from(hour % 3))];
        if hour % 4 == 0 {
            members.push(country(3, 30));
        }
        paths.check(
            hour,
            vec![
                (countries(members), true),
                (divisions(divisions_at(hour)), true),
                (
                    TableInput::Sampled(TableView::Complete(TableRows::ResearchInputs(vec![input(1, 0, 5)]))),
                    hour % 2 == 0,
                ),
            ],
        );
    }
}

#[test]
fn unchanged_opaque_cells_of_a_dense_table_are_kept_with_the_complete_bytes() {
    use replay_data::rows::{RailCooldownRow, RailwayBuildingHealth, RailwayBuildingState, RailwaySelectionInputs, Recorded};
    let mut paths = Paths::of(&[TableId::RailCooldowns], CaptureMode::Full);
    let row = |province: u32, hour: i32| RailCooldownRow {
        province,
        cooldown_ticks: 100 - hour,
        building: Recorded::Value(Some(RailwayBuildingState {
            building: Some(RailwayBuildingHealth {
                level: if hour >= 5 && province == 3 { 2 } else { 1 },
                healthy: 1,
                partial: None,
            }),
        })),
        selection: if province == 7 && hour < 3 {
            Recorded::Value(None)
        } else {
            Recorded::Value(Some(RailwaySelectionInputs {
                levels: Some(vec![1, 2, i32::from(hour >= 4 && province == 2)]),
                pair_count: Some(2),
                building_level: Some(Some(1)),
            }))
        },
        selection_hour: Recorded::Value(Some(hour)),
        building_bound: Default::default(),
    };
    for hour in 1..=7 {
        let provinces = if hour == 7 { 1..=11 } else { 1..=10 };
        let rows = provinces.map(|province| row(province, hour)).collect();
        paths.check(
            hour,
            vec![(TableInput::Sampled(TableView::Complete(TableRows::RailCooldowns(rows))), true)],
        );
    }
    for count in [10_u32, 80_u32] {
        let make_row = |province: u32, hour: i32| RailCooldownRow {
            province,
            cooldown_ticks: if province == 2 && hour >= 3 { 55 } else { 10 },
            building: Recorded::Value(Some(RailwayBuildingState {
                building: Some(RailwayBuildingHealth {
                    level: 1,
                    healthy: 1,
                    partial: None,
                }),
            })),
            selection: Recorded::Value(Some(RailwaySelectionInputs {
                levels: Some(vec![1, 2, 3]),
                pair_count: Some(2),
                building_level: Some(Some(1)),
            })),
            selection_hour: Recorded::Value(if hour == 5 { None } else { Some(hour) }),
            building_bound: Recorded::Value(Some(true)),
        };
        let frame_of = |hour: i32, table: TableInput| {
            let mut tables = std::array::from_fn(|_| {
                TableInput::Sampled(TableView::Unavailable {
                    reason: "unused".into(),
                })
            });
            tables[TableId::RailCooldowns.index()] = table;
            let mut metrics = Metrics::default();
            metrics.table_ns[TableId::RailCooldowns.index()] = 1;
            FrameInput {
                branch: 1,
                hour,
                tables,
                metrics,
            }
        };
        let mut enc_ref = FrameEncoder::new();
        let mut enc_uni = FrameEncoder::new();
        let mut w_ref = FrameWriter::new().unwrap();
        let mut w_uni_with = FrameWriter::new().unwrap();
        let mut w_uni_no_base = FrameWriter::new().unwrap();
        let mut w_uni_rows = FrameWriter::with_staging(native_columns::Staging::Rows).unwrap();
        for hour in 1..=6 {
            let range = match hour {
                4 => 1..=(count + 1),
                6 => 2..=count,
                _ => 1..=count,
            };
            let full_rows: Vec<_> = range.clone().map(|p| make_row(p, hour)).collect();
            let ref_input = frame_of(
                hour,
                TableInput::SharedComplete(Arc::new(TableRows::RailCooldowns(full_rows.clone()))),
            );
            let ref_prepared = enc_ref.prepare_bytes(ref_input.clone(), false).unwrap();
            let ref_hour = w_ref
                .hour_on(
                    &ref_input,
                    &[],
                    Some(RowFrame {
                        baselines: Some(&enc_ref),
                        keyframe: false,
                        changed: &[],
                    }),
                    &OneThread,
                )
                .unwrap();
            let ref_sections: Vec<(usize, Vec<u8>)> = ref_hour
                .sections()
                .map(|(i, b)| (i, b.to_vec()))
                .collect();
            let ref_manifest = ref_hour.manifest.clone();
            w_ref.commit(ref_hour).unwrap();
            enc_ref.commit(ref_prepared).unwrap();

            let uni_table = if hour == 1 {
                TableInput::SharedComplete(Arc::new(TableRows::RailCooldowns(full_rows)))
            } else {
                let (token, _) = enc_uni
                    .slice_baseline(TableId::RailCooldowns, 1, hour, false)
                    .unwrap();
                let (keys, changed) = match hour {
                    2 => (Vec::new(), Vec::new()),
                    3 => (vec![2], vec![make_row(2, hour)]),
                    4 => (vec![u128::from(count + 1)], vec![make_row(count + 1, hour)]),
                    5 => (vec![u128::from(count + 1)], Vec::new()),
                    6 => (vec![1], Vec::new()),
                    _ => unreachable!(),
                };
                TableInput::UniformOptionalI32 {
                    token,
                    column: 3,
                    value: if hour == 5 { None } else { Some(hour) },
                    update: Arc::new(SliceUpdate::keyed(keys, TableRows::RailCooldowns(changed))),
                }
            };
            let uni_input = frame_of(hour, uni_table);
            let uni_prepared = enc_uni.prepare_bytes(uni_input.clone(), false).unwrap();
            let uni_hour = w_uni_with
                .hour_on(
                    &uni_input,
                    &[],
                    Some(RowFrame {
                        baselines: Some(&enc_uni),
                        keyframe: false,
                        changed: &[],
                    }),
                    &OneThread,
                )
                .unwrap();
            let uni_sections: Vec<(usize, Vec<u8>)> = uni_hour
                .sections()
                .map(|(i, b)| (i, b.to_vec()))
                .collect();
            assert_eq!(uni_sections, ref_sections, "count={count} hour={hour}");
            assert_eq!(uni_hour.manifest, ref_manifest, "count={count} hour={hour}");
            w_uni_with.commit(uni_hour).unwrap();

            let no_base_hour = w_uni_no_base.hour(&uni_input).unwrap();
            let no_base_sections: Vec<(usize, Vec<u8>)> = no_base_hour
                .sections()
                .map(|(i, b)| (i, b.to_vec()))
                .collect();
            assert_eq!(no_base_sections, ref_sections, "no_base count={count} hour={hour}");
            w_uni_no_base.commit(no_base_hour).unwrap();

            let rows_hour = w_uni_rows.hour(&uni_input).unwrap();
            let rows_sections: Vec<(usize, Vec<u8>)> = rows_hour
                .sections()
                .map(|(i, b)| (i, b.to_vec()))
                .collect();
            assert_eq!(rows_sections, ref_sections, "rows count={count} hour={hour}");
            w_uni_rows.commit(rows_hour).unwrap();

            enc_uni.commit(uni_prepared).unwrap();
            assert_eq!(
                enc_uni.committed_rows(TableId::RailCooldowns),
                enc_ref.committed_rows(TableId::RailCooldowns),
                "baseline count={count} hour={hour}"
            );
        }
    }
}

struct Conversion {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
    writer: FrameWriter,
    laned: FrameWriter,
    applied: Applied,
}

impl Conversion {
    fn new() -> Self {
        Self {
            encoder: FrameEncoder::new(),
            decoder: FrameDecoder::new(),
            writer: FrameWriter::new().unwrap(),
            laned: FrameWriter::new().unwrap(),
            applied: Applied::new(&TABLES, CaptureMode::Full),
        }
    }

    fn cut(&mut self) {
        self.writer = FrameWriter::new().unwrap();
        self.laned = FrameWriter::new().unwrap();
        self.applied.cut();
    }

    fn hour(
        &mut self,
        hour: i32,
        tables: [(TableInput, bool); 3],
        keyframe: bool,
    ) -> (DecodedFrame, DecodedFrame) {
        let encoded = self.encoder.encode_input(frame(hour, tables), keyframe).unwrap();
        let decoded = self.decoder.apply(&encoded).unwrap();
        let input = FrameInput {
            branch: decoded.branch,
            hour: decoded.hour,
            tables: std::array::from_fn(|index| {
                TableInput::Sampled((*decoded.tables[index]).clone())
            }),
            metrics: decoded.metrics.stored(),
        };
        let recorded: Vec<Source<'_>> = decoded
            .tables
            .iter()
            .map(|view| Source {
                sample: view.sampled_at(),
                delta: None,
            })
            .collect();
        let prepared = self.writer.hour_with(&input, &recorded).unwrap();
        let on_lanes = self.laned.hour_on(&input, &recorded, None, &Threads).unwrap();
        assert_eq!(on_lanes.manifest, prepared.manifest, "hour {hour}");
        assert!(on_lanes.sections().eq(prepared.sections()), "hour {hour}");
        let walked = self.applied.hour(&input, &prepared);
        self.writer.commit(prepared).unwrap();
        self.laned.commit_on(on_lanes, &Threads).unwrap();
        (decoded, walked)
    }
}

#[test]
fn an_unread_snapshot_of_the_held_rows_keeps_no_sample_hour() {
    let mut conversion = Conversion::new();
    let tables = |read: bool| {
        [
            (countries(vec![country(1, 10), country(2, 20)]), read),
            (divisions(vec![division(1, 1_500_000)]), true),
            (unavailable("research refused"), true),
        ]
    };
    let at = TableId::Countries.index();
    let (decoded, walked) = conversion.hour(1, tables(true), false);
    same(&TABLES, &decoded, &walked);
    let (decoded, walked) = conversion.hour(2, tables(false), false);
    assert_eq!(decoded.tables[at].sampled_at(), Some(1));
    same(&TABLES, &decoded, &walked);
    let (decoded, walked) = conversion.hour(3, tables(false), true);
    assert_eq!(decoded.tables[at].sampled_at(), None);
    same(&TABLES, &decoded, &walked);
    assert_eq!(conversion.writer.tables()[at].acquisition().last_read, None);
}

#[test]
fn a_cut_runs_first_hour_keeps_a_truncated_view_and_the_recordings_sample_hours() {
    let mut conversion = Conversion::new();
    let (countries_at, divisions_at) = (TableId::Countries.index(), TableId::Divisions.index());
    let (decoded, walked) = conversion.hour(
        1,
        [
            (countries(vec![country(1, 10), country(2, 20)]), true),
            (divisions(vec![division(1, 1_500_000)]), true),
            (unavailable("research refused"), true),
        ],
        false,
    );
    same(&TABLES, &decoded, &walked);
    conversion.cut();
    let truncated = TableInput::Sampled(TableView::Truncated {
        rows: TableRows::Countries(vec![country(2, 25)]),
        missing: 3,
    });
    let (decoded, walked) = conversion.hour(
        2,
        [
            (truncated, true),
            (divisions(vec![division(1, 1_500_000)]), false),
            (unavailable("research refused"), true),
        ],
        false,
    );
    assert!(matches!(
        &*decoded.tables[countries_at],
        TableView::Truncated { missing: 3, .. }
    ));
    assert_eq!(decoded.tables[divisions_at].sampled_at(), Some(1));
    same(&TABLES, &decoded, &walked);
    let held = &conversion.writer.tables()[divisions_at];
    assert_eq!(held.acquisition().last_read, Some(1));
    assert_eq!(
        conversion.writer.tables()[countries_at].status(),
        Status::Truncated { missing: 3 }
    );
    let (decoded, walked) = conversion.hour(
        3,
        [
            (countries(vec![country(1, 10), country(2, 26)]), true),
            (divisions(vec![division(1, 1_400_000)]), true),
            (unavailable("research refused"), true),
        ],
        false,
    );
    same(&TABLES, &decoded, &walked);
}

#[test]
fn a_truncated_first_hour_is_refused_only_at_a_row_keyframe() {
    let truncated = || {
        TableInput::Sampled(TableView::Truncated {
            rows: TableRows::Countries(vec![country(1, 10)]),
            missing: 2,
        })
    };
    let input = || {
        frame(
            5,
            [
                (truncated(), true),
                (divisions(vec![division(1, 1_000)]), true),
                (unavailable("none"), true),
            ],
        )
    };
    let countries = TableId::Countries.index();
    let mut through = FrameWriter::new().unwrap();
    let row = Some(RowFrame {
        baselines: None,
        keyframe: false,
        changed: &[],
    });
    let hour = through.hour_on(&input(), &[], row, &OneThread).unwrap();
    assert_eq!(hour.statuses()[countries], Status::Truncated { missing: 2 });
    through.commit(hour).unwrap();
    let mut keyed = FrameWriter::new().unwrap();
    let row = Some(RowFrame {
        baselines: None,
        keyframe: true,
        changed: &[],
    });
    let hour = keyed.hour_on(&input(), &[], row, &OneThread).unwrap();
    assert!(matches!(hour.statuses()[countries], Status::Unavailable { .. }));
    keyed.commit(hour).unwrap();
}

struct Stopping {
    caller: std::thread::ThreadId,
    taken: std::sync::atomic::AtomicUsize,
    stopped: std::sync::atomic::AtomicUsize,
    steps: std::sync::atomic::AtomicUsize,
}

impl Lanes for Stopping {
    fn count(&self) -> usize {
        3
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        use std::sync::atomic::Ordering::SeqCst;
        self.taken.store(0, SeqCst);
        self.stopped.store(0, SeqCst);
        std::thread::scope(|scope| {
            scope.spawn(job);
            scope.spawn(job);
            job();
        });
        if self.stopped.load(SeqCst) == 2 {
            self.steps.fetch_add(1, SeqCst);
        }
    }

    fn proceeds(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        if std::thread::current().id() == self.caller {
            let waited = std::time::Instant::now();
            while self.stopped.load(SeqCst) < 2 {
                assert!(waited.elapsed() < std::time::Duration::from_secs(10), "helpers never stopped");
                std::thread::yield_now();
            }
            return true;
        }
        if self.taken.fetch_add(1, SeqCst) < 4 {
            return true;
        }
        self.stopped.fetch_add(1, SeqCst);
        false
    }
}

#[test]
fn lanes_that_stop_leave_their_tables_to_the_caller() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let lanes = Stopping {
        caller: std::thread::current().id(),
        taken: AtomicUsize::new(0),
        stopped: AtomicUsize::new(0),
        steps: AtomicUsize::new(0),
    };
    let mut alone = FrameWriter::new().unwrap();
    let mut laned = FrameWriter::new().unwrap();
    for hour in 1..=4 {
        let tag = |id: u32| CountryRow {
            tag: format!("H{hour}C{id}").into_bytes(),
            ..country(id, 10 + i64::from(hour))
        };
        let input = frame(
            hour,
            [
                (countries(vec![tag(1), country(2, 20)]), true),
                (divisions(vec![division(1, 1_000_000 * i64::from(hour))]), true),
                (unavailable(&format!("refused at {hour}")), true),
            ],
        );
        let expected = alone.hour(&input).unwrap();
        let actual = laned.hour_on(&input, &[], None, &lanes).unwrap();
        assert_eq!(actual.manifest, expected.manifest, "hour {hour}");
        assert!(actual.sections().eq(expected.sections()), "hour {hour}");
        alone.commit(expected).unwrap();
        laned.commit_on(actual, &lanes).unwrap();
        same_restarts(&mut [&mut alone, &mut laned]);
    }
    assert_eq!(lanes.steps.load(SeqCst), 8);
}

#[test]
fn an_unread_table_at_a_row_keyframe_keeps_its_sample() {
    let mut paths = Paths::new();
    let tables = |read: bool| {
        vec![
            (countries(vec![country(1, 10)]), read),
            (divisions(vec![division(1, 1_000_000)]), true),
            (unavailable("none"), true),
        ]
    };
    paths.check(1, tables(true));
    paths.check(2, tables(false));
    paths.restart_rows();
    let at = TableId::Countries.index();
    for hour in [3, 4] {
        let input = paths.frame(hour, tables(false));
        let (rows, columnar) = paths.hour(input);
        assert_eq!(*columnar.tables[at], *rows.tables[at], "hour {hour}");
        assert_eq!(rows.tables[at].sampled_at(), None, "hour {hour}");
        assert_eq!(columnar.tables[at].sampled_at(), Some(1), "hour {hour}");
    }
}

#[test]
fn a_baseline_of_an_hour_the_writer_missed_is_not_taken() {
    let mut encoder = FrameEncoder::new();
    let mut diffed = FrameWriter::new().unwrap();
    let mut plain = FrameWriter::new().unwrap();
    for hour in 1..=5 {
        let second = CountryRow {
            tag: if hour >= 4 { b"XX".to_vec() } else { b"C002".to_vec() },
            ..country(2, 20)
        };
        let input = frame(
            hour,
            [
                (countries(vec![country(1, 10), second]), true),
                (divisions(vec![division(1, 1_000_000)]), true),
                (unavailable("none"), true),
            ],
        );
        let mut row_frame = encoder.prepare_bytes(input.clone(), hour == 1).unwrap();
        if hour != 4 {
            let keyframe = row_frame.frame().keyframe();
            let with_baselines = Some(RowFrame {
                baselines: Some(&encoder),
                keyframe,
                changed: row_frame.changed_rows(),
            });
            let without = Some(RowFrame {
                baselines: None,
                keyframe,
                changed: &[],
            });
            let actual = diffed.hour_on(&input, &[], with_baselines, &OneThread).unwrap();
            let expected = plain.hour_on(&input, &[], without, &OneThread).unwrap();
            assert_eq!(actual.manifest, expected.manifest, "hour {hour}");
            assert!(actual.sections().eq(expected.sections()), "hour {hour}");
            diffed.commit(actual).unwrap();
            plain.commit(expected).unwrap();
        }
        let _ = row_frame.take_frame();
        encoder.commit(row_frame).unwrap();
    }
    same_restarts(&mut [&mut diffed, &mut plain]);
}

#[test]
fn a_complete_table_emptied_and_refilled_writes_its_complete_bytes() {
    let mut paths = Paths::new();
    let held: [&[u64]; 6] = [&[1, 2, 3], &[1, 2, 3], &[], &[], &[2], &[2, 4]];
    for (hour, ids) in (1..).zip(held) {
        let rows = ids.iter().map(|&id| division(id, 1_000_000)).collect();
        paths.check(
            hour,
            vec![
                (countries(vec![country(1, 10)]), true),
                (divisions(rows), true),
                (unavailable("none"), true),
            ],
        );
    }
}

#[test]
fn slice_updates_without_row_changes_match_recorded_changes() {
    let mut encoder = FrameEncoder::new();
    let mut expected_writer = FrameWriter::new().unwrap();
    let mut diff_writer = FrameWriter::new().unwrap();
    let research = |entries: &[(u32, u32, i64)]| {
        TableRows::ResearchInputs(
            entries
                .iter()
                .map(|&(country, index, progress_raw)| ResearchInputRow {
                    country,
                    index,
                    progress_raw,
                    ..Default::default()
                })
                .collect(),
        )
    };
    let initial = frame(
        1,
        [
            (countries(vec![country(1, 10)]), true),
            (
                TableInput::SharedComplete(Arc::new(TableRows::Divisions(vec![
                    division(1, 1_000_000),
                    division(2, 1_000_000),
                    division(3, 1_000_000),
                ]))),
                true,
            ),
            (
                TableInput::SharedComplete(Arc::new(research(&[
                    (1, 0, 10),
                    (1, 1, 20),
                    (2, 0, 30),
                ]))),
                true,
            ),
        ],
    );
    let mut prepared = encoder.prepare_bytes(initial.clone(), true).unwrap();
    let row_full = Some(RowFrame {
        baselines: Some(&encoder),
        keyframe: true,
        changed: prepared.changed_rows(),
    });
    let row_empty = Some(RowFrame {
        baselines: Some(&encoder),
        keyframe: true,
        changed: &[],
    });
    let expected = expected_writer
        .hour_on(&initial, &[], row_full, &OneThread)
        .unwrap();
    let actual = diff_writer
        .hour_on(&initial, &[], row_empty, &OneThread)
        .unwrap();
    assert_eq!(actual.manifest, expected.manifest);
    assert!(actual.sections().eq(expected.sections()));
    expected_writer.commit(expected).unwrap();
    diff_writer.commit(actual).unwrap();
    let _ = prepared.take_frame();
    encoder.commit(prepared).unwrap();

    for hour in 2..=4 {
        let (div_token, _) = encoder
            .slice_baseline(TableId::Divisions, 1, hour, false)
            .unwrap();
        let (res_token, res_prev) = encoder
            .slice_baseline(TableId::ResearchInputs, 1, hour, false)
            .unwrap();
        let div_update = match hour {
            2 => SliceUpdate::keyed(
                vec![1, 2, 4],
                TableRows::Divisions(vec![division(1, 2_000_000), division(4, 500_000)]),
            ),
            3 => SliceUpdate {
                ranges: vec![1..3, 4..5],
                owners: Vec::new(),
                keys: Vec::new(),
                rows: TableRows::Divisions(vec![division(1, 2_000_000), division(2, 3_000_000)]),
            },
            _ => SliceUpdate::keyed(
                vec![1],
                TableRows::Divisions(vec![division(1, 2_000_000)]),
            ),
        };
        let res_update = match hour {
            2 => SliceUpdate::new(
                res_prev,
                research(&[(1, 0, 15), (1, 2, 25)]),
                |owner| owner == 1,
                |_| true,
            )
            .unwrap(),
            3 => SliceUpdate::new(
                res_prev,
                research(&[(1, 0, 15), (1, 2, 25)]),
                |owner| owner == 1,
                |_| true,
            )
            .unwrap(),
            _ => SliceUpdate::new(
                res_prev,
                research(&[(3, 0, 40)]),
                |owner| owner == 2 || owner == 3,
                |owner| owner != 2,
            )
            .unwrap(),
        };
        let input = frame(
            hour,
            [
                (countries(vec![country(1, 10)]), true),
                (
                    TableInput::SliceUpdate {
                        token: div_token,
                        update: Arc::new(div_update),
                    },
                    true,
                ),
                (
                    TableInput::SliceUpdate {
                        token: res_token,
                        update: Arc::new(res_update),
                    },
                    true,
                ),
            ],
        );
        let mut prepared = encoder.prepare_bytes(input.clone(), false).unwrap();
        let row_full = Some(RowFrame {
            baselines: Some(&encoder),
            keyframe: false,
            changed: prepared.changed_rows(),
        });
        let row_empty = Some(RowFrame {
            baselines: Some(&encoder),
            keyframe: false,
            changed: &[],
        });
        let expected = expected_writer
            .hour_on(&input, &[], row_full, &OneThread)
            .unwrap();
        let actual = diff_writer
            .hour_on(&input, &[], row_empty, &OneThread)
            .unwrap();
        assert_eq!(actual.manifest, expected.manifest, "hour {hour}");
        assert!(actual.sections().eq(expected.sections()), "hour {hour}");
        expected_writer.commit(expected).unwrap();
        diff_writer.commit(actual).unwrap();
        let _ = prepared.take_frame();
        encoder.commit(prepared).unwrap();
    }
    same_restarts(&mut [&mut expected_writer, &mut diff_writer]);
}

#[test]
fn compact_owners_merge_narrow_and_wide_slices_in_key_order() {
    let mut narrow = Owners::from_keys(
        TableId::ResearchInputs,
        3,
        [
            (1_u128 << 32) | 1,
            (1_u128 << 32) | 2,
            (2_u128 << 32) | 5,
        ]
        .into_iter(),
    )
    .unwrap();
    assert!(matches!(narrow, Owners::U64(_)));
    let slice = Slice {
        owners: vec![1, 3],
        keys: Owners::from_keys(
            TableId::ResearchInputs,
            2,
            [(1_u128 << 32) | 9, (3_u128 << 32) | 4].into_iter(),
        )
        .unwrap(),
    };
    hold_slice(&mut narrow, &slice, TableId::ResearchInputs).unwrap();
    let Owners::U64(keys) = narrow else {
        panic!("expected narrow u64 owners");
    };
    assert_eq!(keys, [(1 << 32) | 9, (2 << 32) | 5, (3 << 32) | 4]);
}
