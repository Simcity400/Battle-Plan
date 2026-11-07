use super::test_fixtures::{Threat, tension_frame, tension_slot};
use dashboard_base::test_fixtures::{CAMPAIGN, records};
use replay::campaign::Replay;
use replay_data::{rows::CountryLinkRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn prepared_tension_requires_history_for_disappeared_country_names() {
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
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut remembered = Scan::default();
    remembered.observe(format, &first).unwrap();
    remembered.observe(format, &tail).unwrap();
    let mut fresh = Scan::default();
    fresh.observe(format, &tail).unwrap();
    assert_eq!(
        remembered
            .result
            .reads
            .last()
            .unwrap()
            .unnamed_raw
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        fresh.result.reads.last().unwrap().unnamed_raw.as_deref(),
        Some("10000")
    );
    assert_eq!(remembered.result.events[0].tag.as_deref(), Some("D01 mod"));
    assert_eq!(fresh.result.events[0].tag, None);
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
                read.unnamed_raw.as_deref().unwrap(),
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
    use crate::products::world_tension::WorldTension;
    use dashboard_base::products::{BranchParams, Product};
    use dashboard_core::config::LoadContext;
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
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
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
    assert!(
        reply
            .reads
            .iter()
            .all(|read| read.unnamed_raw.as_deref() == Some("1"))
    );
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
    assert_eq!(scanned.reads[0].unnamed_raw.as_deref(), Some("0"));
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
    assert_eq!(scanned.reads[1].unnamed_raw.as_deref(), Some("0"));
}

// regression: a756f86eb6 Fix diplomacy identity handling and history limits
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

