use super::*;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::*,
};

fn frame(hour: i32, members: &[u32], wars: &[(u32, u32, i32)]) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, 3);
    for table in [
        TableRows::HumanControllers(Vec::new()),
        TableRows::Capitulation(Vec::new()),
        TableRows::RelationTuples(Vec::new()),
        TableRows::SliceReads(Vec::new()),
        TableRows::CountryCounterInputs(Vec::new()),
    ] {
        let index = table.table().index();
        *frame.tables[index] = TableView::Complete(table);
    }
    *frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(if members.is_empty() {
            Vec::new()
        } else {
            vec![FactionRow {
                id: 77,
                name: b"Recorded pact".to_vec(),
                ..Default::default()
            }]
        }));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            members
                .iter()
                .enumerate()
                .map(|(ordinal, index)| FactionMemberRow {
                    faction: 77,
                    country: 100_000 + index,
                    ordinal: ordinal as u32,
                })
                .collect(),
        ));
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        wars.iter()
            .map(|(first, second, start)| WarRelationRow {
                first_country: 100_000 + first,
                second_country: 100_000 + second,
                start_hour: *start,
                first_was_instigator: 1,
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

fn scan_frames(frames: Vec<DecodedFrame>) -> (country_groups::Scan, Scan) {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let groups = country_groups::scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let feed = scan(&replay, 1, &groups, &AtomicBool::new(false)).unwrap();
    (groups, feed)
}

fn tag(index: u32) -> String {
    String::from_utf8(scale_tag(index)).unwrap()
}

fn count(frame: &mut DecodedFrame, owned: i32) {
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 100_000,
                owned_state_count: owned,
                ..Default::default()
            },
        ]));
    frame.metrics.table_ns[TableId::CountryCounterInputs.index()] = 1;
}

#[test]
fn event_feed_diplomacy_existence_uses_owned_count_with_persistent_roster_and_read_bound() {
    let mut first = frame(100, &[], &[]);
    let mut last = frame(102, &[], &[]);
    count(&mut first, 2);
    count(&mut last, 0);
    let (_, feed) = scan_frames(vec![first, last]);
    let rows: Vec<_> = feed
        .moments
        .iter()
        .filter(|row| row.kind == "ceasedToExist")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].hour,
            rows[0].read_hour,
            rows[0].name_hour,
            rows[0].exact
        ),
        (102, 102, 100, false)
    );
    assert_eq!(rows[0].tags, [tag(0)]);

    // Missing a Countries row by itself never asserts nonexistence.
    let first = frame(100, &[], &[]);
    let mut absent = frame(101, &[], &[]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *absent.tables[TableId::Countries.index()]
    {
        rows.remove(0);
    }
    let (_, feed) = scan_frames(vec![first, absent]);
    assert!(!feed.moments.iter().any(|row| row.kind == "ceasedToExist"));
}

#[test]
fn event_feed_diplomacy_dissolved_faction_keeps_id_and_previous_name_hour() {
    let (_, feed) = scan_frames(vec![
        frame(100, &[], &[]),
        frame(101, &[0], &[]),
        frame(102, &[], &[]),
    ]);
    let join = feed
        .moments
        .iter()
        .find(|row| row.kind == "factionJoined")
        .unwrap();
    let left = feed
        .moments
        .iter()
        .find(|row| row.kind == "factionLeft")
        .unwrap();
    assert_eq!(join.faction, Some(77));
    assert_eq!(left.faction, Some(77));
    assert_eq!(
        (left.hour, left.read_hour, left.name_hour, left.exact),
        (102, 102, 101, false)
    );
}

