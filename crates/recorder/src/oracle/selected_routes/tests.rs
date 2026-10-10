use super::*;
use replay::rows::{
    CountryWarRow, DivisionMovementRow, LexicalTokenRow, ModifierDefinitionRow, OrderedEnemyInputs,
    ProvinceRow, SelectedAreaNode,
};

const UNIT: u64 = (7_u64 << 32) | 51;

fn fixture() -> (DecodedFrame, Save) {
    let (frame, saved, _, _) = super::super::tests::fixture();
    (frame, saved)
}

fn geometry() -> installed_content::province_geometry::ProvinceGeometry {
    installed_content::province_geometry::ProvinceGeometry {
        land: BTreeSet::from([1, 2, 3]),
        nonsea_indices: BTreeMap::from([(1, 0), (2, 1), (3, 2)]),
        ..Default::default()
    }
}

fn check_areas(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    areas(frame, Ok(&geometry()), report)
}

#[test]
fn area_indices_use_the_content_nonsea_domain_instead_of_the_land_count() {
    let (mut frame, _) = fixture();
    let geometry = installed_content::province_geometry::ProvinceGeometry {
        land: BTreeSet::from([1, 3]),
        lake: BTreeSet::from([2]),
        nonsea_indices: BTreeMap::from([(0, 0), (1, 1), (2, 2), (3, 3)]),
        ..Default::default()
    };
    let mut rows = area_rows(frame.hour);
    for index in [0, 2] {
        let node = area_mut(&mut rows[index]).node.as_mut().unwrap();
        node.nonsea_count = Some(2);
        node.nonsea_index = Some(node.first_land.unwrap() as i32);
    }
    set_areas(&mut frame, rows.clone());
    let mut report = Report::default();
    areas(&frame, Ok(&geometry), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{report:?}");
    area_mut(&mut rows[0]).node.as_mut().unwrap().nonsea_index = Some(2);
    set_areas(&mut frame, rows);
    let mut report = Report::default();
    areas(&frame, Ok(&geometry), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("nonsea_index for province 1"));
}

#[test]
fn missing_content_keeps_independent_selected_area_refusals_counted() {
    let (mut frame, _) = fixture();
    let mut rows = area_rows(frame.hour);
    area_mut(&mut rows[0]).node.as_mut().unwrap().neighbours = None;
    set_areas(&mut frame, rows);
    let mut report = Report::default();
    assert!(areas(&frame, Err("content refused".into()), &mut report).is_err());
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("node.neighbours"));
}

fn area_rows(hour: i32) -> Vec<ProvinceRow> {
    vec![
        ProvinceRow {
            id: 1,
            selected_areas: Some(ProvinceAreaInputs {
                sample_hour: hour,
                controller: 1,
                owner: Some(0),
                node: Some(SelectedAreaNode {
                    first_land: Some(1),
                    nonsea_index: Some(0),
                    nonsea_count: Some(3),
                    land_count: Some(0),
                    neighbours: Some(vec![3, 3]),
                }),
            })
            .into(),
            ..Default::default()
        },
        ProvinceRow {
            id: 2,
            selected_areas: Some(ProvinceAreaInputs {
                sample_hour: hour,
                controller: 1,
                owner: Some(2),
                node: None,
            })
            .into(),
            ..Default::default()
        },
        ProvinceRow {
            id: 3,
            selected_areas: Some(ProvinceAreaInputs {
                sample_hour: hour,
                controller: 3,
                owner: Some(2),
                node: Some(SelectedAreaNode {
                    first_land: Some(3),
                    nonsea_index: Some(2),
                    nonsea_count: Some(3),
                    land_count: Some(-1),
                    neighbours: Some(vec![1]),
                }),
            })
            .into(),
            ..Default::default()
        },
    ]
}

fn area_mut(row: &mut ProvinceRow) -> &mut ProvinceAreaInputs {
    match &mut row.selected_areas {
        Recorded::Value(Some(packet)) => packet,
        _ => panic!("area packet"),
    }
}

