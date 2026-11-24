use super::*;
use replay_data::{
    rows::DivisionMovementRow, rows::RailCooldownRow, rows::RailEdgeRow,
    rows::RailwayBuildingHealth, rows::RailwayBuildingState, rows::Recorded, rows::StateRow,
};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

fn building(location: u32, name: &[u8], level: i16, healthy: i16) -> save::construction::Building {
    save::construction::Building {
        location,
        building_type: name.to_vec(),
        ordinal: 0,
        level,
        healthy_level: healthy,
        partial_health: Decimal::parse(b"20.00001").unwrap(),
    }
}

#[test]
fn selected_railway_building_level_compares_without_demanding_eta_health() {
    use replay_data::rows::RailwaySelectionInputs;
    let (mut frame, mut saved, _, _) = fixtures::fixture();
    let names = BTreeMap::from([(
        engine_native_parts::cbuilding_template::RAIL_WAY_TOKEN,
        b"rail_way".as_slice(),
    )]);
    saved
        .construction
        .province_buildings
        .insert((2, b"rail_way".to_vec()), building(2, b"rail_way", 4, 3));
    set(&mut frame, TableRows::RailEdges(Vec::new()));
    let hour = frame.hour;
    for (level, differences) in [
        (Some(Some(4)), 0),
        (Some(Some(3)), 1),
        (Some(None), 0),
        (None, 1),
    ] {
        set(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 2,
                cooldown_ticks: 0,
                building: None.into(),
                selection_hour: Some(hour).into(),
                building_bound: Default::default(),
                selection: Some(RailwaySelectionInputs {
                    pair_count: Some(1),
                    levels: None,
                    building_level: level,
                })
                .into(),
            }]),
        );
        let mut report = Report::default();
        naval_history::compare_railway_buildings(&frame, &saved.construction, &names, &mut report)
            .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert_eq!(
            report
                .compared_tables
                .get("RailCooldowns.selection.building_level")
                .copied()
                .unwrap_or(0),
            usize::from(level.is_some_and(|level| level.is_some()))
        );
    }
}

