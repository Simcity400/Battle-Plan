use super::*;
use content_reader_save::{Decimal, Division, GameDate};
use replay_data::{
    rows::AirTheatreGroupRow, rows::AirTheatreMemberRow, rows::AirTheatreRow,
    rows::ArmyGroupMemberRow, rows::DivisionRow, rows::LandTheatreGroupRow,
    rows::LandTheatreMemberRow, rows::LandTheatreRow, rows::NavalResultCountryRow,
    rows::NavalResultMemberRow, rows::NavalResultRow, rows::OrdersGroupMemberRow,
    rows::OrdersGroupRow, rows::RailCooldownRow, rows::RailEdgeRow, rows::Recorded,
    rows::SunkShipsRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn supremacy_frame(rows: Vec<NavalSupremacyRow>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::NavalSupremacy.index()] =
        TableView::Complete(TableRows::NavalSupremacy(rows));
    frame
}

const OOB_SAVE: &[u8] = br#"countries={GER={theatres={theatre={
    id={ id=2 type=67 }
    orders_group={
        id={ id=57 type=53 } name="Army 1"
        member={ unit={ id=12456 type=4713 } }
        member={ unit={ id=111 type=51 } }
        leader_unit={ id=12456 type=4713 }
        leader={ id=279 type=4713 }
        field_marshal_group=no motorization_level=0
    }
    field_marshal_group={
        orders_group={ id=57 type=53 }
        id={ id=431 type=53 } name="Army Group 1"
        field_marshal_group=yes motorization_level=0
    }
    theater_group={ id={ id=45 type=68 } priority=1 name="t"
        orders_group={ id=57 type=53 } orders_group={ id=431 type=53 } }
}}}}"#;

fn oob_handle(id: u64, kind: u64) -> u64 {
    (id << 32) | kind
}

fn oob_frame() -> DecodedFrame {
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: oob_handle(57, 53),
                country: 7,
                is_army_group: 0,
                motorization_level: 0,
                theatre: oob_handle(2, 67),
                theatre_ordinal: Some(0),
                theater_group: oob_handle(45, 68),
                parent: oob_handle(431, 53),
                leader: oob_handle(279, 4713),
                hq_provider: oob_handle(12456, 4713),
                name: Recorded::NotRecorded,
            },
            OrdersGroupRow {
                handle: oob_handle(431, 53),
                country: 7,
                is_army_group: 1,
                motorization_level: 0,
                theatre: oob_handle(2, 67),
                theatre_ordinal: Some(0),
                theater_group: oob_handle(45, 68),
                parent: 0,
                leader: 0,
                hq_provider: 0,
                name: Recorded::NotRecorded,
            },
        ]));
    *frame.tables[TableId::OrdersGroupMembers.index()] =
        TableView::Complete(TableRows::OrdersGroupMembers(vec![
            OrdersGroupMemberRow {
                group: oob_handle(57, 53),
                ordinal: 0,
                unit: oob_handle(12456, 4713),
            },
            OrdersGroupMemberRow {
                group: oob_handle(57, 53),
                ordinal: 1,
                unit: oob_handle(111, 51),
            },
        ]));
    *frame.tables[TableId::ArmyGroupMembers.index()] =
        TableView::Complete(TableRows::ArmyGroupMembers(vec![ArmyGroupMemberRow {
            group: oob_handle(431, 53),
            ordinal: 0,
            child: oob_handle(57, 53),
        }]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: oob_handle(111, 51),
            orders_group: oob_handle(57, 53),
            ..DivisionRow::default()
        },
        DivisionRow {
            id: oob_handle(12456, 4713),
            orders_group: oob_handle(57, 53),
            ..DivisionRow::default()
        },
    ]));
    frame
}

fn oob_divisions() -> BTreeMap<u64, Division> {
    [oob_handle(111, 51), oob_handle(12456, 4713)]
        .into_iter()
        .map(|id| {
            (
                id,
                Division {
                    id,
                    roster_owner: b"GER".to_vec(),
                    logical_owner: b"GER".to_vec(),
                    expeditionary_owner: None,
                    province: None,
                    template: None,
                    alternate_template: None,
                    separate_counter: false,
                    hit_points: Decimal::parse(b"0").unwrap(),
                    organisation: Decimal::parse(b"0").unwrap(),
                    country_intel: Vec::new(),
                    presentation: Default::default(),
                },
            )
        })
        .collect()
}

