use super::*;

/// The index keeps one point per value change: 1,500 combats whose units
/// keep changing but whose flags never do cost one point per side,
/// however many hours fold.
#[test]
fn memory_follows_value_changes_not_hours() {
    const COMBATS: u64 = 1_500;
    let links: BTreeMap<u32, u32> = (1..=2 * COMBATS as u32).map(|c| (c, c)).collect();
    let relations = Relations::factions_for_tests(&[]);
    let mut history = engaged::History::default();
    let mut points = Changes::new();
    for hour in 0..240 {
        let sides = (0..COMBATS)
            .flat_map(|combat| {
                [0, 1].map(|side| {
                    (
                        (combat, side),
                        engaged::Side {
                            // A unit joins every tenth hour: a rebuild.
                            units: Some(vec![combat, (hour / 10) as u64]),
                            added: engaged::Added::NoTrigger,
                            lists: Some((vec![2 * combat as u32 + u32::from(side) + 1], vec![])),
                            unit_lists: Some((
                                vec![2 * combat as u32 + u32::from(side) + 1],
                                vec![],
                            )),
                        },
                    )
                })
            })
            .collect();
        let continuity = if hour == 0 {
            Continuity::Load
        } else {
            Continuity::Next
        };
        history.advance(
            engaged::Sample {
                viewer: 0,
                links: &links,
                relations: Ok(&relations),
                controllers: Some(Default::default()),
                sides,
                complete: true,
                removal: engaged::Removal::BothSidesUnlessRoundRetreat,
            },
            continuity,
        );
        record(&mut points, hour, history.sides());
    }
    assert_eq!(points.len(), 2 * COMBATS as usize);
    assert!(
        points
            .values()
            .all(|changes| changes == &[(0, Engaged::No)])
    );
}

fn branch_start(reason: BranchReason, loaded_hour: Option<Hour>) -> BranchStart {
    BranchStart {
        id: 1,
        session: 1,
        loaded_hour,
        first_hour: 20,
        parent: replay::branches::Parentage::Unknown,
        reason,
    }
}

/// Production branch starts: the collector records the loaded hour as
/// the first hour for a load signal and an hour discontinuity alike, and
/// none for a session start. Only a load signal sampled at its loaded
/// hour is a native load: the collector drops the origin after an
/// unreadable callback (spooling `suppress_origin`) while still seeing
/// the new signal, and a first sample after the loaded hour follows
/// unobserved ones.
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

/// A load signal whose origin was lost or precedes the first sample does
/// not rebuild its sides as historical: the pass folds that first hour as
/// a gap, so the flag stays unknown until a side rebuilds, where a load
/// sampled at its loaded hour rebuilds it (No: AAA shares vision with
/// neither side).
#[test]
fn a_load_without_its_origin_folds_its_first_hour_as_a_gap() {
    let fold = |loaded_hour| {
        let start = branch_start(BranchReason::LoadSignal, loaded_hour);
        let mut frame = roster_frame(start.first_hour, Some(&[(40, 4)]), false);
        frame.branch = start.id;
        let shared = Arc::new(Shared::default());
        let country = Country::Tag("AAA".into());
        let mut pass = Pass::new(Arc::clone(&shared), country.clone());
        pass.fold_hour(frame.branch, frame.hour, Some(&frame), native_load(&start));
        let (snapshots, _inbox) = mpsc::channel();
        let index = EngagedIndex {
            country,
            shared,
            snapshots,
            folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
        };
        let viewer = Viewer::read(&frame, &index.country).unwrap();
        combats(
            &frame,
            LAND,
            viewer.slot,
            &viewer.links,
            viewer.relations.as_ref().map_err(|reason| *reason),
            Some(&index),
        )
        .unwrap()
    };
    let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
    assert_eq!(fold(Some(20)), flag(Engaged::No));
    assert_eq!(fold(None), flag(Engaged::Unknown));
    assert_eq!(fold(Some(19)), flag(Engaged::Unknown));
}

