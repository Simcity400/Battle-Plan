use super::*;
use replay::rows::{
    ControllerRiverSpeed, CountryCounterInputRow, CountryMovementInputs, DivisionMovementRow,
    FreshCounterEta, FreshNormalEta, NavalPathInputs, RailCooldownRow, RailEdgeRow,
    RailwayBuildingHealth, RailwayBuildingState, Recorded, RegionTransportSpeed, StateRow,
    TerrainSpeed, TheatreControllerAreas,
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
    use replay::rows::RailwaySelectionInputs;
    let (mut frame, mut saved, _, _) = tests::fixture();
    let names = BTreeMap::from([(
        engine::cbuilding_template::RAIL_WAY_TOKEN,
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
    let (mut frame, mut saved, _, _) = tests::fixture();
    let token = engine::cbuilding_database::INFRASTRUCTURE_TOKEN;
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
        let mut buildings = installed_content::buildings::Buildings::default();
        buildings.types.insert(
            "infrastructure".into(),
            installed_content::buildings::BuildingType {
                name: "infrastructure".into(),
                province_max,
                state_max,
                ..Default::default()
            },
        );
        buildings.declaration_order.push("infrastructure".into());
        let defines = installed_content::defines::Defines::new().unwrap();
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
            // An older decoder retains absence instead of a native zero.
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
    // The disabled null template and missing state building both yield zero.
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
        &installed_content::defines::Defines::new().unwrap(),
        &names,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn selected_rail_building_compares_saved_scalars_without_inventing_cache_presence() {
    let (mut frame, mut saved, _, _) = tests::fixture();
    let names = BTreeMap::from([(
        engine::cbuilding_template::RAIL_WAY_TOKEN,
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
    // Historical absence and observed null remain distinct named limitations.
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
    // Even though partial health is saved, native's undamaged branch omits it.
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
    let (mut frame, saved, _, _) = tests::fixture();
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
fn fresh_movement_profiles_admit_idle_units_without_a_legacy_eta_row() {
    let (mut frame, mut saved, _, _) = tests::fixture();
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
    *frame.tables[TableId::DivisionEtaInputs.index()] = TableView::Unavailable {
        reason: "legacy stored-route operands are deliberately outside this fixture".into(),
    };
    let fresh = FreshCounterEta {
        sample_hour: frame.hour,
        current_speed: Some(i64::MIN),
        normal: Some(FreshNormalEta {
            terrain_speed: vec![TerrainSpeed {
                terrain: 1,
                modifier: i64::MAX,
            }],
            ..Default::default()
        }),
        rail: None,
    };
    let mut run = |value, moving: bool| {
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
                fresh: value,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        military::compare_division_movement(&frame, &current_save, &mut report)?;
        Ok::<_, io::Error>(report)
    };
    for moving in [false, true] {
        let report = run(Recorded::Value(Some(fresh.clone())), moving).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["DivisionMovement"], 1);
        // Current profile refusal and wrong-hour attribution remain differences
        // even when the division has no stored first edge.
        for value in [
            Recorded::Value(None),
            Recorded::Value(Some(FreshCounterEta {
                sample_hour: fresh.sample_hour - 1,
                ..fresh.clone()
            })),
        ] {
            assert_eq!(run(value, moving).unwrap().differences.len(), 1);
        }
        let idle_or_failed_current = run(
            Recorded::Value(Some(FreshCounterEta {
                current_speed: None,
                ..fresh.clone()
            })),
            moving,
        )
        .unwrap();
        assert_eq!(
            idle_or_failed_current.differences.len(),
            usize::from(moving)
        );
        assert!(
            run(
                Recorded::Value(Some(FreshCounterEta {
                    current_speed: Some(0),
                    ..fresh.clone()
                })),
                moving
            )
            .unwrap()
            .differences
            .is_empty()
        );
        let historical = run(Recorded::NotRecorded, moving).unwrap();
        assert!(historical.differences.is_empty());
        assert!(
            historical
                .limitations
                .keys()
                .any(|reason| reason.contains("not recorded"))
        );
        let absent_normal = run(
            Recorded::Value(Some(FreshCounterEta {
                normal: None,
                ..fresh.clone()
            })),
            moving,
        )
        .unwrap();
        assert!(absent_normal.differences.is_empty());
        assert!(
            absent_normal
                .limitations
                .keys()
                .any(|reason| reason.contains("normal domain was not reached"))
        );
        for terrains in [
            vec![
                TerrainSpeed {
                    terrain: 1,
                    modifier: 0
                };
                2
            ],
            vec![TerrainSpeed {
                terrain: 0,
                modifier: 0,
            }],
        ] {
            assert!(
                run(
                    Recorded::Value(Some(FreshCounterEta {
                        normal: Some(FreshNormalEta {
                            terrain_speed: terrains,
                            ..Default::default()
                        }),
                        ..fresh.clone()
                    })),
                    moving
                )
                .is_err()
            );
        }
    }
}

#[test]
fn owner_movement_domains_compare_alias_resolved_controllers_and_independent_refusals() {
    let (mut frame, _, _, _) = tests::fixture();
    let base = CountryMovementInputs {
        sample_hour: frame.hour,
        river: Some(vec![
            ControllerRiverSpeed {
                target_slot: 0,
                modifier: Some(i64::MIN),
            },
            ControllerRiverSpeed {
                target_slot: 7,
                modifier: Some(i64::MAX),
            },
        ]),
        transport: Some(vec![
            RegionTransportSpeed {
                region: 0,
                speed: Some(i64::MAX),
            },
            RegionTransportSpeed {
                region: 900,
                speed: Some(i64::MIN),
            },
        ]),
        naval: None,
        theatre: None,
    };
    let mut run = |movement| {
        set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        country_rosters::compare_movement(&frame, &mut report)?;
        Ok::<_, io::Error>(report)
    };
    assert!(
        run(Recorded::Value(Some(base.clone())))
            .unwrap()
            .differences
            .is_empty()
    );
    let mut broken = base.clone();
    broken.river.as_mut().unwrap()[0].modifier = None;
    broken.transport.as_mut().unwrap()[1].speed = None;
    assert_eq!(
        run(Recorded::Value(Some(broken)))
            .unwrap()
            .differences
            .len(),
        2
    );
    let mut missing = base.clone();
    missing.river.as_mut().unwrap().pop();
    assert_eq!(
        run(Recorded::Value(Some(missing)))
            .unwrap()
            .differences
            .len(),
        1
    );
    let mut duplicate = base.clone();
    duplicate.transport.as_mut().unwrap()[1].region = 0;
    assert!(
        !run(Recorded::Value(Some(duplicate)))
            .unwrap()
            .failed_comparisons
            .is_empty()
    );
    // Undemanded owners legitimately have no lists, rather than empty domains.
    let absent = run(Recorded::Value(Some(CountryMovementInputs {
        sample_hour: base.sample_hour,
        ..Default::default()
    })))
    .unwrap();
    assert!(absent.differences.is_empty());
    assert!(
        absent
            .limitations
            .keys()
            .any(|reason| reason.contains("not demanded"))
    );
    assert!(run(Recorded::NotRecorded).unwrap().differences.is_empty());
    assert_eq!(run(Recorded::Value(None)).unwrap().differences.len(), 1);
    assert_eq!(
        run(Recorded::Value(Some(CountryMovementInputs {
            sample_hour: base.sample_hour - 1,
            ..base
        })))
        .unwrap()
        .differences
        .len(),
        1
    );
}

#[test]
fn undemanded_or_historical_owner_domains_do_not_require_a_province_census() {
    let (mut frame, _, _, _) = tests::fixture();
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "province census refused".into(),
    };
    for movement in [
        Recorded::Value(Some(CountryMovementInputs {
            sample_hour: frame.hour,
            ..Default::default()
        })),
        Recorded::NotRecorded,
    ] {
        set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        country_rosters::compare_movement(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
    }
    let sample_hour = frame.hour;
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay::rows::Recorded::NotRecorded,
            country: 101,
            movement: Recorded::Value(Some(CountryMovementInputs {
                sample_hour,
                river: Some(vec![]),
                transport: Some(vec![RegionTransportSpeed {
                    region: 0,
                    speed: None,
                }]),
                ..Default::default()
            })),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    country_rosters::compare_movement(&frame, &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("province census refused"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("transport region 0"))
    );
}

#[test]
fn owner_naval_lists_preserve_physical_occurrences_and_independent_failures() {
    let (mut frame, _, _, _) = tests::fixture();
    let packet = CountryMovementInputs {
        sample_hour: frame.hour,
        naval: Some(NavalPathInputs {
            navy_slot: 7,
            present: true,
            foreign_navy_slots: Some(vec![0, 900, 900]),
            supply_hubs: Some(vec![0, 2, 2]),
        }),
        theatre: Some(TheatreControllerAreas {
            provinces: vec![1, 2],
            null_controller_area: true,
        }),
        ..Default::default()
    };
    let mut run = |packet: CountryMovementInputs| {
        set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement: Recorded::Value(Some(packet)),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        country_rosters::compare_movement(&frame, &mut report).unwrap();
        report
    };
    let exact = run(packet.clone());
    assert!(exact.differences.is_empty());
    assert!(exact.failed_comparisons.is_empty());
    let mut broken = packet.clone();
    let naval = broken.naval.as_mut().unwrap();
    naval.foreign_navy_slots = None;
    naval.supply_hubs = None;
    broken.theatre.as_mut().unwrap().provinces.reverse();
    let report = run(broken);
    assert_eq!(report.differences.len(), 2);
    assert_eq!(report.failed_comparisons.len(), 1);
    let mut wrong_slot = packet.clone();
    wrong_slot.naval.as_mut().unwrap().navy_slot = 900;
    assert_eq!(run(wrong_slot).differences.len(), 1);
    let null = CountryMovementInputs {
        naval: Some(NavalPathInputs {
            navy_slot: 7,
            present: false,
            foreign_navy_slots: Some(vec![]),
            supply_hubs: Some(vec![]),
        }),
        theatre: None,
        ..packet
    };
    assert!(run(null.clone()).differences.is_empty());
    let mut refused = null;
    let naval = refused.naval.as_mut().unwrap();
    naval.foreign_navy_slots = None;
    naval.supply_hubs = None;
    assert_eq!(run(refused).differences.len(), 2);
}

#[test]
fn fresh_task_force_packet_refusal_is_a_difference_for_idle_and_targeted_units() {
    use replay::rows::TaskForceMapMotionRow;
    let (mut frame, saved, _, _) = tests::fixture();
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
            // Other counter domains have their own fixture availability.
            // Check only this genuine admission contract, without waiving it.
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
    use replay::rows::{FreshRailwayCounterEta, RailwayGunCounterInputRow};
    let (mut frame, saved, _, _) = tests::fixture();
    let hour = frame.hour;
    let gun = (7_u64 << 32) | 51;
    let label = format!("RailwayGunCounterInputs {gun}.fresh");
    for (fresh, refused, historical) in [
        (Recorded::Value(None), true, false),
        (Recorded::NotRecorded, false, true),
        (
            Recorded::Value(Some(FreshRailwayCounterEta {
                sample_hour: hour,
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