fn run_oob(frame: &DecodedFrame, divisions: &BTreeMap<u64, Division>) -> Report {
    run_oob_saved(frame, divisions, OOB_SAVE)
}

fn run_oob_saved(frame: &DecodedFrame, divisions: &BTreeMap<u64, Division>, text: &[u8]) -> Report {
    let germany = CountryRow {
        id: 7,
        canonical_slot: 1,
        tag: b"GER".to_vec(),
        ..CountryRow::default()
    };
    let links = BTreeMap::from([(7, &germany)]);
    let saved = saved::parse(&content_reader_parser::parser::parse(text).unwrap().items).unwrap();
    let mut report = Report::default();
    let _ = compare_order_of_battle(frame, divisions, &saved, &links, &mut report);
    report
}

#[test]
fn a_matching_order_of_battle_has_no_differences() {
    let report = run_oob(&oob_frame(), &oob_divisions());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
}

#[test]
fn stored_command_names_compare_presence_before_the_first_nul_prefix() {
    let original = String::from_utf8(OOB_SAVE.to_vec()).unwrap();
    for (bytes, matching_key) in [
        (b"".as_slice(), ""),
        (b"\0retained".as_slice(), "name=\"\""),
        (b"Army 1\0retained".as_slice(), "name=\"Army 1\""),
    ] {
        for saved_key in ["", "name=\"\"", "name=\"Army 1\""] {
            let mut frame = oob_frame();
            let TableView::Complete(TableRows::OrdersGroups(groups)) =
                &mut *frame.tables[TableId::OrdersGroups.index()]
            else {
                unreachable!()
            };
            groups[0].name = Recorded::Value(Some(bytes.to_vec()));
            let saved = original.replace("name=\"Army 1\"", saved_key);
            let report = run_oob_saved(&frame, &oob_divisions(), saved.as_bytes());
            assert!(
                report.failed_comparisons.is_empty(),
                "{:?}",
                report.failed_comparisons
            );
            assert_eq!(
                report.differences.is_empty(),
                saved_key == matching_key,
                "{:?}",
                report.differences
            );
        }
    }
}

#[test]
fn identity_membership_and_parent_disagreements_are_each_reported_once() {
    let mut frame = oob_frame();
    let TableView::Complete(TableRows::OrdersGroups(groups)) =
        &mut *frame.tables[TableId::OrdersGroups.index()]
    else {
        unreachable!()
    };
    groups[0].leader = 0;
    groups[0].theater_group = 0;
    groups[0].motorization_level = 4;
    groups[1].parent = oob_handle(57, 53);
    let TableView::Complete(TableRows::OrdersGroupMembers(members)) =
        &mut *frame.tables[TableId::OrdersGroupMembers.index()]
    else {
        unreachable!()
    };
    members[1].unit = oob_handle(999, 4713);
    let report = run_oob(&frame, &oob_divisions());
    let differences = report.differences.join("\n");
    assert!(
        differences.contains("orders group 244813135925.leader"),
        "{differences}"
    );
    assert!(differences.contains("theater_group"), "{differences}");
    assert!(differences.contains("motorization_level"), "{differences}");
    assert!(differences.contains("parent"), "{differences}");
    assert!(differences.contains("member 1"), "{differences}");
    assert!(
        differences.contains("recorded member vector"),
        "{differences}"
    );
}

#[test]
fn a_division_parent_pointer_is_compared_only_where_the_save_has_that_division() {
    let mut frame = oob_frame();
    let TableView::Complete(TableRows::Divisions(divisions)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    divisions.push(DivisionRow {
        id: oob_handle(65580, 4713),
        orders_group: 0,
        ..DivisionRow::default()
    });
    let report = run_oob(&frame, &oob_divisions());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["Divisions.orders_group"], 2);
    assert_eq!(
        report.compared_tables["OrdersGroupMembers against division parent pointers"],
        3
    );
}

