use super::*;
use replay_data::rows::NavalProvinceCombatRow;

fn branch_start(reason: BranchReason, loaded_hour: Option<Hour>) -> BranchStart {
    BranchStart {
        id: 1,
        session: 1,
        loaded_hour,
        first_hour: 20,
        parent: replay_frame::branches::Parentage::Unknown,
        reason,
    }
}

#[test]
fn only_a_load_signal_at_its_loaded_hour_is_a_native_load() {
    let start = branch_start;
    assert_eq!(
        native_load(&start(BranchReason::LoadSignal, Some(20))),
        Some(20)
    );
    assert_eq!(native_load(&start(BranchReason::LoadSignal, None)), None);
    assert_eq!(
        native_load(&start(BranchReason::LoadSignal, Some(19))),
        None
    );
    assert_eq!(
        native_load(&start(BranchReason::HourDiscontinuity, Some(20))),
        None
    );
    assert_eq!(native_load(&start(BranchReason::SessionStart, None)), None);
}

#[test]
fn a_load_without_its_origin_folds_its_first_hour_as_a_gap() {
    let fold = |loaded_hour| {
        let start = branch_start(BranchReason::LoadSignal, loaded_hour);
        let mut frame = roster_frame(start.first_hour, Some(&[(40, 4)]), false);
        frame.branch = start.id;
        let shared = Arc::new(History::default());
        let country = Country::Tag("AAA".into());
        let mut pass = Pass::new(Arc::clone(&shared), country.clone());
        pass.fold_hour(frame.branch, frame.hour, Some(&frame), native_load(&start), false);
        let index = EngagedIndex { shared };
        let viewer = Viewer::read(&frame, &country).unwrap();
        combats(
            &frame,
            LAND,
            viewer.slot.unwrap(),
            &viewer.inputs.links,
            viewer.inputs.relations.as_ref().map_err(|reason| *reason),
            Some(index.shared.as_ref()),
        )
        .unwrap()
    };
    let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
    assert_eq!(fold(Some(20)), flag(Engaged::No));
    assert_eq!(fold(None), flag(Engaged::Unknown));
    assert_eq!(fold(Some(19)), flag(Engaged::Unknown));
}

#[test]
fn one_engaged_entry_of_any_class_makes_a_battle_ongoing() {
    let row = NavalProvinceCombatRow {
        province: 10,
        combats: vec![1, 2],
        kinds: vec![NavalProvinceCombatRow::NAVAL, NavalProvinceCombatRow::LAND],
    };
    let flags = |naval, land| CombatFlags {
        naval: BTreeMap::from([(1, naval)]),
        land: BTreeMap::from([(2, land)]),
        ..CombatFlags::default()
    };
    let ongoing = |flags: CombatFlags| flags.engaged_entries(Some(&row), true, 1);
    assert_eq!(ongoing(flags(Engaged::No, Engaged::Yes)), Some(true));
    assert_eq!(ongoing(flags(Engaged::No, Engaged::No)), Some(false));
    assert_eq!(ongoing(flags(Engaged::Unknown, Engaged::Yes)), Some(true));
    let ambiguous = Engaged::Ambiguous(engaged::Ambiguity::Scope);
    assert_eq!(ongoing(flags(ambiguous, Engaged::No)), None);
    let crossed = CombatFlags {
        naval: BTreeMap::from([(2, Engaged::Yes)]),
        land: BTreeMap::from([(1, Engaged::Yes)]),
        ..CombatFlags::default()
    };
    assert_eq!(crossed.engaged_entries(Some(&row), true, 1), None);
}