#[test]
fn five_thousand_countries_resolve_and_fold() {
    use replay::{
        frame::TableView,
        rows::{CountryLinkRow, CountryRow, TableRows},
    };
    const COUNTRIES: u32 = 5_000;
    const COMBATS: u64 = 1_500;
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..COUNTRIES)
            .map(|slot| CountryRow {
                id: slot + 1,
                canonical_slot: slot,
                tag: format!("C{slot:04}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..COUNTRIES)
            .map(|slot| CountryLinkRow {
                id: slot + 1,
                canonical_slot: slot,
            })
            .collect(),
    ));
    for table in [
        TableId::Players,
        TableId::HumanControllers,
        TableId::FactionMembers,
        TableId::WarRelations,
        TableId::Subjects,
    ] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let started = Instant::now();
    let viewer = Viewer::read(&frame, &Country::Tag("c4999".into())).unwrap();
    let read = started.elapsed();
    assert_eq!(viewer.slot, COUNTRIES - 1);
    assert_eq!(viewer.links.len(), COUNTRIES as usize);
    let mut history = engaged::History::default();
    let mut points = Changes::new();
    let (mut changed, mut ended) = (Vec::new(), Vec::new());
    let started = Instant::now();
    for hour in 0..48 {
        let sides = (0..COMBATS)
            .flat_map(|combat| {
                [0_u8, 1].map(|side| {
                    (
                        (combat, side),
                        engaged::Side {
                            units: Some(vec![combat]),
                            added: engaged::Added::NoTrigger,
                            lists: Some((
                                vec![(combat as u32 * 3 + u32::from(side)) % COUNTRIES + 1],
                                vec![],
                            )),
                            unit_lists: Some((
                                vec![(combat as u32 * 3 + u32::from(side)) % COUNTRIES + 1],
                                vec![],
                            )),
                        },
                    )
                })
            })
            .collect();
        let continuity = if hour == 0 {
            Continuity::Load
        } else {
            Continuity::Next
        };
        history.advance_reporting(
            engaged::Sample {
                viewer: viewer.slot,
                links: &viewer.links,
                relations: viewer.relations.as_ref().map_err(|reason| *reason),
                controllers: viewer.controllers.clone(),
                sides,
                complete: true,
                removal: engaged::Removal::BothSidesUnlessRoundRetreat,
            },
            continuity,
            &mut changed,
            &mut ended,
        );
        record(&mut points, hour, changed.iter().copied());
        // After the first hour nothing changes, so nothing is appended
        // under the lock.
        assert_eq!(
            changed.len(),
            if hour == 0 { 2 * COMBATS as usize } else { 0 }
        );
        assert!(ended.is_empty());
    }
    println!(
        "5,000 countries: viewer read {:.3} ms; 48 hours of 1,500 combats folded in {:.3} ms",
        read.as_secs_f64() * 1e3,
        started.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(points.len(), 2 * COMBATS as usize);
    // No side names the viewer and none shares vision with it: one
    // point per side.
    assert!(points.values().all(|changes| changes.len() == 1));
}

/// 140E69830's battle picture needs one engaged entry of the province's
/// vector, of any class; an undecided entry before it leaves it undecided
/// only when no later entry decides it.
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
    // The entry's class picks the kind: a naval flag under the land entry's id is not it.
    let crossed = CombatFlags {
        naval: BTreeMap::from([(2, Engaged::Yes)]),
        land: BTreeMap::from([(1, Engaged::Yes)]),
        ..CombatFlags::default()
    };
    assert_eq!(crossed.engaged_entries(Some(&row), true, 1), None);
}

