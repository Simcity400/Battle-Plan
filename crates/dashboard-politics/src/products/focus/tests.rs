use super::{FocusPages, INCOMPLETE, NO_ROW, NOT_HELD, UNRECORDED, hour_reply, order_page, read};
use crate::products::focus_dto::FocusEntry;
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{CompletedFocusRow, CountryLinkRow, CountryRow, FocusRow, Recorded, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 140;
const GER: u32 = 9001;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    let countries = vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }];
    set(&mut frame, TableId::Countries, TableRows::Countries(countries));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
    let completed: &[&str] = match hour {
        ..105 => &["a"],
        105..112 => &["a", "b"],
        112..120 => &["a", "b", "c"],
        120..125 => &["x"],
        125..132 => &["x", "y"],
        _ => &["x", "y", "z"],
    };
    let count = if (130..132).contains(&hour) { 3 } else { completed.len() as i32 };
    let progress = match hour {
        ..102 => 0,
        102 => 100_000,
        103..105 => 200_000,
        _ => 0,
    };
    let current = (hour < 105).then(|| b"b".to_vec());
    let rows = if (110..112).contains(&hour) {
        Vec::new()
    } else {
        vec![FocusRow { country: GER, tree: Some(b"GER_tree".to_vec()), current, continuous: None, progress, paused: 0, completed_count: count }]
    };
    set(&mut frame, TableId::Focus, TableRows::Focus(rows));
    let completed = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
    set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(completed));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "focus".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn entry(ordinal: u32, token: &str, hour: Option<i32>) -> FocusEntry {
    FocusEntry { ordinal, token: token.into(), completed_by_hour: hour }
}

#[test]
fn a_completion_is_dated_only_where_the_list_grew_at_its_end_read_the_hour_before() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let spans: Vec<(i32, i32)> = fold.segments.iter().map(|segment| (segment.first, segment.last)).collect();
    assert_eq!(spans, [(100, 104), (105, 109), (112, 119), (120, 124), (125, 129), (132, LAST)]);
    let order = |at: usize| fold.segments[at].order.completed.clone();
    assert_eq!(order(0), [entry(0, "a", None)], "a list first read is undated");
    assert_eq!(fold.segments[0].order.current.as_deref(), Some("b"));
    assert_eq!(order(1), [entry(0, "a", None), entry(1, "b", Some(105))]);
    assert_eq!(order(2), [entry(0, "a", None), entry(1, "b", Some(105)), entry(2, "c", None)], "first read after an hour not read: undated");
    assert_eq!(order(3), [entry(0, "x", None)], "a reset dates nothing");
    assert_eq!(order(4), [entry(0, "x", None), entry(1, "y", Some(125))]);
    assert_eq!(order(5), [entry(0, "x", None), entry(1, "y", Some(125)), entry(2, "z", None)]);
    assert_eq!(fold.completions, [entry(1, "b", Some(105)), entry(1, "y", Some(125))]);
    let missing: Vec<(i32, i32, &str)> = fold.missing.iter().map(|missing| (missing.first_hour, missing.last_hour, missing.reason.as_str())).collect();
    assert_eq!(missing, [(110, 111, NO_ROW), (130, 131, INCOMPLETE)]);
    assert!(!fold.missing.iter().any(|missing| missing.reason == NOT_HELD));
    assert!(!fold.missing.iter().any(|missing| missing.reason == UNRECORDED));
}

#[test]
fn progress_gains_only_from_the_hour_before_under_the_same_focus_and_replies_read_the_fold() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.progress(101), Some((0, None)));
    assert_eq!(fold.progress(102), Some((100_000, Some(100_000))));
    assert_eq!(fold.progress(104), Some((200_000, Some(100_000))));
    assert_eq!(fold.progress(105), Some((0, None)), "a fall is no gain");
    assert_eq!(fold.progress(110), None);
    let reply = hour_reply(&fold, 103);
    let focus = reply.focus.unwrap();
    assert_eq!((focus.slot, focus.country_id, focus.hour, focus.last_hour, focus.at), (7, GER, 100, 104, 103));
    assert_eq!(focus.order_first_hour, Some(100));
    assert_eq!(focus.current.as_deref(), Some("b"));
    assert_eq!(focus.completed, FocusPages { first: 0, count: 1, rows: 1 });
    assert_eq!((focus.progress_raw.as_str(), focus.daily_gain_raw.as_deref()), ("200000", Some("100000")));
    let gap = hour_reply(&fold, 111);
    assert!(gap.focus.is_none());
    assert_eq!(gap.missing.unwrap().reason, NO_ROW, "an hour not read says why, never an empty order");
    let pages = hour_reply(&fold, 126).focus.unwrap().completed;
    assert_eq!(pages, FocusPages { first: 4, count: 1, rows: 2 });
    assert_eq!(order_page(&fold, 126, &pages, 0).unwrap().entries.unwrap(), [entry(0, "x", None), entry(1, "y", Some(125))]);
    assert!(order_page(&fold, 126, &pages, 1).unwrap().entries.is_none());
    assert!(order_page(&fold, 126, &FocusPages { first: 0, count: 1, rows: 2 }, 0).is_err(), "pages of another order are refused");
}

#[test]
fn a_tree_change_or_list_reorder_keeps_earlier_focus_completion_dates() {
    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        let countries = vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }];
        set(&mut frame, TableId::Countries, TableRows::Countries(countries));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let (tree, completed): (&[u8], &[&str]) = match hour {
            ..105 => (b"GER_tree_a", &["a"]),
            105..110 => (b"GER_tree_a", &["a", "b"]),
            110..115 => (b"GER_tree_a", &["a", "b", "c"]),
            115..120 => (b"GER_tree_a", &["b", "c"]),
            120..125 => (b"GER_tree_b", &["c", "b"]),
            _ => (b"GER_tree_b", &["c", "b", "d"]),
        };
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        let rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(rows));
        frame
    };
    let frames = (FIRST..=130).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-discontinuity".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let at = |hour: i32| fold.segment_at(hour).unwrap().1.order.completed.clone();
    assert_eq!(at(118), [entry(0, "b", Some(105)), entry(1, "c", Some(110))]);
    assert_eq!(at(122), [entry(0, "c", Some(110)), entry(1, "b", Some(105))]);
    assert_eq!(at(128), [entry(0, "c", Some(110)), entry(1, "b", Some(105)), entry(2, "d", Some(125))]);
    assert_eq!(fold.completions, [entry(1, "b", Some(105)), entry(2, "c", Some(110)), entry(2, "d", Some(125))]);
}
