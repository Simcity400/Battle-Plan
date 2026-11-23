use super::test_support::{DIVISION, LAND, SPEED, content, frame, hour, moving, panel, provinces, put, status_row, two_leg_map, world};
use super::*;
use content_reader_map::movement::{Neighbour, ProvinceTerrain};
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryMovementModifiers,
    rows::DivisionMovementRow, rows::DivisionMovementStats, rows::DivisionPanelStatsRow,
    rows::DivisionRow,
    rows::DivisionStatusRow, rows::MovementDefines as RecordedDefines, rows::ProvinceRow,
    rows::Recorded, rows::SimulationFeatureRow, rows::StateRow, rows::TableRows,
    rows::WarRelationRow,
};
use replay_derive::eta_derive::SpeedTerms;
use replay_frame::frame::TableView;
use std::cell::Cell;

const UNRECORDED: &str = "the division's base speed is not recorded";

fn plain_speed(base: i64, organisation: Option<i64>, defines: &Defines) -> i64 {
    let edge = |_: u32, _: u32| Ok(eta_derive::EdgeTerm::Normal(Default::default()));
    eta_derive::edge_speed(
        base,
        1,
        2,
        &SpeedTerms {
            modifier: 100_000,
            edge: &edge,
            organisation,
            fuel: None,
            base_speed: Some(base),
        },
        defines,
    )
    .unwrap()
}

fn busy_hour(count: u32, divisions: u64, legs: usize) -> (DecodedFrame, MovementMap) {
    let provinces_count = count;
    let mut frame = frame();
    put(&mut frame, provinces(1..=provinces_count));
    let hop = |division: u64, leg: usize| (division as u32 * 7 + leg as u32) % provinces_count + 1;
    put(
        &mut frame,
        TableRows::Divisions(
            (0..divisions)
                .map(|id| DivisionRow {
                    id,
                    province: hop(id, 0),
                    roster_owner: 1,
                    logical_owner: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus((0..divisions).map(status_row).collect()),
    );
    put(
        &mut frame,
        TableRows::DivisionPanelStats((0..divisions).map(|id| panel(id, Some(SPEED))).collect()),
    );
    world(&mut frame);
    put(
        &mut frame,
        TableRows::DivisionMovement(
            (0..divisions)
                .map(|id| DivisionMovementRow {
                    division: id,
                    path: (1..=legs).map(|leg| hop(id, leg)).collect(),
                    path_total: legs as u32,
                    withdrawal: Recorded::Value(Some(0)),
                    fuel: Recorded::Value(Some(0)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let map = MovementMap::from_parts(
        &(1..=provinces_count)
            .map(|id| (id, LAND, Vec::new()))
            .collect::<Vec<_>>(),
    );
    (frame, map)
}

#[test]
fn an_hour_without_movement_has_no_estimate_and_no_refusal() {
    let map = content(two_leg_map());
    let etas = Etas::read(&frame(), Some(&map)).unwrap();
    assert_eq!(etas.report(), &Report::default());
    assert_eq!(etas.get(DIVISION), None);
    assert_eq!(etas.report().line(), None);
    let skipped = Report {
        route_hops_skipped: 2,
        ..Report::default()
    };
    assert!(
        skipped
            .line()
            .is_some_and(|line| line.ends_with("0 routes unplaced, 2 route hops skipped"))
    );
}

#[test]
fn an_estimate_is_the_remaining_route_distance_over_the_edge_speed() {
    let map = content(two_leg_map());
    for (base, speed) in [(SPEED, SPEED), (350_000, 350_000), (1, 100_000)] {
        assert_eq!(plain_speed(base, None, &map.defines), speed);
        let etas = Etas::read(&hour(Some(base).into()), Some(&map)).unwrap();
        let leg = |distance: i64| distance * 100_000 / speed;
        let progress = leg(175_000);
        let next = leg(350_000) - progress;
        let destination = leg(350_000) + leg(700_000) - progress;
        assert_eq!(etas.get(DIVISION), Some((next, destination)), "{base}");
        assert_eq!(etas.estimate(DIVISION), Ok((next, destination)));
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        assert_eq!(
            etas.report(),
            &Report {
                estimated: 1,
                ..Report::default()
            }
        );
    }
    let etas = Etas::read(&hour(Some(SPEED)), Some(&map)).unwrap();
    assert_eq!(etas.get(DIVISION), Some((25_000, 125_000)));
    assert_eq!(super::arrival(&etas, DIVISION, true), Ok(125_000));
    assert_eq!(super::arrival(&etas, DIVISION, false), Ok(25_000));
}

#[test]
fn older_recordings_refuse_estimates_by_name_but_keep_progress() {
    let map = content(two_leg_map());
    for (base_speed, reason) in [(None, UNRECORDED)] {
        let etas = Etas::read(&hour(base_speed), Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(etas.estimate(DIVISION), Err(reason));
        assert_eq!(super::arrival(&etas, DIVISION, true), Err(reason.into()));
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        let report = etas.report();
        assert_eq!((report.estimated, report.refused_total()), (0, 1));
        assert_eq!(report.refused.get(reason).copied(), Some(1));
        assert_eq!(report.progress_refused, 0);
        assert!(
            report
                .line()
                .is_some_and(|line| line.contains(&format!("1 estimates refused (1 {reason})")))
        );
    }
}

#[test]
fn missing_location_content_or_geometry_refuse_by_name() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(&mut frame, TableRows::Divisions(vec![]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.estimate(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.progress(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.report().progress_refused, 1);

    let partial = content(MovementMap::from_parts(&[(1, LAND, Vec::new())]));
    let etas = Etas::read(&hour(Some(SPEED)), Some(&partial)).unwrap();
    assert_eq!(
        etas.estimate(DIVISION),
        Err("province 2 has no content terrain")
    );
    assert_eq!(
        etas.progress(DIVISION),
        Err("eta_derive::division_progress refuses its recorded route with the content map")
    );

    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::DivisionMovement(vec![
            moving(),
            DivisionMovementRow {
                division: DIVISION + 1,
                ..Default::default()
            },
        ]),
    );
    let etas = Etas::read(&frame, None).unwrap();
    assert_eq!(etas.estimate(DIVISION), Err(NO_CONTENT));
    assert_eq!(etas.progress(DIVISION), Err(NO_CONTENT));
    assert_eq!(etas.progress(DIVISION + 1), Ok(0));
    assert_eq!(etas.estimate(DIVISION + 1), Err(NO_ROUTE));
    assert_eq!(
        (
            etas.report().refused_total(),
            etas.report().progress_refused
        ),
        (1, 1)
    );
}

#[test]
fn the_cache_holds_the_hours_in_flight_and_the_latest() {
    let cache = EtaCache::default();
    let built = Cell::new(0);
    let fill = |hour: replay_data::Hour| {
        cache
            .get_or_insert_with((1, hour), || {
                built.set(built.get() + 1);
                Ok::<_, io::Error>(Etas::default())
            })
            .expect("the build cannot fail")
    };
    let ahead: Vec<_> = (1..=3).map(fill).collect();
    assert_eq!(built.get(), 3);
    fill(1);
    fill(2);
    assert_eq!(built.get(), 3, "the look-ahead must still be cached");
    drop(ahead);
    fill(4);
    fill(4);
    assert_eq!(built.get(), 4);
    fill(1);
    assert_eq!(built.get(), 5, "hour 1 should have gone");
}

#[test]
fn a_busy_hour_estimates_every_route() {
    let (frame, map) = busy_hour(130, 10, 30);
    let map = content(map);
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.report().estimated, 10);
    assert_eq!(etas.report().refused_total(), 0);
    assert_eq!(etas.report().progress_refused, 0);
    assert_eq!(etas.get(0).map(|pair| pair.1), Some(30 * 14_285));
}

#[test]
fn each_leg_takes_the_recorded_infrastructure_of_its_destination_state() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    state: if id == 3 { 9 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert!(etas.estimate(DIVISION).unwrap_err().contains("States"));
    put(
        &mut frame,
        TableRows::States(vec![StateRow {
            id: 9,
            movement_infrastructure: -50_000,
            ..Default::default()
        }]),
    );
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.estimate(DIVISION), Ok((25_000, 225_000)));
    put(&mut frame, provinces(1..=2));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(
        etas.estimate(DIVISION),
        Err("Provinces has no row for province 3")
    );
}

#[test]
fn a_low_organisation_hostile_move_slows_by_the_native_factor() {
    let mut map = content(two_leg_map());
    map.defines.organisation = Some(OrganisationDefines {
        threshold: 30_000,
        penalty: -80_000,
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            logical_owner: 1,
            roster_owner: 1,
            organisation: 100_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: if id == 3 { 2 } else { 1 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            organisation_exact: Recorded::Value(Some(100_000)),
            maximum_organisation_exact: Recorded::Value(Some(1_000_000)),
            move_flags: DivisionStatusRow::MOVE_LOW_ORGANISATION,
            ..status_row(DIVISION)
        }]),
    );
    put(&mut frame, TableRows::WarRelations(vec![]));
    let full = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(full.estimate(DIVISION), Ok((25_000, 125_000)));
    put(
        &mut frame,
        TableRows::WarRelations(vec![WarRelationRow {
            first_country: 1,
            second_country: 2,
            ..Default::default()
        }]),
    );
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    let speed = plain_speed(SPEED, Some(46_667), &map.defines);
    assert_eq!(speed, 326_669);
    let leg = |distance: i64| distance * 100_000 / speed;
    let progress = leg(175_000);
    assert_eq!(
        etas.estimate(DIVISION),
        Ok((
            leg(350_000) - progress,
            leg(350_000) + leg(700_000) - progress
        ))
    );
}

#[test]
fn movement_timing_uses_conditional_exact_organisation_and_names_refusals() {
    let mut map = content(two_leg_map());
    map.defines.organisation = Some(OrganisationDefines {
        threshold: 30_000,
        penalty: -80_000,
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            logical_owner: 1,
            roster_owner: 1,
            organisation: 100_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: if id == 3 { 2 } else { 1 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::WarRelations(vec![WarRelationRow {
            first_country: 1,
            second_country: 2,
            ..Default::default()
        }]),
    );
    let mut estimates = Vec::new();
    for current in [100_000, 109_999] {
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                organisation_exact: Recorded::Value(Some(current)),
                maximum_organisation_exact: Recorded::Value(Some(1_009_999)),
                move_flags: DivisionStatusRow::MOVE_LOW_ORGANISATION,
                ..status_row(DIVISION)
            }]),
        );
        estimates.push(
            Etas::read(&frame, Some(&map))
                .unwrap()
                .estimate(DIVISION)
                .unwrap(),
        );
    }
    assert_ne!(estimates[0], estimates[1]);
    for organisation_exact in [Recorded::Value(None), Recorded::NotRecorded] {
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                organisation_exact,
                maximum_organisation_exact: Recorded::Value(Some(1_000_000)),
                move_flags: DivisionStatusRow::MOVE_LOW_ORGANISATION,
                ..status_row(DIVISION)
            }]),
        );
        assert!(
            Etas::read(&frame, Some(&map))
                .unwrap()
                .estimate(DIVISION)
                .unwrap_err()
                .contains("organisation")
        );
    }
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            organisation_exact: Recorded::Value(None),
            maximum_organisation_exact: Recorded::Value(None),
            ..status_row(DIVISION)
        }]),
    );
    assert_eq!(
        Etas::read(&frame, Some(&map)).unwrap().estimate(DIVISION),
        Ok((25_000, 125_000))
    );
}

