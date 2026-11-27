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
        organisation: Some(OrganisationDefines {
            threshold: 30_000,
            penalty: -80_000,
        }),
        rail: None,
        noncore_supply: [Some(-50_000), Some(-25_000)],
        combat_movement: Some(33_000),
    }
}

fn no_edge_terms(_: u32, _: u32) -> Result<EdgeTerm, String> {
    Ok(EdgeTerm::Normal(NormalEdgeOperands::default()))
}

const PLAIN: SpeedTerms<'static> = SpeedTerms {
    modifier: 100_000,
    edge: &no_edge_terms,
    organisation: None,
    fuel: None,
    base_speed: Some(400_000),
};

fn with_infrastructure(
    infrastructure: &dyn Fn(u32) -> Result<i64, String>,
    test: impl FnOnce(&SpeedTerms<'_>),
) {
    let edge = |_: u32, to: u32| {
        Ok(EdgeTerm::Normal(edge_operands(&EdgeTerms {
            infrastructure: infrastructure(to)?,
            ..Default::default()
        })))
    };
    test(&SpeedTerms {
        edge: &edge,
        ..PLAIN
    });
}

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
        ..Default::default()
    }
}

fn estimate_with(
    movement: &DivisionMovementRow,
    location: u32,
    distance: impl Fn(u32, u32) -> Option<i64>,
) -> Result<(i64, i64), String> {
    estimate(movement, location, land, distance, &PLAIN, &defines())
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
    assert_eq!(progress(&path, Some(1)), Some(150_000));
    path.path.truncate(1);
    assert_eq!(progress(&path, Some(1)), Some(150_000));
    path.numerator = i64::MIN;
    assert_eq!(progress(&path, Some(1)), Some(0xffff_ffff));
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
    assert_eq!(estimate_with(&path, 1, distance), Ok((12_500, 75_000)));
    let fast = SpeedTerms {
        base_speed: Some(800_000),
        ..PLAIN
    };
    assert_eq!(
        estimate(&path, 1, land, distance, &fast, &defines()),
        Ok((6_250, 37_500))
    );
    let speed = edge_speed(800_000, 1, 2, &PLAIN, &defines()).unwrap();
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
        estimate(&path, 1, land, distance, &fast, &defines()),
        native.map_err(str::to_owned)
    );
    let long = movement(vec![2]);
    assert_eq!(
        estimate_with(&long, 1, |_, _| Some(11_600_000))
            .map(|(next, _)| game_model::movement::unit::eta_hours(next)),
        Ok(29)
    );
}

#[test]
fn the_edge_speed_floors_at_one_kilometre_and_applies_the_movement_scale() {
    let scaled = Defines {
        outer_scale: 200_000,
        ..defines()
    };
    assert_eq!(edge_speed(400_000, 1, 2, &PLAIN, &scaled), Ok(800_000));
    for base in [0, 50_000, -1, i64::MIN] {
        assert_eq!(
            edge_speed(base, 1, 2, &PLAIN, &scaled),
            Ok(200_000),
            "{base}"
        );
    }
    let path = movement(vec![2]);
    let still = SpeedTerms {
        base_speed: Some(0),
        ..PLAIN
    };
    assert_eq!(
        estimate(&path, 1, land, |_, _| Some(200_000), &still, &scaled),
        Ok((100_000, 100_000))
    );
}

#[test]
fn each_edge_takes_its_destination_state_infrastructure_term() {
    let infrastructure = |to: u32| Ok(if to == 3 { -50_000 } else { 0 });
    let mut path = movement(vec![2, 3]);
    path.numerator = 50_000;
    with_infrastructure(&infrastructure, |terms| {
        assert_eq!(
            estimate(&path, 1, land, |_, _| Some(100_000), terms, &defines()),
            Ok((12_500, 62_500))
        );
    });
    let first = |to: u32| Ok(if to == 2 { -50_000 } else { 0 });
    with_infrastructure(&first, |terms| {
        assert_eq!(
            estimate(&path, 1, land, |_, _| Some(100_000), terms, &defines()),
            Ok((25_000, 50_000))
        );
    });
    let deep = |_: u32| Ok(-95_000);
    with_infrastructure(&deep, |terms| {
        assert_eq!(edge_speed(400_000, 1, 2, terms, &defines()), Ok(100_000));
        assert_eq!(edge_speed(4_000_000, 1, 2, terms, &defines()), Ok(200_000));
    });
    let missing = |_: u32| Err("States state 7 is absent".to_owned());
    with_infrastructure(&missing, |terms| {
        assert!(
            estimate(&path, 1, land, |_, _| Some(100_000), terms, &defines())
                .unwrap_err()
                .contains("state 7")
        );
    });
}

