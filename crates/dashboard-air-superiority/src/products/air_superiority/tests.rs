use super::*;

#[test]
fn air_superiority_viewpoint_indices_and_reads_survive_base_catalogue_growth() {
    let frame = |hour, score| {
        let mut frame = air(hour, score);
        if let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut *frame.tables[TableId::FactionMembers.index()]
        {
            rows.retain(|row| row.country != 66);
        }
        frame
    };
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![frame(23, 0)]);
    let first_groups = groups(&first);
    let mut base = scan_days(&first, 1, &first_groups, None, &cancelled).unwrap();
    let mut view = DayCache {
        viewpoint: Some("c:MOD".into()),
        ..Default::default()
    };
    extend_days_shared(&mut view, &first, 1, &first_groups, Some(&base), &cancelled).unwrap();
    let index = view.side_indices["c:MOD"];
    let old_read = view.sides[index as usize].reads.clone();
    assert!(!base.side_indices.contains_key("f:9"));
    let next = first
        .append(&[continuation_records(&first, 1, vec![frame(47, 200_000)])])
        .unwrap();
    let next_groups = groups(&next);
    extend_days(&mut base, &next, 1, &next_groups, &cancelled).unwrap();
    extend_days_shared(&mut view, &next, 1, &next_groups, Some(&base), &cancelled).unwrap();
    assert_eq!(view.side_indices["c:MOD"], index);
    assert!(!base.side_indices.contains_key("c:MOD"));
    assert_eq!(base.side_indices["f:9"], index);
    assert_eq!(
        &view.sides[index as usize].reads[..old_read.len()],
        old_read
    );
    let Some(AirValue::SideViewpoint { side, .. }) = &view.day(0).unwrap().cells[&7000].0 else {
        panic!("viewpoint cell")
    };
    assert_eq!(*side, index);
    assert_eq!(view.sides[*side as usize].key, "c:MOD");
}

#[test]
fn air_superiority_dissolved_rosters_do_not_supply_historical_anchors_before_reappearance() {
    let frame = |hour| {
        let mut frame = air(hour, 200_000);
        if let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut *frame.tables[TableId::FactionMembers.index()]
        {
            rows.retain(|row| row.country == 9001 || row.country == 44);
        }
        if hour == 101
            && let TableView::Complete(TableRows::Factions(rows)) =
                &mut *frame.tables[TableId::Factions.index()]
        {
            rows.retain(|row| row.id != 9);
        }
        if hour >= 101 {
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(vec![
                    replay_data::rows::WarRelationRow {
                        first_country: 44,
                        second_country: 66,
                        ..Default::default()
                    },
                    replay_data::rows::WarRelationRow {
                        first_country: 9001,
                        second_country: 66,
                        ..Default::default()
                    },
                ]));
        }
        frame
    };
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![frame(100), frame(101)]);
    let mut incremental = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let next = first
        .append(&[continuation_records(&first, 1, vec![frame(102)])])
        .unwrap();
    let next_groups = groups(&next);
    let anchors = assignment::historical_anchors(&next_groups);
    assert_eq!(anchors.faction_hours[&9], 102);
    assert_eq!(anchors.faction_hours[&10], 101);
    assert_eq!(anchors.factions[&9], 10);
    assert_eq!(
        next_groups
            .faction(9)
            .unwrap()
            .3
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (102, 102)]
    );
    extend_days(&mut incremental, &next, 1, &next_groups, &cancelled).unwrap();
    let cold = scan_days(&next, 1, &next_groups, None, &cancelled).unwrap();
    let request = params(AirView::Timeline, 100, 102);
    assert_eq!(
        serde_json::to_value(timeline(&incremental, &request).unwrap()).unwrap(),
        serde_json::to_value(timeline(&cold, &request).unwrap()).unwrap()
    );
    let moment = scan_moment(
        &next,
        1,
        &next_groups,
        &params(AirView::Moment, 102, 102),
        &cancelled,
    )
    .unwrap();
    assert!(
        moment
            .sides
            .iter()
            .any(|side| side.key == "f:10" && side.factions.len() == 2)
    );
    assert!(!moment.sides.iter().any(|side| side.key == "f:9"));
}

#[test]
fn air_superiority_moment_transports_native_look_bounds_and_an_absent_coalition_anchor_read() {
    let cancelled = AtomicBool::new(false);
    let mut replaced = air(1, 200_000);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replaced.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 9001)
            .unwrap()
            .canonical_slot = 11;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *replaced.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 9001)
            .unwrap()
            .canonical_slot = 11;
    }
    let replay = recording(vec![air(0, 200_000), replaced]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 1),
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        reply.segments["GER"],
        [
            AirSpan {
                first_hour: 0,
                last_hour: 0
            },
            AirSpan {
                first_hour: 1,
                last_hour: 1
            }
        ]
    );
    let frames = (0..=2)
        .map(|hour| {
            let mut frame = air(hour, 200_000);
            *frame.tables[TableId::Factions.index()] =
                TableView::Complete(TableRows::Factions(vec![]));
            *frame.tables[TableId::FactionMembers.index()] =
                TableView::Complete(TableRows::FactionMembers(vec![]));
            let mut wars = vec![];
            if hour >= 1 {
                wars.push(replay_data::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 66,
                    ..Default::default()
                });
            }
            if hour <= 1 {
                wars.push(replay_data::rows::WarRelationRow {
                    first_country: 9001,
                    second_country: 66,
                    ..Default::default()
                });
            }
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(wars));
            if hour == 2 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 9001);
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 9001);
                }
                if let TableView::Complete(TableRows::RaidAirScores(rows)) =
                    &mut *frame.tables[TableId::RaidAirScores.index()]
                {
                    rows.retain(|row| row.country != 9001);
                }
            }
            frame
        })
        .collect();
    let replay = recording(frames);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 2, 2),
        &cancelled,
    )
    .unwrap();
    assert!(
        reply
            .sides
            .iter()
            .any(|side| side.key == "w:7:9001" && side.tag.as_deref() == Some("FRA"))
    );
    assert_eq!(reply.anchor_reads["w:7:9001"], ("GER".into(), 0));
    assert_eq!(
        reply.segments["GER"],
        [AirSpan {
            first_hour: 0,
            last_hour: 1
        }]
    );
}

#[test]
fn air_superiority_historical_keys_match_cold_incremental_and_moment_scans() {
    let frame = |hour, joined| {
        let mut frame = air(hour, 200_000);
        let mut wars = vec![replay_data::rows::WarRelationRow {
            first_country: 9001,
            second_country: 66,
            ..Default::default()
        }];
        if joined {
            wars.push(replay_data::rows::WarRelationRow {
                first_country: 44,
                second_country: 66,
                ..Default::default()
            });
            if let TableView::Complete(TableRows::Factions(rows)) =
                &mut *frame.tables[TableId::Factions.index()]
            {
                rows.iter_mut().find(|row| row.id == 10).unwrap().id = 1;
                rows.sort_by_key(|row| row.id);
            }
            if let TableView::Complete(TableRows::FactionMembers(rows)) =
                &mut *frame.tables[TableId::FactionMembers.index()]
            {
                for row in rows {
                    if row.faction == 10 {
                        row.faction = 1;
                    }
                }
            }
        }
        if let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut *frame.tables[TableId::FactionMembers.index()]
        {
            rows.retain(|row| row.country != 66);
        }
        wars.sort_by_key(|row| (row.first_country, row.second_country));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(wars));
        frame
    };
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![frame(23, false)]);
    let mut incremental = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let next = first
        .append(&[continuation_records(&first, 1, vec![frame(47, true)])])
        .unwrap();
    let next_groups = groups(&next);
    extend_days(&mut incremental, &next, 1, &next_groups, &cancelled).unwrap();
    let cold = scan_days(&next, 1, &next_groups, None, &cancelled).unwrap();
    let request = params(AirView::Timeline, 0, 47);
    let incremental = timeline(&incremental, &request).unwrap();
    let cold = timeline(&cold, &request).unwrap();
    assert_eq!(
        serde_json::to_value(&incremental).unwrap(),
        serde_json::to_value(&cold).unwrap()
    );
    let side = cold
        .sides
        .iter()
        .find(|side| side.factions.len() == 1 && side.faction.as_deref() == Some("9"))
        .unwrap();
    assert!(side.reads.iter().any(|read| read.5.len() == 2));
    assert!(!cold.sides.iter().any(|side| side.key == "f:1"));
    let moment = scan_moment(
        &next,
        1,
        &next_groups,
        &params(AirView::Moment, 47, 47),
        &cancelled,
    )
    .unwrap();
    assert!(
        moment
            .sides
            .iter()
            .any(|side| side.key == "f:9" && side.factions.len() == 2)
    );
    assert!(!moment.sides.iter().any(|side| side.key == "f:1"));
}

#[test]
fn air_superiority_failed_extension_preserves_published_segments_and_catalogue() {
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![air(23, 1)]);
    let mut cache = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let old = cache.clone();
    let mut bad = air(47, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *bad.tables[TableId::Countries.index()]
    {
        rows[0].tag = vec![0xff];
    }
    let next = first
        .append(&[continuation_records(&first, 1, vec![bad])])
        .unwrap();
    assert!(extend_days(&mut cache, &next, 1, &groups(&first), &cancelled).is_err());
    assert_eq!(cache.segments.len(), old.segments.len());
    assert!(Arc::ptr_eq(&cache.segments[0], &old.segments[0]));
    assert!(Arc::ptr_eq(&cache.sides, &old.sides));
    assert!(Arc::ptr_eq(&cache.side_indices, &old.side_indices));
}

#[test]
fn air_superiority_compact_sides_project_metadata_to_returned_samples() {
    let mut side = AirSide {
        key: "c:mod".into(),
        faction: None,
        colour_faction: None,
        tag: Some("mod".into()),
        name: "Mod".into(),
        leader: Some("mod".into()),
        co_belligerents: 0,
        factions: vec![],
        reads: [0, 10, 20, 30, 40]
            .map(|hour| {
                (
                    hour,
                    format!("Name {hour}"),
                    None,
                    None,
                    0,
                    vec![],
                    Some(format!("colour {hour}")),
                )
            })
            .into(),
    };
    let mut colour_only = side.reads[1].clone();
    colour_only.0 = 16;
    colour_only.6 = Some("colour 16".into());
    side.reads.insert(2, colour_only);
    let mut cell = AirCell(Some(AirValue::SideViewpoint {
        view: ("1".into(), "2".into()),
        side: 0,
        leader: None,
    }));
    let compact = compact_sides(&[side], &mut [&mut cell], &[15, 16, 17, 20, 30], 30);
    assert_eq!(
        compact[0]
            .reads
            .iter()
            .map(|read| read.0)
            .collect::<Vec<_>>(),
        [15, 16, 20, 30]
    );
    assert_eq!(compact[0].name, "Name 10");
    assert_eq!(compact[0].colour_faction.as_deref(), Some("colour 10"));
    assert_eq!(compact[0].reads[0].1, "Name 10");
    assert_eq!(compact[0].reads[1].6.as_deref(), Some("colour 16"));
}

