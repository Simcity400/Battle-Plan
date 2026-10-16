use super::*;
use dashboard_base::{
    source::SourceKey,
    test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag, source},
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, ShipCounterInputs, ShipModelRow, ShipRow,
        TableId, TableRows, TaskForceRow, tables,
    },
};

fn fleet_frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(101, 7, b"MOD_A"),
        country(102, 8, b"MOD_B"),
        country(103, 9, b"MOD_EMPTY"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(12, 8),
            link(13, 9),
            link(101, 7),
            link(102, 8),
            link(103, 9),
        ]));
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(vec![
        TaskForceRow {
            id: 21,
            roster_owner: 11,
            logical_owner: 12,
            expeditionary_owner: 11,
            ..Default::default()
        },
        TaskForceRow {
            id: 22,
            roster_owner: 12,
            logical_owner: 11,
            expeditionary_owner: 12,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![
        ShipRow {
            id: 31,
            task_force: 21,
            class_id: 41,
            ..Default::default()
        },
        ShipRow {
            id: 32,
            task_force: 21,
            class_id: 41,
            ..Default::default()
        },
        ShipRow {
            id: 33,
            task_force: 22,
            class_id: 42,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 41,
                literal: b"mod_escort".to_vec(),
            },
            LexicalTokenRow {
                token: 42,
                literal: b"mod_carrier".to_vec(),
            },
        ]));
    frame
}

