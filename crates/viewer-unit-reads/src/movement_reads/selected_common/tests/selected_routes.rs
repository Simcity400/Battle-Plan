use super::*;
use replay_data::{
    rows::ControllerAreaNode, rows::CountryWarRow, rows::DivisionStatusRow,
    rows::OrderedEnemyInputs, rows::ProvinceRow,
};

fn apply<T>(frame: &DecodedFrame, query: impl FnOnce(&Moves<'_, '_>) -> T) -> T {
    let mut assets = assets();
    assets.geometry.nonsea_indices = (0..10).map(|id| (id, id as i32)).collect();
    let world = World::read(frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    query(&Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap())
}
fn unit<'a>(army: &'a Army, class: CommonClass) -> CommonUnit<'a> {
    CommonUnit {
        class,
        handle: FORCE,
        logical_owner_raw: 20,
        combat_count: 1,
        army,
    }
}
fn province(id: u32, controller: u32, owner: u32, node: Option<ControllerAreaNode>) -> ProvinceRow {
    ProvinceRow {
        id,
        controller: 30,
        owner: 20,
        controller_area: Recorded::Value(Some(controller)),
        owner_area: Recorded::Value(Some(owner)),
        area_node: Recorded::Value(node),
        ..Default::default()
    }
}
fn areas() -> Vec<ProvinceRow> {
    vec![
        province(
            1,
            1,
            1,
            Some(ControllerAreaNode {
                first_land: Some(2),
                neighbours: Some(vec![3, 3]),
            }),
        ),
        province(2, 1, 1, None),
        province(
            3,
            3,
            3,
            Some(ControllerAreaNode {
                first_land: Some(3),
                neighbours: None,
            }),
        ),
        province(4, 0, 0, None),
    ]
}
#[test]
fn selected_areas_read_the_recorded_labels_and_the_node_on_the_label_province() {
    let mut frame = strait_frame();
    put::<tables::Provinces>(&mut frame, areas());
    apply(&frame, |moves| {
        assert_eq!(moves.controller_area_equal(1, 2), Ok(true));
        assert_eq!(moves.controller_area_equal(2, 3), Ok(false));
        assert_eq!(moves.controller_area_equal(4, 4), Ok(true));
        assert_eq!(moves.owner_area_equal(1, 2), Ok(true));
        assert_eq!(moves.owner_area_equal(1, 3), Ok(false));
        assert_eq!(moves.controller_area(2), Ok(1));
        assert_eq!(moves.controller_area(4), Ok(0));
        let node = moves.area_node(1).unwrap();
        assert_eq!(node.first_land_province, Ok(2));
        assert_eq!(node.nonsea_index, Ok(2));
        assert_eq!(node.land_count, Ok(2));
        assert_eq!(node.neighbors, Ok(vec![3, 3]));
        assert_eq!(moves.area_nonsea_count(1), Ok(10));
        let node = moves.area_node(3).unwrap();
        assert_eq!(node.land_count, Ok(1));
        assert!(node.neighbors.is_err());
        assert!(moves.area_node(2).is_err());
        assert!(moves.area_node(0).is_err());
    });
    for field in [Recorded::Value(None), Recorded::NotRecorded] {
        let mut rows = areas();
        rows[1].controller_area = field;
        put::<tables::Provinces>(&mut frame, rows);
        apply(&frame, |moves| {
            assert!(moves.controller_area_equal(1, 2).is_err());
            assert!(moves.area_node(1).unwrap().land_count.is_err());
        });
    }
}
#[test]
fn a_selected_army_reads_exile_from_its_status_and_retreat_bans_from_the_hour() {
    use replay_data::{rows::DivisionRetreatBanRow, rows::DivisionRow};
    let mut frame = strait_frame();
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: FORCE,
            roster_owner: 30,
            logical_owner: 20,
            province: 1,
            ..Default::default()
        }],
    );
    put::<tables::DivisionStatus>(
        &mut frame,
        vec![DivisionStatusRow {
            division: FORCE,
            move_flags: DivisionStatusRow::MOVE_EXILED,
            ..Default::default()
        }],
    );
    put::<tables::DivisionRetreatBans>(
        &mut frame,
        vec![DivisionRetreatBanRow {
            division: FORCE,
            owner: 2,
            bans: DivisionRetreatBanRow::ATTACKING,
        }],
    );
    put::<tables::SliceReads>(&mut frame, Vec::new());
    let army = Army::default();
    apply(&frame, |moves| {
        let selected = unit(&army, CommonClass::Army);
        assert_eq!(moves.unit_exiled_raw(&selected), Ok(1));
        assert_eq!(moves.unit_retreat_ban(&selected, true), Ok(true));
        assert_eq!(moves.unit_retreat_ban(&selected, false), Ok(false));
    });
    *frame.tables[TableId::DivisionRetreatBans.index()] = TableView::Unavailable {
        reason: "must remain unreached".into(),
    };
    assert_eq!(
        apply(&frame, |moves| moves
            .unit_retreat_ban(&unit(&army, CommonClass::RailwayGun), true)),
        Ok(false)
    );
}
#[test]
fn the_move_fallback_names_a_target_without_a_naval_base_and_refuses_past_it() {
    use replay_data::rows::NavalBaseRow;
    let mut frame = strait_frame();
    put::<tables::NavalBases>(
        &mut frame,
        vec![NavalBaseRow {
            province: 3,
            ..Default::default()
        }],
    );
    let loc = [("TRANSPORT_NO_NAVAL_BASE".into(), "No naval base".into())]
        .into_iter()
        .collect();
    let keys = BTreeMap::new();
    let content = CommonText {
        frame: &frame,
        loc: &loc,
        recorded_language: "english",
        state_keys: &keys,
    };
    let army = Army {
        location: 1,
        ..Default::default()
    };
    apply(&frame, |moves| {
        let selected = unit(&army, CommonClass::Army);
        let mut text = String::new();
        assert_eq!(
            moves.selected_transport(&selected, 2, &mut text, &content),
            Ok(false)
        );
        assert_eq!(text, "No naval base");
        let mut text = String::new();
        assert!(
            moves
                .selected_transport(&selected, 3, &mut text, &content)
                .is_err()
        );
        assert!(text.is_empty());
    });
}
#[test]
fn selected_army_route_takes_the_owner_area_early_accept_before_later_operands() {
    let mut frame = strait_frame();
    put::<tables::Provinces>(&mut frame, areas());
    for table in [
        TableId::RelationLinks,
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::DivisionMovement,
        TableId::DivisionStatus,
        TableId::ReservedCountryNaming,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "later operands must remain unreached".into(),
        };
    }
    let mut assets = assets();
    assets.geometry.land.extend([1, 2, 3]);
    assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
        &content_reader_map::province_graph::ProvinceGraph::default(),
        &assets.geometry,
    ));
    let world = World::read(&frame, assets.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(&frame);
    let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
    let army = Army {
        location: 1,
        ..Default::default()
    };
    let mover = content_reader_unit_path::unit_path::Mover {
        graph: assets.moves.as_ref().unwrap(),
        world: &moves,
    };
    assert_eq!(
        mover.selected_army_path(&unit(&army, CommonClass::Army), 2),
        Ok(true)
    );
    assert!(
        mover
            .selected_army_path(&unit(&army, CommonClass::Army), 3)
            .unwrap_err()
            .contains("later operands must remain unreached")
    );
}
#[test]
fn selected_enemy_occurrences_keep_order_zero_and_repeats_and_refuse_carried_or_legacy_sets() {
    let mut frame = strait_frame();
    let packet = OrderedEnemyInputs {
        sample_hour: replay_data::rows::PacketHour::Stamped(12),
        entries: vec![40, 0, 40, 99],
    };
    put::<tables::CountryWars>(
        &mut frame,
        vec![CountryWarRow {
            country: 99,
            enemies_read: 1,
            enemies: vec![30],
            ordered: Some(packet.clone()).into(),
            ..Default::default()
        }],
    );
    assert_eq!(
        apply(&frame, |moves| moves.ordered_enemy_raws(1)),
        Ok(packet.entries.clone())
    );
    for field in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Some(OrderedEnemyInputs {
            sample_hour: replay_data::rows::PacketHour::Stamped(11),
            ..packet
        })
        .into(),
    ] {
        let mut rows = frame.complete::<tables::CountryWars>().unwrap().to_vec();
        rows[0].ordered = field;
        put::<tables::CountryWars>(&mut frame, rows);
        assert!(apply(&frame, |moves| moves.ordered_enemy_raws(1)).is_err());
    }
}