#[test]
fn railway_infrastructure_uses_saved_health_and_exact_content_maximum() {
    let (mut frame, mut saved, _, _) = fixtures::fixture();
    let token = engine_native_parts::cbuilding_database::INFRASTRUCTURE_TOKEN;
    let names = BTreeMap::from([(token, b"infrastructure".as_slice())]);
    saved.construction.state_buildings.insert(
        (1, b"infrastructure".to_vec()),
        building(1, b"infrastructure", 4, 3),
    );
    for (province_max, state_max, global_max, expected) in [
        (None, 12, 17, 25_000),
        (Some(5), 12, 17, 60_000),
        (None, 0, -7, -42_857),
        (None, 0, 0, i64::from(u32::MAX)),
    ] {
        let mut buildings = content_reader_key_names::buildings::Buildings::default();
        buildings.types.insert(
            "infrastructure".into(),
            content_reader_key_names::buildings::BuildingType {
                name: "infrastructure".into(),
                province_max,
                state_max,
                ..Default::default()
            },
        );
        buildings.declaration_order.push("infrastructure".into());
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines
            .load(
                "fixture.lua",
                format!("NDefines={{NBuildings={{MAX_BUILDING_LEVELS={global_max}}}}}").as_bytes(),
            )
            .unwrap();
        set(
            &mut frame,
            TableRows::States(vec![StateRow {
                id: 1,
                railway_infrastructure: Recorded::Value(Some(expected)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        core::compare_railway_infrastructure(
            &frame,
            &saved,
            &buildings,
            &defines,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.scalar_checks, 1);
        for (value, differences) in [
            (Recorded::Value(Some(expected + 1)), 1),
            (Recorded::Value(None), 1),
            (Recorded::NotRecorded, 0),
        ] {
            set(
                &mut frame,
                TableRows::States(vec![StateRow {
                    id: 1,
                    railway_infrastructure: value,
                    ..Default::default()
                }]),
            );
            let mut report = Report::default();
            core::compare_railway_infrastructure(
                &frame,
                &saved,
                &buildings,
                &defines,
                &names,
                &mut report,
            )
            .unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
        }
    }
    saved.construction.state_buildings.clear();
    set(
        &mut frame,
        TableRows::States(vec![StateRow {
            id: 1,
            railway_infrastructure: Recorded::Value(Some(0)),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    core::compare_railway_infrastructure(
        &frame,
        &saved,
        &Default::default(),
        &content_reader_defines::defines::Defines::new().unwrap(),
        &names,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn selected_rail_building_compares_saved_scalars_without_inventing_cache_presence() {
    let (mut frame, mut saved, _, _) = fixtures::fixture();
    let names = BTreeMap::from([(
        engine_native_parts::cbuilding_template::RAIL_WAY_TOKEN,
        b"rail_way".as_slice(),
    )]);
    set(
        &mut frame,
        TableRows::RailEdges(vec![RailEdgeRow {
            from: 2,
            to: 1,
            level: 0,
        }]),
    );
    saved
        .construction
        .province_buildings
        .insert((2, b"rail_way".to_vec()), building(2, b"rail_way", 4, 3));
    let health = RailwayBuildingHealth {
        level: 4,
        healthy: 3,
        partial: Some(2_000_001),
    };
    let mut run = |value, saved: &Save| {
        set(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 2,
                cooldown_ticks: 0,
                building: value,
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]),
        );
        let mut report = Report::default();
        naval_history::compare_railway_buildings(&frame, &saved.construction, &names, &mut report)
            .unwrap();
        report
    };
    let state = |health| {
        Recorded::Value(Some(RailwayBuildingState {
            building: Some(health),
        }))
    };
    let report = run(state(health), &saved);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 3);
    for bad in [
        RailwayBuildingHealth { level: 5, ..health },
        RailwayBuildingHealth {
            healthy: 2,
            ..health
        },
        RailwayBuildingHealth {
            partial: Some(2_000_000),
            ..health
        },
        RailwayBuildingHealth {
            partial: None,
            ..health
        },
    ] {
        assert_eq!(run(state(bad), &saved).differences.len(), 1);
    }
    assert_eq!(run(Recorded::Value(None), &saved).differences.len(), 1);
    let old = run(Recorded::NotRecorded, &saved);
    assert!(
        old.limitations
            .keys()
            .any(|reason| reason.contains("not recorded"))
    );
    let null = run(
        Recorded::Value(Some(RailwayBuildingState { building: None })),
        &saved,
    );
    assert!(null.differences.is_empty());
    assert!(
        null.limitations
            .keys()
            .any(|reason| reason.contains("null-to-absence"))
    );
    saved
        .construction
        .province_buildings
        .get_mut(&(2, b"rail_way".to_vec()))
        .unwrap()
        .healthy_level = 4;
    assert!(
        run(
            state(RailwayBuildingHealth {
                healthy: 4,
                partial: None,
                ..health
            }),
            &saved
        )
        .differences
        .is_empty()
    );
    saved.construction.province_buildings.clear();
    assert_eq!(run(state(health), &saved).differences.len(), 1);
}

#[test]
fn railway_building_absence_requires_a_proven_unreached_gate() {
    let (mut frame, saved, _, _) = fixtures::fixture();
    for (cooldown, edges, differences) in [
        (
            1,
            vec![RailEdgeRow {
                from: 2,
                to: 1,
                level: 0,
            }],
            0,
        ),
        (0, vec![], 0),
        (
            -1,
            vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }],
            0,
        ),
        (
            0,
            vec![RailEdgeRow {
                from: 2,
                to: 1,
                level: 0,
            }],
            1,
        ),
        (
            -1,
            vec![RailEdgeRow {
                from: 2,
                to: 1,
                level: -9,
            }],
            1,
        ),
    ] {
        set(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 2,
                cooldown_ticks: cooldown,
                building: Recorded::Value(None),
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]),
        );
        set(&mut frame, TableRows::RailEdges(edges));
        let mut report = Report::default();
        naval_history::compare_railway_buildings(
            &frame,
            &saved.construction,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    *frame.tables[TableId::RailEdges.index()] = TableView::Unavailable {
        reason: "directed pair census failed".into(),
    };
    assert!(
        naval_history::compare_railway_buildings(
            &frame,
            &saved.construction,
            &BTreeMap::new(),
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn division_base_speed_is_required_for_a_route_and_never_compared() {
    let (mut frame, mut saved, _, _) = fixtures::fixture();
    let id = (7_u64 << 32) | 51;
    saved.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: Some(1),
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: Decimal::parse(b"0").unwrap(),
            organisation: Decimal::parse(b"0").unwrap(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
    let mut run = |base_speed: Option<i64>, moving: bool| {
        let mut current_save = saved.clone();
        let path = if moving { vec![2] } else { Vec::new() };
        current_save
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .path = path.clone();
        set(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: id,
                path_total: path.len() as u32,
                path,
                withdrawal: Some(0).into(),
                movement_priority: Some(1).into(),
                own_base_speed: Some(base_speed),
                fuel: Recorded::NotRecorded,
                queued_actions: Recorded::Value(Some(0)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        military::compare_division_movement(&frame, &current_save, &mut report).unwrap();
        report
    };
    for moving in [false, true] {
        for speed in [i64::MIN, 0, 400_000, i64::MAX] {
            let report = run(Some(speed), moving);
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert_eq!(report.compared_tables["DivisionMovement"], 1);
            let counted = format!(
                "DivisionMovement.base_speed: {} rows recorded, not compared",
                usize::from(moving)
            );
            assert!(report.limitations.iter().any(|(reason, subjects)| {
                reason.contains("the stat block's maximum-speed field")
                    && reason.contains("the save has no counterpart")
                    && subjects == std::slice::from_ref(&counted)
            }));
            assert!(
                !report
                    .limitations
                    .keys()
                    .chain(&report.differences)
                    .any(|text| text.contains("fresh")),
                "{report:?}"
            );
        }
        let refused = run(None, moving);
        assert_eq!(refused.differences.len(), usize::from(moving));
        if moving {
            assert!(refused.differences[0].contains("base_speed"));
            assert!(refused.differences[0].contains("required stat-block speed"));
        }
    }
}

#[test]
fn fresh_task_force_packet_refusal_is_a_difference_for_idle_and_targeted_units() {
    use replay_data::rows::TaskForceMapMotionRow;
    let (mut frame, saved, _, _) = fixtures::fixture();
    let force = (7_u64 << 32) | 51;
    let label = format!("TaskForceMapMotion {force}.fresh");
    for (target, path) in [(0, Vec::new()), (2, vec![1, 2])] {
        for (fresh, refused) in [
            (Recorded::Value(None), true),
            (Recorded::NotRecorded, false),
        ] {
            set(
                &mut frame,
                TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: force,
                    target,
                    path: path.clone(),
                    fresh,
                    ..Default::default()
                }]),
            );
            let mut report = Report::default();
            pass_inputs::compare(&frame, &saved, &mut report);
            let differences: Vec<_> = report
                .differences
                .iter()
                .filter(|difference| difference.starts_with(&label))
                .collect();
            assert_eq!(differences.len(), usize::from(refused), "{differences:?}");
            if refused {
                assert!(differences[0].contains("required current-hour task-force packet"));
            } else {
                assert!(report.limitations.iter().any(|(reason, subjects)| {
                    reason.contains("TaskForceMapMotion.fresh")
                        && reason.contains("not recorded")
                        && subjects.iter().any(|subject| subject == &label)
                }));
            }
            assert!(
                !report
                    .failed_comparisons
                    .contains_key("TaskForceMapMotion fresh")
            );
        }
    }
}

#[test]
fn fresh_idle_railway_packet_refusal_differs_from_historical_absence_and_native_zero() {
    use replay_data::{rows::FreshRailwayCounterEta, rows::RailwayGunCounterInputRow};
    let (mut frame, saved, _, _) = fixtures::fixture();
    let gun = (7_u64 << 32) | 51;
    let label = format!("RailwayGunCounterInputs {gun}.fresh");
    for (fresh, refused, historical) in [
        (Recorded::Value(None), true, false),
        (Recorded::NotRecorded, false, true),
        (
            Recorded::Value(Some(FreshRailwayCounterEta {
                sample_hour: replay_data::rows::PacketHour::Host,
                maximum_speed: Some(0),
                land_supply_factor: Some(0),
                orders_group: Some(0),
            })),
            false,
            false,
        ),
    ] {
        set(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow {
                gun,
                ordinary_path: Vec::new(),
                fresh,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        railway_guns::compare(&frame, &saved, &mut report);
        let differences: Vec<_> = report
            .differences
            .iter()
            .filter(|difference| difference.starts_with(&label))
            .collect();
        assert_eq!(differences.len(), usize::from(refused), "{differences:?}");
        if refused {
            assert!(differences[0].contains("required current-hour railway-gun packet"));
        }
        if historical {
            assert!(report.limitations.iter().any(|(reason, subjects)| {
                reason.contains("RailwayGunCounterInputs.fresh")
                    && reason.contains("not recorded")
                    && subjects.iter().any(|subject| subject == &label)
            }));
        }
        assert!(
            !report
                .failed_comparisons
                .contains_key("RailwayGunCounterInputs fresh")
        );
    }
}
