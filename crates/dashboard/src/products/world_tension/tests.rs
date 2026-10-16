use crate::test_fixtures::{CAMPAIGN, records, world_frame};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

/// The first hour of day `day`. The tension tables are read every hour, so
/// every recorded hour holding them is a read; a quiet hour publishes the held
/// read, which the fixtures repeat.
fn tension_slot(day: i32) -> i32 {
    assert_eq!(
        replay::cadence::cadence(TableId::WorldTension),
        replay::cadence::Cadence::Hourly
    );
    day * replay::cadence::STAGGER_PERIOD
}

/// A threat source: (country link, date, reason, current, final).
type Threat = (u32, i32, &'static [u8], i64, i64);

/// A world frame holding the tension tables: `aggregates` by slot, `sources` as
/// listed, `total` the world tension; the frame times their walk.
fn tension_frame(
    hour: i32,
    austria: bool,
    total: i64,
    aggregates: &[(u32, i64)],
    sources: &[Threat],
) -> DecodedFrame {
    use replay::rows::{ThreatCountryAggregateRow, ThreatSourceRow, WorldTensionRow};
    let mut frame = world_frame(hour, austria);
    *frame.tables[TableId::WorldTension.index()] =
        TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow {
            total_raw: total,
            source_count: sources.len() as i32,
            country_aggregate_count: 10,
        }]));
    *frame.tables[TableId::ThreatSources.index()] = TableView::Complete(TableRows::ThreatSources(
        sources
            .iter()
            .enumerate()
            .map(
                |(ordinal, &(country, date, reason, current_raw, final_raw))| ThreatSourceRow {
                    ordinal: ordinal as u32,
                    country,
                    second_country: 0,
                    current_raw,
                    final_raw,
                    daily_raw: 0,
                    date,
                    reason: reason.to_vec(),
                },
            )
            .collect(),
    ));
    // Every slot of the census is listed, most at zero.
    *frame.tables[TableId::ThreatCountryAggregates.index()] =
        TableView::Complete(TableRows::ThreatCountryAggregates(
            (0..10)
                .map(|slot| ThreatCountryAggregateRow {
                    country_slot: slot,
                    total_raw: aggregates
                        .iter()
                        .find(|(at, _)| *at == slot)
                        .map_or(0, |(_, raw)| *raw),
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::WorldTension.index()] = 1;
    frame
}

#[test]
fn prepared_tension_requires_history_for_disappeared_country_names_and_pending_tail() {
    use crate::products::world_tension::Scan;
    let first = tension_frame(
        100,
        true,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    let tail = tension_frame(
        101,
        false,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    let format = replay::format::FormatVersion::CURRENT;
    let mut remembered = Scan::default();
    remembered.observe(format, &first).unwrap();
    remembered.observe(format, &tail).unwrap();
    let mut fresh = Scan::default();
    fresh.observe(format, &tail).unwrap();
    assert_eq!(remembered.result.reads.last().unwrap().unnamed_raw, "0");
    assert_eq!(fresh.result.reads.last().unwrap().unnamed_raw, "10000");
    assert_eq!(remembered.result.events[0].tag.as_deref(), Some("D01 mod"));
    assert_eq!(fresh.result.events[0].tag, None);

    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, tail])]).unwrap();
    let root = std::env::temp_dir().join(format!("tension-pending-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("campaign.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let flag = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "tension-pending-cross-frame".into(),
            revision: crate::protocol::replay_revision(&path).unwrap(),
        },
        path,
        &flag,
    )
    .unwrap();
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "series-summary".into(),
        params: serde_json::json!({"branchId":"1","family":{"kind":"world-tension"},"from":100,"to":101,"width":32,"within":{}}),
    };
    let reply: serde_json::Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
    )
    .unwrap();
    assert!(
        reply["data"].is_null(),
        "an unprepared cross-frame tension tail is pending, never a zero-valued chart"
    );
    crate::prepared::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_tension_keeps_equal_read_dates_duplicates_renames_gaps_and_resume_state() {
    use crate::products::{
        BranchParams, LoadContext, Product,
        world_tension::{Scan, WorldTension},
    };
    let mut frames = vec![
        tension_frame(
            100,
            true,
            5000,
            &[(7, 600_000), (8, -100_000)],
            &[
                (11, 90, b"Synthetic cause", 250_000, 350_000),
                (11, 90, b"Synthetic cause", 250_000, 350_000),
            ],
        ),
        tension_frame(
            101,
            true,
            5000,
            &[(7, 600_000), (8, -100_000)],
            &[
                (11, 90, b"Synthetic cause", 250_000, 350_000),
                (11, 90, b"Synthetic cause", 250_000, 350_000),
            ],
        ),
        tension_frame(102, true, 5000, &[(7, 600_000), (8, -100_000)], &[]),
        tension_frame(
            103,
            true,
            5000,
            &[(7, 600_000), (8, -100_000)],
            &[
                (11, 90, b"Synthetic cause", 250_000, 100_000),
                (11, 90, b"Synthetic cause", 250_000, 350_000),
            ],
        ),
        tension_frame(
            104,
            true,
            0,
            &[(7, 0), (8, 0)],
            &[
                (11, 90, b"Synthetic cause", 0, 100_000),
                (11, 90, b"Synthetic cause", 0, 350_000),
            ],
        ),
    ];
    *frames[2].tables[TableId::WorldTension.index()] = TableView::Unavailable {
        reason: "Synthetic missing tension".into(),
    };
    for frame in &mut frames[2..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.iter_mut()
                .find(|row| row.canonical_slot == 7)
                .unwrap()
                .tag = b"renamed mod country".to_vec();
        }
    }
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let flag = AtomicBool::new(false);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "tension".into(),
            revision: "1".into(),
        },
        Default::default(),
        replay.clone(),
        &flag,
    )
    .unwrap();
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut scan = Scan::default();
    for batch in [&frames[..2], &frames[2..]] {
        let mut writer = fixture.writer();
        for frame in batch {
            scan.prepare(replay.format, frame).unwrap();
        }
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        scan = crate::prepared::series::read_catalogue::<Scan>(
            &fixture.store,
            &fixture.key,
            "world-tension",
            1,
        )
        .unwrap()
        .unwrap()
        .1;
        assert_eq!(scan.result.reads.len(), 1);
        assert!(scan.building.iter().all(|country| country.runs.is_empty()));
    }
    let params = BranchParams {
        branch_id: "1".into(),
    };
    let expected =
        WorldTension::load(&source, &params, &LoadContext { content: None }, &flag).unwrap();
    let within: std::collections::BTreeMap<_, _> = scan
        .building
        .iter()
        .enumerate()
        .map(|(index, _)| {
            (
                index.to_string(),
                vec![(index.to_string(), 100, 104, false)],
            )
        })
        .collect();
    let history =
        crate::products::country_history::branch_histories(&source, 1, false, &[], &flag).unwrap();
    let actual = super::prepared::query(
        &source,"series-state",
        &serde_json::json!({"branchId":"1","reads":([100,101,102,103,104].map(|hour|serde_json::json!({"hour":hour})))}),
        &fixture.store,&fixture.key,&history,&flag,
    ).unwrap();
    let next = super::prepared::query(
        &source,"series-next-change",
        &serde_json::json!({"branchId":"1","key":"ArrowRight","from":101,"firstHour":100,"lastHour":104}),
        &fixture.store,&fixture.key,&history,&flag,
    ).unwrap();
    let mut exact = actual.clone();
    for field in [
        "catalogue",
        "summaries",
        "exactHours",
        "navigation",
        "available",
    ] {
        exact.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(exact, serde_json::to_value(&expected).unwrap());
    assert_eq!(next["hour"], 103);
    assert_eq!(expected.events.len(), 2);
    assert_eq!(expected.events[0].added.len(), 2);
    assert_eq!(
        expected
            .reads
            .iter()
            .map(|read| (read.first_hour, read.starts_segment))
            .collect::<Vec<_>>(),
        [(100, true), (101, false), (103, true), (104, false)]
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "tension-series-1")
            .unwrap(),
        Some(2)
    );
    let dense = super::prepared::query(
        &source,"series-summary",
        &serde_json::json!({"branchId":"1","from":100,"to":104,"width":1,"within":within,"share":true}),
        &fixture.store,&fixture.key,&history,&flag,
    ).unwrap();
    let world = dense["summaries"]["world"]["buckets"].as_array().unwrap();
    assert_eq!(
        world
            .iter()
            .filter_map(|bucket| bucket["minValue"].as_f64())
            .reduce(f64::min),
        Some(0.0)
    );
    assert_eq!(
        world
            .iter()
            .filter_map(|bucket| bucket["maxValue"].as_f64())
            .reduce(f64::max),
        Some(5.0)
    );
    assert!(world.iter().any(|bucket| bucket["hasBreak"] == true));
}

