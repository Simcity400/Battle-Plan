use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::{
    rows::OperativeMembershipRow, rows::Recorded, rows::SimulationFeatureRow, rows::TableRows,
};
use replay_frame::frame::{FrameDecoder, TableView};
use std::sync::Arc;

pub(super) fn operative(id: u64) -> OperativeMembershipRow {
    OperativeMembershipRow {
        id,
        owner_country: 1007,
        roster_country: 7,
        roster_kind: 0,
        assignment: 3,
        action_variant: Some(1),
        target_country: Some(9).into(),
        name: b"MOD_operative".to_vec().into(),
        ..Default::default()
    }
}

fn countries() -> (Vec<CountryRow>, Vec<CountryLinkRow>) {
    let countries = vec![
        CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"Mod | country".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 9,
            canonical_slot: 2,
            tag: b"renamed target".to_vec(),
            ..Default::default()
        },
    ];
    let links = vec![
        CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 9,
            canonical_slot: 2,
        },
        CountryLinkRow {
            id: 1007,
            canonical_slot: 1,
        },
    ];
    (countries, links)
}

pub(super) fn frame(hour: i32, read: bool, mut rows: Vec<OperativeMembershipRow>) -> DecodedFrame {
    let mut frame = source(1, hour, None);
    let (countries, links) = countries();
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            ..Default::default()
        }]));
    rows.sort_by_key(|r| (r.id, r.roster_country, r.roster_kind));
    *frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(rows));
    if read {
        frame.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
    }
    frame
}