fn naval_frame(
    countries: u32,
    players: &[u32],
    factions: &[(u32, u32)],
    combats: u64,
    bytes: [u8; 2],
) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::FactionMemberRow,
        rows::NavalCombatDetailsRow, rows::NavalCombatRow, rows::PlayerRow, rows::TableRows,
        rows::TaskForceRow,
    };
    use replay_frame::frame::TableView;
    let mut frame = DecodedFrame {
        branch: 3,
        hour: 0,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let mut set = |rows: TableRows| {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    set(TableRows::Countries(
        (1..=countries)
            .map(|slot| CountryRow {
                id: slot,
                canonical_slot: slot,
                tag: format!("C{slot}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::CountryLinks(
        (1..=countries)
            .map(|slot| CountryLinkRow {
                id: slot,
                canonical_slot: slot,
            })
            .collect(),
    ));
    set(TableRows::Players(
        players
            .iter()
            .enumerate()
            .map(|(id, &country)| PlayerRow {
                id: id as i32,
                country,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::LocalPlayer(vec![
        replay_data::rows::LocalPlayerRow {
            country: players.first().copied().unwrap_or(0),
            observer_country: 0,
        },
    ]));
    set(TableRows::FactionMembers(
        factions
            .iter()
            .enumerate()
            .flat_map(|(faction, &(a, b))| {
                [(a, 0), (b, 1)].map(|(country, ordinal)| FactionMemberRow {
                    country,
                    faction: faction as u64 + 1,
                    ordinal,
                })
            })
            .collect(),
    ));
    for table in [
        TableId::HumanControllers,
        TableId::WarRelations,
        TableId::Subjects,
        TableId::NavalCombatParticipants,
        TableId::CombatEngagementRefreshes,
    ] {
        set(TableRows::empty(table));
    }
    set(TableRows::NavalCombats(
        (0..combats)
            .map(|id| NavalCombatRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    let attacker = |combat: u64| (combat as u32 % (countries - 5)) + 2;
    set(TableRows::NavalCombatDetails(
        (0..combats)
            .map(|combat| {
                let mut row = NavalCombatDetailsRow {
                    combat,
                    ..Default::default()
                };
                for (side, country) in [attacker(combat), 5].into_iter().enumerate() {
                    let detail = &mut row.sides[side];
                    detail.units = vec![2 * combat + 7 + side as u64];
                    detail.summary.countries = vec![country];
                    detail.engaged = bytes[side];
                }
                row
            })
            .collect(),
    ));
    set(TableRows::LandCombats(vec![
        replay_data::rows::LandCombatRow {
            id: 100,
            attacker_classes: vec![0],
            attackers: vec![300],
            defenders: vec![301],
            ..Default::default()
        },
    ]));
    set(TableRows::LandCombatSides(vec![
        replay_data::rows::LandCombatSideRow {
            combat: 100,
            attacker_primary: vec![2],
            defender_primary: vec![5],
            ..Default::default()
        },
    ]));
    set(TableRows::TaskForces(
        (0..combats)
            .flat_map(|combat| {
                [(0, attacker(combat)), (1, 5)].map(|(side, country)| TaskForceRow {
                    id: 2 * combat + 7 + side,
                    logical_owner: country,
                    roster_owner: country,
                    ..Default::default()
                })
            })
            .collect(),
    ));
    frame
}

#[expect(clippy::type_complexity, reason = "Each fixture hour includes players, faction links, and both combat sides.")]
fn fold_hours(tag: &str, hours: &[(Hour, &[u32], &[(u32, u32)], [u8; 2])]) -> Pass {
    let mut pass = Pass::new(Arc::default(), Country::Tag(tag.into()));
    let load = hours.first().map(|hour| hour.0);
    for &(hour, players, factions, bytes) in hours {
        let frame = naval_frame(6, players, factions, 1, bytes);
        pass.fold_hour(3, hour, Some(&frame), load, false);
    }
    pass
}

fn index(pass: &Pass) -> EngagedIndex {
    EngagedIndex { shared: Arc::clone(&pass.shared) }
}

fn flags(index: &EngagedIndex, frame: &mut DecodedFrame, hour: Hour, viewer: u32) -> CombatFlags {
    frame.branch = 3;
    frame.hour = hour;
    let links: BTreeMap<u32, u32> = (1..=6).map(|slot| (slot, slot)).collect();
    let relations = Relations::read(frame, &links);
    CombatFlags::read(
        frame,
        viewer,
        &links,
        relations.as_ref().map_err(|reason| *reason),
        Some(index.shared.as_ref()),
    )
    .unwrap()
}

fn naval_flag(index: &EngagedIndex, frame: &mut DecodedFrame, hour: Hour, viewer: u32) -> Engaged {
    flags(index, frame, hour, viewer).naval[&0]
}

#[test]
fn a_viewer_who_is_the_recording_player_takes_its_player_change_rebuilds() {
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(1, 2)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    for (hour, players, factions, bytes, local_sweep) in [
        (10, &[1][..], none, [0, 0], None),
        (11, &[2][..], none, [1, 0], Some(2)),
        (12, &[2][..], allied, [1, 0], None),
        (13, &[1][..], allied, [1, 0], Some(1)),
    ] {
        let mut frame = naval_frame(6, players, factions, 1, bytes);
        if let Some(target) = local_sweep {
            *frame.tables[TableId::CombatEngagementRefreshes.index()] =
                replay_frame::frame::TableView::Complete(
                    replay_data::rows::TableRows::CombatEngagementRefreshes(vec![
                        replay_data::rows::CombatEngagementRefreshRow {
                            kind: replay_data::rows::CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                            target,
                            side: 0,
                            viewer: 0,
                            triggers:
                                replay_data::rows::CombatEngagementRefreshRow::TRIGGER_RECORDED,
                        },
                    ]),
                );
        }
        pass.fold_hour(3, hour, Some(&frame), Some(10), false);
    }
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (8, 0));
    let index = index(&pass);
    let mut frame = naval_frame(6, &[1], allied, 1, [1, 0]);
    let served = flags(&index, &mut frame, 13, 1);
    assert_eq!(
        (served.naval[&0], served.land[&100]),
        (Engaged::Yes, Engaged::Yes)
    );
    let branches = pass.shared.branches.lock().unwrap();
    for (kind, combat) in [(NAVAL, 0), (LAND, 100)] {
        let points = &branches[&3].folds()[0].points()[kind][&(combat, 0)];
        assert_eq!(at(points, 12), Some(Engaged::No));
        assert_eq!(at(points, 13), Some(Engaged::No));
    }
}

#[test]
fn a_player_change_rebuilds_nothing_for_another_viewer() {
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(2, 5)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C5".into()));
    pass.fold_hour(
        3,
        10,
        Some(&naval_frame(6, &[1], none, 1, [0, 0])),
        Some(10),
        false,
    );
    let mut hour11 = naval_frame(6, &[2], allied, 1, [1, 1]);
    *hour11.tables[TableId::CombatEngagementRefreshes.index()] =
        replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::CombatEngagementRefreshes(vec![
                replay_data::rows::CombatEngagementRefreshRow {
                    kind: replay_data::rows::CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                    target: 2,
                    side: 0,
                    viewer: 0,
                    triggers: replay_data::rows::CombatEngagementRefreshRow::TRIGGER_RECORDED,
                },
            ]),
        );
    pass.fold_hour(3, 11, Some(&hour11), Some(10), false);
    let index = index(&pass);
    let mut frame = naval_frame(6, &[2], allied, 1, [1, 1]);
    let branches = pass.shared.branches.lock().unwrap();
    for (kind, combat) in [(NAVAL, 0), (LAND, 100)] {
        assert_eq!(
            at(&branches[&3].folds()[0].points()[kind][&(combat, 0)], 11),
            Some(Engaged::No)
        );
    }
    drop(branches);
    assert_eq!(naval_flag(&index, &mut frame, 11, 5), Engaged::Yes);
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.undecided), (3, 1));
}

#[test]
fn a_player_change_hidden_in_an_unreadable_block_rebuilds_nothing_on_recovery() {
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(2, 3)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    pass.fold_hour(
        3,
        10,
        Some(&naval_frame(6, &[1], none, 1, [0, 0])),
        Some(10),
        false,
    );
    pass.failed_block(3, 19);
    let mut frame = naval_frame(6, &[3], allied, 1, [0, 0]);
    pass.fold_hour(3, 20, Some(&frame), Some(10), false);
    let index = index(&pass);
    let served = flags(&index, &mut frame, 20, 3);
    assert_eq!(served.land[&100], Engaged::Unknown);
    assert_eq!(served.naval[&0], Engaged::No);
    let summary = &pass.check.summary;
    assert_eq!((summary.failed_blocks, summary.undecided), (1, 2));
}

#[test]
fn mismatches_are_named_and_undecided_sides_counted_apart() {
    let none: &[(u32, u32)] = &[];
    let pass = fold_hours("C1", &[(10, &[1], none, [0, 1])]);
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (1, 1));
    assert_eq!(
        summary.first,
        [naval_check::Mismatch {
            branch: 3,
            hour: 10,
            combat: 0,
            side: 1,
            derived: false,
        }]
    );
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let mut frame = naval_frame(6, &[1], none, 1, [0, 1]);
    pass.fold_hour(3, 10, Some(&frame), None, false);
    assert_eq!(pass.check.summary.undecided, 2);
    let index = index(&pass);
    assert_eq!(naval_flag(&index, &mut frame, 10, 1), Engaged::Yes);
    assert_eq!(naval_flag(&index, &mut frame, 10, 3), Engaged::No);
}

#[test]
fn unread_and_unnamed_hours_are_counted_through_the_pass() {
    let none: &[(u32, u32)] = &[];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    pass.fold_hour(3, 10, None, Some(10), false);
    let mut frame = naval_frame(6, &[1], none, 1, [0, 0]);
    *frame.tables[TableId::CountryLinks.index()] = replay_frame::frame::TableView::Unavailable {
        reason: "not captured".into(),
    };
    pass.fold_hour(3, 11, Some(&frame), Some(10), false);
    let mut frame = naval_frame(6, &[1, 5], none, 1, [0, 0]);
    *frame.tables[TableId::LocalPlayer.index()] = replay_frame::frame::TableView::Unavailable {
        reason: "not captured".into(),
    };
    pass.fold_hour(3, 12, Some(&frame), Some(10), false);
    pass.failed_block(3, 20);
    pass.fold_hour(
        3,
        21,
        Some(&naval_frame(6, &[1], none, 0, [0, 0])),
        Some(10),
        false,
    );
    let summary = pass.shared.check.lock().unwrap().clone();
    assert_eq!(
        (
            summary.hours,
            summary.naval_hours,
            summary.unreadable_hours,
            summary.unidentified_hours,
            summary.failed_blocks,
        ),
        (4, 2, 1, 2, 1)
    );
    assert_eq!(summary.agreements + summary.undecided, 0);
    let lines = summary.lines();
    assert!(lines[1].contains("2 hours whose player the recording does not name, 1 unreadable hours, 1 unreadable blocks"), "{lines:?}");
    let mut clean = Pass::new(Arc::default(), Country::Tag("C1".into()));
    clean.fold_hour(
        3,
        10,
        Some(&naval_frame(6, &[1], none, 0, [0, 0])),
        Some(10),
        false,
    );
    assert!(clean.check.summary.lines()[0].contains("no naval combat side in the 1 folded hours"));
}

#[test]
fn an_observer_view_set_without_a_rebuild_shows_one_sample_later() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let fold = |views: [(u32, u32, [u8; 2], Option<u64>); 3]| {
        let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
        for (hour, (country, observer_country, bytes, local_sweep)) in (10..).zip(views) {
            let mut frame = naval_frame(6, &[1], allied, 1, bytes);
            *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(
                TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country,
                    observer_country,
                }]),
            );
            if let Some(target) = local_sweep {
                *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
                    TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
                        kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                        target,
                        side: 0,
                        viewer: 0,
                        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                    }]),
                );
            }
            pass.fold_hour(3, hour, Some(&frame), Some(10), false);
        }
        pass
    };
    let pass = fold([
        (0, 3, [1, 0], None),
        (0, 4, [1, 0], None),
        (0, 4, [0, 0], Some(4)),
    ]);
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.agreements, summary.mismatches, summary.undecided),
        (6, 0, 0),
        "{:?}",
        summary.first
    );
    let index_obs = index(&pass);
    let mut frame = naval_frame(6, &[1], allied, 1, [1, 0]);
    *frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 0,
            observer_country: 4,
        }]));
    assert_eq!(naval_flag(&index_obs, &mut frame, 11, 4), Engaged::Yes);
    assert_eq!(naval_flag(&index_obs, &mut frame, 12, 4), Engaged::No);
    let pass = fold([
        (3, 0, [1, 0], None),
        (4, 0, [0, 0], Some(4)),
        (4, 0, [0, 0], None),
    ]);
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (6, 0));
    let pass = fold([
        (3, 0, [1, 0], None),
        (0, 4, [1, 0], None),
        (0, 4, [0, 0], Some(4)),
    ]);
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.agreements, summary.mismatches),
        (6, 0),
        "{:?}",
        summary.first
    );

    let mut legacy = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let mut legacy_hour12 = None;
    for (hour, (country, observer_country, bytes)) in (10..).zip([
        (0, 3, [1, 0]),
        (0, 4, [1, 0]),
        (0, 4, [0, 0]),
    ]) {
        let mut frame = naval_frame(6, &[1], allied, 1, bytes);
        *frame.tables[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country,
                observer_country,
            }]));
        *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
            reason: "pre-146".into(),
        };
        legacy.fold_hour(3, hour, Some(&frame), Some(10), false);
        if hour == 12 {
            legacy_hour12 = Some(frame);
        }
    }
    let legacy_index = index(&legacy);
    let mut legacy_hour12 = legacy_hour12.unwrap();
    let legacy_served = flags(&legacy_index, &mut legacy_hour12, 12, 4);
    assert_eq!(
        (legacy_served.land[&100], legacy_served.naval[&0]),
        (Engaged::Ambiguous(engaged::Ambiguity::Scope), Engaged::No)
    );
}

