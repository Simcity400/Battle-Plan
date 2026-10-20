use super::*;
use dashboard_base::test_fixtures::world_frame;
use dashboard_keyed::{Scan, query};
use replay_data::rows::{
    CountryLinkRow, CountryOccupationReserveManpowerRow, CountryRow, TableRows,
};
use replay_frame::frame::TableView;

fn frame(
    hour: i32,
    entries: Option<&[(u32, u32, u32, i32)]>,
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
    *frame.tables[TableId::CountryOccupationReserveManpower.index()] = match entries {
        Some(rows) => TableView::Complete(TableRows::CountryOccupationReserveManpower(
            rows.iter()
                .map(|&(country, ordinal, source_country, amount)| {
                    CountryOccupationReserveManpowerRow {
                        country,
                        ordinal,
                        source_country,
                        amount,
                    }
                })
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "Synthetic reserve refusal".into(),
        },
    };
    frame.metrics.table_ns[TableId::CountryOccupationReserveManpower.index()] = u64::from(fresh);
    frame
}
fn params(value: serde_json::Value) -> OccupationReserveParams {
    serde_json::from_value(value).unwrap()
}
fn observe(scan: &mut Scan<OccupationReserveObserver>, frame: &DecodedFrame) {
    let read = dashboard_text::metadata::slot_read(
        FormatVersion::CURRENT,
        &frame.metrics,
        TableId::CountryOccupationReserveManpower,
        frame.hour,
    );
    scan.prepare(FormatVersion::CURRENT, frame, read).unwrap();
}
fn publish(
    scan: &mut Scan<OccupationReserveObserver>,
    fixture: &dashboard_base::prepared::series::tests::Fixture,
) {
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
}
fn load(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    p: &OccupationReserveParams,
    first: i32,
    last: i32,
    through: Option<i32>,
) -> OccupationReserveReply {
    let flag = AtomicBool::new(false);
    reply(
        query::open::<OccupationReserveObserver>(&fixture.store, &fixture.key, 1, &flag).unwrap(),
        p,
        first,
        last,
        through,
        &flag,
    )
    .unwrap()
}
fn run<'a>(
    reply: &'a OccupationReserveReply,
    name: &str,
) -> Option<&'a dashboard_keyed::query::KeyedSeries> {
    reply.window.series.iter().find(|s| s.name == name)
}

