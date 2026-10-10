use super::*;
use replay::rows::{
    CountryWarRow, DivisionMovementRow, LexicalTokenRow, ModifierDefinitionRow, OrderedEnemyInputs,
    ProvinceAreaInputs, ProvinceRow, SelectedAreaNode, SelectedUnitInputs,
};

fn apply<T>(frame: &DecodedFrame, query: impl FnOnce(&Moves<'_, '_>) -> T) -> T {
    let mut assets = assets();
    assets.geometry.nonsea_indices = (0..10).map(|id| (id, id as i32)).collect();
    let world = World::read(frame, assets.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    query(&Moves::for_common(&world, &assets, &cells, &common).unwrap())
}
fn area(hour: i32, controller: u32, owner: Option<u32>) -> Recorded<Option<ProvinceAreaInputs>> {
    Some(ProvinceAreaInputs {
        sample_hour: hour,
        controller,
        owner,
        node: None,
    })
    .into()
}
#[test]
fn selected_area_identities_are_independent_current_and_never_controller_inference() {
    let mut frame = strait_frame();
    put::<tables::Provinces>(
        &mut frame,
        vec![
            ProvinceRow {
                id: 1,
                controller: 30,
                selected_areas: area(12, 1, Some(1)),
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                controller: 30,
                selected_areas: area(12, 2, Some(1)),
                ..Default::default()
            },
            ProvinceRow {
                id: 3,
                controller: 40,
                selected_areas: area(12, 1, Some(3)),
                ..Default::default()
            },
        ],
    );
    apply(&frame, |moves| {
        assert_eq!(moves.controller_area_equal(1, 2), Ok(false));
        assert_eq!(moves.owner_area_equal(1, 2), Ok(true));
        assert_eq!(moves.controller_area_equal(1, 3), Ok(true));
        assert_eq!(moves.owner_area_equal(1, 3), Ok(false));
    });
    for field in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        area(11, 1, Some(1)),
    ] {
        let mut rows = frame.complete::<tables::Provinces>().unwrap().to_vec();
        rows[0].selected_areas = field;
        put::<tables::Provinces>(&mut frame, rows);
        assert!(apply(&frame, |moves| moves.controller_area_equal(1, 3)).is_err());
    }
}
#[test]
fn selected_army_route_actual_zero_war_uses_the_current_borrowed_owner() {
    use replay::rows::{
        ReservedCountryNamingRow, ReservedReverseVisibilityInputs, ReverseZeroVisibilityInputs,
    };
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
    put::<tables::Provinces>(
        &mut frame,
        vec![
            ProvinceRow {
                id: 1,
                controller: 40,
                owner: 40,
                selected_areas: area(12, 1, Some(1)),
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                controller: 30,
                owner: 30,
                selected_areas: area(12, 2, Some(1)),
                ..Default::default()
            },
        ],
    );
    // A successful D8/F09D0 early return must not demand access, factions,
    // subjects, raw exile or the D0 search graph. The war cell is independent.
    for table in [
        TableId::RelationLinks,
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::DivisionMovement,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "active war must leave later operands unreached".into(),
        };
    }
    let mut assets = assets();
    assets.geometry.land.extend([1, 2]);
    assets.moves = Ok(installed_content::unit_path::MoveGraph::new(
        &installed_content::province_graph::ProvinceGraph::default(),
        &assets.geometry,
    ));
    let query = |frame: &DecodedFrame, raw| {
        let world = World::read(frame, assets.facts());
        let cells = Cells::default();
        let common = CommonCells::new(frame);
        let moves = Moves::for_common(&world, &assets, &cells, &common).unwrap();
        let army = Army {
            location: 1,
            ..Default::default()
        };
        let unit = CommonUnit {
            class: CommonClass::Army,
            handle: FORCE,
            logical_owner_raw: raw,
            combat_count: 0,
            army: &army,
        };
        installed_content::unit_path::Mover {
            graph: assets.moves.as_ref().unwrap(),
            world: &moves,
        }
        .selected_army_path(&unit, 2)
    };
    for (hour, war, reason) in [
        (12, Some([1, 0]), None),
        (11, Some([1, 0]), Some("sample hour mismatch")),
        (12, None, Some("war capture refused")),
    ] {
        put::<tables::ReservedCountryNaming>(
            &mut frame,
            vec![ReservedCountryNamingRow {
                reverse_visibility: Some(ReservedReverseVisibilityInputs {
                    sample_hour: hour,
                    cells: vec![
                        None,
                        None,
                        Some(ReverseZeroVisibilityInputs {
                            war,
                            ..Default::default()
                        }),
                    ],
                })
                .into(),
                ..Default::default()
            }],
        );
        for raw in [0, 12] {
            match reason {
                None => assert_eq!(query(&frame, raw), Ok(true)),
                Some(reason) => assert!(query(&frame, raw).unwrap_err().contains(reason)),
            }
        }
    }
}