#[test]
fn an_observer_switch_with_a_refresh_does_not_serve_the_old_viewers_definite_flag() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));

    let mut frame10 = naval_frame(6, &[1], allied, 1, [1, 0]);
    *frame10.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
        vec![LocalPlayerRow {
            country: 0,
            observer_country: 3,
        }],
    ));
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);

    let mut frame11 = naval_frame(6, &[1], allied, 1, [0, 0]);
    *frame11.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
        vec![LocalPlayerRow {
            country: 0,
            observer_country: 4,
        }],
    ));
    *frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 4,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 4,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ]),
    );
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);

    assert_eq!(pass.check.summary.mismatches, 0, "{:?}", pass.check.summary.first);
    let index_local = index(&pass);
    let served = flags(&index_local, &mut frame11, 11, 4);
    assert_eq!(served.naval[&0], Engaged::No);
    assert_eq!(served.land[&100], Engaged::No);

    let mut side_pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    side_pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    let mut side_frame11 = naval_frame(6, &[1], allied, 1, [0, 0]);
    *side_frame11.tables[TableId::LocalPlayer.index()] = TableView::Complete(
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 0,
            observer_country: 4,
        }]),
    );
    *side_frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 100,
            side: 0,
            viewer: 4,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }]),
    );
    side_pass.fold_hour(3, 11, Some(&side_frame11), Some(10), false);
    let side_index = index(&side_pass);
    let side_served = flags(&side_index, &mut side_frame11, 11, 4);
    assert_eq!(side_served.land[&100], Engaged::No);

    let mut both_pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    both_pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    let mut both_frame11 = side_frame11.clone();
    *both_frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 4,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ]),
    );
    both_pass.fold_hour(3, 11, Some(&both_frame11), Some(10), false);
    let both_index = index(&both_pass);
    let both_served = flags(&both_index, &mut both_frame11, 11, 4);
    assert_eq!(
        both_served.land[&100],
        Engaged::Ambiguous(engaged::Ambiguity::Order)
    );
}

#[test]
fn malformed_refresh_witness_rows_withhold_witness() {
    use replay_data::rows::{CombatEngagementRefreshRow, TableRows};
    use replay_frame::frame::TableView;
    for bad in [
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 0,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::NAVAL_SIDE,
            target: 0,
            side: 1,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 100,
            side: 0,
            viewer: i32::MAX as u32 + 1,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: i32::MAX as u64 + 1,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 0,
            side: 0,
            viewer: 1,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 0,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: 0,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: i32::MAX as u64 + 1,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: 4,
            side: 0,
            viewer: 1,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        },
        CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: 4,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
        },
    ] {
        let mut frame = naval_frame(6, &[1], &[], 1, [0, 0]);
        *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
            TableRows::CombatEngagementRefreshes(vec![bad]),
        );
        assert!(matches!(witness(&frame).unwrap(), Witness::Unavailable));
    }
}

#[test]
fn recording_track_does_not_prove_refresh_from_remote_controller_target_without_local_sweep() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    let observer_frame = |factions: &[(u32, u32)], refreshes: Vec<CombatEngagementRefreshRow>| {
        let mut frame = naval_frame(6, &[1], factions, 1, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: 3,
            }],
        ));
        *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
            TableRows::CombatEngagementRefreshes(refreshes),
        );
        frame
    };
    let frame10 = observer_frame(&[], Vec::new());
    let frame11 = observer_frame(allied, Vec::new());
    let mut frame12 = observer_frame(
        allied,
        vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: 3,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }],
    );
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);
    pass.fold_hour(3, 12, Some(&frame12), Some(10), false);

    assert_eq!(pass.check.summary.mismatches, 0, "{:?}", pass.check.summary.first);
    let index = index(&pass);
    let served = flags(&index, &mut frame12, 12, 3);
    assert_eq!((served.land[&100], served.naval[&0]), (Engaged::No, Engaged::No));

    let branches = pass.shared.branches.lock().unwrap();
    let perspective = &branches[&3].folds()[0];
    assert_eq!(at(&perspective.points()[LAND][&(100, 0)], 12), Some(Engaged::Yes));
    assert_eq!(at(&perspective.points()[NAVAL][&(0, 0)], 12), Some(Engaged::Yes));
}

#[test]
fn local_all_sides_sweep_evaluates_recorded_execution_viewpoint_not_hourly_observer() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C4".into()));
    let observer_frame = |bytes: [u8; 2], refreshes: Vec<CombatEngagementRefreshRow>| {
        let mut frame = naval_frame(6, &[1], allied, 1, bytes);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: 4,
            }],
        ));
        *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
            TableRows::CombatEngagementRefreshes(refreshes),
        );
        frame
    };
    let frame10 = observer_frame([0, 0], Vec::new());
    let mut frame11 = observer_frame(
        [1, 0],
        vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 3,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }],
    );
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);

    assert_eq!(pass.check.summary.mismatches, 0, "{:?}", pass.check.summary.first);
    let index = index(&pass);
    let served = flags(&index, &mut frame11, 11, 4);
    assert_eq!((served.land[&100], served.naval[&0]), (Engaged::Yes, Engaged::Yes));
}

