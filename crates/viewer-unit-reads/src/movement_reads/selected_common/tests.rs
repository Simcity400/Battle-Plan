use super::super::test_support::{assets, frame, put};
use super::*;
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::DirectedWars,
    rows::ProvinceUnitOrderRow, rows::Recorded, rows::ReverseZeroVisibilityInputs,
    rows::ShipModelRow, rows::ShipRow, rows::TableId, rows::TaskForceRow, rows::WarCell,
};
use replay_frame::frame::TableView;

const FORCE: u64 = 0x100;
const SHIP: u64 = 0x200;

fn subject_frame() -> DecodedFrame {
    use replay_data::{rows::RelationLinkRow, rows::TransportSubjectAccess};
    let mut frame = strait_frame();
    put::<tables::FactionMembers>(&mut frame, Vec::new());
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    if let Recorded::Value(Some(packet)) = &mut rows[0].directed_wars {
        packet.complete_active_domain = true;
    }
    put::<tables::CountryCounterInputs>(&mut frame, rows);
    let hour = frame.hour;
    put::<tables::RelationLinks>(
        &mut frame,
        vec![RelationLinkRow {
            country: 30,
            target_index: 2,
            transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                sample_hour: replay_data::rows::PacketHour::Stamped(hour),
                complete_owner: true,
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    frame
}

fn subject_query(frame: &DecodedFrame, current_only: bool) -> Fact<bool> {
    let assets = assets();
    let world = World::read(frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common)?;
    if current_only {
        CommonMoveWorld::current_subject_access(&moves, 2, 3)
    } else {
        moves.common_discloses_access(30, 40, true)
    }
}

#[test]
fn selected_current_subject_pointer_overrides_historical_subject_dates() {
    use replay_data::{
        rows::RelationLinkRow, rows::SubjectRow, rows::TransportSubjectAccess,
        rows::ZeroSubjectInputs,
    };
    let mut frame = subject_frame();
    put::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 30,
            subject: 40,
            start_hour: 0,
            end_hour: i32::MAX,
            ..Default::default()
        }],
    );
    for direct in [false, true] {
        assert_eq!(subject_query(&frame, direct), Ok(false));
    }
    let certificate = frame.complete::<tables::RelationLinks>().unwrap()[0].clone();
    let hour = frame.hour;
    put::<tables::RelationLinks>(
        &mut frame,
        vec![
            certificate,
            RelationLinkRow {
                country: 30,
                target_index: 3,
                transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                    sample_hour: replay_data::rows::PacketHour::Stamped(hour),
                    target_raw: Some(40),
                    subject: Some(ZeroSubjectInputs {
                        relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE),
                        parties: Some([Some(30), Some(40)]),
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            },
        ],
    );
    put::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 30,
            subject: 40,
            start_hour: 50,
            end_hour: 51,
            ..Default::default()
        }],
    );
    for direct in [false, true] {
        assert_eq!(subject_query(&frame, direct), Ok(true));
    }
}

#[test]
fn selected_subject_current_hour_and_complete_table_are_required() {
    let mut frame = subject_frame();
    frame.hour += 1;
    assert!(subject_query(&frame, true).unwrap_err().contains("stale"));
    let mut frame = subject_frame();
    let rows = frame.complete::<tables::RelationLinks>().unwrap().to_vec();
    *frame.tables[TableId::RelationLinks.index()] = TableView::Truncated {
        rows: replay_data::rows::TableRows::RelationLinks(rows),
        missing: 1,
    };
    assert!(
        subject_query(&frame, true)
            .unwrap_err()
            .contains("truncated")
    );
}

fn railway_frame() -> DecodedFrame {
    use replay_data::{rows::RailCooldownRow, rows::RailwaySelectionInputs};
    let mut frame = strait_frame();
    put::<tables::RailCooldowns>(
        &mut frame,
        vec![RailCooldownRow {
            province: 1,
            cooldown_ticks: 0,
            selection_hour: Some(12).into(),
            selection: Some(RailwaySelectionInputs {
                levels: Some(vec![0, 5, 99]),
                pair_count: Some(-1),
                building_level: Some(Some(1)),
            })
            .into(),
            ..Default::default()
        }],
    );
    *frame.tables[TableId::RailEdges.index()] = TableView::Unavailable {
        reason: "sorted pairs cannot supply selected native operands".into(),
    };
    frame
}

