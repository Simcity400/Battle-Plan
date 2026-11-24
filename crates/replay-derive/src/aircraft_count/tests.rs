use super::*;

fn full() -> EfficiencyInputs {
    let full = Container {
        capacity: ONE,
        supply: ONE,
        ..Container::default()
    };
    EfficiencyInputs {
        coverage: ONE,
        penalty: 0,
        mission_mask: 0,
        mission: full,
        effective: full,
        modifier: 0,
        country_region: None,
        fuel_penalty: ONE,
    }
}

#[test]
fn divides_after_each_fixed_product() {
    let mut input = full();
    input.coverage = 50_001;
    input.mission.capacity = 50_001;
    input.mission.supply = 50_001;
    assert_eq!(sortie_efficiency(&input), 12_500);
}

#[test]
fn mission_mask_skips_only_the_reassignment_factor() {
    let mut input = full();
    input.penalty = 40_000;
    assert_eq!(sortie_efficiency(&input), 60_000);
    input.mission_mask = 0x20;
    assert_eq!(sortie_efficiency(&input), ONE);
}

#[test]
fn modifiers_fuel_and_region_follow_native_order() {
    let mut input = full();
    input.coverage = 40_000;
    input.modifier = 10_000;
    input.effective.fuel_demand = 200;
    input.effective.fuel_allocated = 100;
    input.fuel_penalty = 20_000;
    input.country_region = Some(-10_000);
    assert_eq!(sortie_efficiency(&input), 23_760);
}

#[test]
fn fuel_is_full_without_demand_or_at_the_native_extremes() {
    let mut container = Container {
        fuel_demand: 100,
        fuel_allocated: -50,
        ..Container::default()
    };
    assert_eq!(fuel_ratio(container), 0);
    container.fuel_allocated = 200;
    assert_eq!(fuel_ratio(container), ONE);
    for allocated in [i64::MAX, i64::MIN, i64::MIN + 1] {
        container.fuel_allocated = allocated;
        assert_eq!(fuel_ratio(container), ONE);
    }
    container.fuel_demand = 0;
    container.fuel_allocated = 1;
    assert_eq!(fuel_ratio(container), ONE);
}

#[test]
fn final_efficiency_clamps_signed_results() {
    let mut input = full();
    input.coverage = -20_000;
    assert_eq!(sortie_efficiency(&input), 0);
    input.coverage = 200_000;
    assert_eq!(sortie_efficiency(&input), ONE);
}

#[test]
fn ordinary_rounds_half_away_then_applies_the_stance_ratio() {
    let ordinary = Base::Ordinary { stance_ratio: None };
    assert_eq!(scenario_plane_count(ordinary, 3, 50_000), 2);
    assert_eq!(scenario_plane_count(ordinary, -3, 50_000), -2);
    let carrier = Base::Ordinary {
        stance_ratio: Some(50_000),
    };
    assert_eq!(scenario_plane_count(carrier, 349, 1_000), 1);
}

#[test]
fn rocket_rounds_up_and_caps_while_gun_returns_one() {
    assert_eq!(scenario_plane_count(Base::Rocket(9), 10, 12_500), 2);
    assert_eq!(scenario_plane_count(Base::Rocket(9), 1, 12_500), 1);
    assert_eq!(scenario_plane_count(Base::Rocket(9), 10, -12_500), -1);
    assert_eq!(scenario_plane_count(Base::Gun, 10, 0), 1);
}

#[test]
fn rocket_rounding_wraps_its_multiply_back_as_native_does() {
    let efficiency = 9_223_372_036_854_775_800;
    assert_eq!(scenario_plane_count(Base::Rocket(1), 100, efficiency), 100);
    assert_eq!(
        scenario_plane_count(Base::Rocket(1), i32::MAX, efficiency),
        702_313_053
    );
    assert_eq!(
        scenario_plane_count(Base::Rocket(1), i32::MAX, 9_223_372_036_854_700_000),
        -702_313_053
    );
}

fn defines() -> SortieDefines {
    SortieDefines {
        fuel_penalty_raw: 50_000,
        missile_launcher_slots: 1,
        carrier_stance_ratios: vec![ONE, 50_000],
        mine_maximum_raw: ONE,
    }
}