#[test]
fn side_refresh_evaluates_recorded_execution_viewpoint_on_recording_track_and_perspective_viewer_otherwise() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let observer_frame = |bytes: [u8; 2], refreshes: Vec<CombatEngagementRefreshRow>| {
        let mut frame = naval_frame(6, &[1], allied, 1, bytes);
        if let TableView::Complete(TableRows::NavalCombats(rows)) =
            &mut *frame.tables[TableId::NavalCombats.index()]
        {
            rows[0].id = 1;
        }
        if let TableView::Complete(TableRows::NavalCombatDetails(rows)) =
            &mut *frame.tables[TableId::NavalCombatDetails.index()]
        {
            rows[0].combat = 1;
        }
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: 4,
            }],
        ));
        *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
            TableRows::CombatEngagementRefreshes(refreshes),
        );
        frame
    };

    let mut pass = Pass::new(Arc::default(), Country::Tag("C4".into()));
    let frame10 = observer_frame([0, 0], Vec::new());
    let mut frame11 = observer_frame(
        [1, 0],
        vec![
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::NAVAL_SIDE,
                target: 1,
                side: 0,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ],
    );
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);

    assert_eq!(
        (
            pass.check.summary.agreements,
            pass.check.summary.mismatches,
            pass.check.summary.undecided,
        ),
        (4, 0, 0),
        "{:?}",
        pass.check.summary.first
    );
    let idx = index(&pass);
    let served = flags(&idx, &mut frame11, 11, 4);
    assert_eq!((served.land[&100], served.naval[&1]), (Engaged::Yes, Engaged::Yes));

    let branches = pass.shared.branches.lock().unwrap();
    let perspective = &branches[&3].folds()[0];
    assert_eq!(at(&perspective.points()[LAND][&(100, 0)], 11), Some(Engaged::No));
    assert_eq!(at(&perspective.points()[NAVAL][&(1, 0)], 11), Some(Engaged::No));
    drop(branches);

    let mut unknown_pass = Pass::new(Arc::default(), Country::Tag("C4".into()));
    let mut unknown11 = observer_frame(
        [0, 0],
        vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 100,
            side: 0,
            viewer: 99,
            triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
        }],
    );
    unknown_pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    unknown_pass.fold_hour(3, 11, Some(&unknown11), Some(10), false);
    let unknown_idx = index(&unknown_pass);
    let unknown_served = flags(&unknown_idx, &mut unknown11, 11, 4);
    assert_eq!(unknown_served.land[&100], Engaged::Unknown);
    let branches = unknown_pass.shared.branches.lock().unwrap();
    assert_eq!(
        at(&branches[&3].folds()[0].points()[LAND][&(100, 0)], 11),
        Some(Engaged::No)
    );
}

#[test]
fn unindexed_and_pending_load_fallback_match_indexed_load_for_zero_mapped_tag() {
    use replay_data::rows::{
        CountryCounterInputRow, CountryLinkRow, FactionMemberRow, LandCombatSideRow,
        LocalPlayerRow, PacketHour, Recorded, ReservedCountryNamingRow,
        ReservedReverseVisibilityInputs, ReverseZeroVisibilityInputs, TableRows,
        ZeroSubjectInputs,
    };
    use replay_frame::frame::TableView;

    let mut frame = naval_frame(6, &[1], &[], 1, [0, 0]);
    frame.branch = 3;
    frame.hour = 10;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 777,
            canonical_slot: 0,
        });
    }
    *frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 0,
            observer_country: 777,
        }]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 2,
            faction: 9,
            ordinal: 0,
        }]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                faction: Recorded::Value(Some(9)),
                reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                    sample_hour: PacketHour::Host,
                    cells: vec![Some(ReverseZeroVisibilityInputs {
                        war: Some([1, 0]),
                        ..Default::default()
                    })],
                })),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (1..=6)
                .map(|country| CountryCounterInputRow {
                    country,
                    forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                        war: Some([0, 0]),
                        military_access: Some(0),
                        target: Some(0),
                        subject: Some(ZeroSubjectInputs {
                            relation_type: None,
                            parties: None,
                        }),
                    })),
                    ..Default::default()
                })
                .collect(),
        ));
    let links: BTreeMap<u32, u32> = (1..=6)
        .map(|slot| (slot, slot))
        .chain([(777, 0)])
        .collect();
    let relations = Relations::read(&frame, &links).unwrap();
    let pending = History::default();
    let mut pass = Pass::new(Arc::default(), Country::Recorded);
    pass.fold_hour(3, 10, Some(&frame), Some(10), false);
    let indexed = index(&pass);

    assert_eq!(
        combats(&frame, LAND, 0, &links, Ok(&relations), None).unwrap(),
        (BTreeMap::from([(100, Engaged::Yes)]), Provenance::Unindexed)
    );
    assert_eq!(
        combats(&frame, LAND, 0, &links, Ok(&relations), Some(&pending)).unwrap(),
        (BTreeMap::from([(100, Engaged::Yes)]), Provenance::Pending)
    );
    assert_eq!(
        combats(
            &frame,
            LAND,
            0,
            &links,
            Ok(&relations),
            Some(indexed.shared.as_ref()),
        )
        .unwrap(),
        (BTreeMap::from([(100, Engaged::Yes)]), Provenance::Indexed)
    );

    let mut war_frame = frame.clone();
    *war_frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
            combat: 100,
            attacker_primary: vec![777],
            defender_primary: Vec::new(),
            ..Default::default()
        }]));
    let mut war_pass = Pass::new(Arc::default(), Country::Recorded);
    war_pass.fold_hour(3, 10, Some(&war_frame), Some(10), false);
    let war_indexed = index(&war_pass);
    assert_eq!(
        combats(&war_frame, LAND, 0, &links, Ok(&relations), None).unwrap(),
        (BTreeMap::from([(100, Engaged::No)]), Provenance::Unindexed)
    );
    assert_eq!(
        combats(
            &war_frame,
            LAND,
            0,
            &links,
            Ok(&relations),
            Some(war_indexed.shared.as_ref()),
        )
        .unwrap(),
        (BTreeMap::from([(100, Engaged::No)]), Provenance::Indexed)
    );
}

#[test]
fn complete_empty_witness_does_not_infer_all_sides_refresh_from_observer_lag() {
    use replay_data::rows::{LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 4)];
    let gained_vision: &[(u32, u32)] = &[(2, 4), (2, 3)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let observer_frame = |factions: &[(u32, u32)], observer: u32, bytes: [u8; 2]| {
        let mut frame = naval_frame(6, &[1], factions, 1, bytes);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: observer,
            }],
        ));
        frame
    };
    let frame10 = observer_frame(allied, 3, [0, 0]);
    let frame11 = observer_frame(allied, 4, [0, 0]);
    let mut frame12 = observer_frame(gained_vision, 3, [0, 0]);
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);
    pass.fold_hour(3, 12, Some(&frame12), Some(10), false);

    assert_eq!(
        (
            pass.check.summary.agreements,
            pass.check.summary.mismatches,
            pass.check.summary.undecided,
        ),
        (6, 0, 0),
        "{:?}",
        pass.check.summary.first
    );
    let idx = index(&pass);
    let served = flags(&idx, &mut frame12, 12, 3);
    assert_eq!((served.land[&100], served.naval[&0]), (Engaged::No, Engaged::No));

    let mut frame13 = observer_frame(gained_vision, 3, [0, 0]);
    *frame13.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    pass.fold_hour(3, 13, Some(&frame13), Some(10), false);
    let idx = index(&pass);
    let served13 = flags(&idx, &mut frame13, 13, 3);
    assert_eq!(
        served13.land[&100],
        Engaged::Ambiguous(engaged::Ambiguity::Scope)
    );
}