fn railway_answers(frame: &DecodedFrame, target: u32) -> (Fact<bool>, Fact<bool>) {
    let mut assets = assets();
    let graph = content_reader_map::province_graph::ProvinceGraph {
        edges: BTreeMap::from([(1, vec![(3, None), (2, None), (2, None)]), (4, Vec::new())]),
        ..Default::default()
    };
    assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
        &graph,
        &assets.geometry,
    ));
    let world = World::read(frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
    (
        moves.railway_accessible(1),
        moves.railway_retreat_edge(1, target),
    )
}

fn change_railway(
    frame: &mut DecodedFrame,
    edit: impl FnOnce(&mut replay_data::rows::RailCooldownRow),
) {
    let mut rows = frame.complete::<tables::RailCooldowns>().unwrap().to_vec();
    edit(&mut rows[0]);
    put::<tables::RailCooldowns>(frame, rows);
}

#[test]
fn selected_railway_gates_use_raw_count_and_first_index_without_sorted_pairs() {
    let mut frame = railway_frame();
    assert_eq!(railway_answers(&frame, 2), (Ok(true), Ok(true)));
    assert_eq!(railway_answers(&frame, 3), (Ok(true), Ok(false)));
    change_railway(&mut frame, |row| {
        row.selection
            .require_mut("test")
            .unwrap()
            .as_mut()
            .unwrap()
            .levels = Some(vec![0, 0, 99])
    });
    assert_eq!(railway_answers(&frame, 2), (Ok(true), Ok(false)));
    change_railway(&mut frame, |row| {
        let packet = row.selection.require_mut("test").unwrap().as_mut().unwrap();
        packet.pair_count = Some(0);
        packet.levels = Some(vec![0, 5, 99]);
    });
    assert_eq!(railway_answers(&frame, 2), (Ok(false), Ok(true)));
}

#[test]
fn selected_railway_native_early_returns_do_not_demand_later_operands() {
    let mut frame = railway_frame();
    for cooldown in [-1, 1] {
        change_railway(&mut frame, |row| {
            row.cooldown_ticks = cooldown;
            row.selection
                .require_mut("test")
                .unwrap()
                .as_mut()
                .unwrap()
                .building_level = None;
        });
        assert_eq!(railway_answers(&frame, 2), (Ok(false), Ok(false)));
    }
    change_railway(&mut frame, |row| {
        row.cooldown_ticks = 0;
        let packet = row.selection.require_mut("test").unwrap().as_mut().unwrap();
        packet.pair_count = Some(0);
        packet.levels = Some(vec![0, -1, 99]);
    });
    assert_eq!(railway_answers(&frame, 2), (Ok(false), Ok(false)));
    change_railway(&mut frame, |row| row.selection = Recorded::NotRecorded);
    assert_eq!(railway_answers(&frame, 99).1, Ok(false));
    put::<tables::RailCooldowns>(&mut frame, Vec::new());
    assert_eq!(railway_answers(&frame, 2), (Ok(false), Ok(false)));
}

#[test]
fn selected_railway_null_signed_levels_missing_and_stale_packets_stay_distinct() {
    for level in [None, Some(0), Some(-1), Some(1)] {
        let mut frame = railway_frame();
        change_railway(&mut frame, |row| {
            row.selection
                .require_mut("test")
                .unwrap()
                .as_mut()
                .unwrap()
                .building_level = Some(level)
        });
        let expected = Ok(level.is_some_and(|level| level > 0));
        assert_eq!(railway_answers(&frame, 2), (expected.clone(), expected));
    }
    for edit in 0..9 {
        let mut frame = railway_frame();
        change_railway(&mut frame, |row| match edit {
            0 => row.selection = Recorded::NotRecorded,
            1 => row.selection = None.into(),
            2 => row.selection_hour = Some(11).into(),
            6 => row.selection_hour = None.into(),
            7 => row.selection_hour = Recorded::NotRecorded,
            _ => {
                let packet = row.selection.require_mut("test").unwrap().as_mut().unwrap();
                match edit {
                    3 => packet.levels = None,
                    4 => packet.pair_count = None,
                    8 => packet.levels = Some(vec![0]),
                    _ => packet.building_level = None,
                }
            }
        });
        let (access, retreat) = railway_answers(&frame, 2);
        assert_eq!(access.is_err(), edit != 3 && edit != 8);
        assert_eq!(retreat.is_err(), edit != 4);
    }
}