#[test]
fn unavailable_tables_do_not_suppress_independent_naval_and_theatre_diagnostics() {
    let saved = saved::parse(&content_reader_parser::parser::parse(br#"
        region={} rail_way={rail_way={1={cooldown=2}}}
        countries={GER={
            theatres={theatre={id={id=0 type=67}
                theater_group={id={id=0 type=68} priority=1 name="land" orders_group={id=0 type=53}}}}
            air_theatre={id={id=0 type=77} country=GER name="theatre"
                air_group={id={id=0 type=78} air_theatre={id=0 type=77} name="air" air_wing={{id=0 type=69}}}}
        }}
    "#).unwrap().items).unwrap();
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::SunkConvoys.index()] = TableView::Unavailable {
        reason: "convoy read failed".into(),
    };
    *frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]));
    *frame.tables[TableId::NavalResults.index()] = TableView::Unavailable {
        reason: "result read failed".into(),
    };
    *frame.tables[TableId::RailCooldowns.index()] =
        TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
            province: 1,
            cooldown_ticks: 1,
            building: replay_data::rows::Recorded::Value(None),
            selection: replay_data::rows::Recorded::NotRecorded,
            selection_hour: Default::default(),
            building_bound: Default::default(),
        }]));
    *frame.tables[TableId::LandTheatres.index()] = TableView::Unavailable {
        reason: "land theatre read failed".into(),
    };
    *frame.tables[TableId::LandTheatreGroups.index()] =
        TableView::Complete(TableRows::LandTheatreGroups(vec![LandTheatreGroupRow {
            id: 68,
            theatre: 67,
            ordinal: 0,
            priority: 2,
            name: b"land".to_vec(),
        }]));
    *frame.tables[TableId::LandTheatreMembers.index()] =
        TableView::Complete(TableRows::LandTheatreMembers(vec![LandTheatreMemberRow {
            theater_group: 68,
            orders_group: 53,
            ordinal: 1,
        }]));
    *frame.tables[TableId::AirTheatres.index()] = TableView::Truncated {
        rows: TableRows::AirTheatres(Vec::new()),
        missing: 1,
    };
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
            id: 78,
            theatre: 77,
            ordinal: 0,
            name: b"air".to_vec(),
        }]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
            group: 78,
            wing: 69,
            ordinal: 9,
        }]));
    let mut report = Report::default();
    assert!(
        compare(
            &frame,
            &saved,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut report
        )
        .is_err()
    );
    for group in [
        "naval SunkConvoys",
        "naval NavalResults",
        "naval LandTheatres",
        "naval AirTheatres",
    ] {
        assert!(
            report.failed_comparisons.contains_key(group),
            "missing failure for {group}"
        );
    }
    assert_eq!(report.failed_comparisons.len(), 4);
    for table in [
        "SunkShips",
        "NavalResultMembers",
        "NavalResultCountries",
        "NavalSupremacy",
    ] {
        assert_eq!(
            report.compared_tables.get(table),
            Some(&0),
            "missing successful empty comparison for {table}"
        );
    }
    for table in [
        "RailCooldowns",
        "LandTheatreGroups",
        "LandTheatreMembers",
        "AirTheatreGroups",
        "AirTheatreMembers",
    ] {
        assert_eq!(
            report.compared_tables.get(table),
            Some(&1),
            "missing comparison for {table}"
        );
    }
    assert_eq!(report.differences.len(), 4);
    for group in [
        "naval RailCooldowns",
        "naval LandTheatreGroups",
        "naval LandTheatreMembers",
        "naval AirTheatreMembers",
    ] {
        assert_eq!(
            report.difference_groups.get(group),
            Some(&1),
            "missing difference for {group}"
        );
    }
}