#[test]
fn a_hostile_move_below_the_organisation_threshold_slows_by_the_native_factor() {
    let hostile = || Ok(true);
    assert_eq!(
        organisation_factor(Ok((150_000, 1_000_000)), hostile, &defines()),
        Ok(Some(60_000))
    );
    let slowed = SpeedTerms {
        organisation: Some(60_000),
        ..PLAIN
    };
    assert_eq!(edge_speed(400_000, 1, 2, &slowed, &defines()), Ok(240_000));
    assert_eq!(
        organisation_factor(Ok((0, 1_000_000)), hostile, &defines()),
        Ok(Some(20_000))
    );
    assert_eq!(
        organisation_factor(Ok((300_000, 1_000_000)), hostile, &defines()),
        Ok(None)
    );
    let peaceful = || Ok(false);
    assert_eq!(
        organisation_factor(Ok((0, 1_000_000)), peaceful, &defines()),
        Ok(None)
    );
    let refused = || Err("no war".to_owned());
    assert_eq!(
        organisation_factor(Ok((400_000, 1_000_000)), refused, &defines()),
        Ok(None)
    );
    assert_eq!(
        organisation_factor(Err("no status".into()), peaceful, &defines()),
        Ok(None)
    );
    let undefined = Defines {
        organisation: None,
        ..defines()
    };
    assert_eq!(
        organisation_factor(Ok((0, 1_000_000)), peaceful, &undefined),
        Ok(None)
    );
    assert!(
        organisation_factor(Ok((0, 1_000_000)), hostile, &undefined)
            .unwrap_err()
            .contains("defines")
    );
    assert_eq!(
        organisation_factor(Err("no status".into()), hostile, &defines()),
        Err("no status".into())
    );
    assert_eq!(
        organisation_factor(Ok((0, 0)), hostile, &defines()),
        Ok(None)
    );
    assert_eq!(
        organisation_factor(Ok((i64::MIN, 1_000_000)), hostile, &defines()),
        Ok(None)
    );
    assert_eq!(
        organisation_factor(Ok((-5, 1_000_000)), hostile, &defines()),
        Ok(Some(20_000))
    );
    let terms = SpeedTerms {
        organisation: Some(50_000),
        ..PLAIN
    };
    let path = movement(vec![2]);
    assert_eq!(
        estimate(&path, 1, land, |_, _| Some(200_000), &terms, &defines()),
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
        estimate(&path, 1, province, |_, _| None, &PLAIN, &defines()),
        Ok((75_000, 75_000))
    );
    let coastal = |id: u32| {
        land(id).map(|row| ProvinceOperands {
            terrain_flags: if id == 1 { 2 } else { LAND },
            ..row
        })
    };
    assert_eq!(
        estimate(&path, 1, coastal, |_, _| Some(400_000), &PLAIN, &defines()),
        Ok((100_000, 100_000))
    );
}