fn strait_frame() -> DecodedFrame {
    let mut frame = frame();
    frame.hour = 12;
    put::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 20,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 30,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 40,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 99,
                canonical_slot: 1,
            },
        ],
    );
    put::<tables::Countries>(
        &mut frame,
        [30, 40, 99]
            .map(|id| CountryRow {
                id,
                canonical_slot: match id {
                    30 => 2,
                    40 => 3,
                    _ => 1,
                },
                ..Default::default()
            })
            .into(),
    );
    put::<tables::CountryCounterInputs>(
        &mut frame,
        [30, 40]
            .map(|country| CountryCounterInputRow {
                country,
                directed_wars: Recorded::Value(Some(DirectedWars {
                    sample_hour: replay_data::rows::PacketHour::Stamped(12),
                    receiver_raw_id: country as i32,
                    receiver_slot: if country == 30 { 2 } else { 3 },
                    matrix_count: 4,
                    cells: vec![WarCell {
                        target_slot: 1,
                        active: true,
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            })
            .into(),
    );
    put::<tables::ProvinceUnitOrder>(
        &mut frame,
        vec![ProvinceUnitOrderRow {
            province: 2,
            mixed: vec![FORCE],
            naval: vec![FORCE],
        }],
    );
    put::<tables::TaskForces>(
        &mut frame,
        vec![TaskForceRow {
            id: FORCE,
            province: 2,
            logical_owner: 30,
            roster_owner: 40,
            ..Default::default()
        }],
    );
    put::<tables::Ships>(
        &mut frame,
        vec![ShipRow {
            id: SHIP,
            task_force: FORCE,
            ..Default::default()
        }],
    );
    put::<tables::ShipModels>(
        &mut frame,
        vec![ShipModelRow {
            ship: SHIP,
            formation_flags: 0,
            ..Default::default()
        }],
    );
    frame
}

fn query(frame: &DecodedFrame, raw_target: u32) -> Fact<StraitFleet> {
    let assets = assets();
    let world = World::read(frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    Moves::for_common(&world, assets.movement_reads(), &cells, &common)?
        .selected_strait(2, raw_target)
}

#[test]
fn selected_strait_uses_direct_war_with_empty_symmetric_war_table() {
    let frame = strait_frame();
    assert!(frame.complete::<tables::WarRelations>().unwrap().is_empty());
    assert_eq!(query(&frame, 20), Ok(StraitFleet::Unseen));
}

#[test]
fn selected_strait_stops_model_reads_after_the_first_clear_formation_flag() {
    let mut frame = strait_frame();
    let mut ships = frame.complete::<tables::Ships>().unwrap().to_vec();
    ships.push(ShipRow {
        id: SHIP + 1,
        task_force: FORCE,
        ..Default::default()
    });
    put::<tables::Ships>(&mut frame, ships);
    assert_eq!(query(&frame, 20), Ok(StraitFleet::Unseen));
    put::<tables::UnitIntel>(
        &mut frame,
        vec![replay_data::rows::UnitIntelRow {
            unit: FORCE,
            viewer: 1,
            level: 1,
            ..Default::default()
        }],
    );
    assert_eq!(query(&frame, 20), Ok(StraitFleet::Visible));
    let mut models = frame.complete::<tables::ShipModels>().unwrap().to_vec();
    models[0].formation_flags = 0x80;
    put::<tables::ShipModels>(&mut frame, models);
    assert!(
        query(&frame, 20)
            .unwrap_err()
            .contains("no current model row")
    );
}

#[test]
fn selected_strait_missing_order_row_is_unknown_but_known_empty_is_none() {
    let mut frame = strait_frame();
    put::<tables::ProvinceUnitOrder>(&mut frame, Vec::new());
    assert!(
        query(&frame, 20)
            .unwrap_err()
            .contains("occupant-order row")
    );
    put::<tables::ProvinceUnitOrder>(
        &mut frame,
        vec![ProvinceUnitOrderRow {
            province: 2,
            ..Default::default()
        }],
    );
    assert_eq!(query(&frame, 20), Ok(StraitFleet::None));
}

#[test]
fn selected_strait_omitted_direct_target_never_becomes_peace() {
    let mut frame = strait_frame();
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    if let Recorded::Value(Some(packet)) = &mut rows[0].directed_wars {
        packet.cells.clear();
    }
    put::<tables::CountryCounterInputs>(&mut frame, rows);
    assert!(query(&frame, 20).is_err());
}

#[test]
fn selected_strait_missing_class_model_and_backlink_are_refusals() {
    let mut frame = strait_frame();
    put::<tables::TaskForces>(&mut frame, Vec::new());
    assert!(query(&frame, 20).unwrap_err().contains("class census"));
    let mut frame = strait_frame();
    put::<tables::TaskForces>(
        &mut frame,
        vec![TaskForceRow {
            id: FORCE,
            province: 3,
            ..Default::default()
        }],
    );
    assert!(query(&frame, 20).unwrap_err().contains("backlink"));
    let mut frame = strait_frame();
    put::<tables::ShipModels>(&mut frame, Vec::new());
    assert!(query(&frame, 20).unwrap_err().contains("model row"));
}

#[test]
fn selected_strait_retains_raw_zero_alias_in_logical_owner_war_gate() {
    let mut frame = strait_frame();
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.insert(
        0,
        CountryLinkRow {
            id: 12,
            canonical_slot: 0,
        },
    );
    put::<tables::CountryLinks>(&mut frame, links);
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    for row in &mut rows {
        if let Recorded::Value(Some(packet)) = &mut row.directed_wars {
            packet.cells = vec![WarCell {
                target_slot: 0,
                active: true,
            }];
        }
        row.forward_visibility = Some(ReverseZeroVisibilityInputs {
            war: Some([1, 0]),
            ..Default::default()
        })
        .into();
    }
    put::<tables::CountryCounterInputs>(&mut frame, rows.clone());
    assert_eq!(query(&frame, 12), Ok(StraitFleet::Unseen));
    for (input, zero_war, expected) in [
        (
            Recorded::NotRecorded,
            Recorded::Value(None),
            Err("CountryCounterInputs.zero_war capture refused".into()),
        ),
        (
            Recorded::Value(None),
            Recorded::Value(None),
            Err("CountryCounterInputs.forward_visibility capture refused".into()),
        ),
        (
            Recorded::NotRecorded,
            Recorded::Value(Some([1, 0])),
            Ok(StraitFleet::Unseen),
        ),
        (
            Recorded::Value(None),
            Recorded::Value(Some([1, 0])),
            Err("CountryCounterInputs.forward_visibility capture refused".into()),
        ),
    ] {
        rows[1].forward_visibility = input;
        rows[1].zero_war = zero_war;
        put::<tables::CountryCounterInputs>(&mut frame, rows.clone());
        assert_eq!(query(&frame, 12), expected);
    }
}

#[test]
fn selected_strait_route_gate_does_not_demand_unavailable_ship_models() {
    let mut frame = strait_frame();
    let mut rows = frame.complete::<tables::TaskForces>().unwrap().to_vec();
    rows[0].route_count = 1;
    put::<tables::TaskForces>(&mut frame, rows);
    *frame.tables[TableId::ShipModels.index()] = TableView::Unavailable {
        reason: "models must remain unreached".into(),
    };
    assert_eq!(query(&frame, 20), Ok(StraitFleet::None));
}

#[test]
fn selected_common_route_keeps_the_callers_class_when_handles_coincide() {
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::RailwayGunCounterInputRow,
    };
    let mut frame = strait_frame();
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: FORCE,
            ..Default::default()
        }],
    );
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: FORCE,
            path_total: 1,
            path: vec![2],
            ..Default::default()
        }],
    );
    put::<tables::RailwayGunCounterInputs>(
        &mut frame,
        vec![RailwayGunCounterInputRow {
            gun: FORCE,
            ordinary_path: vec![3],
            ..Default::default()
        }],
    );
    let assets = assets();
    let world = World::read(&frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(&frame);
    let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
    let army = Army::default();
    let selected = |class| CommonUnit {
        class,
        handle: FORCE,
        logical_owner_raw: 20,
        combat_count: 0,
        army: &army,
    };
    assert_eq!(
        moves.unit_route_first(&selected(CommonClass::Army)),
        Ok(Some(2))
    );
    assert_eq!(
        moves.unit_route_first(&selected(CommonClass::RailwayGun)),
        Ok(Some(3))
    );
}