#[test]
fn recorded_speed_terms_move_each_edge() {
    let mut map = content(two_leg_map());
    map.map = MovementMap::from_parts(&[
        (
            1,
            LAND,
            vec![Neighbour {
                to: 2,
                distance: Some(350_000),
                ..Default::default()
            }],
        ),
        (
            2,
            LAND,
            vec![
                Neighbour {
                    to: 1,
                    distance: Some(350_000),
                    ..Default::default()
                },
                Neighbour {
                    to: 3,
                    distance: Some(700_000),
                    ..Default::default()
                },
            ],
        ),
        (
            3,
            ProvinceTerrain { flags: 1, index: 2 },
            vec![Neighbour {
                to: 2,
                distance: Some(700_000),
                ..Default::default()
            }],
        ),
    ]);
    map.land = Ok(LandSpeedContent {
        terrain_movement: vec![0, -10_000],
        regions: BTreeMap::new(),
        ..Default::default()
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            movement_stats: Recorded::Value(Some(DivisionMovementStats {
                terrain: vec![0, -50_000],
                scalars: Some([0; 3]),
                fuel_rate: 1_000_000,
                air_defense: 0,
            })),
            ..status_row(DIVISION)
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 1,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                army_speed_factor: Some(-25_000),
                ..Default::default()
            })),
            ..Default::default()
        }]),
    );
    let mut row = moving();
    row.fuel = Recorded::Value(Some(96_000_000));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    let leg = |distance: i64, speed: i64| distance * 100_000 / speed;
    let progress = leg(175_000, 525_000);
    let next = leg(350_000, 525_000) - progress;
    assert_eq!(
        etas.estimate(DIVISION),
        Ok((next, next + leg(700_000, 105_000)))
    );
    row.fuel = Recorded::Value(Some(24_000_000));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let fueled = Etas::read(&frame, Some(&map)).unwrap();
    let first = 525_000 * 55_000 / 100_000;
    let second = 100_000;
    let progress = leg(175_000, first);
    let next = leg(350_000, first) - progress;
    assert_eq!(
        fueled.estimate(DIVISION),
        Ok((next, next + leg(700_000, second)))
    );
    row.strategic_redeployment = 1;
    row.strategic_action = Recorded::Value(replay_data::rows::StrategicAction::Unreached);
    put(&mut frame, TableRows::DivisionMovement(vec![row]));
    let refused = Etas::read(&frame, Some(&map)).unwrap();
    assert!(
        refused
            .estimate(DIVISION)
            .unwrap_err()
            .contains("strategically redeploying")
    );
}