#[test]
fn railway_cooldown_omission_does_not_invent_zero() {
    let saved = saved::parse(
        &content_reader_parser::parser::parse(b"rail_way={rail_way={1={cooldown=2} 2={}}}")
            .unwrap()
            .items,
    )
    .unwrap();
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::RailCooldowns.index()] =
        TableView::Complete(TableRows::RailCooldowns(vec![
            RailCooldownRow {
                province: 1,
                cooldown_ticks: 2,
                building: replay_data::rows::Recorded::Value(None),
                selection: replay_data::rows::Recorded::NotRecorded,
                selection_hour: Default::default(),
                building_bound: Default::default(),
            },
            RailCooldownRow {
                province: 2,
                cooldown_ticks: i32::MIN,
                building: replay_data::rows::Recorded::Value(None),
                selection: replay_data::rows::Recorded::NotRecorded,
                selection_hour: Default::default(),
                building_bound: Default::default(),
            },
        ]));
    *frame.tables[TableId::RailEdges.index()] =
        TableView::Complete(TableRows::RailEdges(vec![RailEdgeRow {
            from: 1,
            to: 2,
            level: i32::MAX,
        }]));
    let mut report = Report::default();
    compare_railways(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["RailCooldowns"], 2);
    assert!(!report.compared_tables.contains_key("RailEdges"));
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("signed-nonpositive cooldown"))
    );
    *frame.tables[TableId::RailCooldowns.index()] =
        TableView::Complete(TableRows::RailCooldowns(vec![
            RailCooldownRow {
                province: 1,
                cooldown_ticks: 1,
                building: replay_data::rows::Recorded::Value(None),
                selection: replay_data::rows::Recorded::NotRecorded,
                selection_hour: Default::default(),
                building_bound: Default::default(),
            },
            RailCooldownRow {
                province: 2,
                cooldown_ticks: 1,
                building: replay_data::rows::Recorded::Value(None),
                selection: replay_data::rows::Recorded::NotRecorded,
                selection_hour: Default::default(),
                building_bound: Default::default(),
            },
        ]));
    compare_railways(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

fn rail_adjacency() -> saved::RailwayAdjacency {
    let definitions = b"0;0;0;0;land\n1;1;0;0;land\n2;2;0;0;sea\n3;3;0;0;lake\n4;4;0;0;land\n";
    let mut bitmap = vec![0; 54];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[2..6].copy_from_slice(&70_u32.to_le_bytes());
    bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
    bitmap[22..26].copy_from_slice(&2_i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bitmap.extend_from_slice(&[0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 3, 0, 0, 4, 0, 0]);
    saved::RailwayAdjacency::parse(definitions, &bitmap, None).unwrap()
}

fn rail_save() -> saved::Pack {
    saved::parse(&content_reader_parser::parser::parse(b"rail_way={rail_way={1={rail_way={5 0 -7}} 2={rail_way={-1 2147483647 9}} 3={rail_way={0 8 0}} 4={rail_way={6 0 0}}}}").unwrap().items).unwrap()
}

fn rail_rows() -> Vec<RailEdgeRow> {
    [
        (1, 2, 5),
        (1, 4, -7),
        (2, 4, i32::MAX),
        (2, 3, 9),
        (3, 2, 8),
        (4, 1, 6),
    ]
    .into_iter()
    .map(|(from, to, level)| RailEdgeRow { from, to, level })
    .collect()
}

#[test]
fn selected_railway_levels_compare_every_signed_index_without_inferring_pair_count() {
    use replay_data::{rows::RailwaySelectionInputs, rows::Recorded};
    let saved = rail_save();
    let rows: Vec<_> = saved
        .railways
        .as_ref()
        .unwrap()
        .iter()
        .map(|(&province, info)| RailCooldownRow {
            province,
            cooldown_ticks: 0,
            selection_hour: Some(1).into(),
            selection: Some(RailwaySelectionInputs {
                levels: Some(info.levels.clone()),
                pair_count: Some(-1),
                building_level: None,
            })
            .into(),
            ..Default::default()
        })
        .collect();
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::RailCooldowns.index()] =
        TableView::Complete(TableRows::RailCooldowns(rows.clone()));
    *frame.tables[TableId::RailEdges.index()] = TableView::Unavailable {
        reason: "separate sorted-pair domain".into(),
    };
    let mut report = Report::default();
    compare_railways(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RailCooldowns.selection.levels"], 4);
    assert_eq!(report.scalar_checks, 12);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no pair count"))
    );
    for case in 0..6 {
        let mut changed = rows.clone();
        match case {
            0 => changed[0].selection = Recorded::NotRecorded,
            1 => changed[0].selection = None.into(),
            2 => changed[0].selection_hour = Some(0).into(),
            _ => {
                let packet = changed[0]
                    .selection
                    .require_mut("test")
                    .unwrap()
                    .as_mut()
                    .unwrap();
                match case {
                    3 => packet.levels = None,
                    4 => packet.levels.as_mut().unwrap()[1] = -1,
                    _ => packet.pair_count = None,
                }
            }
        }
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(changed));
        let mut report = Report::default();
        compare_railways(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            usize::from(case != 0),
            "case {case}: {:?}",
            report.differences
        );
    }
}