fn recorded_source(frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    Source::from_replay(
        SourceKey {
            source_id: "navy-fixture".into(),
            revision: "1".into(),
        },
        "synthetic-navy.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn params() -> NavyParams {
    NavyParams {
        branch_id: "1".into(),
        tags: vec!["MOD_A".into()],
        top: None,
        ownership: NavyOwnership::Roster,
        grouping: NavyGrouping::Class,
        measure: NavyMeasure::Ships,
    }
}

#[test]
fn navy_scan_proves_roster_and_logical_and_preserves_zero_unknown_gaps_and_identity() {
    let same = fleet_frame(1);
    let mut unread = fleet_frame(2);
    *unread.tables[TableId::Ships.index()] = TableView::Unavailable {
        reason: "missing census".into(),
    };
    let mut host_unknown = fleet_frame(3);
    let TableView::Complete(TableRows::TaskForces(forces)) =
        &mut *host_unknown.tables[TableId::TaskForces.index()]
    else {
        unreachable!()
    };
    forces[0].logical_owner = 999;
    let mut replaced = fleet_frame(5);
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *replaced.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries[0].id = 201;
    countries.sort_by_key(|row| row.id);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *replaced.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.retain(|row| row.id != 101);
    links.push(CountryLinkRow {
        id: 201,
        canonical_slot: 7,
    });
    links.sort_by_key(|row| row.id);
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                fleet_frame(0),
                same,
                unread,
                host_unknown,
                replaced,
                fleet_frame(6),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let roster = &scanned.series[NavyOwnership::Roster.index()];
    let host = &scanned.series[NavyOwnership::Logical.index()];
    assert_eq!(roster["MOD_A"][0].total, Some(2));
    assert_eq!(host["MOD_A"][0].total, Some(1));
    assert_eq!(roster["MOD_B"][0].total, Some(1));
    assert_eq!(host["MOD_B"][0].total, Some(2));
    assert_eq!(roster["MOD_EMPTY"][0].total, Some(0));
    assert_eq!(
        (roster["MOD_A"][0].first_hour, roster["MOD_A"][0].last_hour),
        (0, 1)
    );
    assert_eq!(roster["MOD_A"][1].total, None);
    assert_eq!(host["MOD_A"][1].total, None);
    assert_eq!(roster["MOD_A"][2].total, Some(2));
    assert_eq!(host["MOD_A"][1].last_hour, 3);
    assert!(roster["MOD_A"][2].starts_segment);
    assert!(roster["MOD_A"][3].starts_segment);
    assert!(roster["MOD_A"][4].starts_segment);
    assert_eq!(
        scanned.segments["MOD_A"],
        [
            HourSpan {
                first_hour: 0,
                last_hour: 3
            },
            HourSpan {
                first_hour: 5,
                last_hour: 5
            },
            HourSpan {
                first_hour: 6,
                last_hour: 6
            }
        ]
    );
    assert_eq!(scanned.unattributed, [0, 2]);
    assert_eq!(scanned.unread, 1);
}

#[test]
fn navy_run_visitor_reuses_scan_and_keeps_zero_unknown_gap_and_replacement() {
    let mut unread = fleet_frame(1);
    *unread.tables[TableId::Ships.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let mut replacement = fleet_frame(4);
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *replacement.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries[0].id = 201;
    countries.sort_by_key(|row| row.id);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *replacement.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.retain(|row| row.id != 101);
    links.push(CountryLinkRow {
        id: 201,
        canonical_slot: 7,
    });
    links.sort_by_key(|row| row.id);
    let mut absent = replacement.clone();
    absent.hour = 5;
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *absent.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.id != 102);
    *absent.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![]));
    let source = recorded_source(vec![
        fleet_frame(0),
        unread,
        fleet_frame(3),
        replacement,
        absent,
    ]);
    let output = NavySize::load(
        &source,
        &params(),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.series[0].runs[0].total, Some(2));
    let tags = ["MOD_A".into(), "MOD_EMPTY".into(), "unobserved".into()];
    let mut seen = BTreeMap::<String, Vec<_>>::new();
    visit_ship_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(false),
        |tag, first, last, starts, value| {
            seen.entry(tag.to_owned())
                .or_default()
                .push((first, last, starts, value));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        seen["MOD_A"],
        [
            (0, 0, true, Some(2)),
            (1, 1, true, None),
            (3, 3, true, Some(2)),
            (4, 4, true, Some(2)),
            (5, 5, false, Some(0)),
        ]
    );
    assert_eq!(
        seen["MOD_EMPTY"],
        [
            (0, 0, true, Some(0)),
            (1, 1, true, None),
            (3, 5, true, Some(0)),
        ]
    );
    assert!(!seen.contains_key("MOD_B"));
    assert!(!seen.contains_key("unobserved"));
    assert_eq!(scans_run(&source), 1);

    let mut absent_runs = Vec::new();
    visit_ship_runs(
        &source,
        1,
        &["MOD_B".into()],
        &AtomicBool::new(false),
        |_, first, last, _, value| {
            absent_runs.push((first, last, value));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        absent_runs,
        [(0, 0, Some(1)), (1, 1, None), (3, 4, Some(1))]
    );
    let error = visit_ship_runs(
        &source,
        999,
        &tags,
        &AtomicBool::new(false),
        |_, _, _, _, _| panic!("unknown branch must refuse before visiting"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let error = visit_ship_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(true),
        |_, _, _, _, _| panic!("cancelled visitor must not call back"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(scans_run(&source), 1);

    let cancelled = AtomicBool::new(false);
    let mut callbacks = 0;
    let error = visit_ship_runs(&source, 1, &tags, &cancelled, |_, _, _, _, _| {
        callbacks += 1;
        cancelled.store(true, Ordering::Release);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(callbacks, 1);
    let error = visit_ship_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(false),
        |_, _, _, _, _| Err(io::Error::other("synthetic callback refusal")),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "synthetic callback refusal");
    let mut unresolved = fleet_frame(0);
    *unresolved.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "unread country links".into(),
    };
    let cold = recorded_source(vec![unresolved]);
    let error = visit_ship_runs(&cold, 1, &tags, &AtomicBool::new(true), |_, _, _, _, _| {
        panic!("cancelled visitor must not call back")
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(scans_run(&cold), 0);
    visit_ship_runs(&cold, 1, &tags, &AtomicBool::new(false), |_, _, _, _, _| {
        panic!("unavailable country links must not prove a zero or known count")
    })
    .unwrap();
    assert_eq!(scans_run(&cold), 1);
}

#[test]
fn navy_unknown_runs_merge_for_both_measures_and_owners_but_keep_gaps() {
    let frames = (0..8)
        .filter(|hour| *hour != 3)
        .map(|hour| {
            let mut frame = fleet_frame(hour);
            if hour < 2 || (4..6).contains(&hour) {
                *frame.tables[TableId::Ships.index()] = TableView::Unavailable {
                    reason: "unread ships".into(),
                };
            } else if hour >= 6
                && let TableView::Complete(TableRows::Ships(ships)) =
                    &mut *frame.tables[TableId::Ships.index()]
            {
                ships[0].task_force = 999;
            }
            frame
        })
        .collect();
    let source = recorded_source(frames);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    for series in scanned.series.iter().chain(&scanned.strength_series) {
        for runs in series.values() {
            assert_eq!(runs.len(), 3);
            assert_eq!(
                (runs[0].first_hour, runs[0].last_hour, runs[0].total),
                (0, 1, None)
            );
            assert!(runs[1].total.is_some());
            assert_eq!(
                (runs[2].first_hour, runs[2].last_hour, runs[2].total),
                (4, 7, None)
            );
            assert!(runs.iter().all(|run| run.starts_segment));
        }
    }
}

#[test]
fn navy_scan_cancel_and_unknown_branch_are_refused() {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![fleet_frame(0)])]).unwrap();
    assert_eq!(
        scan(&replay, 999, &AtomicBool::new(false))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        scan(&replay, 1, &AtomicBool::new(true)).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn navy_complete_empty_ship_table_proves_zero_without_task_force_or_class_tables() {
    let mut empty = fleet_frame(0);
    *empty.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![]));
    *empty.tables[TableId::TaskForces.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    *empty.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![empty])]).unwrap();
    let scanned = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.unread, 0);
    for owner in &scanned.series {
        assert!(owner.values().all(|runs| runs[0].total == Some(0)));
    }
}

