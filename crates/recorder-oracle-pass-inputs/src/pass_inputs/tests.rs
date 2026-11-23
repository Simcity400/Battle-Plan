use super::*;
use content_reader_save::counter_inputs;
use replay_data::{
    rows::CountryCounterInputRow, rows::FleetCounterInputRow, rows::FleetRow,
    rows::FreshTaskForceCounterEta, rows::NavalPortMissionInputs, rows::Recorded, rows::ShipRow,
    rows::TaskForceCounterInputRow, rows::TaskForceMapMotionRow, rows::TaskForceMissionRow,
    rows::TaskForceRow, rows::UnitSupplyInputs,
};

fn known_supply() -> UnitSupplyInputs {
    UnitSupplyInputs {
        origins: Some(Vec::new()),
        ..Default::default()
    }
}

fn known_port() -> NavalPortMissionInputs {
    NavalPortMissionInputs {
        sample_hour: replay_data::rows::PacketHour::Host,
        searching: Some(Vec::new()),
        engagement_target: None,
        avoided_ship_types: None,
    }
}

#[test]
fn port_suffix_oracle_compares_order_raw_dangling_targets_and_each_known_slot() {
    use replay_data::{
        rows::NavalEngagementTarget, rows::NavalPortMissionInputs, rows::NavalSearcher,
    };
    let target = (u64::from(u32::MAX) << 32) | 51;
    let mut row = TaskForceCounterInputRow {
        task_force: 9,
        port_mission: Recorded::Value(Some(NavalPortMissionInputs {
            sample_hour: replay_data::rows::PacketHour::Host,
            searching: Some(vec![
                NavalSearcher {
                    reference: Some(0),
                    resolves: Some(false),
                    roster_owner: None,
                },
                NavalSearcher {
                    reference: Some(99),
                    resolves: Some(true),
                    roster_owner: Some(0),
                },
            ]),
            engagement_target: Some(NavalEngagementTarget {
                reference: target,
                resolves: Some(false),
            }),
            avoided_ship_types: Some(vec![
                Some(Vec::new()),
                Some(b"type".to_vec()),
                Some(b"type".to_vec()),
            ]),
        })),
        ..Default::default()
    };
    let force = TaskForceRow {
        id: 9,
        mission: 2,
        ..Default::default()
    };
    let saved = counter_inputs::TaskForce {
        spotters: vec![0, 99],
        strike_force_target: Some(target),
        target_ship_types: vec![Vec::new(), b"type".to_vec(), b"type".to_vec()],
        ..Default::default()
    };
    let mut report = Report::default();
    task_force_port(7, &row, &force, &saved, &mut report);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let Recorded::Value(Some(packet)) = &mut row.port_mission else {
        unreachable!()
    };
    packet.searching.as_mut().unwrap().swap(0, 1);
    packet.avoided_ship_types.as_mut().unwrap()[1] = None;
    packet.avoided_ship_types.as_mut().unwrap()[2] = Some(b"changed".to_vec());
    let mut report = Report::default();
    task_force_port(7, &row, &force, &saved, &mut report);
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("avoided_ship_types[2]"))
    );
    let mut missing_target = saved.clone();
    missing_target.strike_force_target = None;
    let mut report = Report::default();
    task_force_port(7, &row, &force, &missing_target, &mut report);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("engagement_target"))
    );
}

#[test]
fn port_saved_strings_compare_first_nul_prefix_and_keep_ordered_empty_slots() {
    for stored in [b"type\0suffix".as_slice(), b"\0suffix".as_slice()] {
        let row = TaskForceCounterInputRow {
            task_force: 9,
            port_mission: Recorded::Value(Some(NavalPortMissionInputs {
                avoided_ship_types: Some(vec![Some(stored.to_vec()), Some(Vec::new())]),
                ..known_port()
            })),
            ..Default::default()
        };
        let force = TaskForceRow {
            id: 9,
            ..Default::default()
        };
        let mut saved = counter_inputs::TaskForce {
            target_ship_types: vec![
                content_reader_parser::parser::c_string(stored).to_vec(),
                Vec::new(),
            ],
            ..Default::default()
        };
        let mut report = Report::default();
        task_force_port(7, &row, &force, &saved, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|field| field.contains("avoided_ship_types[0]"))
        );
        saved.target_ship_types[0] = b"different".to_vec();
        let mut report = Report::default();
        task_force_port(7, &row, &force, &saved, &mut report);
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains("avoided_ship_types[0]"))
        );
        saved.target_ship_types.clear();
        let mut report = Report::default();
        task_force_port(7, &row, &force, &saved, &mut report);
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains("avoided_ship_types.len"))
        );
    }
}

#[test]
fn port_suffix_oracle_keeps_historical_fault_and_unreached_domains_distinct() {
    use replay_data::{rows::NavalEngagementTarget, rows::NavalPortMissionInputs};
    let force = TaskForceRow {
        id: 9,
        mission: 2,
        ..Default::default()
    };
    let saved = counter_inputs::TaskForce::default();
    let mut row = TaskForceCounterInputRow {
        task_force: 9,
        port_mission: Recorded::NotRecorded,
        ..Default::default()
    };
    let mut old = Report::default();
    task_force_port(7, &row, &force, &saved, &mut old);
    assert!(old.differences.is_empty());
    assert!(
        old.limitations
            .contains_key("TaskForceCounterInputs.port_mission not recorded")
    );
    row.port_mission = Recorded::Value(None);
    let mut refused = Report::default();
    task_force_port(7, &row, &force, &saved, &mut refused);
    assert_eq!(refused.differences.len(), 1);
    let packet = NavalPortMissionInputs {
        sample_hour: replay_data::rows::PacketHour::Host,
        searching: Some(Vec::new()),
        engagement_target: None,
        avoided_ship_types: None,
    };
    row.port_mission = Recorded::Value(Some(packet.clone()));
    for (mission, combats, detached) in [(0, 0, 0), (2, 1, 0), (2, 0, 1)] {
        let force = TaskForceRow {
            mission,
            combat_references: combats,
            ..force.clone()
        };
        row.detached_activity = detached;
        let mut report = Report::default();
        task_force_port(7, &row, &force, &saved, &mut report);
        assert!(report.differences.is_empty());
    }
    row.detached_activity = 0;
    let mut reached = Report::default();
    task_force_port(7, &row, &force, &saved, &mut reached);
    assert_eq!(reached.differences.len(), 2);
    for (rule, reference, resolves) in [(4, 0, false), (0, 99, true)] {
        row.engagement_rule = rule;
        row.port_mission = Recorded::Value(Some(NavalPortMissionInputs {
            engagement_target: Some(NavalEngagementTarget {
                reference,
                resolves: Some(resolves),
            }),
            ..packet.clone()
        }));
        let saved = counter_inputs::TaskForce {
            strike_force_target: (reference != 0).then_some(reference),
            ..saved.clone()
        };
        let mut report = Report::default();
        task_force_port(7, &row, &force, &saved, &mut report);
        assert!(report.differences.is_empty());
    }
}

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