#[test]
fn recorded_operands_give_hand_computed_counts() {
    let input = AirMissionInputRow {
        wing: 1,
        coverage_raw: 90_000,
        reassignment_penalty_raw: 10_000,
        ..Default::default()
    };
    let container = AirSortieBaseCountryRow {
        base: 5,
        country: 3,
        capacity_raw: 80_000,
        supply_raw: 95_000,
        fuel_demand_raw: Some(1_000),
        fuel_allocated_raw: Some(700),
    };
    let host = AirSortieSpecialBaseRow {
        base: 6,
        carrier_stance: Some(1),
        ..Default::default()
    };
    let host_container = AirSortieBaseCountryRow {
        base: 6,
        ..container
    };
    let mut sources = SortieSources {
        missions: BTreeMap::new(),
        containers: BTreeMap::from([((5, 3), &container), ((6, 3), &host_container)]),
        modifiers: BTreeMap::from([
            ((AirSortieModifierRow::WEATHER, 0, 9), -5_000),
            ((AirSortieModifierRow::COUNTRY, 3, 0), 2_000),
        ]),
        special: BTreeMap::from([(6, &host)]),
        metadata: ModifierMetadata {
            flags: 0,
            feature_selector: 0,
            callback_kind: 0,
            feature_flags: 0,
        },
    };
    let wing = AirWingRow {
        id: 1,
        planes: 100,
        mission_mask: 1 << 4,
        mission_region: Some(9),
        ..Default::default()
    };
    let state = AirBaseRow {
        id: 5,
        state: Some(2),
        ..Default::default()
    };
    let count = |sources: &SortieSources, wing: &AirWingRow, base: &AirBaseRow| {
        sources.count(wing, &input, base, 3, (3, 3), &defines())
    };
    assert_eq!(count(&sources, &wing, &state), Ok(51));
    let host_base = AirBaseRow {
        id: 6,
        host: 88,
        ..Default::default()
    };
    assert_eq!(count(&sources, &wing, &host_base), Ok(25));
    sources
        .modifiers
        .insert((AirSortieModifierRow::EFFICIENCY_OVERRIDE, 3, 0), -20_500);
    assert_eq!(count(&sources, &wing, &state), Ok(-21));
    sources
        .modifiers
        .insert((AirSortieModifierRow::EFFICIENCY_OVERRIDE, 3, 0), 12_500);
    let rocket = AirBaseRow {
        kind: 1,
        ..state.clone()
    };
    let rocket_host = AirSortieSpecialBaseRow {
        base: 5,
        rocket_slots: Some(9),
        ..Default::default()
    };
    sources.special.insert(5, &rocket_host);
    assert_eq!(count(&sources, &wing, &rocket), Ok(2));
    let gun = AirBaseRow { kind: 2, ..state };
    assert_eq!(count(&sources, &wing, &gun), Ok(1));
}

#[test]
fn only_the_effective_country_needs_its_container_fuel() {
    let host = AirSortieBaseCountryRow {
        base: 5,
        country: 4,
        capacity_raw: 10,
        ..Default::default()
    };
    let sources = SortieSources {
        missions: BTreeMap::new(),
        containers: BTreeMap::from([((5, 4), &host)]),
        modifiers: BTreeMap::new(),
        special: BTreeMap::new(),
        metadata: ModifierMetadata {
            flags: 0,
            feature_selector: 0,
            callback_kind: 0,
            feature_flags: 0,
        },
    };
    assert_eq!(sources.container(5, 4, false).map(|c| c.capacity), Ok(10));
    assert!(sources.container(5, 4, true).is_err());
    assert_eq!(sources.container(5, 7, true), Ok(Container::default()));
}

#[test]
fn a_non_positive_tag_with_a_region_is_refused_by_name() {
    let input = AirMissionInputRow::default();
    let sources = SortieSources {
        missions: BTreeMap::new(),
        containers: BTreeMap::new(),
        modifiers: BTreeMap::new(),
        special: BTreeMap::new(),
        metadata: ModifierMetadata {
            flags: 0,
            feature_selector: 0,
            callback_kind: 0,
            feature_flags: 0,
        },
    };
    let mut wing = AirWingRow {
        planes: 10,
        mission_region: Some(9),
        ..Default::default()
    };
    let base = AirBaseRow {
        state: Some(1),
        ..Default::default()
    };
    let count =
        |wing: &AirWingRow| sources.count(wing, &input, &base, 3, (0x8000_0003, 3), &defines());
    assert!(count(&wing).unwrap_err().contains("non-positive"));
    wing.mission_region = None;
    assert_eq!(count(&wing), Ok(0));
}

#[test]
fn an_in_combat_host_admits_only_bit_0_or_2_over_its_own_region() {
    let host = AirSortieSpecialBaseRow {
        base: 5,
        task_force_combats: Some(1),
        task_force_region: Some(90),
        ..Default::default()
    };
    let mut sources = SortieSources {
        missions: BTreeMap::new(),
        containers: BTreeMap::new(),
        modifiers: BTreeMap::new(),
        special: BTreeMap::from([(5, &host)]),
        metadata: ModifierMetadata {
            flags: 0,
            feature_selector: 0,
            callback_kind: 0,
            feature_flags: 0,
        },
    };
    let mut wing = AirWingRow {
        mission_mask: 1 << 2,
        mission_region: Some(90),
        ..Default::default()
    };
    let mut base = AirBaseRow {
        id: 5,
        ..Default::default()
    };
    assert_eq!(sources.host_admits(&wing, &base), Ok(true));
    wing.mission_region = Some(91);
    assert_eq!(sources.host_admits(&wing, &base), Ok(false));
    wing.mission_region = Some(90);
    wing.mission_mask = 1 << 4;
    assert_eq!(sources.host_admits(&wing, &base), Ok(false));
    let idle = AirSortieSpecialBaseRow {
        task_force_combats: Some(0),
        ..host.clone()
    };
    sources.special.insert(5, &idle);
    assert_eq!(sources.host_admits(&wing, &base), Ok(true));
    sources.special.clear();
    assert!(sources.host_admits(&wing, &base).is_err());
    base.state = Some(3);
    assert_eq!(sources.host_admits(&wing, &base), Ok(true));
}

#[test]
fn metadata_gates_and_clamps_entries() {
    let mut meta = ModifierMetadata {
        flags: 0x10,
        feature_selector: 0x08,
        callback_kind: 1,
        feature_flags: 0,
    };
    assert_eq!(meta.effective(150_000), 0);
    meta.feature_flags = 0x08;
    assert_eq!(meta.effective(150_000), ONE);
    assert_eq!(meta.effective(-1), 0);
    meta.flags = 0;
    assert_eq!(meta.effective(150_000), 150_000);
    meta.callback_kind = 0;
    assert_eq!(meta.effective(150_000), 0);
    meta.feature_selector = 0;
    assert_eq!(meta.effective(150_000), 150_000);
}