#[test]
fn event_feed_diplomacy_invalid_faction_name_does_not_drop_membership_or_other_events() {
    let mut invalid = frame(101, &[0], &[]);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *invalid.tables[TableId::Factions.index()]
    {
        rows[0].name = vec![0xff];
    }
    let (groups, feed) = scan_frames(vec![frame(100, &[], &[]), invalid, frame(102, &[], &[])]);
    let faction = groups.factions.get(&77).unwrap();
    assert!(faction.name.is_empty());
    assert!(
        groups
            .warnings
            .iter()
            .any(|warning| warning == "1 faction names were not valid text and are not recorded.")
    );
    assert_eq!(faction.members.iter().count(), 1);
    assert!(groups.unobserved.factions.is_empty());
    assert_eq!(
        feed.moments
            .iter()
            .filter(|row| row.kind == "factionJoined" || row.kind == "factionLeft")
            .count(),
        2
    );
    assert!(feed.moments.iter().all(|row| row.faction == Some(77)));
    assert!(
        !feed
            .unavailable
            .iter()
            .any(|entry| entry.key == "factionName")
    );
}

#[test]
fn event_feed_diplomacy_group_gap_boundaries_are_counted_without_invented_events() {
    let (_, feed) = scan_frames(vec![frame(100, &[0], &[(0, 1, 100)]), frame(102, &[], &[])]);
    assert!(
        !feed
            .moments
            .iter()
            .any(|row| matches!(row.kind, "factionLeft" | "peace"))
    );
    assert_eq!(
        feed.unavailable
            .iter()
            .filter(|entry| matches!(entry.key, "factionBoundary" | "warBoundary"))
            .map(|entry| entry.count)
            .sum::<u64>(),
        2
    );
}

#[test]
fn event_feed_diplomacy_war_pairs_aggregate_per_country_hour() {
    let (_, feed) = scan_frames(vec![
        frame(100, &[], &[]),
        frame(101, &[], &[(0, 1, 101), (0, 2, 101)]),
        frame(102, &[], &[]),
    ]);
    let starts: Vec<_> = feed
        .moments
        .iter()
        .filter(|row| row.kind == "warStarted")
        .collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].tags, [tag(0), tag(1), tag(2)]);
    let peace: Vec<_> = feed
        .moments
        .iter()
        .filter(|row| row.kind == "peace")
        .collect();
    assert_eq!(peace.len(), 3);
    assert!(peace.iter().all(|row| row.name_hour == row.hour));
    assert_eq!(
        peace.iter().find(|row| row.tags[0] == tag(0)).unwrap().tags,
        [tag(0), tag(1), tag(2)]
    );
}

#[test]
fn event_feed_diplomacy_guarantee_direction_is_first_to_second_from_either_owner_slice() {
    for owner in [100_000, 100_001] {
        let mut observed = frame(101, &[], &[]);
        *observed.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
                owner_country: owner,
                target_country: if owner == 100_000 { 100_001 } else { 100_000 },
                first_country: 100_000,
                second_country: 100_001,
                start_hour: 101,
                type_id: engine::cguarantee_relation::TYPE,
                ..Default::default()
            }]));
        *observed.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::RelationTuples.index() as u32,
                owner,
                sample_hour: 101,
            }]));
        let (_, feed) = scan_frames(vec![frame(100, &[], &[]), observed]);
        let rows: Vec<_> = feed
            .moments
            .iter()
            .filter(|row| row.kind == "guaranteeGranted")
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tags, [tag(0), tag(1)]);
        assert_eq!(
            (rows[0].hour, rows[0].read_hour, rows[0].exact),
            (101, 101, true)
        );
    }
}

#[test]
fn event_feed_diplomacy_initial_guarantees_are_snapshots_even_when_published_later() {
    for publication_hour in [100, 101] {
        let mut observed = frame(publication_hour, &[], &[]);
        *observed.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
                owner_country: 100_000,
                target_country: 100_001,
                first_country: 100_000,
                second_country: 100_001,
                start_hour: 100,
                type_id: engine::cguarantee_relation::TYPE,
                ..Default::default()
            }]));
        *observed.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::RelationTuples.index() as u32,
                owner: 100_000,
                sample_hour: publication_hour,
            }]));
        let frames = if publication_hour == 100 {
            vec![observed]
        } else {
            vec![frame(100, &[], &[]), observed]
        };
        let (_, feed) = scan_frames(frames);
        assert!(feed.observed.contains(&TableId::RelationTuples));
        assert!(
            !feed
                .moments
                .iter()
                .any(|row| row.kind == "guaranteeGranted")
        );
    }
}

