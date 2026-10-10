use super::super::super::entity_animation::{Clip, Node, Rig as ActorRig};
use super::*;

fn defines() -> FlightDefines {
    let sizing = FamilyDefines {
        factor: 1,
        triple: 3,
        limit: 2,
    };
    let stepped = SteppedFamily { sizing, single: 1 };
    FlightDefines {
        fighter: stepped,
        scout: stepped,
        bomber: sizing,
        bomber_single: 1,
        dogfight: stepped,
        transport: stepped,
        training: stepped,
        fire_threshold: 300_000,
        carpet_threshold: 200_000,
        clock_speed: 2.0,
        curve_density: 2.0,
        curve_extrapolation: 20.0,
        curve_elevation: 4.0,
        scale_takeoff: 0.1,
        scale_landing: 0.9,
        scale_min: 0.1,
    }
}

fn actor_rig() -> ActorRig {
    let descriptors = mesh::parse_descriptors(br#"
        entity = { name = actor state = { name = idle } state = { name = success } state = { name = fail } }
    "#, "synthetic.asset").unwrap();
    let actor = descriptors.inherit("actor").unwrap();
    ActorRig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: None,
            clips: actor
                .states
                .values()
                .flatten()
                .map(|state| Clip {
                    state: (*state).clone(),
                    animation: None,
                })
                .collect(),
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        ..Default::default()
    }
}

fn paired_rig(kind: u8) -> Rig {
    let descriptors = mesh::parse_descriptors(
        br#"
        entity = { name = scenario state = { name = default
            event = { time = 2 id = a_1_success }
            event = { time = 3 id = a_2_fail }
            event = { time = 4 id = d_1_fail }
            event = { time = 5 id = d_2_success }
        } }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let bones = scenario_bones(kind);
    let tracks = bones
        .iter()
        .enumerate()
        .map(|(track, _)| {
            let first = Transform {
                translation: [track as f32 * 2.0, 10.0, 0.0],
                ..Transform::IDENTITY
            };
            let mut last = first;
            // Give each side's tracks different travel times, so synchronization
            // must use the surviving tracks rather than a missing-side maximum.
            last.translation[2] += (track + 1) as f32;
            vec![first, last]
        })
        .collect();
    Rig {
        sides: bones.iter().map(|bone| bone.0).collect(),
        tracks,
        fps: 1.0,
        duration: 10.0,
        speed: 1.0,
        events: descriptors.inherit("scenario").unwrap().states["default"][0]
            .events
            .clone(),
    }
}

#[test]
fn paired_placement_keeps_each_surviving_side_and_its_native_callbacks() {
    let geometry = ProvinceGeometry::build(&MapData {
        width: 4,
        height: 2,
        province_ids: vec![1; 8],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    });
    let heights = HeightField {
        width: 2,
        height: 2,
        values: &[0; 4],
    };
    let defines = defines();
    let placement = Placement {
        geometry: &geometry,
        heights: &heights,
        defines: &defines,
    };
    let actor = actor_rig();
    let reference = [20.0, 10.0, 30.0];
    for kind in [12, 13, 14, 18] {
        let rig = paired_rig(kind);
        let key = rig_key(scenario_name(kind), 1);
        let mut paired_yaw = None;
        for (attack, defend) in [(true, true), (true, false), (false, true), (false, false)] {
            let batch = Batch {
                wing: 7,
                region: 9,
                key: &key,
                rig: &rig,
                start: reference,
                attacker: attack.then_some(Actor {
                    mesh: 1,
                    wing: 7,
                    start: reference,
                }),
                defender: defend.then_some(Actor {
                    mesh: 2,
                    wing: 8,
                    start: [-40.0, 10.0, -60.0],
                }),
            };
            let mut out = AircraftPlan::default();
            out.place(&batch, 1, &mut Random::new(37), &placement);
            let expected: Vec<_> = rig
                .sides
                .iter()
                .enumerate()
                .filter_map(|(track, &side)| {
                    (if side == 0 { attack } else { defend }).then_some(track)
                })
                .collect();
            assert_eq!(
                out.flights
                    .iter()
                    .map(|flight| flight.track)
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(out.diagnostics.is_empty());
            let maximum = out
                .flights
                .iter()
                .map(|flight| flight.travel)
                .fold(0.0_f32, f32::max);
            for flight in &out.flights {
                let side = rig.sides[flight.track];
                assert_eq!(
                    (flight.mesh, flight.wing),
                    if side == 0 { (1, 7) } else { (2, 8) }
                );
                assert_eq!(flight.group_travel, maximum);
                assert!((flight.travel + flight.delay - maximum).abs() < 1e-5);
                let direction = sub(flight.origin, reference);
                let yaw = -direction[0].atan2(direction[2]);
                assert!((flight.yaw - yaw).abs() >= 10.0_f32.to_radians());
                assert!((flight.yaw - yaw).abs() <= 40.0_f32.to_radians());
                assert_eq!(flight.yaw, *paired_yaw.get_or_insert(flight.yaw));
                let ordinal = rig.sides[..flight.track]
                    .iter()
                    .filter(|&&member| member == side)
                    .count();
                let requests = flight_requests(flight, &rig, &actor, defines.clock_speed);
                let count = rig.sides.iter().filter(|&&member| member == side).count();
                let expected = match (count, side, ordinal) {
                    // Out-of-range numeric prefixes broadcast in the native
                    // callback, so member 2 also addresses a single actor.
                    (1, 0, 0) => vec![("success", 2.0), ("fail", 3.0)],
                    (1, 1, 0) => vec![("fail", 4.0), ("success", 5.0)],
                    (_, 0, 0) => vec![("success", 2.0)],
                    (_, 0, 1) => vec![("fail", 3.0)],
                    (_, 1, 0) => vec![("fail", 4.0)],
                    (_, 1, 1) => vec![("success", 5.0)],
                    _ => Vec::new(),
                };
                assert_eq!(requests.len(), expected.len());
                for (request, (state, time)) in requests.iter().zip(expected) {
                    assert_eq!(request.state, state);
                    assert!(
                        (request.time - (maximum + time - flight.delay) / defines.clock_speed)
                            .abs()
                            < 1e-5
                    );
                }
            }
        }
    }
}
