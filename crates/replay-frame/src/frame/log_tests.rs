use super::*;
use crate::frame::Metrics;
use crate::rows::{CountryLinkRow, TableId, TableRows};

fn links(rows: &[(u32, u32)]) -> TableView {
    TableView::Complete(TableRows::CountryLinks(
        rows.iter()
            .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ))
}

fn sampled(view: &SharedView, hour: Option<Hour>) -> SharedView {
    view.clone().with_sampled_at(hour)
}

/// Hours of a links table: changed rows, an unchanged hour with a new sample
/// hour, a deletion and an insertion, an unavailable hour and a complete one
/// again, beside a table whose one view every hour shares.
fn hours() -> Vec<DecodedFrame> {
    let shared: SharedView = TableView::Unavailable {
        reason: "kept apart".into(),
    }
    .into();
    let first: SharedView = links(&[(1, 1), (2, 2), (3, 3)]).into();
    let complete: SharedView = links(&[(1, 9)]).into();
    let views = [
        sampled(&first, Some(0)),
        sampled(&first, Some(1)),
        links(&[(1, 1), (2, 5), (3, 3)]).into(),
        links(&[(1, 1), (3, 3), (4, 4)]).into(),
        TableView::Unavailable {
            reason: "unread".into(),
        }
        .into(),
        complete.clone(),
        sampled(&complete, Some(6)),
        links(&[(0, 7), (1, 9), (8, 8)]).into(),
    ];
    views
        .into_iter()
        .enumerate()
        .map(|(hour, view)| {
            let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
            tables[TableId::CountryLinks.index()] = view;
            tables[TableId::Countries.index()] = shared.clone();
            DecodedFrame {
                branch: 1,
                hour: hour as Hour,
                tables,
                metrics: Metrics {
                    total_ns: hour as u64,
                    ..Metrics::default()
                },
            }
        })
        .collect()
}

fn logged(frames: &[DecodedFrame]) -> FrameLog {
    let mut log = FrameLog::new();
    for frame in frames {
        log.push(frame).unwrap();
    }
    log
}

fn assert_same(actual: &DecodedFrame, expected: &DecodedFrame) {
    assert_eq!(actual, expected);
    for (actual, expected) in actual.tables.iter().zip(&expected.tables) {
        assert_eq!(actual.sampled_at(), expected.sampled_at());
    }
}

/// Each frame read back holds the rows, availability and sample hours kept,
/// and shares a table's allocation with the frame before exactly when the
/// frame kept did, whether the reader holds the frame before or not.
#[test]
fn frames_read_back_are_the_frames_kept() {
    let kept = hours();
    let mut previous: Option<DecodedFrame> = None;
    for (at, frame) in logged(&kept).into_frames().enumerate() {
        let frame = frame.unwrap();
        assert_same(&frame, &kept[at]);
        if let Some(previous) = &previous {
            for table in [TableId::CountryLinks, TableId::Countries] {
                let index = table.index();
                assert_eq!(
                    SharedView::ptr_eq(&frame.tables[index], &previous.tables[index]),
                    SharedView::ptr_eq(&kept[at].tables[index], &kept[at - 1].tables[index]),
                    "hour {at} {table:?}"
                );
            }
        }
        previous = Some(frame);
    }
    assert_eq!(previous.map(|frame| frame.hour), Some(7));
    let mut read = 0;
    for (frame, expected) in logged(&kept).into_frames().zip(&kept) {
        assert_same(&frame.unwrap(), expected);
        read += 1;
    }
    assert_eq!(read, kept.len());
}

/// The log keeps the first frame's views and the last frame's whole, and
/// only the changed rows of the frames between.
#[test]
fn a_log_keeps_no_frame_between_the_first_and_the_last() {
    let kept = hours();
    let between: Vec<_> = kept[2..4]
        .iter()
        .map(|frame| frame.tables[TableId::CountryLinks.index()].identity())
        .collect();
    let log = logged(&kept);
    drop(kept);
    assert!(between.iter().all(|view| !view.is_alive()));
    let index = TableId::CountryLinks.index();
    let mut whole: Vec<_> = log
        .views()
        .filter(|view| matches!(&***view, TableView::Complete(TableRows::CountryLinks(_))))
        .map(SharedView::as_ptr)
        .collect();
    whole.sort_unstable();
    whole.dedup();
    assert_eq!(
        whole.len(),
        3,
        "the first, the complete one again, the last"
    );
    assert!(log.bytes() > 0);
    let last = log.into_frames().last().unwrap().unwrap();
    assert_eq!(*last.tables[index], links(&[(0, 7), (1, 9), (8, 8)]));
}

