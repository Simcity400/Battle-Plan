use super::*;
use dashboard_base::test_fixtures::world_frame;
use dashboard_keyed::{Scan, query};
use replay_data::rows::{CountryLinkRow, CountryRow, IntelNetworkSummaryRow, TableRows};
use replay_frame::frame::TableView;

fn network(owner: u32, target: u32) -> IntelNetworkSummaryRow {
    IntelNetworkSummaryRow {
        owner_country: owner,
        target_country: target,
        ..Default::default()
    }
}
fn frame(
    hour: i32,
    entries: Option<Vec<IntelNetworkSummaryRow>>,
    fresh: bool,
    count: u32,
) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("Mod {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::IntelNetworkSummaries.index()] = match entries {
        Some(rows) => TableView::Complete(TableRows::IntelNetworkSummaries(rows)),
        None => TableView::Unavailable {
            reason: "Synthetic summaries refusal".into(),
        },
    };
    frame.metrics.table_ns[TableId::IntelNetworkSummaries.index()] = u64::from(fresh);
    frame
}
fn params(value: serde_json::Value) -> IntelCoverageParams {
    serde_json::from_value(value).unwrap()
}
fn observe(scan: &mut Scan<IntelCoverageObserver>, frame: &DecodedFrame) {
    let read = dashboard_text::metadata::slot_read(
        FormatVersion::CURRENT,
        &frame.metrics,
        TableId::IntelNetworkSummaries,
        frame.hour,
    );
    scan.prepare(FormatVersion::CURRENT, frame, read).unwrap();
}
fn publish(
    scan: &mut Scan<IntelCoverageObserver>,
    fixture: &dashboard_base::prepared::series::tests::Fixture,
) {
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
}
fn load(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    p: &IntelCoverageParams,
    first: i32,
    last: i32,
    through: Option<i32>,
) -> IntelCoverageReply {
    let flag = AtomicBool::new(false);
    reply(
        query::open::<IntelCoverageObserver>(&fixture.store, &fixture.key, 1, &flag).unwrap(),
        p,
        first,
        last,
        through,
        &flag,
    )
    .unwrap()
}
fn pair() -> IntelCoverageParams {
    params(serde_json::json!({"branchId":"1","view":"pair","owner":"Mod 1@1","target":"Mod 2@2"}))
}
fn run<'a>(
    reply: &'a IntelCoverageReply,
    name: &str,
) -> Option<&'a dashboard_keyed::query::KeyedSeries> {
    reply.window.series.iter().find(|s| s.name == name)
}

fn pair_series_count(reply: &IntelCoverageReply) -> usize {
    reply
        .window
        .series
        .iter()
        .filter(|series| series.name.split('|').count() == 3)
        .count()
}

