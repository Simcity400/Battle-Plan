use super::*;

const VIEWER: u32 = 7;

/// Every `NAirGfx` define the flight builder reads, with the training
/// operands distinct from their neighbours'.
fn flight_defines(max_training: i32) -> FlightDefines {
    let defines = Defines::new().unwrap();
    let source = format!(
        "NDefines = {{ NAirGfx = {{
                    FIGHTERS_DIVISION_FACTOR = 20,
                    AIRPLANES_1_FIGHTER_PATROL_ANIM = 1,
                    AIRPLANES_3_FIGHTER_PATROL_ANIM = 3,
                    MAX_PATROL_SCENARIOS = 7,
                    SCOUT_PLANE_DIVISION_FACTOR = 10,
                    AIRPLANES_1_SCOUT_PLANE_PATROL_ANIM = 1,
                    AIRPLANES_3_SCOUT_PLANE_PATROL_ANIM = 3,
                    MAX_SCOUT_SCENARIOS = 7,
                    BOMBERS_DIVISION_FACTOR = 40,
                    AIRPLANES_3_BOMBER_BOMBING_ANIM = 3,
                    AIRPLANES_1_BOMBER_BOMBING_ANIM = 1,
                    AIRPLANES_1_FIGHTER_VS_1_FIGHTER_ANIM = 1,
                    AIRPLANES_3_FIGHTER_VS_3_FIGHTER_ANIM = 3,
                    MAX_DOGFIGHTS_SCENARIOS = 7,
                    STRAT_BOMBER_FIREBOMB_THRESHOLD = 2,
                    STRAT_BOMBER_CARPETBOMB_THRESHOLD = 1,
                    MAX_BOMBING_SCENARIOS = 7,
                    TRANSPORT_DIVISION_FACTOR = 30,
                    AIRPLANES_1_TRANSPORT_SUPPLY_ANIM = 2,
                    AIRPLANES_3_TRANSPORT_SUPPLY_ANIM = 5,
                    MAX_TRANSPORT_SCENARIOS = 7,
                    MAX_TRAINING_SCENARIOS = {max_training},
                    AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED = {{ 0.22 }},
                    AIRPLANES_CURVE_POINT_DENSITY = 2,
                    AIRPLANES_CURVE_MAX_EXTRAPOLATION = 20,
                    AIRPLANES_CURVE_MIN_ELEVATION = 4,
                    AIRPLANES_SCALE_TAKEOFF_DIST = 0.1,
                    AIRPLANES_SCALE_LANDING_DIST = 0.9,
                    AIRPLANES_SCALE_MIN = 0.1,
                }} }}"
    );
    defines.load("test.lua", source.as_bytes()).unwrap();
    FlightDefines::load(&defines).unwrap()
}

fn trainee(id: u64, planes: i32) -> AirWingRow {
    AirWingRow {
        id,
        owner: VIEWER,
        planes,
        mission_mask: AIR_MISSION_TRAINING,
        ..Default::default()
    }
}

fn open() -> TrainingGate {
    TrainingGate {
        participants: true,
        ..Default::default()
    }
}

/// Wing ID `n` stands in region `n / 100`; a pool base handle
/// marks one whose base does not resolve.
fn draws(
    wings: &[AirWingRow],
    gate: &TrainingGate,
    defines: &FlightDefines,
    diagnostics: &mut Vec<String>,
) -> Vec<(u64, u32, usize, usize)> {
    let mut counts = BTreeMap::new();
    training_draws(
        wings,
        |wing| wing.owner == VIEWER,
        |wing| match wing.air_base {
            0 => Ok(Some(((wing.id / 100) as u32, ()))),
            _ => Err("its pool base does not resolve"),
        },
        gate,
        defines.training,
        &mut counts,
        diagnostics,
    )
    .into_iter()
    .map(|draw| (draw.wing.id, draw.region, draw.formation, draw.batches))
    .collect()
}