fn base() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 101,
            owned_state_count: 1,
            controlled_state_count: 1,
            ..Default::default()
        }]),
    );
    save.counter_inputs.prides.insert(b"GER".to_vec(), None);
    (frame, save)
}

#[test]
fn hq_distance_oracle_compares_zero_positive_and_negative_save_omission() {
    let (mut frame, mut save) = base();
    let group = (7 << 32) | 53;
    set(
        &mut frame,
        TableRows::OrdersGroups(vec![replay_data::rows::OrdersGroupRow {
            handle: group,
            ..Default::default()
        }]),
    );
    for (actual, saved, differences) in [
        (i32::MIN, None, 0),
        (-7, None, 0),
        (-1, None, 0),
        (0, Some(0), 0),
        (i32::MAX, Some(i32::MAX), 0),
        (0, None, 1),
        (-1, Some(0), 1),
        (7, Some(8), 1),
    ] {
        let suffix = saved
            .map(|value| format!("distance={value}"))
            .unwrap_or_default();
        let text = format!(
            "countries={{GER={{theatres={{theatre={{id={{id=2 type=67}} orders_group={{id={{id=7 type=53}} name=Army field_marshal_group=no motorization_level=0 {suffix}}}}}}}}}}}"
        );
        save.naval_history = content_reader_save::naval_history::parse(
            &content_reader_parser::parser::parse(text.as_bytes())
                .unwrap()
                .items,
        )
        .unwrap();
        set(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                replay_data::rows::OrdersGroupCounterInputRow {
                    template_context: Recorded::Value(Some(
                        std::sync::Arc::new(Default::default()),
                    )),
                    hq_nearest_front: Recorded::Value(Some(-1)),
                    group,
                    withdrawing: 0,
                    hq_distance: Recorded::Value(Some(actual)),
                    ..Default::default()
                },
            ]),
        );
        let mut report = Report::default();
        groups(&frame, &save, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{actual} {saved:?}: {:?}",
            report.differences
        );
    }
    for (distance, expected_reason) in [
        (
            Recorded::NotRecorded,
            "OrdersGroupCounterInputs.hq_distance not recorded",
        ),
        (Recorded::Value(None), "HQ distance unreadable at capture"),
    ] {
        set(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                replay_data::rows::OrdersGroupCounterInputRow {
                    template_context: Recorded::Value(Some(
                        std::sync::Arc::new(Default::default()),
                    )),
                    hq_nearest_front: Recorded::Value(Some(-1)),
                    group,
                    withdrawing: 0xff,
                    hq_distance: distance,
                    ..Default::default()
                },
            ]),
        );
        let mut report = Report::default();
        groups(&frame, &save, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            1,
            "withdrawing remains independently compared"
        );
        assert!(report.limitations.contains_key(expected_reason));
    }
}

#[test]
fn template_hq_context_oracle_compares_target_deployment_pools_flags_and_refusals() {
    use replay_data::rows::{
        DivisionTemplateRow, HqDeployment, HqEquipment, HqRequirement, LexicalTokenRow,
        TemplateHqContext,
    };
    let (mut frame, mut save) = base();
    let group = (7 << 32) | 53;
    let target = (19 << 32) | 52;
    let variant = (29 << 32) | 70;
    let text=br#"countries={GER={theatres={theatre={id={id=2 type=67}
      orders_group={id={id=7 type=53} field_marshal_group=no motorization_level=0
        target_template={id=19 type=52} deployed=yes deploy_queued=yes
        hq_deploy_distributable={country=GER priority=9
          hq_assembled_equipment={allow_zero_entries=no equipment={id={id=29 type=70} amount=1.23456}}
          hq_requested_equipment={mod_hq_equipment=1.5}
          hq_assembled_manpower=-7 hq_requested_manpower=19 hq_deploy_order=31 hq_requisitioned_from_army=yes
        }
      }
    }}}}"#;
    save.naval_history =
        content_reader_save::naval_history::parse(&content_reader_parser::parser::parse(text).unwrap().items)
            .unwrap();
    set(
        &mut frame,
        TableRows::OrdersGroups(vec![replay_data::rows::OrdersGroupRow {
            handle: group,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: target,
            owner: 101,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"mod_hq_equipment".to_vec(),
        }]),
    );
    let context = TemplateHqContext {
        target,
        deployed: 2,
        queued: 3,
        deployment: Some(HqDeployment {
            country: 102,
            priority: 9,
            allow_zero: 0,
            assembled_manpower: -7,
            requested_manpower: 19,
            order: 31,
            requisitioned: 4,
        }),
        equipment: vec![
            HqEquipment { variant, amount: 0 },
            HqEquipment {
                variant,
                amount: 123456,
            },
        ],
        required: vec![
            HqRequirement {
                token: 77,
                amount: 0,
            },
            HqRequirement {
                token: 77,
                amount: 150000,
            },
        ],
    };
    let mut row = replay_data::rows::OrdersGroupCounterInputRow {
        group,
        template_context: Recorded::Value(Some(std::sync::Arc::new(context))),
        hq_nearest_front: Recorded::Value(Some(-1)),
        hq_distance: Recorded::Value(Some(-1)),
        invasion_source_distance: Recorded::Value(Some(-1)),
        ..Default::default()
    };
    set(
        &mut frame,
        TableRows::OrdersGroupCounterInputs(vec![row.clone()]),
    );
    let mut report = Report::default();
    groups(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    if let Recorded::Value(Some(context)) = &mut row.template_context {
        std::sync::Arc::make_mut(context).equipment[1].amount += 1;
    }
    set(
        &mut frame,
        TableRows::OrdersGroupCounterInputs(vec![row.clone()]),
    );
    let mut report = Report::default();
    groups(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    row.template_context = Recorded::Value(None);
    set(&mut frame, TableRows::OrdersGroupCounterInputs(vec![row]));
    assert!(
        groups(&frame, &save, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("context acquisition refused")
    );
}

#[test]
fn template_hq_nearest_front_oracle_compares_nonnegative_words_and_negative_omission() {
    let (mut frame, mut save) = base();
    let group = (7 << 32) | 53;
    set(
        &mut frame,
        TableRows::OrdersGroups(vec![replay_data::rows::OrdersGroupRow {
            handle: group,
            ..Default::default()
        }]),
    );
    for (actual, saved, differences) in [
        (i32::MIN, None, 0),
        (-1, None, 0),
        (0, Some(0), 0),
        (19, Some(19), 0),
        (19, Some(20), 1),
    ] {
        let suffix = saved
            .map(|value| format!("hq_nearest_front_province_id={value}"))
            .unwrap_or_default();
        let text = format!(
            "countries={{GER={{theatres={{theatre={{id={{id=2 type=67}} orders_group={{id={{id=7 type=53}} field_marshal_group=no motorization_level=0 {suffix}}}}}}}}}}}"
        );
        save.naval_history = content_reader_save::naval_history::parse(
            &content_reader_parser::parser::parse(text.as_bytes())
                .unwrap()
                .items,
        )
        .unwrap();
        set(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                replay_data::rows::OrdersGroupCounterInputRow {
                    template_context: Recorded::Value(Some(
                        std::sync::Arc::new(Default::default()),
                    )),
                    group,
                    hq_nearest_front: Recorded::Value(Some(actual)),
                    hq_distance: Recorded::Value(Some(-1)),
                    invasion_source_distance: Recorded::Value(Some(-1)),
                    ..Default::default()
                },
            ]),
        );
        let mut report = Report::default();
        groups(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), differences);
    }
}