#[test]
fn enemy_air_superiority_slows_an_owner_at_war() {
    let mut map = content(two_leg_map());
    map.land = Ok(LandSpeedContent {
        terrain_movement: vec![0],
        regions: BTreeMap::from([(1, 70), (2, 70), (3, 70)]),
        ..Default::default()
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(RecordedDefines {
                out_of_supply_speed: Some(-80_000),
                retreat_speed: Some(25_000),
                withdrawal_speed: Some(-50_000),
                out_of_fuel_speed: Some(40_000),
                fuel_capacity_scale: Some(9_600_000),
                supply_days: Some(30),
                supply_grace: Some(72),
                air_speed: Some(50_000),
                air_defense_denominator: Some(100_000),
                air_defense_reduction: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::RaidAirScores(vec![replay_data::rows::RaidAirScoreRow {
            country: 1,
            region: 70,
            friendly: 1,
            enemy: 3,
            impact: Recorded::Value(-40_000),
            friendly_missions: 0,
            enemy_missions: 0,
            air_spotting_scale: Some(0),
            menu: replay_data::rows::Recorded::Value(None),
        }]),
    );
    put(&mut frame, TableRows::WarRelations(vec![]));
    let leg = |distance: i64, speed: i64| distance * 100_000 / speed;
    let at_peace = Etas::read(&frame, Some(&map)).unwrap();
    let next = leg(350_000, SPEED) - leg(175_000, SPEED);
    assert_eq!(
        at_peace.estimate(DIVISION),
        Ok((next, next + leg(700_000, SPEED)))
    );
    put(
        &mut frame,
        TableRows::WarRelations(vec![WarRelationRow {
            first_country: 1,
            second_country: 2,
            ..Default::default()
        }]),
    );
    let at_war = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 85_000 / 100_000;
    let next = leg(350_000, slowed) - leg(175_000, slowed);
    assert_eq!(
        at_war.estimate(DIVISION),
        Ok((next, next + leg(700_000, slowed)))
    );
}

fn two_legs(first: i64, second: i64) -> Result<(i64, i64), &'static str> {
    let leg = |distance: i64, speed: i64| distance * 100_000 / speed;
    let next = leg(350_000, first) - leg(175_000, first);
    Ok((next, next + leg(700_000, second)))
}

fn hostile_destination(frame: &mut DecodedFrame, war: bool) {
    put(
        frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: if id == 3 { 2 } else { 1 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
        ]),
    );
    put(
        frame,
        TableRows::WarRelations(if war {
            vec![WarRelationRow {
                first_country: 1,
                second_country: 2,
                ..Default::default()
            }]
        } else {
            vec![]
        }),
    );
}

#[test]
fn attack_speed_applies_only_to_a_hostile_route() {
    let mut map = content(two_leg_map());
    map.defines.organisation = Some(OrganisationDefines {
        threshold: 30_000,
        penalty: -80_000,
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            logical_owner: 1,
            roster_owner: 1,
            organisation: 1_000_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            ..status_row(DIVISION)
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 1,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                army_attack_speed_factor: Some(50_000),
                ..Default::default()
            })),
            ..Default::default()
        }]),
    );
    hostile_destination(&mut frame, false);
    let at_peace = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(at_peace.estimate(DIVISION), two_legs(SPEED, SPEED));
    hostile_destination(&mut frame, true);
    let hostile = Etas::read(&frame, Some(&map)).unwrap();
    let fast = SPEED * 150_000 / 100_000;
    assert_eq!(hostile.estimate(DIVISION), two_legs(fast, fast));
}

#[test]
fn location_weather_is_scaled_by_acclimatization() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![replay_data::rows::WeatherProvinceRow {
            id: 1,
            modifiers: Recorded::Value(Some(replay_data::rows::ProvinceWeatherModifiers {
                entries: vec![replay_data::rows::ProvinceModifierEntry {
                    key: engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
                    value: -20_000,
                }],
            })),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            weather_factor: Recorded::Value(Some(50_000)),
            ..status_row(DIVISION)
        }]),
    );
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 90_000 / 100_000;
    assert_eq!(etas.estimate(DIVISION), two_legs(slowed, slowed));
}

fn commanded(frame: &mut DecodedFrame, terrain: Vec<i64>, modifiers: [Option<i64>; 5]) {
    put(
        frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            roster_owner: 1,
            logical_owner: 1,
            orders_group: 5,
            ..Default::default()
        }]),
    );
    put(
        frame,
        TableRows::OrdersGroupCounterInputs(vec![replay_data::rows::OrdersGroupCounterInputRow {
            group: 5,
            commander_movement: Recorded::Value(Some(Some(
                replay_data::rows::CommanderMovementStats { terrain },
            ))),
            ..Default::default()
        }]),
    );
    put(
        frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            commander_modifiers: Recorded::Value(Some([
                modifiers[0],
                modifiers[1],
                modifiers[2],
                modifiers[3],
                modifiers[4],
            ])),
            ..status_row(DIVISION)
        }]),
    );
}

fn river_map() -> MovementMap {
    MovementMap::from_parts(&[
        (
            1,
            LAND,
            vec![Neighbour {
                to: 2,
                distance: Some(350_000),
                ..Default::default()
            }],
        ),
        (
            2,
            LAND,
            vec![
                Neighbour {
                    to: 1,
                    distance: Some(350_000),
                    ..Default::default()
                },
                Neighbour {
                    to: 3,
                    distance: Some(700_000),
                    kind: 2,
                    ..Default::default()
                },
            ],
        ),
        (
            3,
            LAND,
            vec![Neighbour {
                to: 2,
                distance: Some(700_000),
                kind: 2,
                ..Default::default()
            }],
        ),
    ])
}

#[test]
fn the_commander_river_factor_scales_a_crossing() {
    let mut map = content(river_map());
    map.defines.river_crossing_two = -30_000;
    let mut frame = hour(Some(SPEED));
    commanded(&mut frame, vec![0, 0], [None; 5]);
    let plain = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(
        plain.estimate(DIVISION),
        two_legs(SPEED, SPEED * 70_000 / 100_000)
    );
    commanded(
        &mut frame,
        vec![0, 0],
        [None, None, None, Some(50_000), None],
    );
    let led = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(
        led.estimate(DIVISION),
        two_legs(SPEED, SPEED * 55_000 / 100_000)
    );
}

#[test]
fn commander_terrain_falls_back_to_the_first_entry() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    commanded(&mut frame, vec![0, -20_000], [None; 5]);
    let in_range = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 80_000 / 100_000;
    assert_eq!(in_range.estimate(DIVISION), two_legs(slowed, slowed));
    commanded(&mut frame, vec![-10_000], [None; 5]);
    let fallback = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 90_000 / 100_000;
    assert_eq!(fallback.estimate(DIVISION), two_legs(slowed, slowed));
}

