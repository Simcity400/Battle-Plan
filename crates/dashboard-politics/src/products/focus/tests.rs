use super::{INCOMPLETE, NO_ROW, NOT_HELD, UNRECORDED, read};
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
    frame.metrics.table_ns[table.index()] = 1;
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
fn progress_gains_only_from_the_hour_before_under_the_same_focus() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.progress(101), Some((0, None)));
    assert_eq!(fold.progress(102), Some((100_000, Some(100_000))));
    assert_eq!(fold.progress(104), Some((200_000, Some(100_000))));
    assert_eq!(fold.progress(105), Some((0, None)), "a fall is no gain");
    assert_eq!(fold.progress(110), None);
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

#[test]
fn replacement_holder_and_repeated_focus_tree_swap_do_not_transfer_dates() {
    let make_holder_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        let holder = if hour < 120 { GER } else { GER + 1 };
        let countries = vec![CountryRow { id: holder, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }];
        set(&mut frame, TableId::Countries, TableRows::Countries(countries));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: holder, canonical_slot: 7 }]));
        let completed: &[&str] = if hour < 105 { &["a"] } else if hour < 120 { &["a", "x"] } else { &["x"] };
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: holder, tree: Some(b"GER_tree".to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        let rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: holder, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(rows));
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=125).map(make_holder_frame).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-holder".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(110).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(122).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_repeat_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        let countries = vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }];
        set(&mut frame, TableId::Countries, TableRows::Countries(countries));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let (tree, completed): (&[u8], &[&str]) = match hour {
            ..105 => (b"GER_tree_a", &["x"]),
            105..110 => (b"GER_tree_a", &["x", "x"]),
            _ => (b"GER_tree_b", &["x", "x"]),
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
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=115).map(make_repeat_frame).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-repeat".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(106).unwrap().1.order.completed, [entry(0, "x", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(112).unwrap().1.order.completed, [entry(0, "x", None), entry(1, "x", None)]);
}

#[test]
fn recompleted_focus_unread_completed_table_load_stretch_and_lossy_bytes_never_carry_dates() {
    let make_recomplete = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let completed: &[&str] = match hour {
            ..105 => &["a"],
            105 => &["a", "x"],
            106 => &["a"],
            _ => &["x"],
        };
        let focus_rows = if hour == 106 {
            Vec::new()
        } else {
            vec![FocusRow { country: GER, tree: Some(b"GER_tree".to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]
        };
        set(&mut frame, TableId::Focus, TableRows::Focus(focus_rows));
        let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=110).map(make_recomplete).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-recomplete".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_unread_done = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let completed: &[&str] = match hour {
            ..105 => &["a"],
            105 => &["a", "x"],
            _ => &["x"],
        };
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(b"GER_tree".to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        if hour == 106 {
            frame.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable { reason: "completed focuses unread".into() }.into();
        } else {
            let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
            set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        }
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=110).map(make_unread_done).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-unread-done".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_stretch = |branch: u64, hour: i32, completed: &[&str]| {
        let mut frame = source(branch, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(b"GER_tree".to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        frame
    };
    let run0: Vec<DecodedFrame> = (FIRST..=106).map(|hour| make_stretch(1, hour, if hour < 105 { &["a"] } else { &["a", "x"] })).collect();
    let run1: Vec<DecodedFrame> = (107..=110).map(|hour| make_stretch(2, hour, &["x"])).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, run0), records(2, run1)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-load-stretch".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    assert_eq!(query.timeline.stretches.len(), 2);
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_lossy = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let (tree, tokens): (&[u8], &[&[u8]]) = match hour {
            ..105 => (b"GER_tree_a", &[b"a"]),
            105..107 => (b"GER_tree_a", &[b"a", b"focus_\xff"]),
            _ => (b"GER_tree_b", &[b"focus_\xfe"]),
        };
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: tokens.len() as i32 }]),
        );
        let done_rows = tokens.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=110).map(make_lossy).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-lossy".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed[1].completed_by_hour, Some(105));
    assert_eq!(fold.segment_at(108).unwrap().1.order.completed, [entry(0, "focus_\u{fffd}", None)]);
}