#[test]
fn invasion_source_distance_oracle_preserves_negative_omission_and_hq_comparison() {
    let (mut frame, mut save) = base();
    let group = (7 << 32) | 53;
    set(
        &mut frame,
        TableRows::OrdersGroups(vec![replay_data::rows::OrdersGroupRow {
            handle: group,
            ..Default::default()
        }]),
    );
    for (actual, saved, differences) in [
        (i32::MIN, None, 0),
        (-7, None, 0),
        (-1, None, 0),
        (0, Some(0), 0),
        (i32::MAX, Some(i32::MAX), 0),
        (0, None, 1),
        (-1, Some(0), 1),
        (7, Some(8), 1),
    ] {
        let suffix = saved
            .map(|value| format!("hq_distance_to_naval_invasion_source={value}"))
            .unwrap_or_default();
        let text = format!(
            "countries={{GER={{theatres={{theatre={{id={{id=2 type=67}} orders_group={{id={{id=7 type=53}} name=Army field_marshal_group=no motorization_level=0 distance=31 {suffix}}}}}}}}}}}"
        );
        save.naval_history = content_reader_save::naval_history::parse(
            &content_reader_parser::parser::parse(text.as_bytes())
                .unwrap()
                .items,
        )
        .unwrap();
        set(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                replay_data::rows::OrdersGroupCounterInputRow {
                    template_context: Recorded::Value(Some(
                        std::sync::Arc::new(Default::default()),
                    )),
                    hq_nearest_front: Recorded::Value(Some(-1)),
                    group,
                    hq_distance: Recorded::Value(Some(31)),
                    invasion_source_distance: Recorded::Value(Some(actual)),
                    ..Default::default()
                },
            ]),
        );
        let mut report = Report::default();
        groups(&frame, &save, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{actual} {saved:?}: {:?}",
            report.differences
        );
    }
    for (distance, expected_reason) in [
        (
            Recorded::NotRecorded,
            "OrdersGroupCounterInputs.invasion_source_distance not recorded",
        ),
        (
            Recorded::Value(None),
            "invasion-source distance unreadable at capture",
        ),
    ] {
        set(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                replay_data::rows::OrdersGroupCounterInputRow {
                    template_context: Recorded::Value(Some(
                        std::sync::Arc::new(Default::default()),
                    )),
                    hq_nearest_front: Recorded::Value(Some(-1)),
                    group,
                    hq_distance: Recorded::Value(Some(32)),
                    invasion_source_distance: distance,
                    ..Default::default()
                },
            ]),
        );
        let mut report = Report::default();
        groups(&frame, &save, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            1,
            "known HQ distance is still compared"
        );
        assert!(report.limitations.contains_key(expected_reason));
    }
}

#[test]
fn historical_roster_fields_keep_existing_country_counter_comparisons() {
    let (mut frame, save) = base();
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 101,
            owned_state_count: 1,
            controlled_state_count: 1,
            owned_states: Recorded::NotRecorded,
            controlled_states: Recorded::NotRecorded,
            core_states: Recorded::NotRecorded,
            army_leaders: Recorded::NotRecorded,
            navy_leaders: Recorded::NotRecorded,
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    countries(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 5);
    for name in [
        "owned_states",
        "controlled_states",
        "core_states",
        "army_leaders",
        "navy_leaders",
    ] {
        assert_eq!(
            report.limitations[&format!("CountryCounterInputs.{name} not recorded")].len(),
            1,
        );
    }
}

