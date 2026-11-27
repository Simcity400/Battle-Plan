use super::sample_playback::{session, source};
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
use replay_data::{BranchId, Hour, rows::{TableId, TableRows}};
use replay_frame::{branches::{BranchReason, BranchStart, Parentage}, frame::{FrameEncoder, TableView}};
use std::ops::RangeInclusive;

pub fn power(segment: BranchId, hour: Hour) -> i64 {
    segment as i64 * 1_000_000 + i64::from(hour)
}

pub fn tag(segment: BranchId) -> Vec<u8> {
    format!("T{segment:02}").into_bytes()
}

fn segment(id: BranchId, hours: RangeInclusive<Hour>) -> Vec<Record> {
    let first = *hours.start();
    let mut records = vec![
        Record::SessionStart(session(id)),
        Record::BranchStart(BranchStart { id, session: id, loaded_hour: Some(first - 1), first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
    ];
    let mut encoder = FrameEncoder::new();
    for hour in hours {
        let mut frame = source(id, hour);
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
    Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &segments).unwrap().into_timelines(&Default::default()).unwrap()
}

pub fn single_run(base: Hour) -> Replay {
    let segments = [segment(1, base..=base + 5)];
    Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &segments).unwrap().into_timelines(&Default::default()).unwrap()
}

pub fn segment_of(base: Hour, branch: BranchId, hour: Hour) -> BranchId {
    match (branch, hour - base) {
        (1, _) | (2, ..=2) => 1,
        (2, 3..=6) => 2,
        _ => 3,
    }
}
