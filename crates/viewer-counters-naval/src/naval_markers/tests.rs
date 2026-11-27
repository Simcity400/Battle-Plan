use super::{
    Allegiance, Arc, BTreeMap, Census, Combat, CombatFlags, CounterColorMode, CounterDefines,
    CounterReport, Countries, Decision, DecodedFrame, Direct, Engaged, ForceInput, FrameSource,
    Icon, Kept, MarkerContext, NAVAL_RESULT_SPRITES, NavalCombatDetailsRow, NavalContext,
    NavalMarkerTables, NavalProvinceCombatRow, NavalResultRow, Omitted, ProvinceKind,
    REFUSED_RESULT_WINDOW, ResultFacts, ResultTables, Sources, Transfers, UiColors, Units, Viewer,
    Vision, allegiance, combat_pass, convoy_color, convoy_result_for_test, decide, grid, hq_frame,
    occurrences, refused_results, result_presentation, rotates, rows, tables, two_engaged,
    unconstructed,
};
use viewer_native_gui::sprites;
use viewer_native_gui::sprites::layer;
use viewer_unit_reads::counter_inputs::NativeInputs;
use replay_data::rows::NavalCombatShipRow;
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_engagements::engaged::Ambiguity;
use viewer_engagements::engagement_history::Provenance;

#[test]
fn the_convoy_addition_takes_the_counter_colour_mode() {
    let tints = allegiance::Tints::FIXTURE;
    let countries = Countries {
        index: None,
        tags: BTreeMap::new(),
        flag_tags: BTreeMap::new(),
        sides: BTreeMap::new(),
        groups: BTreeMap::new(),
        ui_colors: UiColors {
            colors: BTreeMap::from([(3, Some([0.5, 0.25, 0.125, 1.0]))]),
            inheritance: viewer_foundation::color_inheritance::Inheritance {
                inherited: BTreeMap::from([(4, 3)]),
                ..Default::default()
            },
            gap: "CountryAppearance has no row for it".into(),
            unavailable: None,
        },
    };
    let color = |mode, first| convoy_color(mode, &tints, &countries, Allegiance::Enemy, first);
    let (allegiance, country) = (CounterColorMode::Allegiance, CounterColorMode::Country);
    assert_eq!(color(allegiance, Some(Ok(3))), Ok(tints.enemy));
    assert_eq!(color(allegiance, None), Ok(tints.enemy));
    assert_eq!(color(country, Some(Ok(3))), Ok([0.5, 0.25, 0.125, 1.0]));
    assert_eq!(color(country, Some(Ok(4))), Ok([0.5, 0.25, 0.125, 1.0]));
    assert_eq!(color(country, None), Ok(tints.other));
    assert!(
        color(country, Some(Ok(9)))
            .unwrap_err()
            .contains("country slot 9's UI colour")
    );
    assert!(color(country, Some(Err(77))).unwrap_err().contains("77"));
}

#[test]
fn a_result_icon_takes_an_ordinal_past_the_combat_icons() {
    let priority = 5;
    let mut report = CounterReport {
        land_combats: 1,
        ..Default::default()
    };
    let combat = report.combat_ordinal();
    assert_eq!(combat, 1);
    report.naval_combats += 1;
    let result = report.combat_ordinal();
    assert_eq!(result, 2);
    let last_child = layer(priority, combat, sprites::CHILD_SLOTS - 1);
    assert!(layer(priority, result, 0) > last_child);
    report.naval_result_icons += 1;
    assert_eq!(report.combat_ordinal(), 3);
}

#[test]
fn the_combat_pass_runs_strictly_below_the_contents_cutoff() {
    let mut defines = CounterDefines::FIXTURE;
    let cutoff = defines.naval_combat_cutoff;
    assert!(combat_pass(cutoff - 0.5, &defines));
    assert!(!combat_pass(cutoff, &defines));
    assert!(!combat_pass(cutoff + 0.5, &defines));
    defines.naval_combat_cutoff = 400.0;
    assert!(combat_pass(399.5, &defines));
    assert!(!combat_pass(400.0, &defines));
    assert!(!combat_pass(cutoff - 0.5, &defines));
}

