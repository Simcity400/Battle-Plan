use super::*;
use installed_content_core::{movement::Neighbour, movement::ProvinceTerrain};
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryMovementModifiers,
    rows::DivisionMovementRow, rows::DivisionMovementStats, rows::DivisionPanelStatsRow,
    rows::DivisionRow,
    rows::DivisionStatusRow, rows::MovementDefines as RecordedDefines, rows::ProvinceRow,
    rows::Recorded, rows::SimulationFeatureRow, rows::StateRow, rows::TableRows,
    rows::WarRelationRow,
};
use replay_derive::eta_derive::SpeedTerms;
use replay_frame::{frame::Metrics, frame::TableView};
use std::cell::Cell;

const DIVISION: u64 = 7;
/// A base speed whose expected pair is arithmetic at movement scale 1.0.
const SPEED: i64 = 700_000;
const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };
const UNRECORDED: &str = "the division's base speed is not recorded";

fn content(map: MovementMap) -> MovementContent {
    MovementContent {
        map,
        defines: Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
        rail: Ok(RailMovementDefines {
            speed: [0; 4],
            maximum_level: 0,
        }),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        // One terrain category without its own movement; no province has a region.
        land: Ok(LandSpeedContent {
            terrain_movement: vec![0],
            regions: BTreeMap::new(),
            ..Default::default()
        }),
    }
}

/// The edge speed of `base` with no term but `organisation`.
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

/// A status row whose speed terms are all neutral on the one terrain category.
fn status_row(division: u64) -> DivisionStatusRow {
    test_support::neutral_status(division, 1)
}

/// The shared rows every estimate reads, for country 1.
fn world(frame: &mut DecodedFrame) {
    test_support::neutral_world(frame, 1);
}

/// Land provinces 1, 2 and 3 with the route edges 1-2 (350000) and 2-3 (700000).
fn two_leg_map() -> MovementMap {
    let edge = |to, distance| Neighbour {
        to,
        distance: Some(distance),
        ..Default::default()
    };
    MovementMap::from_parts(&[
        (1, LAND, vec![edge(2, 350_000)]),
        (2, LAND, vec![edge(1, 350_000), edge(3, 700_000)]),
        (3, LAND, vec![edge(2, 700_000)]),
    ])
}

/// Every table unavailable: an estimate reads no weather or speed-graph row.
fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 60_832_632,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the estimate".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    }
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

fn moving() -> DivisionMovementRow {
    DivisionMovementRow {
        division: DIVISION,
        path: vec![2, 3],
        path_total: 2,
        numerator: 175_000,
        withdrawal: Recorded::Value(Some(0)),
        fuel: Recorded::Value(Some(0)),
        ..Default::default()
    }
}

/// `division`'s panel row: `speed` its stat block's maximum speed, and a maximum organisation
/// of 10.
fn panel(division: u64, speed: Option<i64>) -> DivisionPanelStatsRow {
    let mut panel = DivisionPanelStatsRow {
        division,
        ..Default::default()
    };
    panel.panel_stats[DivisionPanelStatsRow::MAXIMUM_SPEED] = speed;
    panel.panel_stats[1] = Some(1_000_000);
    panel
}

/// Stateless, uncontrolled provinces `ids`: no infrastructure term, no hostile move.
fn provinces(ids: impl IntoIterator<Item = u32>) -> TableRows {
    TableRows::Provinces(
        ids.into_iter()
            .map(|id| ProvinceRow {
                id,
                ..Default::default()
            })
            .collect(),
    )
}

/// One division at province 1 on the two-leg route 2, 3 with `base_speed`.
fn hour(base_speed: Option<i64>) -> DecodedFrame {
    let mut frame = frame();
    put(&mut frame, provinces(1..=3));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            roster_owner: 1,
            logical_owner: 1,
            ..Default::default()
        }]),
    );
    put(&mut frame, TableRows::DivisionMovement(vec![moving()]));
    put(
        &mut frame,
        TableRows::DivisionPanelStats(vec![panel(DIVISION, base_speed)]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![status_row(DIVISION)]),
    );
    world(&mut frame);
    frame
}

/// A busy hour: `provinces` land provinces with no authored neighbours, so
/// every hop takes the native 100000 fallback, and `divisions` movers each
/// on a `legs`-hop route at the same base speed.
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

