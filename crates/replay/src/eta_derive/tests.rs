use super::*;

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

fn movement(path: Vec<u32>) -> DivisionMovementRow {
    DivisionMovementRow {
        division: 51,
        path_total: path.len() as u32,
        path,
        // 4 km/h: with outer scale 1.0 every edge runs at 400000 per hour.
        base_speed: Some(400_000).into(),
        ..Default::default()
    }
}

fn estimate_with(
    movement: &DivisionMovementRow,
    location: u32,
    distance: impl Fn(u32, u32) -> Option<i64>,
) -> Result<(i64, i64), String> {
    estimate(movement, location, land, distance, &defines())
}

#[test]
fn division_progress_needs_only_the_route_location_and_content() {
    let mut path = movement(vec![2, 3]);
    path.numerator = 300_000;
    let progress = |movement: &DivisionMovementRow, location| {
        division_progress(
            movement,
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
    assert_eq!(progress(&path, Some(1)), Some(150_000));
    // The base speed is not an operand of progress.
    path.base_speed = Recorded::NotRecorded;
    assert_eq!(progress(&path, Some(1)), Some(150_000));
    path.path.truncate(1);
    assert_eq!(progress(&path, Some(1)), Some(150_000));
    path.numerator = i64::MIN;
    assert_eq!(progress(&path, Some(1)), Some(0xffff_ffff));
    // Movement progress does not skip a same-province first hop. Its authored
    // no-adjacency distance still divides the numerator.
    assert_eq!(progress(&path, Some(2)), Some(0xffff_ffff));
    assert_eq!(progress(&path, None), None);
    path.path[0] = 0;
    assert_eq!(progress(&path, Some(1)), None);
    path.path.clear();
    assert_eq!(progress(&path, Some(1)), None);
    path.path.push(2);
    path.path_total = 0;
    assert_eq!(progress(&path, None), None);
    path.path.clear();
    assert_eq!(progress(&path, None), Some(0));
    // Class 0 takes the authored embark cost on a sea-to-land first edge.
    let embark = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { SEA } else { LAND },
            ..row
        })
    };
    let mut landing = movement(vec![2]);
    landing.numerator = 150_000;
    assert_eq!(
        division_progress(&landing, Some(1), embark, |_, _| None, &defines()),
        Some(50_000)
    );
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
fn an_estimate_is_the_remaining_route_distance_over_the_base_speed() {
    let mut path = movement(vec![2, 3]);
    path.numerator = 50_000;
    let distance = |from, to| {
        Some(if (from, to) == (2, 3) {
            250_000
        } else {
            100_000
        })
    };
    // Legs 100000/400000 = 0.25 h and 250000/400000 = 0.625 h; progress
    // 50000/400000 = 0.125 h is already behind the division.
    assert_eq!(estimate_with(&path, 1, distance), Ok((12_500, 75_000)));
    // Twice the speed halves every term.
    path.base_speed = Some(800_000).into();
    assert_eq!(estimate_with(&path, 1, distance), Ok((6_250, 37_500)));
    // The estimate is the native walk with each leg at the estimated speed.
    let speed = estimated_speed(800_000, &defines());
    let legs = [
        EtaLeg {
            distance: 100_000,
            speed: EtaSpeed::Fixed(speed),
        },
        EtaLeg {
            distance: 250_000,
            speed: EtaSpeed::Fixed(speed),
        },
    ];
    let native = eta_remaining(
        &EtaInputs {
            division: 51,
            next: 2,
            destination: 3,
            path_total: 2,
            leg_len: 2,
            next_leg_count: 1,
            numerator: 50_000,
            current_speed: EtaSpeed::Fixed(speed),
            ..Default::default()
        },
        &legs,
    );
    assert_eq!(
        estimate_with(&path, 1, distance),
        native.map_err(str::to_owned)
    );
    // A day and five hours of route ceils to the game's whole hours.
    let long = movement(vec![2]);
    assert_eq!(
        estimate_with(&long, 1, |_, _| Some(11_600_000))
            .map(|(next, _)| engine::cunit::eta_hours(next)),
        Ok(29)
    );
}