#[test]
fn air_superiority_region_choices_ignore_viewpoint_and_preserve_group_retention() {
    let cancelled = AtomicBool::new(false);
    let blocks = (1..=5)
        .map(|branch| {
            let mut frame = air(0, 1);
            frame.branch = branch;
            records(branch, vec![frame])
        })
        .collect::<Vec<_>>();
    let replay = Replay::from_records(CAMPAIGN.into(), &blocks).unwrap();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "air-choices".into(),
            revision: "1".into(),
        },
        "choices.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    for branch in 1..=5 {
        let mut request = params(AirView::Choices, 0, 0);
        request.branch_id = branch.to_string();
        request.mode = Some(AirMode::Viewpoint);
        request.viewpoint = Some("not-present".into());
        request.sides = Some(vec!["not-present".into()]);
        let AirReply::Choices(reply) = AirSuperiority::load(
            &source,
            &request,
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap() else {
            unreachable!()
        };
        assert!(!reply.regions.is_empty());
    }
    assert!(
        source
            .append_state::<AirCache>()
            .lock()
            .unwrap()
            .viewpoints
            .is_empty()
    );
    let group_state = source.append_state::<country_groups::GroupCache>();
    let registry = group_state.lock().unwrap();
    assert_eq!(
        registry
            .entries
            .iter()
            .map(|entry| entry.0)
            .collect::<Vec<_>>(),
        [5, 4, 3, 2]
    );
    assert_eq!(
        source
            .append_state::<AirCache>()
            .lock()
            .unwrap()
            .branches
            .iter()
            .map(|entry| entry.0)
            .collect::<Vec<_>>(),
        [5, 4]
    );
}
use dashboard_base::products::country_groups;
use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, ownership_frame, records};
use replay_data::{
    rows::AirWingRow, rows::FactionMemberRow, rows::FactionRow, rows::RaidAirScoreRow,
    rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

pub(super) fn params(view: AirView, first: i32, last: i32) -> AirParams {
    AirParams {
        branch_id: "1".into(),
        view,
        first_hour: Some(first),
        last_hour: Some(last),
        mode: Some(AirMode::Sides),
        viewpoint: None,
        viewpoint_country: None,
        sides: None,
        regions: Some(vec![7000]),
        top: None,
        theatres: None,
        windows_only: None,
    }
}

pub(super) fn air(hour: i32, friendly: i64) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![
        FactionRow {
            id: 9,
            name: b"mod_pact".to_vec(),
            ..Default::default()
        },
        FactionRow {
            id: 10,
            name: b"Other pact".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 44,
                faction: 10,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 55,
                faction: 9,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 66,
                faction: 9,
                ordinal: 2,
            },
            FactionMemberRow {
                country: 9001,
                faction: 9,
                ordinal: 0,
            },
        ]));
    *frame.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(vec![]));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    *frame.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(vec![]));
    *frame.tables[TableId::RaidAirScores.index()] =
        TableView::Complete(TableRows::RaidAirScores(vec![
            RaidAirScoreRow {
                country: 44,
                region: 7000,
                friendly: 100_000,
                enemy: friendly,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            },
            RaidAirScoreRow {
                country: 55,
                region: 7999,
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            },
            RaidAirScoreRow {
                country: 66,
                region: 7999,
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            },
            RaidAirScoreRow {
                country: 9001,
                region: 7000,
                friendly,
                enemy: 100_000,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            },
        ]));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![]));
    frame
}

pub(super) fn recording(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}
pub(super) fn groups(replay: &Replay) -> country_groups::Scan {
    country_groups::scan_branch(replay, 1, &AtomicBool::new(false)).unwrap()
}
fn cell_sides(cell: &AirCell) -> AirSideCell {
    match cell.0.as_ref().unwrap() {
        AirValue::Sides(cell) => cell.clone(),
        AirValue::Scores(scores) => AirSideCell {
            scores: scores.clone(),
            viewpoints: vec![],
            unknown: vec![],
            unknown_set: None,
            no_air_object: vec![],
            filtered: false,
            excluded: 0,
        },
        other => panic!("expected sides, got {other:?}"),
    }
}
fn moment_cell_sides(reply: &AirMomentReply, cell: &AirCell) -> AirSideCell {
    let mut cell = cell_sides(cell);
    if let Some(index) = cell.unknown_set.take() {
        assert!(cell.unknown.is_empty());
        cell.unknown.clone_from(&reply.unknown_sets[index as usize]);
    }
    cell
}

#[test]
fn air_superiority_indexed_daily_snapshots_match_full_reconstruction() {
    let mut frames = Vec::new();
    for hour in 0..49 {
        let mut frame = air(hour, 200_000 + i64::from(hour) * 1000);
        if (10..16).contains(&hour) {
            let TableView::Complete(TableRows::Factions(rows)) =
                &mut *frame.tables[TableId::Factions.index()]
            else {
                unreachable!()
            };
            rows[0].name = b"Renamed pact".to_vec();
        }
        if (20..30).contains(&hour) {
            let TableView::Complete(TableRows::FactionMembers(rows)) =
                &mut *frame.tables[TableId::FactionMembers.index()]
            else {
                unreachable!()
            };
            rows.iter_mut()
                .find(|row| row.country == 55)
                .unwrap()
                .ordinal = 0;
            rows.iter_mut()
                .find(|row| row.country == 9001)
                .unwrap()
                .ordinal = 1;
        }
        if (30..35).contains(&hour) {
            *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
                reason: "fixture war gap".into(),
            };
        }
        if (36..40).contains(&hour) {
            let TableView::Complete(TableRows::CountryLinks(links)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            let canonical_slot = links
                .iter()
                .find(|link| link.id == 9001)
                .unwrap()
                .canonical_slot;
            links.push(replay_data::rows::CountryLinkRow {
                id: 10000,
                canonical_slot,
            });
        }
        if hour == 23 {
            let TableView::Complete(rows) = std::mem::replace(
                &mut *frame.tables[TableId::RaidAirScores.index()],
                TableView::Unavailable {
                    reason: "fixture".into(),
                },
            ) else {
                unreachable!()
            };
            *frame.tables[TableId::RaidAirScores.index()] =
                TableView::Truncated { rows, missing: 1 };
        } else if (24..47).contains(&hour) {
            let TableView::Complete(TableRows::RaidAirScores(rows)) =
                &mut *frame.tables[TableId::RaidAirScores.index()]
            else {
                unreachable!()
            };
            rows.remove(0);
            rows.push(RaidAirScoreRow {
                country: 9001,
                region: 8000,
                friendly: 300_000,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            });
        }
        if (36..40).contains(&hour) {
            let TableView::Complete(TableRows::RaidAirScores(rows)) =
                &mut *frame.tables[TableId::RaidAirScores.index()]
            else {
                unreachable!()
            };
            rows.push(RaidAirScoreRow {
                country: 10000,
                region: 7000,
                friendly: 900_000,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            });
        }
        frames.push(frame);
    }
    let replay = recording(frames);
    let groups = groups(&replay);
    let anchors = assignment::historical_anchors(&groups);
    for view in [(None, false), (Some("GER"), true), (Some("f:9"), false)] {
        let mut full_sides = Vec::new();
        let mut full_indices = HashMap::new();
        let mut indexed_sides = Vec::new();
        let mut indexed_indices = HashMap::new();
        let mut reads = scan::DayReads::default();
        for block in &replay.index {
            let mut full = FrameDecoder::for_tables(scan::MOMENT_TABLES);
            let mut indexed = scan::day_decoder(scan::MOMENT_TABLES);
            for frame in replay
                .read_block_selected(block, scan::MOMENT_TABLES)
                .unwrap()
            {
                let expected_frame = full.advance(&frame).unwrap();
                let indexed_frame = indexed.advance(&frame).unwrap();
                reads.observe(&frame, indexed_frame, true).unwrap();
                let expected = scan::snapshot_kind(
                    expected_frame,
                    &groups,
                    view.0,
                    view.1,
                    (&mut full_sides, &mut full_indices),
                    &anchors,
                    true,
                )
                .unwrap();
                let actual = reads
                    .snapshot(
                        indexed_frame,
                        &groups,
                        view,
                        (&mut indexed_sides, &mut indexed_indices),
                        &anchors,
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    actual.cells, expected.cells,
                    "hour={}, view={view:?}",
                    frame.hour
                );
                assert_eq!(actual.fallback, expected.fallback);
                assert_eq!(actual.active, expected.active);
                assert_eq!(
                    (actual.available, actual.complete, actual.excluded),
                    (expected.available, expected.complete, expected.excluded)
                );
                assert_eq!(actual.wings, expected.wings);
                assert_eq!(actual.wings_known, expected.wings_known);
                assert_eq!(actual.theatres, expected.theatres);
                assert_eq!(actual.theatres_known, expected.theatres_known);
                assert_eq!(indexed_sides, full_sides);
            }
        }
    }
}
fn viewpoint(cell: &AirCell) -> &(String, String) {
    match cell.0.as_ref().unwrap() {
        AirValue::Viewpoint(view) | AirValue::SideViewpoint { view, .. } => view,
        other => panic!("expected viewpoint, got {other:?}"),
    }
}

#[test]
fn equal_values_on_both_sides_of_a_gap_remain_separate_moment_runs() {
    let replay = recording(
        [0, 1, 3, 4]
            .into_iter()
            .map(|hour| {
                let mut frame = air(hour, 200_000);
                *frame.tables[TableId::AirWings.index()] =
                    TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                        id: 17,
                        owner: 9001,
                        mission_region: Some(7000),
                        ..Default::default()
                    }]));
                frame
            })
            .collect(),
    );
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 4),
        &AtomicBool::new(false),
    )
    .unwrap();
    let runs = &reply.regions[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].0, runs[0].1), (0, 1));
    assert_eq!((runs[1].0, runs[1].1), (3, 4));
    assert_eq!(runs[0].2, runs[1].2);
    assert_eq!(reply.regions[0].wings, [(0, 1, 1), (3, 4, 1)]);
    assert_eq!(
        reply.recorded,
        [
            AirSpan {
                first_hour: 0,
                last_hour: 1
            },
            AirSpan {
                first_hour: 3,
                last_hour: 4
            }
        ]
    );
}

