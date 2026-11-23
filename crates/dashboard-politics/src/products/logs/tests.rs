use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::rows::{MarketEventRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::Arc;

const FIRST: i32 = 100;
const UNREAD: i32 = 119;
const LAST: i32 = 140;
const WHY: &str = "The market was not read";

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::MarketEvents.index()] = if hour <= UNREAD {
        TableView::Unavailable { reason: WHY.into() }
    } else {
        let events = if hour % 4 == 0 {
            vec![MarketEventRow { id: hour as u64, occurrence: hour as u64, hour, country: 9001, kind: 1, amount_raw: i64::from(hour), ..MarketEventRow::default() }]
        } else {
            Vec::new()
        };
        TableView::Complete(TableRows::MarketEvents(events))
    };
    frame
}

fn opened() -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "logs".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn spans(missing: &[MissingSpan]) -> Vec<(i32, i32, bool)> {
    missing.iter().map(|span| (span.first_hour, span.last_hour, span.reason.contains(WHY))).collect()
}

fn page(source: &Source, from: Option<i32>) -> LogPageReply {
    let params = LogPageParams { branch_id: "1".into(), log: "market-events".into(), facets: Vec::new(), from: from.map(f64::from), to: f64::from(LAST), cursor: None, page_size: 100, columns: None };
    LogPage::load(source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
}

fn counts(source: &Source, from: i32) -> LogCountsReply {
    let params = LogCountsParams { branch_id: "1".into(), log: "market-events".into(), facets: Vec::new(), from: f64::from(from), to: f64::from(LAST), bin_hours: 6, scope: None };
    LogCounts::load(source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_log_unavailable_for_a_stretch_is_missing_there_with_its_reason_in_pages_and_counts() {
    let source = opened();
    let whole = page(&source, None);
    assert_eq!(whole.total, "6", "the events of the hours read");
    assert_eq!(whole.rows.iter().map(|row| row.hour).collect::<Vec<_>>(), [120, 124, 128, 132, 136, 140]);
    assert_eq!(spans(&whole.missing), [(FIRST, UNREAD, true)], "{:?}", whole.missing);
    assert_eq!(spans(&counts(&source, FIRST).missing), [(FIRST, UNREAD, true)]);
    assert_eq!(spans(&counts(&source, 110).missing), [(110, UNREAD, true)], "cut to the asked range");
    assert!(page(&source, Some(UNREAD + 1)).missing.is_empty());
    assert!(counts(&source, UNREAD + 1).missing.is_empty());
}

#[test]
fn a_scope_on_a_log_no_scope_reads_is_refused_by_name() {
    let source = opened();
    let scope = vec![dashboard_world::products::groups::MembershipRun { tag: "GER".into(), first_hour: FIRST, last_hour: LAST }];
    let params = LogCountsParams { branch_id: "1".into(), log: "market-events".into(), facets: Vec::new(), from: f64::from(FIRST), to: f64::from(LAST), bin_hours: 6, scope: Some(scope) };
    let refused = LogCounts::load(&source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap_err();
    assert!(refused.to_string().contains("The market-events log cannot be kept to a country scope"), "{refused}");
}