#[test]
fn event_feed_diplomacy_carried_relation_rows_without_a_current_read_are_not_events() {
    let mut carried = frame(101, &[], &[]);
    *carried.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
            owner_country: 100_000,
            target_country: 100_001,
            first_country: 100_000,
            second_country: 100_001,
            start_hour: 101,
            type_id: engine::cguarantee_relation::TYPE,
            ..Default::default()
        }]));
    *carried.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::RelationTuples.index() as u32,
            owner: 100_000,
            sample_hour: 100,
        }]));
    let (_, feed) = scan_frames(vec![frame(100, &[], &[]), carried]);
    assert!(!feed.observed.contains(&TableId::RelationTuples));
    assert!(
        !feed
            .moments
            .iter()
            .any(|row| row.kind == "guaranteeGranted")
    );
}

#[test]
fn event_feed_diplomacy_guarantees_use_only_fresh_owner_ranges() {
    let mut observed = frame(101, &[], &[]);
    *observed.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(
            (0..3)
                .map(|index| RelationTupleRow {
                    owner_country: 100_000 + index,
                    target_country: 100_000 + (index + 1) % 3,
                    first_country: 100_000 + index,
                    second_country: 100_000 + (index + 1) % 3,
                    start_hour: 101,
                    type_id: engine::cguarantee_relation::TYPE,
                    ..Default::default()
                })
                .collect(),
        ));
    *observed.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
        (0..3)
            .map(|index| SliceReadRow {
                table: TableId::RelationTuples.index() as u32,
                owner: 100_000 + index,
                sample_hour: if index == 1 { 101 } else { 100 },
            })
            .collect(),
    ));
    let (_, feed) = scan_frames(vec![frame(100, &[], &[]), observed]);
    assert_eq!(feed.moments.len(), 1);
    assert_eq!(feed.moments[0].kind, "guaranteeGranted");
    assert_eq!(feed.moments[0].tags, [tag(1), tag(2)]);
    assert_eq!(
        (feed.moments[0].hour, feed.moments[0].read_hour),
        (101, 101)
    );
}

#[test]
fn event_feed_diplomacy_unattributed_exact_events_are_visibly_counted() {
    let (groups, feed) = scan_frames(vec![frame(100, &[], &[]), frame(103, &[], &[(0, 1, 102)])]);
    assert!(
        groups
            .warnings
            .iter()
            .any(|warning| warning.starts_with("1 war starts or capitulations"))
    );
    let missing = feed
        .unavailable
        .iter()
        .find(|entry| entry.key == "warStartIdentity")
        .unwrap();
    assert_eq!(missing.count, 1);
    assert!(missing.reason.contains("native start hours"));
}

#[test]
fn event_feed_diplomacy_unavailable_reasons_are_counted_once_per_kind() {
    let mut scan = Scan {
        moments: Vec::new(),
        observed: BTreeSet::new(),
        unavailable: Vec::new(),
    };
    for _ in 0..500 {
        scan.refuse(
            "guaranteeIdentity",
            "Guarantees",
            "Guarantees have unresolved countries at their first recorded read.",
        )
        .unwrap();
    }
    assert_eq!(scan.unavailable.len(), 1);
    assert_eq!(scan.unavailable[0].key, "guaranteeIdentity");
    assert_eq!(scan.unavailable[0].count, 500);
    assert_eq!(
        scan.unavailable[0].reason,
        "Guarantees have unresolved countries at their first recorded read."
    );
    scan.unavailable[0].count = u64::MAX;
    assert_eq!(
        scan.refuse(
            "guaranteeIdentity",
            "Guarantees",
            "Guarantees have unresolved countries at their first recorded read."
        )
        .unwrap_err()
        .to_string(),
        "Event feed guaranteeIdentity unavailable count exceeds u64"
    );
}

