use super::*;

#[test]
fn quiet_leader_rejections_name_the_failed_proof_without_advancing() {
    use dashboard_base::prepared::prepass::noted;
    let rows = prepared::test_fixtures::frame(100, 1);
    let unchanged = block::Changes::default();
    let ready = || Scan {
        commands_continuous: true,
        frontier: Some(100),
        read_mask: Some(0),
        ..Default::default()
    };
    for (first, last, commands, frontier, mask, reason) in [
        (
            i32::MIN,
            i32::MIN,
            true,
            Some(100),
            Some(0),
            "leaders.reject.quiet-first-hour",
        ),
        (
            101,
            100,
            true,
            Some(100),
            Some(0),
            "leaders.reject.quiet-order",
        ),
        (
            101,
            102,
            false,
            Some(100),
            Some(0),
            "leaders.reject.quiet-commands",
        ),
        (
            101,
            102,
            true,
            Some(99),
            Some(0),
            "leaders.reject.quiet-frontier",
        ),
        (
            101,
            102,
            true,
            Some(100),
            None,
            "leaders.reject.quiet-read-mask",
        ),
    ] {
        let mut scan = ready();
        scan.commands_continuous = commands;
        scan.frontier = frontier;
        scan.read_mask = mask;
        let before = noted(reason);
        assert!(!scan.advance_quiet(first, last, &unchanged, &rows, None));
        assert_eq!(noted(reason) - before, 1, "{reason}");
        assert_eq!(scan.frontier, frontier, "{reason}");
    }
    let mut scan = ready();
    assert!(scan.advance_quiet(101, 102, &unchanged, &rows, None));
    assert_eq!(scan.frontier, Some(102));
}

#[test]
fn a_kept_leader_visibility_rejection_is_counted_once_per_hour() {
    use dashboard_base::prepared::prepass::noted;
    use dashboard_text::triggers::inputs::{TableChange, TableChanges, capture, table};

    struct Unread;
    impl TableChanges for Unread {
        fn table(&self, table: TableId) -> TableChange<'_> {
            if table == TableId::ScriptVariables {
                TableChange::Any
            } else {
                TableChange::Same
            }
        }
    }

    let frame = prepared::test_fixtures::frame(100, 1);
    let (_, inputs) = capture(&frame, || table(&frame, TableId::ScriptVariables));
    let reduced = Unread;
    let kept = Kept::new(&reduced);
    assert!(!kept.reads_whole(&inputs, &frame, frame.hour));
    let reason = "leaders.reject.kept-inputs";
    let before = noted(reason);
    kept.block_inputs(&inputs, &frame);
    kept.block_inputs(&inputs, &frame);
    assert!(kept.blocked());
    assert_eq!(noted(reason) - before, 1);

    let unchanged = block::Changes::default();
    let whole = Kept::new(&unchanged);
    assert!(whole.reads_whole(&inputs, &frame, frame.hour));
    assert!(!whole.blocked());
    assert_eq!(noted(reason) - before, 1);
}

#[test]
fn missing_trait_sprite_retries_native_gui_fallback_with_registered_spelling() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_text::catalogue::texture_tests::{from_url, png_bytes, texture_catalogue};
    let image = installed_content_image::Rgba8Image {
        width: 2,
        height: 2,
        pixels: [12, 34, 56, 255].repeat(4),
    };
    let (catalogue, _folders) = texture_catalogue(
        &[
            ("common/defines/test.lua", "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}"),
            (
                "common/unit_leader/test.txt",
                "leader_traits={Mod_Trait={} Canonical_Trait={} Broken_Trait={}}",
            ),
            (
                "interface/art.gfx",
                "spriteTypes={spriteType={name=GFX_default_fallback_texture textureFile=gfx/fallback.png}
                    spriteType={name=GFX_trait_Canonical_Trait textureFile=gfx/canonical.png}
                    spriteType={name=GFX_trait_Broken_Trait textureFile=gfx/missing.png}}",
            ),
        ],
        &[("gfx/fallback.png", png_bytes(&image)), ("gfx/canonical.png", png_bytes(&installed_content_image::Rgba8Image { width: 1, height: 1, pixels: vec![23,45,67,255] }))],
    );
    let catalogue = Arc::new(catalogue);
    let content = Content {
        politics: Arc::new(politics_content::PoliticsContent::load(catalogue.clone()).unwrap()),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let info = trait_info("mod_trait", Some(&content));
    assert!(info.missing.is_none(), "{:?}", info.missing);
    assert_eq!(from_url(info.icon.as_ref().unwrap()), image);
    let canonical = trait_info("canonical_trait", Some(&content));
    assert_eq!(
        from_url(canonical.icon.as_ref().unwrap()).pixels,
        [23, 45, 67, 255]
    );
    let broken = trait_info("Broken_Trait", Some(&content));
    assert!(broken.icon.is_none());
    assert!(broken.missing.unwrap().contains("missing.png"));
    assert!(
        trait_info("unregistered", Some(&content))
            .missing
            .unwrap()
            .contains("not registered")
    );
}

#[test]
fn leader_outcomes_join_native_handles_and_keep_pending_battles_unknown() {
    use replay_data::{
        rows::LandCombatDetailsRow, rows::LandCombatResultsRow, rows::LandCombatSideDetails,
        rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let make = |hour, combats: &[u64]| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 11,
                kind: 1,
                category: 1,
                character_id: 9,
                skill_status: 2,
                skill_level: 3,
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(Vec::new()));
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(
                combats
                    .iter()
                    .map(|combat| LandCombatDetailsRow {
                        combat: *combat,
                        sides: [
                            LandCombatSideDetails {
                                commander: reference,
                                ..Default::default()
                            },
                            Default::default(),
                        ],
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame
    };
    let mut ended = make(104, &[11]);
    *ended.tables[TableId::LandCombatResults.index()] =
        TableView::Complete(TableRows::LandCombatResults(vec![
            LandCombatResultsRow {
                id: 1,
                combat: 10,
                end_hour: 102,
                winners: 1,
                ..Default::default()
            },
            LandCombatResultsRow {
                id: 2,
                combat: 12,
                end_hour: 102,
                winners: 2,
                sides: [
                    replay_data::rows::LandResultSide {
                        commander: reference,
                        ..Default::default()
                    },
                    Default::default(),
                ],
                ..Default::default()
            },
        ]));
    let mut repeated = ended.clone();
    repeated.hour = 105;
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(
            1,
            vec![make(100, &[10]), make(101, &[10, 11]), ended, repeated],
        )],
    )
    .unwrap();
    let reply = scan(&replay, &all_params(), None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.leaders.len(), 1);
    assert_eq!(reply.leaders[0].battle_starts, Some(vec![100, 1, 1]));
    assert_eq!(
        reply.leaders[0].battle_won.as_deref(),
        Some(&[Some(true), None, Some(false)][..])
    );
    assert_eq!(
        reply.leaders[0].battle_result_hours.as_deref(),
        Some(&[Some(102), None, Some(102)][..])
    );
    assert_eq!(
        reply.leaders[0].battle_start_read_hours,
        Some(vec![100, 101, 104])
    );
    assert_eq!(reply.leaders[0].unknown_battle_starts, Some(vec![0, 2]));
    assert_eq!(
        reply.leaders[0].battle_result_read_hours.as_deref(),
        Some(&[Some(104), None, Some(104)][..])
    );
}

#[test]
fn naval_outcomes_require_the_exact_persistent_source_and_idle_fleets_are_commands() {
    use replay_data::{
        rows::FleetRow, rows::NavalCombatDetailsRow, rows::NavalCombatResultDetailsRow,
        rows::NavalCombatSideDetails, rows::NavalResultRow, rows::NavalResultSideDetails,
        rows::NavalResultSourceRow, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let combats = [(10_u64 << 32) | 4718, (11_u64 << 32) | 4718];
    let make = |hour, active: bool| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 11,
                kind: 2,
                category: 2,
                character_id: 9,
                skill_status: 2,
                skill_level: 3,
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(Vec::new()));
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(if active {
                combats
                    .iter()
                    .map(|combat| NavalCombatDetailsRow {
                        combat: *combat,
                        custom_sea: Some(Vec::new()),
                        sides: [
                            NavalCombatSideDetails {
                                commander: reference,
                                ..Default::default()
                            },
                            Default::default(),
                        ],
                        ..Default::default()
                    })
                    .collect()
            } else {
                Vec::new()
            }));
        *frame.tables[TableId::Fleets.index()] =
            TableView::Complete(TableRows::Fleets(vec![FleetRow {
                id: 70,
                leader: Recorded::Value(Some(reference)),
                name: Recorded::Value(Some(b"Idle fleet".to_vec())),
                ..Default::default()
            }]));
        frame
    };
    let mut ended = make(104, false);
    *ended.tables[TableId::NavalResultSources.index()] =
        TableView::Complete(TableRows::NavalResultSources(vec![
            NavalResultSourceRow {
                id: 1,
                game_hour: 102,
                combat: combats[0],
                result: 44,
                ..Default::default()
            },
            NavalResultSourceRow {
                id: 2,
                game_hour: 102,
                combat: combats[1],
                result: 45,
                merged: true,
                ..Default::default()
            },
            NavalResultSourceRow {
                id: 3,
                game_hour: 102,
                combat: (12_u64 << 32) | 4718,
                result: 46,
                ..Default::default()
            },
        ]));
    *ended.tables[TableId::NavalResults.index()] =
        TableView::Complete(TableRows::NavalResults(vec![
            NavalResultRow {
                id: 44,
                winning_side: 2,
                ..Default::default()
            },
            NavalResultRow {
                id: 45,
                winning_side: 1,
                ..Default::default()
            },
            NavalResultRow {
                id: 46,
                winning_side: 1,
                ..Default::default()
            },
            NavalResultRow {
                id: 47,
                winning_side: 1,
                ..Default::default()
            },
        ]));
    *ended.tables[TableId::NavalCombatResultDetails.index()] =
        TableView::Complete(TableRows::NavalCombatResultDetails(vec![
            NavalCombatResultDetailsRow {
                result: 46,
                sides: [
                    NavalResultSideDetails {
                        leader: reference,
                        ..Default::default()
                    },
                    Default::default(),
                ],
            },
        ]));
    let mut repeated = ended.clone();
    repeated.hour = 105;
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(
            1,
            vec![make(100, true), make(101, true), ended, repeated],
        )],
    )
    .unwrap();
    let params = UnitLeadersParams {
        view: LeaderView::Career,
        ..all_params()
    };
    let reply = scan(&replay, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.leaders.len(), 1);
    let leader = &reply.leaders[0];
    assert_eq!(
        leader.battle_won.as_deref(),
        Some(&[Some(false), None, Some(true)][..])
    );
    assert_eq!(
        leader.battle_result_hours.as_deref(),
        Some(&[Some(102), None, Some(102)][..])
    );
    assert_eq!(leader.battle_starts, Some(vec![100, 0, 2]));
    assert_eq!(leader.battle_start_read_hours, Some(vec![100, 100, 104]));
    assert_eq!(
        leader.battle_result_read_hours.as_deref(),
        Some(&[Some(104), None, Some(104)][..])
    );
    assert!(
        leader
            .unknown_battle_starts
            .as_ref()
            .is_some_and(|unknown| unknown.contains(&2))
    );
    assert_eq!(leader.commands.len(), 2);
    assert!(leader.commands.iter().all(|command| {
        command.formation == "70" && command.kind == LeaderFormationKind::Fleet
    }));
    assert_eq!(
        leader
            .commands
            .iter()
            .map(|command| (command.first_hour, command.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (104, 105)]
    );
    assert!(reply.observed.iter().all(|read| read.naval_commands));
}

