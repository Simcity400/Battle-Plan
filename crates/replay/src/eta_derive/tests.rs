use super::*;
use crate::rows::TerrainSpeed;

const LAND: u8 = 1;
const SEA: u8 = 0;

#[test]
fn railway_gun_first_edge_uses_captured_speed_content_distance_and_native_progress() {
    use crate::rows::{RailwayGunEtaOperands, Recorded};
    let mut gun = RailwayGunCounterInputRow {
        gun: 74,
        province: 1,
        ordinary_path: vec![2, 3],
        // Later targeted members still take part in a counter's first-edge minimum.
        target: 3,
        eta: Recorded::Value(Some(RailwayGunEtaOperands {
            numerator: 50_000,
            current_speed: 200_000,
        })),
        ..Default::default()
    };
    assert_eq!(
        railway_gun_first_edge(&gun, land, |_, _| Some(300_000), &defines()),
        Some(125_000)
    );
    assert_eq!(
        railway_gun_first_edge(&gun, land, |_, _| None, &defines()),
        None
    );
    let embark = |id| {
        land(id).map(|mut row| {
            if id == 1 {
                row.terrain_flags = SEA;
            }
            row
        })
    };
    assert_eq!(
        railway_gun_first_edge(
            &gun,
            embark,
            |_, _| panic!("embark uses its content define"),
            &defines()
        ),
        Some(125_000)
    );
    // A self edge is skipped by the walk, but progress is still subtracted.
    gun.ordinary_path[0] = 1;
    assert_eq!(
        railway_gun_first_edge(
            &gun,
            |_| panic!("no distance for self edge"),
            |_, _| panic!("no distance for self edge"),
            &defines()
        ),
        Some(-25_000)
    );
    for eta in [Recorded::NotRecorded, Recorded::Value(None)] {
        gun.eta = eta;
        assert_eq!(
            railway_gun_first_edge(&gun, land, |_, _| Some(300_000), &defines()),
            None
        );
    }
    gun.ordinary_path.clear();
    assert_eq!(
        railway_gun_first_edge(&gun, land, |_, _| None, &defines()),
        Some(0)
    );
}

fn defines() -> Defines {
    Defines {
        embark_distance: 300_000,
        transport_scale: 200_000,
        invasion_speed: 700_000,
        outer_scale: 100_000,
        river_crossing_two: -20_000,
        river_crossing_three: -50_000,
    }
}

/// Land, terrain index 1, no fort, state 1; id zero has no row.
fn land(id: u32) -> Option<ProvinceOperands> {
    (id != 0).then_some(ProvinceOperands {
        terrain_flags: LAND,
        terrain_index: 1,
        fort_positive: false,
        state: 1,
    })
}

/// modifier 1.0 with terrain zero, so a plain normal leg runs at speed_base.
fn inputs(_legs: usize) -> DivisionEtaInputsRow {
    DivisionEtaInputsRow {
        division: 51,
        speed_modifier: 100_000,
        speed_base: 400_000,
        current_speed: 400_000,
        terrain_speed: vec![TerrainSpeed {
            terrain: 1,
            modifier: 0,
        }],
        ..Default::default()
    }
}

fn movement(path: Vec<u32>) -> DivisionMovementRow {
    DivisionMovementRow {
        division: 51,
        path_total: path.len() as u32,
        path,
        ..Default::default()
    }
}

fn run(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    location: u32,
) -> Option<(i64, i64)> {
    run_with_distance(inputs, movement, location, |_, _| Some(100_000))
}

fn run_with_distance(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    location: u32,
    distance: impl Fn(u32, u32) -> Option<i64>,
) -> Option<(i64, i64)> {
    run_with_geometry(inputs, movement, location, distance, |_, _| Some(0))
}

fn run_with_geometry(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    location: u32,
    distance: impl Fn(u32, u32) -> Option<i64>,
    crossing_kind: impl Fn(u32, u32) -> Option<u8>,
) -> Option<(i64, i64)> {
    derive(
        inputs,
        movement,
        location,
        land,
        |_| Some(0),
        |_| false,
        distance,
        crossing_kind,
        &defines(),
    )
}