#[test]
fn prepared_tension_share_keeps_wide_amounts_without_intermediate_overflow() {
    let frame = tension_frame(100, false, i64::MAX, &[(7, i64::MAX)], &[]);
    let fixture = crate::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = super::Scan::default();
    scan.prepare(replay::format::FormatVersion::CURRENT, &frame)
        .unwrap();
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = crate::prepared::series::Reader::new(
        &fixture.store,
        &fixture.key,
        "tension-series-1".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader.value_at(5, 100).unwrap().unwrap().value,
        i128::from(i64::MAX)
    );
    assert_eq!(
        reader.value_at(6, 100).unwrap().unwrap().value,
        i128::from(i64::MAX) * i128::from(engine::cworld_threat::SCALAR_DIVISOR)
    );
}

#[test]
fn world_tension_sum_overflow_is_refused_instead_of_saturated() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(10);
    for (aggregates, sources, reason) in [
        (vec![(7, i64::MAX), (8, 1)], vec![], "contribution sum"),
        (
            vec![],
            vec![
                (11, slot, b"First".as_slice(), i64::MAX, 0),
                (21, slot, b"Second".as_slice(), 1, 0),
            ],
            "source sum",
        ),
    ] {
        let frame = tension_frame(slot, true, 100_000, &aggregates, &sources);
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let error = match scan(&replay, 1, &AtomicBool::new(false)) {
            Ok(_) => panic!("An overflowing sum was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(reason), "{error}");
    }
}

#[test]
#[ignore = "large event history beyond the former event ceiling"]
fn world_tension_retains_every_event_beyond_the_former_count_ceiling() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(10);
    let sources = vec![(11, slot, b"Synthetic event".as_slice(), 0, 0); 200_001];
    let frame = tension_frame(slot, true, 0, &[], &sources);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.events.len(), sources.len());
}