fn replay(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

fn decoded(frame: DecodedFrame) -> DecodedFrame {
    let replay = replay(vec![frame]);
    let frames = replay.read_block(&replay.index[0]).unwrap();
    FrameDecoder::new().advance(&frames[0]).unwrap().clone()
}

fn resolved(rows: Vec<OperativeMembershipRow>) -> BTreeMap<PairKey, Vec<PlacedOperative>> {
    let frame = decoded(frame(10, true, rows));
    let localisation =
        dashboard_text::catalogue::tests::loaded("l_english:\n MOD_operative:0 \"Modded name\"\n");
    resolve_frame(&frame, Some(&localisation), &mut Default::default()).unwrap()
}

fn resolve_frame(
    frame: &DecodedFrame,
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
    warnings: &mut std::collections::BTreeSet<String>,
) -> Result<BTreeMap<PairKey, Vec<PlacedOperative>>, String> {
    let links = frame.complete::<tables::CountryLinks>().unwrap();
    let countries = identities(frame.complete::<tables::Countries>().unwrap(), links).unwrap();
    resolve_rows(
        recorded_operatives(frame).unwrap(),
        &countries,
        links,
        localisation,
        warnings,
    )
}

fn load(frames: Vec<DecodedFrame>) -> OperativePlacementReply {
    load_replay(replay(frames), 1)
}

fn load_replay(replay: Replay, branch: u64) -> OperativePlacementReply {
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "operatives".into(),
            revision: "operative-fixture".into(),
        },
        r"fixtures/operative-placements.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    OperativePlacements::load(
        &source,
        &OperativePlacementParams {
            branch_id: branch.to_string(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn placements_holds_the_last_read_until_the_next_read_without_backfilling() {
    let mut frames = vec![frame(9, false, vec![operative(99)])];
    frames.extend((10..34).map(|hour| frame(hour, hour == 10, vec![operative(1)])));
    frames.extend([
        frame(34, true, vec![operative(2)]),
        frame(35, false, vec![operative(2)]),
    ]);
    let reply = load(frames);
    assert_eq!(reply.read_hours, [10, 34]);
    assert_eq!(reply.periods.len(), 2);
    assert_eq!(
        reply.periods[0].availability,
        PlacementAvailability::TablesNotRecorded
    );
    assert_eq!(
        (reply.periods[1].first_hour, reply.periods[1].last_hour),
        (10, 35)
    );
    let runs = &reply.pairs[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (10, 33));
    assert_eq!(runs[0].operatives[0].id, "1");
    assert_eq!((runs[1].first_hour, runs[1].last_hour), (34, 35));
    assert_eq!(runs[1].operatives[0].id, "2");
}

#[test]
fn placements_gap_ends_a_hold_until_the_next_complete_read() {
    let reply = load(vec![
        frame(10, true, vec![operative(1)]),
        frame(11, false, vec![operative(1)]),
        frame(13, false, vec![operative(1)]),
        frame(14, false, vec![operative(1)]),
        frame(15, true, vec![operative(1)]),
        frame(16, false, vec![operative(1)]),
    ]);
    assert_eq!(reply.read_hours, [10, 15]);
    assert_eq!(
        reply.pairs[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(10, 11), (15, 16)]
    );
    assert_eq!(
        reply
            .periods
            .iter()
            .map(|p| (p.first_hour, p.last_hour, p.availability))
            .collect::<Vec<_>>(),
        [
            (10, 11, PlacementAvailability::Recorded),
            (13, 14, PlacementAvailability::TablesNotRecorded),
            (15, 16, PlacementAvailability::Recorded),
        ]
    );
}

#[test]
fn placements_untimed_scheduled_failures_end_holds_until_a_complete_walk() {
    let failed_hour = (11..35)
        .find(|hour| replay_data::cadence::is_refresh(TableId::OperativeMemberships, *hour))
        .unwrap();
    for view in [
        TableView::Unavailable {
            reason: "walk failed before timing".into(),
        },
        TableView::Truncated {
            rows: TableRows::OperativeMemberships(vec![operative(1)]),
            missing: 1,
        },
    ] {
        let mut frames: Vec<_> = (10..failed_hour)
            .map(|hour| frame(hour, hour == 10, vec![operative(1)]))
            .collect();
        let mut failed = frame(failed_hour, false, vec![]);
        *failed.tables[TableId::OperativeMemberships.index()] = view;
        frames.extend([
            failed,
            frame(failed_hour + 1, false, vec![operative(1)]),
            frame(failed_hour + 2, true, vec![operative(1)]),
            frame(failed_hour + 3, false, vec![operative(1)]),
        ]);
        let reply = load(frames);
        assert_eq!(reply.read_hours, [10, failed_hour + 2]);
        assert_eq!(
            reply.pairs[0]
                .runs
                .iter()
                .map(|r| (r.first_hour, r.last_hour))
                .collect::<Vec<_>>(),
            [(10, failed_hour - 1), (failed_hour + 2, failed_hour + 3)]
        );
        assert_eq!(
            (
                reply.periods[1].first_hour,
                reply.periods[1].last_hour,
                reply.periods[1].availability
            ),
            (
                failed_hour,
                failed_hour + 1,
                PlacementAvailability::TablesNotRecorded
            )
        );
    }
}

#[test]
fn placements_held_keyframe_neither_redates_nor_ends_the_read() {
    let frames: Vec<_> = (10..15)
        .map(|hour| frame(hour, hour == 10, vec![operative(1)]))
        .collect();
    let mut wire = records(1, frames.clone());
    let mut inputs = frames.into_iter();
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    for record in &mut wire {
        if let replay::spool::Record::Frame(recorded) = record {
            let input = inputs.next().unwrap();
            let force = input.hour == 12;
            *recorded = encoder.encode(input, force).unwrap();
            if force {
                assert!(recorded.keyframe);
                assert_eq!(
                    recorded.metrics.table_ns[TableId::OperativeMemberships.index()],
                    0
                );
            }
        }
    }
    let reply = load_replay(Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap(), 1);
    assert_eq!(reply.read_hours, [10]);
    assert_eq!(reply.periods.len(), 1);
    assert_eq!(reply.pairs[0].runs.len(), 1);
    assert_eq!(
        (
            reply.pairs[0].runs[0].first_hour,
            reply.pairs[0].runs[0].last_hour
        ),
        (10, 14)
    );
}

#[test]
fn placements_branch_start_cannot_inherit_a_parent_read() {
    let mut child = frame(12, false, vec![operative(1)]);
    child.branch = 2;
    let mut child_read = frame(13, true, vec![operative(2)]);
    child_read.branch = 2;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(
                1,
                vec![
                    frame(10, true, vec![operative(1)]),
                    frame(11, false, vec![operative(1)]),
                ],
            ),
            records(2, vec![child, child_read]),
        ],
    )
    .unwrap();
    let reply = load_replay(replay, 2);
    assert_eq!(reply.read_hours, [13]);
    assert_eq!(
        reply.periods[0].availability,
        PlacementAvailability::TablesNotRecorded
    );
    assert_eq!(reply.pairs[0].runs[0].first_hour, 13);
    assert_eq!(reply.pairs[0].runs[0].operatives[0].id, "2");
}

#[test]
fn placements_all_missions_operations_reservations_and_exclusions_count_each_identity_once() {
    let mut rows: Vec<_> = (1..=8)
        .map(|variant| OperativeMembershipRow {
            action_variant: Some(variant),
            // Quiet networks count; home counter-intelligence/resistance are self ribbons.
            target_country: Some(if matches!(variant, 3 | 4) { 1007 } else { 9 }).into(),
            // Reservation keeps a mission's target, even if the reserved operation is elsewhere.
            operation: (variant == 2).then_some(811).into(),
            ..operative(u64::from(variant))
        })
        .collect();
    rows.push(OperativeMembershipRow {
        id: 90,
        assignment: 4,
        action_variant: None,
        operation: Some(812).into(),
        ..operative(90)
    });
    rows.push(OperativeMembershipRow {
        roster_country: 9,
        ..rows[0].clone()
    });
    for assignment in [0, 1, 2, 5] {
        rows.push(OperativeMembershipRow {
            assignment,
            target_country: None.into(),
            ..operative(100 + u64::from(assignment))
        });
    }
    for roster_kind in 1..=3 {
        rows.push(OperativeMembershipRow {
            roster_kind,
            ..operative(200 + u64::from(roster_kind))
        });
    }
    rows.extend([
        OperativeMembershipRow {
            assignment: 0,
            capturer_country: 9,
            ..operative(300)
        },
        OperativeMembershipRow {
            action_variant: None,
            target_country: None.into(),
            ..operative(301)
        },
        OperativeMembershipRow {
            action_variant: Some(0),
            target_country: None.into(),
            ..operative(302)
        },
        // Reserving an operation without a mission is still idle.
        OperativeMembershipRow {
            action_variant: None,
            target_country: None.into(),
            operation: Some(811).into(),
            ..operative(303)
        },
    ]);
    let mut frame = frame(10, true, rows);
    *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(vec![
        replay_data::rows::OperationRow {
            id: 811,
            owner: 7,
            target: 7,
            ..Default::default()
        },
    ]));
    let frame = decoded(frame);
    let captured = recorded_operatives(&frame).unwrap();
    let reservation = captured.iter().find(|row| row.id == 2).unwrap();
    assert_eq!(
        (reservation.operation, reservation.target_country),
        (Recorded::Value(Some(811)), Recorded::Value(Some(9)))
    );
    let mut warnings = Default::default();
    let localisation =
        dashboard_text::catalogue::tests::loaded("l_english:\n MOD_operative:0 \"Modded name\"\n");
    let pairs = resolve_frame(&frame, Some(&localisation), &mut warnings).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs.values().map(Vec::len).sum::<usize>(), 9);
    let (home, home_rows) = pairs
        .iter()
        .find(|(key, _)| key.owner == key.target)
        .unwrap();
    assert_eq!(home.owner.2, "Mod | country");
    assert_eq!(
        home_rows.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        ["3", "4"]
    );
    let (foreign, foreign_rows) = pairs
        .iter()
        .find(|(key, _)| key.owner != key.target)
        .unwrap();
    assert_eq!(foreign.target.2, "renamed target");
    assert_eq!(foreign_rows.len(), 7);
    assert!(
        foreign_rows
            .iter()
            .all(|o| o.name.as_deref() == Some("Modded name"))
    );
}