/// Countries at slots 1..=`countries` (raw id = slot, tag `C<slot>`), the
/// `players` rows, `factions` pairs allied in one faction each, and
/// `combats` naval combats `c`: attackers task force `2c + 7` of country
/// `2 + c`, defenders task force `2c + 8` of country 5, both sides' stored
/// bytes `bytes`. Land combat 100 always stands beside them: division 300
/// of country 2 attacks division 301 of country 5.
fn naval_frame(
    countries: u32,
    players: &[u32],
    factions: &[(u32, u32)],
    combats: u64,
    bytes: [u8; 2],
) -> DecodedFrame {
    use replay::{
        frame::TableView,
        rows::{
            CountryLinkRow, CountryRow, FactionMemberRow, NavalCombatDetailsRow, NavalCombatRow,
            PlayerRow, TableRows, TaskForceRow,
        },
    };
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
    set(TableRows::LocalPlayer(vec![replay::rows::LocalPlayerRow {
        country: players.first().copied().unwrap_or(0),
        observer_country: 0,
    }]));
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
    set(TableRows::LandCombats(vec![replay::rows::LandCombatRow {
        id: 100,
        attackers: vec![300],
        defenders: vec![301],
        ..Default::default()
    }]));
    set(TableRows::LandCombatSides(vec![
        replay::rows::LandCombatSideRow {
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

/// A pass for the perspective `tag`, folding `hours` of combat 0 (whose
/// attacker is country 2) on branch 3 from a load at the first; each hour
/// is (hour, players, factions, bytes).
#[allow(clippy::type_complexity)]
fn fold_hours(tag: &str, hours: &[(Hour, &[u32], &[(u32, u32)], [u8; 2])]) -> Pass {
    let mut pass = Pass::new(Arc::default(), Country::Tag(tag.into()));
    let load = hours.first().map(|hour| hour.0);
    for &(hour, players, factions, bytes) in hours {
        let frame = naval_frame(6, players, factions, 1, bytes);
        pass.fold_hour(3, hour, Some(&frame), load);
    }
    pass
}

fn index(pass: &Pass, tag: &str) -> EngagedIndex {
    let (snapshots, _inbox) = mpsc::channel();
    EngagedIndex {
        country: Country::Tag(tag.into()),
        shared: Arc::clone(&pass.shared),
        snapshots,
        folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
    }
}

/// The flags `CombatFlags::read` serves `viewer` at `hour` of that frame.
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
        Some(index),
    )
    .unwrap()
}

/// Naval combat 0's flag of [`flags`].
fn naval_flag(index: &EngagedIndex, frame: &mut DecodedFrame, hour: Hour, viewer: u32) -> Engaged {
    flags(index, frame, hour, viewer).naval[&0]
}

/// The recording player goes A (1), B (2), A while A and B ally in
/// between, with no unit change. The client rebuilt every side for B and
/// again for A on its return (140BA9890 → 1413CEBC0 → 1413CEBE0), so A
/// is engaged through its new ally at the last hour: a fixed-A view
/// serves that, not its own held answer, and it agrees with the byte.
#[test]
fn a_viewer_who_is_the_recording_player_takes_its_player_change_rebuilds() {
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(1, 2)];
    let pass = fold_hours(
        "C1",
        &[
            (10, &[1], none, [0, 0]),
            (11, &[2], none, [1, 0]),
            (12, &[2], allied, [1, 0]),
            (13, &[1], allied, [1, 0]),
        ],
    );
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (8, 0));
    let index = index(&pass, "C1");
    let mut frame = naval_frame(6, &[1], allied, 1, [1, 0]);
    // Land combat 100 has the same sides: A is served its rebuilt Yes for
    // both kinds.
    let served = flags(&index, &mut frame, 13, 1);
    assert_eq!(
        (served.naval[&0], served.land[&100]),
        (Engaged::Yes, Engaged::Yes)
    );
    // The perspective's own history held its load-hour answer until the
    // roster moved a player into its country, which it can only take as a
    // possible controller refresh (14113E4B0 then 140BABC00).
    let branches = pass.shared.branches.lock().unwrap();
    for (kind, combat) in [(NAVAL, 0), (LAND, 100)] {
        let points = &branches[&3].points[kind][&(combat, 0)];
        assert_eq!(at(points, 12), Some(Engaged::No));
        assert_eq!(
            at(points, 13),
            Some(Engaged::Ambiguous(engaged::Ambiguity::Scope))
        );
    }
}

/// Another viewer's flags take no rebuild from the recording player's
/// change: country 5 stays unengaged on the attacking side it allied
/// with at the player change, until that side rebuilds.
#[test]
fn a_player_change_rebuilds_nothing_for_another_viewer() {
    let none: &[(u32, u32)] = &[];
    let allied: &[(u32, u32)] = &[(2, 5)];
    let pass = fold_hours(
        "C5",
        &[(10, &[1], none, [0, 0]), (11, &[2], allied, [1, 1])],
    );
    let index = index(&pass, "C5");
    let mut frame = naval_frame(6, &[2], allied, 1, [1, 1]);
    // Country 5 defends: engaged either way, so read the attacking side,
    // of the naval combat and of the land one beside it.
    let branches = pass.shared.branches.lock().unwrap();
    for (kind, combat) in [(NAVAL, 0), (LAND, 100)] {
        assert_eq!(
            at(&branches[&3].points[kind][&(combat, 0)], 11),
            Some(Engaged::No)
        );
    }
    drop(branches);
    assert_eq!(naval_flag(&index, &mut frame, 11, 5), Engaged::Yes);
    // The recording player (2) is engaged as the attacker, and agrees; its
    // defending side's player-change rebuild cannot be ordered against the
    // alliance, so that side is undecided.
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.undecided), (3, 1));
}