#[test]
fn a_zero_supply_days_define_refuses_by_name() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            supply_ratio: Some(50_000),
            ..status_row(DIVISION)
        }]),
    );
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert!(etas.estimate(DIVISION).is_ok());
    put(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(RecordedDefines {
                out_of_supply_speed: Some(-80_000),
                retreat_speed: Some(25_000),
                withdrawal_speed: Some(-50_000),
                out_of_fuel_speed: Some(40_000),
                fuel_capacity_scale: Some(9_600_000),
                supply_days: Some(0),
                supply_grace: Some(72),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    let refused = Etas::read(&frame, Some(&map)).unwrap();
    assert!(
        refused
            .estimate(DIVISION)
            .unwrap_err()
            .contains("supply-days define is zero")
    );
}

#[test]
fn a_strategic_redeployment_moves_at_the_road_or_rail_speed() {
    use replay_data::{
        rows::RailCooldownRow, rows::RailEdgeRow, rows::RailwayBuildingHealth,
        rows::RailwayBuildingState, rows::StrategicAction, rows::StrategicActionFailure,
        rows::StrategicRedeploymentInputs,
    };
    let mut map = content(two_leg_map());
    map.defines.rail = Some(eta_derive::RailDefines {
        speed: [500_000, 1_000_000, 1_000_000, 2_500_000],
        maximum_level: 5,
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    state: if id == 3 { 9 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::States(vec![StateRow {
            id: 9,
            railway_infrastructure: Recorded::Value(Some(50_000)),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::RailCooldowns(vec![RailCooldownRow {
            province: 1,
            cooldown_ticks: 0,
            building: Recorded::Value(Some(RailwayBuildingState {
                building: Some(RailwayBuildingHealth {
                    level: 5,
                    healthy: 5,
                    partial: None,
                }),
            })),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::RailEdges(vec![RailEdgeRow {
            from: 1,
            to: 2,
            level: 5,
        }]),
    );
    let mut row = moving();
    row.strategic_redeployment = 1;
    row.strategic_action = Recorded::Value(StrategicAction::Available(
        StrategicRedeploymentInputs::default(),
    ));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.estimate(DIVISION), two_legs(3_500_000, 1_000_000));
    assert_eq!(
        restricted_estimate(&frame, &map),
        two_legs(3_500_000, 1_000_000).map_err(str::to_owned)
    );
    let unspeeded = |frame: &mut DecodedFrame, row: &DivisionMovementRow| {
        put(frame, TableRows::DivisionMovement(vec![row.clone()]));
        put(frame, TableRows::DivisionPanelStats(vec![panel(DIVISION, None)]));
        let estimate = Etas::read(frame, Some(&map))
            .unwrap()
            .estimate(DIVISION)
            .map_err(str::to_owned);
        put(frame, TableRows::DivisionPanelStats(vec![panel(DIVISION, Some(SPEED))]));
        estimate
    };
    assert_eq!(
        unspeeded(&mut frame, &row),
        two_legs(3_500_000, 1_000_000).map_err(str::to_owned)
    );
    row.strategic_action =
        Recorded::Value(StrategicAction::Available(StrategicRedeploymentInputs {
            country_modifier: 1,
            ..Default::default()
        }));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let blocked = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(blocked.estimate(DIVISION), two_legs(SPEED, SPEED));
    assert_eq!(unspeeded(&mut frame, &row), Err(UNRECORDED.to_owned()));
    row.strategic_action = Recorded::Value(StrategicAction::Withheld(
        StrategicActionFailure::CountryModifier,
    ));
    put(&mut frame, TableRows::DivisionMovement(vec![row]));
    let withheld = Etas::read(&frame, Some(&map)).unwrap();
    assert!(
        withheld
            .estimate(DIVISION)
            .unwrap_err()
            .contains("owner-country modifier unavailable")
    );
}

fn restricted_estimate(frame: &DecodedFrame, map: &MovementContent) -> Result<(i64, i64), String> {
    use replay_data::rows::TableId;
    let mut restricted = frame.clone();
    for table in TableId::ALL {
        if !matches!(table, TableId::Divisions | TableId::DivisionMovement)
            && !replay_derive::eta_derive::frame_terms::TABLES.contains(&table)
        {
            *restricted.tables[table.index()] = TableView::Unavailable {
                reason: "not selected".into(),
            };
        }
    }
    Etas::read(&restricted, Some(map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned)
}

fn sub_unit_database() -> std::sync::Arc<content_reader_script::sub_units::SubUnits> {
    crate::test_support::enter_fixed_modifier_tables();
    let mut units = content_reader_script::sub_units::SubUnits::default();
    units.read(
        b"sub_units={ light_armor={type=armor} marine={special_forces=yes type=infantry} infantry={type=infantry} }",
        "common/units/test.txt",
    );
    std::sync::Arc::new(units)
}

const ARMOUR: u32 = 900_001;
const MARINE: u32 = 900_002;
const INFANTRY: u32 = 900_003;

fn template(frame: &mut DecodedFrame, priority: &[u8], units: &[(u32, i32)]) {
    put(
        frame,
        TableRows::DivisionTemplateIcons(vec![replay_data::rows::DivisionTemplateIconRow {
            template: 0x42,
            priority_unit_name: priority.to_vec(),
            subunits: units
                .iter()
                .map(|&(token, count)| replay_data::rows::TemplateSubunit {
                    token,
                    local: 0,
                    count,
                })
                .collect(),
            speed_keys: Recorded::Value(Some(Vec::new())),
            ..Default::default()
        }]),
    );
    let mut tokens = frame
        .complete::<replay_data::rows::tables::LexicalTokens>()
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    tokens.retain(|row| row.token < ARMOUR);
    tokens.extend(
        [
            (ARMOUR, &b"light_armor"[..]),
            (MARINE, b"marine"),
            (INFANTRY, b"infantry"),
        ]
        .into_iter()
        .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        }),
    );
    put(frame, TableRows::LexicalTokens(tokens));
}

#[test]
fn the_special_forces_factor_applies_at_the_template_share() {
    let mut map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    map.land = Ok(LandSpeedContent {
        terrain_movement: vec![0],
        regions: BTreeMap::new(),
        sub_units: Ok(sub_unit_database()),
    });
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            roster_owner: 1,
            logical_owner: 1,
            template: 0x42,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            supply_ratio: None,
            out_of_supply_days: Recorded::Value(Some(10)),
            supply_ratio_exact: Recorded::Value(Some(50_000)),
            supply_grace_exact: Recorded::Value(Some(0)),
            ..status_row(DIVISION)
        }]),
    );
    let factors = |frame: &mut DecodedFrame, country: Option<i64>, special: Option<i64>| {
        put(
            frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                    out_of_supply_factor: country,
                    special_forces_out_of_supply_factor: special,
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
    };
    template(&mut frame, b"marine", &[(MARINE, 3), (INFANTRY, 1)]);
    factors(&mut frame, None, Some(-50_000));
    let special = Etas::read(&frame, Some(&map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned);
    assert!(special.is_ok(), "{special:?}");
    factors(&mut frame, Some(-37_500), None);
    let country = Etas::read(&frame, Some(&map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned);
    assert_eq!(special, country);
    let unreadable = |frame: &mut DecodedFrame| {
        let mut rows = frame
            .complete::<replay_data::rows::tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        rows[0].model_unreadable = 1;
        put(frame, TableRows::DivisionTemplateIcons(rows));
    };
    factors(&mut frame, None, Some(-50_000));
    let mut refused = frame.clone();
    unreadable(&mut refused);
    let reason = Etas::read(&refused, Some(&map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned);
    assert!(
        reason
            .as_ref()
            .unwrap_err()
            .contains("composition was unreadable"),
        "{reason:?}"
    );
    assert_eq!(restricted_estimate(&frame, &map), special);
    template(&mut frame, b"infantry", &[(INFANTRY, 4)]);
    factors(&mut frame, None, Some(-50_000));
    let none = Etas::read(&frame, Some(&map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned);
    factors(&mut frame, None, None);
    let neither = Etas::read(&frame, Some(&map))
        .unwrap()
        .estimate(DIVISION)
        .map_err(str::to_owned);
    assert_eq!(none, neither);
    assert_ne!(none, special);
}

#[test]
fn an_armoured_priority_sub_unit_with_a_commander_takes_the_armour_speed() {
    let mut map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    map.land = Ok(LandSpeedContent {
        terrain_movement: vec![0, 0],
        regions: BTreeMap::new(),
        sub_units: Ok(sub_unit_database()),
    });
    commanded(&mut frame, vec![0, 0], [None; 5]);
    let division = |frame: &mut DecodedFrame, group: u64| {
        put(
            frame,
            TableRows::Divisions(vec![DivisionRow {
                id: DIVISION,
                province: 1,
                roster_owner: 1,
                logical_owner: 1,
                orders_group: group,
                template: 0x42,
                ..Default::default()
            }]),
        );
    };
    division(&mut frame, 5);
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 1,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                army_armor_speed_factor: Some(20_000),
                ..Default::default()
            })),
            ..Default::default()
        }]),
    );
    template(&mut frame, b"light_armor", &[(ARMOUR, 2)]);
    let fast = SPEED * 120_000 / 100_000;
    let armoured = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(armoured.estimate(DIVISION), two_legs(fast, fast));
    assert_eq!(
        restricted_estimate(&frame, &map),
        two_legs(fast, fast).map_err(str::to_owned)
    );
    template(&mut frame, b"infantry", &[(INFANTRY, 2)]);
    let infantry = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(infantry.estimate(DIVISION), two_legs(SPEED, SPEED));
    template(&mut frame, b"light_armor", &[(ARMOUR, 2)]);
    division(&mut frame, 0);
    let leaderless = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(leaderless.estimate(DIVISION), two_legs(SPEED, SPEED));
}

#[test]
fn deployed_panel_speed_uses_stationary_cache_fuel_and_native_scale_round_trip() {
    let mut frame = hour(None);
    let mut map = content(two_leg_map());
    map.defines.outer_scale = 7_777;
    let status = status_row(DIVISION);
    let mut panel = DivisionPanelStatsRow {
        division: DIVISION,
        ..Default::default()
    };
    panel.panel_stats[21] = Some(333_333);
    panel.panel_stats[1] = Some(100_000);
    put(&mut frame, TableRows::DivisionPanelStats(vec![panel]));
    put(&mut frame, TableRows::DivisionStatus(vec![status]));
    let unit = frame.complete::<tables::Divisions>().unwrap()[0].clone();
    let movement = DivisionMovementRow {
        division: DIVISION,
        fuel: Recorded::Value(Some(0)),
        ..Default::default()
    };
    let expected = (333_333_i64 * 7_777 / 100_000) * 100_000 / 7_777;
    assert_eq!(
        HourTerms::new(&frame)
            .panel_speed(
                &unit,
                &movement,
                &map,
                &map.defines,
                Err("no combat should be read".into())
            )
            .unwrap(),
        expected
    );
    let embarked = DivisionMovementRow {
        naval_transfer: 1,
        transport_speed: Recorded::Value(Some(123_456)),
        ..movement
    };
    map.defines.transport_scale = 89_999;
    let expected = (123_456_i64 * 89_999 / 100_000) * 100_000 / 7_777;
    assert_eq!(
        HourTerms::new(&frame)
            .panel_speed(
                &unit,
                &embarked,
                &map,
                &map.defines,
                Err("no combat should be read".into())
            )
            .unwrap(),
        expected
    );
}

#[test]
fn deployed_panel_width_uses_effective_template_and_both_context_factors() {
    let mut frame = hour(None);
    let mut unit = frame.complete::<tables::Divisions>().unwrap()[0].clone();
    unit.template = 91;
    put(
        &mut frame,
        TableRows::DivisionTemplateIcons(vec![replay_data::rows::DivisionTemplateIconRow {
            template: 91,
            base_width: Recorded::Value(Some(1_234_567)),
            ..Default::default()
        }]),
    );
    let mut status = status_row(DIVISION);
    status.commander_modifiers = Recorded::Value(Some([None; 5]));
    status.commander_width_factor = Recorded::Value(Some(-10_000));
    put(&mut frame, TableRows::DivisionStatus(vec![status]));
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 1,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers::default())),
            combat_width_factor: Recorded::Value(Some(25_000)),
            ..Default::default()
        }]),
    );
    assert_eq!(
        HourTerms::new(&frame).panel_width(&unit).unwrap(),
        1_234_567_i64 * 115_000 / 100_000
    );
    let original = frame.clone();
    for (country_missing, packet_refused) in
        [(true, false), (false, false), (true, true), (false, true)]
    {
        frame = original.clone();
        if country_missing {
            let mut row = frame.complete::<tables::CountryCounterInputs>().unwrap()[0].clone();
            if packet_refused {
                row.movement_modifiers = Recorded::Value(None);
            } else {
                row.combat_width_factor = Recorded::NotRecorded;
            }
            put(&mut frame, TableRows::CountryCounterInputs(vec![row]));
        } else {
            let mut row = frame.complete::<tables::DivisionStatus>().unwrap()[0].clone();
            if packet_refused {
                row.commander_modifiers = Recorded::Value(None);
            } else {
                row.commander_width_factor = Recorded::NotRecorded;
            }
            put(&mut frame, TableRows::DivisionStatus(vec![row]));
        }
        assert!(HourTerms::new(&frame).panel_width(&unit).is_err());
    }
}

