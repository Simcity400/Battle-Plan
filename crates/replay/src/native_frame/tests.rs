use super::*;
use crate::{
    columnar_frames::{Views, schema},
    frame::{DecodedFrame, FrameDecoder, FrameEncoder},
    rows::{CountryRow, DivisionRow, ResearchInputRow, TableRows, sample_extended_table, tables},
};
use replay_columnar::{manifest::Manifest, walk::State};
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
        let columnar = {
            let manifest = Manifest::read(&prepared.manifest, self.writer.columns()).unwrap();
            let sections: Vec<&[u8]> = prepared.sections().map(|(_, bytes)| bytes).collect();
            self.state.apply(&manifest, |at| Ok(sections[at])).unwrap();
            self.views.advance(&self.state, &manifest, first).unwrap()
        };
        self.writer.commit(prepared).unwrap();
        let encoded = self.encoder.encode_input(input, first).unwrap();
        (self.decoder.apply(&encoded).unwrap(), columnar)
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