fn dense_frame(hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, countries);
    let country_rows = frame.complete::<tables::Countries>().unwrap();
    let mut forces = Vec::with_capacity(countries as usize);
    let mut ships = Vec::new();
    let mut models = Vec::new();
    for (index, country) in country_rows.iter().enumerate() {
        let force = index as u64 + 1;
        forces.push(TaskForceRow {
            id: force,
            roster_owner: country.id,
            logical_owner: country.id,
            ..Default::default()
        });
        let count = if index < 10 { 1_200 } else { 3 + index % 7 };
        for ship in 0..count {
            let id = ships.len() as u64 + 1;
            ships.push(ShipRow {
                id,
                task_force: force,
                class_id: 41 + (ship % 3) as i32,
                // Dense hourly deltas exercise reconstruction independently of count.
                strength: 100_000 - (hour as i64 * 10 + ship as i64 % 10),
                ..Default::default()
            });
            models.push(ShipModelRow {
                ship: id,
                counter: Some(ShipCounterInputs {
                    maximum_strength: 200_000,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(forces));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(ships));
    *frame.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(models));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 41,
                literal: b"mod_escort".to_vec(),
            },
            LexicalTokenRow {
                token: 42,
                literal: b"mod_carrier".to_vec(),
            },
            LexicalTokenRow {
                token: 43,
                literal: b"mod_cruiser".to_vec(),
            },
        ]));
    frame
}