#[test]
fn mid_campaign_gap_recovers_recording_manager_cache_through_index() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let observer_frame = |observer: u32| {
        let mut frame = naval_frame(6, &[1], allied, 0, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: observer,
            }],
        ));
        frame
    };
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let frame10 = observer_frame(3);
    let mut frame11 = observer_frame(3);
    *frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 1,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ]),
    );
    let frame12 = observer_frame(4);
    let mut frame13 = observer_frame(4);
    *frame13.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    pass.fold_hour(3, 10, Some(&frame10), None, false);
    pass.fold_hour(3, 11, Some(&frame11), None, false);
    pass.fold_hour(3, 12, Some(&frame12), None, false);
    pass.fold_hour(3, 13, Some(&frame13), None, false);

    let idx = index(&pass);
    let served11 = flags(&idx, &mut frame11, 11, 3);
    assert_eq!(served11.land[&100], Engaged::Yes);
    let served13 = flags(&idx, &mut frame13, 13, 4);
    assert_eq!(served13.land[&100], Engaged::No);
}

#[test]
fn gap_alias_recovery_and_post_switch_narrowing_through_index() {
    use replay_data::rows::{CombatEngagementRefreshRow, CountryLinkRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let observer_frame = |observer: u32, extra_links: &[(u32, u32)]| {
        let mut frame = naval_frame(6, &[1], allied, 0, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: observer,
            }],
        ));
        if !extra_links.is_empty() {
            let mut links: Vec<_> = (1..=6)
                .map(|slot| CountryLinkRow {
                    id: slot,
                    canonical_slot: slot,
                })
                .collect();
            for &(id, canonical_slot) in extra_links {
                links.push(CountryLinkRow { id, canonical_slot });
            }
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links));
        }
        frame
    };

    let aliased = &[(101, 3), (202, 3)];
    let split = &[(101, 3), (202, 4)];
    let mut alias_pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let a10 = observer_frame(101, aliased);
    let mut a11 = observer_frame(101, aliased);
    *a11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 101,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 1,
                viewer: 101,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
        ]),
    );
    let a12 = observer_frame(202, split);
    let mut a13 = observer_frame(202, split);
    *a13.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    alias_pass.fold_hour(3, 10, Some(&a10), None, false);
    alias_pass.fold_hour(3, 11, Some(&a11), None, false);
    alias_pass.fold_hour(3, 12, Some(&a12), None, false);
    alias_pass.fold_hour(3, 13, Some(&a13), None, false);
    let alias_idx = index(&alias_pass);
    assert_eq!(flags(&alias_idx, &mut a11, 11, 3).land[&100], Engaged::Yes);
    assert_eq!(
        flags(&alias_idx, &mut a13, 13, 4).land[&100],
        Engaged::Ambiguous(engaged::Ambiguity::Scope)
    );

    let mut narrow_pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let n10 = observer_frame(4, &[]);
    let n11 = observer_frame(3, &[]);
    let mut n12 = observer_frame(3, &[]);
    *n12.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 3,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
        }]),
    );
    let n13 = observer_frame(3, &[]);
    let n14 = observer_frame(4, &[]);
    let mut n15 = observer_frame(4, &[]);
    *n15.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    narrow_pass.fold_hour(3, 10, Some(&n10), None, false);
    narrow_pass.fold_hour(3, 11, Some(&n11), None, false);
    narrow_pass.fold_hour(3, 12, Some(&n12), None, false);
    narrow_pass.fold_hour(3, 13, Some(&n13), None, false);
    narrow_pass.fold_hour(3, 14, Some(&n14), None, false);
    narrow_pass.fold_hour(3, 15, Some(&n15), None, false);
    let narrow_idx = index(&narrow_pass);
    assert_eq!(flags(&narrow_idx, &mut n12, 12, 3).land[&100], Engaged::Yes);
    assert_eq!(flags(&narrow_idx, &mut n15, 15, 4).land[&100], Engaged::No);
}

#[test]
fn playing_country_switch_sweep_without_cache_write_keeps_prior_cached_viewer() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let playing_frame = |country: u32, factions: &[(u32, u32)]| {
        let mut frame = naval_frame(6, &[1, 3], factions, 0, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country,
                observer_country: 0,
            }],
        ));
        frame
    };
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    let frame10 = playing_frame(1, allied);
    let mut frame11 = playing_frame(3, &[]);
    *frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 3,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }]),
    );
    let mut frame12 = playing_frame(3, &[]);
    *frame12.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);
    pass.fold_hour(3, 12, Some(&frame12), Some(10), false);

    let idx = index(&pass);
    assert_eq!(
        flags(&idx, &mut frame11, 11, 3).land[&100],
        Engaged::Ambiguous(engaged::Ambiguity::Order)
    );
    assert_eq!(flags(&idx, &mut frame12, 12, 3).land[&100], Engaged::No);

    let mut written_pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    let mut written11 = playing_frame(3, &[]);
    *written11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 3,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
        }]),
    );
    written_pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    written_pass.fold_hour(3, 11, Some(&written11), Some(10), false);
    written_pass.fold_hour(3, 12, Some(&frame12), Some(10), false);
    let written_idx = index(&written_pass);
    assert_eq!(
        flags(&written_idx, &mut frame12, 12, 3).land[&100],
        Engaged::Ambiguous(engaged::Ambiguity::Scope)
    );
}

#[test]
fn unreadable_hour_gap_preserves_cache_written_viewer_for_next_unavailable_hour() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    let observer_frame = |observer: u32| {
        let mut frame = naval_frame(6, &[1], allied, 0, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: observer,
            }],
        ));
        frame
    };
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    let mut frame10 = observer_frame(3);
    *frame10.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
            target: 4,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
        }]),
    );
    let mut frame11 = observer_frame(3);
    *frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "witness unavailable".into(),
    };
    pass.fold_hour(3, 9, None, None, false);
    pass.fold_hour(3, 10, Some(&frame10), None, false);
    pass.fold_hour(3, 11, Some(&frame11), None, false);

    let idx = index(&pass);
    assert_eq!(flags(&idx, &mut frame10, 10, 3).land[&100], Engaged::Unknown);
    assert_eq!(flags(&idx, &mut frame11, 11, 3).land[&100], Engaged::Yes);
}

#[test]
fn unidentified_hourly_viewer_keeps_independently_witnessed_side_refresh() {
    use replay_data::rows::{CombatEngagementRefreshRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let allied: &[(u32, u32)] = &[(2, 3)];
    for unlinked_tag in [false, true] {
        let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
        let frame10 = naval_frame(6, &[1], allied, 0, [0, 0]);
        let mut frame11 = naval_frame(6, &[1], allied, 0, [0, 0]);
        *frame11.tables[TableId::LocalPlayer.index()] = if unlinked_tag {
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country: 0,
                observer_country: 999,
            }]))
        } else {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
        };
        *frame11.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
            TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 100,
                side: 0,
                viewer: 3,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }]),
        );
        let mut frame12 = naval_frame(6, &[1], allied, 0, [0, 0]);
        pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
        pass.fold_hour(3, 11, Some(&frame11), Some(10), false);
        pass.fold_hour(3, 12, Some(&frame12), Some(10), false);

        let idx = index(&pass);
        {
            let branches = idx.shared.branches.lock().unwrap();
            assert!(!branches[&3].recording_player(11, 1));
            assert!(branches[&3].recording_player(12, 1));
        }
        let served12 = flags(&idx, &mut frame12, 12, 1);
        assert_eq!(served12.land[&100], Engaged::Yes);
    }
}