/// Viewing a fixed B (3): the recording player changes from A (1) to B
/// inside an unreadable block, and B then allies with both combats'
/// attacker (2) with no unit change. The rebuild for B happened at the
/// hidden change, before the alliance, and the alliance refreshes
/// nothing, so on recovery neither history may evaluate it: land stays
/// unknown, and the naval flag for B, the recording player, is the
/// recorded byte.
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
    );
    pass.failed_block(3, 19);
    let mut frame = naval_frame(6, &[3], allied, 1, [0, 0]);
    pass.fold_hour(3, 20, Some(&frame), Some(10));
    let index = index(&pass, "C3");
    let served = flags(&index, &mut frame, 20, 3);
    assert_eq!(served.land[&100], Engaged::Unknown);
    assert_eq!(served.naval[&0], Engaged::No);
    let summary = &pass.check.summary;
    assert_eq!((summary.failed_blocks, summary.undecided), (1, 2));
}

/// A byte the derivation contradicts is a named mismatch; a side the
/// derivation cannot decide (the branch joined a running game) is
/// counted apart, and the byte settles it for the recording player.
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
    // Without a load the first hour follows unobserved rebuilds.
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    let mut frame = naval_frame(6, &[1], none, 1, [0, 1]);
    pass.fold_hour(3, 10, Some(&frame), None);
    assert_eq!(pass.check.summary.undecided, 2);
    let index = index(&pass, "C1");
    assert_eq!(naval_flag(&index, &mut frame, 10, 1), Engaged::Yes);
    // Another viewer (3, on neither side) keeps its own derivation, not the byte.
    assert_eq!(naval_flag(&index, &mut frame, 10, 3), Engaged::No);
}

/// Hours the check could not compare are counted, never mistaken for an
/// hour without naval combat.
#[test]
fn unread_and_unnamed_hours_are_counted_through_the_pass() {
    let none: &[(u32, u32)] = &[];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    // An undecodable hour.
    pass.fold_hour(3, 10, None, Some(10));
    // Country links unavailable while naval combats are recorded: neither
    // the perspective nor the recording player resolves.
    let mut frame = naval_frame(6, &[1], none, 1, [0, 0]);
    *frame.tables[TableId::CountryLinks.index()] = replay::frame::TableView::Unavailable {
        reason: "not captured".into(),
    };
    pass.fold_hour(3, 11, Some(&frame), Some(10));
    // A failed client read is unidentified even with two humans.
    let mut frame = naval_frame(6, &[1, 5], none, 1, [0, 0]);
    *frame.tables[TableId::LocalPlayer.index()] = replay::frame::TableView::Unavailable {
        reason: "not captured".into(),
    };
    pass.fold_hour(3, 12, Some(&frame), Some(10));
    // An unreadable block.
    pass.failed_block(3, 20);
    pass.fold_hour(
        3,
        21,
        Some(&naval_frame(6, &[1], none, 0, [0, 0])),
        Some(10),
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
    // Only a fully read span without naval sides says it had none.
    let mut clean = Pass::new(Arc::default(), Country::Tag("C1".into()));
    clean.fold_hour(
        3,
        10,
        Some(&naval_frame(6, &[1], none, 0, [0, 0])),
        Some(10),
    );
    assert!(clean.check.summary.lines()[0].contains("no naval combat side in the 1 folded hours"));
}

/// The recording client observes (`CGameState+0x520` zero, so the view is
/// `+0x524`) and another player's `observe` moves its view from 3 to 4
/// between hours 10 and 11. `CChangeCountryControllerCommand::Execute`
/// (14113E4B0) refreshes every combat (140BABC00) only when the viewing
/// country is the command's target country (14113FC98..14113FCCD), and an
/// observe's target is none, so nothing rebuilds at once: the hour-11
/// sample still holds 3's flags, and the next tick's `CCombatManager`
/// hourly check (140BA9890) rebuilds for 4, which hour 12 shows. Country 3
/// allies with the attacker (2); 4 with no one. A tag switch, whose target
/// is the new country, rebuilds at once instead.
#[test]
fn an_observer_view_set_without_a_rebuild_shows_one_sample_later() {
    let allied: &[(u32, u32)] = &[(2, 3)];
    let fold = |views: [(u32, u32, [u8; 2]); 3]| {
        let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
        for (hour, (country, observer_country, bytes)) in (10..).zip(views) {
            let mut frame = naval_frame(6, &[1], allied, 1, bytes);
            *frame.tables[TableId::LocalPlayer.index()] =
                replay::frame::TableView::Complete(replay::rows::TableRows::LocalPlayer(vec![
                    replay::rows::LocalPlayerRow {
                        country,
                        observer_country,
                    },
                ]));
            pass.fold_hour(3, hour, Some(&frame), Some(10));
        }
        pass
    };
    // The observer's bytes: 3's flags at 10 and 11, 4's at 12.
    let pass = fold([(0, 3, [1, 0]), (0, 4, [1, 0]), (0, 4, [0, 0])]);
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.agreements, summary.mismatches, summary.undecided),
        (6, 0, 0),
        "{:?}",
        summary.first
    );
    // Viewing 4, the recording's client, is served its stale flag at 11.
    let index = index(&pass, "C1");
    let mut frame = naval_frame(6, &[1], allied, 1, [1, 0]);
    *frame.tables[TableId::LocalPlayer.index()] =
        replay::frame::TableView::Complete(replay::rows::TableRows::LocalPlayer(vec![
            replay::rows::LocalPlayerRow {
                country: 0,
                observer_country: 4,
            },
        ]));
    assert_eq!(naval_flag(&index, &mut frame, 11, 4), Engaged::Yes);
    assert_eq!(naval_flag(&index, &mut frame, 12, 4), Engaged::No);
    // A tag switch to 4 rebuilds at once: 4's flags already at 11.
    let pass = fold([(3, 0, [1, 0]), (4, 0, [0, 0]), (4, 0, [0, 0])]);
    let summary = &pass.check.summary;
    assert_eq!((summary.agreements, summary.mismatches), (6, 0));
    // The client's own `observe` (14026D810, target none) from playing 3 to
    // observing 4 (3 annexed, so 14113F747..14113F9B7 moves the view) skips
    // 140BABC00 as well: 3's flags at 11, 4's at 12.
    let pass = fold([(3, 0, [1, 0]), (0, 4, [1, 0]), (0, 4, [0, 0])]);
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.agreements, summary.mismatches),
        (6, 0),
        "{:?}",
        summary.first
    );
}