#[test]
fn intel_coverage_six_independent_signed_caches_are_lossless() {
    let row = IntelNetworkSummaryRow {
        covered_core_states: -3,
        covered_controlled_states: 7,
        covered_worth: i32::MAX,
        controlled_strength_sum: i64::MAX,
        core_strength_sum: i64::MIN,
        max_state_strength: -99,
        ..network(1, 2)
    };
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![row]), true, 2));
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 100, None);
    for (measure, value) in [
        ("coveredCore", "-3".to_owned()),
        ("coveredControlled", "7".into()),
        ("coveredWorth", i32::MAX.to_string()),
        ("controlledStrength", i64::MAX.to_string()),
        ("coreStrength", i64::MIN.to_string()),
        ("maxStrength", "-99".into()),
    ] {
        assert_eq!(
            run(&reply, &format!("Mod 1@1|Mod 2@2|{measure}"))
                .unwrap()
                .runs[0]
                .2,
            value
        );
    }
    assert_eq!(pair_series_count(&reply), 6);
    assert!(!reply.window.known.is_empty());
}
#[test]
fn intel_coverage_empty_absent_unknown_are_different_and_directed() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(1, 2)]), true, 2));
    observe(&mut scan, &frame(101, Some(vec![]), true, 2));
    observe(&mut scan, &frame(102, None, true, 2));
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 102, None);
    assert_eq!(
        run(&reply, "Mod 1@1|Mod 2@2|controlledStrength")
            .unwrap()
            .runs,
        [(100, 100, "0".into(), true)]
    );
    assert!(run(&reply, "Mod 2@2|Mod 1@1|controlledStrength").is_none());
    assert_eq!(
        reply
            .window
            .known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101)]
    );
    assert!(reply.window.reads.iter().all(|(_, last, _)| *last <= 101));
    let matrix = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        102,
        None,
    );
    assert_eq!(
        run(&matrix, "rank|Mod 1@1").unwrap().runs,
        [(100, 100, "1".into(), true), (101, 101, "0".into(), false)]
    );
    assert!(
        matrix
            .window
            .series
            .iter()
            .all(|series| series.runs.iter().all(|run| run.1 < 102))
    );
}
#[test]
fn intel_coverage_ranking_is_dated_stable_and_counts_empty_networks() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &frame(
            100,
            Some(vec![network(1, 2), network(2, 1), network(3, 1)]),
            true,
            3,
        ),
    );
    observe(&mut scan, &frame(101, Some(vec![network(2, 3)]), true, 3));
    publish(&mut scan, &fixture);
    let p = params(serde_json::json!({"branchId":"1","shown":2}));
    let early = load(&fixture, &p, 100, 101, Some(100));
    assert_eq!(
        run(&early, "rank|Mod 1@1").unwrap().runs,
        [(100, 100, "3".into(), true)]
    );
    assert_eq!(pair_series_count(&early), 12);
    let later = load(&fixture, &p, 100, 101, None);
    assert_eq!(
        run(&later, "rank|Mod 1@1").unwrap().runs,
        [(100, 100, "3".into(), true), (101, 101, "0".into(), false)]
    );
    assert_eq!(
        run(&later, "rank|Mod 3@3").unwrap().runs,
        [(100, 101, "1".into(), true)]
    );
    assert_eq!(pair_series_count(&later), 24);
    assert_eq!(later.window.reads.last().unwrap().2, 101);
}
#[test]
fn intel_coverage_unresolved_inventory_keeps_independent_numbers_but_not_absence() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &frame(100, Some(vec![network(1, 2), network(1, 999)]), true, 2),
    );
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 100, None);
    assert_eq!(pair_series_count(&reply), 6);
    assert!(reply.window.known.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("endpoint identity unavailable") && w.contains(": 1."))
    );
}
#[test]
fn intel_coverage_gap_refresh_and_serialized_continuation() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(1, 2)]), true, 2));
    scan.advance_quiet(101, false).unwrap();
    publish(&mut scan, &fixture);
    let mut scan = dashboard_base::prepared::series::read_catalogue::<Scan<IntelCoverageObserver>>(
        &fixture.store,
        &fixture.key,
        "intel-network-coverage",
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    observe(&mut scan, &frame(103, Some(vec![network(1, 2)]), false, 2));
    observe(&mut scan, &frame(104, Some(vec![network(1, 2)]), true, 2));
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 104, None);
    assert_eq!(reply.window.reads, [(100, 101, 100), (104, 104, 104)]);
    assert_eq!(
        run(&reply, "Mod 1@1|Mod 2@2|coveredControlled")
            .unwrap()
            .runs,
        [(100, 101, "0".into(), true), (104, 104, "0".into(), true)]
    );
}
#[test]
fn intel_coverage_join_is_frozen_until_read_and_tag_reuse_is_separate() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(1, 2)]), true, 2));
    let mut changed = frame(101, Some(vec![network(1, 2)]), false, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Renamed".to_vec();
        rows[1].tag = b"Mod 1".to_vec();
    }
    observe(&mut scan, &changed);
    changed.hour = 102;
    changed.metrics.table_ns[TableId::IntelNetworkSummaries.index()] = 1;
    observe(&mut scan, &changed);
    publish(&mut scan, &fixture);
    let old = load(&fixture, &pair(), 100, 102, None);
    assert_eq!(pair_series_count(&old), 6);
    assert!(
        old.window
            .series
            .iter()
            .all(|s| s.runs.last().unwrap().1 == 100)
    );
    let now = load(
        &fixture,
        &params(
            serde_json::json!({"branchId":"1","view":"pair","owner":"Renamed@1","target":"Mod 1@2"}),
        ),
        100,
        102,
        None,
    );
    assert_eq!(
        now.window
            .known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(102, 102)]
    );
    assert_eq!(pair_series_count(&now), 6);
}
#[test]
fn intel_coverage_five_thousand_countries_have_no_dense_cube() {
    let rows = (1..=5000)
        .map(|id| network(id, if id == 5000 { 1 } else { id + 1 }))
        .collect();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(rows), true, 5000));
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        100,
        None,
    );
    let ranks: Vec<_> = reply
        .window
        .series
        .iter()
        .filter(|series| series.name.starts_with("rank|"))
        .collect();
    assert_eq!(ranks.len(), 5000);
    assert!(ranks.iter().all(|series| series.runs[0].2 == "2"));
    assert_eq!(scan.names().len(), 35000);
    assert!(pair_series_count(&reply) <= 72);
    let drill = load(
        &fixture,
        &params(
            serde_json::json!({"branchId":"1","view":"pair","owner":"Mod 2500@2500","target":"Mod 2501@2501"}),
        ),
        100,
        100,
        None,
    );
    assert_eq!(pair_series_count(&drill), 6);
    check_reply_budget(&reply).unwrap();
}
#[test]
fn intel_coverage_equal_forced_reads_and_cold_shapes() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::<IntelCoverageObserver>::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for hour in 100..=102 {
        let frame = frame(hour, Some(vec![network(1, 2)]), hour != 101, 2);
        let changes = tracker.observe(&frame, FormatVersion::CURRENT);
        scan.prepare_changed(FormatVersion::CURRENT, &frame, &changes)
            .unwrap();
    }
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 102, None);
    assert_eq!(reply.window.reads, [(100, 101, 100), (102, 102, 102)]);
    let p = params(serde_json::json!({"branchId":"1"}));
    let cold = super::reply(None, &p, 100, 105, Some(99), &AtomicBool::new(false)).unwrap();
    assert!(cold.window.known.is_empty());
    assert!(cold.window.series.is_empty());
    assert_eq!(p.shown, 12);
    assert!(
        IntelCoverage::validate(&params(serde_json::json!({"branchId":"1","view":"pair"})))
            .is_err()
    );
    assert!(
        IntelCoverage::types()
            .iter()
            .any(|s| s.contains("IntelCoverageReply"))
    );
}

