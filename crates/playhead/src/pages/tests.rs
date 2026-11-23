use super::{Pages, Span, WorldPages};
use crate::{Piece, changes::Changes, lane::Lane};
use replay_recording::columnar_frames::Recording;
use replay_journal::fixtures;
use replay_columnar::{store::Value, unit::BLOCK};
use replay_data::rows::TableId;
use std::sync::Arc;

const RUNS: [(u64, i32, u32); 2] = [(1, 0, 3), (2, 10_000, 2)];

fn countries(ordinal: u32) -> Vec<(u128, i64)> {
    let mut rows = vec![(1, 1), (2, i64::from(ordinal)), (3, 0)];
    if ordinal / 100 % 2 == 0 {
        rows.push((4, 0));
    }
    rows
}

fn held(lane: &Lane) -> Vec<(u128, i64)> {
    let cells = lane.cells();
    let mut rows: Vec<_> = cells
        .rows()
        .map(|slot| match cells.value(lane.requested()[0], slot) {
            Value::Number(bits) => (cells.key(slot).unwrap(), bits as i64),
            other => panic!("slot {slot} holds {other:?}"),
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn lane(pages: &Arc<dyn Pages>, run: usize) -> Lane {
    let stability = (0..).find(|&column| TableId::Countries.field_name(column) == Some("stability_raw")).unwrap();
    Lane::new(Arc::clone(pages), run, TableId::Countries.index(), &[stability], true).unwrap()
}

#[test]
fn a_format_12_recording_seeks_inside_and_at_the_end_of_a_block_and_steps_as_written() {
    let (bytes, _) = fixtures::churned(recorder_derive::writers(), &RUNS);
    let opened = WorldPages::over(Recording::open(bytes).unwrap(), Span::Whole).unwrap();
    let wanted: Vec<Piece> = RUNS.iter().enumerate().map(|(run, &(_, first, blocks))| Piece { run, first, last: first + (blocks * BLOCK) as i32 - 1 }).collect();
    assert_eq!(opened.pieces(), wanted.as_slice());
    let pages: Arc<dyn Pages> = Arc::new(opened);
    for (run, &(_, first, blocks)) in RUNS.iter().enumerate() {
        let hours = blocks * BLOCK;
        let mut lane = lane(&pages, run);
        for ordinal in [5, BLOCK - 1, BLOCK, BLOCK + 333, 2 * BLOCK - 1, hours - 1] {
            lane.seek(first + ordinal as i32).unwrap();
            assert_eq!(lane.head(), Some(first + ordinal as i32));
            assert_eq!(held(&lane), countries(ordinal), "run {run}, a seek to ordinal {ordinal}");
        }
        lane.seek(first + 10).unwrap();
        lane.fetch_through(i32::MAX).unwrap();
        let mut changes = Changes::default();
        let mut ordinal = 10;
        while ordinal + 24 < hours {
            ordinal += 24;
            assert_eq!(lane.step(first + ordinal as i32, &mut changes).unwrap(), first + ordinal as i32);
            assert_eq!(held(&lane), countries(ordinal), "run {run}, a step to ordinal {ordinal}");
        }
    }
}

#[test]
fn a_span_of_hours_reads_back_to_its_restores_and_plays_only_the_span() {
    let (bytes, _) = fixtures::churned(recorder_derive::writers(), &RUNS);
    let (first, from) = (RUNS[1].1, RUNS[1].1 + BLOCK as i32 + 300);
    let opened = WorldPages::over(Recording::open(bytes).unwrap(), Span::Hours { from, hours: 50 }).unwrap();
    assert_eq!(opened.pieces(), &[Piece { run: 0, first: from, last: from + 49 }]);
    let run = &opened.runs()[0];
    assert!(run.first_hour <= from && run.last_hour() >= from + 49 && (run.first_hour - first) % BLOCK as i32 == 0);
    let pages: Arc<dyn Pages> = Arc::new(opened);
    let mut lane = lane(&pages, 0);
    lane.seek(from + 20).unwrap();
    assert_eq!(held(&lane), countries(BLOCK + 320));
}

#[test]
fn a_recording_of_an_earlier_format_plays_what_it_held_and_withholds_the_tables_added_since() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_columnar::store::Value;
    use replay_data::rows::{CountryRow, DivisionRowAt141, compatibility::CAPTURE_ADDITIONS_FORMAT};
    let format = CAPTURE_ADDITIONS_FORMAT - 1;
    let runs = [(1, 0, 2)];
    let divisions = [DivisionRowAt141 { id: (3_u64 << 32) | 1, province: 11, ..Default::default() }, DivisionRowAt141 { id: (4_u64 << 32) | 1, province: 12, ..Default::default() }];
    let partial = |ordinal: u32| ordinal % BLOCK >= BLOCK - 10;
    let (bytes, _) = fixtures::written_at(recorder_derive::writers_within, format, b"format 141", &runs, &[TableId::Countries], |synthetic, _, ordinal| {
        let rows: Vec<CountryRow> = countries(ordinal).into_iter().map(|(id, stability)| CountryRow { id: id as u32, stability_raw: stability, ..Default::default() }).collect();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let input = match partial(ordinal) {
            true => Input::Snapshot { rows: &divisions[..1], partial: Some(1), read: true, sample: Sample::FromReadBit },
            false => Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::Divisions, input).unwrap();
    });
    for span in [Span::Whole, Span::Hours { from: BLOCK as i32 + 30, hours: 50 }] {
        let opened = WorldPages::over(Recording::open(bytes.clone()).unwrap(), span).unwrap();
        for table in [TableId::LandUnitOrder, TableId::StrategicRegionWeights, TableId::StateBuildingInputs, TableId::PowerSpendDays] {
            let gap = table.last_change().table_gap(table, format).unwrap();
            assert_eq!(opened.withheld(table.index()), Some(gap.as_str()));
            assert!(opened.schema()[table.index()].is_empty());
            let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(Recording::open(bytes.clone()).unwrap(), span).unwrap());
            assert!(Lane::new(pages, 0, table.index(), &[0], false).is_err());
        }
        let pages: Arc<dyn Pages> = Arc::new(opened);
        let mut lane = lane(&pages, 0);
        for ordinal in [BLOCK + 40, BLOCK + 79] {
            lane.seek(ordinal as i32).unwrap();
            assert_eq!(held(&lane), countries(ordinal), "{span:?}: a seek to ordinal {ordinal}");
        }
        let table = TableId::Divisions;
        let every: Vec<usize> = (0..table.field_count()).collect();
        let [province, theatre] = ["province", "theatre"].map(|name| (0..table.field_count()).find(|&column| table.field_name(column) == Some(name)).unwrap());
        assert_eq!(pages.held(table.index()), table.field_count() - 1);
        let mut lane = Lane::new(Arc::clone(&pages), 0, table.index(), &every, true).unwrap();
        lane.seek(BLOCK as i32 + 40).unwrap();
        let cells = lane.cells();
        let mut rows: Vec<(u128, Value<'_>, Value<'_>)> = cells.rows().map(|slot| (cells.key(slot).unwrap(), cells.value(province, slot), cells.value(theatre, slot))).collect();
        rows.sort_unstable_by_key(|row| row.0);
        assert_eq!(rows, [(u128::from(divisions[0].id), Value::Number(11), Value::Unrecorded), (u128::from(divisions[1].id), Value::Number(12), Value::Unrecorded)], "{span:?}");
        assert_eq!((lane.requested(), lane.reads()), (every.as_slice(), &every[..every.len() - 1]));
        let named = |changes: &Changes| changes.columns().iter().map(|written| written.column()).collect::<Vec<_>>();
        let mut changes = Changes::default();
        lane.reset_into(&mut changes).unwrap();
        assert_eq!(named(&changes), every[..every.len() - 1], "{span:?}: the reset");
        lane.fetch_through(i32::MAX).unwrap();
        for to in [BLOCK as i32 + 41, BLOCK as i32 + 79] {
            assert_eq!(lane.step(to, &mut changes).unwrap(), to);
            assert!(!named(&changes).contains(&theatre), "{span:?}: the step to {to}");
            let cells = lane.cells();
            assert!(cells.rows().all(|slot| cells.value(theatre, slot) == Value::Unrecorded), "{span:?}: the step to {to}");
        }
    }
}

#[test]
fn a_recording_of_an_earlier_format_crosses_its_piece_boundary_without_the_columns_added_since() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{DivisionRowAt141, compatibility::CAPTURE_ADDITIONS_FORMAT};
    let table = TableId::Divisions;
    let runs = [(1, 0, 1), (2, 10_000, 1)];
    let division = |id: u64, province: u32| DivisionRowAt141 { id: (id << 32) | 1, province, ..Default::default() };
    let pieces = [[division(3, 11), division(4, 12)], [division(3, 13), division(5, 14)]];
    let (bytes, _) = fixtures::written_at(recorder_derive::writers_within, CAPTURE_ADDITIONS_FORMAT - 1, b"format 141", &runs, &[table], |synthetic, run, _| {
        synthetic.table(table, Input::Snapshot { rows: &pieces[run as usize], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(Recording::open(bytes).unwrap(), Span::Whole).unwrap());
    let every: Vec<usize> = (0..table.field_count()).collect();
    let [province, theatre] = ["province", "theatre"].map(|name| every.iter().copied().find(|&column| table.field_name(column) == Some(name)).unwrap());
    let mut before = Lane::new(Arc::clone(&pages), 0, table.index(), &every, true).unwrap();
    before.seek(BLOCK as i32 - 1).unwrap();
    let mut after = Lane::new(Arc::clone(&pages), 1, table.index(), &every, true).unwrap();
    after.seek(10_000).unwrap();
    let mut changes = Changes::default();
    let crossed = crate::boundary::crossed(&before, &mut after, &mut changes).unwrap().expect("a table holding state crosses");
    assert_eq!((crossed.stood, crossed.gone, crossed.born, crossed.changed), (1, 1, 1, 1));
    let named: Vec<usize> = changes.columns().iter().map(|written| written.column()).collect();
    assert_eq!(named, every[..every.len() - 1]);
    let written = changes.columns().iter().find(|written| written.column() == province).unwrap();
    assert_eq!(written.len(), 2, "the changed row and the new one");
    let cells = after.cells();
    assert!(cells.rows().all(|slot| cells.value(theatre, slot) == Value::Unrecorded));
}