#[test]
fn placements_names_use_saved_keys_literals_renames_and_native_nul_semantics() {
    let rows = resolved(vec![
        operative(1),
        OperativeMembershipRow {
            name: b"Literal modded name [unchanged]".to_vec().into(),
            ..operative(2)
        },
        OperativeMembershipRow {
            name: b"Renamed operative".to_vec().into(),
            ..operative(3)
        },
        OperativeMembershipRow {
            name: vec![].into(),
            ..operative(4)
        },
        OperativeMembershipRow {
            name: b"MOD_operative\0\xff".to_vec().into(),
            ..operative(5)
        },
        OperativeMembershipRow {
            name: b"\0ignored".to_vec().into(),
            ..operative(6)
        },
    ]);
    let names: Vec<_> = rows
        .values()
        .next()
        .unwrap()
        .iter()
        .map(|o| (o.name.as_deref(), o.unnamed))
        .collect();
    assert_eq!(
        names,
        [
            (Some("Modded name"), false),
            (Some("Literal modded name [unchanged]"), false),
            (Some("Renamed operative"), false),
            (None, true),
            (Some("Modded name"), false),
            (None, true)
        ]
    );
}

#[test]
fn placements_capture_eligibility_uses_assignment_not_a_stale_capturer_link() {
    let rows = resolved(vec![
        OperativeMembershipRow {
            capturer_country: 9,
            ..operative(1)
        },
        OperativeMembershipRow {
            assignment: 0,
            capturer_country: 0,
            ..operative(2)
        },
    ]);
    let included: Vec<_> = rows.values().flatten().map(|row| row.id.as_str()).collect();
    assert_eq!(included, ["1"]);
}