#[test]
fn cleared_continuity_drops_date_at_recovery_and_subsequent_tree_change() {
    let make_absence = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let (tree, completed): (&[u8], &[&str]) = match hour {
            ..105 => (b"GER_tree_a", &["a"]),
            105 => (b"GER_tree_a", &["a", "x"]),
            106 => (b"GER_tree_a", &["a"]),
            107 => (b"GER_tree_a", &["a", "x"]),
            _ => (b"GER_tree_b", &["x"]),
        };
        let focus_rows = if hour == 106 {
            Vec::new()
        } else {
            vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]
        };
        set(&mut frame, TableId::Focus, TableRows::Focus(focus_rows));
        let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=110).map(make_absence).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-carry-absence".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", None)]);
    assert_eq!(fold.segment_at(108).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_stretch = |branch: u64, hour: i32, tree: &[u8], completed: &[&str]| {
        let mut frame = source(branch, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
        set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        frame
    };
    let run0: Vec<DecodedFrame> = (FIRST..=106).map(|hour| make_stretch(1, hour, b"GER_tree_a", if hour < 105 { &["a"] } else { &["a", "x"] })).collect();
    let run1: Vec<DecodedFrame> = (107..=110).map(|hour| make_stretch(2, hour, if hour == 107 { b"GER_tree_a" } else { b"GER_tree_b" }, if hour == 107 { &["a", "x"] } else { &["x"] })).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, run0), records(2, run1)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-carry-load".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    assert_eq!(query.timeline.stretches.len(), 2);
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", None)]);
    assert_eq!(fold.segment_at(108).unwrap().1.order.completed, [entry(0, "x", None)]);

    let make_unavail = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
        let (tree, completed): (&[u8], &[&str]) = match hour {
            ..105 => (b"GER_tree_a", &["a"]),
            105..=107 => (b"GER_tree_a", &["a", "x"]),
            _ => (b"GER_tree_b", &["x"]),
        };
        set(
            &mut frame,
            TableId::Focus,
            TableRows::Focus(vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
        );
        if hour == 106 {
            frame.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable { reason: "completed focuses unavailable".into() }.into();
        } else {
            let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
            set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
        }
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=110).map(make_unavail).collect())])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-carry-unavail".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", None)]);
    assert_eq!(fold.segment_at(108).unwrap().1.order.completed, [entry(0, "x", None)]);
}

#[test]
fn held_complete_status_without_fresh_read_breaks_carry_continuity() {
    use replay_columnar::manifest::Status;
    use replay_data::{format::FormatVersion, rows::{ClockRow, tables}};
    use replay_journal::synthetic::{Input, Sample};
    use replay_lanes::lanes;

    let frames: Vec<DecodedFrame> = (FIRST..=110)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1_000));
            set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
            set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
            set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 7 }]));
            let (tree, completed): (&[u8], &[&str]) = match hour {
                ..105 => (b"GER_tree_a", &["a"]),
                105..=106 => (b"GER_tree_a", &["a", "x"]),
                _ => (b"GER_tree_b", &["x"]),
            };
            set(
                &mut frame,
                TableId::Focus,
                TableRows::Focus(vec![FocusRow { country: GER, tree: Some(tree.to_vec()), current: None, continuous: None, progress: 0, paused: 0, completed_count: completed.len() as i32 }]),
            );
            let done_rows = completed.iter().enumerate().map(|(ordinal, token)| CompletedFocusRow { country: GER, ordinal: ordinal as u32, token: token.as_bytes().to_vec() }).collect();
            set(&mut frame, TableId::CompletedFocuses, TableRows::CompletedFocuses(done_rows));
            frame
        })
        .collect();
    let bytes = replay::campaign::fixtures::encode_framed_at(recorder_derive::writers_within, FormatVersion::CURRENT.version(), CAMPAIGN.into(), &[records(1, frames)], |synthetic, decoded| {
        let countries = decoded.complete::<tables::Countries>()?;
        synthetic.table(TableId::Countries, Input::Snapshot { rows: countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        let links = decoded.complete::<tables::CountryLinks>()?;
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: links, partial: None, read: true, sample: Sample::FromReadBit })?;
        let focus = decoded.complete::<tables::Focus>()?;
        synthetic.table(TableId::Focus, Input::Snapshot { rows: focus, partial: None, read: true, sample: Sample::FromReadBit })?;
        let done = decoded.complete::<tables::CompletedFocuses>()?;
        synthetic.table(TableId::CompletedFocuses, Input::Snapshot { rows: done, partial: None, read: decoded.hour != 106, sample: Sample::FromReadBit })?;
        Ok(())
    })
    .unwrap();
    let replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let statuses = lanes::statuses(replay.recording(), 0, TableId::CompletedFocuses, FIRST, 110).unwrap();
    assert!(statuses.iter().any(|run| run.first <= 106 && 106 <= run.last && run.status == Status::Complete));
    let complete = lanes::complete_reads(replay.recording(), 0, TableId::CompletedFocuses, FIRST, 110).unwrap();
    assert!(!complete.iter().any(|&(first, last)| first <= 106 && 106 <= last));
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "focus-held-complete".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let fold = read(&dashboard_base::queries::query(&src, 1).unwrap(), &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fold.segment_at(105).unwrap().1.order.completed, [entry(0, "a", None), entry(1, "x", Some(105))]);
    assert_eq!(fold.segment_at(107).unwrap().1.order.completed, [entry(0, "x", None)]);
}