#[test]
fn intel_coverage_public_query_shape_and_cancellation() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let frame = frame(100, Some(vec![network(1, 2)]), true, 2);
    let replay = replay::container::Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![frame.clone()],
        )],
    )
    .unwrap();
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "task-five-test".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(replay),
        &flag,
    )
    .unwrap();
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        pending: Vec::new(),
    };
    let params = serde_json::json!({"branchId":"1"});
    let query = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let cold = super::query(&query).unwrap();
    assert!(cold["window"]["series"].as_array().unwrap().is_empty());
    let mut scan = Scan::<IntelCoverageObserver>::default();
    observe(&mut scan, &frame);
    publish(&mut scan, &fixture);
    let value = super::query(&query).unwrap();
    assert!(value["window"]["series"].as_array().unwrap().len() > 0);
    let decoded: std::collections::BTreeSet<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(decoded, ["window", "warnings"].into_iter().collect());
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        super::query(&query).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn intel_coverage_unrepresentable_mod_tag_is_counted_without_losing_other_numbers() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut frame = frame(100, Some(vec![network(1, 2)]), true, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"Unsupported|tag".to_vec();
    }
    let mut scan = Scan::default();
    observe(&mut scan, &frame);
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 100, None);
    assert_eq!(pair_series_count(&reply), 6);
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("name grammar") && w.contains(": 1."))
    );
}