#[test]
fn unlinked_raw_tag_at_load_matches_secondary_owner_and_carries_when_linked() {
    use replay_data::rows::{CountryLinkRow, LandCombatSideRow, LocalPlayerRow, TableRows};
    use replay_frame::frame::TableView;
    let mut pass = Pass::new(Arc::default(), Country::Tag("C4".into()));
    let frame_with = |linked: bool| {
        let mut frame = naval_frame(6, &[1], &[], 0, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(
            vec![LocalPlayerRow {
                country: 0,
                observer_country: 999,
            }],
        ));
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 100,
                attacker_primary: Vec::new(),
                attacker_secondary: vec![999],
                defender_primary: Vec::new(),
                defender_secondary: Vec::new(),
                ..Default::default()
            }]));
        if linked {
            let mut links: Vec<_> = (1..=6)
                .map(|slot| CountryLinkRow {
                    id: slot,
                    canonical_slot: slot,
                })
                .collect();
            links.push(CountryLinkRow {
                id: 999,
                canonical_slot: 4,
            });
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links));
        }
        frame
    };
    let frame10 = frame_with(false);
    let mut frame11 = frame_with(true);
    pass.fold_hour(3, 10, Some(&frame10), Some(10), false);
    pass.fold_hour(3, 11, Some(&frame11), Some(10), false);

    let idx = index(&pass);
    {
        let branches = idx.shared.branches.lock().unwrap();
        assert!(!branches[&3].recording_player(10, 4));
        assert!(branches[&3].recording_player(11, 4));
    }
    let served11 = flags(&idx, &mut frame11, 11, 4);
    assert_eq!(served11.land[&100], Engaged::Yes);
}

#[test]
fn the_recording_players_icons_keep_the_clients_carried_frame() {
    use replay_data::{rows::NavalCombatShipRow, rows::TableRows};
    use replay_frame::frame::TableView;
    let hour = |hour: Hour, player: u32, bytes: [u8; 2], local_sweep: Option<u64>| {
        let mut frame = naval_frame(6, &[player], &[], 1, bytes);
        frame.hour = hour;
        *frame.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(vec![NavalCombatShipRow {
                combat: 0,
                side: 0,
                live_convoy: 1,
                ..Default::default()
            }]));
        if let Some(target) = local_sweep {
            *frame.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
                TableRows::CombatEngagementRefreshes(vec![
                    replay_data::rows::CombatEngagementRefreshRow {
                        kind: replay_data::rows::CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                        target,
                        side: 0,
                        viewer: 0,
                        triggers: replay_data::rows::CombatEngagementRefreshRow::TRIGGER_RECORDED,
                    },
                ]),
            );
        }
        frame
    };
    let frames = [
        hour(10, 2, [1, 0], None),
        hour(11, 2, [1, 0], None),
        hour(12, 3, [0, 0], Some(3)),
    ];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    for frame in &frames {
        pass.fold_hour(3, frame.hour, Some(frame), Some(10), false);
    }
    let index = index(&pass);
    let mut frame = frames[2].clone();
    let naval = flags(&index, &mut frame, 12, 3).naval;
    let (kept, provenance) =
        kept_frames(&frame, NAVAL_ICON, 3, &naval, Some(index.shared.as_ref())).unwrap();
    assert_eq!(provenance, Provenance::Indexed);
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Frame(1)));
    let mut frame = frames[1].clone();
    let naval = flags(&index, &mut frame, 11, 3).naval;
    let (kept, _) =
        kept_frames(&frame, NAVAL_ICON, 3, &naval, Some(index.shared.as_ref())).unwrap();
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Unobserved));
}

#[test]
fn a_joined_recordings_icon_keeps_the_frame_its_recorded_bytes_set() {
    use replay_data::{rows::NavalCombatShipRow, rows::TableRows, rows::TaskForceRow};
    use replay_frame::frame::TableView;
    let hour = |hour: Hour, factions: &[(u32, u32)], defenders: &[u64], bytes: [u8; 2]| {
        let mut frame = naval_frame(6, &[1], factions, 1, bytes);
        frame.hour = hour;
        *frame.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(vec![NavalCombatShipRow {
                combat: 0,
                side: 0,
                live_convoy: 1,
                ..Default::default()
            }]));
        if let TableView::Complete(TableRows::NavalCombatDetails(rows)) =
            &mut *frame.tables[TableId::NavalCombatDetails.index()]
        {
            rows[0].sides[1].units = defenders.to_vec();
        }
        if let TableView::Complete(TableRows::TaskForces(rows)) =
            &mut *frame.tables[TableId::TaskForces.index()]
        {
            rows.push(TaskForceRow {
                id: 99,
                logical_owner: 5,
                roster_owner: 5,
                ..Default::default()
            });
        }
        frame
    };
    let allied: &[(u32, u32)] = &[(1, 2)];
    let frames = [
        hour(10, allied, &[8, 99], [1, 0]),
        hour(11, &[], &[8, 99], [1, 0]),
        hour(12, &[], &[8], [0, 0]),
    ];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    for frame in &frames {
        pass.fold_hour(3, frame.hour, Some(frame), None, false);
    }
    let index = index(&pass);
    let mut frame = frames[2].clone();
    let naval = flags(&index, &mut frame, 12, 1).naval;
    let (kept, provenance) =
        kept_frames(&frame, NAVAL_ICON, 1, &naval, Some(index.shared.as_ref())).unwrap();
    assert_eq!(provenance, Provenance::Indexed);
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Frame(1)));
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.undecided, summary.agreements, summary.mismatches),
        (4, 2, 0)
    );
}

#[test]
fn mp_switch_and_observer_hours_are_checked_with_a_stable_human_roster() {
    let mut pass = Pass::new(Arc::default(), Country::Recorded);
    for (hour, country, observer_country) in [(10, 1, 0), (11, 2, 0), (12, 0, 5)] {
        let mut frame = naval_frame(6, &[1, 2, 5], &[], 1, [0, 0]);
        *frame.tables[TableId::LocalPlayer.index()] = replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::LocalPlayer(vec![replay_data::rows::LocalPlayerRow {
                country,
                observer_country,
            }]),
        );
        pass.fold_hour(3, hour, Some(&frame), Some(10), false);
    }
    let summary = &pass.check.summary;
    assert_eq!(
        (
            summary.hours,
            summary.naval_hours,
            summary.unidentified_hours
        ),
        (3, 3, 0)
    );
    assert_eq!(
        summary.agreements + summary.mismatches + summary.undecided,
        6
    );
}

#[test]
fn mismatch_counts_stay_exact_while_only_the_first_five_are_kept() {
    const COMBATS: u64 = 10;
    let mismatches = 2 * COMBATS * 48;
    let frame = naval_frame(12, &[1], &[], COMBATS, [1, 1]);
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    for hour in 0..48 {
        pass.fold_hour(3, hour, Some(&frame), Some(0), false);
    }
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (0, mismatches));
    assert_eq!(summary.first.len(), 5);
    assert!(summary.first.iter().all(|mismatch| mismatch.hour == 0));
    assert_eq!(
        summary.lines().last().unwrap(),
        &format!("Naval engaged mismatch: {} more not listed.", mismatches - 5)
    );
}