#[test]
#[ignore = "large 5000-country scan fixture; keep routine filtered tests light"]
fn scan_5000_countries_changed_rows_and_gaps() {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::TableRows, rows::UnitLeaderTraitRow,
    };
    use replay_frame::frame::TableView;
    let countries: Vec<_> = (0..5000)
        .map(|c| CountryRow {
            id: c + 1,
            canonical_slot: c + 1,
            tag: format!("mod country {c}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = (0..5000)
        .map(|c| CountryLinkRow {
            id: c + 1,
            canonical_slot: c + 1,
        })
        .collect();
    let mut frames = Vec::new();
    for tick in 0..80 {
        // Attributes change every other read; every fifth read follows a gap.
        let hour = 100 + tick + tick / 5;
        let mut f = dashboard_core::test_fixtures::source(1, hour, None);
        *f.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries.clone()));
        *f.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links.clone()));
        *f.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(
            (0..20_000_u32)
                .map(|n| UnitLeaderRow {
                    id: n + 1,
                    reference_type: 4713,
                    character_id: n + 1,
                    owner: n / 4 + 1,
                    kind: 1,
                    category: 1,
                    skill_status: 2,
                    skill_level: 3,
                    components: [3 + tick / 2, 2, 2, 1],
                    ..Default::default()
                })
                .collect(),
        ));
        *f.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
            (0..20_000_u32)
                .map(|n| CharacterRow {
                    id: n + 1,
                    token: format!("Commander {n}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *f.tables[TableId::CharacterPortraits.index()] =
            TableView::Complete(TableRows::CharacterPortraits(
                (0..20_000_u32)
                    .map(|n| CharacterPortraitRow {
                        character: n + 1,
                        portraits: vec![],
                    })
                    .collect(),
            ));
        *f.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(
                (0..20_000_u32)
                    .map(|n| UnitLeaderTraitRow {
                        leader_reference: (u64::from(n + 1) << 32) | 4713,
                        token: b"mod_trait".to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *f.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![]));
        *f.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![]));
        frames.push(f);
    }
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::All,
        view: LeaderView::Career,
        leaders: LeaderKinds::All,
    };
    let reply = scan(&replay, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.leaders.len(), 20_000);
    assert!(reply.leaders.iter().all(|leader| leader.runs.len() > 40));
}

#[test]
fn scan_joins_raw_owners_commands_and_counts_each_combat_once() {
    use replay_data::{
        rows::CountryRow, rows::LandCombatDetailsRow, rows::LandCombatSideDetails,
        rows::NavalCombatDetailsRow, rows::NavalCombatSideDetails, rows::OrdersGroupPlanRow,
        rows::OrdersGroupRow, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let reference = (5u64 << 32) | 4713;
    let make = |hour, combat, commander| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![
                CountryRow {
                    id: 9001,
                    canonical_slot: 7,
                    tag: b"mod country with spaces".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 9002,
                    canonical_slot: 8,
                    tag: b"second country with spaces".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 11,
                kind: 1,
                category: 1,
                character_id: 9,
                skill_level: 3,
                skill_status: 2,
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(vec![]));
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(if combat == 0 {
                vec![]
            } else {
                vec![LandCombatDetailsRow {
                    combat,
                    sides: [
                        LandCombatSideDetails {
                            commander,
                            ..Default::default()
                        },
                        LandCombatSideDetails {
                            commander,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }]
            }));
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
                combat: 90,
                custom_sea_error: 1,
                sides: [
                    NavalCombatSideDetails {
                        commander: 0,
                        ..Default::default()
                    },
                    NavalCombatSideDetails::default(),
                ],
                ..Default::default()
            }]));
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 70,
                leader: reference,
                is_army_group: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::OrdersGroupPlans.index()] =
            TableView::Complete(TableRows::OrdersGroupPlans(vec![OrdersGroupPlanRow {
                group: 70,
                name: b"Exact recorded name".to_vec(),
                ..Default::default()
            }]));
        frame
    };
    let mut transferred = make(107, 21, reference);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *transferred.tables[TableId::CountryLinks.index()]
    {
        links
            .iter_mut()
            .find(|l| l.id == 11)
            .unwrap()
            .canonical_slot = 8;
    }
    let frames = vec![
        make(100, 20, reference),
        make(101, 20, reference),
        make(102, 0, 0),
        make(103, 21, reference),
        make(106, 21, reference),
        transferred,
    ];
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::Leader {
            reference: reference.to_string(),
        },
        view: LeaderView::Career,
        leaders: LeaderKinds::All,
    };
    let reply = scan(&replay, &params, None, &AtomicBool::new(false)).unwrap();
    let l = &reply.leaders[0];
    assert_eq!(
        reply.roles[l.runs[0].value.role as usize].owner.as_deref(),
        Some("mod country with spaces")
    );
    assert_eq!(l.runs.len(), 3);
    assert_eq!(l.commands.len(), 2);
    assert_eq!(l.commands[0].kind, LeaderFormationKind::ArmyGroup);
    // Both sides and repeated hours still count once; the gap never fills.
    assert_eq!(l.battle_starts, Some(vec![100, 3]));
    assert_eq!(l.unknown_battle_starts, Some(vec![0]));
    assert!(l.battle_won.is_none());
    let country_request = UnitLeadersParams {
        selection: LeaderSelection::Countries {
            tags: vec!["second country with spaces".into()],
        },
        view: LeaderView::Leaderboard,
        ..params.clone()
    };
    let transferred = scan(&replay, &country_request, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(transferred.leaders.len(), 1);
    assert_eq!(transferred.leaders[0].runs[0].first_hour, 100);
    assert_eq!(
        transferred.leaders[0].battle_starts.as_ref().map(Vec::len),
        Some(2)
    );
    let excluded = UnitLeadersParams {
        selection: LeaderSelection::Countries {
            tags: vec!["different country".into()],
        },
        view: LeaderView::Leaderboard,
        ..params
    };
    assert!(
        scan(&replay, &excluded, None, &AtomicBool::new(false))
            .unwrap()
            .leaders
            .is_empty()
    );
    assert!(scan(&replay, &excluded, None, &AtomicBool::new(true)).is_err());
}

