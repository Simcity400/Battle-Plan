use super::*;
use crate::{
    columnar_frames::{Views, schema},
    frame::{DecodedFrame, FrameDecoder, FrameEncoder},
    rows::{CountryRow, DivisionRow, ResearchInputRow, TableRows, sample_extended_table, tables},
};
use replay_columnar::{manifest::Manifest, walk::State, writer::Work};
use std::sync::Arc;

const TABLES: [TableId; 3] = [
    TableId::Countries,
    TableId::Divisions,
    TableId::ResearchInputs,
];

/// The row encoder and decoder beside the columnar writer and a walker, fed the same frames.
struct Paths {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
    writer: FrameWriter,
    /// The same hours staged a row at a time, which write the same bytes.
    rows: FrameWriter,
    /// The same hours clearing and finishing every column of every staged table, and
    /// staging hours of no rows: the same bytes again.
    swept: FrameWriter,
    state: State,
    views: Views,
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
            state: State::new(schema().unwrap()).unwrap(),
            views: Views::new(tables, FormatVersion::CURRENT, mode).unwrap(),
            tables: tables.to_vec(),
            mode,
        }
    }

    /// The paths' frame of the given tables' inputs, read or not.
    fn frame(&self, hour: i32, inputs: Vec<(TableInput, bool)>) -> FrameInput {
        frame_of(self.mode, &self.tables, hour, inputs)
    }

    /// The hour through both paths, compared.
    fn check(&mut self, hour: i32, inputs: Vec<(TableInput, bool)>) {
        let input = self.frame(hour, inputs);
        let tables = self.tables.clone();
        let (rows, columnar) = self.hour(input);
        same(&tables, &rows, &columnar);
    }

    /// The hour as the row recording decodes it, and as the columnar one walks it.
    fn hour(&mut self, input: FrameInput) -> (DecodedFrame, DecodedFrame) {
        let first = input.hour == 1;
        let prepared = self.writer.hour(&input).unwrap();
        let staged = self.rows.hour(&input).unwrap();
        let swept = self.swept.hour(&input).unwrap();
        for other in [&staged, &swept] {
            assert_eq!(other.manifest, prepared.manifest, "hour {}", input.hour);
            assert!(other.sections().eq(prepared.sections()), "hour {}", input.hour);
        }
        let columnar = {
            let manifest = Manifest::read(&prepared.manifest, self.writer.columns()).unwrap();
            let sections: Vec<&[u8]> = prepared.sections().map(|(_, bytes)| bytes).collect();
            self.state.apply(&manifest, |at| Ok(sections[at])).unwrap();
            self.views.advance(&self.state, &manifest, first).unwrap()
        };
        self.writer.commit(prepared).unwrap();
        self.rows.commit(staged).unwrap();
        self.swept.commit(swept).unwrap();
        same_restarts(&mut [&mut self.writer, &mut self.rows, &mut self.swept]);
        let encoded = self.encoder.encode_input(input, first).unwrap();
        (self.decoder.apply(&encoded).unwrap(), columnar)
    }
}

/// The writers' restarts, strings and every table's state, are the same.
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

/// The three tables as given, read or not; every other table unavailable and unread.
fn frame(hour: i32, tables: [(TableInput, bool); 3]) -> FrameInput {
    frame_of(CaptureMode::Full, &TABLES, hour, tables.into())
}

/// `tables` as given, read or not. Every other table the mode records is unavailable and
/// unread, and every table it does not record is marked so.
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
    // The wire holds stepped fields at their step, as the spool rounds them before encoding.
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

/// Every view, sample hour, read bit and metric the walker gives matches the row decoder's.
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

/// Every form today's conversion hands the encoder (sampled complete, truncated and
/// unavailable views, shared complete rows, compared equality, compared changes and owner
/// slice updates), read and unread, makes the columnar views the row recording decodes.
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

    // Compared equality, one compared change, and owner 1's slice read again with a change.
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

    // Unread hours: a changed table loses its sample, an unchanged one keeps it, and a slice
    // update drops a dead owner's rows.
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

    // A truncated read, a removal with an insertion, and equality after a slice update.
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

    // Whole tables again after a truncated read, an unavailable table, and new shared rows.
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

    // Available again, and an unread unchanged hour after an unread change.
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

    // Unread compared changes beside unread equality, then an unread truncated read.
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

    // A keyed slice update replacing one holder and dropping another.
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

    // A key-range slice update bringing an owner's rows back, unread.
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

/// A staggered table's off-slot refusal carries complete rows from an earlier hour of the
/// run, and stays a refusal when nothing complete is held.
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

/// A Map recording marks the tables its mode leaves out, and they walk as not recorded.
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