#[test]
fn a_focus_card_lists_dated_completions_newest_first_then_undated_and_holds_only_until_its_reading_changes() {
    use crate::products::focus_card::{NO_GAIN, card_at, progress_figures};
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let fold = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let early = card_at(&fold, 103);
    assert_eq!(early.completed, [("a".to_owned(), None)]);
    assert_eq!((early.undated, early.segment_first), (1, Some(100)));
    assert_eq!(early.current, Some(("b".to_owned(), false, false)));
    assert_eq!(early.progress, Some((200_000, Some(100_000))));
    assert_eq!(early.valid_through, 104, "the answer stands no later than its progress reading");
    let mut delayed = fold.clone();
    delayed.progress.retain(|run| run.first > 100);
    let before = card_at(&delayed, 100);
    assert_eq!((before.progress, before.valid_through), (None, 101));
    assert!(card_at(&delayed, 102).progress.is_some());
    let later = card_at(&fold, 107);
    assert_eq!(later.completed, [("b".to_owned(), Some(105)), ("a".to_owned(), None)], "dated newest first, the undated after them");
    assert_eq!((later.undated, later.current.clone(), later.valid_through), (1, None, 109));
    let gap = card_at(&fold, 110);
    assert!(gap.completed.is_empty() && gap.current.is_none());
    assert_eq!(gap.missing.as_ref().map(|missing| missing.reason.as_str()), Some(NO_ROW));
    assert_eq!(gap.valid_through, 111);
    assert_eq!(progress_figures(Some((150_000, Some(50_000))), Some(70.0)), (Some(1), Some(214), Some(137), None));
    assert_eq!(progress_figures(Some((150_000, None)), Some(70.0)), (Some(1), Some(214), None, Some(NO_GAIN.to_owned())));
    assert_eq!(progress_figures(Some((9_000_000, Some(1))), Some(70.0)).1, Some(10_000), "progress past the duration is complete, not more");
}

#[test]
fn completions_are_ordered_by_their_dates_and_each_is_presented_at_the_hour_it_was_observed() {
    use super::{Fold, Segment};
    use crate::products::focus_card::{card_at, observed_hours};
    use crate::products::focus_dto::FocusOrder;
    let order = FocusOrder { completed: vec![entry(0, "early", None), entry(1, "later", Some(110)), entry(2, "middle", Some(105))], current: Some("now".into()), continuous: None, paused: false };
    let fold = Fold { segments: vec![Segment { first: 100, last: 200, slot: 7, id: 1, order }], ..Default::default() };
    let card = card_at(&fold, 150);
    assert_eq!(card.completed, [("later".to_owned(), Some(110)), ("middle".to_owned(), Some(105)), ("early".to_owned(), None)], "a reordered list still reads newest first by date");
    let hours = observed_hours(&card);
    assert_eq!(hours.get(&110), Some(&vec!["later".to_owned()]));
    assert_eq!(hours.get(&105), Some(&vec!["middle".to_owned()]));
    assert_eq!(hours.get(&100), Some(&vec!["early".to_owned()]), "an undated completion is presented at the first read that held it, never at the displayed hour");
    assert!(!hours.contains_key(&150));
}