#[test]
fn missing_trait_reads_kinds_and_lightweight_choices_stay_honest() {
    use replay_data::{rows::TableRows, rows::UnitLeaderTraitRow};
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let make = |hour: i32| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![
                UnitLeaderRow {
                    id: 5,
                    reference_type: 4713,
                    kind: 1,
                    owner: 11,
                    category: 1,
                    skill_status: if hour == 100 { 0 } else { 2 },
                    skill_level: 3,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 6,
                    reference_type: 4713,
                    kind: 2,
                    owner: 11,
                    category: 2,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 7,
                    reference_type: 4713,
                    kind: 3,
                    owner: 11,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 8,
                    reference_type: 4713,
                    kind: 1,
                    owner: 11,
                    category: 1,
                    captured: 1,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 9,
                    reference_type: 4713,
                    kind: 1,
                    owner: 11,
                    category: 1,
                    visibility_choice: 1,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::UnitLeaderTraits.index()] = if hour == 101 {
            TableView::Truncated {
                rows: TableRows::UnitLeaderTraits(vec![]),
                missing: 1,
            }
        } else {
            TableView::Complete(TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
                leader_reference: reference,
                token: b"extra_mod_trait".to_vec(),
                ..Default::default()
            }]))
        };
        frame
    };
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(
            1,
            vec![make(100), make(101), make(102)],
        )],
    )
    .unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::All,
        view: LeaderView::Leaderboard,
        leaders: LeaderKinds::All,
    };
    let reply = scan(&replay, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.leaders.len(), 2); // Operatives, captured and undecidable leaders are withheld.
    let army = &reply.leaders[0];
    assert_eq!(army.runs[0].value.skill, None);
    assert_eq!(army.runs[1].value.traits, None);
    assert_eq!(army.runs[2].value.traits.as_deref(), Some(&[0][..]));
    assert_eq!(army.trait_gains[0].date, LeaderChangeDate::HeldBefore);
    assert_eq!(army.trait_gains.len(), 1); // A missing read never re-dates an existing holding.
    assert!(reply.observed.iter().all(|o| !o.leaders));
    assert!(reply.traits[0].icon.is_none());
    assert!(reply.traits[0].missing.is_some());
    assert!(reply.warnings.iter().any(|warning| {
        warning.contains("Recorded content unavailable") && warning.contains("extra_mod_trait")
    }));
    for (kinds, kind) in [(LeaderKinds::Land, 1), (LeaderKinds::Navy, 2)] {
        let filtered = scan(
            &replay,
            &UnitLeadersParams {
                leaders: kinds,
                ..params.clone()
            },
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(filtered.leaders.len(), 1);
        assert_eq!(
            filtered.roles[filtered.leaders[0].runs[0].value.role as usize].kind,
            kind
        );
        assert!(filtered.people.len() <= reply.people.len());
    }
    let choices = scan(
        &replay,
        &UnitLeadersParams {
            view: LeaderView::Choices,
            ..params
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(choices.traits.is_empty());
    assert!(choices.leaders.iter().all(|leader| leader.runs.len() == 1
        && leader.runs[0].value.traits.is_none()
        && leader.battle_starts.as_ref().is_some_and(Vec::is_empty)));
}

#[test]
fn country_selection_keeps_only_its_trait_catalogue_and_remaps_gains() {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::TableRows, rows::UnitLeaderTraitRow,
    };
    use replay_frame::frame::TableView;
    let mut frame = dashboard_core::test_fixtures::source(1, 100, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [(7, "excluded country"), (8, "selected country")]
            .into_iter()
            .map(|(slot, tag)| CountryRow {
                id: slot,
                canonical_slot: slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (12, 8)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(
        [(5, 11), (6, 12)]
            .into_iter()
            .map(|(id, owner)| UnitLeaderRow {
                id,
                character_id: id,
                reference_type: 4713,
                owner,
                kind: 1,
                category: 1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::UnitLeaderTraits.index()] =
        TableView::Complete(TableRows::UnitLeaderTraits(
            [(5_u64, "excluded_trait"), (6, "selected_trait")]
                .into_iter()
                .map(|(id, token)| UnitLeaderTraitRow {
                    leader_reference: (id << 32) | 4713,
                    token: token.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, vec![frame])],
    )
    .unwrap();
    let full = scan(&replay, &all_params(), None, &AtomicBool::new(false)).unwrap();
    assert_eq!(full.traits.len(), 2);
    let reply = project(
        &full,
        &UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: vec!["selected country".into()],
            },
            ..all_params()
        },
        None,
    );
    assert_eq!(reply.traits.len(), 1);
    assert_eq!(reply.traits[0].token, "selected_trait");
    assert_eq!(reply.leaders.len(), 1);
    assert_eq!(
        reply.leaders[0].runs[0].value.traits.as_deref(),
        Some(&[0][..])
    );
    assert_eq!(reply.leaders[0].trait_gains[0].token, 0);
}

#[test]
fn dates_and_gaps_do_not_invent_gains() {
    use replay_data::{rows::TableRows, rows::UnitLeaderTraitRow};
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let frames = [
        (100, 2, Some(vec!["mod_trait"])),
        (101, 2, Some(vec!["mod_trait"])),
        (102, 3, Some(vec!["mod_trait", "extra"])),
        (105, 4, None),
        (106, 4, Some(vec!["mod_trait", "extra", "unknown_date"])),
    ]
    .into_iter()
    .map(|(hour, skill, traits)| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 11,
                kind: 1,
                category: 1,
                skill_status: 2,
                skill_level: skill,
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] = traits
            .map(|traits| {
                TableView::Complete(TableRows::UnitLeaderTraits(
                    traits
                        .into_iter()
                        .enumerate()
                        .map(|(ordinal, token)| UnitLeaderTraitRow {
                            leader_reference: reference,
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ))
            })
            .unwrap_or_else(|| TableView::Unavailable {
                reason: "trait read missing".into(),
            });
        frame
    })
    .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(&replay, &all_params(), None, &AtomicBool::new(false)).unwrap();
    let leader = &reply.leaders[0];
    assert_eq!(leader.runs[0].last_hour, 101);
    assert_eq!(leader.runs[2].first_hour, 105);
    assert_eq!(leader.trait_gains.len(), 3);
    assert_eq!(leader.trait_gains[0].date, LeaderChangeDate::HeldBefore);
    assert_eq!(leader.trait_gains[1].date, LeaderChangeDate::Gained);
    assert_eq!(leader.trait_gains[2].date, LeaderChangeDate::FirstObserved);
}

#[test]
fn skill_ceiling_follows_every_defined_level() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let levels = (1..=5000)
        .map(|level| format!("{level}={{type=corps_commander cost=1}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("leader_skills={{{levels}}}");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "common/unit_leader/mod.txt",
        &text,
    )]);
    let leaders: installed_content_core::unit_leader::Leaders =
        installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None).unwrap();
    assert_eq!(maximum_skill(&leaders.skills, 1, 1), Some(5000));
}

#[test]
fn captured_hidden_and_unresolved_visibility_are_withheld() {
    let mut row = UnitLeaderRow {
        kind: 1,
        category: 1,
        captured: 1,
        ..Default::default()
    };
    let visible = |row: &UnitLeaderRow| visible(row, None, &mut None, None, true);
    assert_eq!(visible(&row), Ok(false));
    row.category = -1; // Captured categories compare as unsigned.
    assert_eq!(visible(&row), Ok(true));
    row.category = 1;
    row.captured = 0;
    assert_eq!(visible(&row), Ok(true));
    row.visibility_choice = 1;
    assert!(visible(&row).is_err());
    assert_ne!(
        handle(&row),
        handle(&UnitLeaderRow {
            reference_type: 99,
            ..row
        })
    );
}

/// A read visibility needs the hour's World only for `visible` conditions,
/// an unlinked owner or changed inputs; the core tables still refuse first.
#[test]
fn read_leader_visibility_reads_the_world_only_to_evaluate() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    fn hour<'a>(content: &'a ScriptedContent, frame: &'a DecodedFrame) -> HourWorld<'a> {
        HourWorld {
            content,
            frame,
            world: OnceCell::from(Err("the World is read".to_owned())),
            gap: OnceCell::new(),
            kept: None,
        }
    }
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "common/defines/test.lua",
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )]);
    let content = catalogue.scripted_content().unwrap();
    let row = UnitLeaderRow {
        kind: 1,
        category: 1,
        character_id: 9,
        template_visibility: 1,
        ..Default::default()
    };
    let read = |visibility| {
        Some(VisibilityRead {
            key: (Some(1), 9, 0, 1),
            visibility,
            noted: false,
        })
    };
    let refused = |why: &str| Err::<bool, _>(Some(why.to_owned()));
    let mut frame = dashboard_core::test_fixtures::source(1, 100, None);
    assert_eq!(
        visible(
            &row,
            Some(&hour(content, &frame)),
            &mut read(LeaderVisibility::Unconditioned),
            Some(1),
            true
        ),
        refused("Provinces is unavailable: outside country fixture")
    );
    for table in [TableId::Provinces, TableId::States] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let hour = hour(content, &frame);
    let visible = |read: &mut Option<VisibilityRead>, epoch, linked| {
        visible(&row, Some(&hour), read, epoch, linked)
    };
    assert_eq!(
        visible(&mut read(LeaderVisibility::Unconditioned), Some(1), true),
        Ok(true)
    );
    for (visibility, epoch, linked) in [
        (LeaderVisibility::Conditioned(0), Some(1), true),
        (LeaderVisibility::Unconditioned, Some(1), false),
        (LeaderVisibility::Unconditioned, Some(2), true),
        (LeaderVisibility::Unconditioned, None, true),
    ] {
        assert_eq!(
            visible(&mut read(visibility), epoch, linked),
            refused("the World is read")
        );
    }
    // Refusals are a set: a read unknown is noted once.
    let mut unknown = read(LeaderVisibility::Decided(Eval::unknown("refused role")));
    assert_eq!(
        visible(&mut unknown, Some(1), true),
        refused("refused role")
    );
    assert_eq!(visible(&mut unknown, Some(1), true), Err(None));
}