#[test]
fn selected_transport_follows_areas_and_naval_bases_and_refuses_past_them() {
    use viewer_runtime::unit_handle::Unit;
    use replay_data::{
        rows::ControllerAreaNode, rows::DivisionRow, rows::NavalBaseRow, rows::ProvinceRow,
        rows::RailwayGunCounterInputRow,
    };
    let check = |frame: &DecodedFrame, selection: &[Unit], target| {
        let mut assets = assets();
        assets.geometry.land.extend([1, 2, 3]);
        assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
            &content_reader_map::province_graph::ProvinceGraph::default(),
            &assets.geometry,
        ));
        let world = World::read(frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let common = CommonCells::new(frame);
        let loc = [("TRANSPORT_NO_FROM_NAVAL_BASE".into(), "No port here".into())]
            .into_iter()
            .collect();
        let keys = BTreeMap::new();
        let text = CommonText {
            frame,
            loc: &loc,
            recorded_language: "english",
            state_keys: &keys,
        };
        Moves::for_common(&world, assets.movement_reads(), &cells, &common)
            .unwrap()
            .selected_transport_message(selection, target, &text)
    };
    let none = || -> Result<content_reader_unit_path::unit_path::selected_transport::Outputs, String> {
        Ok(Default::default())
    };
    let bases = |frame: &mut DecodedFrame, provinces: &[u32]| {
        put::<tables::NavalBases>(
            frame,
            provinces
                .iter()
                .map(|&province| NavalBaseRow {
                    province,
                    ..Default::default()
                })
                .collect(),
        );
    };
    let mut frame = strait_frame();
    put::<tables::Provinces>(
        &mut frame,
        [(1, 1), (2, 1), (3, 3)]
            .map(|(id, area)| ProvinceRow {
                id,
                controller_area: Recorded::Value(Some(area)),
                owner_area: Recorded::Value(Some(area)),
                area_node: Recorded::Value((id == area).then(|| ControllerAreaNode {
                    first_land: Some(id),
                    neighbours: Some(Vec::new()),
                })),
                ..Default::default()
            })
            .into(),
    );
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: FORCE,
            province: 2,
            ..Default::default()
        }],
    );
    bases(&mut frame, &[2]);
    let army = [Unit::Division(FORCE)];
    for target in [1, 3] {
        assert_eq!(check(&frame, &army, target), none());
    }
    assert_eq!(check(&frame, &[Unit::TaskForce(FORCE)], 2), none());
    assert_eq!(check(&frame, &army, 2), none());
    bases(&mut frame, &[1, 2]);
    assert_eq!(check(&frame, &army, 1), Err(TRANSPORT_RETIRED.into()));
    bases(&mut frame, &[1, 3]);
    assert_eq!(check(&frame, &army, 1), none());
    assert_eq!(check(&frame, &army, 3), Err(TRANSPORT_RETIRED.into()));
    put::<tables::RailwayGunCounterInputs>(
        &mut frame,
        vec![RailwayGunCounterInputRow {
            gun: FORCE,
            province: 2,
            ..Default::default()
        }],
    );
    assert_eq!(
        check(&frame, &[Unit::RailwayGun(FORCE)], 3),
        Ok(
            content_reader_unit_path::unit_path::selected_transport::Outputs {
                primary: Vec::new(),
                secondary: b"No port here".to_vec(),
            }
        )
    );
}