fn with_context<T>(test: impl FnOnce(&MarkerContext<'_>) -> T) -> T {
    let vision = Vision::sharing(1, []);
    let owner = |raw: u32| (raw < 100).then_some(raw);
    let flags = CombatFlags::default();
    let frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    let native = NativeInputs::read(&frame).unwrap();
    let country_ids = BTreeMap::new();
    let naval = NavalContext {
        type_tints: &viewer_counter_kit::type_tint::Index::default(),
        vision: &vision,
        native: &native,
        ship_inputs: None,
        aggression: &[1],
        ship_task_forces: Arc::new(BTreeMap::new()),
        ships_complete: true,
        ships: Arc::new(BTreeMap::new()),
        country_ids: &country_ids,
        ideology: &|_| Ok(None),
        missions: Arc::new(BTreeMap::new()),
        mission_skips: Default::default(),
        forces: &[],
        forces_complete: true,
    };
    let countries = Countries::default();
    test(&MarkerContext {
        vision: &vision,
        owner: &owner,
        naval: Ok(&naval),
        countries: &countries,
        flags: &flags,
    })
}

fn viewer<'a>(context: &MarkerContext<'a>) -> Viewer<'a> {
    Viewer {
        vision: context.vision,
        owner: context.owner,
    }
}

fn detail(attackers: &[u32], defenders: &[u32]) -> NavalCombatDetailsRow {
    let mut row = NavalCombatDetailsRow {
        combat: 7,
        ..Default::default()
    };
    row.sides[0].summary.countries = attackers.to_vec();
    row.sides[1].summary.countries = defenders.to_vec();
    row
}

#[test]
fn the_hq_frame_counts_only_forward_group_members_of_the_viewers_side() {
    let force = replay_data::rows::TaskForceRow {
        id: 30,
        roster_owner: 1,
        ..Default::default()
    };
    let bare = replay_data::rows::TaskForceRow {
        id: 31,
        ..force
    };
    let forces = BTreeMap::from([
        (30, Ok((&force, 1))),
        (31, Ok((&bare, 0))),
        (
            32,
            Err("the TaskForceCounterInputs table has no row for task force 32".to_owned()),
        ),
    ]);
    let ship_forces = BTreeMap::from([(20, 30), (22, 31), (23, 32)]);
    let member = |combat, group_index, member_source| NavalCombatShipRow {
        combat,
        side: 1,
        group_index,
        member_source,
        ship: 20,
        live_ship: 1,
        ..Default::default()
    };
    with_context(|context| {
        let row = detail(&[5], &[1]);
        let frame = |members: &[NavalCombatShipRow]| {
            let forward = grid::forward_members(members);
            hq_frame(
                viewer(context),
                &row,
                forward.get(&7).into_iter().flatten().copied(),
                &ship_forces,
                &forces,
            )
        };
        assert_eq!(frame(&[member(7, 0, 0)]), Ok(true));
        assert_eq!(frame(&[member(7, u32::MAX, 0)]), Ok(false));
        assert_eq!(frame(&[member(7, 0, 1)]), Ok(false));
        assert_eq!(frame(&[member(8, 0, 0)]), Ok(false));
        let unknown = NavalCombatShipRow {
            ship: 21,
            ..member(7, 0, 0)
        };
        assert_eq!(frame(&[unknown.clone(), member(7, 0, 0)]), Ok(true));
        assert_eq!(frame(&[member(7, 0, 0), unknown.clone()]), Ok(true));
        let without = NavalCombatShipRow {
            ship: 22,
            ..member(7, 0, 0)
        };
        assert_eq!(frame(std::slice::from_ref(&without)), Ok(false));
        assert_eq!(
            frame(&[without, unknown]),
            Err("the Ships table has no row for ship 21".to_owned())
        );
        let uninputs = NavalCombatShipRow {
            ship: 23,
            ..member(7, 0, 0)
        };
        assert_eq!(
            frame(&[uninputs]),
            Err("the TaskForceCounterInputs table has no row for task force 32".to_owned())
        );
        let unresolved = detail(&[500], &[1]);
        assert_eq!(
            hq_frame(
                viewer(context),
                &unresolved,
                &[member(7, 0, 0)],
                &ship_forces,
                &forces
            ),
            Err("side 0 country 500 has no CountryLinks row".to_owned())
        );
        let neither = detail(&[5], &[6]);
        assert_eq!(
            hq_frame(
                viewer(context),
                &neither,
                &[member(7, 0, 0)],
                &ship_forces,
                &forces
            ),
            Ok(false)
        );
    });
}