#[test]
fn missing_command_name_is_unknown_without_fragmenting_the_command() {
    let mut l = LeaderState::new(5);
    append_command(
        &mut l,
        70,
        LeaderFormationKind::Army,
        Some("Named army"),
        100,
    );
    append_command(&mut l, 70, LeaderFormationKind::Army, None, 101);
    append_command(
        &mut l,
        70,
        LeaderFormationKind::Army,
        Some("Named army"),
        102,
    );
    assert_eq!(l.career.commands.len(), 1);
    assert_eq!(l.career.commands[0].last_hour, 102);
    assert_eq!(l.career.commands[0].name_read_hour, Some(100));
    assert_eq!(
        l.career.commands[0].name_unknown,
        vec![HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
}

#[test]
fn coverage_tracks_each_leaders_entry_hidden_reads_and_own_gaps() {
    use replay_data::{rows::TableRows, rows::UnitLeaderTraitRow};
    use replay_frame::frame::TableView;
    let frames = (100..106)
        .map(|hour| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
            let mut rows = vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 9999,
                kind: 1,
                category: 1,
                ..Default::default()
            }];
            if hour >= 101 && hour != 103 {
                rows.push(UnitLeaderRow {
                    id: 6,
                    reference_type: 4713,
                    owner: 11,
                    kind: 1,
                    category: 1,
                    ..Default::default()
                });
            }
            rows.push(UnitLeaderRow {
                id: 7,
                reference_type: 4713,
                owner: 11,
                kind: 1,
                category: 1,
                captured: u8::from(hour == 100),
                ..Default::default()
            });
            *frame.tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(rows));
            *frame.tables[TableId::UnitLeaderTraits.index()] =
                TableView::Complete(TableRows::UnitLeaderTraits(Vec::<UnitLeaderTraitRow>::new()));
            *frame.tables[TableId::LandCombatDetails.index()] =
                TableView::Complete(TableRows::LandCombatDetails(vec![]));
            *frame.tables[TableId::NavalCombatDetails.index()] =
                TableView::Complete(TableRows::NavalCombatDetails(vec![]));
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(
        &replay,
        &UnitLeadersParams {
            branch_id: "1".into(),
            selection: LeaderSelection::All,
            view: LeaderView::Leaderboard,
            leaders: LeaderKinds::All,
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.leaders[0].partial_from, None);
    assert_eq!(reply.leaders[1].partial_from, Some(104)); // Exact entry at 101; only his own later gap changes it.
    assert_eq!(reply.leaders[2].partial_from, None); // A hidden pre-recording holding does not hide recorded totals.
}

#[test]
fn scan_uses_layered_trait_skill_and_localisation_override_of_rank_key() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{
        rows::OrdersGroupPlanRow, rows::OrdersGroupRow, rows::TableRows, rows::UnitLeaderTraitRow,
    };
    use replay_frame::frame::TableView;
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/00.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/a.txt",
            "leader_skills={1={type=corps_commander cost=1} 2={type=corps_commander cost=2}} leader_traits={original={}}",
        ),
        (
            "localisation/base_l_english.yml",
            "\u{feff}l_english:\n cc_leader:0 \"Ground Captain\"\n",
        ),
    ]);
    let addon = folder.0.join("addon");
    std::fs::create_dir_all(addon.join("common/unit_leader")).unwrap();
    std::fs::create_dir_all(addon.join("localisation/replace")).unwrap();
    std::fs::write(
        addon.join("common/unit_leader/b.txt"),
        "leader_skills={3={type=corps_commander cost=3}} leader_traits={orbital_tactician={}}",
    )
    .unwrap();
    std::fs::write(addon.join("localisation/replace/mod_l_english.yml"), "\u{feff}l_english:\n cc_leader:0 \"Star Commander\"\n orbital_tactician:0 \"Orbital Tactician\"\n SKILL_ATTACK:0 \"Mod Attack\"\n SKILL_DEFENSE:0 \"Mod Defence\"\n SKILL_PLANNING:0 \"Mod Planning\"\n SKILL_LOGISTICS:0 \"Mod Logistics\"\n").unwrap();
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[]), (&addon, &[])]).unwrap();
    let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
    let content = Content {
        politics: Arc::new(politics_content::PoliticsContent::load(catalogue.clone()).unwrap()),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let mut frame = dashboard_core::test_fixtures::source(1, 100, None);
    let reference = (5_u64 << 32) | 4713;
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 5,
            reference_type: 4713,
            owner: 11,
            kind: 1,
            category: 1,
            skill_status: 2,
            skill_level: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaderTraits.index()] =
        TableView::Complete(TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
            leader_reference: reference,
            token: b"orbital_tactician".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 70,
            leader: reference,
            name: replay_data::rows::Recorded::Value(Some(b"Exact recorded formation".to_vec())),
            ..Default::default()
        }]));
    *frame.tables[TableId::OrdersGroupPlans.index()] =
        TableView::Complete(TableRows::OrdersGroupPlans(vec![OrdersGroupPlanRow {
            group: 70,
            name: b"Exact recorded formation".to_vec(),
            ..Default::default()
        }]));
    let frames = (100..103)
        .map(|hour| {
            let mut read = frame.clone();
            read.hour = hour;
            if hour == 101 {
                if let TableView::Complete(TableRows::OrdersGroups(rows)) =
                    &mut *read.tables[TableId::OrdersGroups.index()]
                {
                    rows[0].name = replay_data::rows::Recorded::Value(None);
                }
                *read.tables[TableId::OrdersGroupPlans.index()] = TableView::Unavailable {
                    reason: "missing formation name read".into(),
                };
            }
            read
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(
        &replay,
        &UnitLeadersParams {
            branch_id: "1".into(),
            selection: LeaderSelection::Leader {
                reference: reference.to_string(),
            },
            view: LeaderView::Career,
            leaders: LeaderKinds::All,
        },
        Some(&content),
        &AtomicBool::new(false),
    )
    .unwrap();
    let role = &reply.roles[reply.leaders[0].runs[0].value.role as usize];
    assert_eq!(role.maximum_skill, Some(3));
    assert_eq!(role.rank.as_deref(), Some("Star Commander"));
    assert_eq!(role.attribute_names[0].as_deref(), Some("Mod Attack"));
    assert_eq!(reply.traits[0].name.as_deref(), Some("Orbital Tactician"));
    assert_eq!(
        reply.leaders[0].runs[0].value.traits.as_deref(),
        Some(&[0][..])
    );
    let commands = &reply.leaders[0].commands;
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].name.as_deref(),
        Some("Exact recorded formation")
    );
    assert_eq!(commands[0].name_read_hour, Some(100));
    assert_eq!(commands[0].last_hour, 102);
    assert_eq!(
        commands[0].name_unknown,
        vec![HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
}

#[test]
fn transport_budget_refuses_without_truncating_and_selection_has_no_count_cap() {
    let mut reply = scan_fixture(2, 4, 3, false);
    let leaders = reply.leaders.len();
    assert_eq!(leaders, 8);
    assert!(reply_bytes(&reply).is_ok());
    reply.warnings.push("x".repeat(MAX_REPLY_BYTES));
    assert!(
        reply_bytes(&reply)
            .unwrap_err()
            .to_string()
            .contains("transport budget")
    );
    assert_eq!(reply.leaders.len(), leaders);
    let replay = Arc::new(
        Replay::from_records(
            b"analytics-test".to_vec(),
            &[dashboard_core::test_fixtures::records(
                1,
                vec![dashboard_core::test_fixtures::source(1, 100, None)],
            )],
        )
        .unwrap(),
    );
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "leader-cache".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let cache = source.state::<Cache>();
    let slot = cache.lock().unwrap().branch(None, "1");
    slot.state.lock().unwrap().scan = Some(Ok(Arc::new(reply)));
    let params = all_params();
    let context = LoadContext { content: None };
    let first = UnitLeaders::load(&source, &params, &context, &AtomicBool::new(false)).unwrap_err();
    assert!(first.to_string().contains("transport budget"));
    // A retry must reuse the refusal, even when a fresh projection would now fit.
    slot.state.lock().unwrap().scan = Some(Ok(Arc::new(empty_reply())));
    let retry = UnitLeaders::load(&source, &params, &context, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(retry.to_string(), first.to_string());
    assert!(
        UnitLeaders::validate(&UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: (0..5000)
                    .map(|n| format!("country with spaces {n}"))
                    .collect()
            },
            ..all_params()
        })
        .is_ok()
    );
}

#[test]
#[ignore = "manual large reply-size measurement; routine tests use a smaller scan fixture"]
fn transport_budget_counts_1500_countries_and_refuses_oversize() {
    let reply = scan_fixture(1500, 4, 50, false);
    assert_eq!(reply.leaders.len(), 6000);
    assert!(reply.leaders.iter().all(|leader| leader.runs.len() == 50));
    assert!(report_size("6000 leaders / 1500 countries / 50 read runs", &reply) < MAX_REPLY_BYTES);
}

#[test]
#[ignore = "manual large combat reply-size measurement"]
fn combat_heavy_1500_country_fixture_fits_the_transport_budget() {
    let reply = scan_fixture(1500, 4, 50, true);
    assert_eq!(reply.leaders.len(), 6000);
    assert!(
        reply
            .leaders
            .iter()
            .all(|leader| leader.battle_starts.as_ref().map(Vec::len) == Some(50))
    );
    assert!(reply_bytes(&reply).is_ok());
    report_size("6000 leaders / 50 fights each", &reply);
}

#[test]
fn naval_commands_join_the_roster_fleet_and_keep_observation_gaps() {
    use replay_data::{rows::NavalMissionTaskForceRow, rows::TableRows, rows::TaskForceRow};
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let make = |hour| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                kind: 2,
                category: 2,
                owner: 11,
                ..Default::default()
            }]));
        *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
            (100..102)
                .map(|id| TaskForceRow {
                    id,
                    fleet: 200,
                    ..Default::default()
                })
                .collect(),
        ));
        let missions = TableRows::NavalMissionTaskForces(
            (100..102)
                .map(|task_force| NavalMissionTaskForceRow {
                    task_force,
                    leader: if hour == 105 && task_force == 101 {
                        77
                    } else {
                        reference
                    },
                    range_fleet: 999,
                    ..Default::default()
                })
                .collect(),
        );
        *frame.tables[TableId::NavalMissionTaskForces.index()] = if hour == 102 {
            TableView::Truncated {
                rows: missions,
                missing: 1,
            }
        } else {
            TableView::Complete(missions)
        };
        frame
    };
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(
            1,
            (100..106).map(make).collect(),
        )],
    )
    .unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        view: LeaderView::Career,
        leaders: LeaderKinds::All,
        selection: LeaderSelection::Leader {
            reference: reference.to_string(),
        },
    };
    let reply = scan(&replay, &params, None, &AtomicBool::new(false)).unwrap();
    let commands = &reply.leaders[0].commands;
    assert_eq!(commands.len(), 2);
    assert_eq!((commands[0].first_hour, commands[0].last_hour), (100, 101));
    assert_eq!((commands[1].first_hour, commands[1].last_hour), (103, 104));
    assert_eq!(
        reply
            .observed
            .iter()
            .map(|read| (read.first_hour, read.last_hour, read.naval_commands))
            .collect::<Vec<_>>(),
        vec![
            (100, 101, true),
            (102, 102, false),
            (103, 104, true),
            (105, 105, false)
        ]
    );
    assert!(commands.iter().all(|command| command.formation == "200"
        && command.kind == LeaderFormationKind::Fleet
        && command.name.is_none()));
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("Conflicting recorded fleet commander"))
    );
    let leaderboard = scan(
        &replay,
        &UnitLeadersParams {
            view: LeaderView::Leaderboard,
            ..params
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(leaderboard.leaders[0].commands.is_empty());
}

#[test]
fn requests_share_only_the_portrait_size_the_view_draws() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use base64::Engine;
    use replay_data::{rows::CharacterPortrait, rows::TableRows};
    use replay_frame::frame::TableView;
    let mut tga = vec![0_u8; 18];
    tga[2] = 2;
    tga[12] = 128;
    tga[14] = 172;
    tga[16] = 24;
    tga[17] = 32;
    tga.extend([24, 48, 72].repeat(128 * 172));
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/00.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/mod.txt",
            "leader_skills={1={type=corps_commander cost=1} 2={type=corps_commander cost=2} 3={type=corps_commander cost=3}} leader_traits={orbital_tactician={}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n cc_leader:0 \"Star Commander\"\n orbital_tactician:0 \"Orbital Tactician\"\n SKILL_ATTACK:0 \"Mod Attack\"\n SKILL_DEFENSE:0 \"Mod Defence\"\n SKILL_PLANNING:0 \"Mod Planning\"\n SKILL_LOGISTICS:0 \"Mod Logistics\"\n",
        ),
        (
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_mod_portrait texturefile=\"gfx/mod.tga\"} spriteType={name=GFX_trait_orbital_tactician texturefile=\"gfx/mod.tga\"}}",
        ),
    ]);
    std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
    std::fs::write(folder.0.join("gfx/mod.tga"), tga).unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder.0, &[])]).unwrap();
    let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
    let content = Content {
        politics: Arc::new(politics_content::PoliticsContent::load(catalogue.clone()).unwrap()),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let mut frame = dashboard_core::test_fixtures::source(1, 100, None);
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(
        (0..2)
            .map(|n| UnitLeaderRow {
                id: 5 + n,
                reference_type: 4713,
                character_id: 9 + n,
                owner: 11,
                category: 1,
                kind: 1,
                skill_status: 2,
                skill_level: 1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::UnitLeaderTraits.index()] =
        TableView::Complete(TableRows::UnitLeaderTraits(vec![
            replay_data::rows::UnitLeaderTraitRow {
                leader_reference: (5_u64 << 32) | 4713,
                token: b"orbital_tactician".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CharacterPortraits.index()] =
        TableView::Complete(TableRows::CharacterPortraits(
            (0..2)
                .map(|n| CharacterPortraitRow {
                    character: 9 + n,
                    portraits: vec![CharacterPortrait {
                        category: PORTRAIT_ARMY,
                        size: replay_data::rows::PORTRAIT_LARGE,
                        is_set: 1,
                        kind: 0,
                        identity: b"GFX_mod_portrait".to_vec(),
                    }],
                })
                .collect(),
        ));
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, vec![frame])],
    )
    .unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::All,
        view: LeaderView::Leaderboard,
        leaders: LeaderKinds::All,
    };
    let reply = scan(&replay, &params, Some(&content), &AtomicBool::new(false)).unwrap();
    assert!(
        serde_json::to_value(&reply)
            .unwrap()
            .get("portraits")
            .is_none()
    );
    assert_eq!(reply.people[0].portrait_hour, 100);
    let replay::container::SeekResult::Frame(frame) = replay.seek(1, 100).unwrap() else {
        panic!("frame");
    };
    for (edge, expected) in [(86, (64, 86)), (118, (88, 118))] {
        let image =
            portraits::artwork(&frame, (5_u64 << 32) | 4713, &content.politics, edge).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image.split_once(',').unwrap().1)
            .unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        assert_eq!((reader.info().width, reader.info().height), expected);
    }
}