#[test]
fn complete_absent_owner_has_no_air_object_and_partial_absence_is_unknown() {
    let mut absent = air(0, 200_000);
    *absent.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 44,
            mission_region: Some(7000),
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::RaidAirScores(rows)) =
        &mut *absent.tables[TableId::RaidAirScores.index()]
    {
        rows.retain(|row| row.country != 9001);
    }
    let replay = recording(vec![absent.clone()]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 0),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cell = moment_cell_sides(&reply, &reply.regions[0].runs[0].2);
    let faction = reply
        .sides
        .iter()
        .position(|side| side.key == "f:9")
        .unwrap() as u32;
    assert_eq!(cell.no_air_object, [faction]);
    assert!(cell.unknown.is_empty());
    assert_eq!(
        reply.complete,
        [AirSpan {
            first_hour: 0,
            last_hour: 0
        }]
    );
    let mut viewpoint = params(AirView::Moment, 0, 0);
    viewpoint.mode = Some(AirMode::Viewpoint);
    viewpoint.viewpoint = Some("f:9".into());
    let side_reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &viewpoint,
        &AtomicBool::new(false),
    )
    .unwrap();
    let side_cell = moment_cell_sides(&side_reply, &side_reply.regions[0].runs[0].2);
    assert_eq!(side_cell.no_air_object.len(), 1);
    assert_eq!(
        side_reply.sides[side_cell.no_air_object[0] as usize]
            .leader
            .as_deref(),
        Some("GER")
    );
    assert!(side_cell.unknown.is_empty());
    viewpoint.viewpoint = Some("GER".into());
    viewpoint.viewpoint_country = Some(true);
    let country_reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &viewpoint,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        country_reply.regions[0].runs[0].2,
        AirCell(Some(AirValue::Reason("no-air-object".into())))
    );
    let complete = absent.clone();
    let TableView::Complete(rows) = std::mem::replace(
        &mut *absent.tables[TableId::RaidAirScores.index()],
        TableView::Unavailable {
            reason: "fixture".into(),
        },
    ) else {
        unreachable!()
    };
    *absent.tables[TableId::RaidAirScores.index()] = TableView::Truncated { rows, missing: 1 };
    // A truncated keyframe is unavailable on the wire; retain partial rows in
    // an ordinary hourly update after a complete baseline.
    absent.hour = 1;
    let replay = recording(vec![complete, absent]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 1, 1),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cell = moment_cell_sides(&reply, &reply.regions[0].runs[0].2);
    let faction = reply
        .sides
        .iter()
        .position(|side| side.key == "f:9")
        .unwrap() as u32;
    assert_eq!(cell.unknown, [faction]);
    assert!(cell.no_air_object.is_empty());
    assert!(reply.complete.is_empty());
}

#[test]
fn selected_sides_keep_a_filtered_state_and_numeric_faction_order() {
    let replay = recording(vec![air(0, 200_000)]);
    let cache = scan_days(&replay, 1, &groups(&replay), None, &AtomicBool::new(false)).unwrap();
    let reply = timeline(&cache, &params(AirView::Timeline, 0, 0)).unwrap();
    assert_eq!(
        reply
            .sides
            .iter()
            .map(|side| side.key.as_str())
            .collect::<Vec<_>>(),
        ["f:9", "f:10"]
    );
    let mut selected = params(AirView::Timeline, 0, 0);
    selected.sides = Some(vec!["f:missing".into()]);
    let reply = timeline(&cache, &selected).unwrap();
    let cell = cell_sides(&reply.rows[0].cells[0].1);
    assert!(cell.filtered);
    assert!(cell.scores.is_empty());
    assert!(reply.sides.is_empty());
    selected.sides = Some(vec![]);
    let cell = AirCell(Some(AirValue::Sides(AirSideCell {
        scores: vec![],
        viewpoints: vec![],
        unknown: vec![0],
        unknown_set: None,
        no_air_object: vec![1],
        filtered: false,
        excluded: 0,
    })));
    assert_eq!(
        super::selected(&cell, &cache.sides, side_filter(&selected).as_ref()),
        cell
    );
}

#[test]
fn wing_only_regions_are_included_and_zero_runs_do_not_fill_the_content_roster() {
    let mut frame = air(0, 200_000);
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 9001,
            mission_region: Some(9000),
            ..Default::default()
        }]));
    let replay = recording(vec![frame]);
    let mut request = params(AirView::Moment, 0, 0);
    request.regions = None;
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .regions
            .iter()
            .map(|region| region.id)
            .collect::<Vec<_>>(),
        [7000, 9000]
    );
    assert_eq!(
        reply
            .regions
            .iter()
            .find(|region| region.id == 9000)
            .unwrap()
            .wings,
        [(0, 0, 1)]
    );
    assert_eq!(
        reply.wings_known,
        [AirSpan {
            first_hour: 0,
            last_hour: 0
        }]
    );
}

#[test]
fn the_largest_region_id_never_supplies_the_absent_region_fallback() {
    let mut frame = air(0, 200_000);
    if let TableView::Complete(TableRows::RaidAirScores(rows)) =
        &mut *frame.tables[TableId::RaidAirScores.index()]
    {
        rows.push(RaidAirScoreRow {
            country: 9001,
            region: u32::MAX,
            friendly: 900_000,
            enemy: 0,
            impact: Default::default(),
            friendly_missions: 0,
            enemy_missions: 0,
            air_spotting_scale: Some(0),
        });
        rows.sort_by_key(|row| (row.country, row.region));
    }
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 9001,
            mission_region: Some(9000),
            ..Default::default()
        }]));
    let replay = recording(vec![frame]);
    let mut request = params(AirView::Moment, 0, 0);
    request.regions = None;
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    let missing = reply
        .regions
        .iter()
        .find(|region| region.id == 9000)
        .unwrap();
    assert!(cell_sides(&missing.runs[0].2).scores.is_empty());
    let largest = reply
        .regions
        .iter()
        .find(|region| region.id == u32::MAX)
        .unwrap();
    assert!(
        cell_sides(&largest.runs[0].2)
            .scores
            .iter()
            .any(|(_, score)| score == "900000")
    );
}

#[test]
fn air_superiority_moment_keeps_absent_object_defaults_when_no_region_is_active() {
    let mut frame = air(0, 0);
    *frame.tables[TableId::RaidAirScores.index()] =
        TableView::Complete(TableRows::RaidAirScores(vec![]));
    *frame.tables[TableId::AirTheatres.index()] =
        TableView::Complete(TableRows::AirTheatres(vec![]));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![]));
    let replay = recording(vec![frame]);
    let mut request = params(AirView::Moment, 0, 0);
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("GER".into());
    request.viewpoint_country = Some(true);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.regions.is_empty());
    assert_eq!(
        reply.fallback,
        [(
            0,
            0,
            AirCell(Some(AirValue::Reason("no-air-object".into())))
        )]
    );
    request.theatres = Some(vec!["700".into()]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.fallback,
        [(
            0,
            0,
            AirCell(Some(AirValue::Reason("outside-theatres".into())))
        )]
    );
}

#[test]
fn air_superiority_ambiguous_canonical_tags_cannot_supply_a_score() {
    let mut frame = air(0, 200_000);
    let groups = groups(&recording(vec![frame.clone()]));
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    let slot = countries
        .iter()
        .find(|row| row.id == 9001)
        .unwrap()
        .canonical_slot;
    countries
        .iter_mut()
        .find(|row| row.id == 44)
        .unwrap()
        .canonical_slot = slot;
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links
        .iter_mut()
        .find(|row| row.id == 44)
        .unwrap()
        .canonical_slot = slot;
    let sample = scan::snapshot(&frame, &groups, None, &mut vec![]).unwrap();
    assert!(sample.cells.is_empty());
    assert_eq!(
        sample.fallback,
        AirCell(Some(AirValue::Reason("identity-not-recorded".into())))
    );
}

#[test]
fn unsupported_tag_bytes_are_refused_by_form_and_count() {
    for bytes in [vec![0xff], b"opaque\0tag".to_vec()] {
        let mut frame = air(0, 200_000);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = bytes;
        }
        let replay = recording(vec![frame, air(1, 200_000)]);
        let error = scan_moment(
            &replay,
            1,
            &groups(&replay),
            &params(AirView::Moment, 0, 0),
            &AtomicBool::new(false),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("1 recorded country tags"));
        assert!(error.to_string().contains("UTF-8 without NUL"));
        // Choices retain every observed identity, including one replaced before
        // the day's last read. Refuse unsupported bytes there too.
        let error = scan_days(&replay, 1, &groups(&replay), None, &AtomicBool::new(false))
            .err()
            .unwrap();
        assert!(error.to_string().contains("1 recorded country tags"));
        assert!(error.to_string().contains("UTF-8 without NUL"));
    }
}