#[test]
fn world_tension_reads_carry_contributions_and_events_exactly_and_never_cross_gaps() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(500);
    let war: Threat = (
        11,
        slot - 1000,
        b"Declared war on \x11YPoland\x11!",
        400_000,
        400_000,
    );
    let claim: Threat = (21, slot - 500, b"Claims on Albania", 100_000, 200_000);
    let mut frames = Vec::new();
    // A branch-start read one hour before the day: Germany's war and Italy's claim.
    frames.push(tension_frame(
        slot - 1,
        true,
        5_000,
        &[(7, 400_000), (8, 100_000)],
        &[war, claim],
    ));
    // The day's first read: Italy's claim has ramped up; Austria (slot 9) adds 50,000.
    let austria: Threat = (31, slot, b"Threat from actions", 50_000, 50_000);
    let claim_up: Threat = (21, slot - 500, b"Claims on Albania", 200_000, 200_000);
    frames.push(tension_frame(
        slot,
        true,
        6_500,
        &[(7, 400_000), (8, 200_000), (9, 50_000)],
        &[war, claim_up, austria],
    ));
    // Quiet hours publish the held read; Austria is gone from Countries but its
    // slot is remembered.
    for hour in 1..24 {
        frames.push(tension_frame(
            slot + hour,
            false,
            6_500,
            &[(7, 400_000), (8, 200_000), (9, 50_000)],
            &[war, claim_up, austria],
        ));
    }
    // The next day's read: Italy's claim changed what it adds; slot 5 is never named.
    let claim_changed: Threat = (21, slot - 500, b"Claims on Albania", 200_000, 300_000);
    let next_day = |hour| {
        tension_frame(
            hour,
            false,
            7_000,
            &[(7, 400_000), (8, 200_000), (9, 50_000), (5, 50_000)],
            &[
                war,
                claim_changed,
                austria,
                (99, slot + 20, b"Unknown", 50_000, 50_000),
            ],
        )
    };
    frames.push(next_day(slot + 24));
    frames.push(next_day(slot + 25));
    // Hour 26 is a gap; the read at hour 27 starts a new segment.
    frames.push(next_day(slot + 27));
    // A read without its aggregates is not a read.
    let mut incomplete = tension_frame(slot + 48, false, 700, &[], &[]);
    *incomplete.tables[TableId::ThreatCountryAggregates.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    frames.push(incomplete);
    // Later: the sources pass the cap (120%), so world tension is 100%.
    let over: Threat = (
        11,
        slot + 70,
        b"Declared war on the world",
        12_000_000,
        12_000_000,
    );
    frames.push(tension_frame(
        slot + 72,
        false,
        100_000,
        &[(7, 12_000_000)],
        &[over],
    ));
    // World tension that is not the cap of the sum, and aggregates that are not the sources'.
    frames.push(tension_frame(
        slot + 73,
        false,
        5,
        &[(7, 1)],
        &[(11, slot + 70, b"Declared war on the world", 2, 2)],
    ));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let reads: Vec<_> = scanned
        .reads
        .iter()
        .map(|read| {
            (
                read.first_hour - slot,
                read.last_hour - slot,
                read.starts_segment,
                read.tension_raw.as_str(),
                read.sum_raw.as_str(),
                read.unnamed_raw.as_str(),
            )
        })
        .collect();
    let mut expected = vec![(-1, -1, true, "5000", "500000", "0")];
    expected.extend((0..24).map(|hour| (hour, hour, false, "6500", "650000", "0")));
    expected.extend([
        (24, 24, false, "7000", "700000", "50000"),
        (25, 25, false, "7000", "700000", "50000"),
        (27, 27, true, "7000", "700000", "50000"),
        (72, 72, true, "100000", "12000000", "0"),
        (73, 73, false, "5", "1", "0"),
    ]);
    assert_eq!(reads, expected);
    let runs = |tag: &str| {
        scanned
            .countries
            .iter()
            .find(|country| country.tag == tag)
            .unwrap()
            .runs
            .iter()
            .map(|run| {
                (
                    run.first_hour - slot,
                    run.last_hour - slot,
                    run.value_raw.clone(),
                    run.starts_segment,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        scanned
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["D01 mod", "GER", "ITA"]
    );
    assert_eq!(
        runs("GER"),
        [
            (-1, 25, "400000".into(), true),
            (27, 27, "400000".into(), true),
            (72, 72, "12000000".into(), true),
            (73, 73, "1".into(), false)
        ]
    );
    assert_eq!(
        runs("ITA"),
        [
            (-1, -1, "100000".into(), true),
            (0, 25, "200000".into(), false),
            (27, 27, "200000".into(), true)
        ]
    );
    // Austria keeps its tag after its country row is gone; its line starts from a
    // recorded zero, so it is not a new segment, and breaks only at the gap.
    assert_eq!(
        runs("D01 mod"),
        [
            (0, 25, "50000".into(), false),
            (27, 27, "50000".into(), true)
        ]
    );
    assert_eq!(
        (
            scanned.tension_mismatches,
            scanned.aggregate_mismatches,
            scanned.partial_sources
        ),
        (1, 1, 0)
    );
    let events: Vec<_> = scanned
        .events
        .iter()
        .map(|event| {
            (
                event.hour - slot,
                event.tag.as_deref(),
                event.reason.as_str(),
                event
                    .added
                    .iter()
                    .map(|added| (added.hour - slot, added.added_raw.as_str()))
                    .collect::<Vec<_>>(),
                event.first_read - slot,
                event.last_read - slot,
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            (
                -1000,
                Some("GER"),
                "Declared war on Poland",
                vec![(-1, "400000")],
                -1,
                27
            ),
            (
                -500,
                Some("ITA"),
                "Claims on Albania",
                vec![(-1, "200000"), (24, "300000")],
                -1,
                27
            ),
            (
                0,
                Some("D01 mod"),
                "Threat from actions",
                vec![(0, "50000")],
                0,
                27
            ),
            (20, None, "Unknown", vec![(24, "50000")], 24, 27),
            (
                70,
                Some("GER"),
                "Declared war on the world",
                vec![(72, "12000000"), (73, "2")],
                72,
                73
            ),
        ]
    );
}

#[test]
fn world_tension_reply_states_its_scales_and_what_did_not_add_up() {
    use crate::products::{BranchParams, LoadContext, Product, world_tension::WorldTension};
    let slot = tension_slot(10);
    let frames = vec![
        tension_frame(
            slot,
            false,
            900,
            &[(7, 900_000), (4, 1)],
            &[(11, slot, b"War", 900_000, 900_000)],
        ),
        // The next hour publishes the held read again.
        tension_frame(
            slot + 1,
            false,
            900,
            &[(7, 900_000), (4, 1)],
            &[(11, slot, b"War", 900_000, 900_000)],
        ),
    ];
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "wt".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        WorldTension::validate(&BranchParams {
            branch_id: "x".into()
        })
        .is_err()
    );
    let load = |branch: &str| {
        WorldTension::load(
            &source,
            &BranchParams {
                branch_id: branch.into(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load("1");
    assert_eq!(
        (
            reply.tension_scale,
            reply.contribution_scale,
            reply.ceiling_raw.as_str()
        ),
        (1_000, 100_000, "10000000")
    );
    assert_eq!(reply.reads.len(), 2);
    assert!(reply.reads.iter().all(|read| read.unnamed_raw == "1"));
    assert_eq!(reply.warnings.len(), 3, "{:?}", reply.warnings);
    for start in [
        "Reads whose country contributions",
        "Reads whose world tension",
        "Reads with a contribution from a country",
    ] {
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.starts_with(start)),
            "{start}"
        );
    }
    let empty = load("2");
    assert!(empty.reads.is_empty() && empty.warnings[0].starts_with("No world tension"));
}

#[test]
fn world_tension_remembers_countries_recorded_at_hours_without_its_tables() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(20);
    let austria: Threat = (31, slot - 10, b"Threat from actions", 50_000, 50_000);
    // Austria (slot 9, link 31) is recorded only at an hour whose aggregates are missing.
    let mut identified = tension_frame(slot - 24, true, 500, &[(9, 50_000)], &[austria]);
    *identified.tables[TableId::ThreatCountryAggregates.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    // The next read no longer lists Austria's row or link, but its slot still contributes.
    let frames = vec![
        identified,
        tension_frame(slot, false, 500, &[(9, 50_000)], &[austria]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads.len(), 1);
    assert_eq!(scanned.reads[0].unnamed_raw, "0");
    assert_eq!(
        scanned
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["D01 mod"]
    );
    assert_eq!(scanned.events.len(), 1);
    assert_eq!(scanned.events[0].tag.as_deref(), Some("D01 mod"));
}

#[test]
fn world_tension_keeps_one_event_when_its_country_is_named_at_a_later_read() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(30);
    // Link 99 is unresolved at the first read and resolves to Germany's slot 7 at the next.
    let source: Threat = (99, 100, b"Threat from actions", 100_000, 100_000);
    let mut named = tension_frame(slot + 24, false, 1_000, &[(7, 100_000)], &[source]);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *named.tables[TableId::CountryLinks.index()]
    else {
        panic!("world frames record CountryLinks");
    };
    let at = links.partition_point(|link| link.id < 99);
    links.insert(
        at,
        CountryLinkRow {
            id: 99,
            canonical_slot: 7,
        },
    );
    let frames = vec![
        tension_frame(slot, false, 1_000, &[(7, 100_000)], &[source]),
        named,
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let events: Vec<_> = scanned
        .events
        .iter()
        .map(|event| {
            (
                event.hour,
                event.tag.as_deref(),
                event.added.len(),
                event.first_read - slot,
                event.last_read - slot,
            )
        })
        .collect();
    assert_eq!(events, [(100, Some("GER"), 1, 0, 24)]);
}

#[test]
fn world_tension_keeps_two_countries_sharing_a_tag_apart() {
    use crate::products::world_tension::scan;
    let slot = tension_slot(40);
    // Renames the country rows of a frame: (slot, tag), and drops those not listed.
    let retag = |frame: &mut DecodedFrame, tags: &[(u32, &[u8])]| {
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            panic!("world frames record Countries");
        };
        rows.retain_mut(|row| {
            let tag = tags.iter().find(|(at, _)| *at == row.canonical_slot);
            if let Some((_, tag)) = tag {
                row.tag = tag.to_vec();
            }
            tag.is_some()
        });
    };
    // Slot 7 holds AAA and contributes.
    let mut first = tension_frame(slot, false, 1_000, &[(7, 100_000)], &[]);
    retag(&mut first, &[(7, b"AAA"), (8, b"ITA")]);
    // Slot 7 has left the countries but still contributes; slot 8 now holds AAA and contributes too.
    let mut second = tension_frame(slot + 24, false, 1_500, &[(7, 100_000), (8, 50_000)], &[]);
    retag(&mut second, &[(8, b"AAA")]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, second])]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let countries: Vec<_> = scanned
        .countries
        .iter()
        .map(|country| {
            (
                country.tag.as_str(),
                country.slot,
                country
                    .runs
                    .iter()
                    .map(|run| {
                        (
                            run.first_hour - slot,
                            run.last_hour - slot,
                            run.value_raw.as_str(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    // Two countries, each its own runs: nothing overlaps, and neither is Other.
    assert_eq!(
        countries,
        [
            // The reads are a day apart with nothing recorded between: a gap.
            ("AAA", 7, vec![(0, 0, "100000"), (24, 24, "100000")]),
            ("AAA", 8, vec![(24, 24, "50000")]),
        ]
    );
    assert_eq!(scanned.reads[1].unnamed_raw, "0");
}

#[test]
fn tension_changes_keep_quiet_hours_and_closed_history_flat_and_source_changes_avoid_aggregate_joins()
 {
    use super::Scan;
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{assert_flat, scaled_work, unchanged_frames},
    };
    let format = replay::format::FormatVersion::CURRENT;
    let initial = tension_frame(
        100,
        true,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    assert_flat(
        scaled_work(10, |hours| {
            let mut scan = Scan::default();
            let mut tracker = FrameChangeTracker::new();
            scan.prepare_changed(format, &initial, &tracker.observe(&initial, format))?;
            unchanged_frames(
                scan.work,
                &initial,
                hours as i32,
                format,
                |frame, changes| {
                    scan.prepare_changed(format, frame, changes)?;
                    Ok(scan.work)
                },
            )
        })
        .unwrap(),
    );
    assert_flat(
        scaled_work(10, |history| {
            let mut scan = Scan::default();
            let mut tracker = FrameChangeTracker::new();
            let mut current = initial.clone();
            for index in 0..history {
                current.hour = 100 + index as i32;
                if let TableView::Complete(TableRows::ThreatSources(rows)) =
                    &mut *current.tables[TableId::ThreatSources.index()]
                {
                    rows[0].date = current.hour;
                }
                scan.prepare_changed(format, &current, &tracker.observe(&current, format))?;
            }
            assert_eq!(scan.result.events.len(), history);
            unchanged_frames(scan.work, &current, 10, format, |frame, changes| {
                scan.prepare_changed(format, frame, changes)?;
                Ok(scan.work)
            })
        })
        .unwrap(),
    );
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    scan.prepare_changed(format, &initial, &tracker.observe(&initial, format))
        .unwrap();
    let before = scan.work;
    let mut changed = initial.clone();
    changed.hour += 1;
    if let TableView::Complete(TableRows::ThreatSources(rows)) =
        &mut *changed.tables[TableId::ThreatSources.index()]
    {
        rows[0].final_raw += 1;
    }
    scan.prepare_changed(format, &changed, &tracker.observe(&changed, format))
        .unwrap();
    assert_eq!(
        scan.work.row_visits - before.row_visits,
        1,
        "only the changed source is joined; aggregate/identity tables are held"
    );
    assert_eq!(
        scan.work.history_visits - before.history_visits,
        0,
        "a source-only change revisits no country series"
    );
}

#[test]
fn tension_deferred_sources_and_refusal_rates_match_oracle_after_quiet_commits_resume_append_and_gap()
 {
    use super::Scan;
    use crate::prepared::{frame_changes::FrameChangeTracker, series};
    let flag = AtomicBool::new(false);
    let initial = tension_frame(
        100,
        true,
        101,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 9_000, 10_000)],
    );
    let mut frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let mut after = initial.clone();
    after.hour = 113;
    frames.push(after.clone());
    after.hour = 114;
    frames.push(after);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "tension-deferred".into(),
            revision: "1".into(),
        },
        Default::default(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap()),
        &flag,
    )
    .unwrap();
    let mut fixture = series::tests::Fixture::new();
    let mut scan = Scan::default();
    for (index, batch) in [&frames[..6], &frames[6..11], &frames[11..]]
        .into_iter()
        .enumerate()
    {
        let mut tracker = FrameChangeTracker::new();
        for frame in batch {
            scan.prepare_changed(
                source.replay_ref().format,
                frame,
                &tracker.observe(frame, source.replay_ref().format),
            )
            .unwrap();
        }
        let mut writer = if index == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        scan = series::read_catalogue::<Scan>(&fixture.store, &fixture.key, "world-tension", 1)
            .unwrap()
            .unwrap()
            .1;
    }
    let histories =
        crate::products::country_history::branch_histories(&source, 1, false, &[], &flag).unwrap();
    let expected = super::scan(source.replay_ref(), 1, &flag)
        .unwrap()
        .reply(
            &crate::products::BranchParams {
                branch_id: "1".into(),
            },
            &histories,
        )
        .unwrap();
    let mut actual = super::prepared::query(&source, "series-state", &serde_json::json!({"branchId":"1", "reads":frames.iter().map(|frame| serde_json::json!({"hour":frame.hour})).collect::<Vec<_>>()}), &fixture.store, &fixture.key, &histories, &flag).unwrap();
    for field in [
        "catalogue",
        "summaries",
        "exactHours",
        "navigation",
        "available",
    ] {
        actual.as_object_mut().unwrap().remove(field);
    }
    crate::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
    assert_eq!(actual["events"][0]["lastRead"], 114);
}

#[test]
fn tension_identity_change_reuses_unchanged_aggregate_and_source_operands() {
    use super::Scan;
    use crate::prepared::frame_changes::FrameChangeTracker;
    let format = replay::format::FormatVersion::CURRENT;
    let initial = tension_frame(
        100,
        true,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    let mut scan = Scan::default();
    let mut oracle = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    scan.prepare_changed(format, &initial, &tracker.observe(&initial, format))
        .unwrap();
    oracle.observe(format, &initial).unwrap();
    let before = scan.work;
    let mut changed = initial.clone();
    changed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.canonical_slot == 9)
            .unwrap()
            .tag = b"RENAMED".to_vec();
    }
    let changed_rows = changed
        .complete::<replay::rows::tables::Countries>()
        .unwrap()
        .len();
    scan.prepare_changed(format, &changed, &tracker.observe(&changed, format))
        .unwrap();
    oracle.observe(format, &changed).unwrap();
    assert_eq!(
        scan.work.row_visits - before.row_visits,
        changed_rows,
        "neither held native table nor CountryLinks is walked again"
    );
    assert_eq!(scan.result.events, oracle.result.events);
    assert_eq!(scan.result.unnamed_reads, oracle.result.unnamed_reads);
    assert_eq!(scan.building.last().unwrap().tag, "RENAMED");
    assert_eq!(scan.result.reads.last(), oracle.result.reads.last());
}

#[test]
fn tension_one_source_change_never_walks_five_thousand_held_contributions() {
    use super::Scan;
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{Work, assert_flat, scaled_work},
    };
    assert_flat(
        scaled_work(500, |countries| {
            let mut first = crate::test_fixtures::scale_frame(1, 100, countries as u32);
            *first.tables[TableId::WorldTension.index()] =
                TableView::Complete(TableRows::WorldTension(vec![
                    replay::rows::WorldTensionRow {
                        total_raw: countries as i64 / engine::cworld_threat::SCALAR_DIVISOR,
                        source_count: 1,
                        country_aggregate_count: countries as i32,
                    },
                ]));
            *first.tables[TableId::ThreatCountryAggregates.index()] =
                TableView::Complete(TableRows::ThreatCountryAggregates(
                    (0..countries)
                        .map(|slot| replay::rows::ThreatCountryAggregateRow {
                            country_slot: slot as u32,
                            total_raw: 1,
                        })
                        .collect(),
                ));
            *first.tables[TableId::ThreatSources.index()] =
                TableView::Complete(TableRows::ThreatSources(vec![
                    replay::rows::ThreatSourceRow {
                        ordinal: 0,
                        country: 100_000,
                        current_raw: countries as i64,
                        final_raw: countries as i64,
                        date: 90,
                        reason: b"Recorded source".to_vec(),
                        ..Default::default()
                    },
                ]));
            first.metrics.table_ns[TableId::WorldTension.index()] = 1;
            let format = replay::format::FormatVersion::CURRENT;
            let mut scan = Scan::default();
            let mut tracker = FrameChangeTracker::new();
            scan.prepare_changed(format, &first, &tracker.observe(&first, format))?;
            assert_eq!(scan.current.len(), countries);
            let before = scan.work;
            let mut changed = first.clone();
            changed.hour = 101;
            if let TableView::Complete(TableRows::ThreatSources(rows)) =
                &mut *changed.tables[TableId::ThreatSources.index()]
            {
                rows[0].final_raw += 1;
            }
            scan.prepare_changed(format, &changed, &tracker.observe(&changed, format))?;
            let work = Work {
                row_visits: scan.work.row_visits - before.row_visits,
                history_visits: scan.work.history_visits - before.history_visits,
                ..Default::default()
            };
            assert_eq!((work.row_visits, work.history_visits), (1, 0));
            assert_eq!(scan.current.len(), countries);
            assert_eq!(scan.result.events[0].added.len(), 2);
            Ok(work)
        })
        .unwrap(),
    );
}