fn empty_reply() -> UnitLeadersReply {
    UnitLeadersReply {
        branch_id: "1".into(),
        first_hour: 100,
        leaders: vec![],
        people: vec![],
        roles: vec![],
        traits: vec![],
        observed: vec![],
        selection_tags: None,
        preparing: Vec::new(),
        results_missing: RESULTS_MISSING.into(),
        navy_commands_missing: NAVY_COMMANDS_MISSING.into(),
        warnings: vec![],
    }
}

fn all_params() -> UnitLeadersParams {
    UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::All,
        view: LeaderView::Leaderboard,
        leaders: LeaderKinds::All,
    }
}

fn scan_fixture(
    country_count: u32,
    per_country: u32,
    periods: i32,
    fights: bool,
) -> UnitLeadersReply {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::LandCombatDetailsRow,
        rows::LandCombatSideDetails, rows::TableRows, rows::UnitLeaderTraitRow,
    };
    use replay_frame::frame::TableView;
    let start = 60_826_800;
    let frames = (0..periods)
        .map(|period| {
            let hour = start + period * (9 * 365 * 24 / periods);
            let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=country_count)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("mod country {id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    (1..=country_count)
                        .map(|id| CountryLinkRow {
                            id,
                            canonical_slot: id,
                        })
                        .collect(),
                ));
            *frame.tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(
                    (1..=country_count * per_country)
                        .map(|id| UnitLeaderRow {
                            id,
                            reference_type: 4713,
                            owner: (id - 1) / per_country + 1,
                            kind: 1,
                            category: 1,
                            character_id: id,
                            skill_status: 2,
                            skill_level: 3 + period % 3,
                            components: [period, 2, 3, 4],
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::UnitLeaderTraits.index()] =
                TableView::Complete(TableRows::UnitLeaderTraits(
                    (1..=country_count * per_country)
                        .map(|id| UnitLeaderTraitRow {
                            leader_reference: (u64::from(id) << 32) | 4713,
                            token: b"mod_trait".to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::LandCombatDetails.index()] =
                TableView::Complete(TableRows::LandCombatDetails(if fights {
                    (1..=country_count * per_country)
                        .map(|id| LandCombatDetailsRow {
                            combat: ((period as u64 + 1) << 32) | u64::from(id),
                            sides: [
                                LandCombatSideDetails {
                                    commander: (u64::from(id) << 32) | 4713,
                                    ..Default::default()
                                },
                                LandCombatSideDetails::default(),
                            ],
                            ..Default::default()
                        })
                        .collect()
                } else {
                    Vec::new()
                }));
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    scan(&replay, &all_params(), None, &AtomicBool::new(false)).unwrap()
}

fn history_items(reply: &UnitLeadersReply) -> usize {
    reply.leaders.len()
        + reply.people.len()
        + reply.roles.len()
        + reply.observed.len()
        + reply
            .leaders
            .iter()
            .map(|leader| {
                leader.runs.len()
                    + leader.trait_gains.len()
                    + leader.commands.len()
                    + leader.battle_starts.as_ref().map_or(0, Vec::len)
            })
            .sum::<usize>()
}

/// Disjoint wire-byte accounting, including JSON syntax in each named partition.
fn report_size(label: &str, reply: &UnitLeadersReply) -> usize {
    let size = |value: &serde_json::Value| serde_json::to_vec(value).unwrap().len();
    let wire = serde_json::to_value(reply).unwrap();
    let mut identity = size(&wire["people"]) + size(&wire["roles"]);
    let mut histories = [0; 4];
    for leader in wire["leaders"].as_array().unwrap() {
        let mut parts = 0;
        for (i, key) in ["runs", "traitGains", "commands", "battleStarts"]
            .into_iter()
            .enumerate()
        {
            let bytes = size(&leader[key]);
            histories[i] += bytes;
            parts += bytes;
        }
        identity += size(leader) - parts;
    }
    let portraits = 0;
    let traits = size(&wire["traits"]);
    let total = dashboard_core::transport::json_len(reply, usize::MAX).unwrap();
    let envelope = total - identity - histories.iter().sum::<usize>() - portraits - traits;
    eprintln!(
        "{label}: total={total}, identity/roles={identity}, runs={}, trait gains={}, commands={}, combats={}, portraits={portraits}, trait catalogue={traits}, envelope/coverage={envelope}",
        histories[0], histories[1], histories[2], histories[3]
    );
    eprintln!(
        "{label}: leaders={}, runs={}, trait gains={}, commands={}, battles={}, observed spans={}, items={}",
        reply.leaders.len(),
        reply
            .leaders
            .iter()
            .map(|leader| leader.runs.len())
            .sum::<usize>(),
        reply
            .leaders
            .iter()
            .map(|leader| leader.trait_gains.len())
            .sum::<usize>(),
        reply
            .leaders
            .iter()
            .map(|leader| leader.commands.len())
            .sum::<usize>(),
        reply
            .leaders
            .iter()
            .map(|leader| leader.battle_starts.as_ref().map_or(0, Vec::len))
            .sum::<usize>(),
        reply.observed.len(),
        history_items(reply),
    );
    total
}

#[test]
fn shared_trait_indices_keep_null_empty_order_and_compact_wire() {
    for traits in [None, Some(vec![]), Some(vec![0, u32::MAX, 0])] {
        let values = LeaderValues {
            person: 2,
            role: 3,
            skill: Some(4),
            components: [5, 6, 7, 8],
            progress: None,
            traits: traits.clone().map(Arc::from),
        };
        let wire = serde_json::to_value(&values).unwrap();
        assert_eq!(wire["traits"], serde_json::to_value(&traits).unwrap());
        assert_eq!(
            serde_json::from_value::<LeaderValues>(wire.clone()).unwrap(),
            values
        );
        if traits.is_none() {
            let mut without_traits = wire;
            without_traits.as_object_mut().unwrap().remove("traits");
            assert_eq!(
                serde_json::from_value::<LeaderValues>(without_traits).unwrap(),
                values
            );
        }
        let run = UnitLeaderRun {
            first_hour: 100,
            last_hour: 101,
            value: values,
        };
        let wire = serde_json::to_value(&run).unwrap();
        assert_eq!(wire[6], serde_json::to_value(&traits).unwrap());
        assert_eq!(serde_json::from_value::<UnitLeaderRun>(wire).unwrap(), run);
    }
}

#[test]
fn compact_wire_retains_scan_runs_traits_and_recorded_battles() {
    let reply = scan_fixture(1, 2, 9, true);
    assert_eq!(reply.leaders.len(), 2);
    for leader in &reply.leaders {
        assert_eq!(leader.runs.len(), 9);
        assert_eq!(leader.trait_gains.len(), 1);
        let starts = leader.battle_starts.as_deref().unwrap();
        assert_eq!(starts.len(), 9);
        assert_eq!(
            starts.iter().sum::<i64>(),
            i64::from(leader.runs.last().unwrap().first_hour)
        );
    }
    assert!(report_size("2 leaders / nine yearly recorded reads", &reply) < 60 * 1024);
    let wire = serde_json::to_value(&reply).unwrap();
    assert!(wire["leaders"][0]["runs"][0].is_array());
    assert!(wire["leaders"][0].get("battleWon").is_none());
}

#[test]
fn cache_retains_recent_branches_replies_and_transport_refusals() {
    let mut cache = Cache::default();
    let first = cache.branch(None, "1");
    assert!(Arc::ptr_eq(&first, &cache.branch(None, "1")));
    for branch in ["2", "3"] {
        cache.branch(None, branch);
    }
    assert_eq!(cache.branches.len(), CACHED_BRANCHES);
    assert!(!Arc::ptr_eq(&first, &cache.branch(None, "1")));
    let reply = Arc::new(empty_reply());
    for reference in 0..CACHED_REPLIES + 1 {
        let params = UnitLeadersParams {
            selection: LeaderSelection::Leader {
                reference: reference.to_string(),
            },
            ..all_params()
        };
        first.remember(&params, Ok(Arc::clone(&reply)));
    }
    assert_eq!(first.state.lock().unwrap().replies.len(), CACHED_REPLIES);
    assert!(
        first
            .reply(&UnitLeadersParams {
                selection: LeaderSelection::Leader {
                    reference: "0".into()
                },
                ..all_params()
            })
            .is_none()
    );
    let params = all_params();
    first.remember(
        &params,
        Err((
            io::ErrorKind::Other,
            "Leader reply exceeds its transport budget".into(),
        )),
    );
    assert!(
        recalled(first.reply(&params).unwrap())
            .unwrap_err()
            .to_string()
            .contains("transport budget")
    );
}

#[test]
fn response_warnings_keep_cached_history_shared_and_wire_fields_unchanged() {
    use dashboard_base::products::ReplyWarnings;
    let history = Arc::new(scan_fixture(1, 2, 3, true));
    let mut response = UnitLeadersResponse::from(Arc::clone(&history));
    let mut expected = serde_json::to_value(&*history).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
    let warning = "Content read is not hash-verified";
    response.warnings_mut().push(warning.into());
    assert!(Arc::ptr_eq(&history, &response.history));
    assert!(!history.warnings.iter().any(|line| line == warning));
    expected["warnings"] = serde_json::to_value(&response.warnings).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
}

#[test]
fn concurrent_requests_share_one_scan_and_waiters_can_cancel() {
    use std::sync::mpsc;
    let slot = Arc::new(BranchSlot::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_slot = Arc::clone(&slot);
    let worker = std::thread::spawn(move || {
        worker_slot
            .scanned(&AtomicBool::new(false), || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(empty_reply())
            })
            .unwrap()
    });
    started_rx.recv().unwrap();
    assert!(
        slot.state.try_lock().is_ok(),
        "scan does not hold the slot lock"
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let waiter_slot = Arc::clone(&slot);
    let waiter_cancellation = Arc::clone(&cancellation);
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        waiting_tx.send(()).unwrap();
        let result = waiter_slot.scanned(&waiter_cancellation, || panic!("waiter must not scan"));
        finished_tx.send(result.unwrap_err().kind()).unwrap();
    });
    waiting_rx.recv().unwrap();
    cancellation.store(true, Ordering::Release);
    assert_eq!(finished_rx.recv().unwrap(), io::ErrorKind::Interrupted);
    waiter.join().unwrap();
    release_tx.send(()).unwrap();
    let scanned = worker.join().unwrap();
    let reused = slot
        .scanned(&AtomicBool::new(false), || {
            panic!("cached scan must be shared")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&scanned, &reused));
}

#[test]
fn cancelled_branch_scan_is_retried_instead_of_cached() {
    let slot = BranchSlot::default();
    let cancelled = slot.scanned(&AtomicBool::new(false), || {
        Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"))
    });
    assert_eq!(cancelled.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert!(
        slot.scanned(&AtomicBool::new(false), || Ok(empty_reply()))
            .is_ok()
    );
}

#[test]
fn projections_share_a_branch_scan_and_remap_selected_histories() {
    let full = scan_fixture(2, 2, 3, true);
    let selected = project(
        &full,
        &UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: vec!["mod country 2".into()],
            },
            ..all_params()
        },
        None,
    );
    assert_eq!(selected.leaders.len(), 2);
    assert_eq!(selected.roles.len(), 1);
    assert_eq!(selected.roles[0].owner.as_deref(), Some("mod country 2"));
    assert!(selected.leaders.iter().all(|leader| leader.runs.len() == 3
        && leader.battle_starts.is_none()
        && leader.fought_hours.as_ref().map(Vec::len) == Some(3)));
    assert!(selected.leaders.iter().all(|leader| {
        leader
            .runs
            .iter()
            .all(|run| (run.value.person as usize) < selected.people.len() && run.value.role == 0)
    }));
    let choices = project(
        &full,
        &UnitLeadersParams {
            view: LeaderView::Choices,
            ..all_params()
        },
        None,
    );
    assert_eq!(choices.leaders.len(), 4);
    assert!(choices.traits.is_empty());
    assert!(choices.leaders.iter().all(|leader| leader.runs.len() == 1
        && leader.runs[0].first_hour == leader.runs[0].last_hour
        && leader.battle_starts.is_none()
        && leader.fought_hours.is_none()));
}

/// Raw battles as the scan keeps them: unknown starts before the recording,
/// shared hours, delayed reads, undecided battles and results read before or
/// after their participation.
fn record_battles(leader: &mut LeaderCareer, first_hour: i32, count: u32) {
    let mut previous = 0_i64;
    let (mut starts, mut reads, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
    let (mut won, mut result_hours, mut result_reads) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..count {
        let x = i.wrapping_mul(2_654_435_761) >> 13;
        let start = first_hour - 10 + 20 * (i / 2) as i32;
        let result = (x % 5 != 0).then_some(start + (x % 11) as i32);
        starts.push(i64::from(start) - previous);
        previous = i64::from(start);
        reads.push(start.max(first_hour) + (x % 4) as i32 * 9);
        if start < first_hour {
            unknown.push(i);
        }
        won.push(result.map(|_| x % 2 == 0));
        result_hours.push(result);
        result_reads.push(result.map(|hour| hour.max(first_hour) + (x % 3) as i32));
    }
    leader.battle_starts = Some(starts);
    leader.battle_start_read_hours = Some(reads);
    leader.unknown_battle_starts = Some(unknown);
    leader.battle_won = Some(won);
    leader.battle_result_hours = Some(result_hours);
    leader.battle_result_read_hours = Some(result_reads);
}

#[test]
fn leaderboard_hours_give_the_career_counts_at_every_hour() {
    fn through(hours: &[i64], hour: i64) -> usize {
        hours.iter().filter(|&&at| at <= hour).count()
    }
    let mut scanned = scan_fixture(1, 1, 1, false);
    let first_hour = scanned.first_hour;
    record_battles(&mut scanned.leaders[0], first_hour, 400);
    let career = scanned.leaders[0].clone();
    let reference = career.reference.clone();
    let career_params = UnitLeadersParams {
        selection: LeaderSelection::Leader { reference },
        view: LeaderView::Career,
        ..all_params()
    };
    assert_eq!(project(&scanned, &career_params, None).leaders[0], career);
    let board = project(&scanned, &all_params(), None).leaders.remove(0);
    assert_eq!(
        board,
        LeaderCareer {
            battle_starts: None,
            battle_start_read_hours: None,
            unknown_battle_starts: None,
            battle_won: None,
            battle_result_hours: None,
            battle_result_read_hours: None,
            fought_hours: board.fought_hours.clone(),
            won_hours: board.won_hours.clone(),
            lost_hours: board.lost_hours.clone(),
            ..career.clone()
        }
    );
    let decoded = |deltas: &Option<Vec<i64>>| {
        let mut hour = 0_i64;
        let hours: Vec<i64> = deltas
            .as_deref()
            .unwrap()
            .iter()
            .map(|delta| {
                hour += delta;
                hour
            })
            .collect();
        assert!(hours.is_sorted());
        hours
    };
    let [fought, won, lost] =
        [&board.fought_hours, &board.won_hours, &board.lost_hours].map(decoded);
    // The career's counts: a participation shows from the later of its date and
    // first read; a result also waits for its own date and read.
    let mut start = 0_i64;
    let shown: Vec<i64> = career
        .battle_starts
        .as_deref()
        .unwrap()
        .iter()
        .zip(career.battle_start_read_hours.as_deref().unwrap())
        .map(|(delta, read)| {
            start += delta;
            start.max(i64::from(*read))
        })
        .collect();
    let outcomes = career.battle_won.as_deref().unwrap();
    let result_hours = career.battle_result_hours.as_deref().unwrap();
    let result_reads = career.battle_result_read_hours.as_deref().unwrap();
    let last = *fought.iter().chain(&won).chain(&lost).max().unwrap();
    for hour in i64::from(first_hour) - 1..=last + 1 {
        let decided = |outcome: bool| {
            (0..shown.len())
                .filter(|&i| {
                    outcomes[i] == Some(outcome)
                        && shown[i]
                            .max(i64::from(result_hours[i].unwrap()))
                            .max(i64::from(result_reads[i].unwrap()))
                            <= hour
                })
                .count()
        };
        assert_eq!(
            [
                through(&fought, hour),
                through(&won, hour),
                through(&lost, hour)
            ],
            [through(&shown, hour), decided(true), decided(false)],
            "hour {hour}"
        );
    }
    let leader = &mut scanned.leaders[0];
    (
        leader.battle_won,
        leader.battle_result_hours,
        leader.battle_result_read_hours,
    ) = (None, None, None);
    let unrecorded = project(&scanned, &all_params(), None).leaders.remove(0);
    assert_eq!(unrecorded.fought_hours, board.fought_hours);
    assert!(unrecorded.won_hours.is_none() && unrecorded.lost_hours.is_none());
}

#[test]
fn million_participation_leaderboard_fits_where_career_records_did_not() {
    let mut scanned = scan_fixture(1, 1, 1, false);
    let (first_hour, template) = (scanned.first_hour, scanned.leaders.remove(0));
    scanned.leaders = (1..=4000_u32)
        .map(|id| {
            let mut leader = LeaderCareer {
                reference: id.to_string(),
                ..template.clone()
            };
            record_battles(&mut leader, first_hour, 250);
            leader
        })
        .collect();
    // The former leaderboard sent these per-participation records unchanged.
    assert!(reply_bytes(&scanned).is_err());
    assert!(reply_bytes(&project(&scanned, &all_params(), None)).is_ok());
}

#[test]
fn country_selection_excludes_leaders_of_unrelated_alias_holders() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableRows};
    use replay_frame::frame::TableView;
    let frames = (100..=102)
        .map(|hour| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
            *frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![
                    CountryRow {
                        id: 10,
                        canonical_slot: 1,
                        tag: match hour {
                            100 => b"original".to_vec(),
                            101 => b"alias".to_vec(),
                            _ => b"current".to_vec(),
                        },
                        ..Default::default()
                    },
                    CountryRow {
                        id: 20,
                        canonical_slot: 2,
                        tag: if hour == 102 {
                            b"alias".to_vec()
                        } else {
                            b"other".to_vec()
                        },
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow {
                        id: 10,
                        canonical_slot: 1,
                    },
                    CountryLinkRow {
                        id: 20,
                        canonical_slot: 2,
                    },
                ]));
            *frame.tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(
                    [(5, 10), (6, 20)]
                        .into_iter()
                        .map(|(id, owner)| UnitLeaderRow {
                            id,
                            owner,
                            character_id: id,
                            reference_type: 4713,
                            kind: 1,
                            category: 1,
                            ..Default::default()
                        })
                        .collect(),
                ));
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let full = scan(&replay, &all_params(), None, &cancelled).unwrap();
    let history = dashboard_base::products::country_history::scan_histories(
        &replay,
        1,
        false,
        &[],
        &cancelled,
    )
    .unwrap();
    for (tag, count) in [("original", 1), ("alias", 2)] {
        let selected = project(
            &full,
            &UnitLeadersParams {
                selection: LeaderSelection::Countries {
                    tags: vec![tag.into()],
                },
                ..all_params()
            },
            Some(&history),
        );
        assert_eq!(selected.leaders.len(), count);
        assert_eq!(selected.leaders[0].runs.first().unwrap().first_hour, 100);
        assert_eq!(selected.leaders[0].runs.last().unwrap().last_hour, 102);
    }
}