#[test]
fn regions_with_empty_arrays_fly_planes_over_the_transport_factor_as_fighter_patrols() {
    let defines = flight_defines(2);
    // The transport factor with the fighter-patrol steps and limit.
    assert_eq!(
        defines.training,
        SteppedFamily {
            sizing: FamilyDefines {
                factor: 30,
                triple: 3,
                limit: 2,
            },
            single: 1,
        }
    );
    let mut foreign = trainee(102, 300);
    foreign.owner = 8;
    // An override flies with its pool, not its country.
    foreign.country_override = VIEWER as i32;
    let mut lent = trainee(401, 300);
    lent.country_override = 8;
    let untrained = AirWingRow {
        mission_mask: 1 << 2,
        ..trainee(204, 300)
    };
    let wings = [
        trainee(101, 100),
        foreign,
        trainee(201, 60),
        trainee(301, 300),
        lent,
        untrained,
    ];
    let mut diagnostics = Vec::new();
    assert_eq!(
        draws(&wings, &open(), &defines, &mut diagnostics),
        [
            // 100 / 30 = 3 units: one triple.
            (101, 1, 3, 1),
            // 60 / 30 = 2 units: two singles.
            (201, 2, 1, 2),
            // 300 / 30 = 10 units: three triples, beyond the limit
            // of two, which is checked before the batch.
            (301, 3, 3, 3),
            // A country override leaves the wing in its owner's pool.
            (401, 4, 3, 3),
        ]
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn a_region_with_bomber_or_fighter_wings_does_not_train() {
    let defines = flight_defines(2);
    let mut gate = open();
    // Base-equipment, transport and scout arrays leave it open.
    for group in 2..5 {
        gate.reach(1, group);
    }
    // Either side's bombers or fighters close it, grouped or full.
    gate.reach(2, 0);
    gate.reach(3, 1);
    let wings = [trainee(101, 60), trainee(201, 60), trainee(301, 60)];
    let mut diagnostics = Vec::new();
    assert_eq!(
        draws(&wings, &gate, &defines, &mut diagnostics),
        [(101, 1, 1, 2)]
    );
    assert!(diagnostics.is_empty());
    // A wing omitted before its array is known leaves a region
    // undecided; one decided outside those arrays does not.
    let mut gate = open();
    gate.undecided(1, Some(3));
    gate.undecided(2, Some(1));
    gate.undecided(3, None);
    let mut diagnostics = Vec::new();
    assert_eq!(
        draws(&wings, &gate, &defines, &mut diagnostics),
        [(101, 1, 1, 2)]
    );
    assert_eq!(
        diagnostics,
        [
            "training omitted in 2 regions: a wing that may fill their bomber or fighter arrays is undecided"
        ]
    );
    // Unknown participant arrays leave every open region unknown.
    let mut gate = TrainingGate::default();
    gate.reach(3, 0);
    let mut diagnostics = Vec::new();
    assert!(draws(&wings, &gate, &defines, &mut diagnostics).is_empty());
    assert_eq!(
        diagnostics,
        [
            "2 training wings omitted: the participant arrays that decide their regions' training gate are unknown"
        ]
    );
}

#[test]
fn a_wing_below_a_single_or_a_refused_batch_ends_the_regions_pass() {
    let defines = flight_defines(2);
    let wings = [
        trainee(101, 0),
        trainee(102, 0),
        trainee(201, 60),
        trainee(202, 60),
        trainee(203, 60),
    ];
    let mut diagnostics = Vec::new();
    let flown = draws(&wings, &open(), &defines, &mut diagnostics);
    // Either zero-unit wing ends region1. The first chosen two-single
    // batch fills the limit in region2; its next batch ends that pass.
    assert_eq!(flown.len(), 1);
    assert!((201..=203).contains(&flown[0].0));
    assert_eq!((flown[0].1, flown[0].2, flown[0].3), (2, 1, 2));
    assert_eq!(draws(&wings, &open(), &defines, &mut Vec::new()), flown);
    assert!(diagnostics.is_empty());
    // Native admits one batch against a nonpositive scenario limit.
    let flown = draws(&wings[2..], &open(), &flight_defines(0), &mut diagnostics);
    assert_eq!(flown.len(), 1);
    assert_eq!(flown[0].3, 2);
}

/// Region 1 lists 51 candidates whose pool order runs against their
/// IDs: the first50 in pool order fly with deterministic random selection;
/// the lowest ID, last in its pools, stays on the ground.
#[test]
fn the_first_fifty_in_pool_order_fly_with_repeatable_random_removal() {
    let defines = flight_defines(1_000);
    let last = GROUP_CAPACITY as u64;
    let mut wings: Vec<_> = (0..=last)
        .map(|index| AirWingRow {
            pool_ordinal: (last - index) as u32,
            ..trainee(100 + index, 30)
        })
        .chain([trainee(201, 30)])
        .collect();
    let mut diagnostics = Vec::new();
    let flown = draws(&wings, &open(), &defines, &mut diagnostics);
    let ids: Vec<u64> = flown.iter().map(|draw| draw.0).collect();
    let expected: Vec<u64> = (101..=150).chain([201]).collect();
    assert_ne!(ids, expected);
    let mut sorted = ids;
    sorted.sort_unstable();
    assert_eq!(sorted, expected);
    assert_eq!(draws(&wings, &open(), &defines, &mut Vec::new()), flown);
    assert!(diagnostics.is_empty());
    // Exactly 50 in pool order all fly.
    wings.remove(1);
    let flown = draws(&wings, &open(), &defines, &mut diagnostics);
    assert_eq!(flown.len(), GROUP_CAPACITY + 1);
    assert!(flown.iter().any(|draw| draw.0 == 100));
    assert!(diagnostics.is_empty());
    // A trainee whose base does not resolve could hold one of the
    // 50 places in the game, so the full region is counted.
    wings.push(AirWingRow {
        air_base: 99,
        ..trainee(999, 30)
    });
    let flown = draws(&wings, &open(), &defines, &mut diagnostics);
    assert_eq!(flown.len(), GROUP_CAPACITY + 1);
    assert_eq!(
        diagnostics,
        [
            "training wing 999 omitted: its pool base does not resolve",
            "1 regions list at least 50 training wings beside 1 whose pool base does not resolve; one of those may take a place among the 50 that fly"
        ]
    );
}

#[test]
fn an_unresolved_pool_base_is_named_and_the_others_fly() {
    let defines = flight_defines(2);
    let lost = AirWingRow {
        air_base: 99,
        ..trainee(101, 60)
    };
    let wings = [lost, trainee(102, 30)];
    let mut diagnostics = Vec::new();
    assert_eq!(
        draws(&wings, &open(), &defines, &mut diagnostics),
        [(102, 1, 1, 1)]
    );
    assert_eq!(
        diagnostics,
        ["training wing 101 omitted: its pool base does not resolve"]
    );
}

#[test]
fn a_carrier_base_stands_where_its_task_force_is() {
    let registry = AirRegistrySources {
        hosts_without_task_force: BTreeSet::from([3]),
        ..Default::default()
    };
    let ships = [
        ShipRow {
            id: 50,
            task_force: 60,
            ..Default::default()
        },
        ShipRow {
            id: 51,
            task_force: 61,
            ..Default::default()
        },
    ];
    let task_forces = [
        TaskForceRow {
            id: 60,
            province: 412,
            ..Default::default()
        },
        TaskForceRow {
            id: 61,
            ..Default::default()
        },
    ];
    let carriers = CarrierSources {
        ships: Some(&ships),
        task_forces: Some(&task_forces),
        registry: &registry,
        origin: &|_| Err("unused training province lookup"),
    };
    let base = |id, host| AirBaseRow {
        id,
        host,
        ..Default::default()
    };
    assert_eq!(carriers.province(&base(1, 50)), Ok(412));
    assert!(carriers.province(&base(2, 51)).is_err());
    assert!(carriers.province(&base(3, 50)).is_err());
    assert!(carriers.province(&base(4, 52)).is_err());
}
