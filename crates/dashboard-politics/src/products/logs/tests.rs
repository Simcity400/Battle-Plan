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

#[test]
fn log_tail_and_dated_panel_match_page_in_reverse_with_exact_facet_totals() {
    let source = opened();
    let params = LogTailParams {
        branch_id: "1".into(),
        log: "market-events".into(),
        facets: Vec::new(),
        from: None,
        hour: Some(136),
        offset: None,
        limit: 3,
        columns: None,
        count_facets: vec![LogFacetCountAsk { key: "kind1".into(), facets: vec![FacetFilter { name: "kind".into(), values: vec![1], ..Default::default() }] }],
    };
    let reply = LogTail::load(&source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.total, "5");
    assert_eq!(reply.rows.iter().map(|row| row.hour).collect::<Vec<_>>(), [136, 132, 128]);
    assert_eq!(reply.facet_totals, vec![LogFacetTotal { key: "kind1".into(), total: "5".into() }]);
    assert_eq!(spans(&reply.missing), [(FIRST, UNREAD, true)]);

    let asked = serde_json::json!({
        "panel": "log-tail",
        "branchId": "1",
        "log": "market-events",
        "limit": 3,
        "countFacets": [{ "key": "kind1", "facets": [{ "name": "kind", "values": [1] }] }]
    });
    let panel = dated_panel(&source, &LoadContext { content: None }, &asked).unwrap();
    let exact = panel.exact().unwrap();
    let stepped = (exact.advance)(136, true, &AtomicBool::new(false), None);
    let cold = (exact.cold)(136, &AtomicBool::new(false));
    assert_eq!(*stepped.value, cold);
    assert_eq!(stepped.value.as_ref().as_ref().unwrap(), &serde_json::to_value(&reply).unwrap());
}

#[test]
fn tail_coverage_cost_does_not_grow_with_elapsed_campaign() {
    let first = 100;
    let last = first + 2_500;
    let frames = (first..=last)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            *frame.tables[TableId::MarketEvents.index()] = if (200..=220).contains(&hour) || (2_400..=2_410).contains(&hour) {
                TableView::Unavailable { reason: WHY.into() }
            } else {
                let events = if hour % 10 == 0 {
                    vec![MarketEventRow { id: hour as u64, occurrence: hour as u64, hour, country: 9001, kind: 1, amount_raw: 1000, ..MarketEventRow::default() }]
                } else {
                    Vec::new()
                };
                TableView::Complete(TableRows::MarketEvents(events))
            };
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "logs-cov".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: None };
    let ask = |hour: i32, count_facets: Vec<LogFacetCountAsk>| LogTailParams {
        branch_id: "1".into(),
        log: "market-events".into(),
        facets: Vec::new(),
        from: None,
        hour: Some(hour),
        offset: None,
        limit: 8,
        columns: None,
        count_facets,
    };

    let (first_reply, first_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(first + 250, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert!(first_counts.status_blocks > 0, "cold call indexes the branch coverage once");
    assert_eq!(first_reply.missing, Vec::<MissingSpan>::new(), "8 newest events at H350 span H280..=H350 after the H200..=H220 gap");

    let (early_reply, early_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(first + 150, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(early_counts.status_blocks, 0, "indexed coverage reads zero status blocks on subsequent early steps");
    assert_eq!(spans(&early_reply.missing), [(200, 220, true)], "8 newest events at H250 cross the H200..=H220 gap");

    let (late_reply, late_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(2_430, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(late_counts.status_blocks, 0, "indexed coverage reads zero status blocks late in the campaign");
    assert_eq!(spans(&late_reply.missing), [(2_400, 2_410, true)], "8 newest events at H2430 cross only the H2400..=H2410 gap");

    let count_asks = vec![LogFacetCountAsk { key: "kind1".into(), facets: vec![FacetFilter { name: "kind".into(), values: vec![1], ..Default::default() }] }];
    let (counted_reply, counted_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(last, count_asks), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(counted_counts.status_blocks, 0, "counted tail also reuses the branch coverage index");
    assert_eq!(spans(&counted_reply.missing), [(200, 220, true), (2_400, 2_410, true)], "counted tail carries all gaps over the counted range");
}