#[test]
fn commands_have_coverage_even_when_the_recorded_inventory_is_empty() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let frames = (100..103)
        .map(|hour| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
            *frame.tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                    id: 5,
                    reference_type: 4713,
                    owner: 11,
                    kind: 1,
                    category: 1,
                    ..Default::default()
                }]));
            *frame.tables[TableId::OrdersGroups.index()] = if hour == 101 {
                TableView::Unavailable {
                    reason: "orders not read".into(),
                }
            } else {
                TableView::Complete(TableRows::OrdersGroups(vec![]))
            };
            *frame.tables[TableId::NavalMissionTaskForces.index()] =
                TableView::Complete(TableRows::NavalMissionTaskForces(vec![]));
            *frame.tables[TableId::TaskForces.index()] =
                TableView::Complete(TableRows::TaskForces(vec![]));
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[dashboard_core::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(
        &replay,
        &UnitLeadersParams {
            view: LeaderView::Career,
            ..all_params()
        },
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.leaders[0].commands.is_empty());
    assert!(reply.observed[0].land_commands);
    assert!(!reply.observed[1].land_commands);
    assert!(reply.observed[2].land_commands);
    assert!(reply.observed.iter().all(|read| read.naval_commands));
}

#[test]
fn repeated_outcome_keeps_first_supplying_read_and_conflicts_stay_unresolved() {
    let mut outcomes = BTreeMap::new();
    assert!(!record_outcome(&mut outcomes, (1, 7), (102, 104, 1)));
    assert!(!record_outcome(&mut outcomes, (1, 7), (102, 105, 1)));
    assert_eq!(outcomes[&(1, 7)], Some((102, 104, 1)));
    assert!(record_outcome(&mut outcomes, (1, 7), (102, 106, 2)));
    assert!(!record_outcome(&mut outcomes, (1, 7), (102, 107, 1)));
    assert_eq!(outcomes[&(1, 7)], None);
}