#[test]
fn timeline_days_are_dated_gap_columns_are_null_and_latest_partial_read_has_no_future() {
    let replay = recording(vec![
        air(1, 100_000),
        air(23, 200_000),
        air(49, 300_000),
        air(60, 400_000),
    ]);
    let groups = groups(&replay);
    let mut cache = scan_days(&replay, 1, &groups, None, &AtomicBool::new(false)).unwrap();
    let reply = timeline(&cache, &params(AirView::Timeline, 0, 60)).unwrap();
    assert_eq!(reply.days, [Some(23), None, Some(60)]);
    assert_eq!(reply.rows[0].cells[1].1, AirCell(None));
    // Seeking before the stored final read must select the last read at or
    // before the requested hour, never borrow the future day's final score.
    let sample = scan::partial_day(&replay, 1, &groups, &mut cache, 49, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    cache.partial = Some((49, std::sync::Arc::new(sample)));
    let reply = timeline(&cache, &params(AirView::Timeline, 0, 49)).unwrap();
    assert_eq!(reply.days, [Some(23), None, Some(49)]);
    let cell = cell_sides(&reply.rows[0].cells.last().unwrap().1);
    assert!(cell.scores.iter().any(|(_, raw)| raw == "300000"));
    assert!(!cell.scores.iter().any(|(_, raw)| raw == "400000"));
}

#[test]
fn moment_window_accepts_5000_hours_and_keeps_every_recorded_read() {
    let replay = recording(vec![air(0, 200_000), air(4999, 300_000)]);
    let request = params(AirView::Moment, 0, 4999);
    assert!(AirSuperiority::validate(&request).is_ok());
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((reply.first_hour, reply.last_hour), (0, 4999));
    assert_eq!(
        reply.recorded,
        [
            AirSpan {
                first_hour: 0,
                last_hour: 0
            },
            AirSpan {
                first_hour: 4999,
                last_hour: 4999
            }
        ]
    );
    assert_eq!(reply.regions[0].runs.len(), 2);
    assert_eq!(reply.regions[0].runs[1].0, 4999);
    assert!(check_reply_size(&AirReply::Moment(reply)).is_ok());
}

#[test]
fn country_viewpoint_keeps_recorded_scores_without_faction_or_war_tables() {
    let mut frame = air(0, 200_003);
    for table in [
        TableId::Factions,
        TableId::FactionMembers,
        TableId::WarRelations,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "fixture groups not recorded".into(),
        };
    }
    let replay = recording(vec![frame.clone()]);
    let grouped = groups(&replay);
    let mut request = params(AirView::Moment, 0, 0);
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("GER".into());
    request.viewpoint_country = Some(true);
    let reply = scan_moment(&replay, 1, &grouped, &request, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        viewpoint(&reply.regions[0].runs[0].2),
        &("200003".into(), "100000".into())
    );
    let cache = scan::scan_days_kind(
        &replay,
        1,
        &grouped,
        Some("GER"),
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    request.view = AirView::Timeline;
    let reply = timeline(&cache, &request).unwrap();
    assert_eq!(
        viewpoint(&reply.rows[0].cells[0].1),
        &("200003".into(), "100000".into())
    );
    let side = scan::snapshot(&frame, &grouped, Some("f:9"), &mut vec![]).unwrap();
    assert_eq!(
        side.fallback,
        AirCell(Some(AirValue::Reason("sides-not-recorded".into())))
    );
}

#[test]
fn viewpoint_keeps_exact_fixed5_values_and_a_witness_certifies_only_complete_absent_pairs() {
    let replay = recording(vec![air(0, 200_003)]);
    let mut request = params(AirView::Moment, 0, 0);
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("f:9".into());
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        viewpoint(&reply.regions[0].runs[0].2),
        &("200003".into(), "100000".into())
    );
    let AirValue::SideViewpoint { side, .. } = reply.regions[0].runs[0].2.0.as_ref().unwrap()
    else {
        panic!("side viewpoint must identify dated leadership")
    };
    assert_eq!(reply.sides[*side as usize].leader.as_deref(), Some("GER"));
    let mut direct = request.clone();
    direct.viewpoint = Some("GER".into());
    direct.viewpoint_country = Some(true);
    let direct_reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &direct,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        direct_reply.regions[0].runs[0].2,
        AirCell(Some(AirValue::Viewpoint((
            "200003".into(),
            "100000".into()
        ))))
    );
    let mut frame = air(0, 200_003);
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 9001,
            mission_region: Some(9000),
            ..Default::default()
        }]));
    let replay = recording(vec![frame.clone()]);
    request.regions = Some(vec![9000]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        viewpoint(&reply.regions[0].runs[0].2),
        &("0".into(), "0".into())
    );
    let TableView::Complete(rows) = std::mem::replace(
        &mut *frame.tables[TableId::RaidAirScores.index()],
        TableView::Unavailable {
            reason: "fixture".into(),
        },
    ) else {
        unreachable!()
    };
    *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated { rows, missing: 1 };
    let replay = recording(vec![frame]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    let cell = moment_cell_sides(&reply, &reply.regions[0].runs[0].2);
    assert_eq!(cell.unknown.len(), 1);
    assert_eq!(reply.sides[cell.unknown[0] as usize].key, "f:9");
    assert_eq!(
        reply.sides[cell.unknown[0] as usize].leader.as_deref(),
        Some("GER")
    );
    assert!(cell.scores.is_empty());
    assert!(cell.no_air_object.is_empty());
    request.viewpoint = Some("GER".into());
    request.viewpoint_country = Some(true);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.regions[0].runs[0].2,
        AirCell(Some(AirValue::Reason("score-not-recorded".into())))
    );
}

#[test]
fn theatre_projection_distinguishes_outside_regions_and_missing_theatre_reads() {
    let mut frame = air(0, 200_000);
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 9001,
            mission_region: Some(7000),
            ..Default::default()
        }]));
    *frame.tables[TableId::AirTheatres.index()] =
        TableView::Complete(TableRows::AirTheatres(vec![
            replay_data::rows::AirTheatreRow {
                id: 700,
                owner: 9001,
                name: b"Modded theatre".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![
            replay_data::rows::AirTheatreGroupRow {
                id: 701,
                theatre: 700,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![
            replay_data::rows::AirTheatreMemberRow {
                wing: 17,
                group: 701,
                ..Default::default()
            },
        ]));
    let replay = recording(vec![frame.clone()]);
    let mut dictionary = Vec::new();
    let mut sample = scan::snapshot(&frame, &groups(&replay), None, &mut dictionary).unwrap();
    assert!(sample.theatres_known);
    assert_eq!(sample.theatres.get("700"), Some(&[7000].into()));
    let mut request = params(AirView::Moment, 0, 0);
    request.theatres = Some(vec!["700".into()]);
    let keys = side_filter(&request);
    let theatres = TheatreFilter::new(std::iter::once(&sample), &request);
    assert!(
        !cell_sides(&cell_at(
            &sample,
            7000,
            &dictionary,
            theatres.at(&sample),
            keys.as_ref()
        ))
        .scores
        .is_empty()
    );
    let excluded = cell_at(
        &sample,
        9000,
        &dictionary,
        theatres.at(&sample),
        keys.as_ref(),
    );
    assert_eq!(
        excluded,
        AirCell(Some(AirValue::Reason("outside-theatres".into())))
    );
    sample.theatres_known = false;
    assert_eq!(
        cell_at(
            &sample,
            7000,
            &dictionary,
            theatres.at(&sample),
            keys.as_ref()
        ),
        AirCell(Some(AirValue::Reason("theatres-not-recorded".into())))
    );
    request.theatres = None;
    let keys = side_filter(&request);
    let theatres = TheatreFilter::new(std::iter::once(&sample), &request);
    assert!(
        !cell_sides(&cell_at(
            &sample,
            7000,
            &dictionary,
            theatres.at(&sample),
            keys.as_ref()
        ))
        .scores
        .is_empty()
    );
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((reply.first_hour, reply.last_hour), (0, 0));
    assert_eq!(
        reply.recorded,
        [AirSpan {
            first_hour: 0,
            last_hour: 0
        }]
    );
}

#[test]
fn recorded_content_stack_supports_extra_regions_and_overridden_mod_faction_names() {
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "base/common/factions/templates/a.txt",
            "mod_template={name=mod_pact}",
        ),
        (
            "base/map/strategicregions/a.txt",
            "strategic_region={id=7000 name=base_region provinces={1}}",
        ),
        (
            "base/localisation/english/a_l_english.yml",
            "\u{feff}l_english:\n base_region:0 \"Base region\"\n mod_pact:0 \"Base pact\"\n",
        ),
        (
            "mod/map/strategicregions/a.txt",
            "strategic_region={id=7000 name=changed_region provinces={1}} strategic_region={id=9000 name=extra_region provinces={2}}",
        ),
        (
            "mod/localisation/english/a_l_english.yml",
            "\u{feff}l_english:\n changed_region:0 \"Modded region\"\n extra_region:0 \"Extra air region\"\n mod_pact:0 \"Modded coalition\"\n",
        ),
    ]);
    let base = folder.0.join("base");
    let modification = folder.0.join("mod");
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&base, &[]), (&modification, &[])])
            .unwrap();
    let regions = installed_content_regions::regions::read(&layers).unwrap();
    let loc = installed_content_localisation::localisation::read(&layers, "english").unwrap();
    assert_eq!(
        catalogue::localised(
            &loc,
            &regions.regions[&7000].name_key,
            catalogue::View::Plain
        )
        .unwrap(),
        "Modded region"
    );
    assert_eq!(
        catalogue::localised(
            &loc,
            &regions.regions[&9000].name_key,
            catalogue::View::Plain
        )
        .unwrap(),
        "Extra air region"
    );
    let mut frame = air(0, 200_000);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    {
        rows[0].name.clear();
        rows[0].template_token = Some(17);
    }
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay_data::rows::LexicalTokenRow {
                token: 17,
                literal: b"mod_template".to_vec(),
            },
        ]));
    let replay = recording(vec![frame]);
    let group_scan = groups(&replay);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 0),
        &AtomicBool::new(false),
    )
    .unwrap();
    let faction = reply.sides.iter().find(|side| side.key == "f:9").unwrap();
    assert_eq!(
        group_scan
            .resolved_faction_name_at(
                faction.faction.as_ref().unwrap().parse().unwrap(),
                0,
                layers.faction_templates().as_deref().map_err(Clone::clone),
                &loc,
                "english",
            )
            .unwrap(),
        "Modded coalition"
    );
}

#[test]
fn unseen_air_tables_stay_unavailable_instead_of_becoming_zero_activity() {
    let mut frame = air(0, 200_000);
    *frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
        reason: "fixture missing scores".into(),
    };
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 17,
            owner: 9001,
            mission_region: Some(7000),
            ..Default::default()
        }]));
    let replay = recording(vec![frame]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 0),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!reply.available);
    assert!(reply.complete.is_empty());
    assert!(
        !moment_cell_sides(&reply, &reply.regions[0].runs[0].2)
            .unknown
            .is_empty()
    );
}

#[test]
fn a_side_dictionary_dates_renames_and_leadership_changes() {
    let mut next = air(1, 200_000);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *next.tables[TableId::Factions.index()]
    {
        rows[0].name = b"Renamed mod coalition".to_vec();
    }
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *next.tables[TableId::FactionMembers.index()]
    {
        for row in rows {
            if row.country == 44 {
                row.faction = 9;
            }
            if row.country == 9001 {
                row.faction = 10;
            }
        }
    }
    let replay = recording(vec![air(0, 200_000), next]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 1),
        &AtomicBool::new(false),
    )
    .unwrap();
    let variants: Vec<_> = reply
        .sides
        .iter()
        .filter(|side| side.key == "f:9")
        .collect();
    assert_eq!(variants.len(), 1);
    assert_eq!(
        (variants[0].name.as_str(), variants[0].leader.as_deref()),
        ("mod_pact", Some("GER"))
    );
    assert_eq!(
        (
            variants[0].reads[1].1.as_str(),
            variants[0].reads[1].2.as_deref()
        ),
        ("Renamed mod coalition", Some("FRA"))
    );
    let runs = &reply.regions[0].runs;
    assert_ne!(runs[0].2, runs[1].2);
    let mut request = params(AirView::Moment, 0, 1);
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("f:9".into());
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    let leaders: Vec<_> = reply.regions[0]
        .runs
        .iter()
        .map(|(_, _, cell)| {
            let AirValue::SideViewpoint { leader, .. } = cell.0.as_ref().unwrap() else {
                panic!("side viewpoint must identify dated leadership")
            };
            leader.as_deref()
        })
        .collect();
    assert_eq!(leaders, [Some("GER"), Some("FRA")]);
}

#[test]
fn timeline_dates_are_refused_only_when_their_json_exceeds_the_reply_budget() {
    let request = params(
        AirView::Timeline,
        0,
        (MAX_PRODUCT_REPLY_BYTES as i32) * DAY_HOURS,
    );
    let error = timeline(&DayCache::default(), &request).unwrap_err();
    assert!(error.to_string().contains("MAX_PRODUCT_REPLY_BYTES"));
    let reply = timeline(
        &DayCache::default(),
        &params(AirView::Timeline, 0, 5000 * DAY_HOURS),
    )
    .unwrap();
    assert_eq!(reply.days.len(), 5001);
    assert!(check_reply_size(&AirReply::Timeline(reply)).is_ok());
}