#[test]
fn event_feed_diplomacy_guarantee_seen_after_gap_has_observation_bound() {
    let mut observed = frame(103, &[], &[]);
    *observed.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
            owner_country: 100_000,
            target_country: 100_001,
            first_country: 100_000,
            second_country: 100_001,
            start_hour: 102,
            type_id: engine::cguarantee_relation::TYPE,
            ..Default::default()
        }]));
    *observed.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::RelationTuples.index() as u32,
            owner: 100_000,
            sample_hour: 103,
        }]));
    let (_, feed) = scan_frames(vec![frame(100, &[], &[]), observed]);
    let row = feed
        .moments
        .iter()
        .find(|row| row.kind == "guaranteeGranted")
        .unwrap();
    assert_eq!(
        (row.hour, row.read_hour, row.name_hour, row.exact),
        (103, 103, 103, false)
    );
}

#[test]
fn event_feed_diplomacy_multiple_countries_have_exact_existence_results() {
    const COUNTRIES: u32 = 3;
    let frames = [100, 101].map(|hour| {
        let mut frame = frame(hour, &[], &[]);
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                (0..COUNTRIES)
                    .map(|index| CountryCounterInputRow {
                        country: 100_000 + index,
                        owned_state_count: if hour == 100 { 1 } else { 0 },
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame.metrics.table_ns[TableId::CountryCounterInputs.index()] = 1;
        frame
    });
    let (_, feed) = scan_frames(frames.into());
    assert_eq!(feed.moments.len(), COUNTRIES as usize);
    for (index, row) in feed.moments.iter().enumerate() {
        assert_eq!(row.kind, "ceasedToExist");
        assert_eq!(row.tags, [tag(index as u32)]);
        assert_eq!(
            (row.hour, row.read_hour, row.name_hour, row.exact),
            (101, 101, 100, false)
        );
    }
}

#[test]
fn event_feed_diplomacy_existence_identity_change_is_counted() {
    let mut first = frame(100, &[], &[]);
    let mut renamed = frame(101, &[], &[]);
    count(&mut first, 1);
    count(&mut renamed, 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"changed identity".to_vec();
    }
    let (_, feed) = scan_frames(vec![first, renamed]);
    assert!(!feed.moments.iter().any(|row| row.kind == "ceasedToExist"));
    assert_eq!(
        feed.unavailable
            .iter()
            .filter(|entry| entry.key == "existenceIdentity")
            .map(|entry| entry.count)
            .sum::<u64>(),
        1
    );
}

#[test]
fn unread_capitulation_hours_and_unattributed_events_have_separate_counts() {
    let mut missing = frame(101, &[], &[]);
    *missing.tables[TableId::Capitulation.index()] = TableView::Unavailable {
        reason: "missing sample".into(),
    };
    *missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "missing identities".into(),
    };
    let mut observed = frame(102, &[], &[]);
    *observed.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(vec![CapitulationRow {
            country: 100_000,
            capitulated: 1,
            capitulated_hour: 101,
            ..Default::default()
        }]));
    let mut held = observed.clone();
    held.hour = 103;
    let (_, feed) = scan_frames(vec![frame(100, &[], &[]), missing, observed, held]);
    assert!(!feed.moments.iter().any(|row| row.kind == "capitulated"));
    assert_eq!(
        feed.unavailable
            .iter()
            .find(|entry| entry.key == "capitulationCoverage")
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        feed.unavailable
            .iter()
            .find(|entry| entry.key == "capitulationIdentity")
            .unwrap()
            .count,
        1
    );
}