#[test]
fn progress_uses_only_the_first_native_eta_edge() {
    let row = inputs(2);
    let mut path = movement(vec![2, 3]);
    path.numerator = 300_000;
    let progress =
        |movement: &DivisionMovementRow, inputs: Option<&DivisionEtaInputsRow>, location| {
            movement_progress(
                movement,
                inputs,
                location,
                land,
                |from, to| {
                    Some(if (from, to) == (1, 2) {
                        200_000
                    } else {
                        900_000
                    })
                },
                &defines(),
            )
        };
    assert_eq!(progress(&path, Some(&row), Some(1)), Some(150_000));
    path.path.truncate(1);
    assert_eq!(progress(&path, Some(&row), Some(1)), Some(150_000));
    path.numerator = i64::MIN;
    assert_eq!(progress(&path, Some(&row), Some(1)), Some(0xffff_ffff));
    assert_eq!(progress(&path, None, Some(1)), None);
    // BEDCC0 does not skip a same-province first hop. Its authored
    // no-adjacency distance still divides the numerator.
    assert_eq!(progress(&path, Some(&row), Some(2)), Some(0xffff_ffff));
    assert_eq!(progress(&path, Some(&row), None), None);
    path.path[0] = 0;
    assert_eq!(progress(&path, Some(&row), Some(1)), None);
    path.path.clear();
    assert_eq!(progress(&path, Some(&row), Some(1)), None);
    path.path.push(2);
    path.path_total = 0;
    assert_eq!(progress(&path, None, None), None);
    path.path.clear();
    assert_eq!(progress(&path, None, None), Some(0));
}

#[test]
fn railway_gun_progress_uses_class_thirteen_and_the_ordinary_first_edge() {
    let province = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { SEA } else { LAND },
            ..row
        })
    };
    let progress = unit_progress(13, 150_000, 1, 2, province, |_, _| None, &defines());
    assert_eq!(progress, Some(50_000));
    assert_eq!(
        unit_progress(13, 1, 1, 0, land, |_, _| Some(1), &defines()),
        None
    );
}

#[test]
fn embark_distance_overrides_adjacency_for_land_unit_classes() {
    let province = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { SEA } else { LAND },
            ..row
        })
    };
    let mut row = inputs(1);
    row.leg_transport_speed = vec![100_000];
    let path = movement(vec![2]);
    for class in [0, 13] {
        row.class = class;
        assert_eq!(
            derive(
                &row,
                &path,
                1,
                province,
                |_| Some(0),
                |_| false,
                |_, _| None,
                |_, _| Some(0),
                &defines()
            ),
            Some((150_000, 150_000)),
            "embark cost bypasses adjacency for class {class}"
        );
        let mut progressing = path.clone();
        progressing.numerator = 150_000;
        assert_eq!(
            movement_progress(
                &progressing,
                Some(&row),
                Some(1),
                province,
                |_, _| None,
                &defines()
            ),
            Some(50_000)
        );
    }
    row.class = 1;
    assert_eq!(
        derive(
            &row,
            &path,
            1,
            province,
            |_| Some(0),
            |_| false,
            |_, _| Some(400_000),
            |_, _| Some(0),
            &defines()
        ),
        Some((200_000, 200_000)),
        "other unit classes use the adjacency distance"
    );
    row.class = 0;
    let coastal = |id: u32| {
        province(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { 2 } else { LAND },
            ..row
        })
    };
    assert_eq!(
        derive(
            &row,
            &path,
            1,
            coastal,
            |_| Some(0),
            |_| false,
            |_, _| Some(400_000),
            |_, _| Some(0),
            &defines()
        ),
        Some((200_000, 200_000)),
        "a source with terrain flag 2 is not the native embark override"
    );
}

#[test]
fn native_progress_keeps_signed_wrap_and_exceptional_results() {
    assert_eq!(native_progress_fraction(125, 100), 125_000);
    assert_eq!(native_progress_fraction(-25, 100), -25_000);
    assert_eq!(native_progress_fraction(125, -100), -125_000);
    for value in [i64::MAX, i64::MIN, i64::MIN + 1] {
        assert_eq!(native_progress_fraction(value, 1), 0xffff_ffff);
    }
    assert_eq!(native_progress_fraction(1, 0), 0xffff_ffff);
    assert_eq!(
        native_progress_fraction(i64::MAX - 1, 1),
        (i64::MAX - 1).wrapping_mul(100_000)
    );
}

#[test]
fn a_normal_route_truncates_each_leg_and_subtracts_progress() {
    let row = inputs(2);
    let mut path = movement(vec![2, 3]);
    path.numerator = 50_000;
    // Speed 400000: legs are 25000 and 62500 fixed5 hours, progress 12500.
    assert_eq!(
        run_with_distance(&row, &path, 1, |from, to| {
            Some(if (from, to) == (2, 3) {
                250_000
            } else {
                100_000
            })
        }),
        Some((12_500, 75_000))
    );
}