#[test]
fn selected_5000_countries_are_served_and_oversize_replies_are_never_truncated() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow};
    let mut frame = air(0, 200_000);
    let countries: Vec<_> = (0..5000)
        .map(|index| CountryRow {
            id: 100_000 + index,
            canonical_slot: index,
            tag: format!("Mod country {index}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let selected: Vec<_> = countries
        .iter()
        .map(|country| format!("c:{}", std::str::from_utf8(&country.tag).unwrap()))
        .collect();
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        countries
            .iter()
            .map(|country| CountryLinkRow {
                id: country.id,
                canonical_slot: country.canonical_slot,
            })
            .collect(),
    ));
    *frame.tables[TableId::RaidAirScores.index()] = TableView::Complete(TableRows::RaidAirScores(
        countries
            .iter()
            .map(|country| RaidAirScoreRow {
                country: country.id,
                region: 7000,
                friendly: 100_000,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
            })
            .collect(),
    ));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![]));
    let replay = recording(vec![frame]);
    let mut request = params(AirView::Moment, 0, 0);
    request.sides = Some(selected);
    assert!(AirSuperiority::validate(&request).is_ok());
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.sides.len(), 5000);
    assert_eq!(
        moment_cell_sides(&reply, &reply.regions[0].runs[0].2)
            .scores
            .len(),
        5000
    );
    let mut reply = AirReply::Moment(reply);
    assert!(check_reply_size(&reply).is_ok());
    let AirReply::Moment(moment) = &mut reply else {
        unreachable!()
    };
    moment.regions[0].name = "x".repeat(MAX_PRODUCT_REPLY_BYTES);
    assert!(
        check_reply_size(&reply)
            .unwrap_err()
            .to_string()
            .contains("MAX_PRODUCT_REPLY_BYTES")
    );
    let AirReply::Moment(moment) = reply else {
        unreachable!()
    };
    assert_eq!(moment.sides.len(), 5000);
    assert_eq!(
        moment_cell_sides(&moment, &moment.regions[0].runs[0].2)
            .scores
            .len(),
        5000
    );
}

#[test]
fn moment_windows_follow_recorded_blocks_and_cover_unrecorded_hours() {
    let first = recording(vec![air(0, 200_000), air(4999, 300_000)]);
    let added = continuation_records(&first, 1, vec![air(7000, 400_000)]);
    let replay = first.append(&[added]).unwrap();
    let windows = moment_windows(&replay, 1);
    assert_eq!(windows.len(), replay.index.len());
    assert_eq!(windows[0].first_hour, 0);
    assert_eq!(windows.last().unwrap().last_hour, 7000);
    assert!(
        windows
            .windows(2)
            .all(|pair| pair[0].last_hour + 1 == pair[1].first_hour)
    );
    assert!(
        windows
            .iter()
            .any(|window| window.last_hour - window.first_hour >= 168)
    );
}

#[test]
fn appended_day_index_extends_only_new_blocks_and_shares_unchanged_daily_snapshots() {
    use std::sync::Arc;
    let first = recording(vec![air(0, 200_000), air(23, 200_000)]);
    let added = continuation_records(
        &first,
        1,
        vec![air(24, 300_000), air(47, 300_000), air(48, 400_000)],
    );
    let next = first.append(&[added]).unwrap();
    assert_eq!(&next.index[..first.index.len()], &first.index);
    let cancelled = AtomicBool::new(false);
    let original = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let mut extended = original.clone();
    let old_count = original.built_hours().len();
    assert_eq!(old_count, 2);
    extend_days(&mut extended, &next, 1, &groups(&next), &cancelled).unwrap();
    assert_eq!(extended.built_hours().len(), old_count + 3);
    assert!(Arc::ptr_eq(original.days()[&0], extended.days()[&0]));
    assert_eq!(original.days().len(), 1);
    assert_eq!(extended.days().len(), 3);
    assert_eq!(extended.days()[&1].hour, 47);
    let last = Arc::clone(extended.days()[&2]);
    let count = extended.built_hours().len();
    extend_days(&mut extended, &next, 1, &groups(&next), &cancelled).unwrap();
    assert_eq!(extended.built_hours().len(), count);
    assert!(Arc::ptr_eq(&last, extended.days()[&2]));
}

#[test]
fn cancelled_scans_refuse_to_return_partial_air_views() {
    let replay = recording(vec![air(0, 200_000)]);
    let groups = groups(&replay);
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        scan_moment(
            &replay,
            1,
            &groups,
            &params(AirView::Moment, 0, 0),
            &cancelled
        )
        .err()
        .unwrap()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        scan_days(&replay, 1, &groups, None, &cancelled)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn absent_neutral_owners_use_a_count_without_expanding_the_side_dictionary() {
    let mut frames = vec![air(0, 200_000), air(1, 200_000)];
    for frame in &mut frames {
        if let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut *frame.tables[TableId::FactionMembers.index()]
        {
            rows.retain(|row| row.country != 55 && row.country != 66);
        }
        if let TableView::Complete(TableRows::RaidAirScores(rows)) =
            &mut *frame.tables[TableId::RaidAirScores.index()]
        {
            rows.retain(|row| row.country != 55 && row.country != 66);
        }
    }
    let replay = recording(frames);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 1),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.sides.len(), 2);
    assert_eq!(reply.excluded, [(0, 1, 2)]);
    let cell = moment_cell_sides(&reply, &reply.regions[0].runs[0].2);
    assert!(cell.unknown.is_empty());
    assert!(cell.no_air_object.is_empty());
    assert_eq!(cell.scores.len(), 2);
}

#[test]
fn product_load_inherits_verified_append_state_and_keeps_prior_source_independent() {
    use std::sync::Arc;
    let first = recording(vec![air(0, 200_000)]);
    let added = continuation_records(&first, 1, vec![air(24, 300_000)]);
    let next = first.append(&[added]).unwrap();
    let cancelled = AtomicBool::new(false);
    let make = |revision: &str, replay| {
        Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "air-load".into(),
                revision: revision.into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap()
    };
    let first_source = make("1", first);
    let context = LoadContext { content: None };
    let AirReply::Timeline(first_reply) = AirSuperiority::load(
        &first_source,
        &params(AirView::Timeline, 0, 0),
        &context,
        &cancelled,
    )
    .unwrap() else {
        unreachable!()
    };
    let next_source = make("2", next);
    next_source.inherit_append_states(&first_source);
    let AirReply::Timeline(next_reply) = AirSuperiority::load(
        &next_source,
        &params(AirView::Timeline, 0, 24),
        &context,
        &cancelled,
    )
    .unwrap() else {
        unreachable!()
    };
    assert_eq!(next_reply.days, [Some(0), Some(24)]);
    assert_eq!(first_reply.days, [Some(0)]);
    let first_state = first_source.state::<AirCache>();
    let next_state = next_source.state::<AirCache>();
    let first_state = first_state.lock().unwrap();
    let next_state = next_state.lock().unwrap();
    let first_entry = Arc::clone(&first_state.branches[0].1);
    let next_entry = Arc::clone(&next_state.branches[0].1);
    drop(first_state);
    drop(next_state);
    {
        let first_cache = first_entry.data.lock().unwrap();
        let next_cache = next_entry.data.lock().unwrap();
        assert!(Arc::ptr_eq(first_cache.days()[&0], next_cache.days()[&0]));
        assert!(Arc::ptr_eq(
            &first_cache.segments[0],
            &next_cache.segments[0]
        ));
        assert_eq!(first_cache.built_hours().len(), 1);
        assert_eq!(next_cache.built_hours().len(), 2);
    }
    let AirReply::Timeline(first_again) = AirSuperiority::load(
        &first_source,
        &params(AirView::Timeline, 0, 0),
        &context,
        &cancelled,
    )
    .unwrap() else {
        unreachable!()
    };
    assert_eq!(first_again.rows[0].cells, first_reply.rows[0].cells);
    // A rewritten recording whose block index differs (it starts an hour later); a
    // rewrite that compresses to the same block lengths is indistinguishable here.
    let changed_source = make("changed", recording(vec![air(1, 900_000)]));
    changed_source.inherit_append_states(&first_source);
    assert!(
        changed_source
            .state::<AirCache>()
            .lock()
            .unwrap()
            .branches
            .is_empty()
    );
}

#[test]
fn opaque_country_tags_equal_to_side_keys_keep_their_explicit_viewpoint_kind() {
    let mut frame = air(0, 200_003);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.id == 9001).unwrap().tag = b"f:9".to_vec();
    }
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        for row in rows {
            if row.country == 44 {
                row.faction = 9;
            }
            if row.country == 9001 {
                row.faction = 10;
            }
        }
    }
    let replay = recording(vec![frame]);
    let mut request = params(AirView::Moment, 0, 0);
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("f:9".into());
    request.viewpoint_country = Some(true);
    let country = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        country.regions[0].runs[0].2,
        AirCell(Some(AirValue::Viewpoint((
            "200003".into(),
            "100000".into()
        ))))
    );
    assert!(country.sides.is_empty());
    request.viewpoint_country = Some(false);
    let side = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &request,
        &AtomicBool::new(false),
    )
    .unwrap();
    let AirValue::SideViewpoint {
        view, side: index, ..
    } = side.regions[0].runs[0].2.0.as_ref().unwrap()
    else {
        panic!("side choice must keep its dated leader")
    };
    assert_eq!(view, &("100000".into(), "200003".into()));
    assert_eq!(side.sides[*index as usize].key, "f:9");
    assert_eq!(side.sides[*index as usize].leader.as_deref(), Some("FRA"));
}

#[test]
fn top_region_ranking_preserves_single_fixed5_unit_differences_above_f64_precision() {
    let smaller = 9_007_199_254_740_992_i64;
    let mut frame = air(23, smaller);
    if let TableView::Complete(TableRows::RaidAirScores(rows)) =
        &mut *frame.tables[TableId::RaidAirScores.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 44)
            .unwrap()
            .friendly = 0;
        rows.push(RaidAirScoreRow {
            country: 9001,
            region: 9000,
            friendly: smaller + 1,
            enemy: 0,
            impact: Default::default(),
            friendly_missions: 0,
            enemy_missions: 0,
            air_spotting_scale: Some(0),
        });
        rows.sort_by_key(|row| (row.country, row.region));
    }
    let replay = recording(vec![frame]);
    let cache = scan_days(&replay, 1, &groups(&replay), None, &AtomicBool::new(false)).unwrap();
    let mut request = params(AirView::Timeline, 0, 23);
    request.regions = None;
    request.top = Some(1);
    let reply = timeline(&cache, &request).unwrap();
    assert_eq!(reply.rows.len(), 1);
    assert_eq!(reply.rows[0].region, 9000);
    assert_eq!(
        cell_sides(&reply.rows[0].cells[0].1).scores[0].1,
        (smaller + 1).to_string()
    );
}