/// The client's combat icons take their frames from its own flag
/// (`1418E3E30` reads the client's `+0xBA` through 1413CD880), so a viewer
/// who is the recording player at the hour is served the icon history
/// folded over the client's flags, not the perspective's. Viewing a fixed
/// C3: the client plays the attacker (2) at hours 10 and 11, engaged, its
/// convoy's frame 1 set; at 12 it switches to C3 (at once, 140BABC00), not
/// allied with 2, so the combat is unengaged and the icon keeps frame 1.
/// C3's own history never saw the combat engaged.
#[test]
fn the_recording_players_icons_keep_the_clients_carried_frame() {
    use replay::{
        frame::TableView,
        rows::{NavalCombatShipRow, TableRows},
    };
    let hour = |hour: Hour, player: u32, bytes: [u8; 2]| {
        let mut frame = naval_frame(6, &[player], &[], 1, bytes);
        frame.hour = hour;
        *frame.tables[TableId::NavalCombatShips.index()] =
            TableView::Complete(TableRows::NavalCombatShips(vec![NavalCombatShipRow {
                combat: 0,
                side: 0,
                live_convoy: 1,
                ..Default::default()
            }]));
        frame
    };
    let frames = [
        hour(10, 2, [1, 0]),
        hour(11, 2, [1, 0]),
        hour(12, 3, [0, 0]),
    ];
    let mut pass = Pass::new(Arc::default(), Country::Tag("C3".into()));
    for frame in &frames {
        pass.fold_hour(3, frame.hour, Some(frame), Some(10));
    }
    let index = index(&pass, "C3");
    let mut frame = frames[2].clone();
    let naval = flags(&index, &mut frame, 12, 3).naval;
    let (kept, provenance) = kept_frames(&frame, NAVAL_ICON, 3, &naval, Some(&index)).unwrap();
    assert_eq!(provenance, Provenance::Indexed);
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Frame(1)));
    // Before the switch C3 is not the recording player: its own icon
    // history, which never saw the combat engaged.
    let mut frame = frames[1].clone();
    let naval = flags(&index, &mut frame, 11, 3).naval;
    let (kept, _) = kept_frames(&frame, NAVAL_ICON, 3, &naval, Some(&index)).unwrap();
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Unobserved));
}