#[test]
fn a_direct_icon_bound_elsewhere_rotates_and_a_spillover_does_not() {
    let combats = [
        Combat {
            id: 8,
            home: 12,
            registry_ordinal: 0,
        },
        Combat {
            id: 9,
            home: 11,
            registry_ordinal: 1,
        },
    ];
    let direct = [Direct {
        province: 10,
        combats: vec![8],
    }];
    let transfers = [
        Transfers {
            combat: 8,
            transfers: vec![100],
        },
        Transfers {
            combat: 9,
            transfers: vec![101],
        },
    ];
    let units = [
        Units {
            transfer: 100,
            units: vec![200],
        },
        Units {
            transfer: 101,
            units: vec![201],
        },
    ];
    let locations = BTreeMap::from([(200, 10), (201, 10)]);
    let kind = |province: u32| {
        Some(if province == 12 {
            ProvinceKind::Land
        } else {
            ProvinceKind::Sea
        })
    };
    let placed = occurrences::derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&locations),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(true),
    );
    let icons = &placed.by_province[&10];
    assert_eq!(icons.len(), 2);
    let (bound, spilled) = (&icons[0], &icons[1]);
    assert_eq!((bound.combat, bound.anchor_province), (8, 12));
    assert_eq!(bound.ordinal, Some(1));
    assert!(rotates(12, bound));
    assert!(!rotates(11, spilled));
    let census = Ok([
        Census {
            convoys: 1,
            ..Default::default()
        },
        Census {
            other: 1,
            ..Default::default()
        },
    ]);
    assert_eq!(
        decide(rotates(12, bound), Engaged::Yes, census, None),
        Ok(Decision {
            variant: 1,
            progress: true,
            frame: FrameSource::Engaged,
        })
    );
    assert_eq!(
        decide(rotates(11, spilled), Engaged::Yes, census, None),
        Ok(Decision {
            variant: 0,
            progress: false,
            frame: FrameSource::Spillover,
        })
    );
    let unknown = Err("the NavalCombatShips table is unavailable (x)");
    assert_eq!(
        decide(false, Engaged::Unknown, unknown, None).map(|d| d.variant),
        Ok(0)
    );
    let ambiguous = Engaged::Ambiguous(Ambiguity::Scope);
    assert_eq!(
        decide(true, ambiguous, census, None),
        Err(Ambiguity::Scope.reason().to_owned())
    );
    assert_eq!(
        decide(true, Engaged::Yes, unknown, None),
        Err("convoy variant undecided: the NavalCombatShips table is unavailable (x)".to_owned())
    );
    for (kept, variant) in [(Some(Kept::Frame(1)), 1), (Some(Kept::Fresh), 0), (None, 0)] {
        assert_eq!(
            decide(true, Engaged::No, unknown, kept),
            Ok(Decision {
                variant,
                progress: false,
                frame: FrameSource::Kept(kept.unwrap_or(Kept::Undecided)),
            })
        );
    }
}

#[test]
fn the_progress_size_counts_engaged_land_and_naval_entries_alike() {
    let row = NavalProvinceCombatRow {
        province: 10,
        combats: vec![1, 2, 3],
        kinds: vec![
            NavalProvinceCombatRow::NAVAL,
            NavalProvinceCombatRow::LAND,
            NavalProvinceCombatRow::BORDER_WAR,
        ],
    };
    let flags = |naval: Engaged, land: Engaged, border: Engaged| CombatFlags {
        naval: BTreeMap::from([(1, naval)]),
        land: BTreeMap::from([(2, land), (3, border)]),
        ..CombatFlags::default()
    };
    use Engaged::*;
    let size = |flags: CombatFlags| two_engaged(&flags, 10, Some(&row), true).ok();
    assert_eq!(size(flags(Yes, Yes, No)), Some(true));
    assert_eq!(size(flags(Yes, No, No)), Some(false));
    assert_eq!(size(flags(No, Yes, Yes)), Some(true));
    assert_eq!(size(flags(Yes, Yes, Unknown)), Some(true));
    let ambiguous = Ambiguous(Ambiguity::Scope);
    assert_eq!(size(flags(Yes, ambiguous, No)), None);
    assert_eq!(size(flags(No, Unknown, No)), Some(false));
    assert_eq!(
        two_engaged(&CombatFlags::default(), 10, Some(&row), true),
        Err("an engaged flag in province 10's combat vector is undecided".to_owned())
    );
    let all = flags(Yes, Yes, Yes);
    assert_eq!(two_engaged(&all, 10, None, true), Ok(false));
    assert!(
        two_engaged(&all, 10, None, false)
            .is_err_and(|cause| cause.starts_with("province 10's combat vector is unknown"))
    );
}