#[test]
fn theatre_read_dates_follow_daily_slot_reads_and_survive_off_slot_keyframes() {
    use replay::spool::Record;
    use replay_frame::frame::FrameEncoder;
    let first_read = (0..DAY_HOURS)
        .find(|&hour| replay_data::cadence::is_refresh(TableId::AirTheatres, hour))
        .expect("air theatres have a recorded daily refresh slot");
    let next_read = first_read + 2;
    let theatre_frame = |hour, name: &[u8]| {
        let mut frame = air(hour, 200_000);
        frame.metrics.table_ns.fill(0);
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 17,
                owner: 9001,
                mission_region: Some(7000),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirTheatres.index()] =
            TableView::Complete(TableRows::AirTheatres(vec![
                replay_data::rows::AirTheatreRow {
                    id: 700,
                    owner: 9001,
                    name: name.to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![
                replay_data::rows::AirTheatreGroupRow {
                    id: 701,
                    theatre: 700,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![
                replay_data::rows::AirTheatreMemberRow {
                    wing: 17,
                    group: 701,
                    ..Default::default()
                },
            ]));
        frame
    };
    let mut read = theatre_frame(first_read, b"First theatre read");
    read.metrics.table_ns[TableId::AirTheatres.index()] = 1;
    let carried = theatre_frame(first_read + 1, b"First theatre read");
    let mut changed = theatre_frame(next_read, b"Changed theatre read");
    changed.metrics.table_ns[TableId::AirTheatres.index()] = 1;
    let carried_again = theatre_frame(next_read + 1, b"Changed theatre read");
    assert!(!dashboard_text::metadata::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &carried.metrics,
        TableId::AirTheatres,
        carried.hour
    ));
    assert!(dashboard_text::metadata::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &changed.metrics,
        TableId::AirTheatres,
        changed.hour
    ));
    assert!(!dashboard_text::metadata::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &carried_again.metrics,
        TableId::AirTheatres,
        carried_again.hour
    ));
    let mut encoded = records(1, vec![read]);
    let mut encoder = FrameEncoder::new();
    // A wire keyframe carries complete daily tables without saying the native
    // reader refreshed them. Its zero daily timing must retain the prior date.
    encoded.push(Record::Frame(
        encoder.encode(carried.clone(), true).unwrap(),
    ));
    encoded.push(Record::Frame(encoder.encode(changed, false).unwrap()));
    encoded.push(Record::Frame(encoder.encode(carried_again, false).unwrap()));
    let replay = Replay::from_records(CAMPAIGN.into(), &[encoded]).unwrap();
    let cancelled = AtomicBool::new(false);
    let grouped = groups(&replay);
    let reply = scan_moment(
        &replay,
        1,
        &grouped,
        &params(AirView::Moment, first_read, next_read + 1),
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        reply.theatre_reads,
        [
            (first_read, first_read + 1, first_read),
            (next_read, next_read + 1, next_read)
        ]
    );
    let window = scan_moment(
        &replay,
        1,
        &grouped,
        &params(AirView::Moment, first_read + 1, first_read + 1),
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        window.theatre_reads,
        [(first_read + 1, first_read + 1, first_read)]
    );
    let cache = scan_days(&replay, 1, &grouped, None, &cancelled).unwrap();
    let timeline = timeline(
        &cache,
        &params(AirView::Timeline, first_read, next_read + 1),
    )
    .unwrap();
    assert!(
        timeline
            .theatre_reads
            .iter()
            .any(|(_, _, supplied)| *supplied == next_read)
    );
    // A recording opening with only that off-slot keyframe has no supplying
    // read date to recover, even though its theatre roster can be displayed.
    let carried_only = recording(vec![carried]);
    let reply = scan_moment(
        &carried_only,
        1,
        &groups(&carried_only),
        &params(AirView::Moment, first_read + 1, first_read + 1),
        &cancelled,
    )
    .unwrap();
    assert!(reply.theatre_reads.is_empty());
    let mut incomplete = theatre_frame(next_read, b"Incomplete theatre read");
    incomplete.metrics.table_ns[TableId::AirTheatres.index()] = 1;
    *incomplete.tables[TableId::AirTheatreMembers.index()] = TableView::Unavailable {
        reason: "fixture missing join".into(),
    };
    let restored = theatre_frame(next_read + 1, b"Incomplete theatre read");
    let broken = recording(vec![
        theatre_frame(first_read, b"First theatre read"),
        theatre_frame(first_read + 1, b"First theatre read"),
        incomplete,
        restored,
    ]);
    let reply = scan_moment(
        &broken,
        1,
        &groups(&broken),
        &params(AirView::Moment, first_read, next_read + 1),
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        reply.theatre_reads,
        [(first_read, first_read + 1, first_read)]
    );
}

#[test]
fn air_superiority_retains_two_views_and_one_replaced_partial_without_waiting_on_scans() {
    use std::sync::Arc;
    let replay = recording(vec![air(0, 1), air(23, 2)]);
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "retention".into(),
            revision: "1".into(),
        },
        "retention.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let groups = Arc::new(groups(source.replay_ref()));
    let entry = cached_branch(&source, 1, None);
    for last in [1, 7, 12] {
        let cache = extend_branch(
            &entry,
            source.replay_ref(),
            1,
            &groups,
            None,
            Some(last),
            &cancelled,
        )
        .unwrap();
        assert_eq!(cache.partial.as_ref().map(|(at, _)| *at), Some(last));
        assert_eq!(cache.segments.len(), 1);
    }
    for tag in ["one", "two", "three"] {
        cached_branch(&source, 1, Some((tag, true)));
    }
    let registry = source.append_state::<AirCache>();
    assert_eq!(registry.lock().unwrap().viewpoints.len(), 2);
    let _cold_scan = entry.gate.lock().unwrap();
    let inherited = registry.lock().unwrap().clone();
    assert!(Arc::ptr_eq(
        &inherited.branches[0].1.data.lock().unwrap().segments[0],
        &entry.data.lock().unwrap().segments[0]
    ));
    assert!(inherited.branches[0].1.gate.try_lock().is_ok());
}

#[test]
fn air_superiority_window_choices_do_not_create_any_scan_state() {
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "windows".into(),
            revision: "1".into(),
        },
        "windows.battleplan".into(),
        std::sync::Arc::new(recording(vec![air(0, 1)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut params = params(AirView::Choices, 0, 0);
    params.windows_only = Some(true);
    let AirReply::Choices(reply) = AirSuperiority::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap() else {
        unreachable!()
    };
    assert!(!reply.windows.is_empty());
    assert!(reply.sides.is_empty());
    assert!(
        source
            .state::<AirCache>()
            .lock()
            .unwrap()
            .branches
            .is_empty()
    );
}

#[test]
fn air_superiority_absent_side_has_a_named_reason_and_unknown_lone_scores_do_not_erase_known_sides()
{
    let mut frame = air(0, 200_000);
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        rows.retain(|row| row.country != 55);
    }
    let complete = frame.clone();
    if let TableView::Complete(TableRows::RaidAirScores(mut rows)) =
        (*frame.tables[TableId::RaidAirScores.index()]).clone()
    {
        rows.retain(|row| row.country != 55);
        *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated {
            missing: 1,
            rows: TableRows::RaidAirScores(rows),
        };
    }
    // A partial keyframe is unavailable; this must be an ordinary hourly read.
    frame.hour = 1;
    let replay = recording(vec![complete, frame]);
    let groups = groups(&replay);
    let cancelled = AtomicBool::new(false);
    let mut request = params(AirView::Moment, 1, 1);
    let reply = scan_moment(&replay, 1, &groups, &request, &cancelled).unwrap();
    let cell = moment_cell_sides(&reply, &reply.regions[0].runs[0].2);
    assert_eq!(cell.scores.len(), 2);
    assert_eq!(cell.unknown.len(), 1);
    assert_eq!(reply.sides[cell.unknown[0] as usize].key, "c:CTL");
    request.mode = Some(AirMode::Viewpoint);
    request.viewpoint = Some("f:absent".into());
    let reply = scan_moment(&replay, 1, &groups, &request, &cancelled).unwrap();
    assert!(
        matches!(&reply.regions[0].runs[0].2.0, Some(AirValue::Reason(reason)) if reason == "side-not-present")
    );
}

#[test]
fn air_superiority_new_viewpoints_share_auxiliary_reads_side_catalogue_and_historical_anchors() {
    let replay = recording(vec![air(0, 1), air(23, 2)]);
    let groups = groups(&replay);
    let cancelled = AtomicBool::new(false);
    let base = scan_days(&replay, 1, &groups, None, &cancelled).unwrap();
    let mut view = DayCache {
        viewpoint: Some("GER".into()),
        viewpoint_country: true,
        ..Default::default()
    };
    extend_days_shared(&mut view, &replay, 1, &groups, Some(&base), &cancelled).unwrap();
    assert!(Arc::ptr_eq(&view.anchors, &base.anchors));
    assert!(Arc::ptr_eq(&view.sides, &base.sides));
    assert!(Arc::ptr_eq(
        &view.segments[0].auxiliary[0].2,
        &base.segments[0].auxiliary[0].2
    ));
}

#[test]
fn air_superiority_within_day_appends_replace_the_prior_partial_snapshot() {
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![air(0, 1)]);
    let mut cache = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let original = cache.clone();
    let mut replay = first;
    for hour in 1..24 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![air(hour, i64::from(hour))],
            )])
            .unwrap();
        extend_days(&mut cache, &replay, 1, &groups(&replay), &cancelled).unwrap();
        assert_eq!(
            cache
                .segments
                .iter()
                .map(|segment| segment.days.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(cache.days()[&0].hour, hour);
    }
    assert_eq!(original.days()[&0].hour, 0);
}

#[test]
fn air_superiority_coalition_key_survives_departure_of_its_first_current_member() {
    let replay = recording(vec![air(0, 1), air(1, 1)]);
    let mut groups = groups(&replay);
    groups.factions.clear();
    groups.wars = ["GER", "CTL", "MOD"]
        .map(|tag| country_groups::WarPairRun {
            first: tag.into(),
            second: "FRA".into(),
            first_hour: 0,
            last_hour: 1,
        })
        .into();
    let before = ["GER", "CTL", "MOD", "FRA"].map(String::from).into();
    let after = ["GER", "MOD", "FRA"].map(String::from).into();
    let first = assignment::assign_sides(&groups, 0, &before).unwrap();
    let next = assignment::assign_sides(&groups, 1, &after).unwrap();
    assert_eq!(
        first
            .iter()
            .find(|side| side.key.starts_with("w:"))
            .unwrap()
            .key,
        "w:9:55"
    );
    assert_eq!(
        next.iter()
            .find(|side| side.key.starts_with("w:"))
            .unwrap()
            .key,
        "w:9:55"
    );
    let last = ["MOD", "FRA"].map(String::from).into();
    let remnant = assignment::assign_sides(&groups, 1, &last).unwrap();
    assert_eq!(
        remnant
            .iter()
            .find(|side| side.members == ["MOD"])
            .unwrap()
            .key,
        "w:9:55"
    );
}