/// A recording that joins a running game (no load) starts every flag
/// unknown, but the client's recorded byte (`CCombatant+0xBA`) says
/// whether it saw the combat engaged, and `1418E3E30` sets the icon frame
/// from that byte and keeps it after. The client (1) allies with the
/// attacker (2) at hours 10 and 11 (bytes [1, 0]); the alliance ends at 11,
/// and at 12 a defending task force leaves (a naval removal rebuilds both
/// sides, 1413CE250), so the combat is unengaged with both sides still
/// holding countries: the icon keeps the frame the engaged hours set.
#[test]
fn a_joined_recordings_icon_keeps_the_frame_its_recorded_bytes_set() {
    use replay::{
        frame::TableView,
        rows::{NavalCombatShipRow, TableRows, TaskForceRow},
    };
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
        // No load: the branch joined a running game.
        pass.fold_hour(3, frame.hour, Some(frame), None);
    }
    let index = index(&pass, "C1");
    let mut frame = frames[2].clone();
    let naval = flags(&index, &mut frame, 12, 1).naval;
    let (kept, provenance) = kept_frames(&frame, NAVAL_ICON, 1, &naval, Some(&index)).unwrap();
    assert_eq!(provenance, Provenance::Indexed);
    assert_eq!((naval[&0], kept[&0]), (Engaged::No, Kept::Frame(1)));
    // The byte check keeps the independent derivation: the joined hours
    // are undecided, the rebuilt one agrees.
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
        *frame.tables[TableId::LocalPlayer.index()] =
            replay::frame::TableView::Complete(replay::rows::TableRows::LocalPlayer(vec![
                replay::rows::LocalPlayerRow {
                    country,
                    observer_country,
                },
            ]));
        pass.fold_hour(3, hour, Some(&frame), Some(10));
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

/// Both tracks and the check at 1,500 combats between 5,000 countries.
#[test]
fn the_check_folds_fifteen_hundred_combats_of_five_thousand_countries() {
    for countries in [1_500_u32, 5_000] {
        let frame = naval_frame(countries, &[1], &[], 1_500, [0, 0]);
        let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
        let started = Instant::now();
        for hour in 0..48 {
            pass.fold_hour(3, hour, Some(&frame), Some(0));
        }
        let elapsed = started.elapsed();
        println!(
            "{countries} countries: 48 hours of 1,500 naval combats folded and checked in {:.3} ms ({:.3} ms an hour)",
            elapsed.as_secs_f64() * 1e3,
            elapsed.as_secs_f64() * 1e3 / 48.0
        );
        let summary = &pass.check.summary;
        assert_eq!(summary.agreements, 2 * 1_500 * 48);
        assert_eq!(summary.naval_hours, 48);
    }
}

/// Every side-hour contradicting its byte is counted exactly, and only the first five are
/// kept by name: 1,500 combats over 48 hours are 144,000 mismatches in a summary that
/// stays five entries long.
#[test]
fn mismatch_counts_stay_exact_while_only_the_first_five_are_kept() {
    let frame = naval_frame(5_000, &[1], &[], 1_500, [1, 1]);
    let mut pass = Pass::new(Arc::default(), Country::Tag("C1".into()));
    for hour in 0..48 {
        pass.fold_hour(3, hour, Some(&frame), Some(0));
    }
    let summary = &pass.check.summary;
    assert_eq!(
        (summary.agreements, summary.mismatches),
        (0, 2 * 1_500 * 48)
    );
    assert_eq!(summary.first.len(), 5);
    assert!(summary.first.iter().all(|mismatch| mismatch.hour == 0));
    assert_eq!(
        summary.lines().last().unwrap(),
        &format!(
            "Naval engaged mismatch: {} more not listed.",
            2 * 1_500 * 48 - 5
        )
    );
}