fn two_leg_map_to(third: ProvinceTerrain) -> MovementMap {
    let edge = |to, distance| Neighbour {
        to,
        distance: Some(distance),
        ..Default::default()
    };
    MovementMap::from_parts(&[
        (1, LAND, vec![edge(2, 350_000)]),
        (2, LAND, vec![edge(1, 350_000), edge(3, 700_000)]),
        (3, third, vec![edge(2, 700_000)]),
    ])
}

#[test]
fn a_move_by_sea_takes_the_logical_owners_transport_speed() {
    use replay_data::rows::OwnerTransportSpeedRow;
    let mut map = content(two_leg_map());
    map.defines.transport_scale = 50_000;
    map.land = Ok(LandSpeedContent {
        terrain_movement: vec![0],
        regions: BTreeMap::from([(1, 70), (2, 71), (3, 71)]),
        ..Default::default()
    });
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            roster_owner: 1,
            logical_owner: 4,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 4,
                canonical_slot: 4,
            },
        ]),
    );
    put(&mut frame, TableRows::WarRelations(vec![]));
    let speeds = |frame: &mut DecodedFrame, second: Option<i64>| {
        put(
            frame,
            TableRows::DivisionTransportSpeeds(vec![
                OwnerTransportSpeedRow {
                    owner: 4,
                    region: 70,
                    speed: Some(800_000),
                },
                OwnerTransportSpeedRow {
                    owner: 4,
                    region: 71,
                    speed: second,
                },
            ]),
        );
    };
    speeds(&mut frame, Some(400_000));
    let mut row = moving();
    row.naval_transfer = 1;
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(400_000, 200_000).unwrap())
    );
    let mut transfer = frame.clone();
    let mut transfer_row = row.clone();
    transfer_row.withdrawal = Recorded::Value(None);
    put(
        &mut transfer,
        TableRows::DivisionMovement(vec![transfer_row]),
    );
    for table in replay_derive::eta_derive::frame_terms::TABLES {
        if table != replay_data::rows::TableId::DivisionTransportSpeeds {
            *transfer.tables[table.index()] = TableView::Unavailable {
                reason: "unreached land input".into(),
            };
        }
    }
    assert_eq!(
        restricted_estimate(&transfer, &map),
        Ok(two_legs(400_000, 200_000).unwrap())
    );
    {
        let mut unspeeded = frame.clone();
        put(
            &mut unspeeded,
            TableRows::DivisionPanelStats(vec![panel(DIVISION, None)]),
        );
        assert_eq!(
            restricted_estimate(&unspeeded, &map),
            Ok(two_legs(400_000, 200_000).unwrap())
        );
    }
    speeds(&mut frame, None);
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("transport speed was refused")
    );
    speeds(&mut frame, Some(400_000));
    map.map = two_leg_map_to(ProvinceTerrain { flags: 0, index: 1 });
    row.naval_transfer = 0;
    put(&mut frame, TableRows::DivisionMovement(vec![row]));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, 200_000).unwrap())
    );
    put(&mut frame, TableRows::DivisionTransportSpeeds(vec![]));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Err(replay_derive::eta_derive::frame_terms::TRANSPORT_NOT_RECORDED.to_owned())
    );
}