#[test]
fn first_division_matches_the_first_saved_division() {
    let (mut frame, mut save) = base();
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 101,
            owned_state_count: 1,
            controlled_state_count: 1,
            first_division: (7 << 32) | 4,
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    countries(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("first_division"));
    save.counter_inputs
        .first_divisions
        .insert(b"GER".to_vec(), (7 << 32) | 4);
    let mut report = Report::default();
    countries(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn controlled_state_count_follows_the_saved_controller_or_owner() {
    let (frame, mut save) = base();
    let rows = |report: &Report| {
        report
            .differences
            .iter()
            .filter(|d| d.contains("controlled_state_count"))
            .count()
    };
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert_eq!(rows(&report), 0, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("controlled_state_count"))
    );
    let state = save.states.values_mut().next().unwrap();
    state.controller = Some(b"ENG".to_vec());
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert_eq!(rows(&report), 1, "{:?}", report.differences);
    assert!(
        !report
            .differences
            .iter()
            .any(|d| d.contains("owned_state_count"))
    );
}

#[test]
fn naval_country_and_supply_membership_inputs_compare_with_saved_rules() {
    let (mut frame, mut save) = base();
    let fleet = (1_u64 << 32) | 61;
    let task = (43_u64 << 32) | 51;
    let pride = (32_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::Fleets(vec![FleetRow {
            id: fleet,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: task,
            fleet,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
            fleet,
            mission_region_count: 3,
            mission_regions: (Some(std::sync::Arc::new(vec![206, 206, 207]))).into(),
            task_forces: vec![task],
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: task,
            movement_numerator: Recorded::Value(Some(0)),
            retreat: Recorded::Value(Some(0)),
            withdrawal: Recorded::Value(Some(0)),
            naval_transfer: Recorded::Value(Some(0)),
            combat_links: Recorded::Value(Some([0; 2])),
            fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                sample_hour: replay_data::rows::PacketHour::Host,
                base_speed: Some(0),
                speed_modifier: Some(0),
                terrain_class: Some(0),
                modified_range: Some(0),
                exiled: Some(0),
                strategic_redeployment: Some(0),
            })),
            ..Default::default()
        }]),
    );
    save.task_forces.insert(
        task,
        content_reader_save::TaskForce {
            id: task,
            name: None,
            fleet,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: Some(1),
            mission: 0,
            country_intel: Vec::new(),
            path: Vec::new(),
            target: None,
            alternate_path: Vec::new(),
            movement_progress: None,
            retreat: false,
            withdrawal: false,
            exile: false,
        },
    );
    set(
        &mut frame,
        TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
            task_force: task,
            name: Recorded::Value(Some(Vec::new())),
            supply: Recorded::Value(Some(known_supply())),
            port_mission: Recorded::Value(Some(known_port())),
            is_in_regions: 255,
            engagement_rule: 2,
            bombardment_region: 206,
            mission_region_count: 5,
            repair_parent: 7,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMissions(vec![TaskForceMissionRow {
            task_force: task,
            owner: 101,
            current_region: 0,
            regions: vec![206, 207, 206, 208, 209],
        }]),
    );
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 101,
            pride_of_the_fleet: pride,
            owned_state_count: 1,
            controlled_state_count: 1,
            ..Default::default()
        }]),
    );
    save.counter_inputs = counter_inputs::Pack {
        sent_expeditionaries: Default::default(),
        sent_volunteers: Default::default(),
        first_divisions: Default::default(),
        division_orders: Default::default(),
        fleet_orders: [(b"GER".to_vec(), vec![fleet])].into(),
        prides: [(b"GER".to_vec(), Some(pride))].into(),
        fleets: [(
            fleet,
            counter_inputs::Fleet {
                owner: b"GER".to_vec(),
                color: Some(vec![219, 219, 10]),
                strategic_regions: Some(vec![206, 207, 206]),
                task_forces: vec![task],
            },
        )]
        .into(),
        task_forces: [(
            task,
            counter_inputs::TaskForce {
                fleet,
                is_in_regions: Some(true),
                engagement_rule: Some(2),
                bombardment_region: Some(206),
                spotting_region: None,
                accessible_regions: Some(vec![206, 207, 206, 208, 209]),
                repair_parent: None,
                detached_activity: None,
                repair_last_mission: None,
                attached_headquarters: 0,
                ..Default::default()
            },
        )]
        .into(),
        naval_transfers: Default::default(),
        naval_transfer_inputs: Default::default(),
    };
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("repair_parent only when"))
    );
    let saved = save.counter_inputs.fleets.get_mut(&fleet).unwrap();
    saved.task_forces.push(task + (1 << 32));
    saved.strategic_regions = None;
    save.counter_inputs
        .prides
        .insert(b"GER".to_vec(), Some(pride + 1));
    save.states.clear();
    let task_row = save.counter_inputs.task_forces.get_mut(&task).unwrap();
    task_row.detached_activity = Some(b"refitting".to_vec());
    task_row.repair_last_mission = Some(3);
    task_row.attached_headquarters = 1;
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert_eq!(report.differences.len(), 9, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains(".mission_regions"))
    );
    save.counter_inputs.prides.insert(b"GER".to_vec(), None);
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("pride_of_the_fleet (omitted)")),
        "{:?}",
        report.differences
    );
    let mut report = Report::default();
    save.economy.supply = Some(Default::default());
    super::super::supply_trains::compare(
        &frame,
        &save,
        &Err("membership not needed".to_string()),
        &mut report,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("supply-system membership")),
        "{:?}",
        report.differences
    );
}

fn serialized(task_force: &str, combats: &str) -> Save {
    content_reader_save::parse(format!(
        "HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ GER={{units={{fleet={{id={{id=1 type=61}} task_force={{id={{id=43 type=51}} {task_force} location=1 logical_country=\"GER\" mission={{mission=0}}}}}}}}}} }} states={{}} provinces={{}} combat={{{combats}}}"
    ).as_bytes())
    .unwrap()
}

#[test]
fn task_force_names_compare_cached_bytes_and_native_saved_presence() {
    let force = (43_u64 << 32) | 51;
    let run = |name: Recorded<Option<Vec<u8>>>, suffix: &str| {
        let (mut frame, _) = base();
        let mut saved = serialized(suffix, "");
        saved
            .counter_inputs
            .task_forces
            .get_mut(&force)
            .unwrap()
            .engagement_rule = Some(0);
        set(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: force,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: force,
                name,
                supply: Recorded::Value(Some(known_supply())),
                port_mission: Recorded::Value(Some(known_port())),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        task_forces(&frame, &saved, &mut report).unwrap();
        report
    };
    for (name, suffix, differences) in [
        (Some(Vec::new()), "", 0),
        (Some(Vec::new()), "name=\"\"", 1),
        (Some(b"Cached".to_vec()), "name=Cached", 0),
        (Some(b"Cached".to_vec()), "name=Different", 1),
        (Some(b"Cached".to_vec()), "", 1),
        (Some(b"Cached\0suffix".to_vec()), "name=Cached", 0),
        (Some(b"Cached\0suffix".to_vec()), "name=Different", 1),
        (Some(b"\0suffix".to_vec()), "name=\"\"", 0),
        (Some(b"\0suffix".to_vec()), "", 1),
        (None, "name=Cached", 1),
    ] {
        let has_suffix = name.as_ref().is_some_and(|name| name.contains(&0));
        let report = run(Recorded::Value(name), suffix);
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert!(report.differences.iter().all(|difference| {
            difference.contains(&format!("TaskForceCounterInputs {force}.name"))
        }));
        assert_eq!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("first NUL")),
            has_suffix,
        );
    }
    let historical = run(Recorded::NotRecorded, "name=Cached");
    assert!(historical.differences.is_empty());
    assert_eq!(
        historical.limitations["TaskForceCounterInputs.name not recorded"],
        vec![format!("TaskForceCounterInputs {force}.name")],
    );
    assert_eq!(historical.compared_tables["TaskForceCounterInputs"], 1);
}