fn project(scanned: &ScannedNavy, request: &NavyParams) -> NavyReply {
    checked_reply(
        reply(
            scanned,
            request,
            None,
            |_| None,
            vec![],
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn navy_lexical_spelling_recovers_without_backdating_or_merging_reloaded_tokens() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut first = fleet_frame(0);
    *first.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let mut changed = fleet_frame(2);
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *changed.tables[TableId::LexicalTokens.index()]
    {
        rows[0].literal = b"other_mod_hull".to_vec();
    }
    let source = recorded_source(vec![first, fleet_frame(1), changed]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let runs = &scanned.series[0]["MOD_A"];
    assert_eq!(runs.len(), 3);
    for (run, spelling) in runs
        .iter()
        .zip([None, Some("mod_escort"), Some("other_mod_hull")])
    {
        assert_eq!(run.classes.len(), 1);
        let (&class, &count) = run.classes.first_key_value().unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            scanned.classes[class as usize],
            (41, spelling.map(str::to_owned))
        );
    }
    assert_eq!(scanned.classes.len(), 5);
    let mut units = installed_content::unit_leader::SubUnits::default();
    units.read(
        b"sub_units={mod_escort={type=screen_ship} other_mod_hull={type=carrier}}",
        "mod.txt",
    );
    let output = reply(
        &scanned,
        &params(),
        Some(&units),
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.parts.len(), 3);
    for (hour, class) in [(1, "class:mod_escort"), (2, "class:other_mod_hull")] {
        let column = output
            .parts
            .iter()
            .position(|part| part.key == class)
            .unwrap();
        assert_eq!(output.series[0].runs[hour].values[column], Some(2));
        assert_eq!(output.series[0].runs[0].values[column], Some(0));
    }
    assert_eq!(
        output.parts.iter().filter(|part| part.unresolved).count(),
        1
    );
}

#[test]
fn navy_selected_parts_use_layered_mod_classes_and_native_role_masks() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!("hoi4_navy_mod_{}", uuid::Uuid::new_v4()));
    let base = root.join("base");
    let addon = root.join("mod");
    for (folder, text) in [
        (&base, "sub_units={mod_escort={type=submarine}}"),
        (
            &addon,
            "sub_units={mod_escort={type=screen_ship} mod_carrier={type={carrier capital_ship}}}",
        ),
    ] {
        std::fs::create_dir_all(folder.join("common/units")).unwrap();
        std::fs::write(folder.join("common/units/fleet.txt"), text).unwrap();
    }
    let layers = installed_content::ResolvedLayers::from_folders(&[
        (&base, &[][..]),
        (&addon, &["common/units"][..]),
    ])
    .unwrap();
    let units = installed_content::unit_leader::sub_units(&layers).unwrap();
    drop(layers);
    std::fs::remove_dir_all(root).unwrap();
    let source = recorded_source(vec![fleet_frame(0)]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let classes = reply(
        &scanned,
        &params(),
        Some(&units),
        |key| Some(format!("mod label:{key}")),
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    // MOD_B's carrier must not add a column to MOD_A's selected reply.
    assert_eq!(classes.parts.len(), 1);
    assert_eq!(classes.parts[0].key, "class:mod_escort");
    assert_eq!(
        classes.parts[0].name.as_deref(),
        Some("mod label:mod_escort")
    );
    assert_eq!(classes.series[0].runs[0].values, [Some(2)]);
    let mut request = params();
    request.tags.push("MOD_B".into());
    request.grouping = NavyGrouping::Type;
    let roles = reply(
        &scanned,
        &request,
        Some(&units),
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(roles.parts.len(), 2);
    assert!(roles.parts.iter().all(|part| part.name.is_none()));
    let missing = project(&scanned, &request);
    assert!(missing.parts.iter().all(|part| part.unresolved));
    assert_eq!(
        missing.series[0].runs[0]
            .values
            .iter()
            .flatten()
            .sum::<i64>(),
        2
    );
}

#[test]
fn navy_strength_is_exact_fixed5_hp_with_independently_known_maximum() {
    let mut frame = fleet_frame(0);
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *frame.tables[TableId::Ships.index()]
    {
        rows[0].strength = 125_001;
        rows[1].strength = 50;
        rows[2].strength = 500_000;
    }
    *frame.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(vec![
        // Extra models and a missing ship's model exercise the sorted merge join.
        ShipModelRow {
            ship: 30,
            counter: Some(ShipCounterInputs {
                maximum_strength: 999_000,
                ..Default::default()
            }),
            ..Default::default()
        },
        ShipModelRow {
            ship: 31,
            counter: Some(ShipCounterInputs {
                maximum_strength: 300_000,
                ..Default::default()
            }),
            ..Default::default()
        },
        // Maximum strength clamps stored maxima to at least 100.
        ShipModelRow {
            ship: 32,
            counter: Some(ShipCounterInputs {
                maximum_strength: 0,
                ..Default::default()
            }),
            ..Default::default()
        },
        ShipModelRow {
            ship: 34,
            counter: Some(ShipCounterInputs {
                maximum_strength: 999_000,
                ..Default::default()
            }),
            ..Default::default()
        },
    ]));
    let source = recorded_source(vec![frame]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let mut request = params();
    request.measure = NavyMeasure::Strength;
    request.tags.push("MOD_B".into());
    let output = project(&scanned, &request);
    assert_eq!(output.series[0].runs[0].total, Some(125_051));
    assert_eq!(output.series[0].runs[0].reference_total, Some(300_100));
    assert_eq!(
        output.series[0].runs[0]
            .values
            .iter()
            .flatten()
            .sum::<i64>(),
        125_051
    );
    assert_eq!(output.series[1].runs[0].total, Some(500_000));
    assert_eq!(output.series[1].runs[0].reference_total, None);
    request.ownership = NavyOwnership::Logical;
    assert_eq!(
        project(&scanned, &request).series[0].runs[0].total,
        Some(500_000)
    );
    assert_eq!(
        project(&scanned, &params()).series[0].runs[0].total,
        Some(2)
    );
    assert_eq!(scans_run(&source), 1);
}

#[test]
fn navy_hp_refusals_are_local_and_never_refuse_ship_counts() {
    let frames = (0..4)
        .map(|hour| {
            let mut frame = fleet_frame(hour);
            if let TableView::Complete(TableRows::Ships(ships)) =
                &mut *frame.tables[TableId::Ships.index()]
            {
                ships[0].strength = match hour {
                    0 | 1 => i64::MAX,
                    2 => 10,
                    _ => -1,
                };
                ships[1].strength = match hour {
                    0 | 1 => 1,
                    2 => 20,
                    _ => 2,
                };
                ships[2].strength = 125_000;
                if hour == 1 {
                    ships[1].class_id = 42;
                }
            }
            *frame.tables[TableId::ShipModels.index()] =
                TableView::Complete(TableRows::ShipModels(vec![
                    ShipModelRow {
                        ship: 31,
                        counter: Some(ShipCounterInputs {
                            maximum_strength: if hour < 3 { i64::MAX } else { 200_000 },
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ShipModelRow {
                        ship: 32,
                        counter: Some(ShipCounterInputs {
                            maximum_strength: 200_000,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ShipModelRow {
                        ship: 33,
                        counter: Some(ShipCounterInputs {
                            maximum_strength: 200_000,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect();
    let source = recorded_source(frames);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let counts = project(&scanned, &params());
    for hour in 0..4 {
        assert_eq!(
            counts.series[0]
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .and_then(|run| run.total),
            Some(2)
        );
    }
    let roster = &scanned.strength_series[0];
    assert_eq!(roster["MOD_A"].len(), 3);
    assert_eq!(
        (roster["MOD_A"][0].last_hour, roster["MOD_A"][0].total),
        (1, None)
    );
    assert_eq!(roster["MOD_A"][1].total, Some(30));
    assert_eq!(roster["MOD_A"][1].reference_total, None);
    assert_eq!(roster["MOD_A"][2].total, None);
    assert_eq!(roster["MOD_A"][2].reference_total, Some(400_000));
    assert_eq!(roster["MOD_B"].len(), 1);
    assert_eq!(roster["MOD_B"][0].total, Some(125_000));
    assert_eq!(roster["MOD_B"][0].reference_total, Some(200_000));
    assert_eq!(scanned.refused_strength, [3, 3]);
    assert_eq!(scanned.maximum_overflows, [3, 3]);
    let mut request = params();
    request.measure = NavyMeasure::Strength;
    let output = project(&scanned, &request);
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("3 country-hours have negative or overflowing"))
    );
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("3 country-hours have overflowing maximum"))
    );
    assert!(
        !project(&scanned, &params())
            .warnings
            .iter()
            .any(|warning| warning.contains("hull HP"))
    );
}

#[test]
fn navy_top_ranks_by_selected_measure_and_refuses_only_transport_budget() {
    let mut first = fleet_frame(0);
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *first.tables[TableId::Ships.index()]
    {
        rows[2].strength = 999_000;
    }
    let source = recorded_source(vec![first]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let mut request = params();
    request.tags.clear();
    request.top = Some(1);
    assert_eq!(project(&scanned, &request).series[0].tag, "MOD_A");
    request.measure = NavyMeasure::Strength;
    assert_eq!(project(&scanned, &request).series[0].tag, "MOD_B");
    let error = checked_reply(
        reply(
            &scanned,
            &request,
            None,
            |_| None,
            vec!["x".repeat(MAX_REPLY_BYTES)],
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("transport budget"));
    request.top = Some(5000);
    NavySize::validate(&request).unwrap();
    request.tags.push("opaque country".into());
    assert!(NavySize::validate(&request).is_err());
}

#[test]
fn navy_large_native_hp_only_refuses_the_json_strength_projection() {
    let mut frame = fleet_frame(0);
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *frame.tables[TableId::Ships.index()]
    {
        rows[0].strength = MAX_EXACT + 1;
    }
    let source = recorded_source(vec![frame]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        project(&scanned, &params()).series[0].runs[0].total,
        Some(2)
    );
    let mut request = params();
    request.measure = NavyMeasure::Strength;
    assert!(
        reply(
            &scanned,
            &request,
            None,
            |_| None,
            vec![],
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("exact integer range")
    );
}

#[test]
fn navy_manual_aliases_and_top_replies_keep_native_holder_history() {
    let frames = (0..=2)
        .map(|hour| {
            let mut frame = fleet_frame(hour);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                if hour > 0 {
                    rows[0].tag = b"renamed naval holder".to_vec();
                }
                if hour == 2 {
                    rows[1].tag = b"MOD_A".to_vec();
                }
            }
            frame
        })
        .collect();
    let source = recorded_source(frames);
    let mut request = params();
    for top in [false, true] {
        if top {
            request.tags.clear();
            request.top = Some(1);
        }
        let output = NavySize::load(
            &source,
            &request,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let old = output
            .series
            .iter()
            .find(|series| series.tag == "MOD_A")
            .unwrap();
        let renamed = output
            .series
            .iter()
            .find(|series| series.tag == "renamed naval holder")
            .unwrap();
        assert_eq!(old.identities[0].last_hour, 0);
        assert_eq!(old.identities[1].country_id, 102);
        assert_eq!(old.identities[1].first_hour, 2);
        assert_eq!(renamed.identities[0].country_id, 101);
        assert_eq!(renamed.identities[0].slot, 7);
        assert_eq!(renamed.identities[0].last_hour, 2);
        assert_eq!(renamed.runs.last().unwrap().total, Some(2));
    }
}

#[test]
fn navy_serves_five_thousand_countries_and_reuses_scoreboard_scan() {
    let mut frame = scale_frame(1, 0, 5000);
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![]));
    let source = recorded_source(vec![frame]);
    let mut request = params();
    request.tags = (0..5000)
        .map(|index| String::from_utf8(scale_tag(index)).unwrap())
        .collect();
    for top in [false, true] {
        if top {
            request.tags.clear();
            request.top = Some(5000);
        }
        NavySize::validate(&request).unwrap();
        let output = NavySize::load(
            &source,
            &request,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output.series.len(), 5000);
        assert!(
            output
                .series
                .iter()
                .all(|series| series.identities.len() == 1)
        );
        assert!(
            output
                .series
                .iter()
                .all(|series| series.runs[0].total == Some(0))
        );
        assert!(dashboard_core::transport::json_len(&output, MAX_REPLY_BYTES).is_some());
    }
    let tags: Vec<_> = (0..5000)
        .map(|index| String::from_utf8(scale_tag(index)).unwrap())
        .collect();
    let mut visited = 0;
    visit_ship_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(false),
        |_, _, _, _, _| {
            visited += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(visited, 5000);
    assert_eq!(scans_run(&source), 1);
}

#[test]
fn navy_unsupported_country_tags_are_named_and_counted() {
    let mut frame = fleet_frame(0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = vec![255];
    }
    let source = recorded_source(vec![frame]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert!(
        project(&scanned, &params())
            .warnings
            .iter()
            .any(|warning| warning.contains("Country tag bytes [255]: 1 observations"))
    );
    let error = visit_ship_runs(
        &source,
        1,
        &["MOD_B".into()],
        &AtomicBool::new(false),
        |_, _, _, _, _| panic!("undisplayable native tags must refuse before visiting"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("[255]: 1"));
}

#[test]
fn navy_cache_retains_two_scanned_branches_and_rebuilds_evicted_history() {
    let branches: Vec<_> = (1..=3)
        .map(|branch| {
            let mut frame = fleet_frame(0);
            frame.branch = branch;
            records(branch, vec![frame])
        })
        .collect();
    let source = Source::from_replay(
        SourceKey {
            source_id: "navy-cache".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    for branch in [1, 2, 3, 2] {
        let mut values = Vec::new();
        visit_ship_runs(
            &source,
            branch,
            &["MOD_A".into()],
            &AtomicBool::new(false),
            |_, first, last, _, value| {
                assert_eq!((first, last), (0, 0));
                values.push(value);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(values, [Some(2)]);
    }
    let state = source.state::<Scans>();
    assert_eq!(state.lock().unwrap().runs, 3);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|(branch, _)| *branch)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    visit_ship_runs(
        &source,
        1,
        &["MOD_A".into()],
        &AtomicBool::new(false),
        |_, _, _, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(state.lock().unwrap().runs, 4);
}

#[test]
#[ignore = "dense decode-inclusive measurement; lead runs optimised"]
fn navy_dense_scale_including_decode() {
    use std::time::Instant;
    for countries in [1500, 5000] {
        let frames: Vec<_> = (0..24).map(|hour| dense_frame(hour, countries)).collect();
        let ships = frames[0].complete::<tables::Ships>().unwrap().len();
        let source = recorded_source(frames);
        let started = Instant::now();
        let mut decode_started = None;
        let mut decode_time = std::time::Duration::ZERO;
        let scanned = scan::scan_with_decode(
            source.replay_ref(),
            1,
            &AtomicBool::new(false),
            |decoding| {
                if decoding {
                    decode_started = Some(Instant::now());
                } else {
                    decode_time += decode_started.take().unwrap().elapsed();
                }
            },
        )
        .unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0 / 24.0;
        let decode_ms = decode_time.as_secs_f64() * 1000.0 / 24.0;
        let product_ms = ms - decode_ms;
        assert_eq!(scanned.series[0].len(), countries as usize);
        let major = String::from_utf8(scale_tag(0)).unwrap();
        assert_eq!(scanned.series[0][&major][0].total, Some(1200));
        assert_eq!(
            scanned.strength_series[0][&major][0].total,
            Some(119_994_600)
        );
        assert_eq!(
            scanned.strength_series[0][&major][0].reference_total,
            Some(240_000_000)
        );
        let mut request = params();
        request.tags.clear();
        request.top = Some(10);
        let bytes =
            dashboard_core::transport::json_len(&project(&scanned, &request), MAX_REPLY_BYTES)
                .unwrap();
        request.measure = NavyMeasure::Strength;
        let hp_bytes =
            dashboard_core::transport::json_len(&project(&scanned, &request), MAX_REPLY_BYTES)
                .unwrap();
        println!(
            "navy countries={countries} ships={ships} hours=24 scan_with_decode_ms_per_hour={ms:.3} decode_ms_per_hour={decode_ms:.3} product_ms_per_hour={product_ms:.3} top10_ship_bytes={bytes} top10_hp_bytes={hp_bytes} bound={MAX_REPLY_BYTES}"
        );
    }
}

#[test]
fn navy_sparse_ship_and_model_updates_match_full_censuses_and_share_counts() {
    let mut first = fleet_frame(0);
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *first.tables[TableId::Ships.index()]
    {
        for row in rows {
            row.strength = 1000;
        }
    }
    *first.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(
        [31, 32, 33]
            .map(|ship| ShipModelRow {
                ship,
                counter: Some(ShipCounterInputs {
                    maximum_strength: 100_000,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .into(),
    ));
    let mut hp = first.clone();
    hp.hour = 1;
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *hp.tables[TableId::Ships.index()] {
        rows[0].strength -= 100;
    }
    let mut model = hp.clone();
    model.hour = 2;
    if let TableView::Complete(TableRows::ShipModels(rows)) =
        &mut *model.tables[TableId::ShipModels.index()]
    {
        rows[1].counter.as_mut().unwrap().maximum_strength += 100;
    }
    let mut removed = model.clone();
    removed.hour = 3;
    if let TableView::Complete(TableRows::Ships(rows)) =
        &mut *removed.tables[TableId::Ships.index()]
    {
        rows.retain(|row| row.id != 32);
        rows.push(ShipRow {
            id: 34,
            task_force: 21,
            class_id: 41,
            strength: 50,
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::ShipModels(rows)) =
        &mut *removed.tables[TableId::ShipModels.index()]
    {
        rows.retain(|row| row.ship != 32);
        rows.push(ShipModelRow {
            ship: 34,
            counter: Some(ShipCounterInputs {
                maximum_strength: 100_000,
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    let frames = vec![first, hp, model, removed];
    let source = recorded_source(frames.clone());
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.updated_ships, 6);
    assert_eq!(scanned.series[0]["MOD_A"].len(), 1);
    assert_eq!(scanned.strength_series[0]["MOD_A"].len(), 4);
    for observed in frames {
        let hour = observed.hour;
        let rebuilt = scan(
            &Replay::from_records(CAMPAIGN.into(), &[records(1, vec![observed])]).unwrap(),
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        for measure in [NavyMeasure::Ships, NavyMeasure::Strength] {
            for ownership in [NavyOwnership::Roster, NavyOwnership::Logical] {
                let mut request = params();
                request.measure = measure;
                request.ownership = ownership;
                let expected = &rebuilt.series(&request)["MOD_A"][0];
                let run = scanned.series(&request)["MOD_A"]
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    .unwrap();
                assert_eq!(
                    (run.total, run.reference_total),
                    (expected.total, expected.reference_total)
                );
                let named = |scan: &ScannedNavy, run: &RawNavyRun| {
                    run.classes
                        .iter()
                        .map(|(&id, &count)| (scan.classes[id as usize].clone(), count))
                        .collect::<BTreeMap<_, _>>()
                };
                assert_eq!(named(&scanned, run), named(&rebuilt, expected));
            }
        }
    }
}

#[test]
#[ignore = "realistic-change decode-inclusive measurement; lead runs optimised"]
fn navy_realistic_scale_including_decode() {
    use std::time::Instant;
    for countries in [1500, 5000] {
        let mut previous = dense_frame(0, countries);
        let ships = previous.complete::<tables::Ships>().unwrap().len();
        let mut frames = vec![previous.clone()];
        for hour in 1..24 {
            previous.hour = hour;
            if let TableView::Complete(TableRows::Ships(rows)) =
                &mut *previous.tables[TableId::Ships.index()]
            {
                // Rotating 1/64 of units, plus one large battle; HP is fixed5.
                for ship in rows {
                    if hour == 12 || ship.id % 64 == hour as u64 % 64 {
                        ship.strength -= 100;
                    }
                }
            }
            frames.push(previous.clone());
        }
        let source = recorded_source(frames);
        let started = Instant::now();
        let mut decode_started = None;
        let mut decode_time = std::time::Duration::ZERO;
        let scanned = scan::scan_with_decode(
            source.replay_ref(),
            1,
            &AtomicBool::new(false),
            |decoding| {
                if decoding {
                    decode_started = Some(Instant::now());
                } else {
                    decode_time += decode_started.take().unwrap().elapsed();
                }
            },
        )
        .unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0 / 24.0;
        let decode_ms = decode_time.as_secs_f64() * 1000.0 / 24.0;
        let product_ms = ms - decode_ms;
        assert_eq!(scanned.series[0].len(), countries as usize);
        let major = String::from_utf8(scale_tag(0)).unwrap();
        assert_eq!(scanned.series[0][&major][0].total, Some(1200));
        let expected: i64 = previous
            .complete::<tables::Ships>()
            .unwrap()
            .iter()
            .filter(|ship| ship.task_force == 1)
            .map(|ship| ship.strength)
            .sum();
        let final_run = scanned.strength_series[0][&major].last().unwrap();
        assert_eq!(final_run.total, Some(expected));
        assert_eq!(final_run.reference_total, Some(240_000_000));
        let mut request = params();
        request.tags.clear();
        request.top = Some(10);
        let bytes =
            dashboard_core::transport::json_len(&project(&scanned, &request), MAX_REPLY_BYTES)
                .unwrap();
        request.measure = NavyMeasure::Strength;
        let hp_bytes =
            dashboard_core::transport::json_len(&project(&scanned, &request), MAX_REPLY_BYTES)
                .unwrap();
        println!(
            "navy_realistic countries={countries} ships={ships} hours=24 scan_with_decode_ms_per_hour={ms:.3} decode_ms_per_hour={decode_ms:.3} product_ms_per_hour={product_ms:.3} top10_ship_bytes={bytes} top10_hp_bytes={hp_bytes} bound={MAX_REPLY_BYTES}"
        );
    }
}

#[test]
fn phase_b_navy_database_success_and_failure_are_bound_before_observation() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for removed in [false, true] {
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "common/units/fleet.txt",
            "sub_units={mod_hull={}}",
        )]);
        let source = recorded_source(vec![fleet_frame(100)]);
        let catalogue = Arc::new(catalogue);
        if removed {
            std::fs::remove_file(folder.0.join("common/units/fleet.txt")).unwrap();
        }
        let configured = content_units(&catalogue, &mut Vec::new());
        assert_eq!(configured.is_some(), !removed);
        let database = catalogue
            .content
            .layers
            .derived::<catalogue::CachedResult<installed_content::unit_leader::SubUnits>>();
        let binding = part_content(&source, Some(&catalogue));
        let key = (41, Some("mod_hull".into()));
        let classes = BTreeSet::from([0]);
        let expected = parts(
            &classes,
            std::slice::from_ref(&key),
            configured.as_deref(),
            NavyGrouping::Class,
            |_| None,
            Some(&binding),
        );
        for _ in 0..10 {
            let mut warnings = Vec::new();
            let units = content_units(&catalogue, &mut warnings);
            assert_eq!(units.is_some(), !removed);
            assert_eq!(
                parts(
                    &classes,
                    std::slice::from_ref(&key),
                    units.as_deref(),
                    NavyGrouping::Class,
                    |_| None,
                    Some(&binding)
                ),
                expected
            );
        }
        assert_eq!(database.loads(), 1);
        assert_eq!(binding.loads(&(key, 0)), 1);
    }
}