/// Hours whose links change a row every hour.
fn moving(hours: u32) -> Vec<DecodedFrame> {
    (0..hours)
        .map(|hour| {
            let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
            tables[TableId::CountryLinks.index()] = links(&[(1, 1), (2, 2 + hour), (3, 3)]).into();
            DecodedFrame {
                branch: 1,
                hour: hour as Hour,
                tables,
                metrics: Metrics::default(),
            }
        })
        .collect()
}

fn counted(kind: &str) -> u64 {
    crate::cpu::frame_log_counts()
        .get(kind)
        .map_or(0, |count| count[0])
}

/// A reader holding each frame until it has the next, as a fold comparing
/// hours does, makes the first changed table read back a copy; every later
/// one catches up the view the reader let go in place. The frames, and which
/// of them share an allocation, are those kept.
#[test]
fn a_reader_holding_the_frame_before_costs_one_copy() {
    // Equal frames apart from those kept, which the log alone then holds.
    let kept = moving(6);
    let index = TableId::CountryLinks.index();
    let (copied, reused) = (counted("rebuild-copied"), counted("rebuild-reused"));
    let mut previous: Option<DecodedFrame> = None;
    for (at, frame) in logged(&moving(6)).into_frames().enumerate() {
        let frame = frame.unwrap();
        assert_same(&frame, &kept[at]);
        if let Some(previous) = &previous {
            assert!(!previous.tables[index].identity().is(&frame.tables[index]));
            assert!(previous.tables[0].identity().is(&frame.tables[0]));
        }
        previous = Some(frame);
    }
    assert_eq!(counted("rebuild-copied") - copied, 1);
    assert_eq!(counted("rebuild-reused") - reused, 4);
}

/// A reader holding every frame keeps every version: each changed table read
/// back is a copy, and each frame its own rows.
#[test]
fn a_reader_holding_every_frame_gets_each_its_own_rows() {
    let kept = moving(5);
    let (copied, reused) = (counted("rebuild-copied"), counted("rebuild-reused"));
    let read: Vec<DecodedFrame> = logged(&kept).into_frames().map(Result::unwrap).collect();
    for (read, kept) in read.iter().zip(&kept) {
        assert_same(read, kept);
    }
    assert_eq!(counted("rebuild-copied") - copied, 4);
    assert_eq!(counted("rebuild-reused"), reused);
}

/// A second log keeping the same frames on the same thread takes the changes
/// the first found between the same two allocations instead of comparing the
/// tables again, and reads back the same frames.
#[test]
fn logs_keeping_the_same_frames_compare_each_table_once() {
    let kept = moving(4);
    let (diffs, shared) = (counted("diff"), counted("diff-shared"));
    let (first, second) = (logged(&kept), logged(&kept));
    assert_eq!(counted("diff") - diffs, 3);
    assert_eq!(counted("diff-shared") - shared, 3);
    for log in [first, second] {
        for (read, kept) in log.into_frames().zip(&kept) {
            assert_same(&read.unwrap(), kept);
        }
    }
}

/// Changes found between two views are taken again only between those same
/// allocations: once both logs and frames are gone, equal frames at new
/// allocations are compared anew, never matched to the released pair.
#[test]
fn changes_between_released_views_are_never_taken_for_new_ones() {
    let (diffs, shared) = (counted("diff"), counted("diff-shared"));
    drop(logged(&moving(2)));
    let again = moving(2);
    let log = logged(&again);
    assert_eq!(counted("diff") - diffs, 2);
    assert_eq!(counted("diff-shared"), shared);
    for (read, kept) in log.into_frames().zip(&again) {
        assert_same(&read.unwrap(), kept);
    }
}

/// Only logs keep the changes they found: once the log that found them is
/// gone, a log keeping the same frames compares the tables again.
#[test]
fn changes_found_live_only_as_long_as_a_log_keeps_them() {
    let kept = moving(3);
    let (diffs, shared) = (counted("diff"), counted("diff-shared"));
    drop(logged(&kept));
    let log = logged(&kept);
    assert_eq!(counted("diff") - diffs, 4);
    assert_eq!(counted("diff-shared"), shared);
    for (read, kept) in log.into_frames().zip(&kept) {
        assert_same(&read.unwrap(), kept);
    }
}