fn scale_frame(ships: u64, divisions: u64, combats: u64) -> DecodedFrame {
    use replay_data::{
        rows::DivisionRow, rows::NavalCombatRow, rows::NavalCombatTransferRow,
        rows::NavalResultCountryRow, rows::NavalResultMemberRow, rows::NavalTransferUnitRow,
        rows::ShipRow, rows::TableRows, rows::TaskForceCounterInputRow, rows::TaskForceRow,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 1,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not needed".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    let mut set = |rows: TableRows| {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    let id = |combat: u64| combat + 1;
    let home = |combat: u64| 1_000 + combat as u32;
    set(TableRows::Ships(
        (0..ships)
            .map(|ship| ShipRow {
                id: ship,
                task_force: ship / 2,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::TaskForces(
        (0..ships.div_ceil(2))
            .map(|force| TaskForceRow {
                id: force,
                roster_owner: 1 + (force % 2) as u32,
                province: 3_000,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::TaskForceCounterInputs(
        (0..ships.div_ceil(2))
            .map(|force| TaskForceCounterInputRow {
                task_force: force,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::Divisions(
        (0..divisions)
            .map(|division| DivisionRow {
                id: 1 << 40 | division,
                province: 5_000 + (division % 1_000) as u32,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::RailwayGunCounterInputs(Vec::new()));
    set(TableRows::NavalCombats(
        (0..combats)
            .map(|combat| NavalCombatRow {
                id: id(combat),
                province: home(combat),
                registry_ordinal: combat as u32,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::NavalCombatDetails(
        (0..combats)
            .map(|combat| {
                let mut row = detail(&[1], &[2]);
                row.combat = id(combat);
                row
            })
            .collect(),
    ));
    set(TableRows::NavalProvinceCombats(
        (0..combats)
            .map(|combat| NavalProvinceCombatRow {
                province: home(combat),
                combats: vec![id(combat)],
                kinds: vec![NavalProvinceCombatRow::NAVAL],
            })
            .collect(),
    ));
    set(TableRows::NavalCombatShips(
        (0..combats)
            .flat_map(|combat| {
                (0..8).map(move |slot| NavalCombatShipRow {
                    combat: id(combat),
                    side: (slot / 4) as u8,
                    ship: 8 * combat + slot,
                    member_index: (slot % 4) as u32,
                    live_ship: 1,
                    ..Default::default()
                })
            })
            .collect(),
    ));
    set(TableRows::NavalCombatTransfers(
        (0..combats)
            .map(|combat| NavalCombatTransferRow {
                combat: id(combat),
                transfers: vec![1 << 48 | combat],
            })
            .collect(),
    ));
    set(TableRows::NavalTransferUnits(
        (0..combats)
            .map(|combat| NavalTransferUnitRow {
                transfer: 1 << 48 | combat,
                units: vec![1 << 40 | (2 * combat), 1 << 40 | (2 * combat + 1)],
            })
            .collect(),
    ));
    set(TableRows::NavalResults(
        (0..combats)
            .map(|combat| NavalResultRow {
                id: id(combat),
                province: home(combat),
                winning_side: 1,
                losing_side: 2,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::NavalResultMembers(
        (0..combats)
            .map(|combat| NavalResultMemberRow {
                result: id(combat),
                presence_value: 1,
                ..Default::default()
            })
            .collect(),
    ));
    set(TableRows::NavalResultCountries(
        (0..combats)
            .flat_map(|combat| {
                [(0, 1), (1, 2), (2, 1)].map(|(list, country)| NavalResultCountryRow {
                    result: id(combat),
                    country,
                    list,
                    ordinal: 0,
                })
            })
            .collect(),
    ));
    frame
}

fn scale_flags(frame: &DecodedFrame) -> CombatFlags {
    CombatFlags {
        naval: rows::<tables::NavalCombats>(frame)
            .unwrap()
            .rows()
            .iter()
            .map(|row| (row.id, Engaged::Yes))
            .collect(),
        ..indexed()
    }
}

fn indexed() -> CombatFlags {
    CombatFlags {
        land_provenance: Provenance::Indexed,
        naval_provenance: Provenance::Indexed,
        ..CombatFlags::default()
    }
}

fn scale_kind(_: u32) -> Option<ProvinceKind> {
    Some(ProvinceKind::Sea)
}

fn scale_anchor(province: u32) -> Option<[f32; 3]> {
    Some([province as f32, 0.0, 0.0])
}

#[test]
fn every_omitted_naval_icon_reaches_the_diagnostics_by_cause_and_combat() {
    use replay_data::rows::TableId;
    use replay_frame::frame::TableView;
    let vision = Vision::sharing(1, []);
    let owner = |raw: u32| (raw < 100).then_some(raw);
    let viewer = Viewer {
        vision: &vision,
        owner: &owner,
    };
    let lines = |frame: &DecodedFrame, flags: &CombatFlags, refused: Option<&str>| {
        let anchor = |province: u32| (province != 1_000).then_some([0.0; 3]);
        let hour =
            NavalMarkerTables::read(frame, viewer, flags, &BTreeMap::new(), &scale_kind, &anchor)
                .unwrap();
        let mut report = CounterReport::default();
        hour.combats
            .as_ref()
            .unwrap()
            .report_omitted(refused, &mut report);
        (
            report.combat_gap_lines().collect::<Vec<_>>(),
            report.unavailable_naval_combats,
        )
    };
    let mut frame = scale_frame(16, 4, 2);
    let flags = scale_flags(&frame);
    assert_eq!(
        lines(&frame, &flags, None),
        (
            vec![
                "Naval combat icons: 1 undecided: icon omitted: the content places no \
                 Standstill unit-stack anchor for province 1000; combats 1"
                    .to_owned()
            ],
            1
        )
    );
    let anchor = |province: u32| (province != 1_000).then_some([0.0; 3]);
    let hour = NavalMarkerTables::read(
        &frame,
        viewer,
        &flags,
        &BTreeMap::new(),
        &scale_kind,
        &anchor,
    )
    .unwrap();
    let omitted: Vec<&Omitted> = hour
        .combats
        .as_ref()
        .unwrap()
        .icons
        .iter()
        .filter_map(|icon| icon.as_ref().err())
        .collect();
    assert_eq!(
        omitted,
        [&Omitted {
            combat: 1,
            province: 1_000,
            rotates: true,
            place: None,
            cause: "icon omitted: the content places no Standstill unit-stack anchor for \
                    province 1000"
                .to_owned(),
        }]
    );
    assert_eq!(
        lines(&frame, &flags, Some(&unconstructed("x"))),
        (
            vec![
                "Naval combat icons: 2 undecided: icon omitted: naval_combat_mapicon does not \
                 lay out: x; combats 1, 2"
                    .to_owned()
            ],
            6
        )
    );
    let unknown = CombatFlags {
        naval: BTreeMap::from([(1, Engaged::Yes)]),
        ..indexed()
    };
    let (stopped, _) = lines(&frame, &unknown, None);
    assert!(
        stopped.contains(
            &"Naval combat icons: 2 undecided: spillover icons omitted: naval-combat country \
          engagement is unknown; combats 1, 2"
                .to_owned()
        )
    );
    assert!(
        stopped.contains(
            &"Naval combat icons: 1 undecided: icon omitted: its place in province 1001's icon \
          stack is unknown; combats 2"
                .to_owned()
        )
    );
    let transfers = rows::<tables::NavalCombatTransfers>(&frame).unwrap().rows()[..1].to_vec();
    let mut direct_only = frame.clone();
    *direct_only.tables[TableId::NavalCombatTransfers.index()] = TableView::Complete(
        replay_data::rows::TableRows::NavalCombatTransfers(transfers),
    );
    assert!(
        lines(&direct_only, &unknown, None).0.contains(
            &"Naval combat icons: 1 undecided: icon omitted: the combat's engaged flag for the \
          viewer is unknown; combats 2"
                .to_owned()
        )
    );
    let members = rows::<tables::NavalCombatShips>(&frame)
        .unwrap()
        .rows()
        .to_vec();
    *frame.tables[TableId::NavalCombatShips.index()] = TableView::Truncated {
        rows: replay_data::rows::TableRows::NavalCombatShips(members),
        missing: 3,
    };
    assert!(
        lines(&frame, &flags, None).0.contains(
            &"Naval combat icons: 1 undecided: icon omitted: convoy variant undecided: the \
          NavalCombatShips table is truncated (3 rows unread); combats 2"
                .to_owned()
        )
    );
    let details = rows::<tables::NavalCombatDetails>(&frame).unwrap().rows()[..1].to_vec();
    *frame.tables[TableId::NavalCombatDetails.index()] =
        TableView::Complete(replay_data::rows::TableRows::NavalCombatDetails(details));
    assert!(
        lines(&frame, &flags, None).0.contains(
            &"Naval combat icons: 1 undecided: icon omitted: its side countries are unknown: the \
          NavalCombatDetails table has no row for combat 2, which province 1001 names; \
          combats 2"
                .to_owned()
        )
    );
    let anchor = |province: u32| (province != 1_000).then_some([0.0; 3]);
    let hour = NavalMarkerTables::read(
        &frame,
        viewer,
        &flags,
        &BTreeMap::new(),
        &scale_kind,
        &anchor,
    )
    .unwrap();
    let undecided: Vec<(u64, u32, bool, bool)> = hour
        .combats
        .as_ref()
        .unwrap()
        .undecided
        .iter()
        .map(|omitted| {
            (
                omitted.combat,
                omitted.province,
                omitted.rotates,
                omitted.place.is_none(),
            )
        })
        .collect();
    assert_eq!(undecided, [(2, 1001, true, true)]);
}

#[test]
fn the_result_presentation_follows_the_results_it_holds() {
    let fact = |subtype, winning_side, losing_side| ResultFacts {
        lists: [Some(true), Some(false), Some(true)],
        subtype,
        winning_side,
        losing_side,
        present: true,
        ..Default::default()
    };
    let (won, lost) = (fact(true, 1, 2), fact(false, 2, 1));
    assert_eq!(
        result_presentation(&[won, lost], 2),
        Ok((NAVAL_RESULT_SPRITES[2], 2))
    );
    assert_eq!(
        result_presentation(&[won], 1),
        Ok((NAVAL_RESULT_SPRITES[3], 1))
    );
    assert_eq!(
        result_presentation(&[won], 2),
        Ok((NAVAL_RESULT_SPRITES[2], 1))
    );
    assert_eq!(
        result_presentation(&[], 1),
        Ok((NAVAL_RESULT_SPRITES[0], 2))
    );
    let tables = |undecided| ResultTables {
        provinces: BTreeMap::from([(10, vec![0, 1])]),
        facts: BTreeMap::from([(0, won), (1, lost)]),
        undecided,
    };
    assert_eq!(tables(None).visible(10), Ok(vec![0, 1]));
    assert!(tables(Some("incomplete")).visible(10).is_err());
}

#[test]
fn a_refused_result_window_counts_every_shown_icon() {
    let seen = convoy_result_for_test();
    let unseen = ResultFacts {
        lists: [Some(false), Some(false), Some(false)],
        ..seen
    };
    let tables = ResultTables {
        provinces: BTreeMap::from([(1, vec![10]), (2, vec![20]), (3, vec![30])]),
        facts: BTreeMap::from([(10, seen), (20, unseen), (30, seen)]),
        undecided: None,
    };
    let mut report = CounterReport::default();
    refused_results(&tables, &mut report);
    assert_eq!(report.unavailable_naval_results, 2);
    assert_eq!(report.naval_result_gaps[REFUSED_RESULT_WINDOW], 2);
}