#[test]
fn every_scan_decodes_the_trigger_kit_its_visibility_reads() {
    for (career, choices) in [(true, false), (false, false), (false, true)] {
        let tables = scan_tables(career, choices);
        for table in dashboard_text::triggers::RECORDED_TABLES {
            assert!(tables.contains(table), "{table:?} is not decoded");
        }
    }
}

/// A leader's visibility evaluation is reused while every input it read
/// reads the same value, and evaluated again once one reads differently or
/// its owner changes, giving the same verdicts with check mode on (each
/// reused evaluation run anyway) and off (as in preparation).
#[test]
fn leader_visibility_evaluations_are_reused_until_an_input_reads_differently() {
    reused_leader_visibility(true);
    dashboard_text::triggers::inputs::reusing(|| reused_leader_visibility(false));
}

fn reused_leader_visibility(checking: bool) {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use dashboard_text::triggers::{ContentFacts, LeaderRoles};
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::ScriptFlagRow,
        rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "common/defines/test.lua",
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )]);
    let content = catalogue.scripted_content().unwrap();
    let names = installed_content_core::key_names::KeyNames::default();
    let roles = LeaderRoles::new(vec![installed_content_core::characters::LeaderTemplate {
        path: "common/characters/mod.txt".into(),
        tag: "mod_commander".into(),
        line: Some(4),
        role: Ok(Some(
            installed_content_core::triggers::read(b"has_country_flag = x", &names).items,
        )),
    }]);
    let triggers = installed_content_core::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::new(installed_content_core::tag_aliases::TagAliases::default());
    // Countries 101 and 102; flag x (token 900) set on 101.
    let mut flagged = DecodedFrame {
        branch: 1,
        hour: 100,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    for rows in [
        TableRows::Countries(
            [(101, 1, "AAA"), (102, 2, "BBB")]
                .into_iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 102,
                canonical_slot: 2,
            },
        ]),
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 900,
            literal: b"x".to_vec(),
        }]),
        TableRows::ScriptFlags(vec![ScriptFlagRow {
            scope_kind: 1,
            scope_id: 101,
            token: 900,
            value: 1,
            ..Default::default()
        }]),
    ] {
        let table = rows.table();
        *flagged.tables[table.index()] = TableView::Complete(rows);
    }
    // Later hours share every table they do not change.
    let mut cleared = flagged.clone();
    *cleared.tables[TableId::ScriptFlags.index()] =
        TableView::Complete(TableRows::empty(TableId::ScriptFlags));
    let at = |frame: &DecodedFrame, hour| DecodedFrame {
        hour,
        ..frame.clone()
    };
    let row = UnitLeaderRow {
        id: 1,
        reference_type: 4713,
        owner: 101,
        kind: 1,
        category: 1,
        character_id: 1,
        template_visibility: 1,
        ..Default::default()
    };
    let mut read = Some(VisibilityRead {
        key: (Some(1), 1, 0, 1),
        visibility: LeaderVisibility::Conditioned(0),
        noted: false,
    });
    let mut evaluation = None;
    let mut counts = ReuseCounts::default();
    // Each hour's frame, owner, verdict, and evaluations run and reused so far.
    let hours: [(DecodedFrame, u32, bool, u64, u64); 5] = [
        (at(&flagged, 100), 101, true, 1, 0),
        // Every input reads the same value: reused.
        (at(&flagged, 101), 101, true, 1, 1),
        // The flag is cleared, so its read differs: evaluated again.
        (at(&cleared, 102), 101, false, 2, 1),
        (at(&cleared, 103), 101, false, 2, 2),
        // Another owner is another evaluation.
        (at(&cleared, 104), 102, false, 3, 2),
    ];
    for (frame, owner, verdict, evaluated, reused) in hours {
        let world = World::read(
            &frame,
            ContentFacts {
                state_content: Ok(()),
                leader_roles: Ok(&roles),
                ..ContentFacts::content_only(&dlcs, &triggers, &aliases)
            },
        );
        let hour = HourWorld {
            content,
            frame: &frame,
            world: OnceCell::from(Ok(world)),
            gap: OnceCell::new(),
            kept: None,
        };
        let mut rereads = Rereads::new();
        let decided = visible_reusing(
            &UnitLeaderRow {
                owner,
                ..row.clone()
            },
            Some(&hour),
            &mut read,
            Some(Reuse {
                evaluation: &mut evaluation,
                rereads: &mut rereads,
                counts: &mut counts,
                since: None,
            }),
            Some(1),
            true,
        );
        assert_eq!(decided, Ok(verdict), "hour {}", frame.hour);
        assert_eq!(
            (counts.evaluated, counts.reused),
            (evaluated, reused),
            "hour {}",
            frame.hour
        );
    }
    let checked = if checking { counts.reused } else { 0 };
    assert_eq!(counts.checked, checked, "check mode runs every reuse");
    assert_eq!(counts.mismatched, 0);
}

/// An hour whose changes leave every input of a leader's visibility
/// evaluation unread reuses its verdict without building the hour's World
/// or reading any input again: political power and another country's flag
/// change, rows of tables the evaluation reads, in fields and rows it does
/// not. An hour changing the flag it reads evaluates again.
#[test]
fn leader_visibility_is_reused_without_a_world_when_only_unread_fields_change() {
    leader_visibility_from_changes(true);
    dashboard_text::triggers::inputs::reusing(|| leader_visibility_from_changes(false));
}