#[test]
fn placements_name_refusals_retain_counts_and_name_each_unknown_operative() {
    let frame = decoded(frame(
        10,
        true,
        vec![
            operative(1),
            operative(2),
            OperativeMembershipRow {
                name: vec![0xff].into(),
                ..operative(3)
            },
        ],
    ));
    let localisation =
        dashboard_text::catalogue::tests::loaded("l_english:\n MOD_operative:0 \"Jos\u{14}?\"\n");
    let mut warnings = Default::default();
    let rows = resolve_frame(&frame, Some(&localisation), &mut warnings).unwrap();
    let operatives = rows.values().next().unwrap();
    assert_eq!(operatives.len(), 3);
    assert!(operatives.iter().all(|o| o.name.is_none() && !o.unnamed));
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("2 operative names not shown:") && w.contains("MOD_operative"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("1 operative names not shown:") && w.contains('3'))
    );
}

#[test]
fn placements_unknown_targets_references_and_conflicting_memberships_refuse_whole_reads() {
    for rows in [
        vec![OperativeMembershipRow {
            target_country: None.into(),
            ..operative(1)
        }],
        vec![OperativeMembershipRow {
            target_country: Some(333).into(),
            ..operative(1)
        }],
        vec![OperativeMembershipRow {
            assignment: 4,
            operation: None.into(),
            ..operative(1)
        }],
        vec![
            operative(1),
            OperativeMembershipRow {
                roster_country: 9,
                target_country: Some(7).into(),
                ..operative(1)
            },
        ],
    ] {
        let frame = decoded(frame(10, true, rows));
        assert!(resolve_frame(&frame, None, &mut Default::default()).is_err());
    }
    let frame = decoded(frame(
        10,
        true,
        vec![
            OperativeMembershipRow {
                action_variant: Some(9),
                ..operative(1)
            },
            OperativeMembershipRow {
                action_variant: Some(9),
                ..operative(2)
            },
            OperativeMembershipRow {
                assignment: 6,
                ..operative(3)
            },
            OperativeMembershipRow {
                roster_kind: 4,
                ..operative(4)
            },
        ],
    ));
    let reason = resolve_frame(&frame, None, &mut Default::default()).unwrap_err();
    assert!(reason.contains(
        "2 operative placements not shown: Unsupported operative mission variant 9; 1, 2"
    ));
    assert!(reason.contains("Unsupported operative assignment 6 / roster 0; 3"));
    assert!(reason.contains("Unsupported operative assignment 3 / roster 4; 4"));
}