#[test]
fn air_superiority_region_names_reuse_the_source_content_key_and_invalidate_on_language_change() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "settings.txt",
        "language=\"l_english\"",
    )]);
    let content = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.join("missing-game"),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "region-names".into(),
            revision: "1".into(),
        },
        "names.battleplan".into(),
        Arc::new(recording(vec![air(0, 1)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let key = content::localisation_key(&content).unwrap();
    let names = Arc::new([(7000, "Cached mod region".into())].into());
    source.state::<RegionNames>().lock().unwrap().entry = Some((
        key,
        Arc::clone(&names),
        Arc::new(Default::default()),
        vec![],
    ));
    let context = LoadContext {
        content: Some(content),
    };
    let request = params(AirView::Moment, 0, 0);
    for _ in 0..2 {
        let AirReply::Moment(reply) =
            AirSuperiority::load(&source, &request, &context, &AtomicBool::new(false)).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(reply.regions[0].name, "Cached mod region");
    }
    assert!(Arc::ptr_eq(
        &names,
        &source
            .state::<RegionNames>()
            .lock()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .1
    ));
    let next = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "region-names-next".into(),
            revision: "2".into(),
        },
        "names.battleplan".into(),
        source.replay(),
        &AtomicBool::new(false),
    )
    .unwrap();
    next.inherit_append_states(&source);
    assert!(Arc::ptr_eq(
        &names,
        &next
            .state::<RegionNames>()
            .lock()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .1
    ));
    std::fs::write(folder.0.join("settings.txt"), "language=\"l_french\"").unwrap();
    let AirReply::Moment(reply) =
        AirSuperiority::load(&source, &request, &context, &AtomicBool::new(false)).unwrap()
    else {
        unreachable!()
    };
    assert_ne!(reply.regions[0].name, "Cached mod region");
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Air region names show ids:"))
    );
}

#[test]
fn air_superiority_alliance_split_resolves_faction_and_country_collisions_in_every_scan() {
    let frame = |hour, split, factions| {
        let mut frame = air(hour, 200_000);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for (id, slot, tag) in [(77, 12, "X"), (88, 13, "Y")] {
                rows.push(replay_data::rows::CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.as_bytes().into(),
                    ..Default::default()
                });
            }
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.extend([(77, 12), (88, 13)].map(|(id, canonical_slot)| {
                replay_data::rows::CountryLinkRow { id, canonical_slot }
            }));
            rows.sort_by_key(|row| row.id);
        }
        *frame.tables[TableId::Factions.index()] =
            TableView::Complete(TableRows::Factions(if factions {
                [9, 10, 11]
                    .map(|id| FactionRow {
                        id,
                        name: format!("Pact {id}").into_bytes(),
                        ..Default::default()
                    })
                    .into()
            } else {
                vec![]
            }));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(if factions {
                [(44, 10), (55, 9), (9001, 11)]
                    .map(|(country, faction)| FactionMemberRow {
                        country,
                        faction,
                        ordinal: 0,
                    })
                    .into()
            } else {
                vec![]
            }));
        let pairs = if split {
            vec![(44, 77), (9001, 77), (55, 88), (66, 88)]
        } else {
            vec![(55, 77), (44, 77), (9001, 77)]
        };
        let mut wars: Vec<_> = pairs
            .into_iter()
            .map(
                |(first_country, second_country)| replay_data::rows::WarRelationRow {
                    first_country,
                    second_country,
                    ..Default::default()
                },
            )
            .collect();
        wars.sort_by_key(|row| (row.first_country, row.second_country));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(wars));
        if let TableView::Complete(TableRows::RaidAirScores(rows)) =
            &mut *frame.tables[TableId::RaidAirScores.index()]
        {
            rows.iter_mut()
                .find(|row| row.country == 55)
                .unwrap()
                .region = 7000;
            rows.iter_mut()
                .find(|row| row.country == 55)
                .unwrap()
                .friendly = 75_000;
            rows.sort_by_key(|row| (row.country, row.region));
        }
        frame
    };
    let cancelled = AtomicBool::new(false);
    for factions in [true, false] {
        let first = recording(vec![frame(23, false, factions)]);
        let mut incremental = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
        let next = first
            .append(&[continuation_records(
                &first,
                1,
                vec![frame(47, true, factions)],
            )])
            .unwrap();
        let next_groups = groups(&next);
        extend_days(&mut incremental, &next, 1, &next_groups, &cancelled).unwrap();
        let cold = scan_days(&next, 1, &next_groups, None, &cancelled).unwrap();
        let request = params(AirView::Timeline, 0, 47);
        let cold_reply = timeline(&cold, &request).unwrap();
        assert_eq!(
            serde_json::to_value(timeline(&incremental, &request).unwrap()).unwrap(),
            serde_json::to_value(&cold_reply).unwrap()
        );
        let moment = scan_moment(
            &next,
            1,
            &next_groups,
            &params(AirView::Moment, 47, 47),
            &cancelled,
        )
        .unwrap();
        let keys: BTreeSet<_> = moment.sides.iter().map(|side| side.key.as_str()).collect();
        assert_eq!(
            keys,
            if factions {
                BTreeSet::from(["f:9", "f:10"])
            } else {
                BTreeSet::from(["w:9:55", "w:8:44"])
            }
        );
        let cell = cell_sides(
            &moment
                .regions
                .iter()
                .find(|region| region.id == 7000)
                .unwrap()
                .runs[0]
                .2,
        );
        assert!(cell.unknown.is_empty());
        assert_eq!(cell.scores.len(), 2);
    }
}

#[test]
fn air_superiority_unused_anchor_additions_keep_published_segments() {
    let first = recording(vec![air(23, 1)]);
    let cancelled = AtomicBool::new(false);
    let mut cache = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let segment = Arc::clone(&cache.segments[0]);
    let mut appended = air(47, 2);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *appended.tables[TableId::Factions.index()]
    {
        rows.push(FactionRow {
            id: 99,
            name: b"Unused new faction".into(),
            ..Default::default()
        });
    }
    let next = first
        .append(&[continuation_records(&first, 1, vec![appended])])
        .unwrap();
    let next_groups = groups(&next);
    assert!(cache.anchors.as_ref() != &assignment::historical_anchors(&next_groups));
    extend_days(&mut cache, &next, 1, &next_groups, &cancelled).unwrap();
    assert!(Arc::ptr_eq(&cache.segments[0], &segment));
    assert_eq!(cache.built_hours(), [23, 47]);
}

#[test]
fn air_superiority_moment_reuses_revision_anchors_without_building_daily_segments() {
    let cancelled = AtomicBool::new(false);
    let replay = recording(vec![air(0, 1), air(1, 2)]);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "moment-anchors".into(),
            revision: "1".into(),
        },
        "moment.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    let groups = country_groups::cached_scan(&source, 1, &cancelled).unwrap();
    let entry = cached_branch(&source, 1, None);
    let anchors = branch_anchors(&entry, &groups);
    for hour in 0..=1 {
        AirSuperiority::load(
            &source,
            &params(AirView::Moment, hour, hour),
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert!(Arc::ptr_eq(&anchors, &branch_anchors(&entry, &groups)));
    }
    assert!(entry.data.lock().unwrap().segments.is_empty());
}

#[test]
fn air_superiority_moment_wire_is_lossless_for_5000_unknown_countries() {
    let replay = recording(vec![air(0, 200_000)]);
    let mut reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 0),
        &AtomicBool::new(false),
    )
    .unwrap();
    reply.sides = (0..5000)
        .map(|index| {
            let tag = format!("mod country [{index}]");
            AirSide {
                key: format!("c:{tag}"),
                faction: None,
                colour_faction: None,
                tag: Some(tag.clone()),
                name: tag.clone(),
                leader: Some(tag.clone()),
                co_belligerents: 0,
                factions: vec![],
                reads: vec![(
                    0,
                    tag.clone(),
                    Some(tag.clone()),
                    Some(tag),
                    0,
                    vec![],
                    None,
                )],
            }
        })
        .collect();
    let unknown: Vec<_> = (0..5000).collect();
    let cell = AirCell(Some(AirValue::Sides(AirSideCell {
        scores: vec![],
        viewpoints: vec![],
        unknown: unknown.clone(),
        unknown_set: None,
        no_air_object: vec![],
        filtered: false,
        excluded: 0,
    })));
    reply.regions = [7000, 7001]
        .map(|id| AirMomentRegion {
            id,
            name: format!("Region {id}"),
            active: vec![],
            runs: vec![(0, 0, cell.clone())],
            wings: vec![],
        })
        .into();
    reply.unknown_sets = compact_unknowns(&mut reply.regions, &mut reply.fallback);
    assert_eq!(
        reply.unknown_sets.as_slice(),
        std::slice::from_ref(&unknown)
    );
    let json = serde_json::to_vec(&reply).unwrap();
    assert!(
        json.len() < 200_000,
        "the exact synthetic moment needs no repeated country properties or unknown lists"
    );
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(
        value["sides"][4999],
        serde_json::json!(["mod country [4999]", 0])
    );
    assert!(value["regions"][0]["runs"][0][2].get("unknown").is_none());
    assert_eq!(value["regions"][1]["runs"][0][2]["unknownSet"], 0);
    let decoded: AirMomentReply = serde_json::from_slice(&json).unwrap();
    assert_eq!(decoded.sides, reply.sides);
    for region in &decoded.regions {
        assert_eq!(
            moment_cell_sides(&decoded, &region.runs[0].2),
            cell_sides(&cell)
        );
    }
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), json);
}

#[test]
fn air_superiority_moment_country_compaction_preserves_nontrivial_side_reads() {
    let replay = recording(vec![air(0, 1)]);
    let mut reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 0),
        &AtomicBool::new(false),
    )
    .unwrap();
    let base = AirSide {
        key: "c:mod tag".into(),
        faction: None,
        colour_faction: None,
        tag: Some("mod tag".into()),
        name: "mod tag".into(),
        leader: Some("mod tag".into()),
        co_belligerents: 0,
        factions: vec![],
        reads: vec![(
            0,
            "mod tag".into(),
            Some("mod tag".into()),
            Some("mod tag".into()),
            0,
            vec![],
            None,
        )],
    };
    for change in 0..4 {
        let mut side = base.clone();
        match change {
            0 => side.key = "w:historical member".into(),
            1 => side.co_belligerents = 2,
            2 => side.reads[0].1 = "Dated name".into(),
            _ => side
                .reads
                .push((1, "Later name".into(), None, None, 0, vec![], None)),
        }
        reply.sides = vec![side.clone()];
        let value = serde_json::to_value(&reply).unwrap();
        assert!(value["sides"][0].is_object());
        let decoded: AirMomentReply = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.sides, reply.sides);
    }
}