#[test]
fn the_estimated_speed_floors_at_one_kilometre_and_applies_the_movement_scale() {
    let scaled = Defines {
        outer_scale: 200_000,
        ..defines()
    };
    assert_eq!(estimated_speed(400_000, &scaled), 800_000);
    // A slower stat speed still moves at the native floor.
    for base in [0, 50_000, -1, i64::MIN] {
        assert_eq!(estimated_speed(base, &scaled), 200_000, "{base}");
    }
    let mut path = movement(vec![2]);
    path.base_speed = Some(0).into();
    assert_eq!(
        estimate(&path, 1, land, |_, _| Some(200_000), &scaled),
        Ok((100_000, 100_000))
    );
}

#[test]
fn a_self_first_edge_emits_no_leg_so_next_remaining_is_minus_progress() {
    let mut path = movement(vec![1, 2]);
    path.numerator = 50_000;
    assert_eq!(
        estimate_with(&path, 1, |_, _| Some(100_000)),
        Ok((-12_500, 12_500))
    );
}

#[test]
fn class_zero_estimates_take_the_embark_cost_on_a_sea_to_land_edge() {
    let province = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { SEA } else { LAND },
            ..row
        })
    };
    let path = movement(vec![2]);
    assert_eq!(
        estimate(&path, 1, province, |_, _| None, &defines()),
        Ok((75_000, 75_000))
    );
    // A source with terrain flag 2 is not the native embark override.
    let coastal = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { 2 } else { LAND },
            ..row
        })
    };
    assert_eq!(
        estimate(&path, 1, coastal, |_, _| Some(400_000), &defines()),
        Ok((100_000, 100_000))
    );
}

#[test]
fn an_estimate_refuses_by_name_instead_of_guessing() {
    let path = movement(vec![2, 3]);
    let ok = |_, _| Some(100_000);
    // A division standing still needs no speed.
    let mut still = movement(Vec::new());
    still.base_speed = Recorded::Value(None);
    assert_eq!(estimate_with(&still, 1, ok), Ok((0, 0)));
    let mut old = path.clone();
    old.base_speed = Recorded::NotRecorded;
    assert!(
        estimate_with(&old, 1, ok)
            .unwrap_err()
            .contains("DivisionMovement.base_speed")
    );
    let mut refused = path.clone();
    refused.base_speed = Recorded::Value(None);
    assert!(
        estimate_with(&refused, 1, ok)
            .unwrap_err()
            .contains("refused")
    );
    assert!(
        estimate_with(&path, 0, ok)
            .unwrap_err()
            .contains("location")
    );
    let mut truncated = path.clone();
    truncated.path_total = 3;
    assert!(
        estimate_with(&truncated, 1, ok)
            .unwrap_err()
            .contains("truncated")
    );
    let mut null = path.clone();
    null.path[1] = 0;
    assert!(estimate_with(&null, 1, ok).unwrap_err().contains("null"));
    assert!(
        estimate_with(&path, 1, |from, _| (from == 1).then_some(1))
            .unwrap_err()
            .contains("2->3")
    );
}

#[test]
fn a_prospective_route_subtracts_progress_only_on_the_recorded_first_edge() {
    let mut path = movement(vec![2, 3]);
    path.numerator = 50_000;
    let distance = |_, _| Some(100_000);
    // The recorded first edge: 0.25 h + 0.25 h less 0.125 h of progress.
    assert_eq!(
        estimate_route(&path, 1, &[2, 5], land, distance, &defines()),
        Ok(37_500)
    );
    // Another first edge starts afresh.
    assert_eq!(
        estimate_route(&path, 1, &[4, 5], land, distance, &defines()),
        Ok(50_000)
    );
    // An empty search result needs no speed.
    path.base_speed = Recorded::NotRecorded;
    assert_eq!(
        estimate_route(&path, 1, &[], land, distance, &defines()),
        Ok(0)
    );
    assert!(
        estimate_route(&path, 1, &[2], land, distance, &defines())
            .unwrap_err()
            .contains("base_speed")
    );
}