/// A table the recording omits, retired from the format or left out by the capture mode, keeps
/// no read bit even when its producer timed it, and walks as the row recording decodes it.
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

/// A run's first hour refuses a truncated read as the keyframe a row recording starts with does;
/// a later truncated read stays truncated.
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

/// Complete tables arriving hour after hour in the order they are held: unchanged, changed in
/// place with a FILL, with rows coming and going, and held again.
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

/// Each table's work, by table index.
fn works(writer: &FrameWriter) -> Vec<Work> {
    writer.tables().iter().map(TableWriter::work).collect()
}

/// Hours of tables that change nothing: a table whose hour stages no rows (compared equal,
/// compared changes of no row, or no rows at all) begins no hour, and one staged whole and
/// unchanged clears and finishes no column; each writes what the writer sweeping every column
/// writes.
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
    // The work of countries, divisions and research inputs, by the writers working the columns
    // an hour touched and by the swept one. No writer begins another table's hours: each is
    // unavailable or not recorded.
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
    // The inserts staged every column; the swept writer cleared each first.
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
    // The countries' hour clears what the inserts left, then finishes nothing.
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
    // One country's tag changes: its column alone is finished, then cleared.
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

/// The recorder's writer, from complete rows, and a converter's, from the same rows with the
/// keys each table's delta names and the sample hours the row recording of the same hours
/// decodes, staged by column, a row at a time, and by column clearing and finishing every
/// column.
struct Converters {
    live: FrameWriter,
    converted: FrameWriter,
    rows: FrameWriter,
    swept: FrameWriter,
    encoder: FrameEncoder,
    decoder: FrameDecoder,
}

/// One hour through the recorder's writer and the converters': all write the same manifest
/// and sections.
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
    for other in [&actual, &staged, &every] {
        assert_eq!(other.manifest, expected.manifest, "hour {hour}");
        assert!(other.sections().eq(expected.sections()), "hour {hour}");
    }
    live.commit(expected).unwrap();
    converted.commit(actual).unwrap();
    rows.commit(staged).unwrap();
    swept.commit(every).unwrap();
}

/// A recorded delta's keys write what its complete rows write, and are not taken where the
/// held rows are not the rows the delta changed: a run's first hour, and after a truncated or
/// refused hour.
#[test]
fn a_recorded_delta_writes_what_its_complete_rows_write() {
    let mut writers = Converters {
        live: FrameWriter::new().unwrap(),
        converted: FrameWriter::new().unwrap(),
        rows: FrameWriter::with_staging(native_columns::Staging::Rows).unwrap(),
        swept: FrameWriter::with_sweep(Sweep::Every).unwrap(),
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
    // Deltas naming nothing, of rows the writers do not hold complete.
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
        ..
    } = &mut writers;
    same_restarts(&mut [live, converted, rows, swept]);
}

/// A delta naming a key its complete rows do not hold ends the run.
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

/// A row recording's hours converted from the views and sample hours they decode to, as the
/// converter converts them, in runs that may start where the recording's does not; and the
/// converted run's walk.
struct Conversion {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
    writer: FrameWriter,
    state: State,
    views: Views,
    /// The next hour starts the converted run.
    starts: bool,
}

impl Conversion {
    fn new() -> Self {
        Self {
            encoder: FrameEncoder::new(),
            decoder: FrameDecoder::new(),
            writer: FrameWriter::new().unwrap(),
            state: State::new(schema().unwrap()).unwrap(),
            views: Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap(),
            starts: true,
        }
    }

    /// Starts another converted run at the next hour, as a cut does; the recording goes on.
    fn cut(&mut self) {
        self.writer = FrameWriter::new().unwrap();
        self.state = State::new(schema().unwrap()).unwrap();
        self.views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        self.starts = true;
    }

    /// The hour as the row recording decodes it, stored as a keyframe when `keyframe` says,
    /// and as its conversion walks.
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
        let walked = {
            let manifest = Manifest::read(&prepared.manifest, self.writer.columns()).unwrap();
            let sections: Vec<&[u8]> = prepared.sections().map(|(_, bytes)| bytes).collect();
            self.state.apply(&manifest, |at| Ok(sections[at])).unwrap();
            self.views.advance(&self.state, &manifest, self.starts).unwrap()
        };
        self.writer.commit(prepared).unwrap();
        self.starts = false;
        (decoded, walked)
    }
}

/// An unread table a recording's keyframe stores whole again has no sample hour there, though
/// its rows are the ones the converter holds, and its converted hour keeps none either; between
/// keyframes the recording leaves it unchanged, and it keeps its sample.
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

/// A converted run cut where the recording's goes on starts with the recording's views: a
/// truncated table truncated, with its rows and missing count, and an unread one sampled when
/// the recording last read it.
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