#[test]
fn an_unengaged_naval_icon_keeps_its_last_engaged_frame_through_the_index() {
    use replay_data::{rows::NavalCombatShipRow, rows::TableRows};
    use replay_frame::frame::TableView;
    let convoy = |frame: &mut DecodedFrame| {
        *frame.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(vec![NavalCombatShipRow {
                combat: 0,
                side: 0,
                live_convoy: 1,
                ..Default::default()
            }]));
    };
    let hour = |hour: Hour, factions: &[(u32, u32)], joined: bool| {
        let mut frame = naval_frame(6, &[1], factions, 1, [1, 0]);
        frame.hour = hour;
        convoy(&mut frame);
        if joined
            && let TableView::Complete(TableRows::NavalCombatDetails(rows)) =
                &mut *frame.tables[TableId::NavalCombatDetails.index()]
        {
            rows[0].sides[0].units.push(99);
        }
        frame
    };
    let frames = [
        hour(10, &[(1, 2)], false),
        hour(11, &[], false),
        hour(12, &[], true),
    ];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    for frame in &frames {
        pass.fold_hour(3, frame.hour, Some(frame), Some(10), false);
    }
    let index = index(&pass);
    let kept = |at: usize| {
        let mut frame = frames[at].clone();
        let hour = frame.hour;
        let naval = flags(&index, &mut frame, hour, 1).naval;
        let (kept, provenance) =
            kept_frames(&frame, NAVAL_ICON, 1, &naval, Some(index.shared.as_ref())).unwrap();
        assert_eq!(provenance, Provenance::Indexed);
        (naval[&0], kept[&0])
    };
    assert_eq!(kept(0), (Engaged::Yes, Kept::Frame(1)));
    assert_eq!(kept(1), (Engaged::Yes, Kept::Frame(1)));
    assert_eq!(kept(2), (Engaged::No, Kept::Frame(1)));
    let (unindexed, provenance) = kept_frames(
        &frames[2],
        NAVAL_ICON,
        1,
        &BTreeMap::from([(0, Engaged::No)]),
        None,
    )
    .unwrap();
    assert_eq!(
        (unindexed[&0], provenance),
        (Kept::Unindexed, Provenance::Unindexed)
    );
}

#[test]
fn a_side_whose_combat_ends_is_reported_once() {
    let snapshot = Arc::new(engaged::Snapshot {
        links: BTreeMap::from([(1, 1)]),
        relations: Ok(Relations::factions_for_tests(&[])),
        zero_visibility: ZeroVisibility::unavailable("unused"),
        controllers: Some(Default::default()),
    });
    let side = || engaged::Side {
        units: Some(vec![1]),
        added: engaged::Added::NoTrigger,
        lists: Some((vec![1], vec![])),
        unit_lists: Some((vec![1], vec![])),
    };
    let mut history = engaged::History::default();
    let (mut changed, mut ended) = (Vec::new(), Vec::new());
    for (hour, sides, expect_changed, expect_ended) in [
        (0, vec![(7, 0), (7, 1)], 2, 0),
        (1, vec![(7, 0), (7, 1)], 0, 0),
        (2, vec![], 0, 2),
    ] {
        history.advance_reporting(
            engaged::Sample {
                viewer: Some(1),
                viewer_raw: None,
                alternate_viewer: None,
                alternate_viewer_raw: None,
                recording: false,
                snapshot: &snapshot,
                refreshes: engaged::RefreshWitness::Unavailable,
                sides: sides.into_iter().map(|key| (key, side())).collect(),
                complete: true,
                removal: engaged::Removal::BothSidesUnlessRoundRetreat,
            },
            if hour == 0 {
                Continuity::Load
            } else {
                Continuity::Next
            },
            &mut changed,
            &mut ended,
        );
        assert_eq!((changed.len(), ended.len()), (expect_changed, expect_ended));
    }
}

#[test]
fn a_lookup_takes_the_last_change_at_or_before_the_hour_for_the_folded_viewer() {
    let shared = Arc::new(History::default());
    let mut covered = Covered::new(10);
    (covered.last, covered.viewers) = (20, vec![(10, 1)]);
    for (hour, side) in [(10, Engaged::No), (11, Engaged::No), (14, Engaged::Yes)] {
        covered
            .fold(false)
            .record(LAND, hour, &[((62, 0), Engaged::No), ((62, 1), side)], &[]);
    }
    assert_eq!(
        covered.folds()[0].points()[LAND][&(62, 1)],
        [(10, Engaged::No), (14, Engaged::Yes)]
    );
    shared.branches.lock().unwrap().insert(3, covered);
    let changed = |from, to| shared.flags_changed(LAND, 3, from, to, 1);
    assert_eq!(changed(10, 14), Some(vec![62]), "side 1 took a new value at 14");
    assert_eq!(changed(14, 20), Some(Vec::new()), "nothing changed after it");
    assert_eq!(changed(10, 21), None, "past the fold every flag is read again");
    let index = EngagedIndex { shared };
    let at = |hour, viewer| index.shared.lookup(LAND, 3, hour, viewer, &[62]);
    assert_eq!(at(13, 1), Ok(BTreeMap::from([(62, Engaged::No)])));
    assert_eq!(at(14, 1), Ok(BTreeMap::from([(62, Engaged::Yes)])));
    assert_eq!(at(21, 1), Err(Provenance::Pending));
    assert_eq!(at(14, 2), Err(Provenance::OtherViewer));
    assert_eq!(
        index.shared.lookup(LAND, 4, 14, 1, &[62]),
        Err(Provenance::Pending)
    );
}