#[test]
fn intel_coverage_untimed_baseline_and_partial_inventory_never_prove_zero() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(1, 2)]), false, 2));
    let mut partial = frame(101, Some(vec![network(1, 2)]), true, 2);
    let TableView::Complete(rows) =
        (*partial.tables[TableId::IntelNetworkSummaries.index()]).clone()
    else {
        unreachable!()
    };
    *partial.tables[TableId::IntelNetworkSummaries.index()] =
        TableView::Truncated { rows, missing: 1 };
    observe(&mut scan, &partial);
    observe(&mut scan, &frame(102, Some(vec![network(1, 2)]), true, 2));
    publish(&mut scan, &fixture);
    let reply = load(&fixture, &pair(), 100, 102, None);
    assert_eq!(
        reply
            .window
            .known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(102, 102)]
    );
    assert!(
        reply
            .window
            .series
            .iter()
            .all(|s| s.runs.iter().all(|r| r.0 == 102))
    );
}

#[test]
fn intel_coverage_r1_top_changes_mid_window_and_departed_members_keep_history() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &frame(
            100,
            Some(vec![
                network(1, 2),
                network(2, 1),
                network(3, 1),
                network(4, 5),
            ]),
            true,
            5,
        ),
    );
    scan.advance_quiet(101, false).unwrap();
    observe(
        &mut scan,
        &frame(
            102,
            Some(vec![network(2, 3), network(3, 2), network(4, 5)]),
            true,
            5,
        ),
    );
    scan.advance_quiet(103, false).unwrap();
    let mut departed = frame(
        104,
        Some(vec![network(2, 3), network(3, 2), network(4, 5)]),
        true,
        5,
    );
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *departed.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != 1);
    }
    observe(&mut scan, &departed);
    publish(&mut scan, &fixture);
    let p = params(serde_json::json!({"branchId":"1", "shown":2}));
    let reply = load(&fixture, &p, 100, 104, None);
    assert_eq!(
        run(&reply, "rank|Mod 1@1").unwrap().runs,
        [(100, 101, "3".into(), true), (102, 103, "0".into(), false)]
    );
    assert_eq!(
        run(&reply, "rank|Mod 2@2").unwrap().runs,
        [(100, 104, "2".into(), true)]
    );
    assert_eq!(
        run(&reply, "rank|Mod 3@3").unwrap().runs,
        [(100, 101, "1".into(), true), (102, 104, "2".into(), false)]
    );
    assert_eq!(
        run(&reply, "member|Mod 1@1").unwrap().runs,
        [(100, 103, "1".into(), true)]
    );
    assert_eq!(
        reply.window.known,
        [HourSpan {
            first_hour: 100,
            last_hour: 104
        }]
    );
    assert_eq!(pair_series_count(&reply), 30);
    assert!(run(&reply, "Mod 1@1|Mod 2@2|coveredControlled").is_some());
    assert!(run(&reply, "Mod 2@2|Mod 3@3|coveredControlled").is_some());
    assert!(run(&reply, "Mod 4@4|Mod 5@5|coveredControlled").is_none());
    let prefix = load(&fixture, &p, 100, 104, Some(101));
    assert_eq!(pair_series_count(&prefix), 12);
    assert!(run(&prefix, "Mod 3@3|Mod 1@1|coveredControlled").is_none());
    assert!(
        prefix
            .window
            .series
            .iter()
            .all(|series| series.runs.iter().all(|run| run.1 <= 101))
    );
    let later = load(&fixture, &p, 102, 104, None);
    assert_eq!(pair_series_count(&later), 12);
    assert!(run(&later, "Mod 1@1|Mod 2@2|coveredControlled").is_none());
}

