use super::*;

const VIEWER: u32 = 7;

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
                    MISSILES_DIVISION_FACTOR = 60,
                    MAX_MISSILE_BOMBING_SCENARIOS = 2,
                    AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED = {{ 0.22 }},
                    AIRPLANES_CURVE_POINT_DENSITY = 2,
                    AIRPLANES_CURVE_MAX_EXTRAPOLATION = 20,
                    AIRPLANES_CURVE_MIN_ELEVATION = 4,
                    AIRPLANES_SCALE_TAKEOFF_DIST = 0.1,
                    AIRPLANES_SCALE_LANDING_DIST = 0.9,
                    AIRPLANES_SCALE_MIN = 0.1,
                    AIRPLANES_SMOOTH_INTERPOLATION_MOVE = 0.13,
                    AIRPLANES_SMOOTH_INTERPOLATION_TURN = 0.12,
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
            (101, 1, 3, 1),
            (201, 2, 1, 2),
            (301, 3, 3, 3),
            (401, 4, 3, 3),
        ]
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn a_region_with_bomber_or_fighter_wings_does_not_train() {
    let defines = flight_defines(2);
    let mut gate = open();
    for group in 2..5 {
        gate.reach(1, group);
    }
    gate.reach(2, 0);
    gate.reach(3, 1);
    let wings = [trainee(101, 60), trainee(201, 60), trainee(301, 60)];
    let mut diagnostics = Vec::new();
    assert_eq!(
        draws(&wings, &gate, &defines, &mut diagnostics),
        [(101, 1, 1, 2)]
    );
    assert!(diagnostics.is_empty());
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
    assert_eq!(flown.len(), 1);
    assert!((201..=203).contains(&flown[0].0));
    assert_eq!((flown[0].1, flown[0].2, flown[0].3), (2, 1, 2));
    assert_eq!(draws(&wings, &open(), &defines, &mut Vec::new()), flown);
    assert!(diagnostics.is_empty());
    let flown = draws(&wings[2..], &open(), &flight_defines(0), &mut diagnostics);
    assert_eq!(flown.len(), 1);
    assert_eq!(flown[0].3, 2);
}

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
    wings.remove(1);
    let flown = draws(&wings, &open(), &defines, &mut diagnostics);
    assert_eq!(flown.len(), GROUP_CAPACITY + 1);
    assert!(flown.iter().any(|draw| draw.0 == 100));
    assert!(diagnostics.is_empty());
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

#[test]
fn a_hidden_mission_leaves_every_other_flight_on_its_own_clock() {
    let defines = flight_defines(2);
    let pose = |x: f32| mesh::Transform {
        basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [x, 0.0, 0.0],
    };
    let rig = Rig {
        sides: vec![0],
        tracks: vec![(0..12).map(|frame| pose(frame as f32)).collect()],
        fps: 1.0,
        duration: 10.0,
        speed: 1.0,
        events: Vec::new(),
        unbound: vec![None],
        lands: vec![true],
        rockets: vec![false],
    };
    let rigs = BTreeMap::from([(String::new(), rig)]);
    let route = Route {
        points: Vec::new(),
        arcs: Vec::new(),
    };
    let flight = |mission, phase| Flight {
        mission,
        mesh: 0,
        wing: 1,
        region: 7,
        rig: String::new(),
        track: 0,
        origin: [0.0; 3],
        yaw: 0.0,
        outgoing: route.clone(),
        incoming: route.clone(),
        travel: 1.0,
        delay: 0.0,
        group_travel: 0.0,
        phase,
        requests: Arc::default(),
        impact: None,
        lands: true,
        explode: Explode::Absent,
        terrain: None,
    };
    let flights = [
        flight(AirMission::Patrol, 0.1),
        flight(AirMission::Bombing, 0.5),
        flight(AirMission::Supply, 0.7),
    ];
    let every = place_flights(&flights, &rigs, &defines, 3.0);
    assert_eq!(every.len(), 3);
    assert_eq!(every[2].1.animation_seed, 2);
    let hidden = hidden_flights(&flights, &every, &|mission| mission != AirMission::Bombing);
    let (seed, identity) = every[1].1.animation_key();
    assert_eq!(hidden, BTreeSet::from([(0, seed, identity)]));
    assert!(hidden_flights(&flights, &every, &|_| true).is_empty());
}
