use super::paired::defines;
use super::*;
use viewer_models::entity_animation::{Clip, Clock, Node, Player, Rig as ActorRig, StateTiming};

fn geometry() -> ProvinceGeometry {
    ProvinceGeometry::build(&MapData {
        width: 4,
        height: 2,
        province_ids: vec![1; 8],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    })
}

fn diving_rig(kind: u8, outcome: u8) -> Rig {
    let pose = |y: f32| Transform {
        translation: [0.0, y, 0.0],
        ..Transform::IDENTITY
    };
    let bones = scenario_bones(kind);
    Rig {
        tracks: vec![vec![pose(5.0), pose(2.0), pose(0.5), pose(-1.0)]],
        sides: vec![bones[0].0],
        fps: 1.0,
        duration: 3.0,
        events: Vec::new(),
        speed: 1.0,
        unbound: vec![None],
        lands: vec![lands(kind, outcome, bones[0].0)],
        rockets: vec![kind == scenarios::MISSILE_KIND],
    }
}

const BAND: Band = Band {
    offset: 1.0,
    explosion_delay: 0.6,
};

fn place(rig: &Rig, key: &str, explode: Explode) -> (AircraftPlan, Random) {
    let geometry = geometry();
    let heights = HeightField {
        width: 4,
        height: 2,
        values: &[100; 8],
    };
    let defines = defines();
    let placement = Placement {
        geometry: &geometry,
        heights: &heights,
        defines: &defines,
    };
    let start = [80.0, 10.0, 60.0];
    let batch = Batch {
        wing: 7,
        region: 9,
        key,
        mission: scenario_mission(16),
        rig,
        start,
        attacker: Some(Actor {
            mesh: 1,
            wing: 7,
            start,
            explode,
        }),
        defender: None,
    };
    let mut out = AircraftPlan::default();
    let mut random = Random::new(37);
    out.place(&batch, 1, &mut random, &placement, &mut Keeping::default());
    (out, random)
}

fn at_rig(plan: &AircraftPlan, rigs: &BTreeMap<String, Rig>, rig_time: f32) -> f32 {
    let defines = defines();
    let flight = &plan.flights[0];
    let total = flight.cycle(rigs[&flight.rig].duration);
    ((flight.group_travel + rig_time - flight.phase * total).rem_euclid(total) + total)
        / defines.clock_speed
}

fn shown(
    plan: &AircraftPlan,
    rigs: &BTreeMap<String, Rig>,
    rig_time: f32,
) -> Option<ModelInstance> {
    let defines = defines();
    place_flights(&plan.flights, rigs, &defines, at_rig(plan, rigs, rig_time))
        .into_iter()
        .find(|(index, _)| *index == 0)
        .map(|(_, instance)| instance)
}