#[test]
fn selected_graph_keeps_repeats_and_field_local_refusals_and_authenticates_dictionary_root() {
    let mut frame = strait_frame();
    let packet = ProvinceAreaInputs {
        sample_hour: 12,
        controller: 1,
        owner: None,
        node: Some(SelectedAreaNode {
            first_land: Some(3),
            nonsea_index: Some(7),
            nonsea_count: Some(2),
            land_count: Some(-1),
            neighbours: Some(vec![2, 2, 3]),
        }),
    };
    put::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            selected_areas: Some(packet.clone()).into(),
            ..Default::default()
        }],
    );
    apply(&frame, |moves| {
        let node = moves.area_node(1).unwrap();
        assert_eq!(node.neighbors, Ok(vec![2, 2, 3]));
        assert_eq!(node.land_count, Ok(-1));
        assert_eq!(moves.area_nonsea_count(1), Ok(10));
        assert!(moves.owner_area_equal(1, 1).is_err());
    });
    let mut refused = packet.clone();
    refused.node.as_mut().unwrap().neighbours = None;
    put::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            selected_areas: Some(refused).into(),
            ..Default::default()
        }],
    );
    apply(&frame, |moves| {
        let node = moves.area_node(1).unwrap();
        assert!(node.neighbors.is_err());
        assert_eq!(node.land_count, Ok(-1));
    });
    let mut wrong = packet;
    wrong.controller = 2;
    put::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            selected_areas: Some(wrong).into(),
            ..Default::default()
        }],
    );
    assert!(apply(&frame, |moves| moves.area_node(1)).is_err());
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
fn selected(frame: &mut DecodedFrame, value: Recorded<Option<SelectedUnitInputs>>) {
    put::<tables::DivisionMovement>(
        frame,
        vec![DivisionMovementRow {
            division: FORCE,
            selected: value,
            ..Default::default()
        }],
    );
}
fn retreat_definitions(frame: &mut DecodedFrame) {
    let keys = replay::fixed_layout::RETREAT_BANS.keys;
    put::<tables::ModifierDefinitions>(
        frame,
        keys.iter()
            .map(|&index| ModifierDefinitionRow {
                index,
                token: index,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::LexicalTokens>(
        frame,
        keys.iter()
            .map(|&index| LexicalTokenRow {
                token: index,
                literal: engine::modifier_keys::ALL
                    .iter()
                    .find(|key| key.index == index)
                    .unwrap()
                    .name
                    .as_bytes()
                    .to_vec(),
            })
            .collect(),
    );
}
#[test]
fn selected_retreat_uses_independent_current_values_and_known_null_commander() {
    let mut frame = strait_frame();
    let army = Army::default();
    selected(
        &mut frame,
        Some(SelectedUnitInputs {
            sample_hour: 12,
            exiled: Some(255),
            commander: Some(false),
            retreat: [None; 2],
        })
        .into(),
    );
    apply(&frame, |moves| {
        assert_eq!(
            moves.unit_retreat_ban(&unit(&army, CommonClass::Army), false),
            Ok(false)
        );
        assert_eq!(
            moves.unit_exiled_raw(&unit(&army, CommonClass::Army)),
            Ok(255)
        );
    });
    retreat_definitions(&mut frame);
    selected(
        &mut frame,
        Some(SelectedUnitInputs {
            sample_hour: 12,
            exiled: Some(1),
            commander: Some(true),
            retreat: [Some(1), None],
        })
        .into(),
    );
    apply(&frame, |moves| {
        assert_eq!(
            moves.unit_retreat_ban(&unit(&army, CommonClass::Army), false),
            Ok(true)
        );
        assert!(
            moves
                .unit_retreat_ban(&unit(&army, CommonClass::Army), true)
                .is_err()
        );
    });
    for value in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Some(SelectedUnitInputs {
            sample_hour: 11,
            commander: Some(false),
            ..Default::default()
        })
        .into(),
    ] {
        selected(&mut frame, value);
        assert!(
            apply(&frame, |moves| moves
                .unit_retreat_ban(&unit(&army, CommonClass::Army), false))
            .is_err()
        );
    }
    // Class13 vf8 ReturnZero is known before any Army/daily packet or layout.
    *frame.tables[TableId::DivisionMovement.index()] = TableView::Unavailable {
        reason: "must remain unreached".into(),
    };
    assert_eq!(
        apply(&frame, |moves| moves
            .unit_retreat_ban(&unit(&army, CommonClass::RailwayGun), true)),
        Ok(false)
    );
}
#[test]
fn selected_retreat_layout_mismatch_masks_values_but_preserves_null_context() {
    let mut frame = strait_frame();
    let army = Army::default();
    retreat_definitions(&mut frame);
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens[0].literal = b"wrong".to_vec();
    put::<tables::LexicalTokens>(&mut frame, tokens);
    for (commander, success) in [(true, false), (false, true)] {
        selected(
            &mut frame,
            Some(SelectedUnitInputs {
                sample_hour: 12,
                commander: Some(commander),
                retreat: [Some(1); 2],
                ..Default::default()
            })
            .into(),
        );
        assert_eq!(
            apply(&frame, |moves| moves
                .unit_retreat_ban(&unit(&army, CommonClass::Army), false))
            .is_ok(),
            success
        );
    }
}
#[test]
fn selected_enemy_occurrences_keep_order_zero_and_repeats_and_refuse_carried_or_legacy_sets() {
    let mut frame = strait_frame();
    let packet = OrderedEnemyInputs {
        sample_hour: 12,
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
            sample_hour: 11,
            ..packet.clone()
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
    use replay::rows::{
        RelationLinkRow, ReservedCountryNamingRow, ReservedReverseVisibilityInputs,
        ReverseZeroVisibilityInputs, ZeroSubjectInputs,
    };
    let mut frame = strait_frame();
    let mut rows = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    rows.push(CountryCounterInputRow {
        country: 99,
        directed_wars: Some(DirectedWars {
            sample_hour: 12,
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
        // This read succeeds from the original forward cell even without any
        // reconstructed RelationLinks row toward actual zero.
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
                sample_hour: 12,
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
