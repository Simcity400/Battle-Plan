use crate::scope::{OriginalTagLists, tests::dynamic_frame};
use crate::test_fixtures::{CAMPAIGN, session};
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
use replay_data::{rows::Recorded, rows::TableId, rows::TableRows};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    frame::TableView,
};
use std::sync::atomic::AtomicBool;

#[test]
fn native_list_snapshots_keep_order_duplicates_and_independent_failures() {
    let mut frame = dynamic_frame(10, ["GER", "GER", ""], [1, 1, 1, 0]);
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(Some(vec![
            vec![],
            vec![0, 202, 201, 202, 100],
            vec![],
            vec![],
            vec![],
        ]));
        rows[0].country_array = Recorded::Value(Some(vec![0, 202, 100, 201, 202]));
    }
    let snapshot = OriginalTagLists::read(&frame);
    assert_eq!(snapshot.bucket(1), Ok(&[0, 202, 201, 202, 100][..]));
    assert_eq!(snapshot.country_array(), Ok(&[0, 202, 100, 201, 202][..]));
    for value in [Recorded::NotRecorded, Recorded::Value(None)] {
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag_buckets = value;
        }
        let snapshot = OriginalTagLists::read(&frame);
        assert!(snapshot.bucket(1).is_err());
        assert!(snapshot.country_array().is_ok());
    }
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(Vec::new()));
    assert!(
        OriginalTagLists::read(&frame)
            .bucket(1)
            .unwrap_err()
            .contains("singleton")
    );
    *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    assert!(OriginalTagLists::read(&frame).country_array().is_err());
}

#[test]
fn original_tag_lists_derive_requested_exact_snapshots_without_history() {
    let mut frames = vec![
        dynamic_frame(10, ["GER", "GER", ""], [0, 1, 1, 0]),
        dynamic_frame(11, ["GER", "GER", ""], [0, 1, 1, 0]),
    ];
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frames[1].tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag_buckets = Recorded::Value(Some(vec![
            vec![],
            vec![202, 201, 202],
            vec![],
            vec![],
            vec![],
        ]));
    }
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: None,
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for frame in frames {
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records]).unwrap();
    let derived = OriginalTagLists::derive(
        &replay,
        1,
        &[11].into_iter().collect(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[&11].bucket(1), Ok(&[202, 201, 202][..]));
    assert_eq!(
        derived[&11].country_array(),
        Ok(&[0, 100, 201, 202, 203][..])
    );
}