#[test]
fn occupation_reserve_repeated_sources_signed_extremes_and_positive_only_ranking() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &frame(
            100,
            Some(&[
                (1, 0, 2, 100),
                (1, 1, 2, i32::MIN),
                (1, 2, 2, 0),
                (1, 3, 2, 7),
                (2, 0, 1, 90),
            ]),
            true,
            2,
        ),
    );
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","top":1})),
        100,
        100,
        None,
    );
    assert_eq!(reply.holders.len(), 1);
    assert_eq!(reply.holders[0].slot, 1);
    assert_eq!(
        run(&reply, "Mod 1@1|1|amount").unwrap().runs[0].2,
        i32::MIN.to_string()
    );
    assert_eq!(run(&reply, "Mod 1@1|2|amount").unwrap().runs[0].2, "0");
    let source = run(&reply, "Mod 1@1|0|source").unwrap().runs[0].2.clone();
    assert_eq!(reply.sources[&source], "Mod 2@2");
    assert_eq!(run(&reply, "Mod 1@1|1|source").unwrap().runs[0].2, source);
    assert_eq!(reply.window.series.len(), 8);
}
#[test]
fn occupation_reserve_complete_empty_is_not_authenticated_empty_holder() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(&[]), true, 2));
    observe(&mut scan, &frame(101, None, true, 2));
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","view":"holder","tag":"Mod 1@1"})),
        100,
        101,
        None,
    );
    assert_eq!(reply.window.known.len(), 1);
    assert_eq!(reply.window.known[0].last_hour, 100);
    assert!(reply.window.series.is_empty());
    assert_eq!(reply.holders.len(), 1);
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("admission witness"))
    );
}
#[test]
fn occupation_reserve_missing_source_keeps_signed_amount_and_counts_refusal() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &frame(100, Some(&[(1, 0, 999, i32::MAX)]), true, 2),
    );
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        100,
        None,
    );
    assert_eq!(reply.window.series.len(), 1);
    assert_eq!(reply.window.series[0].runs[0].2, i32::MAX.to_string());
    assert!(reply.sources.is_empty());
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("source identity unavailable") && w.contains(": 1."))
    );
}
#[test]
fn occupation_reserve_read_dates_gap_and_serialized_continuation() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(&[(1, 0, 2, 5)]), true, 2));
    scan.advance_quiet(101, false).unwrap();
    publish(&mut scan, &fixture);
    let mut scan = dashboard_base::prepared::series::read_catalogue::<
        Scan<OccupationReserveObserver>,
    >(&fixture.store, &fixture.key, "occupation-reserve", 1)
    .unwrap()
    .unwrap()
    .1;
    observe(&mut scan, &frame(103, Some(&[(1, 0, 2, 5)]), false, 2));
    observe(&mut scan, &frame(104, Some(&[(1, 0, 2, 5)]), true, 2));
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","view":"holder","tag":"Mod 1@1"})),
        100,
        104,
        None,
    );
    assert_eq!(reply.window.reads, [(100, 101, 100), (104, 104, 104)]);
    assert_eq!(
        run(&reply, "Mod 1@1|0|amount").unwrap().runs,
        [(100, 101, "5".into(), true), (104, 104, "5".into(), true)]
    );
    let clipped = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        104,
        Some(101),
    );
    assert_eq!(clipped.window.reads, [(100, 101, 100)]);
}
#[test]
fn occupation_reserve_identity_changes_wait_for_supplier_and_tag_reuse_never_merges() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(&[(1, 0, 2, 4)]), true, 2));
    let mut changed = frame(101, Some(&[(1, 0, 2, 4)]), false, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Renamed mod".to_vec();
        rows[1].tag = b"Mod 1".to_vec();
    }
    observe(&mut scan, &changed);
    changed.hour = 102;
    changed.metrics.table_ns[TableId::CountryOccupationReserveManpower.index()] = 1;
    observe(&mut scan, &changed);
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        102,
        None,
    );
    assert_eq!(
        reply
            .window
            .known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (102, 102)]
    );
    assert_eq!(run(&reply, "Mod 1@1|0|amount").unwrap().runs[0].1, 100);
    assert!(run(&reply, "Renamed mod@1|0|amount").is_some());
    assert!(reply.sources.values().any(|s| s == "Mod 1@2"));
}
#[test]
fn occupation_reserve_equal_forced_reads_are_watched_and_dated() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::<OccupationReserveObserver>::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for hour in 100..=102 {
        let frame = frame(hour, Some(&[(1, 0, 2, 3)]), hour != 101, 2);
        let changes = tracker.observe(&frame, FormatVersion::CURRENT);
        scan.prepare_changed(FormatVersion::CURRENT, &frame, &changes)
            .unwrap();
    }
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        102,
        None,
    );
    assert_eq!(reply.window.reads, [(100, 101, 100), (102, 102, 102)]);
    let fields = OccupationReserveObserver::fields().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f.table == TableId::CountryOccupationReserveManpower && f.read)
    );
}
#[test]
fn occupation_reserve_five_thousand_holders_sparse_top_and_drill() {
    let rows: Vec<_> = (1..=5000).map(|id| (id, 0, 1, id as i32)).collect();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(&rows), true, 5000));
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","top":2})),
        100,
        100,
        None,
    );
    assert_eq!(
        reply.holders.iter().map(|h| h.slot).collect::<Vec<_>>(),
        [5000, 4999]
    );
    assert_eq!(reply.window.series.len(), 4);
    assert_eq!(scan.names().len(), 10000);
    let drill = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","view":"holder","tag":"Mod 2500@2500"})),
        100,
        100,
        None,
    );
    assert_eq!(drill.window.series.len(), 2);
}
#[test]
fn occupation_reserve_cold_validation_and_exported_types() {
    let p = params(serde_json::json!({"branchId":"1"}));
    let cold = reply(None, &p, 100, 105, Some(99), &AtomicBool::new(false)).unwrap();
    assert!(cold.window.known.is_empty());
    assert!(cold.holders.is_empty());
    let bad = params(serde_json::json!({"branchId":"1","view":"holder"}));
    assert!(OccupationReserve::validate(&bad).is_err());
    assert!(
        OccupationReserve::types()
            .iter()
            .any(|s| s.contains("OccupationReserveReply"))
    );
}

#[test]
fn occupation_reserve_public_query_shape_and_cancellation() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let frame = frame(100, Some(&[(1, 0, 2, 8)]), true, 2);
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
    let params = serde_json::json!({"branchId":"1","top":1});
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
    let mut scan = Scan::<OccupationReserveObserver>::default();
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
    assert_eq!(
        decoded,
        ["window", "holders", "sources", "warnings"]
            .into_iter()
            .collect()
    );
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        super::query(&query).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn occupation_reserve_unrepresentable_mod_tag_is_counted_without_losing_other_numbers() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut frame = frame(100, Some(&[(1, 0, 2, 8)]), true, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"Unsupported|tag".to_vec();
    }
    let mut scan = Scan::default();
    observe(&mut scan, &frame);
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1","view":"holder","tag":"Mod 1@1"})),
        100,
        100,
        None,
    );
    assert_eq!(reply.window.series.len(), 2);
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("name grammar") && w.contains(": 1."))
    );
}

#[test]
fn occupation_reserve_untimed_baseline_and_partial_inventory_never_prove_zero() {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, Some(&[(1, 0, 2, 8)]), false, 2));
    let mut partial = frame(101, Some(&[(1, 0, 2, 8)]), true, 2);
    let TableView::Complete(rows) =
        (*partial.tables[TableId::CountryOccupationReserveManpower.index()]).clone()
    else {
        unreachable!()
    };
    *partial.tables[TableId::CountryOccupationReserveManpower.index()] =
        TableView::Truncated { rows, missing: 1 };
    observe(&mut scan, &partial);
    observe(&mut scan, &frame(102, Some(&[(1, 0, 2, 8)]), true, 2));
    publish(&mut scan, &fixture);
    let reply = load(
        &fixture,
        &params(serde_json::json!({"branchId":"1"})),
        100,
        102,
        None,
    );
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