#[test]
fn task_force_supply_reports_runtime_caches_and_counts_independent_refusals() {
    let force = (43_u64 << 32) | 51;
    let run = |supply: Recorded<Option<UnitSupplyInputs>>, whole_frame: bool| {
        let (mut frame, _) = base();
        let mut saved = serialized("", "");
        saved
            .counter_inputs
            .task_forces
            .get_mut(&force)
            .unwrap()
            .engagement_rule = Some(0);
        set(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: force,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: force,
                name: Some(Vec::new()).into(),
                supply,
                port_mission: Recorded::Value(Some(known_port())),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        if whole_frame {
            report.record_tables(&frame);
        }
        task_forces(&frame, &saved, &mut report).unwrap();
        report
    };
    let baseline = run(Some(known_supply()).into(), false);
    for disruption in [i64::MIN, 0, i64::MAX] {
        let report = run(
            Some(UnitSupplyInputs {
                asked: i64::MAX,
                received: -1,
                disruption,
                ..known_supply()
            })
            .into(),
            false,
        );
        assert!(report.differences.is_empty(), "{report:?}");
        assert_eq!(report.scalar_checks, baseline.scalar_checks);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no save path"))
        );
    }
    for (supply, field) in [
        (None, "supply"),
        (
            Some(UnitSupplyInputs {
                origins: None,
                ..known_supply()
            }),
            "supply.origins",
        ),
    ] {
        let report = run(supply.into(), false);
        assert_eq!(report.differences.len(), 1, "{report:?}");
        assert!(report.differences[0].contains(&format!("TaskForceCounterInputs {force}.{field}")));
        assert_eq!(report.scalar_checks, baseline.scalar_checks);
    }
    for whole_frame in [false, true] {
        let report = run(Recorded::NotRecorded, whole_frame);
        assert!(report.differences.is_empty(), "{report:?}");
        assert_eq!(
            report.limitations["TaskForceCounterInputs.supply not recorded"].len(),
            1
        );
        assert_eq!(report.scalar_checks, baseline.scalar_checks);
        assert_eq!(report.compared_tables["TaskForceCounterInputs"], 1);
    }
}

#[test]
fn pride_name_oracle_requires_a_present_full_ship_and_preserves_country_comparisons() {
    let pride = (32_u64 << 32) | 51;
    let run = |handle: u64,
               name: Recorded<Option<Vec<u8>>>,
               ships: Option<Vec<ShipRow>>,
               whole_frame: bool| {
        let (mut frame, mut saved) = base();
        saved
            .counter_inputs
            .prides
            .insert(b"GER".to_vec(), (handle != 0).then_some(handle));
        set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 101,
                pride_of_the_fleet: handle,
                owned_state_count: 1,
                controlled_state_count: 1,
                pride_name: name,
                ..Default::default()
            }]),
        );
        if let Some(ships) = ships {
            set(&mut frame, TableRows::Ships(ships));
        } else {
            *frame.tables[TableId::Ships.index()] = TableView::Unavailable {
                reason: "test refusal".into(),
            };
        }
        let mut report = Report::default();
        if whole_frame {
            report.record_tables(&frame);
        }
        countries(&frame, &saved, &mut report).unwrap();
        report
    };
    let ships = || {
        Some(vec![ShipRow {
            id: pride,
            ..Default::default()
        }])
    };
    let baseline = run(pride, Some(Vec::new()).into(), ships(), false);
    for name in [Vec::new(), vec![0xff, 0, b'N']] {
        let report = run(pride, Some(name).into(), ships(), false);
        assert!(report.differences.is_empty(), "{report:?}");
        assert_eq!(report.scalar_checks, baseline.scalar_checks);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("omits that cache"))
        );
    }
    let refused = run(pride, None.into(), ships(), false);
    assert_eq!(refused.differences.len(), 1, "{refused:?}");
    assert!(refused.differences[0].contains("CountryCounterInputs GER.pride_name"));
    assert_eq!(refused.scalar_checks, baseline.scalar_checks);
    for (handle, ships) in [
        (0, None),
        (pride, Some(Vec::new())),
        (
            pride,
            Some(vec![ShipRow {
                id: pride ^ 1,
                ..Default::default()
            }]),
        ),
    ] {
        let report = run(handle, None.into(), ships, false);
        assert!(report.differences.is_empty(), "{report:?}");
        assert!(
            !report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.ends_with(".pride_name"))
        );
        assert_eq!(report.compared_tables["CountryCounterInputs"], 1);
    }
    let unavailable = run(pride, None.into(), None, false);
    assert!(unavailable.differences.is_empty(), "{unavailable:?}");
    assert_eq!(unavailable.scalar_checks, baseline.scalar_checks);
    assert!(
        unavailable
            .limitations
            .keys()
            .any(|reason| reason.contains("complete Ships is required"))
    );
    for whole_frame in [false, true] {
        let historical = run(pride, Recorded::NotRecorded, ships(), whole_frame);
        assert!(historical.differences.is_empty(), "{historical:?}");
        assert_eq!(
            historical.limitations["CountryCounterInputs.pride_name not recorded"].len(),
            1
        );
        assert_eq!(historical.scalar_checks, baseline.scalar_checks);
    }
}