/// Without an infrastructure term or a hostile move, the estimate is the remaining route
/// distance over the edge speed of the base speed: this hour holds no weather, state or
/// speed-graph rows.
#[test]
fn an_estimate_is_the_remaining_route_distance_over_the_edge_speed() {
    let map = content(two_leg_map());
    for (base, speed) in [(SPEED, SPEED), (350_000, 350_000), (1, 100_000)] {
        assert_eq!(plain_speed(base, None, &map.defines), speed);
        let etas = Etas::read(&hour(Some(base).into()), Some(&map)).unwrap();
        // Legs 350000 and 700000, less the 175000 numerator already moved.
        let leg = |distance: i64| distance * 100_000 / speed;
        let progress = leg(175_000);
        let next = leg(350_000) - progress;
        let destination = leg(350_000) + leg(700_000) - progress;
        assert_eq!(etas.get(DIVISION), Some((next, destination)), "{base}");
        assert_eq!(etas.estimate(DIVISION), Ok((next, destination)));
        // Progress divides the numerator by the first edge's content distance.
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

/// A recording older than the base speed keeps progress and refuses only
/// the estimate, by the replay estimate's own name.
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
    // No census row: neither progress nor estimate.
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    put(&mut frame, TableRows::Divisions(vec![]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.estimate(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.progress(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.report().progress_refused, 1);

    // A province absent from the content map is missing geometry, never the
    // null terrain or an ordinary land province.
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

    // Content that failed to load refuses every moving division, but a
    // division without a route still stands at progress zero.
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
    // A worker builds the hours ahead and holds them; the window then arrives at them.
    let ahead: Vec<_> = (1..=3).map(fill).collect();
    assert_eq!(built.get(), 3);
    fill(1);
    fill(2);
    assert_eq!(built.get(), 3, "the look-ahead must still be cached");
    drop(ahead);
    // Nothing holds hour 1 once hour 4 is the latest: it derives again.
    fill(4);
    fill(4);
    assert_eq!(built.get(), 4);
    fill(1);
    assert_eq!(built.get(), 5, "hour 1 should have gone");
}

/// Each division's route made alone ([`DivisionRoutes::route_of`] with [`estimate_one`]) equals
/// the hour's whole routes, for a moving division with and without a recorded speed and a standing
/// one, over two hours so the second takes its kept placement.
#[test]
fn a_division_s_route_made_alone_equals_the_whole_hour_s() {
    use crate::map::counter_move::{DivisionRoutes, RouteMemo};
    let map = content(two_leg_map());
    let anchor = |province: u32| Some([province as f32 * 10.0, 0.0, 1.0]);
    let (whole_memo, alone_memo) = (RouteMemo::default(), RouteMemo::default());
    let standing = DivisionMovementRow { division: DIVISION + 1, ..Default::default() };
    for (at, speed) in [Some(SPEED), Some(SPEED), None].into_iter().enumerate() {
        let mut frame = hour(speed);
        frame.hour += at as i32;
        let mut census = rows::<tables::Divisions>(&frame).unwrap().rows().to_vec();
        census.push(DivisionRow { id: DIVISION + 1, province: 2, ..Default::default() });
        put(&mut frame, TableRows::Divisions(census));
        let mut movement = rows::<tables::DivisionMovement>(&frame).unwrap().rows().to_vec();
        movement.push(standing.clone());
        put(&mut frame, TableRows::DivisionMovement(movement));
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        let whole = DivisionRoutes::read_kept(&frame, &etas, Some(&map.map), &anchor, &whole_memo).unwrap();
        let terms = HourTerms::new(&frame);
        let census = rows::<tables::Divisions>(&frame).unwrap().rows();
        for row in rows::<tables::DivisionMovement>(&frame).unwrap().rows() {
            let division = census.iter().find(|division| division.id == row.division);
            let eta = estimate_one(row, division, &terms, Some(&map));
            assert_eq!(eta, etas.get(row.division), "hour {at}, division {}", row.division);
            let alone = DivisionRoutes::route_of(row, division.map_or(0, |division| division.province), eta, Some(&map.map), &anchor, &alone_memo);
            assert_eq!(Some(&alone), whole.by_division.get(&row.division), "hour {at}, division {}", row.division);
        }
    }
}

/// Exercise a large synthetic hour without tying correctness to the speed
/// or load of the machine running the test.
#[test]
fn a_busy_hour_estimates_every_route() {
    let (frame, map) = busy_hour(130, 10, 30);
    let map = content(map);
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.report().estimated, 10);
    assert_eq!(etas.report().refused_total(), 0);
    assert_eq!(etas.report().progress_refused, 0);
    // Thirty known non-neighbour hops at the native 100000 distance, each
    // truncated to 14285 at speed 700000.
    assert_eq!(etas.get(0).map(|pair| pair.1), Some(30 * 14_285));
}

/// Province 3's state has infrastructure -0.5, so the second leg runs at half speed.
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
    // Without the States table the term is unknown and the estimate refuses by name.
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
    // 0.5 h and 2 h less 0.25 h of progress (base speed alone gave 0.25 h and 1.25 h).
    assert_eq!(etas.estimate(DIVISION), Ok((25_000, 225_000)));
    // A province absent from the hour's Provinces refuses by name.
    put(&mut frame, provinces(1..=2));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(
        etas.estimate(DIVISION),
        Err("Provinces has no row for province 3")
    );
}

/// A division at 10% organisation moving into a province its owner's enemy controls
/// moves at 1 + (1 - 0.1 / 0.3) * -0.8 of its speed; the same move into a friendly or
/// neutral province does not slow.
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
    // Factor 1 - 2/3 * 0.8 = 0.46667 (fixed5 46667): speed 326669.
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

/// Display-equal organisation words can give different movement penalties.
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
    // The native gate certifies no factor even if display rounding crossed its threshold.
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

/// The recorded terms move every edge: country army speed -0.25, a forest edge whose
/// stat-block movement is -0.5, days out of supply and a quarter tank of fuel; a
/// strategically redeploying division whose redeployment inputs were not read refuses
/// by name.
#[test]
fn recorded_speed_terms_move_each_edge() {
    let mut map = content(two_leg_map());
    // Province 3 is terrain 2, whose own adjuster adds -0.1.
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
    // A full tank: capacity 10 * 96.
    row.fuel = Recorded::Value(Some(96_000_000));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    // Edge into 2: 7 * 0.75 = 5.25 km/h; into 3: 7 * (0.75 - 0.6) = 1.05 km/h.
    let leg = |distance: i64, speed: i64| distance * 100_000 / speed;
    let progress = leg(175_000, 525_000);
    let next = leg(350_000, 525_000) - progress;
    assert_eq!(
        etas.estimate(DIVISION),
        Ok((next, next + leg(700_000, 105_000)))
    );
    // A quarter tank: fuel factor (1 - 0.4) * 0.25 + 0.4 = 0.55.
    row.fuel = Recorded::Value(Some(24_000_000));
    put(&mut frame, TableRows::DivisionMovement(vec![row.clone()]));
    let fueled = Etas::read(&frame, Some(&map)).unwrap();
    let first = 525_000 * 55_000 / 100_000;
    // The second edge floors at 1 km/h.
    let second = 100_000;
    let progress = leg(175_000, first);
    let next = leg(350_000, first) - progress;
    assert_eq!(
        fueled.estimate(DIVISION),
        Ok((next, next + leg(700_000, second)))
    );
    // A strategically redeploying division whose redeployment inputs were not read
    // refuses by name.
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

/// Enemy air superiority over the location's strategic region slows an owner in a war
/// relation: the owner's recorded pair and movement impact, scaled by the air speed
/// define. Outside a war relation the same scores add nothing.
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
    // Three quarters enemy superiority at impact -0.4, scaled by 0.5: -0.15.
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

/// The expected pair of the two-leg route at edge speeds `first` (1 -> 2) and `second`
/// (2 -> 3), less the 175000 numerator already moved at the first.
fn two_legs(first: i64, second: i64) -> Result<(i64, i64), &'static str> {
    let leg = |distance: i64, speed: i64| distance * 100_000 / speed;
    let next = leg(350_000, first) - leg(175_000, first);
    Ok((next, next + leg(700_000, second)))
}

/// Country 1, at war with country 2 when `war`, and country 2 controlling the route's
/// destination, province 3.
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

/// The country's attack speed factor (+0.5) speeds only a hostile move: one whose
/// recorded route ends at a province controlled by a country its owner is at war with.
/// The division is at full organisation, so the low-organisation slowdown stays out.
#[test]
fn attack_speed_applies_only_to_a_hostile_route() {
    let mut map = content(two_leg_map());
    map.defines.organisation = Some(OrganisationDefines {
        threshold: 30_000,
        penalty: -80_000,
    });
    let mut frame = hour(Some(SPEED));
    // Full organisation: the low-organisation hostile slowdown does not apply.
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

/// The location's province weather speed entry (-0.2) is scaled by the division's
/// acclimatization factor (0.5) before it joins the common modifier.
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

/// The division in orders group 5, whose commander has terrain movement `terrain`, and
/// whose selected commander block holds `modifiers`.
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

/// The two-leg route with a river (kind 2) on the edge 2-3.
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

/// Across a river (kind 2, -0.3), the commander block's river crossing factor (+0.5)
/// scales the crossing term: 1 + 1.5 * -0.3 = 0.55 on the crossing edge, 0.7 without it.
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

/// The commander's terrain movement is read at the destination's terrain index; an
/// index past the vector's end takes its first entry, as the game's lookup does.
#[test]
fn commander_terrain_falls_back_to_the_first_entry() {
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    // Terrain index 1 is in range: its own entry, -0.2.
    commanded(&mut frame, vec![0, -20_000], [None; 5]);
    let in_range = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 80_000 / 100_000;
    assert_eq!(in_range.estimate(DIVISION), two_legs(slowed, slowed));
    // A one-entry vector: index 1 is past its end and takes entry 0, -0.1.
    commanded(&mut frame, vec![-10_000], [None; 5]);
    let fallback = Etas::read(&frame, Some(&map)).unwrap();
    let slowed = SPEED * 90_000 / 100_000;
    assert_eq!(fallback.estimate(DIVISION), two_legs(slowed, slowed));
}

/// Under a supply-days define of zero, a division with no days out of supply still has
/// a nonzero out-of-supply severity (zero over zero is the game's sentinel), and that
/// severity reads received supply recorded only at its display step: refused by name,
/// never computed from the stepped value.
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

/// A strategic redeployment no disable-redeployment modifier blocks moves at the larger
/// of the road speed (over the destination state's railway infrastructure) and the rail
/// speed (over the departure province's directed railway percentage), times the outer
/// scale, without the common modifier. A blocked one moves at the ordinary speed; one
/// whose redeployment inputs were withheld refuses by name.
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
    // Province 3 lies in state 9, whose railway infrastructure is half its maximum.
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
    // Province 1 has a healthy level-5 railway to 2; province 2 has no rail info.
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
    // 1 -> 2: max(5 + 0 * 10, 10 + 1 * 25) = 35 km/h. 2 -> 3: max(5 + 0.5 * 10, 10 + 0)
    // = 10 km/h.
    assert_eq!(etas.estimate(DIVISION), two_legs(3_500_000, 1_000_000));
    assert_eq!(
        restricted_estimate(&frame, &map),
        two_legs(3_500_000, 1_000_000).map_err(str::to_owned)
    );
    // The road and rail speeds never read the stat block's base speed.
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
    // A disable-redeployment modifier blocks the rail: the ordinary speed applies.
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

/// The estimate on `frame` with every table but the mover's own and those the speed terms
/// read ([`replay_derive::eta_derive::frame_terms::TABLES`]) unavailable, as the derivation
/// checker's selected decoder leaves them.
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

/// Sub-units `light_armor` (type `armor`), `marine` (special forces) and `infantry`.
fn sub_unit_database() -> std::sync::Arc<installed_content_core::unit_leader::SubUnits> {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let mut units = installed_content_core::unit_leader::SubUnits::default();
    units.read(
        b"sub_units={ light_armor={type=armor} marine={special_forces=yes type=infantry} infantry={type=infantry} }",
        "common/units/test.txt",
    );
    std::sync::Arc::new(units)
}

/// Lexical tokens naming `light_armor`, `marine` and `infantry`, above every modifier
/// definition token the neutral world installs.
const ARMOUR: u32 = 900_001;
const MARINE: u32 = 900_002;
const INFANTRY: u32 = 900_003;

/// The division's template 0x42, whose priority sub-unit is `priority` and whose
/// composition is `units` of (token, count), with the sub-unit names added to the
/// hour's lexical tokens.
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

/// The out-of-supply severity's special-forces factor applies at the template's
/// special-forces share: 3 marines and 1 infantry give 0.75, so a -0.5 factor adds
/// -0.375 to the multiplier, as a -0.375 country out-of-supply factor would; a template
/// without special forces takes none of it.
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
    // Ten days out of supply at half the received supply, without grace.
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
    // A composition unreadable when recorded refuses the estimate by name.
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
    // Without special forces the factor adds nothing.
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

/// A division with a commander whose priority sub-unit is of type `armor` takes the
/// country's armour speed (+0.2); without a commander, or with another priority
/// sub-unit, it takes none.
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

/// Land provinces 1 and 2 and province 3 on `third` terrain, with the two-leg route's
/// edges.
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

/// A naval transfer moves every edge, and a land division every edge to or from the sea,
/// at its logical owner's recorded transport speed in the edge's source strategic region
/// times the content's transport scale, without the common modifier. An edge whose
/// region's speed was not recorded, or was refused, refuses by name.
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
    // The logical owner 4 is not the roster owner 1.
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
    // Both owners are linked, and the logical owner is in no war relation, so a normal
    // edge's enemy air step adds nothing.
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
    // From region 70: 8 * 0.5 = 4 km/h; from region 71: 4 * 0.5 = 2 km/h.
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(400_000, 200_000).unwrap())
    );
    // Transfers never inspect the land status, template, weather, supply or
    // withdrawal inputs, even when those independent captures are unavailable.
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
    // Nor do they read the stat block's base speed.
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
    // Without a transfer, the land edge 1 -> 2 moves at the normal speed and the edge
    // 2 -> 3 into the sea at the transport speed in region 71.
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

/// A division in combat that is not withdrawing starts its common modifier at the land
/// combat movement base instead of one: the combat movement define (0.33) plus its first
/// land combat's movement modifier (-0.13), so 7 km/h moves at 1.4 km/h; at a naval
/// combat it starts at zero and moves at the 1 km/h floor. A withdrawing division keeps
/// the one, and a refused combat operand refuses by name.
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
    // Withdrawing: the one stays, and the withdrawal define (-0.5) applies.
    combat(&mut frame, Some(Some(-13_000)));
    row.withdrawal = Recorded::Value(Some(1));
    put(&mut frame, TableRows::DivisionMovement(vec![row]));
    assert_eq!(
        restricted_estimate(&frame, &map),
        Ok(two_legs(350_000, 350_000).unwrap())
    );
}

