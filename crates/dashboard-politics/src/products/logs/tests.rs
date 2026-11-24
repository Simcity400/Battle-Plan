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
    assert!(first_counts.status_blocks > 0, "cold call indexes coverage for the read blocks");
    assert_eq!(spans(&first_reply.missing), [(200, 220, true)], "offset zero returns full-interval coverage regardless of countFacets");

    let (early_reply, early_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(first + 150, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(early_counts.status_blocks, 0, "cached block coverage reads zero status blocks on subsequent early steps");
    assert_eq!(spans(&early_reply.missing), [(200, 220, true)]);

    let (late_reply, _late_cold_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(2_430, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(spans(&late_reply.missing), [(200, 220, true), (2_400, 2_410, true)], "offset zero late tail carries all gaps across the requested interval");
    let (late_warm, late_warm_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(2_430, Vec::new()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(late_warm_counts.status_blocks, 0, "warm repeat late read decodes zero status blocks");
    assert_eq!(late_warm.missing, late_reply.missing);

    let count_asks = vec![LogFacetCountAsk { key: "kind1".into(), facets: vec![FacetFilter { name: "kind".into(), values: vec![1], ..Default::default() }] }];
    let (counted_reply, _counted_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(last, count_asks.clone()), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(spans(&counted_reply.missing), [(200, 220, true), (2_400, 2_410, true)], "counted tail carries all gaps over the counted range");
    let (_, counted_warm_counts) = replay_columnar::unit::reader::count_decodes(|| LogTail::load(&source, &ask(last, count_asks), &load, &AtomicBool::new(false)).unwrap());
    assert_eq!(counted_warm_counts.status_blocks, 0, "warm counted tail reuses per-block coverage cache");
}

#[test]
fn log_pages_keep_recorded_coverage_gaps_between_page_boundaries() {
    let frames = (100..=200)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            *frame.tables[TableId::MarketEvents.index()] = if (140..=150).contains(&hour) {
                TableView::Truncated { rows: TableRows::MarketEvents(Vec::new()), missing: 3 }
            } else if hour == 110 || hour == 190 {
                TableView::Complete(TableRows::MarketEvents(vec![MarketEventRow {
                    id: hour as u64,
                    occurrence: hour as u64,
                    hour,
                    country: 9001,
                    kind: 1,
                    ..MarketEventRow::default()
                }]))
            } else {
                TableView::Complete(TableRows::MarketEvents(Vec::new()))
            };
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "logs-gap-pages".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let ask = |cursor: Option<String>| LogPageParams {
        branch_id: "1".into(),
        log: "market-events".into(),
        facets: Vec::new(),
        from: Some(100.0),
        to: 200.0,
        cursor,
        page_size: 1,
        columns: None,
    };
    let page0 = LogPage::load(&src, &ask(None), &load, &AtomicBool::new(false)).unwrap();
    let page1 = LogPage::load(&src, &ask(Some("1".into())), &load, &AtomicBool::new(false)).unwrap();
    assert_eq!(page0.rows.iter().map(|r| r.hour).collect::<Vec<_>>(), [110]);
    assert_eq!(page1.rows.iter().map(|r| r.hour).collect::<Vec<_>>(), [190]);
    let gap = |m: &[MissingSpan]| m.iter().map(|s| (s.first_hour, s.last_hour)).collect::<Vec<_>>();
    assert_eq!(gap(&page0.missing), [(140, 150)], "page 0 keeps full-interval truncation gap");
    assert_eq!(gap(&page1.missing), [(140, 150)], "page 1 keeps full-interval truncation gap");
}

#[test]
fn later_damaged_run_does_not_prevent_reading_earlier_healthy_interval() {
    use dashboard_base::test_fixtures::continuation_records;

    let run0_frames: Vec<DecodedFrame> = (100..=150)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let events = if hour == 120 {
                vec![MarketEventRow { id: 1, occurrence: 1, hour, country: 9001, kind: 1, ..MarketEventRow::default() }]
            } else {
                Vec::new()
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let first_part = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, run0_frames.clone())]).unwrap();
    let run1_frames: Vec<DecodedFrame> = (200..=240)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let events = if hour == 220 {
                vec![MarketEventRow { id: 2, occurrence: 2, hour, country: 9001, kind: 1, ..MarketEventRow::default() }]
            } else {
                Vec::new()
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let continued = continuation_records(&first_part, 1, run1_frames);
    let clean = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, run0_frames), continued]).unwrap();
    let entry = clean.recording().table_entry(1, 0, TableId::MarketEvents.index()).unwrap();
    let mut damaged = clean.recording().memory().unwrap().to_vec();
    let mut corrupted = 0usize;
    for part in &entry.unit.parts {
        for frame_entry in &part.frames {
            damaged[(frame_entry.offset + u64::from(frame_entry.stored) / 2) as usize] ^= 0x40;
            corrupted += 1;
        }
    }
    if let Some(first) = &entry.unit.first {
        for (_, chunks) in &first.streams {
            for chunk in chunks {
                let frame_entry = &chunk.frame;
                damaged[(frame_entry.offset + u64::from(frame_entry.stored) / 2) as usize] ^= 0x40;
                corrupted += 1;
            }
        }
    }
    assert!(corrupted > 0, "run 1 MarketEvents block must hold at least one frame to corrupt");
    let replay = Replay::open(damaged).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "logs-later-damage".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let early = LogPage::load(
        &src,
        &LogPageParams { branch_id: "1".into(), log: "market-events".into(), facets: Vec::new(), from: Some(100.0), to: 150.0, cursor: None, page_size: 32, columns: None },
        &load,
        &AtomicBool::new(false),
    )
    .expect("healthy earlier run must succeed despite later run damage");
    assert_eq!(early.rows.iter().map(|r| r.hour).collect::<Vec<_>>(), [120]);
    assert!(early.missing.is_empty());
    let late = LogPage::load(
        &src,
        &LogPageParams { branch_id: "1".into(), log: "market-events".into(), facets: Vec::new(), from: Some(100.0), to: 240.0, cursor: None, page_size: 32, columns: None },
        &load,
        &AtomicBool::new(false),
    );
    assert!(late.is_err(), "reading into the damaged later run still reports the error");
}

#[test]
fn tail_offset_zero_returns_full_interval_coverage_without_count_facets() {
    let frames: Vec<DecodedFrame> = (50..=260)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            *frame.tables[TableId::MarketEvents.index()] = if hour == 150 {
                TableView::Truncated { rows: TableRows::MarketEvents(Vec::new()), missing: 5 }
            } else if (50..=99).contains(&hour) || (201..=250).contains(&hour) {
                let events = (0..3)
                    .map(|k| MarketEventRow { id: hour as u64 * 10 + k, occurrence: hour as u64 * 10 + k, hour, country: 9001, kind: 1, ..MarketEventRow::default() })
                    .collect();
                TableView::Complete(TableRows::MarketEvents(events))
            } else {
                TableView::Complete(TableRows::MarketEvents(Vec::new()))
            };
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "logs-tail-cov".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let tail = LogTail::load(
        &src,
        &LogTailParams {
            branch_id: "1".into(),
            log: "market-events".into(),
            facets: Vec::new(),
            from: Some(50.0),
            hour: Some(250),
            offset: None,
            limit: 64,
            columns: None,
            count_facets: Vec::new(),
        },
        &load,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(tail.rows.len(), 64);
    assert!(tail.rows.last().unwrap().hour > 200, "tail rows lie strictly above the gap at 150");
    let gap = |m: &[MissingSpan]| m.iter().map(|s| (s.first_hour, s.last_hour)).collect::<Vec<_>>();
    assert_eq!(gap(&tail.missing), [(150, 150)], "offset 0 tail carries gap at 150 even with empty countFacets");

    let asked = serde_json::json!({ "panel": "log-tail", "branchId": "1", "log": "market-events", "from": 50, "limit": 64 });
    let panel = dated_panel(&src, &load, &asked).unwrap();
    let exact = panel.exact().unwrap();
    let _seed = (exact.advance)(250, true, &AtomicBool::new(false), None);
    let quiet = (exact.advance)(255, true, &AtomicBool::new(false), None);
    let cold = (exact.cold)(255, &AtomicBool::new(false));
    assert_eq!(*quiet.value, cold, "quiet advance preserves full-interval gap matching cold read");
    let quiet_reply: LogTailReply = serde_json::from_value(quiet.value.as_ref().as_ref().unwrap().clone()).unwrap();
    assert_eq!(gap(&quiet_reply.missing), [(150, 150)]);
}