#[test]
fn task_force_names_join_the_full_saved_handle_and_refuse_a_missing_force() {
    let force = (43_u64 << 32) | 51;
    let other = (44_u64 << 32) | 51;
    let (mut frame, _) = base();
    let mut saved = serialized("name=Cached", "");
    let mut second = saved.task_forces[&force].clone();
    second.id = other;
    second.name = Some(b"Other".to_vec());
    saved.task_forces.insert(other, second);
    let mut operands = saved.counter_inputs.task_forces[&force].clone();
    operands.engagement_rule = Some(0);
    saved
        .counter_inputs
        .task_forces
        .insert(force, operands.clone());
    saved.counter_inputs.task_forces.insert(other, operands);
    set(
        &mut frame,
        TableRows::TaskForces(
            [force, other]
                .map(|id| TaskForceRow {
                    id,
                    ..Default::default()
                })
                .to_vec(),
        ),
    );
    let mut run = |names: [&[u8]; 2], saved: &Save| {
        set(
            &mut frame,
            TableRows::TaskForceCounterInputs(
                [force, other]
                    .into_iter()
                    .zip(names)
                    .map(|(task_force, name)| TaskForceCounterInputRow {
                        task_force,
                        name: Recorded::Value(Some(name.to_vec())),
                        supply: Recorded::Value(Some(known_supply())),
                        port_mission: Recorded::Value(Some(known_port())),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let mut report = Report::default();
        task_forces(&frame, saved, &mut report).unwrap();
        report
    };
    assert!(run([b"Cached", b"Other"], &saved).differences.is_empty());
    let swapped = run([b"Other", b"Cached"], &saved);
    assert_eq!(swapped.differences.len(), 2);
    for id in [force, other] {
        assert!(swapped.differences.iter().any(|difference| {
            difference.contains(&format!("TaskForceCounterInputs {id}.name"))
        }));
    }
    saved.task_forces.remove(&force);
    let missing = run([b"Cached", b"Other"], &saved);
    assert_eq!(missing.differences.len(), 1);
    assert!(missing.differences[0].contains(&format!("TaskForceCounterInputs {force}.name")));
    assert!(missing.differences[0].contains("absent from save"));
}

fn naval_combat(id: u32, attacker: &str, defender: &str) -> String {
    format!(
        "naval_combat={{id={{id={id} type=62}} location=1 attacker={{{attacker} positioning=0 anti_air=0}} defender={{{defender} positioning=0 anti_air=0}}}}"
    )
}

#[test]
fn task_force_missions_compare_the_saved_regions_exactly() {
    let (mut frame, mut save) = base();
    let patrol = (6689_u64 << 32) | 51;
    let idle = (8739_u64 << 32) | 51;
    let spotting = (2306_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(
            [patrol, idle, spotting]
                .map(|id| TaskForceRow {
                    id,
                    ..Default::default()
                })
                .into(),
        ),
    );
    let saved = |spotting_region, accessible_regions| counter_inputs::TaskForce {
        spotting_region,
        accessible_regions,
        ..Default::default()
    };
    save.counter_inputs.task_forces = [
        (patrol, saved(Some(79), Some(vec![76, 79, 87, 79, 177]))),
        (idle, saved(None, None)),
        (spotting, saved(Some(84), None)),
    ]
    .into();
    let row = |task_force, current_region, regions: &[u32]| TaskForceMissionRow {
        task_force,
        owner: 101,
        current_region,
        regions: regions.to_vec(),
    };
    let run = |frame: &DecodedFrame, save: &Save| {
        let mut report = Report::default();
        report.attempt_table(TableId::TaskForceMissions, |r| {
            task_force_missions(frame, save, r)
        });
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report.differences
    };
    let exact = [
        row(patrol, 79, &[76, 79, 87, 79, 177]),
        row(spotting, 84, &[]),
    ];
    set(&mut frame, TableRows::TaskForceMissions(exact.to_vec()));
    assert!(run(&frame, &save).is_empty());
    for (current, regions, field) in [
        (79, &[76, 79, 87, 79, 178][..], "regions"),
        (79, &[79, 76, 87, 79, 177][..], "regions"),
        (79, &[76, 79, 87, 90, 177][..], "regions"),
        (87, &[76, 79, 87, 79, 177][..], "current_region"),
    ] {
        let mut rows = exact.to_vec();
        rows[0] = row(patrol, current, regions);
        set(&mut frame, TableRows::TaskForceMissions(rows));
        let differences = run(&frame, &save);
        assert!(
            differences.len() == 1
                && differences[0].contains(&format!("TaskForceMissions {patrol}.{field}")),
            "{differences:?}"
        );
    }
    set(
        &mut frame,
        TableRows::TaskForceMissions(vec![exact[0].clone(), row(idle, 0, &[5])]),
    );
    let differences = run(&frame, &save);
    assert_eq!(differences.len(), 2, "{differences:?}");
    let extra = format!("TaskForceMissions {idle}: ");
    assert!(
        differences
            .iter()
            .any(|d| d.contains(&extra) && d.contains("no saved spotting_region"))
            && differences
                .iter()
                .any(|d| d.contains(&format!("TaskForceMissions {spotting}"))
                    && d.contains("absent")),
        "{differences:?}"
    );
    let mut rows = exact.to_vec();
    rows.push(row(idle, 0, &[]));
    set(&mut frame, TableRows::TaskForceMissions(rows));
    let differences = run(&frame, &save);
    assert!(
        differences.len() == 1
            && differences[0].contains(&extra)
            && differences[0].contains("no saved spotting_region"),
        "{differences:?}"
    );
}

#[test]
fn task_force_style_compares_exact_progress_and_truth_with_independent_refusals() {
    let (mut frame, _) = base();
    let force = (43_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    let base = TaskForceMapMotionRow {
        task_force: force,
        movement_numerator: Recorded::Value(Some(-9_007_199_254_740_993)),
        retreat: Recorded::Value(Some(255)),
        withdrawal: Recorded::Value(Some(2)),
        naval_transfer: Recorded::Value(Some(1)),
        combat_links: Recorded::Value(Some([0; 2])),
        fresh: Recorded::NotRecorded,
        ..Default::default()
    };
    let saved = serialized(
        "movement_progress=-90071992547.40993 retreat=yes withdraw=yes",
        "",
    );
    let mut run = |row: TaskForceMapMotionRow, saved: &Save| {
        set(&mut frame, TableRows::TaskForceMapMotion(vec![row]));
        let mut report = Report::default();
        task_force_motion(&frame, saved, &mut report).unwrap();
        report
    };
    let report = run(base.clone(), &saved);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("does not serialize"))
    );
    let absent = serialized("", "");
    let report = run(
        TaskForceMapMotionRow {
            movement_numerator: Recorded::Value(Some(0)),
            retreat: Recorded::Value(Some(0)),
            withdrawal: Recorded::Value(Some(0)),
            ..base.clone()
        },
        &absent,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(run(base.clone(), &absent).differences.len(), 3);
    for (value, differences) in [(Recorded::NotRecorded, 0), (Recorded::Value(None), 1)] {
        let report = run(
            TaskForceMapMotionRow {
                movement_numerator: value,
                ..base.clone()
            },
            &saved,
        );
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    for change in [
        (|row: &mut TaskForceMapMotionRow| row.retreat = Recorded::Value(None))
            as fn(&mut TaskForceMapMotionRow),
        |row| row.withdrawal = Recorded::Value(None),
        |row| row.naval_transfer = Recorded::Value(None),
        |row| row.combat_links = Recorded::Value(None),
    ] {
        let mut row = base.clone();
        change(&mut row);
        row.target = 2;
        let report = run(row, &saved);
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("target"))
        );
    }
    let report = run(
        TaskForceMapMotionRow {
            movement_numerator: Recorded::NotRecorded,
            retreat: Recorded::NotRecorded,
            withdrawal: Recorded::NotRecorded,
            naval_transfer: Recorded::NotRecorded,
            combat_links: Recorded::NotRecorded,
            ..base.clone()
        },
        &saved,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for name in [
        "movement_numerator",
        "retreat",
        "withdrawal",
        "naval_transfer",
        "combat_links",
    ] {
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains(&format!("TaskForceMapMotion.{name}")))
        );
    }
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            naval_transfer: Recorded::Value(Some(2)),
            ..base
        }]),
    );
    assert!(task_force_motion(&frame, &saved, &mut Report::default()).is_err());
}