#[test]
fn a_division_in_combat_starts_at_the_combat_movement_base() {
    let mut map = content(two_leg_map());
    map.defines.combat_movement = Some(33_000);
    let mut frame = hour(Some(SPEED));
    let combat = |frame: &mut DecodedFrame, entry: Option<Option<i64>>| {
        put(
            frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                combat_movement: Recorded::Value(entry),
                ..status_row(DIVISION)
            }]),
        );
    };
    let mut row = moving();
    row.combat_count = 1;
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    combat(&mut frame, Some(Some(-13_000)));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(140_000, 140_000).unwrap())
    );
    combat(&mut frame, Some(None));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(100_000, 100_000).unwrap())
    );
    combat(&mut frame, None);
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("combat movement operand was refused")
    );
    combat(&mut frame, Some(Some(-13_000)));
    row.withdrawal = Recorded::Value(Some(1));
    put(&mut frame, TableRows::DivisionMovement(vec![row]));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(350_000, 350_000).unwrap())
    );
}

#[test]
fn the_location_state_adds_its_speed_entries() {
    use replay_data::{rows::MovementStateTermRow, rows::StateMovementTerms};
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    state: if id == 1 { 9 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let state = |frame: &mut DecodedFrame, branch: u8| {
        put(
            frame,
            TableRows::MovementStateTerms(vec![MovementStateTermRow {
                state: 9,
                roster: 1,
                owner: 1,
                terms: Some(StateMovementTerms {
                    army_speed: Some(-20_000),
                    attack_speed: None,
                    relation_speed: Some(if branch == StateMovementTerms::FRIENDLY {
                        10_000
                    } else {
                        -30_000
                    }),
                    branch,
                }),
            }]),
        );
    };
    state(&mut frame, StateMovementTerms::FRIENDLY);
    let slowed = SPEED * 90_000 / 100_000;
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(slowed, slowed).unwrap())
    );
    state(&mut frame, StateMovementTerms::ENEMY);
    let slowed = SPEED * 50_000 / 100_000;
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(slowed, slowed).unwrap())
    );
    put(&mut frame, TableRows::MovementStateTerms(vec![]));
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("MovementStateTerms has no row")
    );
}

#[test]
fn the_priority_sub_unit_and_category_entries_add_with_repeats() {
    use replay_data::{rows::CountryUnitSpeedEntryRow, rows::DivisionTemplateIconRow};
    let (a, b) = (
        engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
        engine_executable::modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
    );
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: 0,
            speed_keys: Recorded::Value(Some(vec![a, b, a])),
            ..Default::default()
        }]),
    );
    let mut entries = vec![
        CountryUnitSpeedEntryRow {
            country: 1,
            key: a,
            entry: Some(Some(10_000)),
        },
        CountryUnitSpeedEntryRow {
            country: 1,
            key: b,
            entry: Some(None),
        },
    ];
    entries.sort_unstable_by_key(|row| (row.country, row.key));
    put(
        &mut frame,
        TableRows::CountryUnitSpeedEntries(entries.clone()),
    );
    let fast = SPEED * 120_000 / 100_000;
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(fast, fast).unwrap())
    );
    entries.retain(|row| row.key != b);
    put(&mut frame, TableRows::CountryUnitSpeedEntries(entries));
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("CountryUnitSpeedEntries has no row")
    );
}

#[test]
fn an_owner_without_core_supply_access_adds_the_noncore_supply_speed() {
    use replay_data::rows::CoreSupplyAccessRow;
    let mut map = content(two_leg_map());
    map.defines.noncore_supply = [Some(-50_000), Some(-25_000)];
    let mut frame = hour(Some(SPEED));
    let access = |frame: &mut DecodedFrame, access: Option<bool>| {
        put(
            frame,
            TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow {
                owner: 1,
                province: 1,
                access,
            }]),
        );
    };
    let flags = |frame: &mut DecodedFrame, move_flags: u8| {
        put(
            frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                move_flags,
                ..status_row(DIVISION)
            }]),
        );
    };
    access(&mut frame, Some(true));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, SPEED).unwrap())
    );
    access(&mut frame, Some(false));
    let half = SPEED * 50_000 / 100_000;
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(half, half).unwrap())
    );
    flags(&mut frame, DivisionStatusRow::MOVE_NONCORE_AIR);
    let air = SPEED * 75_000 / 100_000;
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(air, air).unwrap())
    );
    for bypass in [
        DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS,
        DivisionStatusRow::MOVE_EXILED,
    ] {
        flags(&mut frame, bypass);
        assert_eq!(
            restricted_estimate(&frame, &map),
            Ok(two_legs(SPEED, SPEED).unwrap()),
            "{bypass}"
        );
    }
    flags(&mut frame, 0);
    access(&mut frame, None);
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("core supply access was refused")
    );
}