#[test]
fn selected_rule_warning_localises_the_native_name_and_preserves_defined_empty() {
    let frame = strait_frame();
    let state_keys = BTreeMap::new();
    let rule = content_reader_map::adjacency_rules::parse(b"adjacency_rule = { name = fixture_rule }")
        .unwrap()
        .remove(0);
    for (value, expected) in [
        ("Authored name", "Blocked: Authored name"),
        ("", "Blocked: "),
    ] {
        let loc = [
            ("fixture_rule".into(), value.into()),
            (
                "UNIT_ADJECENCY_RULE_BLOCKED".into(),
                "Blocked: $RULE_NAME$".into(),
            ),
        ]
        .into_iter()
        .collect();
        let text = CommonText {
            frame: &frame,
            loc: &loc,
            recorded_language: "",
            state_keys: &state_keys,
        };
        assert_eq!(
            text.rule("UNIT_ADJECENCY_RULE_BLOCKED", &rule),
            Ok(expected.into())
        );
    }
}

#[test]
fn selected_primary_combat_scan_keeps_order_and_border_duration_uses_its_own_gate() {
    use replay_data::{
        rows::DivisionMovementRow, rows::LandBorderWarInputs, rows::LandCombatDetailsRow,
        rows::LandCombatRow, rows::UnitCombatReference, rows::UnitCombatReferences,
    };
    let mut frame = strait_frame();
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: FORCE,
            combat_count: 2,
            combat_references: Recorded::Value(Some(UnitCombatReferences {
                sample_hour: replay_data::rows::PacketHour::Stamped(12),
                entries: vec![
                    UnitCombatReference { combat: 8, kind: 1 },
                    UnitCombatReference { combat: 9, kind: 2 },
                ],
            })),
            ..Default::default()
        }],
    );
    put::<tables::LandCombats>(
        &mut frame,
        vec![LandCombatRow {
            id: 8,
            province: 2,
            duration: 4,
            ..Default::default()
        }],
    );
    put::<tables::LandCombatDetails>(
        &mut frame,
        vec![LandCombatDetailsRow {
            combat: 8,
            border_war: Recorded::Value(Some(LandBorderWarInputs::default())),
            ..Default::default()
        }],
    );
    *frame.tables[TableId::NavalCombats.index()] = TableView::Unavailable {
        reason: "later location must be undemanded after first match".into(),
    };
    let mut assets = assets();
    assets.naval_combat_min_duration = Err("unreached naval define".into());
    let world = World::read(&frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(&frame);
    let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
    let army = Army::default();
    let selected = CommonUnit {
        class: CommonClass::Army,
        handle: FORCE,
        logical_owner_raw: 20,
        combat_count: 2,
        army: &army,
    };
    assert_eq!(moves.primary_path_combat(&selected, 2), Ok(true));
    assert!(moves.primary_path_combat(&selected, 3).is_err());
    assert_eq!(
        moves.combat_minimum_duration(CommonCombat { combat: 8, kind: 1 }),
        Ok(true)
    );
    assert!(
        moves
            .combat_minimum_duration(CommonCombat { combat: 8, kind: 0 })
            .is_err()
    );
}