#[test]
fn task_force_motion_compares_routes_target_and_saved_combat_membership() {
    let (mut frame, mut save) = base();
    let force = (43_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    let row = TaskForceMapMotionRow {
        task_force: force,
        target: 2,
        combat_role: 1,
        path: vec![2, 1],
        alternate_path: vec![1, 2],
        movement_numerator: Recorded::Value(Some(0)),
        retreat: Recorded::Value(Some(0)),
        withdrawal: Recorded::Value(Some(0)),
        naval_transfer: Recorded::Value(Some(0)),
        combat_links: Recorded::Value(Some([(9_u64 << 32) | 62; 2])),
        fresh: Recorded::NotRecorded,
    };
    let us = "unit={id=43 type=51}";
    let mut run = |row: &TaskForceMapMotionRow, saved: &Save| {
        set(&mut frame, TableRows::TaskForceMapMotion(vec![row.clone()]));
        save.task_forces.clone_from(&saved.task_forces);
        save.naval_combats.clone_from(&saved.naval_combats);
        let mut report = Report::default();
        report.attempt_table(TableId::TaskForceMapMotion, |r| {
            task_force_motion(&frame, &save, r)
        });
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report
    };
    let moving = "support_attack=2 path={ 2 1 } full_path={ 1 2 }";
    let attacking = serialized(moving, &naval_combat(9, us, ""));
    assert!(attacking.task_forces[&force].path == [2, 1]);
    let report = run(&row, &attacking);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 10);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("zero/nonzero truth")),
        "{:?}",
        report.limitations
    );
    let defending = serialized(moving, &naval_combat(9, "", us));
    let report = run(&row, &defending);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .iter()
            .any(|(reason, fields)| reason.contains("owner-country set")
                && fields == &[format!("TaskForceMapMotion {force}.combat_role")])
    );
    assert!(
        run(
            &TaskForceMapMotionRow {
                combat_role: 2,
                ..row.clone()
            },
            &defending
        )
        .differences
        .is_empty()
    );
    let calm = serialized(moving, &naval_combat(9, "unit={id=44 type=51}", ""));
    assert_eq!(run(&row, &calm).differences.len(), 1);
    assert!(
        run(
            &TaskForceMapMotionRow {
                combat_role: 0,
                combat_links: Recorded::Value(Some([0; 2])),
                ..row.clone()
            },
            &calm
        )
        .differences
        .is_empty()
    );
    let standing = serialized("", &naval_combat(9, us, ""));
    let report = run(&row, &standing);
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let still = TaskForceMapMotionRow {
        target: 0,
        path: Vec::new(),
        alternate_path: Vec::new(),
        ..row.clone()
    };
    assert!(run(&still, &standing).differences.is_empty());
    let mut unsaved = attacking.clone();
    unsaved.task_forces.clear();
    assert_eq!(run(&row, &unsaved).differences.len(), 2);
    for listing in [
        format!("{} {}", naval_combat(9, us, ""), naval_combat(10, "", us)),
        format!("{} {}", naval_combat(9, us, ""), naval_combat(10, us, "")),
        naval_combat(9, &format!("{us} {us}"), ""),
    ] {
        let twice = serialized(moving, &listing);
        let report = run(&row, &twice);
        assert!(
            report.failed_comparisons.is_empty() && report.differences.is_empty(),
            "{listing}: {:?}",
            report.differences
        );
    }
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            combat_role: 3,
            ..row
        }]),
    );
    let mut report = Report::default();
    report.attempt_table(TableId::TaskForceMapMotion, |r| {
        task_force_motion(&frame, &attacking, r)
    });
    assert!(!report.failed_comparisons.is_empty());
}

#[test]
fn fresh_task_force_packets_require_each_independent_operand() {
    use replay_data::rows::FreshTaskForceCounterEta;
    let (mut frame, _) = base();
    let force = (43_u64 << 32) | 51;
    let saved = serialized("path={1}", "");
    let packet = FreshTaskForceCounterEta {
        sample_hour: replay_data::rows::PacketHour::Host,
        base_speed: Some(i64::MIN),
        speed_modifier: Some(i64::MAX),
        terrain_class: Some(i32::MIN),
        modified_range: Some(-1),
        exiled: Some(0),
        strategic_redeployment: Some(0),
    };
    let row = TaskForceMapMotionRow {
        task_force: force,
        path: vec![1],
        movement_numerator: Recorded::Value(Some(0)),
        retreat: Recorded::Value(Some(0)),
        withdrawal: Recorded::Value(Some(0)),
        naval_transfer: Recorded::Value(Some(0)),
        combat_links: Recorded::Value(Some([0; 2])),
        fresh: Recorded::Value(Some(packet)),
        ..Default::default()
    };
    let mut run = |row: TaskForceMapMotionRow| {
        set(&mut frame, TableRows::TaskForceMapMotion(vec![row]));
        let mut report = Report::default();
        task_force_fresh(&frame, &saved, &mut report).unwrap();
        report
    };
    assert!(run(row.clone()).differences.is_empty());
    for change in [
        (|packet: &mut FreshTaskForceCounterEta| packet.base_speed = None)
            as fn(&mut FreshTaskForceCounterEta),
        |packet| packet.speed_modifier = None,
        |packet| packet.terrain_class = None,
        |packet| packet.modified_range = None,
        |packet| packet.exiled = None,
        |packet| packet.strategic_redeployment = None,
    ] {
        let mut packet = packet;
        change(&mut packet);
        assert_eq!(
            run(TaskForceMapMotionRow {
                fresh: Recorded::Value(Some(packet)),
                ..row.clone()
            })
            .differences
            .len(),
            1
        );
    }
    assert_eq!(
        run(TaskForceMapMotionRow {
            fresh: Recorded::Value(None),
            ..row.clone()
        })
        .differences
        .len(),
        1
    );
    for changed in [
        TaskForceMapMotionRow {
            path: vec![],
            fresh: Recorded::Value(None),
            ..row.clone()
        },
        TaskForceMapMotionRow {
            target: 2,
            fresh: Recorded::Value(None),
            ..row.clone()
        },
    ] {
        let captured = TaskForceMapMotionRow {
            fresh: Recorded::Value(Some(packet)),
            ..changed.clone()
        };
        let report = run(changed);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains(&format!("TaskForceMapMotion {force}.fresh")));
        assert!(report.outcome().is_err());
        assert!(run(captured).differences.is_empty());
    }
    let historical = run(TaskForceMapMotionRow {
        fresh: Recorded::NotRecorded,
        ..row.clone()
    });
    assert!(historical.differences.is_empty());
    assert_eq!(
        historical.limitations["TaskForceMapMotion.fresh not recorded"],
        vec![format!("TaskForceMapMotion {force}.fresh")],
    );
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            path: vec![2],
            fresh: Recorded::Value(None),
            ..row
        }]),
    );
    let saved = serialized("path={1}", "");
    let mut report = Report::default();
    task_force_motion(&frame, &saved, &mut report).unwrap();
    task_force_fresh(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains(".path"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains(".fresh"))
    );
}