#[test]
fn railway_edges_compare_exact_directed_levels_in_content_neighbor_order() {
    let adjacency = rail_adjacency();
    let saved = rail_save();
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::RailEdges.index()] =
        TableView::Complete(TableRows::RailEdges(rail_rows()));
    let mut report = Report::default();
    compare_rail_edges(&frame, &saved, &adjacency, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["RailEdges"], 6);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("nonpositive railway levels"))
    );

    let mut rows = rail_rows();
    rows[2].level = -8;
    rows.remove(3);
    rows.push(RailEdgeRow {
        from: 1,
        to: 3,
        level: 1,
    });
    rows.push(RailEdgeRow {
        from: 5,
        to: 1,
        level: 99,
    });
    *frame.tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(rows));
    let mut report = Report::default();
    compare_rail_edges(&frame, &saved, &adjacency, &mut report).unwrap();
    assert_eq!(report.differences.len(), 4);
}

#[test]
fn railway_edge_mapping_errors_and_duplicate_keys_fail() {
    let adjacency = rail_adjacency();
    let mut saved = rail_save();
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::RailEdges.index()] =
        TableView::Complete(TableRows::RailEdges(rail_rows()));
    saved
        .railways
        .as_mut()
        .unwrap()
        .get_mut(&2)
        .unwrap()
        .levels
        .pop();
    assert!(
        compare_rail_edges(&frame, &saved, &adjacency, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("saved levels")
    );
    let mut rows = rail_rows();
    rows.push(rows[0].clone());
    *frame.tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(rows));
    assert!(
        compare_rail_edges(&frame, &rail_save(), &adjacency, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn theatre_oracle_checks_typed_parent_handles_priority_and_membership_order() {
    let saved = saved::parse(&content_reader_parser::parser::parse(br#"countries={GER={
        theatres={theatre={id={id=4294967295 type=67} theater_group={id={id=0 type=68} priority=-1 name="group" orders_group={id=4294967295 type=53}}}}
        air_theatre={id={id=4294967295 type=77} country=GER name="theatre"
            air_group={id={id=0 type=78} air_theatre={id=4294967295 type=77} name="air" air_wing={{id=4294967295 type=69}}}}
    }}"#).unwrap().items).unwrap();
    let land = (u64::from(u32::MAX) << 32) | 67;
    let air = (u64::from(u32::MAX) << 32) | 77;
    let orders = (u64::from(u32::MAX) << 32) | 53;
    let wing = (u64::from(u32::MAX) << 32) | 69;
    let country = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(123, &country)]);
    let mut frame = supremacy_frame(Vec::new());
    *frame.tables[TableId::LandTheatres.index()] =
        TableView::Complete(TableRows::LandTheatres(vec![LandTheatreRow {
            id: land,
            owner: 123,
            volunteers_theatre: 0,
            ordinal: 0,
        }]));
    *frame.tables[TableId::LandTheatreGroups.index()] =
        TableView::Complete(TableRows::LandTheatreGroups(vec![LandTheatreGroupRow {
            id: 68,
            theatre: land,
            ordinal: 0,
            priority: -1,
            name: b"group\0tail".to_vec(),
        }]));
    *frame.tables[TableId::LandTheatreMembers.index()] =
        TableView::Complete(TableRows::LandTheatreMembers(vec![LandTheatreMemberRow {
            orders_group: orders,
            theater_group: 68,
            ordinal: 0,
        }]));
    *frame.tables[TableId::AirTheatres.index()] =
        TableView::Complete(TableRows::AirTheatres(vec![AirTheatreRow {
            id: air,
            owner: 123,
            ordinal: 0,
            name: b"theatre\0tail".to_vec(),
        }]));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
            id: 78,
            theatre: air,
            ordinal: 0,
            name: b"air\0tail".to_vec(),
        }]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
            wing,
            group: 78,
            ordinal: 0,
        }]));
    let mut report = Report::default();
    compare_theatres(&frame, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    for table in [
        "LandTheatres",
        "LandTheatreGroups",
        "LandTheatreMembers",
        "AirTheatres",
        "AirTheatreGroups",
        "AirTheatreMembers",
    ] {
        assert_eq!(report.compared_tables[table], 1);
    }
    let TableView::Complete(TableRows::LandTheatreGroups(groups)) =
        &mut *frame.tables[TableId::LandTheatreGroups.index()]
    else {
        unreachable!()
    };
    groups[0].priority = 0;
    groups[0].theatre ^= 1;
    let TableView::Complete(TableRows::AirTheatreMembers(members)) =
        &mut *frame.tables[TableId::AirTheatreMembers.index()]
    else {
        unreachable!()
    };
    members[0].ordinal = 1;
    compare_theatres(&frame, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3);
    *frame.tables[TableId::AirTheatreMembers.index()] = TableView::Unavailable {
        reason: "reverse registry incomplete".into(),
    };
    assert!(compare_theatres(&frame, &saved, &links, &mut report).is_err());
}