#[test]
fn a_self_first_edge_emits_no_leg_so_next_remaining_is_minus_progress() {
    let row = inputs(1);
    let mut path = movement(vec![1, 2]);
    path.numerator = 50_000;
    // cunit.rs:637 and :916 - route[0] == location.
    assert_eq!(run(&row, &path, 1), Some((-12_500, 12_500)));
}

#[test]
fn a_zero_speed_leg_stops_the_walk_and_zeroes_the_duration() {
    // Normal movement floors its pre-scale speed at 100000, even when the
    // base speed is zero. This route therefore never hits the zero guard.
    let mut row = inputs(2);
    row.speed_base = 0;
    assert_eq!(
        run(&row, &movement(vec![2, 3]), 1),
        Some((100_000, 200_000))
    );
    // A zero transport speed on leg zero stops the walk before the next
    // route edge and before progress is evaluated.
    let mut row = inputs(1);
    row.leg_transport_speed = vec![0];
    let first_water = |id: u32| {
        land(id).map(|p| ProvinceOperands {
            terrain_flags: if id == 2 { SEA } else { LAND },
            ..p
        })
    };
    assert_eq!(
        derive(
            &row,
            &movement(vec![2, 3]),
            1,
            first_water,
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines(),
        ),
        Some((0, 0))
    );
    // A later zero leg keeps next_remaining and zeroes the destination.
    let mut row = inputs(2);
    row.leg_transport_speed = vec![0, 0];
    let water = |id: u32| {
        land(id).map(|p| ProvinceOperands {
            terrain_flags: if id == 3 { SEA } else { LAND },
            ..p
        })
    };
    let derived = derive(
        &row,
        &movement(vec![2, 3]),
        1,
        water,
        |_| Some(0),
        |_| false,
        |_, _| Some(100_000),
        |_, _| Some(0),
        &defines(),
    );
    assert_eq!(derived, Some((25_000, 0)));
}

/// The recorder stops its scan at the first zero-speed leg, so the leg
/// vectors are a prefix of the route. The walk stops there too, and must
/// never reach for an operand of a leg neither side evaluates.
#[test]
fn a_route_recorded_only_up_to_its_stop_still_derives() {
    // Provinces 1..=3 are recorded and 4 is not, so touching 4 refuses.
    let known = |sea: u32| {
        move |id: u32| {
            (1..=3).contains(&id).then_some(ProvinceOperands {
                terrain_flags: if id == sea { SEA } else { LAND },
                terrain_index: 1,
                fort_positive: false,
                state: 1,
            })
        }
    };
    let evaluate = |row: &DivisionEtaInputsRow, sea: u32| {
        derive(
            row,
            &movement(vec![2, 3, 4]),
            1,
            known(sea),
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines(),
        )
    };
    // Stopping at leg zero: one recorded leg for a three-leg route.
    let mut row = inputs(1);
    row.leg_transport_speed = vec![0];
    assert_eq!(evaluate(&row, 2), Some((0, 0)));
    // Stopping mid route: two recorded legs, and leg two is never reached.
    let mut row = inputs(2);
    row.leg_transport_speed = vec![0, 0];
    assert_eq!(evaluate(&row, 3), Some((25_000, 0)));
    // A route that never stops still needs a leg for every route edge.
    assert_eq!(evaluate(&inputs(2), 0), None);
}