#[test]
fn fresh_task_force_exile_compares_saved_truth_without_claiming_raw_magnitude() {
    use replay_data::rows::FreshTaskForceCounterEta;
    let (mut frame, _) = base();
    let force = (43_u64 << 32) | 51;
    let packet = FreshTaskForceCounterEta {
        sample_hour: replay_data::rows::PacketHour::Host,
        base_speed: Some(0),
        speed_modifier: Some(0),
        terrain_class: Some(0),
        modified_range: Some(0),
        exiled: Some(0),
        strategic_redeployment: Some(255),
    };
    for (suffix, raw, differences) in [
        ("", 0, 0),
        ("exile=no", 0, 0),
        ("exile=yes", 1, 0),
        ("exile=yes", 255, 0),
        ("exile=yes", 0, 1),
        ("", 255, 1),
    ] {
        set(
            &mut frame,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                task_force: force,
                path: vec![1],
                fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                    exiled: Some(raw),
                    ..packet
                })),
                ..Default::default()
            }]),
        );
        let saved = serialized(&format!("path={{1}} {suffix}"), "");
        let mut report = Report::default();
        task_force_fresh(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("nonzero exile byte identity"))
        );
    }
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: force,
            path: vec![2],
            movement_numerator: Recorded::Value(Some(0)),
            retreat: Recorded::Value(Some(0)),
            withdrawal: Recorded::Value(Some(0)),
            naval_transfer: Recorded::Value(Some(0)),
            combat_links: Recorded::Value(Some([0; 2])),
            fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                exiled: Some(1),
                ..packet
            })),
            ..Default::default()
        }]),
    );
    let saved = serialized("path={1}", "");
    let mut report = Report::default();
    task_force_motion(&frame, &saved, &mut report).unwrap();
    task_force_fresh(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains(".path"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("exiled truth"))
    );
}

#[test]
fn task_force_roles_are_unverified_by_saved_participant_sides() {
    let (mut frame, _) = base();
    let force = (43_u64 << 32) | 51;
    let first = (9_u64 << 32) | 62;
    let later = (10_u64 << 32) | 62;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    for (attacker, defender) in [("unit={id=43 type=51}", ""), ("", "unit={id=43 type=51}")] {
        let saved = serialized(
            "",
            &format!(
                "{} {}",
                naval_combat(9, attacker, defender),
                naval_combat(10, attacker, defender)
            ),
        );
        for role in [2, 1, 0] {
            for (links, differences) in [([first; 2], 0), ([first, later], 1)] {
                set(
                    &mut frame,
                    TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                        task_force: force,
                        combat_role: role,
                        combat_links: Recorded::Value(Some(links)),
                        movement_numerator: Recorded::Value(Some(0)),
                        retreat: Recorded::Value(Some(0)),
                        withdrawal: Recorded::Value(Some(0)),
                        naval_transfer: Recorded::Value(Some(0)),
                        fresh: Recorded::NotRecorded,
                        ..Default::default()
                    }]),
                );
                let mut report = Report::default();
                task_force_motion(&frame, &saved, &mut report).unwrap();
                assert_eq!(
                    report.differences.len(),
                    differences + usize::from(role == 0),
                    "{:?}",
                    report.differences
                );
                assert_eq!(
                    report
                        .limitations
                        .iter()
                        .filter(|(reason, _)| reason.contains("owner-country set"))
                        .flat_map(|(_, fields)| fields)
                        .count(),
                    1
                );
            }
        }
    }
}

#[test]
fn task_force_combat_links_select_saved_identity_without_a_singleton_assumption() {
    let (mut frame, _) = base();
    let force = (43_u64 << 32) | 51;
    let first = (9_u64 << 32) | 62;
    let second = (10_u64 << 32) | 62;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    let saved = serialized(
        "path={ 2 1 }",
        &format!(
            "{} {}",
            naval_combat(9, "unit={id=43 type=51}", ""),
            naval_combat(10, "", "unit={id=43 type=51}")
        ),
    );
    let base = TaskForceMapMotionRow {
        task_force: force,
        path: vec![2, 1],
        combat_role: 2,
        movement_numerator: Recorded::Value(Some(0)),
        retreat: Recorded::Value(Some(0)),
        withdrawal: Recorded::Value(Some(0)),
        naval_transfer: Recorded::Value(Some(0)),
        combat_links: Recorded::Value(Some([second; 2])),
        fresh: Recorded::NotRecorded,
        ..Default::default()
    };
    let mut run = |row: TaskForceMapMotionRow| {
        set(&mut frame, TableRows::TaskForceMapMotion(vec![row]));
        let mut report = Report::default();
        task_force_motion(&frame, &saved, &mut report).unwrap();
        report
    };
    assert!(run(base.clone()).differences.is_empty());
    let other_first = run(TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([first; 2])),
        ..base.clone()
    });
    assert!(other_first.differences.is_empty());
    assert!(
        other_first
            .limitations
            .keys()
            .any(|reason| reason.contains("owner-country set"))
    );
    for (naval, differences) in [(second, 1), (0, 2)] {
        let impossible = run(TaskForceMapMotionRow {
            combat_links: Recorded::Value(Some([first, naval])),
            ..base.clone()
        });
        assert_eq!(impossible.differences.len(), differences);
        assert!(
            impossible
                .differences
                .iter()
                .any(|difference| difference.contains("first naval selection"))
        );
        assert!(
            impossible
                .limitations
                .keys()
                .any(|reason| reason.contains("owner-country set"))
        );
        assert!(
            !impossible
                .limitations
                .keys()
                .any(|reason| reason.contains("does not identify the first combat"))
        );
    }
    let missing = run(TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([(11_u64 << 32) | 62; 2])),
        ..base.clone()
    });
    assert_eq!(missing.differences.len(), 1);
    assert!(missing.differences[0].contains("saved naval identity"));
    let nonnaval_missing = run(TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([u64::MAX, (11_u64 << 32) | 62])),
        ..base.clone()
    });
    assert_eq!(nonnaval_missing.differences.len(), 1);
    assert!(nonnaval_missing.differences[0].contains("saved naval identity"));
    let no_naval = run(TaskForceMapMotionRow {
        combat_links: Recorded::Value(Some([u64::MAX, 0])),
        ..base.clone()
    });
    assert_eq!(no_naval.differences.len(), 1);
    assert!(no_naval.differences[0].contains("no saved naval membership"));
    let empty = run(TaskForceMapMotionRow {
        combat_role: 0,
        combat_links: Recorded::Value(Some([0; 2])),
        ..base.clone()
    });
    assert_eq!(empty.differences.len(), 1);
    assert!(empty.differences[0].contains("empty saved naval membership"));
    for links in [
        Recorded::Value(None),
        Recorded::NotRecorded,
        Recorded::Value(Some([u64::MAX, first])),
    ] {
        let refused = matches!(&links, Recorded::Value(None));
        let report = run(TaskForceMapMotionRow {
            path: vec![1, 2],
            combat_links: links,
            ..base.clone()
        });
        assert_eq!(report.differences.len(), if refused { 2 } else { 1 });
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(".path"))
        );
        if !refused {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("combat"))
            );
        }
    }
}