#[path = "tests/selected_routes.rs"]
mod selected_routes;

#[test]
fn selected_naval_duration_uses_its_own_signed_content_threshold() {
    use replay_data::rows::NavalCombatDetailsRow;
    let mut frame = strait_frame();
    *frame.tables[TableId::LandCombats.index()] = TableView::Unavailable {
        reason: "naval duration check must not demand land combat data".into(),
    };
    for (minimum, duration, expected) in [
        (7, 6, false),
        (7, 7, true),
        (7, 8, true),
        (-5, -6, false),
        (-5, -5, true),
        (-5, -4, true),
        (i32::MAX, i32::MIN, false),
        (i32::MIN, i32::MAX, true),
    ] {
        put::<tables::NavalCombatDetails>(
            &mut frame,
            vec![NavalCombatDetailsRow {
                combat: 9,
                day: i32::MAX,
                duration,
                custom_sea: Some(Vec::new()),
                ..Default::default()
            }],
        );
        let mut assets = assets();
        assets.naval_combat_min_duration = Ok(minimum);
        let world = World::read(&frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let common = CommonCells::new(&frame);
        let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
        assert_eq!(
            moves.combat_minimum_duration(CommonCombat { combat: 9, kind: 2 }),
            Ok(expected),
            "minimum {minimum}, duration {duration}"
        );
    }
}

#[test]
fn selected_naval_duration_refuses_missing_rows_tables_defines_and_unknown_classes() {
    use replay_data::{rows::NavalCombatDetailsRow, rows::TableRows};
    let mut frame = strait_frame();
    let query = |frame: &DecodedFrame, minimum: Fact<i32>, kind| {
        let mut assets = assets();
        assets.naval_combat_min_duration = minimum;
        let world = World::read(frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let common = CommonCells::new(frame);
        Moves::for_common(&world, assets.movement_reads(), &cells, &common)
            .unwrap()
            .combat_minimum_duration(CommonCombat { combat: 9, kind })
    };
    assert!(
        query(&frame, Ok(7), 2)
            .unwrap_err()
            .contains("missing naval combat duration")
    );
    let row = NavalCombatDetailsRow {
        combat: 9,
        duration: 7,
        custom_sea: Some(Vec::new()),
        ..Default::default()
    };
    put::<tables::NavalCombatDetails>(&mut frame, vec![row.clone()]);
    let missing = "missing NDefines.NNavy.COMBAT_MIN_DURATION";
    assert_eq!(query(&frame, Err(missing.into()), 2), Err(missing.into()));
    assert!(
        query(&frame, Ok(7), 3)
            .unwrap_err()
            .contains("kind 3 is unsupported")
    );
    *frame.tables[TableId::NavalCombatDetails.index()] = TableView::Truncated {
        rows: TableRows::NavalCombatDetails(vec![row]),
        missing: 1,
    };
    assert_eq!(query(&frame, Ok(7), 2), Ok(true));
    *frame.tables[TableId::NavalCombatDetails.index()] = TableView::Truncated {
        rows: TableRows::NavalCombatDetails(Vec::new()),
        missing: 1,
    };
    assert!(
        query(&frame, Ok(7), 2)
            .unwrap_err()
            .contains("missing naval combat duration")
    );
    *frame.tables[TableId::NavalCombatDetails.index()] = TableView::Unavailable {
        reason: "duration capture failed".into(),
    };
    assert!(
        query(&frame, Ok(7), 2)
            .unwrap_err()
            .contains("duration capture failed")
    );
}

#[test]
fn selected_transport_access_stage_reads_the_recorded_relations() {
    use replay_data::{rows::DivisionRow, rows::ProvinceRow, rows::RelationLinkRow};
    let mut frame = subject_frame();
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: FORCE,
            province: 2,
            logical_owner: 30,
            ..Default::default()
        }],
    );
    put::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 5,
            controller: 40,
            ..Default::default()
        }],
    );
    let access = |frame: &DecodedFrame| {
        let assets = assets();
        let world = World::read(frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let common = CommonCells::new(frame);
        let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
        transport::AdmissionInputs::military_access(&mut PartialAdmission {
            moves: &moves,
            unit: transport::Unit::Army(FORCE),
            source: 2,
            target: 5,
        })
    };
    assert_eq!(access(&frame), Ok(false));
    let mut links = frame.complete::<tables::RelationLinks>().unwrap().to_vec();
    links.push(RelationLinkRow {
        country: 30,
        target_index: 3,
        links: RelationLinkRow::MILITARY_ACCESS,
        transport_subject: Recorded::NotRecorded,
    });
    put::<tables::RelationLinks>(&mut frame, links);
    assert_eq!(access(&frame), Ok(true));
    put::<tables::Provinces>(&mut frame, Vec::new());
    assert!(access(&frame).is_err());
}