#[test]
fn every_branch_takes_its_own_recorded_leg_value() {
    // Transport: either endpoint not land, or the naval transfer bit.
    let mut row = inputs(1);
    row.leg_transport_speed = vec![50_000];
    let water = |id: u32| {
        land(id).map(|p| ProvinceOperands {
            terrain_flags: if id == 2 { SEA } else { LAND },
            ..p
        })
    };
    let transport = derive(
        &row,
        &movement(vec![2]),
        1,
        water,
        |_| Some(0),
        |_| false,
        |_, _| Some(100_000),
        |_, _| Some(0),
        &defines(),
    );
    // 50000 scaled by 200000 is speed 100000 over distance 100000.
    assert_eq!(transport, Some((100_000, 100_000)));
    let mut transfer = movement(vec![2]);
    transfer.naval_transfer = 1;
    assert_eq!(run(&row, &transfer, 1), Some((100_000, 100_000)));
    // The first sea-to-land edge is transport. Once ashore, the original
    // sea-to-land route header selects the fixed invasion speed.
    let mut row = inputs(2);
    row.current_speed = 100_000;
    row.leg_transport_speed = vec![50_000, 0];
    let sea_start = |id: u32| {
        land(id).map(|operand| ProvinceOperands {
            terrain_flags: if id == 1 { SEA } else { LAND },
            ..operand
        })
    };
    assert_eq!(
        derive(
            &row,
            &movement(vec![2, 3]),
            1,
            sea_start,
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines(),
        ),
        Some((300_000, 314_285))
    );
    // Rail: strategic redeployment with the rail branch unblocked.
    let mut row = inputs(1);
    row.rail_country_modifier = Some(0);
    row.rail_state_modifier = Some(0);
    row.rail_friendly_state_modifier = Some(0);
    row.leg_rail_speed = vec![250_000];
    let mut rail = movement(vec![2]);
    rail.strategic_redeployment = 1;
    assert_eq!(run(&row, &rail, 1), Some((40_000, 40_000)));
    // Any nonzero modifier blocks rail and falls through to normal speed.
    row.rail_friendly_state_modifier = Some(1);
    row.leg_rail_speed.clear();
    assert_eq!(run(&row, &rail, 1), Some((25_000, 25_000)));
    row.rail_friendly_state_modifier = None;
    row.rail_state_modifier = Some(1);
    assert_eq!(run(&row, &rail, 1), Some((25_000, 25_000)));
    row.rail_state_modifier = None;
    row.rail_country_modifier = Some(1);
    assert_eq!(run(&row, &rail, 1), Some((25_000, 25_000)));
}

#[test]
fn invasion_gate_uses_original_route_header_and_refuses_missing_terrain() {
    let path = movement(vec![2, 3]);
    let terrain = |id: u32| {
        land(id).map(|operand| ProvinceOperands {
            terrain_flags: match id {
                1 => 16, // sea with authored coast, still flags & 3 == 0
                2 | 3 => LAND,
                _ => SEA,
            },
            ..operand
        })
    };
    assert_eq!(invasion_branch(0, &path, 1, &terrain), Some(true));
    assert_eq!(invasion_branch(13, &path, 1, &terrain), Some(false));
    assert_eq!(
        invasion_branch(0, &path, 1, &|id| terrain(id).filter(|_| id != 2)),
        None
    );
    assert_eq!(
        invasion_branch(0, &path, 1, &|id| terrain(id).filter(|_| id != 1)),
        None
    );
    // A land or lake source skips the next-province flag read entirely.
    for flags in [LAND, 2] {
        let source_only = |id| {
            (id == 1).then_some(ProvinceOperands {
                terrain_flags: flags,
                ..Default::default()
            })
        };
        assert_eq!(invasion_branch(0, &path, 1, &source_only), Some(false));
    }
}

#[test]
fn evaluated_branches_require_their_operands_but_preserve_recorded_zero() {
    let mut row = inputs(1);
    let mut route = movement(vec![2]);
    route.naval_transfer = 1;
    assert_eq!(run(&row, &route, 1), None);
    row.leg_transport_speed = vec![0];
    assert_eq!(run(&row, &route, 1), Some((0, 0)));

    route.naval_transfer = 0;
    route.strategic_redeployment = 1;
    assert_eq!(run(&row, &route, 1), None);
    row.leg_rail_speed = vec![0];
    assert_eq!(run(&row, &route, 1), None);
    row.rail_country_modifier = Some(0);
    assert_eq!(run(&row, &route, 1), Some((0, 0)));

    route.strategic_redeployment = 0;
    assert_eq!(
        run_with_geometry(&row, &route, 1, |_, _| Some(100_000), |_, _| Some(1)),
        None
    );
    row.leg_river_combined = vec![0];
    assert_eq!(
        run_with_geometry(&row, &route, 1, |_, _| Some(100_000), |_, _| Some(1)),
        Some((25_000, 25_000))
    );
    assert_eq!(
        run_with_geometry(&row, &route, 1, |_, _| Some(100_000), |_, _| None),
        None,
        "unavailable authored crossing kind refuses the edge"
    );
}