#[test]
fn placements_failed_reads_clear_holds_and_complete_empty_reads_remain_known() {
    let mut missing = frame(11, true, vec![]);
    *missing.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    let mut truncated = frame(12, true, vec![]);
    *truncated.tables[TableId::OperativeMemberships.index()] = TableView::Truncated {
        rows: TableRows::OperativeMemberships(vec![operative(1)]),
        missing: 1,
    };
    // Even a cadence-slot hour and a timed sibling cannot fabricate this read.
    let mut unread = frame(16, false, vec![operative(1)]);
    unread.metrics.table_ns[TableId::WingCumulative.index()] = 1;
    for frame in [&missing, &truncated] {
        assert!(recorded_operatives(&decoded(frame.clone())).is_none());
    }
    assert_eq!(
        recorded_operatives(&decoded(frame(17, true, vec![]))),
        Some(&[][..])
    );
    let reply = load(vec![
        frame(10, true, vec![operative(1)]),
        missing,
        truncated,
        unread,
        frame(17, true, vec![]),
        frame(18, false, vec![]),
        frame(20, true, vec![operative(1)]),
    ]);
    assert_eq!(reply.read_hours, [10, 17, 20]);
    assert_eq!(reply.pairs.len(), 1);
    assert_eq!(
        reply.pairs[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour, r.count))
            .collect::<Vec<_>>(),
        [(10, 10, 1), (20, 20, 1)]
    );
    assert_eq!(
        reply
            .periods
            .iter()
            .map(|p| (p.first_hour, p.last_hour, p.availability))
            .collect::<Vec<_>>(),
        [
            (10, 10, PlacementAvailability::Recorded),
            (11, 12, PlacementAvailability::TablesNotRecorded),
            (16, 16, PlacementAvailability::TablesNotRecorded),
            (17, 18, PlacementAvailability::Recorded),
            (20, 20, PlacementAvailability::Recorded),
        ]
    );
}

#[test]
fn placements_feature_absence_is_not_inferred_from_empty_or_missing_rosters() {
    let mut absent = frame(10, true, vec![]);
    *absent.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    let mut unknown = frame(11, true, vec![]);
    *unknown.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "feature not read".into(),
    };
    let reply = load(vec![absent, unknown, frame(12, true, vec![])]);
    assert!(reply.pairs.is_empty());
    assert_eq!(reply.read_hours, [12]);
    assert_eq!(
        reply
            .periods
            .iter()
            .map(|p| p.availability)
            .collect::<Vec<_>>(),
        [
            PlacementAvailability::FeatureAbsent,
            PlacementAvailability::TablesNotRecorded,
            PlacementAvailability::Recorded
        ]
    );
}

#[test]
fn placements_feature_changes_and_unknown_features_clear_the_last_read() {
    let mut absent = frame(12, false, vec![operative(1)]);
    *absent.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    let mut unknown = frame(15, false, vec![operative(1)]);
    *unknown.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "feature not read".into(),
    };
    let reply = load(vec![
        frame(10, true, vec![operative(1)]),
        frame(11, false, vec![operative(1)]),
        absent,
        frame(13, false, vec![operative(1)]),
        frame(14, true, vec![operative(1)]),
        unknown,
        frame(16, false, vec![operative(1)]),
        frame(17, true, vec![operative(1)]),
        frame(18, false, vec![operative(1)]),
    ]);
    assert_eq!(reply.read_hours, [10, 14, 17]);
    assert_eq!(
        reply.pairs[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(10, 11), (14, 14), (17, 18)]
    );
    assert_eq!(
        reply
            .periods
            .iter()
            .map(|p| (p.first_hour, p.last_hour, p.availability))
            .collect::<Vec<_>>(),
        [
            (10, 11, PlacementAvailability::Recorded),
            (12, 12, PlacementAvailability::FeatureAbsent),
            (13, 13, PlacementAvailability::TablesNotRecorded),
            (14, 14, PlacementAvailability::Recorded),
            (15, 16, PlacementAvailability::TablesNotRecorded),
            (17, 18, PlacementAvailability::Recorded),
        ]
    );
}