#[test]
fn intel_coverage_r1_explicit_tags_keep_pairs_without_unknown_ranking_as_zero() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(3, 4)]), true, 4));
    observe(
        &mut scan,
        &frame(101, Some(vec![network(3, 4), network(1, 999)]), true, 4),
    );
    observe(&mut scan, &frame(102, Some(vec![network(3, 4)]), true, 4));
    publish(&mut scan, &fixture);
    let p = params(serde_json::json!({"branchId":"1", "shown":1, "tags":["Mod 3@3", "Mod 4"]}));
    let reply = load(&fixture, &p, 100, 102, None);
    assert_eq!(
        reply.window.known,
        [
            HourSpan {
                first_hour: 100,
                last_hour: 100
            },
            HourSpan {
                first_hour: 102,
                last_hour: 102
            },
        ]
    );
    assert_eq!(
        run(&reply, "rank|Mod 3@3").unwrap().runs,
        [(100, 100, "1".into(), true), (102, 102, "1".into(), true)]
    );
    assert_eq!(
        run(&reply, "member|Mod 4@4").unwrap().runs,
        [(100, 102, "1".into(), true)]
    );
    assert_eq!(
        run(&reply, "Mod 3@3|Mod 4@4|coveredControlled")
            .unwrap()
            .runs,
        [(100, 102, "0".into(), true)]
    );
    assert!(run(&reply, "rank|Mod 1@1").is_none());
    let one_endpoint = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1", "tags":["Mod 3"]})),
        100,
        100,
        None,
    );
    assert_eq!(
        run(&one_endpoint, "rank|Mod 3@3").unwrap().runs,
        [(100, 100, "1".into(), true)]
    );
    assert_eq!(pair_series_count(&one_endpoint), 0);
    let unknown = load(&fixture, &p, 101, 101, None);
    assert!(unknown.window.known.is_empty());
    assert!(
        unknown
            .window
            .series
            .iter()
            .all(|series| !series.name.starts_with("rank|"))
    );
    assert_eq!(pair_series_count(&unknown), 6);
    let empty = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1", "tags":[]})),
        100,
        102,
        None,
    );
    assert!(empty.window.series.is_empty());
}

#[test]
fn intel_coverage_r1_members_and_ranks_follow_gaps_reuse_and_continuation() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(vec![network(1, 2)]), false, 2));
    observe(&mut scan, &frame(101, Some(vec![network(1, 2)]), true, 2));
    observe(&mut scan, &frame(102, None, true, 2));
    publish(&mut scan, &fixture);
    let mut scan = dashboard_base::prepared::series::read_catalogue::<Scan<IntelCoverageObserver>>(
        &fixture.store,
        &fixture.key,
        IntelCoverageObserver::NAME,
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    observe(&mut scan, &frame(104, Some(vec![network(1, 2)]), false, 2));
    let mut changed = frame(105, Some(vec![network(1, 2)]), true, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Renamed".to_vec();
        rows[1].tag = b"Mod 1".to_vec();
    }
    observe(&mut scan, &changed);
    scan.advance_quiet(106, false).unwrap();
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        106,
        None,
    );
    assert_eq!(
        reply.window.known,
        [
            HourSpan {
                first_hour: 101,
                last_hour: 101
            },
            HourSpan {
                first_hour: 105,
                last_hour: 106
            },
        ]
    );
    assert_eq!(
        run(&reply, "rank|Mod 1@1").unwrap().runs,
        [(101, 101, "1".into(), true)]
    );
    assert_eq!(
        run(&reply, "member|Mod 1@1").unwrap().runs,
        [(101, 101, "1".into(), true)]
    );
    assert_eq!(
        run(&reply, "rank|Mod 1@2").unwrap().runs,
        [(105, 106, "1".into(), true)]
    );
    assert_eq!(
        run(&reply, "member|Renamed@1").unwrap().runs,
        [(105, 106, "1".into(), true)]
    );
    let reused = params(
        serde_json::json!({"branchId":"1", "view":"pair", "owner":"Renamed", "target":"Mod 1"}),
    );
    let drill = load(&fixture, &reused, 100, 106, None);
    assert_eq!(
        drill.window.known,
        [HourSpan {
            first_hour: 105,
            last_hour: 106
        }]
    );
    assert_eq!(pair_series_count(&drill), 6);
}