fn roster_frame(hour: Hour, players: Option<&[(i32, u32)]>, allied: bool) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::FactionMemberRow, rows::HumanControllerRow,
        rows::LandCombatRow, rows::LandCombatSideRow, rows::PlayerRow, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let mut set = |table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        TableId::Countries,
        TableRows::Countries(
            (1..=4)
                .zip(["AAA", "BBB", "CCC", "DDD"])
                .map(|(id, tag)| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryLinks,
        TableRows::CountryLinks(
            (1..=4)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        ),
    );
    set(
        TableId::FactionMembers,
        TableRows::FactionMembers(if allied {
            vec![
                FactionMemberRow {
                    country: 1,
                    faction: 9,
                    ordinal: 0,
                },
                FactionMemberRow {
                    country: 2,
                    faction: 9,
                    ordinal: 1,
                },
            ]
        } else {
            vec![]
        }),
    );
    for table in [
        TableId::WarRelations,
        TableId::Subjects,
        TableId::CombatEngagementRefreshes,
    ] {
        set(table, TableRows::empty(table));
    }
    if let Some(players) = players {
        set(
            TableId::Players,
            TableRows::Players(
                players
                    .iter()
                    .map(|&(id, country)| PlayerRow {
                        id,
                        country,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let mut counts = BTreeMap::<u32, i8>::new();
        for &(_, country) in players {
            *counts.entry(country).or_default() += 1;
        }
        set(
            TableId::HumanControllers,
            TableRows::HumanControllers(
                counts
                    .into_iter()
                    .map(|(country, count)| HumanControllerRow { country, count })
                    .collect(),
            ),
        );
    }
    set(
        TableId::LandCombats,
        TableRows::LandCombats(vec![LandCombatRow {
            id: 62,
            attacker_classes: vec![0],
            attackers: vec![7],
            defenders: vec![9],
            ..Default::default()
        }]),
    );
    set(
        TableId::LandCombatSides,
        TableRows::LandCombatSides(vec![LandCombatSideRow {
            combat: 62,
            attacker_primary: vec![2],
            defender_primary: vec![3],
            ..Default::default()
        }]),
    );
    frame
}

fn fold_for_aaa(hours: &[DecodedFrame]) -> (BTreeMap<u64, Engaged>, Provenance) {
    let shared = Arc::new(History::default());
    let country = Country::Tag("AAA".into());
    let mut pass = Pass::new(Arc::clone(&shared), country.clone());
    for frame in hours {
        pass.fold_hour(frame.branch, frame.hour, Some(frame), Some(0), false);
    }
    let index = EngagedIndex { shared };
    let last = hours.last().unwrap();
    let viewer = Viewer::read(last, &country).unwrap();
    combats(
        last,
        LAND,
        viewer.slot.unwrap(),
        &viewer.inputs.links,
        viewer.inputs.relations.as_ref().map_err(|reason| *reason),
        Some(index.shared.as_ref()),
    )
    .unwrap()
}

#[test]
fn a_controller_joining_the_viewer_reaches_the_index_as_a_possible_refresh() {
    use replay_data::rows::{CombatEngagementRefreshRow, TableRows};
    use replay_frame::frame::TableView;
    let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
    let steady = fold_for_aaa(&[
        roster_frame(0, Some(&[(40, 4)]), false),
        roster_frame(1, Some(&[(40, 4)]), true),
        roster_frame(2, Some(&[(40, 4)]), true),
    ]);
    assert_eq!(steady, flag(Engaged::No));

    let mut targeted = roster_frame(2, Some(&[(40, 4)]), true);
    *targeted.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
            target: 1,
            side: 0,
            viewer: 0,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }]),
    );
    assert_eq!(
        fold_for_aaa(&[
            roster_frame(0, Some(&[(40, 4)]), false),
            roster_frame(1, Some(&[(40, 4)]), true),
            targeted,
        ]),
        flag(Engaged::Yes)
    );

    let mut unrecorded = roster_frame(2, Some(&[(40, 1)]), true);
    *unrecorded.tables[TableId::CombatEngagementRefreshes.index()] = TableView::NotRecorded {
        mode: replay_data::scope::CaptureMode::Full,
    };
    let scope = Engaged::Ambiguous(engaged::Ambiguity::Scope);
    assert_eq!(
        fold_for_aaa(&[
            roster_frame(0, Some(&[(40, 4)]), false),
            roster_frame(1, Some(&[(40, 4)]), true),
            unrecorded,
        ]),
        flag(scope)
    );

    let mut unavailable = roster_frame(2, None, true);
    *unavailable.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Unavailable {
        reason: "withheld".into(),
    };
    assert_eq!(
        fold_for_aaa(&[
            roster_frame(0, Some(&[(40, 4)]), false),
            roster_frame(1, Some(&[(40, 4)]), true),
            unavailable,
        ]),
        flag(scope)
    );
}

#[test]
fn a_load_rebuilt_flag_reports_its_provenance() {
    let frames = [
        roster_frame(0, Some(&[(40, 4)]), false),
        roster_frame(1, Some(&[(40, 4)]), true),
    ];
    let viewer = Viewer::read(&frames[1], &Country::Tag("AAA".into())).unwrap();
    let rebuilt = |index: Option<&History>| {
        combats(
            &frames[1],
            LAND,
            viewer.slot.unwrap(),
            &viewer.inputs.links,
            viewer.inputs.relations.as_ref().map_err(|reason| *reason),
            index,
        )
        .unwrap()
    };
    let yes = BTreeMap::from([(62, Engaged::Yes)]);
    assert_eq!(rebuilt(None), (yes.clone(), Provenance::Unindexed));
    let shared = Arc::new(History::default());
    let mut pass = Pass::new(Arc::clone(&shared), Country::Tag("AAA".into()));
    pass.fold_hour(1, 0, Some(&frames[0]), Some(0), false);
    let index = EngagedIndex { shared };
    assert_eq!(
        rebuilt(Some(index.shared.as_ref())),
        (yes.clone(), Provenance::Pending)
    );
    let mut unread = frames[1].clone();
    unread.hour = 0;
    pass.fold_hour(2, 0, None, None, false);
    unread.branch = 2;
    assert_eq!(
        combats(
            &unread,
            LAND,
            viewer.slot.unwrap(),
            &viewer.inputs.links,
            viewer.inputs.relations.as_ref().map_err(|reason| *reason),
            Some(index.shared.as_ref()),
        )
        .unwrap(),
        (yes, Provenance::Unread)
    );
    for provenance in [
        Provenance::Pending,
        Provenance::OtherViewer,
        Provenance::Unread,
        Provenance::Unindexed,
    ] {
        assert!(
            provenance
                .limitation()
                .is_some_and(|line| line.starts_with("engaged flag load-rebuilt; "))
        );
    }
    assert_eq!(Provenance::Indexed.limitation(), None);
}


#[test]
fn a_block_played_by_two_branches_is_read_once() {
    let block = |branch, page: u32, first: Hour| replay::campaign::BlockIndex { branch, first_hour: first, last_hour: first + 719, run: 2, page };
    let index = [block(3, 0, 1000), block(7, 0, 1000), block(3, 1, 1720), block(7, 1, 1720), block(3, 0, 1000)];
    let taken: Vec<(u32, Hour)> = piece_blocks(&index, 2, 1000, 2439).iter().map(|block| (block.page, block.first_hour)).collect();
    assert_eq!(taken, [(0, 1000), (1, 1720)]);
    assert!(piece_blocks(&index, 0, 1000, 2439).is_empty());
    let taken: Vec<u32> = piece_blocks(&index, 2, 1800, 2000).iter().map(|block| block.page).collect();
    assert_eq!(taken, [1]);
}

#[test]
fn combat_engagement_refreshes_update_witness_without_rebuilding_cached_inputs() {
    use replay_data::rows::{CombatEngagementRefreshRow, TableRows};
    use replay_frame::frame::TableView;
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(1, 2)];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let frame10 = naval_frame(6, &[1], none, 0, [0, 0]);
    let frame11 = naval_frame(6, &[1], allied, 0, [0, 0]);
    let mut frame12 = naval_frame(6, &[1], allied, 0, [0, 0]);
    *frame12.tables[TableId::CombatEngagementRefreshes.index()] = TableView::Complete(
        TableRows::CombatEngagementRefreshes(vec![CombatEngagementRefreshRow {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: 100,
            side: 0,
            viewer: 1,
            triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
        }]),
    );
    let frame13 = frame12.clone();

    pass.fold_source(3, 10, Some(&frame10), Some(10), false, false, false);
    pass.fold_source(3, 11, Some(&frame11), Some(10), false, false, false);
    let cached_inputs = Arc::clone(pass.inputs.as_ref().unwrap());
    pass.fold_source(3, 12, Some(&frame12), Some(10), false, true, false);
    assert!(Arc::ptr_eq(pass.inputs.as_ref().unwrap(), &cached_inputs));
    assert!(Arc::ptr_eq(&pass.viewer.as_ref().unwrap().inputs, &cached_inputs));
    pass.fold_source(3, 13, Some(&frame13), Some(10), false, true, true);
    assert!(Arc::ptr_eq(pass.inputs.as_ref().unwrap(), &cached_inputs));

    let idx = index(&pass);
    let mut check11 = frame11.clone();
    let mut check12 = frame12.clone();
    let mut check13 = frame13.clone();
    assert_eq!(flags(&idx, &mut check11, 11, 1).land[&100], Engaged::No);
    assert_eq!(flags(&idx, &mut check12, 12, 1).land[&100], Engaged::Yes);
    assert_eq!(flags(&idx, &mut check13, 13, 1).land[&100], Engaged::Yes);
}