#[test]
fn placements_a_refused_read_clears_placements_until_the_next_complete_read() {
    let reply = load(vec![
        frame(10, true, vec![operative(1)]),
        frame(
            11,
            true,
            vec![OperativeMembershipRow {
                target_country: None.into(),
                ..operative(1)
            }],
        ),
        frame(12, false, vec![operative(1)]),
        frame(13, true, vec![operative(1)]),
    ]);
    assert_eq!(reply.read_hours, [10, 13]);
    assert_eq!(
        reply.pairs[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(10, 10), (13, 13)]
    );
    assert_eq!(
        reply.periods[1].availability,
        PlacementAvailability::TablesNotRecorded
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|w| w.contains("Target not recorded; 1"))
    );
}

#[test]
fn placements_equal_reads_compress_but_operative_names_and_country_identities_split() {
    let first = frame(10, true, vec![operative(1)]);
    let mut renamed_country = frame(13, true, vec![operative(1)]);
    *renamed_country.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![
            CountryRow {
                id: 8,
                canonical_slot: 1,
                tag: b"New opaque country".to_vec(),
                ..Default::default()
            },
            countries().0[1].clone(),
        ]));
    *renamed_country.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 8,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 9,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 1007,
                canonical_slot: 1,
            },
        ]));
    let reply = load(vec![
        first.clone(),
        frame(11, true, vec![operative(1)]),
        frame(
            12,
            true,
            vec![OperativeMembershipRow {
                name: b"New literal name".to_vec().into(),
                ..operative(1)
            }],
        ),
        renamed_country,
    ]);
    assert_eq!(reply.read_hours, [10, 11, 12, 13]);
    assert_eq!(reply.pairs.len(), 2);
    let original = reply
        .pairs
        .iter()
        .find(|p| p.owner == "Mod | country")
        .unwrap();
    assert_eq!(
        original
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(10, 11), (12, 12)]
    );
    assert_eq!(
        original.runs[1].operatives[0].name.as_deref(),
        Some("New literal name")
    );
    let renamed = reply
        .pairs
        .iter()
        .find(|p| p.owner == "New opaque country")
        .unwrap();
    assert_eq!(
        (renamed.runs[0].first_hour, renamed.runs[0].last_hour),
        (13, 13)
    );
}

#[test]
fn held_placements_project_hourly_tags_without_redating_the_membership() {
    let (mut countries, mut links) = countries();
    let mut history = History::default();
    let rows = resolve_rows(
        &[operative(1)],
        &identities(&countries, &links).unwrap(),
        &links,
        None,
        &mut Default::default(),
    )
    .unwrap();
    history.update(10, Observation::Read(rows));
    countries[1].tag = b"hourly renamed target".to_vec();
    history
        .project(&identities(&countries, &links).unwrap())
        .unwrap();
    history.update(11, Observation::Hold);
    assert_eq!(history.reads, [10]);
    let before = history
        .pairs
        .values()
        .find(|pair| pair.target == "renamed target")
        .unwrap();
    let after = history
        .pairs
        .values()
        .find(|pair| pair.target == "hourly renamed target")
        .unwrap();
    assert_eq!(
        (before.runs[0].first_hour, before.runs[0].last_hour),
        (10, 10)
    );
    assert_eq!(
        (after.runs[0].first_hour, after.runs[0].last_hour),
        (11, 11)
    );
    assert_eq!(before.runs[0].operatives, after.runs[0].operatives);
    countries[1].id = 11;
    links[1].id = 11;
    assert!(
        history
            .project(&identities(&countries, &links).unwrap())
            .is_err()
    );
    history.update(
        12,
        Observation::Unavailable(PlacementAvailability::TablesNotRecorded),
    );
    assert!(history.held.is_none());
}