/// `1418E3E30` sets the icon's frame only
/// while `any_side_engaged@1413CD880` holds, so an unengaged combat's icon
/// keeps the frame of its last engaged hour, which the index carries. C1
/// shares vision with the attacker (2) at the load, so combat 0 is engaged
/// and its convoy (a live forward-group convoy on the attacking side, no
/// other ship) sets the convoy variant, frame 1. The alliance ends at
/// hour 11 (the flag lags, still engaged) and a task force joins the
/// attackers at hour 12, which rebuilds them for C1: unengaged, the icon
/// keeps frame 1, carried, at that hour and on a seek back to it.
#[test]
fn an_unengaged_naval_icon_keeps_its_last_engaged_frame_through_the_index() {
    use replay::{
        frame::TableView,
        rows::{NavalCombatShipRow, TableRows},
    };
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
        pass.fold_hour(3, frame.hour, Some(frame), Some(10));
    }
    let index = index(&pass, "C1");
    let kept = |at: usize| {
        let mut frame = frames[at].clone();
        let hour = frame.hour;
        let naval = flags(&index, &mut frame, hour, 1).naval;
        let (kept, provenance) = kept_frames(&frame, NAVAL_ICON, 1, &naval, Some(&index)).unwrap();
        assert_eq!(provenance, Provenance::Indexed);
        (naval[&0], kept[&0])
    };
    assert_eq!(kept(0), (Engaged::Yes, Kept::Frame(1)));
    assert_eq!(kept(1), (Engaged::Yes, Kept::Frame(1)));
    assert_eq!(kept(2), (Engaged::No, Kept::Frame(1)));
    // Without the index only the hour is known: an unengaged icon's frame is unindexed.
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
    let links = BTreeMap::from([(1, 1)]);
    let relations = Relations::factions_for_tests(&[]);
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
                viewer: 1,
                links: &links,
                relations: Ok(&relations),
                controllers: Some(Default::default()),
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
    let shared = Arc::new(Shared::default());
    let mut points = BTreeMap::new();
    record(
        &mut points,
        10,
        [((62, 0), Engaged::No), ((62, 1), Engaged::No)].into_iter(),
    );
    record(
        &mut points,
        11,
        [((62, 0), Engaged::No), ((62, 1), Engaged::No)].into_iter(),
    );
    record(
        &mut points,
        14,
        [((62, 0), Engaged::No), ((62, 1), Engaged::Yes)].into_iter(),
    );
    assert_eq!(points[&(62, 1)], [(10, Engaged::No), (14, Engaged::Yes)]);
    shared.branches.lock().unwrap().insert(
        3,
        Covered {
            points: vec![points],
            viewers: vec![(10, 1)],
            last: 20,
            ..Covered::new(10)
        },
    );
    let (snapshots, _inbox) = mpsc::channel();
    let index = EngagedIndex {
        country: Country::Recorded,
        shared,
        snapshots,
        folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
    };
    let at = |hour, viewer| index.lookup(LAND, 3, hour, viewer, &[62]);
    assert_eq!(at(13, 1), Ok(BTreeMap::from([(62, Engaged::No)])));
    assert_eq!(at(14, 1), Ok(BTreeMap::from([(62, Engaged::Yes)])));
    // Past the fold, another viewer or another branch is not covered.
    assert_eq!(at(21, 1), Err(Provenance::Pending));
    assert_eq!(at(14, 2), Err(Provenance::OtherViewer));
    assert_eq!(
        index.lookup(LAND, 4, 14, 1, &[62]),
        Err(Provenance::Pending)
    );
}