#[test]
fn a_successful_kamikaze_explodes_in_the_band_and_does_not_fly_home() {
    let key = rig_key(scenario_name(16), SUCCESS);
    let rig = diving_rig(16, SUCCESS);
    assert_eq!(rig.lands, [false]);
    let rigs = BTreeMap::from([(key.clone(), diving_rig(16, SUCCESS))]);
    let defines = defines();
    let band = BAND;
    let frames = |plan: &AircraftPlan| -> Vec<f32> {
        (0..=180)
            .map(|frame| at_rig(plan, &rigs, frame as f32 / 60.0))
            .collect()
    };
    let (mut plan, _) = place(&rig, &key, Explode::Plays);
    assert!(!plan.flights[0].lands && plan.flights[0].terrain.is_some());
    assert_eq!(plan.flights[0].origin[1], 10.0);
    let mut entered = None;
    for frame in frames(&plan) {
        plan.advance(&rigs, &defines, Some(band), frame, &|_| None);
        if let (None, Ground::Exploding { pose, .. }) = (entered, plan.runs[0].grounds[0]) {
            entered = Some((frame, pose));
        }
    }
    let (frame, pose) = entered.unwrap();
    assert!((frame - at_rig(&plan, &rigs, 1.0 + 2.0 / 3.0)).abs() <= 1.0 / 60.0);
    assert!(pose.translation[1] <= 11.0 && pose.translation[1] > 10.9);
    assert_eq!(
        plan.placed_runs(&rigs, &defines, at_rig(&plan, &rigs, 2.9))[0]
            .1
            .position,
        pose.translation
    );
    assert!(
        plan.placed_runs(&rigs, &defines, at_rig(&plan, &rigs, 3.0 + 0.1))
            .is_empty()
    );
    assert_eq!(plan.runs[0].cycle, 1);
    let (mut plan, _) = place(&rig, &key, Explode::Plays);
    let mut ended = None;
    for frame in frames(&plan) {
        let cycle = plan.runs[0].cycle;
        plan.advance(&rigs, &defines, Some(band), frame, &|_| {
            Some(Some((0.6, 2.0)))
        });
        if ended.is_none() && plan.ran_at.is_some() && plan.runs[0].cycle > cycle && cycle > 0 {
            ended = Some(frame);
        }
    }
    let ended = ended.unwrap();
    assert!(ended > frame && ended - frame <= 1.5 / 60.0);
    let (mut plan, _) = place(&rig, &key, Explode::Absent);
    let mut ended = None;
    for frame in frames(&plan) {
        let cycle = plan.runs[0].cycle;
        plan.advance(&rigs, &defines, Some(band), frame, &|_| None);
        if ended.is_none() && plan.runs[0].cycle > cycle && cycle > 0 {
            ended = Some(frame);
        }
    }
    assert!((ended.unwrap() - frame).abs() < 1e-4);
}

#[test]
fn landing_scenarios_keep_their_draws_and_presentation() {
    for (kind, outcome) in [(0, 0), (16, 0), (6, 0), (19, 0)] {
        let key = rig_key(scenario_name(kind), outcome);
        let rig = diving_rig(kind, outcome);
        assert_eq!(rig.lands, [true], "{key}");
        let (plan, mut random) = place(&rig, &key, Explode::Absent);
        let mut expected = Random::new(37);
        for _ in 0..3 {
            expected.next();
        }
        assert_eq!(random.next(), expected.next(), "{key}");
        let flight = &plan.flights[0];
        assert!(flight.lands && flight.terrain.is_none(), "{key}");
        assert!(flight_requests(flight, &rig, &paired::actor_rig(), 2.0).is_empty());
        let rigs = BTreeMap::from([(key.clone(), diving_rig(kind, outcome))]);
        assert!((shown(&plan, &rigs, 2.5).unwrap().position[1] - 9.75).abs() < 1e-3);
        assert!(shown(&plan, &rigs, 3.0 + flight.travel * 0.5).is_some());
    }
}

#[test]
fn the_anchor_and_route_take_the_game_s_clamped_terrain_sample() {
    let values = [100, 140, 180, 220, 100, 140, 180, 220];
    let heights = HeightField {
        width: 4,
        height: 2,
        values: &values,
    };
    let geometry = geometry();
    let defines = defines();
    let placement = Placement {
        geometry: &geometry,
        heights: &heights,
        defines: &defines,
    };
    let key = rig_key(scenario_name(0), 0);
    let rig = diving_rig(0, 0);
    let start = [80.0, 10.0, 60.0];
    let batch = Batch {
        wing: 7,
        region: 9,
        key: &key,
        mission: scenario_mission(0),
        rig: &rig,
        start,
        attacker: Some(Actor {
            mesh: 1,
            wing: 7,
            start,
            explode: Explode::Absent,
        }),
        defender: None,
    };
    let mut plan = AircraftPlan::default();
    plan.place(&batch, 1, &mut Random::new(37), &placement, &mut Keeping::default());
    let [x, _, z] = plan.flights[0].origin;
    assert_eq!(plan.flights[0].origin[1], heights.native_height(x, z));
    assert_ne!(heights.native_height(x, z), heights.height_at_world(x, z));
    let route = Route::build(
        [0.6, 0.0, 0.5],
        [2.6, 0.0, 0.5],
        [0.0; 3],
        true,
        &heights,
        &defines,
    );
    let interior = &route.points[1..route.points.len() - 1];
    assert!(!interior.is_empty());
    for point in interior {
        assert_eq!(
            point[1],
            heights.native_height(point[0], point[2]) + defines.curve_elevation
        );
    }
    assert!(
        interior
            .iter()
            .any(|point| heights.native_height(point[0], point[2])
                != heights.height_at_world(point[0], point[2]))
    );
}