/// The location state's modifier block selected for the roster owner adds its army
/// speed (-0.2) and, by the owner's recorded relation to the state's controller, its
/// controller army speed (+0.1, friendly) or its enemy army speed (-0.3, enemy): 0.9 or
/// 0.5 of the speed. A location without its state's row refuses by name.
#[test]
fn the_location_state_adds_its_speed_entries() {
    use replay_data::{rows::MovementStateTermRow, rows::StateMovementTerms};
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED));
    // The location, province 1, lies in state 9.
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

/// The owner country's entries for the template's movement priority sub-unit speed keys
/// add in native order with repeats: keys a, b, a with entries +0.1, none and +0.1 give
/// 1.2 of the speed. A key without its country row refuses by name.
#[test]
fn the_priority_sub_unit_and_category_entries_add_with_repeats() {
    use replay_data::{rows::CountryUnitSpeedEntryRow, rows::DivisionTemplateIconRow};
    // Any modifier index the hour defines can be a sub-unit's or category's key.
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

/// A division whose owner lacks core supply access at its location adds the content's
/// noncore supply speed (-0.5), its air form (-0.25) when air-supplied; the recorded
/// bypass, or exile, adds none. A refused access refuses by name.
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

/// Across a river (kind 2, -0.3), the owner's relation towards the destination's
/// controller (country 2) adds its river crossing factor against that country (+0.5):
/// 1 + 1.5 * -0.3 = 0.55 on the crossing edge, 0.7 without an entry. A crossing without
/// its relation's row refuses by name.
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
    // The later ordinary edge reads air at the actual lake location. Its
    // owner is at war and that location has region 70, so supply the neutral
    // pair rather than leaving a genuinely reached table unavailable.
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
    // Region 70 is needed for the sea leg, so ordinary first-speed air
    // admission also needs the owner's war state. Peace skips air scores.
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
            // A neutral common modifier is still required by the actual first A -> A speed.
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
        // An available unselected define cannot conceal a refused selected one.
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
    // Equal hours differ by branch, so a cache key without the branch would show.
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
        // This reference bypasses both caches and the ticket gate.
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