#[test]
fn intel_coverage_r1_sparse_ranking_sweeps_runs_across_the_full_hour_domain() {
    use dashboard_keyed::query::KeyedSeries;
    let mut evidence = KeyedWindow {
        first_hour: i32::MIN,
        last_hour: i32::MAX,
        ..Default::default()
    };
    for id in 1..=5000 {
        evidence.series.push(KeyedSeries {
            name: format!("member|Mod {id}@{id}"),
            runs: vec![(i32::MIN, i32::MAX, "1".into(), true)],
        });
        let next = if id == 5000 { 1 } else { id + 1 };
        evidence.series.push(KeyedSeries {
            name: format!("Mod {id}@{id}|Mod {next}@{next}|coveredControlled"),
            runs: vec![
                (i32::MIN, -1, "0".into(), true),
                (0, i32::MAX, "7".into(), false),
            ],
        });
    }
    let flag = AtomicBool::new(false);
    let ranks = ranking::series(
        &evidence,
        &[HourSpan {
            first_hour: i32::MIN,
            last_hour: i32::MAX,
        }],
        &flag,
    )
    .unwrap();
    assert_eq!(ranks.len(), 5000);
    assert!(
        ranks
            .iter()
            .all(|series| series.runs == [(i32::MIN, i32::MAX, "2".into(), true)])
    );
    let chosen = ranking::shown_anywhere(&ranks, 12, &flag).unwrap();
    let expected: BTreeSet<_> = (1..=5000).map(|id| format!("Mod {id}@{id}")).collect();
    assert_eq!(chosen, expected.into_iter().take(12).collect());
}

#[test]
fn intel_coverage_r1_cold_and_public_queries_use_only_the_dated_window_contract() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let frames = [
        frame(100, Some(vec![network(1, 2)]), true, 2),
        frame(101, None, true, 2),
    ];
    let replay = replay::container::Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames.to_vec())],
    )
    .unwrap();
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "intel-coverage-r1".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(replay),
        &flag,
    )
    .unwrap();
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 101,
            complete: true,
        },
        pending: Vec::new(),
    };
    let p = serde_json::json!({"branchId":"1"});
    let query = Query {
        source: &source,
        params: &p,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let cold = super::query(&query).unwrap();
    assert_eq!(
        cold,
        serde_json::json!({"window":{"firstHour":100,"lastHour":101,"known":[],"reads":[],"series":[]},"warnings":[]})
    );
    let mut scan = Scan::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&mut scan, &fixture);
    let value = super::query(&query).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        ["window", "warnings"].into_iter().collect()
    );
    assert_eq!(
        value["window"]["known"],
        serde_json::json!([{"firstHour":100,"lastHour":100}])
    );
    for name in [
        "rank|Mod 1@1",
        "member|Mod 1@1",
        "Mod 1@1|Mod 2@2|coveredControlled",
    ] {
        let series = value["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|series| series["name"] == name)
            .unwrap();
        assert_eq!(
            series["runs"],
            serde_json::json!([[
                100,
                100,
                if name.ends_with("coveredControlled") {
                    "0"
                } else {
                    "1"
                },
                true
            ]])
        );
    }
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        super::query(&query).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}