#[test]
fn air_superiority_all_request_modes_reuse_one_country_group_scan() {
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "group-reuse".into(),
            revision: "1".into(),
        },
        "group-reuse.battleplan".into(),
        Arc::new(recording(vec![air(0, 1)])),
        &cancelled,
    )
    .unwrap();
    let original = country_groups::cached_scan(&source, 1, &cancelled).unwrap();
    for view in [AirView::Moment, AirView::Choices, AirView::Timeline] {
        for country in [None, Some("GER"), Some("FRA"), Some("CTL")] {
            let mut request = params(view, 0, 0);
            if let Some(country) = country {
                request.mode = Some(AirMode::Viewpoint);
                request.viewpoint = Some(country.into());
                request.viewpoint_country = Some(true);
            }
            AirSuperiority::load(
                &source,
                &request,
                &LoadContext { content: None },
                &cancelled,
            )
            .unwrap();
            assert!(Arc::ptr_eq(
                &original,
                &country_groups::cached_scan(&source, 1, &cancelled).unwrap()
            ));
        }
    }
}

#[test]
fn air_superiority_faction_names_follow_native_operands_and_dictionary_reads() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/factions/templates/a.txt",
            "demo={name={localization_key=PACT ID=token:member}}",
        ),
        (
            "localisation/a_l_english.yml",
            "\u{feff}l_english:\n PACT:0 \"Pact $ID$\"\n mod_pact:0 \"Translated key\"\n",
        ),
        (
            "map/strategicregions/a.txt",
            "strategic_region={id=7000 name=region provinces={1}}",
        ),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let frames = (0..4)
        .map(|hour| {
            let mut frame = air(hour, 200_000);
            if let TableView::Complete(TableRows::Factions(rows)) =
                &mut *frame.tables[TableId::Factions.index()]
            {
                rows[0].name = match hour {
                    0 | 1 => vec![],
                    2 => b"mod_pact".to_vec(),
                    _ => vec![0xff],
                };
                rows[0].template_token = Some(17);
            }
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    replay_data::rows::LexicalTokenRow {
                        token: 17,
                        literal: b"demo".to_vec(),
                    },
                    replay_data::rows::LexicalTokenRow {
                        token: if hour == 0 { 31 } else { 47 },
                        literal: b"member".to_vec(),
                    },
                ]));
            frame
        })
        .collect();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "native-air-names".into(),
            revision: "1".into(),
        },
        "native-air-names.battleplan".into(),
        Arc::new(recording(frames)),
        &AtomicBool::new(false),
    )
    .unwrap();
    source.set_test_content(
        options.options.clone(),
        Arc::new(installed_content_core::content::ResolvedContent {
            layers,
            stats: Default::default(),
        }),
    );
    let AirReply::Moment(reply) = AirSuperiority::load(
        &source,
        &params(AirView::Moment, 0, 3),
        &LoadContext {
            content: Some(options),
        },
        &AtomicBool::new(false),
    )
    .unwrap() else {
        unreachable!()
    };
    let side = reply.sides.iter().find(|side| side.key == "f:9").unwrap();
    assert_eq!(
        side.reads
            .iter()
            .map(|read| (read.0, read.1.as_str()))
            .collect::<Vec<_>>(),
        [
            (0, "Pact 31"),
            (1, "Pact 47"),
            (2, "mod_pact"),
            (3, "Faction 9")
        ]
    );
    assert!(reply.warnings.iter().any(|reason| reason.contains("UTF-8")));
}

#[test]
fn air_superiority_region_names_preserve_empty_results_and_distinguish_missing_keys() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "map/strategicregions/empty.txt",
            "strategic_region={ id=7000 name=EMPTY }",
        ),
        (
            "map/strategicregions/default.txt",
            "strategic_region={ id=7001 }",
        ),
        (
            "map/strategicregions/absent.txt",
            "strategic_region={ id=7002 name=ABSENT }",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n EMPTY:0 \"\"\n",
        ),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "air-region-names".into(),
            revision: "1".into(),
        },
        "air-region-names.battleplan".into(),
        Arc::new(recording(vec![air(0, 200_000)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    source.set_test_content(
        options.options.clone(),
        Arc::new(installed_content_core::content::ResolvedContent {
            layers: installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap(),
            stats: Default::default(),
        }),
    );
    let mut request = params(AirView::Choices, 0, 0);
    request.regions = None;
    let AirReply::Choices(reply) = AirSuperiority::load(
        &source,
        &request,
        &LoadContext {
            content: Some(options),
        },
        &AtomicBool::new(false),
    )
    .unwrap() else {
        unreachable!()
    };
    assert_eq!(
        reply
            .regions
            .iter()
            .map(|region| (region.id, region.name.as_str()))
            .collect::<Vec<_>>(),
        [(7000, ""), (7001, ""), (7002, "Region 7002")]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Region 7002"))
    );
}

#[test]
fn air_superiority_theatre_names_preserve_raw_bytes_and_guard_the_recording_language() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "map/strategicregions/a.txt",
            "strategic_region={id=7000 name=region provinces={1}}",
        ),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    let mut frame = air(0, 200_000);
    *frame.tables[TableId::AirTheatres.index()] = TableView::Complete(TableRows::AirTheatres(
        [b"Synthetic theatre".to_vec(), vec![0xff], vec![]]
            .into_iter()
            .enumerate()
            .map(|(index, name)| replay_data::rows::AirTheatreRow {
                id: 700 + index as u64,
                owner: 9001,
                name,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![]));
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "air-theatre-names".into(),
            revision: "1".into(),
        },
        "air-theatre-names.battleplan".into(),
        Arc::new(recording(vec![frame])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    source.set_test_content(
        options.options.clone(),
        Arc::new(installed_content_core::content::ResolvedContent {
            layers,
            stats: Default::default(),
        }),
    );
    let context = LoadContext {
        content: Some(options),
    };
    let load = || {
        let AirReply::Choices(reply) = AirSuperiority::load(
            &source,
            &params(AirView::Choices, 0, 0),
            &context,
            &AtomicBool::new(false),
        )
        .unwrap() else {
            unreachable!()
        };
        reply
    };
    let english = load();
    assert_eq!(english.theatres[0].name, "Synthetic theatre");
    assert_eq!(
        english.theatres[1].name,
        "Air theatre 701 (name not recorded)"
    );
    assert!(
        english
            .warnings
            .iter()
            .any(|reason| reason.contains("Air theatre name is not UTF-8"))
    );
    assert!(
        english
            .warnings
            .iter()
            .any(|reason| reason.contains("Air theatre name is empty"))
    );
    std::fs::write(folder.0.join("settings.txt"), "language=\"l_french\"").unwrap();
    let french = load();
    assert_eq!(
        french.theatres[0].name,
        "Air theatre 700 (name not recorded)"
    );
    assert!(
        french
            .warnings
            .iter()
            .any(|reason| reason.contains("Air theatre name is text the game stored in english"))
    );
}

#[test]
fn air_superiority_native_coalition_survives_rename_without_inheriting_reused_tags() {
    let frame = |hour| {
        let mut frame = air(hour, 200_000);
        *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![]));
        if hour > 0 {
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                panic!()
            };
            rows.iter_mut().find(|row| row.id == 9001).unwrap().tag = b"Y".to_vec();
            rows.iter_mut().find(|row| row.id == 55).unwrap().tag = b"GER".to_vec();
            if hour == 2 {
                rows.iter_mut().find(|row| row.id == 9001).unwrap().id = 9002;
                rows.sort_by_key(|row| row.id);
                let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                else {
                    panic!()
                };
                rows.push(replay_data::rows::CountryLinkRow {
                    id: 9002,
                    canonical_slot: 7,
                });
                rows.sort_by_key(|row| row.id);
            }
        }
        let pairs = if hour == 0 {
            vec![(9001, 44), (66, 44)]
        } else if hour == 1 {
            vec![(9001, 44), (66, 44), (55, 66)]
        } else {
            vec![(9001, 44), (55, 66)]
        };
        let mut wars: Vec<_> = pairs
            .into_iter()
            .map(
                |(first_country, second_country)| replay_data::rows::WarRelationRow {
                    first_country,
                    second_country,
                    ..Default::default()
                },
            )
            .collect();
        wars.sort_by_key(|row| (row.first_country, row.second_country));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(wars));
        frame
    };
    let cancelled = AtomicBool::new(false);
    let first = recording(vec![frame(0)]);
    let mut incremental = scan_days(&first, 1, &groups(&first), None, &cancelled).unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, vec![frame(1), frame(2)])])
        .unwrap();
    let mut groups = groups(&replay);
    let anchors = assignment::historical_anchors(&groups);
    assert_eq!(anchors.country_reads["7:9001"], ("GER".into(), 0));
    assert_ne!(anchors.countries.get("9:55"), Some(&"7:9001".into()));
    let present = ["Y", "GER", "MOD", "FRA"].map(String::from).into();
    let sides = assignment::assign_sides(&groups, 1, &present).unwrap();
    let original = sides
        .iter()
        .find(|side| side.members.contains(&"Y".into()))
        .unwrap();
    assert_eq!(original.key, "w:7:9001");
    assert!(!original.members.contains(&"GER".into()));
    let replaced = assignment::assign_sides(&groups, 2, &present).unwrap();
    assert!(
        replaced
            .iter()
            .filter(|side| side.members.contains(&"Y".into()))
            .all(|side| side.key != "w:7:9001")
    );
    extend_days(&mut incremental, &replay, 1, &groups, &cancelled).unwrap();
    let cold = scan_days(&replay, 1, &groups, None, &cancelled).unwrap();
    assert_eq!(
        serde_json::to_value(timeline(&incremental, &params(AirView::Timeline, 0, 2)).unwrap())
            .unwrap(),
        serde_json::to_value(timeline(&cold, &params(AirView::Timeline, 0, 2)).unwrap()).unwrap()
    );
    groups.identities.remove("Y");
    assert!(assignment::assign_sides(&groups, 1, &present).is_none());
}

#[test]
fn air_superiority_cached_rosters_split_native_replacements_with_unchanged_tags_and_wars() {
    let frame = |hour| {
        let mut frame = air(hour, 200_000);
        *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![]));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![
                replay_data::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 66,
                    ..Default::default()
                },
                replay_data::rows::WarRelationRow {
                    first_country: 44,
                    second_country: 9001,
                    ..Default::default()
                },
            ]));
        if hour == 1 {
            let replacements = {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                else {
                    panic!()
                };
                let mut replacements = Vec::new();
                for row in rows.iter_mut().filter(|row| row.id == 66 || row.id == 9001) {
                    row.id += 1;
                    replacements.push(replay_data::rows::CountryLinkRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                    });
                }
                rows.sort_by_key(|row| row.id);
                replacements
            };
            let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            else {
                panic!()
            };
            rows.extend(replacements);
            rows.sort_by_key(|row| row.id);
        }
        frame
    };
    let replay = recording(vec![frame(0), frame(1)]);
    let reply = scan_moment(
        &replay,
        1,
        &groups(&replay),
        &params(AirView::Moment, 0, 1),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.sides.iter().any(|side| side.key == "w:7:9001"));
    assert!(reply.sides.iter().any(|side| side.key == "w:7:9002"));
}