fn set_areas(frame: &mut DecodedFrame, rows: Vec<ProvinceRow>) {
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(rows));
}

#[test]
fn area_checks_preserve_independent_labels_nulls_and_repeated_graph_occurrences() {
    let (mut frame, _) = fixture();
    let hour = frame.hour;
    set_areas(&mut frame, area_rows(hour));
    let mut report = Report::default();
    check_areas(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{report:?}");
    assert!(report.compared_tables.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("list lifetime"))
    );

    let mut rows = area_rows(frame.hour);
    let first = area_mut(&mut rows[0]);
    first.controller = 0;
    first.node = None;
    area_mut(&mut rows[1]).controller = 0;
    area_mut(&mut rows[2]).node.as_mut().unwrap().neighbours = Some(vec![]);
    set_areas(&mut frame, rows);
    let mut report = Report::default();
    check_areas(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{report:?}");
    assert!(report.compared_tables.is_empty());
}

#[test]
fn area_history_read_refusals_and_stale_samples_are_distinct() {
    let (mut frame, _) = fixture();
    for (field, differences, historical) in [
        (Recorded::NotRecorded, 0, true),
        (Recorded::Value(None), 1, false),
        (
            Some(ProvinceAreaInputs {
                sample_hour: frame.hour + 1,
                controller: 0,
                owner: Some(0),
                node: None,
            })
            .into(),
            1,
            false,
        ),
        (
            Some(ProvinceAreaInputs {
                sample_hour: frame.hour,
                controller: 0,
                owner: None,
                node: None,
            })
            .into(),
            1,
            false,
        ),
    ] {
        set_areas(
            &mut frame,
            vec![ProvinceRow {
                id: 1,
                selected_areas: field,
                ..Default::default()
            }],
        );
        let mut report = Report::default();
        check_areas(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert_eq!(
            report
                .limitations
                .contains_key("Provinces.selected_areas not recorded"),
            historical
        );
        assert!(report.compared_tables.is_empty());
    }
}

#[test]
fn negative_area_indices_remain_native_states_with_a_reach_limitation() {
    let (mut frame, _) = fixture();
    for index in [i32::MIN, -1] {
        let mut rows = area_rows(frame.hour);
        area_mut(&mut rows[0]).node.as_mut().unwrap().nonsea_index = Some(index);
        set_areas(&mut frame, rows);
        let mut report = Report::default();
        check_areas(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{report:?}");
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("negative start nonsea index"))
        );
        assert!(report.compared_tables.is_empty());
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn area_bad_root_node_index_and_each_orphan_occurrence_remain_differences() {
    let (mut frame, _) = fixture();
    for change in 0..8 {
        let mut rows = area_rows(frame.hour);
        match change {
            0 => area_mut(&mut rows[1]).node = Some(SelectedAreaNode::default()),
            1 => area_mut(&mut rows[0]).node = None,
            2 => area_mut(&mut rows[0]).node.as_mut().unwrap().first_land = Some(77),
            3 => area_mut(&mut rows[0]).node.as_mut().unwrap().nonsea_index = Some(3),
            4 => area_mut(&mut rows[2]).node.as_mut().unwrap().nonsea_count = Some(4),
            5 => area_mut(&mut rows[0]).node.as_mut().unwrap().neighbours = Some(vec![77, 77]),
            6 => area_mut(&mut rows[0]).node.as_mut().unwrap().neighbours = Some(vec![2]),
            7 => area_mut(&mut rows[2]).owner = Some(1),
            _ => unreachable!(),
        }
        set_areas(&mut frame, rows);
        let mut report = Report::default();
        check_areas(&frame, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            if change == 5 { 2 } else { 1 },
            "{change}: {report:?}"
        );
        assert!(report.compared_tables.is_empty());
    }
}

fn serialized_unit(suffix: &str) -> Save {
    save::parse(format!("HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ AAA={{ units={{ division={{ id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 {suffix} }} }} }} }} states={{}} provinces={{}}")
        .as_bytes()).unwrap()
}

fn unit_row(hour: i32) -> DivisionMovementRow {
    DivisionMovementRow {
        division: UNIT,
        selected: Some(SelectedUnitInputs {
            sample_hour: hour,
            exiled: Some(0),
            commander: None,
            retreat: [None; 2],
        })
        .into(),
        ..Default::default()
    }
}

fn set_unit(frame: &mut DecodedFrame, row: DivisionMovementRow) {
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![row]));
}

#[test]
fn saved_army_exile_checks_only_exact_truth_and_never_certifies_raw_magnitude() {
    let (mut frame, _) = fixture();
    for (suffix, raw, differences) in [
        ("", 0, 0),
        ("exile=no", 0, 0),
        ("exile=yes", 1, 0),
        ("exile=yes", 255, 0),
        ("exile=yes", 0, 1),
        ("", 255, 1),
    ] {
        let mut row = unit_row(frame.hour);
        row.selected = Some(SelectedUnitInputs {
            exiled: Some(raw),
            sample_hour: frame.hour,
            ..Default::default()
        })
        .into();
        set_unit(&mut frame, row);
        let mut report = Report::default();
        units(&frame, &serialized_unit(suffix), &mut report).unwrap();
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert_eq!(
            report.compared_tables["DivisionMovement.selected exile saved truth"],
            1
        );
        assert_eq!(report.scalar_checks, 1);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("XORs the raw byte"))
        );
    }
}

