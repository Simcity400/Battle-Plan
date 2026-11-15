use super::{Pages, Span, WorldPages};
use crate::{Piece, changes::Changes, lane::Lane};
use replay::columnar_frames::{Recording, fixtures};
use replay_columnar::{store::Value, unit::BLOCK};
use replay_data::rows::TableId;
use std::sync::Arc;

/// Two runs of churned countries (branch, first hour, blocks).
const RUNS: [(u64, i32, u32); 2] = [(1, 0, 3), (2, 10_000, 2)];

/// The countries the fixture writes at a run's ordinal, by key, with their stability.
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
        .map(|slot| match cells.value(lane.columns()[0], slot) {
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

/// A format-12 recording read whole: each run a piece; a seek inside a block, at a block's end and
/// at the run's last hour holds what the writer wrote there, and so does every step after a seek.
#[test]
fn a_format_12_recording_seeks_inside_and_at_the_end_of_a_block_and_steps_as_written() {
    let (bytes, _) = fixtures::churned(&RUNS);
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

/// A span of hours reads its run from the block of the tables' restores before it: a seek in the
/// span holds what the writer wrote, and its piece is the span.
#[test]
fn a_span_of_hours_reads_back_to_its_restores_and_plays_only_the_span() {
    let (bytes, _) = fixtures::churned(&RUNS);
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