#[test]
fn the_normal_edge_adds_fort_snow_infrastructure_and_the_river_terms() {
    let mut row = inputs(1);
    row.speed_modifier = 200_000;
    row.terrain_speed[0].modifier = -10_000;
    row.fort_penalty = -20_000;
    row.snow_penalty = -30_000;
    row.river_adjustment = -40_000;
    row.river_leader = 30_000;
    row.leg_river_combined = vec![50_000];
    let fortified = |id: u32| {
        land(id).map(|p| ProvinceOperands {
            fort_positive: true,
            ..p
        })
    };
    let derived = derive(
        &row,
        &movement(vec![2]),
        1,
        fortified,
        |_| Some(5_000),
        |_| true,
        |_, _| Some(100_000),
        |_, _| Some(2),
        &defines(),
    );
    let speed = PreparedNormalSpeed {
        modifier: 200_000,
        base_speed: 400_000,
        organisation: None,
        fuel: None,
        outer_scale: 100_000,
    }
    .evaluate(NormalEdgeOperands {
        additive: -10_000 - 20_000 - 30_000 + 5_000,
        river: Some((80_000, -40_000, -20_000)),
    });
    assert_eq!(
        derived,
        Some((10_000_000_000 / speed, 10_000_000_000 / speed))
    );
}

#[test]
fn a_terrain_past_a_byte_finds_its_own_modifier() {
    // KaiserreduX's rouen is category 256; the index is the native i32.
    let mut row = inputs(1);
    row.terrain_speed = vec![
        TerrainSpeed {
            terrain: 1,
            modifier: 0,
        },
        TerrainSpeed {
            terrain: 256,
            modifier: -10_000,
        },
    ];
    let at = |terrain_index: i32| {
        move |id: u32| land(id).map(|p| ProvinceOperands { terrain_index, ..p })
    };
    let derive_at = |terrain_index: i32| {
        derive(
            &row,
            &movement(vec![2]),
            1,
            at(terrain_index),
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines(),
        )
    };
    let speed = PreparedNormalSpeed {
        modifier: 100_000,
        base_speed: 400_000,
        organisation: None,
        fuel: None,
        outer_scale: 100_000,
    }
    .evaluate(NormalEdgeOperands {
        additive: -10_000,
        river: None,
    });
    assert_eq!(
        derive_at(256),
        Some((10_000_000_000 / speed, 10_000_000_000 / speed))
    );
    // A terrain the route's read did not list is unknown, never zero.
    assert_eq!(derive_at(2), None);
}

#[test]
fn the_organisation_and_fuel_factors_apply_when_present() {
    let mut row = inputs(1);
    row.speed_org = Some(50_000);
    row.speed_fuel = Some(80_000);
    // 400000 halved and then scaled by 0.8 is 160000.
    assert_eq!(run(&row, &movement(vec![2]), 1), Some((62_500, 62_500)));
}

#[test]
fn a_missing_operand_refuses_the_row_but_a_fallback_distance_does_not() {
    // Matching geometry reports a known no-adjacency edge as Some(100000).
    let row = inputs(1);
    assert_eq!(
        run_with_distance(&row, &movement(vec![2]), 1, |_, _| Some(100_000)),
        Some((25_000, 25_000))
    );
    assert_eq!(
        run_with_distance(&row, &movement(vec![2]), 1, |_, _| None),
        None,
        "unknown matching geometry is not a no-adjacency edge"
    );
    let refuse = |_: u32| None;
    let none = |_: u32| None;
    assert_eq!(
        derive(
            &row,
            &movement(vec![2]),
            1,
            refuse,
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines()
        ),
        None,
        "a province with no recorded row"
    );
    assert_eq!(
        derive(
            &row,
            &movement(vec![2]),
            1,
            land,
            none,
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines()
        ),
        None,
        "a state with no recorded infrastructure"
    );
    let deep = |id: u32| {
        land(id).map(|p| ProvinceOperands {
            terrain_index: 9,
            ..p
        })
    };
    assert_eq!(
        derive(
            &row,
            &movement(vec![2]),
            1,
            deep,
            |_| Some(0),
            |_| false,
            |_, _| Some(100_000),
            |_, _| Some(0),
            &defines()
        ),
        None,
        "a terrain index outside the recorded vector"
    );
    let mut short = inputs(1);
    short.leg_river_combined = vec![0];
    assert_eq!(
        run_with_geometry(
            &short,
            &movement(vec![2, 3]),
            1,
            |_, _| Some(100_000),
            |_, _| Some(2)
        ),
        None,
        "an evaluated branch vector shorter than the walk"
    );
    let mut truncated = movement(vec![2, 3]);
    truncated.path_total = 3;
    assert_eq!(run(&inputs(2), &truncated, 1), None, "a truncated path");
    assert_eq!(
        run(&inputs(2), &movement(vec![2, 0]), 1),
        None,
        "a null route province"
    );
    assert_eq!(
        run(&inputs(1), &movement(vec![2]), 0),
        None,
        "a null location"
    );
}