#[test]
fn unit_history_staleness_and_isolated_exile_refusal_do_not_claim_save_acceptance() {
    let (mut frame, _) = fixture();
    for (field, differences) in [
        (Recorded::NotRecorded, 0),
        (Recorded::Value(None), 1),
        (
            Some(SelectedUnitInputs {
                sample_hour: frame.hour + 1,
                exiled: Some(0),
                ..Default::default()
            })
            .into(),
            1,
        ),
        (
            Some(SelectedUnitInputs {
                sample_hour: frame.hour,
                exiled: None,
                ..Default::default()
            })
            .into(),
            1,
        ),
    ] {
        let mut row = unit_row(frame.hour);
        row.selected = field;
        set_unit(&mut frame, row);
        let mut report = Report::default();
        units(&frame, &serialized_unit(""), &mut report).unwrap();
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert!(report.compared_tables.is_empty());
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn saved_exile_difference_does_not_hide_an_existing_stored_path_difference() {
    let (mut frame, _) = fixture();
    let mut row = unit_row(frame.hour);
    row.path = vec![9];
    row.path_total = 1;
    row.withdrawal = Some(0).into();
    row.movement_priority = Some(1).into();
    row.fresh = Recorded::NotRecorded;
    set_unit(&mut frame, row);
    let saved = serialized_unit("exile=yes");
    let mut report = Report::default();
    units(&frame, &saved, &mut report).unwrap();
    super::super::military::compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{report:?}");
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("exiled saved truth"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("path/total"))
    );
}

fn retreat_layout(frame: &mut DecodedFrame) {
    engine::executable_tables::initialize_for_tests()
        .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
    let keys: Vec<_> = engine::modifier_keys::all()
        .filter(|key| replay::fixed_layout::RETREAT_BANS.keys.contains(&key.index))
        .collect();
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| ModifierDefinitionRow {
                    index: key.index,
                    token: key.index + 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        keys.iter()
            .map(|key| LexicalTokenRow {
                token: key.index + 1,
                literal: key.name.as_bytes().to_vec(),
            })
            .collect(),
    ));
}