fn clip(name: &str, seconds: u32, speed: f32, start: f32, looping: bool) -> Clip {
    let mut animation = mesh::Animation::default();
    animation.fps = 1.0;
    animation.samples = seconds + 1;
    Clip {
        state: mesh::EntityState {
            name: name.into(),
            chance: 1.0,
            speed,
            start,
            looping,
            ..mesh::EntityState::default()
        },
        animation: Some(Arc::new(animation)),
    }
}

fn actor(clips: Vec<Clip>) -> ActorRig {
    ActorRig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: None,
            clips,
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        start: 0,
        end: 1,
    }
}

#[test]
fn the_explode_state_plays_the_band_delay_as_the_player_reports_it() {
    let rig = actor(vec![
        clip("idle", 3, 1.0, 0.0, true),
        clip("explode", 2, 2.0, 0.5, false),
    ]);
    let timing = StateTiming {
        start: 0.5,
        speed: 2.0,
        length: 2.0,
        looping: false,
    };
    let (from, until) = timing.reads_at_least(2.4).unwrap();
    assert!((from - 0.95).abs() < 1e-6 && until == Some(1.0));
    assert_eq!(timing.reads_at_least(2.0), Some((0.75, None)));
    assert_eq!(timing.reaches(0.4), Some(0.0));
    assert!((timing.reaches(0.6).unwrap() - 0.05).abs() < 1e-6);
    assert_eq!(timing.reaches(2.0), Some(0.75));
    assert!((timing.reaches(2.4).unwrap() - 0.95).abs() < 1e-6);
    assert_eq!(timing.reaches(2.6), None);
    let entry = 1.0;
    let requests = Arc::new(vec![viewer_models::entity_animation::Request {
        time: entry,
        phase_time: None,
        state: EXPLODE_STATE.into(),
    }]);
    let mut player = Player::new(&rig);
    let mut palette = [viewer_models::PaletteJoint::from_transform(
        &Transform::IDENTITY,
    )];
    for step in 0..=40 {
        let time = step as f32 * 0.05;
        player.evaluate_clock(
            &rig,
            ModelInstance::default(),
            &Clock {
                time,
                phase: None,
                requests: Arc::clone(&requests),
            },
            &mut palette,
            &mut Vec::new(),
        );
        if time < entry + 1e-4 {
            continue;
        }
        let (position, _) = player.root_play_position(&rig).unwrap();
        let age = time - entry;
        for threshold in [0.6, 1.9, 2.0, 2.4] {
            let (from, until) = timing.reads_at_least(threshold).unwrap();
            let edges = [Some(from), until];
            if edges.iter().flatten().all(|edge| (age - edge).abs() > 1e-3) {
                assert_eq!(
                    position >= threshold - 1e-4,
                    from < age && until.is_none_or(|until| age < until),
                    "threshold {threshold} at {time}: position {position}"
                );
            }
        }
    }
    let (read, _, _) = player.root_play_clip(&rig).unwrap();
    assert_eq!(read, timing);
    assert_eq!(explode(&rig), Explode::Plays);
    assert_eq!(explode(&actor(Vec::new())), Explode::Absent);
}

#[test]
fn a_negative_launch_delay_reads_as_zero() {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "fixture",
            b"NDefines = { NAirGfx = { RANDOM_SCENARIO_ANIMATION_MAX_DELAY = -3.0 } }",
        )
        .unwrap();
    assert_eq!(launches::max_delay(&defines), Ok(0.0));
    defines
        .load(
            "fixture",
            b"NDefines = { NAirGfx = { RANDOM_SCENARIO_ANIMATION_MAX_DELAY = 3.0 } }",
        )
        .unwrap();
    assert_eq!(launches::max_delay(&defines), Ok(3.0));
}