#[test]
fn an_estimate_refuses_by_name_instead_of_guessing() {
    let path = movement(vec![2, 3]);
    let ok = |_, _| Some(100_000);
    let unknown = SpeedTerms {
        base_speed: None,
        ..PLAIN
    };
    let still = movement(Vec::new());
    assert_eq!(
        estimate(&still, 1, land, ok, &unknown, &defines()),
        Ok((0, 0))
    );
    assert_eq!(
        estimate(&path, 1, land, ok, &unknown, &defines()),
        Err("the division's base speed is not recorded".into())
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
    assert_eq!(
        estimate_route(&path, 1, &[2, 5], land, distance, &PLAIN, &defines()),
        Ok(37_500)
    );
    assert_eq!(
        estimate_route(&path, 1, &[4, 5], land, distance, &PLAIN, &defines()),
        Ok(50_000)
    );
    let unknown = SpeedTerms {
        base_speed: None,
        ..PLAIN
    };
    assert_eq!(
        estimate_route(&path, 1, &[], land, distance, &unknown, &defines()),
        Ok(0)
    );
    assert_eq!(
        estimate_route(&path, 1, &[2], land, distance, &unknown, &defines()),
        Err("the division's base speed is not recorded".into())
    );
}

#[test]
fn only_a_normal_edge_reads_the_base_speed() {
    let special = |from: u32, _: u32| {
        Ok(if from == 1 {
            EdgeTerm::Scaled {
                speed: 800_000,
                scale: 50_000,
            }
        } else {
            EdgeTerm::Fixed(700_000)
        })
    };
    let terms = SpeedTerms {
        edge: &special,
        base_speed: None,
        ..PLAIN
    };
    let path = movement(vec![2, 3]);
    let distance = |_, _| Some(350_000);
    {
        assert_eq!(
            estimate(&path, 1, land, distance, &terms, &defines()),
            Ok((87_500, 137_500))
        );
        assert_eq!(
            estimate_route(&path, 1, &[2, 3], land, distance, &terms, &defines()),
            Ok(137_500)
        );
    }
    let mixed = |from: u32, _: u32| {
        Ok(if from == 1 {
            EdgeTerm::Fixed(700_000)
        } else {
            EdgeTerm::Normal(NormalEdgeOperands::default())
        })
    };
    let terms = SpeedTerms {
        edge: &mixed,
        base_speed: None,
        ..PLAIN
    };
    assert_eq!(
        estimate(&path, 1, land, distance, &terms, &defines()),
        Err("the division's base speed is not recorded".into())
    );
}

#[test]
fn the_common_modifier_adds_country_commander_weather_supply_and_retreat_terms() {
    let modifiers = CommonModifiers {
        country_speed: 10_000,
        leader_speed: 5_000,
        country_attack_speed: 20_000,
        weather: [-30_000, -10_000, -20_000, -40_000],
        ..Default::default()
    };
    assert_eq!(
        common_modifier(&modifiers, false, 50_000, -7, -11, 3),
        100_000 + 10_000 + 5_000 - 15_000 - 10_000 - 7 - 11 + 3
    );
    assert_eq!(
        common_modifier(&modifiers, true, 50_000, 0, 0, 0),
        100_000 + 10_000 + 5_000 + 20_000 - 15_000 - 5_000 - 10_000 - 20_000
    );
    let odd = CommonModifiers {
        weather: [-3, 0, -3, 0],
        ..Default::default()
    };
    assert_eq!(
        common_modifier(&odd, false, 50_000, 0, 0, 0),
        100_000 - 1 - 1
    );
    let edge = |_: u32, _: u32| {
        Ok(EdgeTerm::Normal(edge_operands(&EdgeTerms {
            terrain: -20_000,
            fort: Some(-10_000),
            snow: None,
            leader_terrain: 5_000,
            infrastructure: -5_000,
            river: None,
        })))
    };
    let terms = SpeedTerms {
        modifier: 90_000,
        edge: &edge,
        organisation: None,
        fuel: Some(50_000),
        base_speed: Some(400_000),
    };
    assert_eq!(edge_speed(400_000, 1, 2, &terms, &defines()), Ok(120_000));
}

#[test]
fn the_common_modifier_adds_state_unit_and_noncore_terms() {
    let modifiers = CommonModifiers {
        state_speed: -20_000,
        state_attack_speed: 30_000,
        unit_speed: 15_000,
        noncore: -50_000,
        ..Default::default()
    };
    assert_eq!(
        common_modifier(&modifiers, false, 0, 0, 0, 0),
        100_000 - 20_000 + 15_000 - 50_000
    );
    assert_eq!(
        common_modifier(&modifiers, true, 0, 0, 0, 0),
        100_000 - 20_000 + 15_000 - 50_000 + 30_000
    );
}

#[test]
fn the_combat_base_replaces_the_common_modifiers_one() {
    assert_eq!(combat_base(Some(-13_000), 33_000), 20_000);
    assert_eq!(combat_base(None, 33_000), 0);
    let modifiers = CommonModifiers {
        combat_base: Some(combat_base(Some(-13_000), 33_000)),
        country_speed: 10_000,
        ..Default::default()
    };
    assert_eq!(common_modifier(&modifiers, false, 0, 0, 0, 0), 30_000);
    let naval = CommonModifiers {
        combat_base: Some(combat_base(None, 33_000)),
        country_speed: 10_000,
        ..Default::default()
    };
    assert_eq!(common_modifier(&naval, false, 0, 0, 0, 0), 10_000);
    let terms = SpeedTerms {
        modifier: 20_000,
        ..PLAIN
    };
    assert_eq!(edge_speed(400_000, 1, 2, &terms, &defines()), Ok(100_000));
}

#[test]
fn a_scaled_edge_multiplies_its_speed_by_its_own_scale() {
    let transport = |_: u32, _: u32| {
        Ok(EdgeTerm::Scaled {
            speed: 300_000,
            scale: 50_000,
        })
    };
    let terms = SpeedTerms {
        modifier: 0,
        edge: &transport,
        organisation: Some(10_000),
        fuel: Some(10_000),
        base_speed: Some(400_000),
    };
    assert_eq!(edge_speed(400_000, 1, 2, &terms, &defines()), Ok(150_000));
    let fixed = |_: u32, _: u32| Ok(EdgeTerm::Fixed(70_000));
    let terms = SpeedTerms {
        edge: &fixed,
        ..terms
    };
    assert_eq!(edge_speed(400_000, 1, 2, &terms, &defines()), Ok(70_000));
}

#[test]
fn the_supply_term_scales_the_unsupplied_share_by_the_days_out_of_supply() {
    let operands = SupplyOperands {
        grace_maximum: 6_400_000,
        grace_remaining: 1_600_000,
        ratio: 20_000,
        days: 15,
        supply_days: 30,
        supply_grace: 64,
        penalty: -80_000,
    };
    let unknown = || Err("the template's special-forces share is not recorded".to_owned());
    let factors = SupplyFactors {
        country: -25_000,
        leader: 0,
        special_forces: [0, 0],
        share: &unknown,
    };
    assert_eq!(supply_severity(&operands, &factors), Ok(28_125));
    assert_eq!(supply_term(false, &operands, &factors), Ok(-22_500));
    assert_eq!(supply_term(true, &operands, &factors), Ok(0));
    for operands in [
        SupplyOperands {
            days: 0,
            ..operands
        },
        SupplyOperands {
            ratio: 100_000,
            ..operands
        },
    ] {
        assert_eq!(supply_term(false, &operands, &factors), Ok(0));
    }
    let empty = SupplyOperands {
        grace_maximum: 0,
        ..operands
    };
    assert_eq!(supply_severity(&empty, &factors), Ok(37_500));
    let special = SupplyFactors {
        special_forces: [-50_000, 0],
        ..factors
    };
    assert!(
        supply_term(false, &operands, &special)
            .unwrap_err()
            .contains("special-forces")
    );
    let half = || Ok(50_000);
    let special = SupplyFactors {
        share: &half,
        ..special
    };
    assert_eq!(supply_severity(&operands, &special), Ok(18_750));
    let negative = SupplyFactors {
        country: -200_000,
        ..factors
    };
    assert_eq!(supply_severity(&operands, &negative), Ok(0));
    let factors = SupplyFactors {
        country: 0,
        ..factors
    };
    assert_eq!(
        attrition_supply_severity(&operands, 50_000, &factors),
        Ok(25_000)
    );
    assert_eq!(
        attrition_supply_severity(&operands, 25_000, &factors),
        Ok(0)
    );
    assert_eq!(attrition_supply_severity(&operands, 0, &factors), Ok(0));
}

#[test]
fn retreat_and_fuel_terms_follow_the_native_branches() {
    assert_eq!(retreat_term(true, true, 25_000, 10_000, -50_000), 35_000);
    assert_eq!(retreat_term(false, true, 25_000, 10_000, -50_000), -50_000);
    assert_eq!(retreat_term(false, false, 25_000, 10_000, -50_000), 0);
    assert_eq!(
        fuel_factor(24_000_000, 1_000_000, 9_600_000, 40_000),
        Ok(Some(55_000))
    );
    assert_eq!(
        fuel_factor(96_000_000, 1_000_000, 9_600_000, 40_000),
        Ok(None)
    );
    assert_eq!(fuel_factor(0, 0, 9_600_000, 40_000), Ok(None));
    assert_eq!(
        fuel_factor(i64::MIN, 1_000_000, 9_600_000, 40_000),
        Ok(None)
    );
    assert_eq!(
        fuel_factor(-5, 1_000_000, 9_600_000, 40_000),
        Ok(Some(40_000))
    );
}

#[test]
fn air_term_follows_the_enemy_share() {
    let air = AirOperands {
        friendly: 1,
        enemy: 3,
        impact: -40_000,
        scale: 50_000,
        defense: 2_000,
    };
    assert_eq!(air_term(Some(&air)), Ok(-13_000));
    assert_eq!(
        air_term(Some(&AirOperands {
            defense: 20_000,
            ..air
        })),
        Ok(0)
    );
    for enemy in [0, -5] {
        assert_eq!(air_term(Some(&AirOperands { enemy, ..air })), Ok(0));
    }
    assert_eq!(air_term(None), Ok(0));
}
