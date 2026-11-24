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

