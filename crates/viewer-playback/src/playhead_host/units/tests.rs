use super::of;
use crate::playhead_host::{
    open,
    views::{Need, Pace, apart::Apart, tests::hourly_steps},
};
use playhead::Piece;
use replay::{
    campaign::{Replay, SeekResult},
    spool::Record,
};
use replay_data::{
    BranchId, Hour,
    rows::{TableId, TableRows, tables},
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, TableView},
};
use std::{ops::RangeInclusive, sync::Arc, time::Duration};

const LIMIT: Duration = Duration::from_secs(120);

pub fn power(segment: BranchId, hour: Hour) -> i64 {
    segment as i64 * 1_000_000 + i64::from(hour)
}

pub fn tag(segment: BranchId) -> Vec<u8> {
    format!("T{segment:02}").into_bytes()
}

fn segment(id: BranchId, hours: RangeInclusive<Hour>) -> Vec<Record> {
    let first = *hours.start();
    let mut records = vec![
        Record::SessionStart(crate::playback::tests::session(id)),
        Record::BranchStart(BranchStart { id, session: id, loaded_hour: Some(first - 1), first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
    ];
    let mut encoder = FrameEncoder::new();
    for hour in hours {
        let mut frame = crate::playback::tests::source(id, hour);
        if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
            rows[0].political_power_raw = Some(power(id, hour));
            rows[0].tag = tag(id);
        }
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    records
}

pub fn reloaded(base: Hour) -> Replay {
    let segments = [segment(1, base..=base + 5), segment(2, base + 3..=base + 6), segment(3, base + 7..=base + 8)];
    Replay::from_records(b"campaign".to_vec(), &segments).unwrap().into_timelines(&Default::default()).unwrap()
}

pub fn segment_of(base: Hour, branch: BranchId, hour: Hour) -> BranchId {
    match (branch, hour - base) {
        (1, _) | (2, ..=2) => 1,
        (2, 3..=6) => 2,
        _ => 3,
    }
}

fn political_power(frame: &DecodedFrame) -> Option<i64> {
    frame.complete::<tables::Countries>().ok()?.first()?.political_power_raw
}

#[test]
fn a_recording_loaded_back_in_time_has_a_playhead_on_each_timeline_and_none_on_a_branch_it_does_not_hold() {
    let replay = reloaded(1_000);
    let ids: Vec<BranchId> = replay.timelines().unwrap().list.iter().map(|timeline| timeline.id).collect();
    assert_eq!(ids, [1, 2], "the recording's timelines");
    let pieces = |branch: BranchId| of(&replay, branch).unwrap_or_else(|error| panic!("branch {branch} is refused: {error}")).timeline.pieces().to_vec();
    assert_eq!(pieces(1), [Piece { run: 0, first: 1_000, last: 1_005 }]);
    assert_eq!(pieces(2), [Piece { run: 0, first: 1_000, last: 1_002 }, Piece { run: 1, first: 1_003, last: 1_006 }, Piece { run: 2, first: 1_007, last: 1_008 }]);
    let refused = of(&replay, 9).err().expect("a branch the recording does not hold is refused");
    assert!(refused.to_string().contains("holds no branch 9"), "refused as {refused}");
}

#[test]
fn rows_at_the_head_of_a_timeline_are_its_own_pieces_and_never_another_timelines_hours() {
    let base = 2_000;
    let replay = reloaded(base);
    for (branch, last) in [(2, base + 8), (1, base + 5)] {
        let entry = open(of(&replay, branch).unwrap(), hourly_steps()).unwrap();
        entry.set_capture(replay.capture_mode);
        let mut apart = Apart::new(&entry, branch).unwrap();
        let reader = apart.store().handle().reader_at("branch rows", &[Need::every(TableId::Countries)], Pace::Step);
        for hour in base..=last {
            entry.playhead().seek(hour);
            assert!(apart.drive(hour), "branch {branch}: the store never reached hour {hour}");
            let frame = reader.wait(|frame| frame.hour == hour, LIMIT).unwrap_or_else(|error| panic!("branch {branch}, hour {hour}: {error}"));
            let segment = segment_of(base, branch, hour);
            assert_eq!(political_power(&frame), Some(power(segment, hour)), "branch {branch}, hour {hour} reads segment {segment}");
            assert_eq!(frame.branch, branch, "hour {hour}'s rows are stamped with the branch asked");
            let today = match replay.seek_selected(branch, hour, Some(&[TableId::Countries])).unwrap() {
                SeekResult::Frame(frame) => political_power(&frame),
                SeekResult::Gap { reason, .. } => panic!("today's read of branch {branch} has a gap at hour {hour}: {reason}"),
            };
            assert_eq!(political_power(&frame), today, "branch {branch}, hour {hour}: today's read");
        }
    }
}

#[test]
fn the_segments_of_one_timeline_share_one_playhead_and_another_timeline_has_its_own() {
    let replay = reloaded(3_000);
    let (first, again, other) = (open(of(&replay, 2).unwrap(), hourly_steps()).unwrap(), open(of(&replay, 2).unwrap(), hourly_steps()).unwrap(), open(of(&replay, 1).unwrap(), hourly_steps()).unwrap());
    assert!(Arc::ptr_eq(&first, &again), "a second opening of timeline 2 shares its playhead");
    assert!(!Arc::ptr_eq(&first, &other), "timeline 1 has a playhead of its own");
    assert!(Arc::ptr_eq(first.pages(), other.pages()), "both timelines read the recording's pages once");
    assert_eq!(first.playhead().timeline().pieces().len(), 3, "segments 1, 2 and 3 are pieces of timeline 2");
}