fn leader_visibility_from_changes(checking: bool) {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use dashboard_text::triggers::{ContentFacts, LeaderRoles, inputs::Since};
    use replay_data::{
        format::FormatVersion, rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow,
        rows::ScriptFlagRow, rows::ScriptVariablesRow, rows::TableRows,
    };
    use replay_frame::{frame::FrameEncoder, frame::TableView};
    let (catalogue, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[(
        "common/defines/test.lua",
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )]);
    let content = catalogue.scripted_content().unwrap();
    let names = installed_content_core::key_names::KeyNames::default();
    let roles = LeaderRoles::new(vec![installed_content_core::characters::LeaderTemplate {
        path: "common/characters/mod.txt".into(),
        tag: "mod_commander".into(),
        line: Some(4),
        role: Ok(Some(
            installed_content_core::triggers::read(b"has_country_flag = x", &names).items,
        )),
    }]);
    let triggers = installed_content_core::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::new(installed_content_core::tag_aliases::TagAliases::default());
    // Countries 101 and 102, with flag x (token 900) on 101 and y on 102.
    let frame = |hour, power, flag: i16, other: i16| {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, None);
        for rows in [
            TableRows::Countries(
                [(101, 1, "AAA"), (102, 2, "BBB")]
                    .into_iter()
                    .map(|(id, canonical_slot, tag)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.as_bytes().to_vec(),
                        political_power_raw: Some(power),
                        ..Default::default()
                    })
                    .collect(),
            ),
            TableRows::CountryLinks(
                [(101, 1), (102, 2)]
                    .into_iter()
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ),
            TableRows::empty(TableId::Provinces),
            TableRows::empty(TableId::States),
            // Visibility initializes the scope's variable seed even when this
            // condition does not draw a random value. Model complete stores
            // instead of generating a new hour-specific refusal.
            TableRows::ScriptVariables(
                [101, 102]
                    .map(|scope_id| ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id,
                        seed: replay_data::rows::Recorded::Value(Some(1)),
                        ..Default::default()
                    })
                    .into(),
            ),
            TableRows::LexicalTokens(
                [(900, "x"), (901, "y")]
                    .into_iter()
                    .map(|(token, literal)| LexicalTokenRow {
                        token,
                        literal: literal.as_bytes().to_vec(),
                    })
                    .collect(),
            ),
            TableRows::ScriptFlags(
                [(101, 900, flag), (102, 901, other)]
                    .into_iter()
                    .filter(|(_, _, value)| *value != 0)
                    .map(|(scope_id, token, value)| ScriptFlagRow {
                        scope_kind: 1,
                        scope_id,
                        token,
                        value,
                        ..Default::default()
                    })
                    .collect(),
            ),
        ] {
            let table = rows.table();
            *frame.tables[table.index()] = TableView::Complete(rows);
        }
        frame
    };
    let row = UnitLeaderRow {
        id: 1,
        reference_type: 4713,
        owner: 101,
        kind: 1,
        category: 1,
        character_id: 1,
        template_visibility: 1,
        ..Default::default()
    };
    let mut read = Some(VisibilityRead {
        key: (Some(1), 1, 0, 1),
        visibility: LeaderVisibility::Conditioned(0),
        noted: false,
    });
    let mut evaluation = None;
    let mut counts = ReuseCounts::default();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    // Each hour's power, flags, verdict, whether the World may be built,
    // and evaluations run and reused so far.
    let hours = [
        (100, 10, 1, 1, true, true, 1, 0),
        // Political power and the other country's flag change.
        (101, 11, 1, 2, true, false, 1, 1),
        (102, 12, 1, 3, true, false, 1, 2),
        // The flag the visibility reads is cleared.
        (103, 12, 0, 3, false, true, 2, 2),
    ];
    for (hour, power, flag, other, verdict, builds, evaluated, reused) in hours {
        let wire = encoder
            .encode(frame(hour, power, flag, other), false)
            .unwrap();
        let decoded = decoder.advance(&wire).unwrap().clone();
        let changes = tracker.observe_updates(&decoded, Arc::new(wire), FormatVersion::CURRENT);
        let since = Since::hour_before(&decoded, &changes);
        // The World an evaluation reads holds the content's role; one an
        // unchanged hour reuses through is never built.
        let world = OnceCell::new();
        if builds || checking {
            assert!(
                world
                    .set(Ok(World::read(
                        &decoded,
                        ContentFacts {
                            state_content: Ok(()),
                            leader_roles: Ok(&roles),
                            ..ContentFacts::content_only(&dlcs, &triggers, &aliases)
                        },
                    )))
                    .is_ok()
            );
        }
        let hour_world = HourWorld {
            content,
            frame: &decoded,
            world,
            gap: OnceCell::new(),
            kept: None,
        };
        let mut rereads = Rereads::new();
        let decided = visible_reusing(
            &row,
            Some(&hour_world),
            &mut read,
            Some(Reuse {
                evaluation: &mut evaluation,
                rereads: &mut rereads,
                counts: &mut counts,
                since: since.as_ref(),
            }),
            Some(1),
            true,
        );
        assert_eq!(decided, Ok(verdict), "hour {hour}");
        assert_eq!(
            (counts.evaluated, counts.reused),
            (evaluated, reused),
            "hour {hour}: rereads {rereads:?}, inputs {:?}",
            evaluation.as_ref().map(|evaluation| &evaluation.inputs)
        );
        if !builds && !checking {
            assert!(rereads.is_empty(), "hour {hour} read {rereads:?}");
            assert!(
                hour_world.world.get().is_none(),
                "hour {hour} built its World"
            );
        }
    }
    assert_eq!(counts.mismatched, 0);
}

/// A kept frame's visibility pass that another leader's verdict stops
/// (`Kept::blocked`) leaves the hour's observation on its whole frame
/// nothing to depend on: a leader whose visibility input has just become
/// active and whose template is unknown reads it again there and records
/// its refusal, as the per-hour oracle does.
#[test]
fn a_blocked_kept_visibility_pass_leaves_the_whole_frame_its_refusals() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use dashboard_core::test_fixtures::{CAMPAIGN, records};
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/00.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/a.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    let content = Content {
        politics: Arc::new(politics_content::PoliticsContent::load(catalogue.clone()).unwrap()),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let mut base = prepared::test_fixtures::frame(100, 1);
    for table in [TableId::Provinces, TableId::States] {
        *base.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    // The leader's own visibility choice becomes active at 101; its
    // character has no recorded template.
    let frames: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base.clone();
            frame.hour = hour;
            if hour == 101
                && let TableView::Complete(TableRows::UnitLeaders(rows)) =
                    &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].visibility_choice = 1;
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let params = UnitLeadersParams {
        branch_id: "1".into(),
        selection: LeaderSelection::All,
        view: LeaderView::Career,
        leaders: LeaderKinds::All,
    };
    dashboard_text::triggers::inputs::reusing(|| {
        let mut scan = Scan::new(&replay, 1).unwrap();
        scan.reuse_evaluations = true;
        let mut tracker = FrameChangeTracker::new();
        let first = tracker.observe(&frames[0], replay.format);
        assert!(
            scan.observe_changes(
                &frames[0],
                &replay,
                &params,
                Some(&content),
                &first,
                &first,
                None,
                &mut |_, _, _| {},
            )
            .unwrap()
        );
        let second = tracker.observe(&frames[1], replay.format);
        let reduced = block::Changes::default();
        let kept = Kept::new(&reduced);
        // Another leader's verdict needs the whole frame.
        kept.block("leaders.reject.test");
        assert!(
            !scan
                .observe_changes(
                    &frames[1],
                    &replay,
                    &params,
                    Some(&content),
                    &second,
                    &second,
                    Some(&kept),
                    &mut |_, _, _| {},
                )
                .unwrap(),
            "the blocked hour is observed on its whole frame"
        );
        assert!(
            scan.observe_changes(
                &frames[1],
                &replay,
                &params,
                Some(&content),
                &second,
                &second,
                None,
                &mut |_, _, _| {},
            )
            .unwrap()
        );
        let actual = scan.finish(&params, Some(&content)).unwrap();
        let mut oracle = Scan::new(&replay, 1).unwrap();
        for frame in &frames {
            oracle
                .observe_read(frame, &replay, &params, Some(&content), &mut |_, _, _| {})
                .unwrap();
        }
        let expected = oracle.finish(&params, Some(&content)).unwrap();
        assert!(
            expected
                .warnings
                .iter()
                .any(|warning| warning.contains("no template")),
            "{:?}",
            expected.warnings
        );
        dashboard_base::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
    });
}

#[test]
fn experience_changes_create_visible_runs_and_keep_complete_ordered_training() {
    use replay_data::rows::{LexicalTokenRow, TableRows, UnitLeaderTraitExperienceRow};
    use replay_frame::frame::TableView;
    let reference = (5_u64 << 32) | 4713;
    let make = |hour, experience| {
        let mut frame = dashboard_core::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 5,
                reference_type: 4713,
                owner: 11,
                kind: 1,
                category: 1,
                character_id: 9,
                skill_status: 2,
                skill_level: 3,
                experience,
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(Vec::new()));
        *frame.tables[TableId::UnitLeaderTraitExperience.index()] =
            TableView::Complete(TableRows::UnitLeaderTraitExperience(vec![
                UnitLeaderTraitExperienceRow {
                    list: 0,
                    leader_reference: reference,
                    ordinal: 0,
                    token_id: 100,
                    definition_ordinal: 7,
                    experience: i64::MAX,
                },
                UnitLeaderTraitExperienceRow {
                    list: 0,
                    leader_reference: reference,
                    ordinal: 1,
                    token_id: 100,
                    definition_ordinal: 7,
                    experience: -1,
                },
                UnitLeaderTraitExperienceRow {
                    leader_reference: reference,
                    list: 1,
                    ordinal: 0,
                    token_id: 100,
                    definition_ordinal: 7,
                    experience: -25_000,
                },
            ]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 100,
                literal: b"mod_training".to_vec(),
            }]));
        frame
    };
    let replay = Replay::from_records(
        dashboard_core::test_fixtures::CAMPAIGN.to_vec(),
        &[dashboard_core::test_fixtures::records(
            1,
            vec![make(100, 3), make(101, i64::MAX)],
        )],
    )
    .unwrap();
    let reply = scan(&replay, &all_params(), None, &AtomicBool::new(false)).unwrap();
    let runs = &reply.leaders[0].runs;
    assert_eq!(runs.len(), 2);
    let first = runs[0].value.progress.as_ref().unwrap();
    assert_eq!(first.experience_raw, "3");
    let training = first.training.as_ref().unwrap();
    assert_eq!(training.len(), 2);
    assert_eq!(training[0].token.as_deref(), Some("mod_training"));
    assert_eq!(training[0].experience_raw, i64::MAX.to_string());
    assert_eq!(training[1].experience_raw, "-1");
    assert_eq!(first.factors.as_ref().unwrap()[0].factor_raw, "-25000");
    assert_eq!(
        runs[1].value.progress.as_ref().unwrap().experience_raw,
        i64::MAX.to_string()
    );
}