#[test]
fn current_retreat_checks_null_context_independent_failures_and_layout_without_save_credit() {
    let (mut frame, _) = fixture();
    retreat_layout(&mut frame);
    for (commander, retreat, differences) in [
        (Some(false), [Some(0); 2], 0),
        (Some(false), [Some(1), Some(0)], 1),
        (None, [None; 2], 1),
        (Some(true), [Some(i64::MIN), Some(i64::MAX)], 0),
        (Some(true), [None, Some(0)], 1),
        (Some(true), [Some(0), None], 1),
        (Some(true), [None; 2], 2),
    ] {
        let packet = SelectedUnitInputs {
            sample_hour: frame.hour,
            exiled: Some(0),
            commander,
            retreat,
        };
        let mut report = Report::default();
        super::retreat(&frame, &mut report, "unit", &packet);
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert!(report.compared_tables.is_empty());
        assert_eq!(report.scalar_checks, 0);
    }
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![]));
    let packet = SelectedUnitInputs {
        commander: Some(true),
        retreat: [Some(0); 2],
        ..Default::default()
    };
    let mut report = Report::default();
    super::retreat(&frame, &mut report, "unit", &packet);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("fixed layout"));
    let packet = SelectedUnitInputs {
        commander: Some(false),
        ..packet
    };
    let mut report = Report::default();
    super::retreat(&frame, &mut report, "unit", &packet);
    assert!(report.differences.is_empty());
}

#[test]
fn ordered_enemies_accept_reserved_zero_repeats_and_native_order_without_save_credit() {
    let (mut frame, _) = fixture();
    *frame.tables[TableId::CountryWars.index()] =
        TableView::Complete(TableRows::CountryWars(vec![
            CountryWarRow {
                country: 0,
                enemies_read: 0,
                enemies: vec![],
                ordered: Some(OrderedEnemyInputs {
                    sample_hour: frame.hour,
                    entries: vec![77, 0, 77, 9],
                })
                .into(),
                ..Default::default()
            },
            CountryWarRow {
                country: 77,
                enemies_read: 1,
                enemies: vec![77, 0, 9, 77],
                ordered: Some(OrderedEnemyInputs {
                    sample_hour: frame.hour,
                    entries: vec![77, 0, 9, 77],
                })
                .into(),
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    enemies(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{report:?}");
    assert!(report.compared_tables.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("not this vector"))
    );

    let TableView::Complete(TableRows::CountryWars(rows)) =
        &mut *frame.tables[TableId::CountryWars.index()]
    else {
        panic!("enemies");
    };
    rows[1].enemies.swap(1, 3);
    let mut report = Report::default();
    enemies(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("legacy shared vector"));
}

#[test]
fn ordered_enemy_history_refusal_staleness_and_empty_current_packet_are_distinct() {
    let (mut frame, _) = fixture();
    for (field, differences, historical) in [
        (Recorded::NotRecorded, 0, true),
        (Recorded::Value(None), 1, false),
        (
            Some(OrderedEnemyInputs {
                sample_hour: frame.hour + 1,
                entries: vec![],
            })
            .into(),
            1,
            false,
        ),
        (
            Some(OrderedEnemyInputs {
                sample_hour: frame.hour,
                entries: vec![],
            })
            .into(),
            0,
            false,
        ),
    ] {
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
                country: 0,
                ordered: field,
                ..Default::default()
            }]));
        let mut report = Report::default();
        enemies(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert_eq!(
            report
                .limitations
                .contains_key("CountryWars.ordered not recorded"),
            historical
        );
        assert!(report.compared_tables.is_empty());
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn whole_frame_historical_suffixes_are_counted_once_without_comparison_credit() {
    let (mut frame, saved) = fixture();
    set_areas(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            selected_areas: Recorded::NotRecorded,
            ..Default::default()
        }],
    );
    let mut row = unit_row(frame.hour);
    row.selected = Recorded::NotRecorded;
    set_unit(&mut frame, row);
    *frame.tables[TableId::CountryWars.index()] =
        TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
            country: 0,
            ordered: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let mut report = Report::default();
    report.record_tables(&frame);
    compare(&frame, &saved, Err("unreached content".into()), &mut report);
    assert!(report.differences.is_empty(), "{report:?}");
    for field in [
        "Provinces.selected_areas",
        "DivisionMovement.selected",
        "CountryWars.ordered",
    ] {
        assert_eq!(
            report.limitations[&format!("{field} not recorded")].len(),
            1
        );
    }
    assert!(report.compared_tables.is_empty());
    assert_eq!(report.scalar_checks, 0);
}