#[test]
fn supremacy_uses_runtime_slot_and_compares_every_fixed5_unit() {
    let text = content_reader_parser::parser::parse(br#"region={1={dominance={values={
        { --- {current=90071992547.40993 base_target=-92233720368547.75808 target=92233720368547.75807 individual_ratio=-0.00001 previous=0 decline_from=1.00001} }
    }}}}"#).unwrap();
    let saved = saved::parse(&text.items).unwrap();
    let row = NavalSupremacyRow {
        region: 0x8765_4321,
        region_slot: 1,
        country: 0,
        current: 9_007_199_254_740_993,
        base_target: i64::MIN,
        target: i64::MAX,
        individual_ratio: -1,
        previous: 0,
        decline_from: 100_001,
    };
    let mut report = Report::default();
    compare_supremacy(
        &supremacy_frame(vec![row.clone()]),
        &saved,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    let mut changed = row.clone();
    changed.current -= 1;
    changed.base_target += 1;
    changed.target -= 1;
    changed.individual_ratio += 1;
    changed.previous += 1;
    changed.decline_from -= 1;
    compare_supremacy(
        &supremacy_frame(vec![changed]),
        &saved,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 6);
    let mut wrong_slot = row.clone();
    wrong_slot.region_slot = 2;
    let mut report = Report::default();
    compare_supremacy(
        &supremacy_frame(vec![wrong_slot]),
        &saved,
        &BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        compare_supremacy(
            &supremacy_frame(vec![row.clone(), row]),
            &saved,
            &BTreeMap::new(),
            &mut report
        )
        .is_err()
    );
    assert!(
        compare_supremacy(
            &supremacy_frame(Vec::new()),
            &saved::Pack::default(),
            &BTreeMap::new(),
            &mut report
        )
        .is_err()
    );
    let mut unavailable = supremacy_frame(Vec::new());
    *unavailable.tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
        reason: "source read failed".into(),
    };
    assert!(compare_supremacy(&unavailable, &saved, &BTreeMap::new(), &mut report).is_err());
}