/// Countries 1..=4 (slot = id, tags AAA to DDD) and land combat 62:
/// country 2 attacks country 3. `players` are (key, country), `None`
/// leaving `Players` unrecorded; `allied` puts 1 and 2 in one faction.
fn roster_frame(hour: Hour, players: Option<&[(i32, u32)]>, allied: bool) -> DecodedFrame {
    use replay::{
        frame::TableView,
        rows::{
            CountryLinkRow, CountryRow, FactionMemberRow, HumanControllerRow, LandCombatRow,
            LandCombatSideRow, PlayerRow, TableRows,
        },
    };
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
    for table in [TableId::WarRelations, TableId::Subjects] {
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

/// Folds `hours` for AAA from a load at hour 0 and answers the last one
/// through [`combats`], as the window does.
fn fold_for_aaa(hours: &[DecodedFrame]) -> (BTreeMap<u64, Engaged>, Provenance) {
    let shared = Arc::new(Shared::default());
    let country = Country::Tag("AAA".into());
    let mut pass = Pass::new(Arc::clone(&shared), country.clone());
    for frame in hours {
        pass.fold_hour(frame.branch, frame.hour, Some(frame), Some(0));
    }
    let (snapshots, _inbox) = mpsc::channel();
    let index = EngagedIndex {
        country,
        shared,
        snapshots,
        folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
    };
    let last = hours.last().unwrap();
    let viewer = Viewer::read(last, &index.country).unwrap();
    combats(
        last,
        LAND,
        viewer.slot,
        &viewer.links,
        viewer.relations.as_ref().map_err(|reason| *reason),
        Some(&index),
    )
    .unwrap()
}

/// Player 40 moving from DDD to AAA may be a controller change into the
/// viewer's country (14113E4B0 then 140BABC00), which would refresh the
/// flag the earlier alliance left stale: the rosters make it ambiguous,
/// never Yes. Unrecorded rosters cannot rule it out either.
#[test]
fn a_controller_joining_the_viewer_reaches_the_index_as_a_possible_refresh() {
    let fold = |last: Option<&[(i32, u32)]>| {
        fold_for_aaa(&[
            roster_frame(0, Some(&[(40, 4)]), false),
            roster_frame(1, Some(&[(40, 4)]), true),
            roster_frame(2, last, true),
        ])
    };
    let flag = |engaged| (BTreeMap::from([(62, engaged)]), Provenance::Indexed);
    assert_eq!(fold(Some(&[(40, 4)])), flag(Engaged::No));
    let scope = Engaged::Ambiguous(engaged::Ambiguity::Scope);
    assert_eq!(fold(Some(&[(40, 1)])), flag(scope));
    assert_eq!(fold(None), flag(scope));
}

/// A flag the index does not answer is still the load rebuild, and says
/// why: not folded yet, no index at all, or no viewer read on the branch.
#[test]
fn a_load_rebuilt_flag_reports_its_provenance() {
    let frames = [
        roster_frame(0, Some(&[(40, 4)]), false),
        roster_frame(1, Some(&[(40, 4)]), true),
    ];
    let viewer = Viewer::read(&frames[1], &Country::Tag("AAA".into())).unwrap();
    let rebuilt = |index: Option<&EngagedIndex>| {
        combats(
            &frames[1],
            LAND,
            viewer.slot,
            &viewer.links,
            viewer.relations.as_ref().map_err(|reason| *reason),
            index,
        )
        .unwrap()
    };
    let yes = BTreeMap::from([(62, Engaged::Yes)]);
    assert_eq!(rebuilt(None), (yes.clone(), Provenance::Unindexed));
    let shared = Arc::new(Shared::default());
    let mut pass = Pass::new(Arc::clone(&shared), Country::Tag("AAA".into()));
    pass.fold_hour(1, 0, Some(&frames[0]), Some(0));
    let (snapshots, _inbox) = mpsc::channel();
    let index = EngagedIndex {
        country: Country::Tag("AAA".into()),
        shared,
        snapshots,
        folding: Mutex::new(Arc::new(crate::playback::tests::fixture())),
    };
    assert_eq!(rebuilt(Some(&index)), (yes.clone(), Provenance::Pending));
    // An unreadable first hour folds no viewer.
    let mut unread = frames[1].clone();
    unread.hour = 0;
    pass.fold_hour(2, 0, None, None);
    unread.branch = 2;
    assert_eq!(
        combats(
            &unread,
            LAND,
            viewer.slot,
            &viewer.links,
            viewer.relations.as_ref().map_err(|reason| *reason),
            Some(&index),
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

/// A newer snapshot keeps the folded index only when every folded block
/// has the same bytes and every branch start is the same, not merely the
/// same block positions.
#[test]
fn a_snapshot_extends_the_index_only_with_the_same_blocks_and_branch_starts() {
    use replay::spool::Record;
    let open =
        |records: Vec<Record>| Replay::from_records(b"campaign".to_vec(), &[records]).unwrap();
    let records = || crate::playback::tests::records(1, &[100, 101, 102]);
    let older = Arc::new(open(records()));
    let same = Arc::new(Replay::open(older.bytes().to_vec()).unwrap());
    // Equal metadata, one block byte changed.
    let mut bytes = older.bytes().to_vec();
    let block = &older.index[0];
    bytes[(block.offset + block.length - 1) as usize] ^= 1;
    let changed = Arc::new(Replay::open(bytes).unwrap());
    assert_eq!(
        (&changed.index, &changed.branches),
        (&older.index, &older.branches)
    );
    // The same blocks, but the branch now starts with a load signal.
    let mut loaded = records();
    for record in &mut loaded {
        if let Record::BranchStart(start) = record {
            start.reason = BranchReason::LoadSignal;
        }
    }
    let reloaded = Arc::new(open(loaded));
    assert_eq!(reloaded.index, older.index);
    let index = EngagedIndex::start(Arc::clone(&older), Country::Recorded).unwrap();
    assert!(!index.extend(&changed));
    assert!(!index.extend(&reloaded));
    assert!(index.extend(&same));
}