#[test]
fn a_relations_river_crossing_factor_scales_a_crossing() {
    use replay_data::rows::RiverRelationRow;
    let mut map = content(river_map());
    map.defines.river_crossing_two = -30_000;
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: if id == 3 { 2 } else { 1 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
        ]),
    );
    put(&mut frame, TableRows::WarRelations(vec![]));
    let relation = |frame: &mut DecodedFrame, entry: Option<i64>| {
        put(
            frame,
            TableRows::RiverRelations(vec![RiverRelationRow {
                owner: 1,
                controller: 2,
                entry: Some(entry),
            }]),
        );
    };
    relation(&mut frame, Some(50_000));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, SPEED * 55_000 / 100_000).unwrap())
    );
    relation(&mut frame, None);
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, SPEED * 70_000 / 100_000).unwrap())
    );
    put(&mut frame, TableRows::RiverRelations(vec![]));
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("RiverRelations has no row")
    );
}

#[test]
fn failed_air_supply_refuses_only_reached_noncore_movement() {
    use replay_data::rows::CoreSupplyAccessRow;
    let mut map = content(two_leg_map());
    map.defines.noncore_supply = [Some(-50_000), Some(-25_000)];
    let mut frame = hour(Some(SPEED));
    let access = |frame: &mut DecodedFrame, value| {
        put(
            frame,
            TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow {
                owner: 1,
                province: 1,
                access: Some(value),
            }]),
        )
    };
    access(&mut frame, false);
    let status = |frame: &mut DecodedFrame, flags| {
        put(
            frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                move_flags: flags | DivisionStatusRow::MOVE_NONCORE_AIR_REFUSED,
                ..status_row(DIVISION)
            }]),
        )
    };
    status(&mut frame, 0);
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("air-supply sign was refused")
    );
    for flags in [
        DivisionStatusRow::MOVE_EXILED,
        DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS,
    ] {
        status(&mut frame, flags);
        assert_eq!(
            restricted_estimate(&frame, &map),
            Ok(two_legs(SPEED, SPEED).unwrap())
        );
    }
    status(&mut frame, 0);
    access(&mut frame, true);
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, SPEED).unwrap())
    );
}

#[test]
fn a_failed_shared_state_attack_entry_only_refuses_hostile_movement() {
    use replay_data::{rows::MovementStateTermRow, rows::StateMovementTerms};
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    let provinces = |frame: &mut DecodedFrame, war| {
        hostile_destination(frame, war);
        let mut rows = frame
            .complete::<replay_data::rows::tables::Provinces>()
            .unwrap()
            .to_vec();
        rows[0].state = 9;
        put(frame, TableRows::Provinces(rows));
    };
    provinces(&mut frame, false);
    put(
        &mut frame,
        TableRows::MovementStateTerms(vec![MovementStateTermRow {
            state: 9,
            roster: 1,
            owner: 1,
            terms: Some(StateMovementTerms {
                branch: StateMovementTerms::ATTACK_REFUSED,
                ..Default::default()
            }),
        }]),
    );
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(SPEED, SPEED).unwrap())
    );
    provinces(&mut frame, true);
    assert!(
        restricted_estimate(&frame, &map)
            .unwrap_err()
            .contains("hostile-destination attack speed was refused")
    );
}

#[test]
fn lake_departure_keeps_later_ordinary_land_speed_and_exact_organisation() {
    use replay_data::rows::OwnerTransportSpeedRow;
    let edge = |to| Neighbour {
        to,
        distance: Some(350_000),
        ..Default::default()
    };
    let mut map = content(MovementMap::from_parts(&[
        (1, ProvinceTerrain { flags: 2, index: 1 }, vec![edge(2)]),
        (2, LAND, vec![edge(3)]),
        (3, LAND, vec![]),
    ]));
    map.defines.transport_scale = 100_000;
    map.defines.invasion_speed = 900_000;
    map.defines.organisation = Some(OrganisationDefines {
        threshold: 30_000,
        penalty: -80_000,
    });
    map.land.as_mut().unwrap().regions.insert(1, 70);
    let mut frame = hour(Some(SPEED));
    hostile_destination(&mut frame, true);
    put(
        &mut frame,
        TableRows::RaidAirScores(vec![replay_data::rows::RaidAirScoreRow {
            country: 1,
            region: 70,
            friendly: 0,
            enemy: 0,
            impact: Recorded::Value(0),
            ..Default::default()
        }]),
    );
    let mut movement = moving();
    movement.numerator = 0;
    put(&mut frame, TableRows::DivisionMovement(vec![movement]));
    put(
        &mut frame,
        TableRows::DivisionTransportSpeeds(vec![OwnerTransportSpeedRow {
            owner: 1,
            region: 70,
            speed: Some(700_000),
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            organisation_exact: Recorded::Value(Some(0)),
            maximum_organisation_exact: Recorded::Value(Some(1_000_000)),
            move_flags: DivisionStatusRow::MOVE_LOW_ORGANISATION,
            ..status_row(DIVISION)
        }]),
    );
    let division = frame
        .complete::<replay_data::rows::tables::Divisions>()
        .unwrap()[0]
        .clone();
    let movement = &frame
        .complete::<replay_data::rows::tables::DivisionMovement>()
        .unwrap()[0];
    let terms = replay_derive::eta_derive::frame_terms::FrameTerms::new(&frame);
    let speed = terms
        .with_terms(&division, movement, &map, &map.defines, |terms| {
            replay_derive::eta_derive::edge_speed(SPEED, 2, 3, terms, &map.defines)
        })
        .unwrap();
    assert_eq!(speed, 140_000);
}

#[test]
fn actual_first_self_speed_is_kept_when_the_itinerary_omits_its_distance() {
    let mut map = content(two_leg_map_to(ProvinceTerrain { flags: 0, index: 1 }));
    map.defines.transport_scale = 100_000;
    map.land.as_mut().unwrap().regions.insert(1, 70);
    let mut frame = hour(Some(SPEED));
    put(&mut frame, TableRows::WarRelations(vec![]));
    assert!(
        frame
            .complete::<replay_data::rows::tables::RaidAirScores>()
            .is_err()
    );
    let mut movement = moving();
    movement.path = vec![1, 3];
    movement.path_total = 2;
    put(
        &mut frame,
        TableRows::DivisionTransportSpeeds(vec![replay_data::rows::OwnerTransportSpeedRow {
            owner: 1,
            region: 70,
            speed: Some(500_000),
        }]),
    );
    let division = frame
        .complete::<replay_data::rows::tables::Divisions>()
        .unwrap()[0]
        .clone();
    let terms = replay_derive::eta_derive::frame_terms::FrameTerms::new(&frame);
    let speed = terms
        .with_route_terms(&division, &movement, &[3], &map, &map.defines, |terms| {
            Ok(terms.modifier)
        })
        .unwrap();
    assert_eq!(speed, 100_000);
    assert!(
        frame
            .complete::<replay_data::rows::tables::RaidAirScores>()
            .is_err()
    );
}