#[test]
fn supremacy_country_aliases_keep_tuple_multiplicity_and_missing_values_are_empty() {
    let text = content_reader_parser::parser::parse(br#"region={1={dominance={values={
        { GER {current=0.00001 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
        { GER {current=0.00002 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
    }}}}"#).unwrap();
    let saved = saved::parse(&text.items).unwrap();
    let country = CountryRow {
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(1, &country), (2, &country)]);
    let one = NavalSupremacyRow {
        region: 99,
        region_slot: 1,
        country: 1,
        current: 2,
        ..Default::default()
    };
    let two = NavalSupremacyRow {
        region: 99,
        region_slot: 1,
        country: 2,
        current: 1,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_supremacy(
        &supremacy_frame(vec![one.clone(), two]),
        &saved,
        &links,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("multiplicities"))
    );
    compare_supremacy(
        &supremacy_frame(vec![one.clone()]),
        &saved,
        &links,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    let empty = content_reader_parser::parser::parse(b"region={1={dominance={}}}").unwrap();
    let empty = saved::parse(&empty.items).unwrap();
    let mut report = Report::default();
    compare_supremacy(&supremacy_frame(vec![one]), &empty, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn retained_result_census_checks_full_handles_and_child_order() {
    let date = GameDate::parse(b"1939.1.1.1").unwrap();
    let id = (u64::from(u32::MAX) << 32) | 61;
    let member = (u64::from(u32::MAX) << 32) | 69;
    let saved = saved::Pack {
        results: BTreeMap::from([(
            id,
            saved::NavalResult {
                id,
                province: u32::MAX,
                result_date: date,
                retire_date: date,
                category: i32::MIN as u32,
                port_strike: true,
                naval_strike: false,
                sides: [
                    saved::ResultSide {
                        details: Default::default(),
                        members: vec![saved::ResultMember {
                            details: Default::default(),
                            id: member,
                            sunk_by: Some(vec![0xff, b'a']),
                        }],
                        countries: vec![b"---".to_vec()],
                    },
                    saved::ResultSide::default(),
                ],
                admitted: Vec::new(),
            },
        )]),
        ..Default::default()
    };
    let mut frame = DecodedFrame {
        branch: 1,
        hour: date.engine_hour().unwrap(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::NavalResults.index()] =
        TableView::Complete(TableRows::NavalResults(vec![NavalResultRow {
            id,
            province: u32::MAX,
            result_hour: frame.hour,
            retire_hour: frame.hour,
            category: i32::MIN as u32,
            flag_a: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::NavalResultMembers.index()] =
        TableView::Complete(TableRows::NavalResultMembers(vec![NavalResultMemberRow {
            result: id,
            member,
            presence_value: 2,
            ..Default::default()
        }]));
    *frame.tables[TableId::NavalResultCountries.index()] =
        TableView::Complete(TableRows::NavalResultCountries(vec![
            NavalResultCountryRow {
                result: id,
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    compare_results(&frame, &saved, &mut report).unwrap();
    compare_members(&frame, &saved, &mut report).unwrap();
    compare_countries(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty());
    let TableView::Complete(TableRows::NavalResultMembers(rows)) =
        &mut *frame.tables[TableId::NavalResultMembers.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.ordinal = 1;
    compare_members(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    let TableView::Complete(TableRows::NavalResultMembers(rows)) =
        &mut *frame.tables[TableId::NavalResultMembers.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.member ^= 1;
    compare_members(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3);
    *frame.tables[TableId::NavalResultCountries.index()] = TableView::Unavailable {
        reason: "native read failed".into(),
    };
    assert!(compare_countries(&frame, &saved, &BTreeMap::new(), &mut report).is_err());
}

#[test]
fn anonymous_ship_fields_compare_exactly_and_omissions_remain_limits() {
    let expected = saved::SunkShip {
        ship_name: Some(vec![0xff]),
        attribution_name: None,
        victim_country: b"---".to_vec(),
        attacker_country: b"---".to_vec(),
        sink_date: GameDate::parse(b"1939.1.1.1").unwrap(),
        class: b"destroyer".to_vec(),
        attribution_class: b"none".to_vec(),
        model_level: i32::MIN,
        victim_equipment_variant: Some((u64::from(u32::MAX) << 32) | 70),
        damage_source_reference: None,
        province: None,
        battle: 0,
        cause_record: false,
        assisted_kill: false,
    };
    let mut actual = SunkShipEntry {
        ship_name: vec![0xff],
        attribution_name: b"retained".to_vec(),
        sink_hour: expected.sink_date.engine_hour().unwrap(),
        class_token: 1,
        attribution_class_token: 2,
        attribution_class_present: 1,
        model_level: i32::MIN,
        victim_equipment_variant: expected.victim_equipment_variant.unwrap(),
        damage_source_reference: u64::MAX,
        ..Default::default()
    };
    let tokens = BTreeMap::from([(1, b"destroyer".as_slice()), (2, b"none".as_slice())]);
    let mut report = Report::default();
    compare_ship(
        &actual,
        &expected,
        "ship",
        &BTreeMap::new(),
        &tokens,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 2);
    actual.victim_equipment_variant ^= 1;
    actual.model_level += 1;
    actual.province_present = 1;
    compare_ship(
        &actual,
        &expected,
        "ship",
        &BTreeMap::new(),
        &tokens,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 3);
    let incomplete_tokens = BTreeMap::from([(1, b"destroyer".as_slice())]);
    assert!(
        compare_ship(
            &actual,
            &expected,
            "ship",
            &BTreeMap::new(),
            &incomplete_tokens,
            &mut report
        )
        .is_err()
    );
    let mut unavailable_report = Report::default();
    compare_ship(
        &actual,
        &expected,
        "ship",
        &BTreeMap::new(),
        &BTreeMap::new(),
        &mut unavailable_report,
    )
    .unwrap();
    assert!(
        unavailable_report
            .limitations
            .keys()
            .any(|reason| reason.contains("lexical tokens are unavailable"))
    );
}

#[test]
fn sunk_ship_lexical_classes_keep_nul_limits_and_explicit_pointer_presence() {
    let saved = saved::parse(
        &content_reader_parser::parser::parse(
            br#"history={sunk_ship={
        name="visible" killer_name="" country=--- killer_country=--- level=0
        definition=destroyer killer_definition=none date=1939.1.1.1
        battle={id=0 type=0} convoy=no
    }}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let expected = &saved.sunk_ships[0];
    let mut actual = SunkShipEntry {
        ship_name: b"visible\0\xffretained".to_vec(),
        sink_hour: expected.sink_date.engine_hour().unwrap(),
        class_token: 1,
        attribution_class_token: 2,
        attribution_class_present: 1,
        class_name: b"different captured name".to_vec(),
        attribution_class_name: Vec::new(),
        ..Default::default()
    };
    let tokens = BTreeMap::from([
        (1, b"destroyer\0\xffsuffix".as_slice()),
        (2, b"none\0suffix".as_slice()),
    ]);
    let mut report = Report::default();
    compare_ship(
        &actual,
        expected,
        "ship",
        &BTreeMap::new(),
        &tokens,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("first NUL"))
            .unwrap()
            .1
            .len(),
        1
    );
    assert_eq!(
        report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("unproved text-stream termination"))
            .unwrap()
            .1
            .len(),
        2
    );

    let wrong = BTreeMap::from([(1, b"submarine".as_slice()), (2, b"none".as_slice())]);
    let mut report = Report::default();
    compare_ship(
        &actual,
        expected,
        "ship",
        &BTreeMap::new(),
        &wrong,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("ship.definition"));
    assert!(
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &BTreeMap::from([(1, b"destroyer".as_slice())]),
            &mut Report::default()
        )
        .is_err()
    );

    actual.attribution_class_present = 0;
    actual.attribution_class_token = 0;
    let mut report = Report::default();
    compare_ship(
        &actual,
        expected,
        "ship",
        &BTreeMap::new(),
        &tokens,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("ship.killer_definition"));
    actual.attribution_class_present = 2;
    assert!(
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &tokens,
            &mut Report::default()
        )
        .is_err()
    );
    actual.attribution_class_present = 1;
    actual.attribution_class_token = 2;
    actual.class_token = 0x167;
    let reserved = BTreeMap::from([(0x167, b"destroyer".as_slice()), (2, b"none".as_slice())]);
    assert!(
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &reserved,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn sunk_convoy_count_uses_signed_native_integer_without_scaling() {
    let expected = saved::SunkConvoy {
        month: i32::MIN,
        killer_country: b"---".to_vec(),
        owner_country: b"---".to_vec(),
        convoys: i32::MAX,
    };
    let mut actual = SunkConvoyRow {
        month: i32::MIN,
        convoys: i32::MAX,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_convoy(&actual, &expected, "convoy", &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty());
    actual.convoys -= 1;
    compare_convoy(&actual, &expected, "convoy", &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(flag(128).is_err());
}
