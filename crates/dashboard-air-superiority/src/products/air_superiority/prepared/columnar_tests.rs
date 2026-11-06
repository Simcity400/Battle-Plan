//! Air superiority's columnar preparation held to the frame classifier on the same hours,
//! written as the native recorder writes them: every view the published pages answer, across
//! segment starts, gaps, a truncated and an invalid score table, flushes and resumes.

use super::tests::{mixed_frames, parity};
use super::*;
use crate::products::air_superiority::{
    columnar::{self, Pairs, Project},
    emit::Emit,
};
use dashboard_base::prepared::series::tests::Fixture;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator as _},
};
use replay_data::{
    format::FormatVersion,
    rows::{RowCodec, RowsVisit, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

const BRANCH: u64 = 1;

/// One table's rows written into a walk.
struct Write<'w> {
    walk: &'w mut Synthetic,
    table: TableId,
    partial: Option<u32>,
    read: bool,
}

impl RowsVisit for Write<'_> {
    type Output = io::Result<()>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<()> {
        self.walk.table(
            self.table,
            Input::Snapshot {
                rows,
                partial: self.partial,
                read: self.read,
                sample: Sample::FromReadBit,
            },
        )
    }
}

/// Writes `frame`'s tables into `walk`, the theatres read where the frame reads them.
fn write(walk: &mut Synthetic, frame: &DecodedFrame) {
    for &table in columnar::TABLES {
        if table == TableId::Campaign {
            continue;
        }
        let read = table != TableId::AirTheatres || frame.metrics.table_ns[table.index()] != 0;
        let sample = Sample::FromReadBit;
        let written = match &*frame.tables[table.index()] {
            TableView::Complete(rows) => rows.visit(Write {
                walk: &mut *walk,
                table,
                partial: None,
                read,
            }),
            TableView::Truncated { rows, missing } => rows.visit(Write {
                walk: &mut *walk,
                table,
                partial: Some(*missing),
                read,
            }),
            TableView::Unavailable { reason } => walk.table::<replay_data::rows::AirWingRow>(
                table,
                Input::Unavailable {
                    reason,
                    read,
                    sample,
                },
            ),
            TableView::NotRecorded { .. } => walk.table::<replay_data::rows::AirWingRow>(
                table,
                Input::NotRecorded { read, sample },
            ),
        };
        written.unwrap();
    }
}

/// The hours the segments' operators hand on, a fresh operator starting at each of `starts`
/// and at each walk's first hour (after a gap the walk starts again), each ending its segment
/// before the next starts, and the row frames of the same hours, read as the walk read them.
fn projected(starts: &[i32]) -> (Vec<columnar::Hour>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let needs = columnar::needs().unwrap();
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut operator = Project::new(format).unwrap();
    for (ordinal, mut observed) in (0..).zip(mixed_frames()) {
        let hour = observed.hour;
        let gap = frames.last().is_some_and(|last| last.hour + 1 != hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(columnar::TABLES, BRANCH, hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, &observed);
        let cold = starts.contains(&hour);
        let metrics = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                if starts {
                    operator.end(&mut out).unwrap();
                    operator = Project::new(format).unwrap();
                }
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                operator
                    .hour(
                        &segments::Hour {
                            branch: BRANCH,
                            hour: decoded.hour,
                            run: 0,
                            ordinal,
                            starts,
                            opens: starts,
                            state,
                            manifest,
                            world,
                        },
                        &mut out,
                    )
                    .unwrap();
                decoded.metrics.clone()
            })
            .unwrap();
        observed.metrics = metrics;
        frames.push(observed);
    }
    operator.end(&mut out).unwrap();
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
}

/// The pages the emit publishes from `hours`, flushed and committed after each of `cuts`' hours,
/// the last ending the branch; after each of `resumes`, the emit is saved and loaded again, as a
/// preparation of a grown recording goes on at the next segment.
fn build(hours: Vec<columnar::Hour>, cuts: &[i32], resumes: &[i32]) -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut emit = Emit::new();
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        let mut writer = fixture.writer();
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.observe(hour).unwrap();
        }
        emit.flush(&mut writer, BRANCH, &flag).unwrap();
        writer.commit().unwrap();
        if resumes.contains(&cut) {
            let saved = emit.save().unwrap();
            emit = Emit::load(&saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is flushed");
    fixture
}

#[test]
fn the_pages_answer_as_the_frame_classifier_across_segments_flushes_and_resumes() {
    let every: Vec<i32> = mixed_frames().iter().map(|frame| frame.hour).collect();
    for (starts, cuts, resumes) in [
        (&[][..], &[130][..], &[][..]),
        (
            &[30, 100][..],
            &[29, 47, 99, 130][..],
            &[29, 47, 99][..],
        ),
        (&every[..], &[23, 79, 110, 130][..], &[23, 79, 110][..]),
    ] {
        let (projected, frames) = projected(starts);
        let fixture = build(projected, cuts, resumes);
        parity(&fixture, &frames, None);
        parity(&fixture, &frames, Some(vec!["700".into()]));
    }
}

/// An hour inside a segment hands on only the pairs whose scores moved: on day 0 the two
/// pairs of region 7000, never the table whole; an hour starting a segment gives every pair.
#[test]
fn an_hour_inside_a_segment_hands_on_only_the_pairs_that_moved() {
    let (out, _) = projected(&[10]);
    let held = |hour: i32| out.iter().find(|held| held.hour == hour).unwrap();
    let scores = |hour: i32| held(hour).air.scores.as_deref().expect("the scores moved");
    match &scores(5).pairs {
        Pairs::Changed { changed, removed } => {
            // The owners hour 5's segment named by then.
            let start = out
                .iter()
                .rposition(|held| held.hour <= 5 && held.air.first)
                .unwrap();
            let owners: Vec<&str> = out[start..]
                .iter()
                .take_while(|held| held.hour <= 5)
                .flat_map(|held| held.air.owners.iter().map(String::as_str))
                .collect();
            let mut moved: Vec<_> = changed
                .iter()
                .map(|&(owner, region, ..)| (owners[owner as usize], region))
                .collect();
            moved.sort_unstable();
            assert_eq!(moved, [("FRA", 7000), ("GER", 7000)]);
            assert!(removed.is_empty());
        }
        Pairs::Whole(_) => panic!("hour 5 continues its segment"),
    }
    assert!(!held(5).air.whole);
    assert!(held(5).air.countries.is_empty(), "the countries did not move");
    assert!(held(5).air.auxiliary.is_none(), "the wings did not move");
    assert!(held(10).air.whole);
    assert!(matches!(scores(10).pairs, Pairs::Whole(_)));
}