#[test]
fn railway_country_access_uses_direct_veto_exile_law_and_zero_cell_military_pointer() {
    use replay_data::{
        rows::RelationLinkRow, rows::ReservedCountryNamingRow,
        rows::ReservedReverseVisibilityInputs, rows::ReverseZeroVisibilityInputs,
        rows::ZeroSubjectInputs,
    };
    let mut frame = strait_frame();
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    rows.push(CountryCounterInputRow {
        country: 99,
        directed_wars: Some(DirectedWars {
            sample_hour: replay_data::rows::PacketHour::Stamped(12),
            receiver_raw_id: 99,
            receiver_slot: 1,
            matrix_count: 4,
            cells: vec![WarCell {
                target_slot: 2,
                active: false,
            }],
            ..Default::default()
        })
        .into(),
        forward_visibility: Some(ReverseZeroVisibilityInputs {
            war: Some([0, 0]),
            military_access: Some(1),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    });
    rows.sort_unstable_by_key(|row| row.country);
    put::<tables::CountryCounterInputs>(&mut frame, rows);
    put::<tables::RelationLinks>(
        &mut frame,
        vec![RelationLinkRow {
            country: 99,
            target_index: 2,
            links: RelationLinkRow::MILITARY_ACCESS,
            transport_subject: Recorded::NotRecorded,
        }],
    );
    apply(&frame, |moves| {
        assert_eq!(moves.railway_path_owner_admits(20, 30, 0), Ok(true));
        assert_eq!(moves.railway_path_owner_admits(20, 30, 255), Ok(true));
        assert_eq!(moves.railway_path_owner_admits(20, 0, 0), Ok(true));
    });
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    let row = rows.iter_mut().find(|row| row.country == 99).unwrap();
    if let Recorded::Value(Some(packet)) = &mut row.directed_wars {
        packet.cells[0].active = true;
    }
    if let Recorded::Value(Some(cell)) = &mut row.forward_visibility {
        cell.military_access = None;
    }
    put::<tables::CountryCounterInputs>(&mut frame, rows);
    apply(&frame, |moves| {
        assert_eq!(moves.railway_path_owner_admits(20, 30, 0), Ok(false));
        assert_eq!(moves.railway_path_owner_admits(20, 30, 1), Ok(false));
        assert!(moves.railway_path_owner_admits(20, 0, 0).is_err());
    });
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            reverse_visibility: Some(ReservedReverseVisibilityInputs {
                sample_hour: replay_data::rows::PacketHour::Stamped(12),
                cells: vec![
                    None,
                    None,
                    Some(ReverseZeroVisibilityInputs {
                        war: Some([0, 0]),
                        military_access: Some(1),
                        subject: Some(ZeroSubjectInputs::default()),
                        ..Default::default()
                    }),
                ],
            })
            .into(),
            ..Default::default()
        }],
    );
    assert_eq!(
        apply(&frame, |moves| moves.railway_path_owner_admits(0, 30, 0)),
        Ok(true)
    );
}