#[test]
fn noncore_supply_uses_only_the_selected_ground_or_air_define() {
    use replay_data::rows::CoreSupplyAccessRow;
    let mut frame = hour(Some(SPEED));
    put(
        &mut frame,
        TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow {
            owner: 1,
            province: 1,
            access: Some(false),
        }]),
    );
    for air in [false, true] {
        let mut map = content(two_leg_map());
        let index = usize::from(air);
        let factor = if air { -25_000 } else { -50_000 };
        map.defines.noncore_supply = [None; 2];
        map.defines.noncore_supply[index] = Some(factor);
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                move_flags: if air {
                    DivisionStatusRow::MOVE_NONCORE_AIR
                } else {
                    0
                },
                ..status_row(DIVISION)
            }]),
        );
        let speed = SPEED * (100_000 + factor) / 100_000;
        assert_eq!(
            restricted_estimate(&frame, &map),
            Ok(two_legs(speed, speed).unwrap())
        );
        map.defines.noncore_supply.swap(0, 1);
        let name = if air { "air noncore" } else { "ground noncore" };
        assert!(
            restricted_estimate(&frame, &map)
                .unwrap_err()
                .contains(name)
        );
    }
}

#[test]
fn worker_ticket_etas_match_direct_derivation_across_hours_branches_and_refusals() {
    let cache = EtaCache::default();
    let map = content(two_leg_map());
    let mut scenarios = Vec::new();
    for (branch, scale) in [(1, 1), (2, 3)] {
        for (at, speed) in [Some(SPEED * scale), None, None, Some(SPEED * 2 * scale)]
        .into_iter()
        .enumerate()
        {
            let mut frame = hour(speed);
            frame.branch = branch;
            frame.hour += at as i32;
            scenarios.push((frame, true));
        }
    }
    assert_ne!(
        Etas::read(&scenarios[0].0, Some(&map)).unwrap().by_division,
        Etas::read(&scenarios[4].0, Some(&map)).unwrap().by_division
    );
    let mut empty = frame();
    empty.hour += 10;
    put(
        &mut empty,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: DIVISION,
            path_total: 0,
            ..Default::default()
        }]),
    );
    scenarios.push((empty, false));
    let mut missing = frame();
    missing.hour += 20;
    scenarios.push((missing, false));
    let mut partial = hour(Some(SPEED));
    partial.hour += 21;
    *partial.tables[replay_data::rows::TableId::DivisionMovement.index()] = TableView::Truncated {
        rows: TableRows::DivisionMovement(vec![moving()]),
        missing: 1,
    };
    scenarios.push((partial, true));
    let mut no_content = hour(Some(SPEED));
    no_content.hour += 22;
    scenarios.push((no_content, false));
    let mut malformed = frame();
    malformed.hour += 23;
    *malformed.tables[replay_data::rows::TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::Countries(Vec::new()));
    scenarios.push((malformed, false));
    for (frame, use_content) in scenarios {
        let movement = use_content.then_some(&map);
        let expected = Etas::read(&frame, movement);
        let ticket = WorkerEtas::default();
        for _ in 0..2 {
            let result = ticket.get_or_try_init(|| cache.etas(&frame, movement));
            match (&expected, result) {
                (Ok(expected), Ok(result)) => {
                    assert_eq!(result.by_division, expected.by_division);
                    assert_eq!(result.estimate_gaps, expected.estimate_gaps);
                    assert_eq!(result.progress, expected.progress);
                    assert_eq!(result.progress_gaps, expected.progress_gaps);
                    assert_eq!(result.report, expected.report);
                    assert_eq!(result.report.line(), expected.report.line());
                    for id in [0, DIVISION, u64::MAX] {
                        assert_eq!(arrival(&result, id, true), arrival(expected, id, true));
                        assert_eq!(arrival(&result, id, false), arrival(expected, id, false));
                    }
                }
                (Err(expected), Err(error)) => {
                    assert_eq!(error.kind(), expected.kind());
                    assert_eq!(error.to_string(), expected.to_string());
                }
                _ => panic!("worker ticket changed ETA success or refusal"),
            }
        }
    }
}

#[test]
fn zero_supply_days_uses_exact_operands_at_zero_days_and_proves_historical_full_supply() {
    let map = content(two_leg_map());
    let mut base = hour(Some(SPEED));
    put(
        &mut base,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(RecordedDefines {
                out_of_supply_speed: Some(-80_000),
                retreat_speed: Some(25_000),
                withdrawal_speed: Some(-50_000),
                out_of_fuel_speed: Some(40_000),
                fuel_capacity_scale: Some(9_600_000),
                supply_days: Some(0),
                supply_grace: Some(72),
                air_speed: Some(-50_000),
                air_defense_denominator: Some(1_000_000),
                air_defense_reduction: Some(100_000),
                out_of_supply_attrition: Some(20_000),
            }),
            ..Default::default()
        }]),
    );
    let run = |row: DivisionStatusRow| {
        let mut frame = base.clone();
        put(&mut frame, TableRows::DivisionStatus(vec![row]));
        Etas::read(&frame, Some(&map))
            .unwrap()
            .estimate(DIVISION)
            .map_err(str::to_owned)
    };
    let exact = run(DivisionStatusRow {
        supply_ratio: None,
        out_of_supply_days: Recorded::Value(Some(0)),
        supply_ratio_exact: Recorded::Value(Some(35_500)),
        supply_grace_exact: Recorded::Value(Some(0)),
        ..status_row(DIVISION)
    });
    assert!(exact.is_ok(), "{exact:?}");
    let historical_ambiguous = run(DivisionStatusRow {
        supply_ratio: Some(35_000),
        out_of_supply_days: Recorded::Value(Some(0)),
        supply_ratio_exact: Recorded::Value(None),
        supply_grace_exact: Recorded::Value(None),
        ..status_row(DIVISION)
    });
    assert_eq!(
        historical_ambiguous.as_ref().map_err(String::as_str),
        Err(replay_derive::eta_derive::frame_terms::SUPPLY_DAYS_ZERO)
    );
    let historical_full = run(DivisionStatusRow {
        supply_ratio: Some(100_000),
        out_of_supply_days: Recorded::Value(Some(0)),
        supply_ratio_exact: Recorded::Value(None),
        supply_grace_exact: Recorded::Value(None),
        ..status_row(DIVISION)
    });
    assert!(historical_full.is_ok(), "{historical_full:?}");
    let historical_no_grace_max = run(DivisionStatusRow {
        supply_ratio: Some(35_000),
        supply_grace_maximum: Recorded::Value(0),
        out_of_supply_days: Recorded::Value(Some(0)),
        supply_ratio_exact: Recorded::Value(None),
        supply_grace_exact: Recorded::Value(None),
        ..status_row(DIVISION)
    });
    assert!(historical_no_grace_max.is_ok(), "{historical_no_grace_max:?}");
}
