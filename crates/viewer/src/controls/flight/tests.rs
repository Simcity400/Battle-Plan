use super::*;
use crate::controls::Pad;
use eframe::egui;
use std::collections::BTreeSet;
use std::f32::consts::FRAC_PI_2;
use viewer_gpu::camera::{MapDefines, MapSize};

const SIZE: f32 = 1.2;
const STEP: f32 = 1.0 / 60.0;
const PLANE: Airframe = Airframe {
    size: SIZE,
    explodes: true,
    ground: GroundContact {
        offset: 0.0,
        explosion: 0.6,
    },
};

fn base() -> Camera {
    Camera {
        center: [3000.0, 1000.0],
        height: 300.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

fn flat(_: f32, _: f32) -> f32 {
    WATER_HEIGHT
}

fn keys(held: &[egui::Key]) -> Frame {
    Frame {
        dt: STEP,
        held: held.iter().copied().collect::<BTreeSet<_>>(),
        ..Frame::default()
    }
}

fn idle() -> Frame {
    keys(&[])
}

fn fly(flight: &mut Flight, frame: &Frame, floor: Floor<'_>, seconds: f32) {
    for _ in 0..(seconds / frame.dt).round() as usize {
        flight.step(frame, &base(), floor, &PLANE);
    }
}

fn level(height: f32) -> Flight {
    Flight::new([3000.0, height, 1000.0], 0.0, 0.0, 0.0)
}

#[test]
fn with_nothing_held_it_flies_level_ahead_at_the_throttle_speed() {
    let mut flight = level(200.0);
    fly(&mut flight, &idle(), &flat, 2.0);
    assert!(flight.yaw.abs() < 1e-6 && flight.pitch.abs() < 1e-6 && flight.roll.abs() < 1e-6);
    let travelled = flight.position[2] - 1000.0;
    assert!(
        (travelled - 2.0 * cruise(START_THROTTLE)).abs() < 0.05,
        "{travelled}"
    );
    assert!((flight.position[0] - 3000.0).abs() < 1e-3);
    assert!((flight.position[1] - 200.0).abs() < 1e-3);
}

#[test]
fn steering_right_banks_into_a_right_turn_and_letting_go_levels_the_wings() {
    let mut flight = level(200.0);
    fly(&mut flight, &keys(&[egui::Key::D]), &flat, 1.0);
    assert!(flight.yaw > 0.5, "{}", flight.yaw);
    assert!(flight.roll > 0.5 * MAX_BANK, "{}", flight.roll);
    assert!(flight.position[0] > 3000.0);
    fly(&mut flight, &idle(), &flat, 3.0);
    assert!(flight.roll.abs() < 0.02, "{}", flight.roll);
    let heading = flight.yaw;
    fly(&mut flight, &idle(), &flat, 1.0);
    assert!((flight.yaw - heading).abs() < 2e-3);
    for frame in [
        keys(&[egui::Key::ArrowLeft]),
        Frame {
            pad: Pad {
                left: [-1.0, 0.0],
                ..Pad::default()
            },
            ..idle()
        },
    ] {
        let mut flight = level(200.0);
        fly(&mut flight, &frame, &flat, 1.0);
        assert!(flight.yaw < -0.5 && flight.roll < 0.0);
    }
}

#[test]
fn the_pointer_moves_the_steering_point_by_its_motion_and_the_aircraft_follows() {
    let mut flight = level(200.0);
    let moved = Frame {
        look: [100.0, -40.0],
        captured: true,
        ..idle()
    };
    flight.step(&moved, &base(), &flat, &PLANE);
    assert!((flight.aim[0] - 100.0 * MOUSE_AIM).abs() < 1e-5);
    assert!((flight.aim[1] - 40.0 * MOUSE_AIM).abs() < 1e-5);
    fly(&mut flight, &idle(), &flat, 2.0);
    assert!(
        (flight.yaw - 100.0 * MOUSE_AIM).abs() < 0.01,
        "{}",
        flight.yaw
    );
    assert!(flight.position[1] > 200.0);
    for pixels in [1_500.0, 1.0e5, -1_500.0] {
        let mut flight = level(200.0);
        flight.step(
            &Frame {
                look: [pixels, 0.0],
                ..idle()
            },
            &base(),
            &flat,
            &PLANE,
        );
        let lead = wrap(flight.aim[0] - flight.yaw) * f32::signum(pixels);
        assert!(
            (MAX_LEAD - TURN_RATE * STEP - 1e-4..=MAX_LEAD + 1e-4).contains(&lead),
            "{pixels}: {lead}"
        );
        assert_eq!(flight.yaw.signum(), f32::signum(pixels), "{pixels}");
    }
}

#[test]
fn a_climb_holds_its_pitch_after_the_controls_are_let_go() {
    let mut flight = level(200.0);
    fly(&mut flight, &keys(&[egui::Key::ArrowUp]), &flat, 0.5);
    let aim = flight.aim[1];
    assert!(aim > 0.4 && flight.pitch > 0.2);
    fly(&mut flight, &idle(), &flat, 3.0);
    assert_eq!(flight.aim[1], aim);
    assert!((flight.pitch - aim).abs() < 0.01, "{}", flight.pitch);
    let mut flight = level(200.0);
    fly(&mut flight, &keys(&[egui::Key::ArrowDown]), &flat, 0.5);
    let aim = flight.aim[1];
    fly(&mut flight, &idle(), &flat, 2.0);
    assert_eq!(flight.aim[1], aim);
    assert!(flight.pitch < -0.2 && flight.position[1] < 200.0);
}

#[test]
fn flying_level_into_high_ground_crashes_into_it() {
    let wall = |_: f32, z: f32| if z > 1100.0 { 25.5 } else { WATER_HEIGHT };
    let mut flight = level(WATER_HEIGHT + 3.0 * clearance(SIZE));
    for _ in 0..600 {
        flight.step(&idle(), &base(), &wall, &PLANE);
        if flight.phase != Phase::Flying {
            break;
        }
        assert_eq!(flight.pitch, 0.0);
    }
    assert!(
        matches!(flight.phase, Phase::Crashed { .. }),
        "{:?}",
        flight.phase
    );
    assert!(flight.position[2] <= 1100.0 + SIZE, "{:?}", flight.position);
}

#[test]
fn it_never_stalls_and_the_throttle_and_boost_set_its_speed() {
    let mut flight = level(200.0);
    fly(
        &mut flight,
        &keys(&[egui::Key::S, egui::Key::ArrowUp]),
        &flat,
        8.0,
    );
    assert_eq!(flight.throttle, 0.0);
    assert!(flight.speed >= SLOWEST);
    let at = flight.position;
    fly(&mut flight, &idle(), &flat, 1.0);
    assert!(
        (0..3)
            .map(|axis| (flight.position[axis] - at[axis]).powi(2))
            .sum::<f32>()
            .sqrt()
            >= SLOWEST * 0.99
    );
    fly(
        &mut flight,
        &keys(&[egui::Key::W]),
        &flat,
        1.0 / THROTTLE_RATE + 8.0,
    );
    assert_eq!(flight.throttle, 1.0);
    assert!((flight.speed - FASTEST).abs() < 0.1, "{}", flight.speed);
    let boost = Frame {
        modifiers: egui::Modifiers::CTRL,
        ..idle()
    };
    fly(&mut flight, &boost, &flat, 8.0);
    assert!(
        (flight.speed - FASTEST * BOOST).abs() < 0.5,
        "{}",
        flight.speed
    );
    let mut pad = level(200.0);
    let triggers = Frame {
        pad: Pad {
            triggers: [0.0, 1.0],
            ..Pad::default()
        },
        ..idle()
    };
    fly(&mut pad, &triggers, &flat, 1.0);
    assert!((pad.throttle - (START_THROTTLE + THROTTLE_RATE)).abs() < 0.02);
}

#[test]
fn the_ceiling_and_the_map_edges_hold_it_and_east_and_west_wrap_round() {
    let map = base();
    let mut flight = level(2900.0);
    fly(
        &mut flight,
        &keys(&[egui::Key::ArrowUp, egui::Key::W]),
        &flat,
        20.0,
    );
    assert!(flight.position[1] <= map.defines.heights[1] + 1e-3);
    let mut north = Flight::new([3000.0, 200.0, 2000.0], 0.0, 0.0, 0.0);
    fly(&mut north, &keys(&[egui::Key::W]), &flat, 5.0);
    assert!(north.position[2] <= map.map.y());
    let mut east = Flight::new([map.map.x() - 5.0, 200.0, 1000.0], FRAC_PI_2, 0.0, 0.0);
    fly(&mut east, &idle(), &flat, 2.0);
    assert!(east.position[0] < 50.0, "{}", east.position[0]);
}

#[test]
fn the_chase_view_draws_the_aircraft_ahead_of_an_eye_behind_and_above_it() {
    let mut flight = level(200.0);
    flight.step(&idle(), &base(), &flat, &PLANE);
    let sight = flight.sight(base(), &flat, &PLANE);
    let eye = sight.camera.eye();
    let aircraft = sight.aircraft.expect("the chase view draws the aircraft");
    assert_eq!(aircraft.position, flight.position);
    assert!(eye[2] < flight.position[2] - SIZE * CHASE_DISTANCE * 0.9);
    assert!(eye[1] > flight.position[1]);
    let viewport = [1600, 900];
    let [x, y] = sight
        .camera
        .project(flight.position, viewport)
        .expect("in front of the eye");
    assert!((x - 800.0).abs() < 1.0, "{x}");
    assert!(y > 450.0 && y < 700.0, "{y}");
    assert_eq!(sight.camera.lens().fov_y, FOV_Y);
}

#[test]
fn the_nose_view_looks_out_along_the_attitude_and_draws_no_aircraft() {
    let mut flight = level(200.0);
    flight.step(
        &Frame {
            pressed: vec![egui::Key::V],
            ..idle()
        },
        &base(),
        &flat,
        &PLANE,
    );
    assert_eq!(flight.view, View::Nose);
    flight.yaw = 0.4;
    flight.pitch = 0.1;
    flight.roll = 0.3;
    let sight = flight.sight(base(), &flat, &PLANE);
    assert!(sight.aircraft.is_none());
    let free = *sight.camera.free().expect("a free eye");
    assert_eq!([free.yaw, free.pitch, free.roll], [0.4, 0.1, 0.3]);
    let ahead = free.forward();
    let eye = sight.camera.eye();
    for axis in 0..3 {
        let expected = flight.position[axis] + ahead[axis] * SIZE * NOSE;
        assert!((eye[axis] - expected).abs() < 1e-3);
    }
    flight.step(
        &Frame {
            pad: Pad {
                pressed: GamepadButton::North.bit(),
                held: GamepadButton::North.bit(),
                ..Pad::default()
            },
            ..idle()
        },
        &base(),
        &flat,
        &PLANE,
    );
    assert_eq!(flight.view, View::Chase);
}

#[test]
fn across_the_seam_the_aircraft_stands_beside_the_eye() {
    let mut flight = Flight::new([1.0, 200.0, 1000.0], FRAC_PI_2, 0.0, 0.0);
    flight.step(&Frame { dt: 0.0, ..idle() }, &base(), &flat, &PLANE);
    let sight = flight.sight(base(), &flat, &PLANE);
    let eye = sight.camera.eye();
    assert!(eye[0] >= 0.0 && eye[0] < base().map.x());
    let aircraft = sight.aircraft.expect("drawn");
    assert!((aircraft.position[0] - eye[0]).abs() < SIZE * CHASE_DISTANCE * 1.1);
    assert!((aircraft.position[0] - (1.0 + base().map.x())).abs() < 1e-3);
}

#[test]
fn the_instance_points_the_nose_along_the_heading_as_the_game_poses_planes() {
    let mut flight = Flight::new([3000.0, 200.0, 1000.0], FRAC_PI_2, 0.0, 0.0);
    flight.roll = 0.5;
    let aircraft = flight.instance(flight.position);
    assert_eq!(aircraft.native_flags, 4);
    let [left, up, tail] = aircraft.native_basis;
    assert!((tail[0] + 1.0).abs() < 1e-5 && tail[1].abs() < 1e-5);
    assert!(left[1] > 0.4 && left[2] > 0.0);
    assert!(up[1] > 0.8);
    let determinant = left[0] * (up[1] * tail[2] - up[2] * tail[1])
        - left[1] * (up[0] * tail[2] - up[2] * tail[0])
        + left[2] * (up[0] * tail[1] - up[1] * tail[0]);
    assert!((determinant - 1.0).abs() < 1e-4);
    assert_eq!(
        (aircraft.animation_seed, aircraft.animation_identity),
        (ANIMATION_SEED, 1)
    );
}

#[test]
fn looking_around_turns_the_chase_camera_and_it_springs_back() {
    let mut flight = level(200.0);
    flight.step(&idle(), &base(), &flat, &PLANE);
    let relative = |flight: &Flight| {
        let eye = flight.sight(base(), &flat, &PLANE).camera.eye();
        [eye[0] - flight.position[0], eye[2] - flight.position[2]]
    };
    let ahead = relative(&flight);
    let stick = Frame {
        pad: Pad {
            right: [1.0, 0.0],
            ..Pad::default()
        },
        ..idle()
    };
    fly(&mut flight, &stick, &flat, 1.0);
    let turned = flight.sight(base(), &flat, &PLANE).camera.eye();
    assert!(turned[0] < flight.position[0] - SIZE, "{turned:?}");
    fly(&mut flight, &idle(), &flat, 1.5);
    let [a, b] = [relative(&flight), ahead];
    assert!(
        (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05,
        "{a:?} {b:?}"
    );
}

fn until_crash(flight: &mut Flight, frame: &Frame, floor: Floor<'_>, seconds: f32) -> usize {
    for step in 0..(seconds / frame.dt).round() as usize {
        flight.step(frame, &base(), floor, &PLANE);
        if matches!(flight.phase, Phase::Crashed { .. }) {
            return step;
        }
    }
    panic!("no crash within {seconds} s: {:?}", flight.position);
}

#[test]
fn flying_into_a_ridge_crashes_explodes_for_the_content_s_time_and_flies_again_above_it() {
    let cliff = |_: f32, z: f32| if z > 1004.0 { 25.5 } else { WATER_HEIGHT };
    let mut flight = level(14.0);
    let boost = Frame {
        modifiers: egui::Modifiers::CTRL,
        ..idle()
    };
    flight.speed = FASTEST * BOOST;
    until_crash(&mut flight, &boost, &cliff, 1.0);
    let Phase::Crashed { eye, .. } = flight.phase else {
        unreachable!()
    };
    let struck = flight.position;
    assert!(
        struck[2] > 1000.0 && struck[2] < 1006.0 + SIZE,
        "{struck:?}"
    );
    let held = flight.sight(base(), &cliff, &PLANE);
    flight.step(
        &keys(&[egui::Key::W, egui::Key::D]),
        &base(),
        &cliff,
        &PLANE,
    );
    assert_eq!(flight.position, struck);
    let after = flight.sight(base(), &cliff, &PLANE);
    assert_eq!(after.camera.eye(), held.camera.eye());
    assert_eq!(held.camera.eye(), eye);
    let exploding = held.aircraft.expect("exploding");
    assert_eq!(
        exploding.animation,
        viewer_models::entity_animation::named_request(EXPLODE)
    );
    assert_eq!(exploding.position, struck);
    fly(&mut flight, &idle(), &cliff, PLANE.ground.explosion + STEP);
    assert!(flight.sight(base(), &cliff, &PLANE).aircraft.is_none());
    let heading = flight.yaw;
    let mut waited = PLANE.ground.explosion + STEP;
    while matches!(flight.phase, Phase::Crashed { .. }) {
        flight.step(&idle(), &base(), &cliff, &PLANE);
        waited += STEP;
    }
    assert!((waited - RESPAWN_AFTER).abs() < 2.0 * STEP, "{waited}");
    assert_eq!(flight.crashes, 1);
    assert_eq!(flight.yaw, heading);
    assert_eq!((flight.pitch, flight.roll), (0.0, 0.0));
    assert_eq!(
        [flight.position[0], flight.position[2]],
        [struck[0], struck[2]]
    );
    assert!(flight.position[1] >= 25.5 + clearance(SIZE) * START_CLEARANCES);
    let alive = flight
        .sight(base(), &cliff, &PLANE)
        .aircraft
        .expect("drawn");
    assert_eq!(alive.animation, 0);
    assert_eq!(alive.animation_identity, 2);
    fly(&mut flight, &keys(&[egui::Key::W]), &cliff, 3.0);
    assert_eq!(flight.phase, Phase::Flying);
}

#[test]
fn diving_into_the_sea_crashes_on_the_water() {
    let mut flight = Flight::new([3000.0, 30.0, 1000.0], 0.0, -MAX_PITCH, 0.0);
    flight.speed = FASTEST * BOOST;
    let dive = Frame {
        modifiers: egui::Modifiers::CTRL,
        ..keys(&[egui::Key::ArrowDown])
    };
    until_crash(&mut flight, &dive, &flat, 1.0);
    let [_, _, forward] = flight.attitude().axes();
    let nose = flight.position[1] + forward[1] * SIZE;
    assert!(nose <= WATER_HEIGHT + 1e-3, "{nose}");
    assert!(nose > WATER_HEIGHT - FASTEST * BOOST * STEP, "{nose}");
}

#[test]
fn no_speed_carries_it_through_a_ridge_between_two_ground_tests() {
    let ridge = |_: f32, z: f32| {
        if (1100.0..1101.0).contains(&z) {
            25.5
        } else {
            WATER_HEIGHT
        }
    };
    let mut flight = Flight::new([3000.0, 18.0, 1090.0], 0.0, 0.0, 0.0);
    flight.speed = FASTEST * BOOST;
    let long = Frame {
        dt: crate::controls::LONGEST_STEP,
        modifiers: egui::Modifiers::CTRL,
        ..Frame::default()
    };
    until_crash(&mut flight, &long, &ridge, 2.0);
    assert!(flight.position[2] < 1101.0 + SIZE, "{:?}", flight.position);
}

#[test]
fn without_an_explode_state_or_content_contact_it_goes_at_once() {
    let mut flight = Flight::new([3000.0, 30.0, 1000.0], 0.0, -MAX_PITCH, 0.0);
    flight.speed = FASTEST * BOOST;
    let airframe = Airframe {
        explodes: false,
        ..PLANE
    };
    for _ in 0..60 {
        flight.step(&keys(&[egui::Key::ArrowDown]), &base(), &flat, &airframe);
    }
    assert!(matches!(flight.phase, Phase::Crashed { .. }));
    assert!(flight.sight(base(), &flat, &airframe).aircraft.is_none());
    assert_eq!(Airframe::NONE.ground.offset, 0.0);
    const { assert!(!Airframe::NONE.explodes) };
}

#[test]
fn the_content_s_contact_offset_raises_the_ground_it_meets() {
    let raised = Airframe {
        ground: GroundContact {
            offset: 3.0,
            explosion: 0.6,
        },
        ..PLANE
    };
    let mut flight = level(WATER_HEIGHT + 2.0);
    flight.step(&idle(), &base(), &flat, &PLANE);
    assert_eq!(flight.phase, Phase::Flying);
    flight.step(&idle(), &base(), &flat, &raised);
    assert!(matches!(flight.phase, Phase::Crashed { .. }));
}

fn drawn(width: u32, height: u32, heights: impl Fn(u32) -> f32) -> Vec<u8> {
    (0..height)
        .rev()
        .flat_map(|bottom| {
            let value = (heights(bottom) * 10.0).round() as u8;
            std::iter::repeat_n(value, width as usize)
        })
        .collect()
}

#[test]
fn a_crest_between_two_steps_is_still_met() {
    let values = drawn(16, 2048, |bottom| if bottom == 1100 { 25.5 } else { 9.5 });
    let field = viewer_gpu::province_map::HeightField {
        width: 16,
        height: 2048,
        values: &values,
    };
    let floor = |x: f32, z: f32| field.surface_height(x, z);
    let mut flight = Flight::new([3000.0, 25.0, 1090.05], 0.0, 0.0, 0.0);
    flight.throttle = 1.0;
    flight.speed = FASTEST * BOOST;
    let long = Frame {
        dt: crate::controls::LONGEST_STEP,
        modifiers: egui::Modifiers::CTRL,
        ..Frame::default()
    };
    flight.step(&long, &base(), &floor, &PLANE);
    assert!(
        matches!(flight.phase, Phase::Crashed { .. }),
        "{:?}",
        flight.position
    );
    assert!(flight.position[2] > 1098.0 && flight.position[2] < 1101.5);
    let clear = sweeps_into(
        &floor,
        5632.0,
        [3000.0, 25.6, 1099.8],
        [3000.0, 25.6, 1100.3],
        0.0,
    );
    let met = sweeps_into(
        &floor,
        5632.0,
        [3000.0, 25.4, 1099.8],
        [3000.0, 25.4, 1100.3],
        0.0,
    );
    assert!(!clear && met);
}

#[test]
fn a_respawn_clears_the_ground_under_its_wingtips_too() {
    let hillside = |x: f32, _: f32| if x > 3000.6 { 25.5 } else { WATER_HEIGHT };
    let airframe = Airframe { size: 1.0, ..PLANE };
    let mut flight = Flight::new([3000.0, 12.0, 1000.0], 0.0, 0.0, 0.0);
    flight.phase = Phase::Crashed {
        seconds: 0.0,
        eye: [3000.0, 20.0, 990.0],
        exploded: false,
    };
    while matches!(flight.phase, Phase::Crashed { .. }) {
        flight.step(&idle(), &base(), &hillside, &airframe);
    }
    assert_eq!(flight.phase, Phase::Flying);
    assert!(flight.position[1] >= 25.5 + clearance(1.0) * START_CLEARANCES);
    assert!(!flight.strikes(&hillside, 5632.0, &airframe));
    fly_with(&mut flight, &idle(), &hillside, &airframe, 2.0);
    assert_eq!(flight.phase, Phase::Flying);
}

fn low_ceiling() -> Camera {
    let mut map = base();
    map.map = MapSize {
        width: 256,
        height: 1200,
    };
    map.center = [128.0, 600.0];
    map.defines.heights[1] = 25.0;
    map
}

fn respawn_on(map: &Camera, floor: Floor<'_>, at: [f32; 3]) -> Flight {
    let mut flight = Flight::new(at, 0.0, 0.0, 0.0);
    flight.phase = Phase::Crashed {
        seconds: RESPAWN_AFTER,
        eye: [at[0], at[1] + 1.0, at[2] - 10.0],
        exploded: false,
    };
    flight.step(&idle(), map, floor, &PLANE);
    flight
}

fn margin() -> f32 {
    Footprints::radius(SIZE) as f32
}

#[test]
fn a_respawn_finds_room_under_a_low_ceiling_and_never_lies_in_the_ground() {
    let low = low_ceiling();
    let hills = |_: f32, z: f32| if z > 900.0 { 25.5 } else { WATER_HEIGHT };
    let flight = respawn_on(&low, &hills, [128.0, 24.0, 1000.0]);
    assert_eq!(flight.phase, Phase::Flying);
    assert_eq!(
        flight.position,
        [
            128.0,
            WATER_HEIGHT + clearance(SIZE) * START_CLEARANCES,
            900.0 - margin()
        ]
    );
    assert!(!flight.strikes(&hills, 256.0, &PLANE));
    let everywhere = |_: f32, _: f32| 25.5;
    let mut wreck = respawn_on(&low, &everywhere, [128.0, 24.0, 1000.0]);
    assert_eq!(
        wreck.phase,
        Phase::Wrecked {
            eye: [128.0, 25.0, 990.0]
        }
    );
    assert!(wreck.sight(low, &everywhere, &PLANE).aircraft.is_none());
    wreck.step(&keys(&[egui::Key::W]), &low, &everywhere, &PLANE);
    assert_eq!(wreck.position, [128.0, 24.0, 1000.0]);
}

#[test]
fn a_respawn_searches_the_whole_map_not_only_the_heading_line() {
    let low = low_ceiling();
    let channel = |x: f32, _: f32| {
        if (180.0..=200.0).contains(&x) {
            WATER_HEIGHT
        } else {
            25.5
        }
    };
    let flight = respawn_on(&low, &channel, [100.0, 24.0, 600.0]);
    assert_eq!(flight.phase, Phase::Flying);
    assert_eq!(
        flight.position[0],
        180.0 + margin(),
        "{:?}",
        flight.position
    );
    assert_eq!(flight.position[2], 600.0);
    assert!(!flight.strikes(&channel, 256.0, &PLANE));
}

#[test]
fn a_respawn_finds_room_across_the_east_west_seam() {
    let low = low_ceiling();
    let seam = |x: f32, _: f32| if x < 40.0 { WATER_HEIGHT } else { 25.5 };
    let flight = respawn_on(&low, &seam, [253.0, 24.0, 600.0]);
    assert_eq!(flight.phase, Phase::Flying);
    assert_eq!(flight.position[0], margin());
    assert_eq!(flight.position[2], 600.0);
}

#[test]
fn a_channel_just_wide_enough_for_the_aircraft_s_reach_has_room() {
    let low = low_ceiling();
    assert_eq!(Footprints::radius(SIZE), 3);
    let channel = |x: f32, _: f32| {
        if (100.0..=106.0).contains(&x) {
            WATER_HEIGHT
        } else {
            25.5
        }
    };
    let flight = respawn_on(&low, &channel, [60.0, 24.0, 600.0]);
    assert_eq!(flight.phase, Phase::Flying);
    assert_eq!([flight.position[0], flight.position[2]], [103.0, 600.0]);
    assert!(!flight.strikes(&channel, 256.0, &PLANE));
}

#[test]
fn the_nearest_room_is_measured_from_the_wreck_itself_not_its_vertex() {
    let low = low_ceiling();
    let pits = |x: f32, z: f32| {
        let north = (145.0..=151.0).contains(&x) && (607.0..=613.0).contains(&z);
        let south = (149.0..=155.0).contains(&x) && (597.0..=603.0).contains(&z);
        if north || south { WATER_HEIGHT } else { 25.5 }
    };
    let flight = respawn_on(&low, &pits, [150.3, 24.0, 605.0]);
    assert_eq!(flight.phase, Phase::Flying);
    assert_eq!([flight.position[0], flight.position[2]], [152.0, 600.0]);
    let mut footprints = Footprints::default();
    footprints.ensure(&pits, &low, SIZE);
    assert_eq!(
        footprints.nearest([150.0, 605.0], [0.0, 1.0], 20.0),
        Some([148.0, WATER_HEIGHT, 610.0])
    );
}

#[test]
fn the_footprints_are_built_once() {
    let map = Camera {
        defines: MapDefines {
            heights: [50.0, 25.0],
            ..MapDefines::test()
        },
        ..base()
    };
    let high = |_: f32, _: f32| 25.5;
    let mut footprints = Footprints::default();
    footprints.ensure(&high, &map, SIZE);
    assert_eq!(footprints.nearest([3000.0, 1000.0], [0.0, 1.0], 20.0), None);
    footprints.ensure(
        &|_: f32, _: f32| -> f32 { unreachable!("rebuilt") },
        &map,
        SIZE,
    );
}

fn explosion(
    airframe: &Airframe,
    played: impl Fn(f32) -> Option<Option<(f32, f32)>>,
) -> (Option<f32>, Option<f32>) {
    let mut flight = level(200.0);
    flight.phase = Phase::Crashed {
        seconds: 0.0,
        eye: [3000.0, 205.0, 995.0],
        exploded: !airframe.explodes,
    };
    let mut footprints = Footprints::default();
    let shown = |flight: &Flight| flight.sight(base(), &flat, airframe).aircraft.is_some();
    let mut gone = (!shown(&flight)).then_some(0.0);
    let mut elapsed = 0.0;
    while elapsed < 20.0 {
        let Phase::Crashed { seconds, .. } = flight.phase else {
            unreachable!("{:?}", flight.phase)
        };
        flight.update(
            &idle(),
            &base(),
            &flat,
            airframe,
            played(seconds),
            &mut footprints,
        );
        elapsed += STEP;
        if flight.phase == Phase::Flying {
            return (gone.or(Some(elapsed)), Some(elapsed));
        }
        if gone.is_none() && !shown(&flight) {
            gone = Some(elapsed);
        }
    }
    (gone, None)
}

fn near(seconds: Option<f32>, expected: f32) -> bool {
    seconds.is_some_and(|seconds| (seconds - expected).abs() < 3.0 * STEP)
}

#[test]
fn the_explosion_goes_when_its_entity_has_played_the_content_s_time_not_when_that_time_has_passed()
{
    let long = Airframe {
        ground: GroundContact {
            offset: 0.0,
            explosion: 4.0,
        },
        ..PLANE
    };
    let (gone, flies) = explosion(&long, |seconds| Some(Some((seconds * 0.5, 10.0))));
    assert!(near(gone, 8.0) && near(flies, 8.0), "{gone:?} {flies:?}");
    let (gone, flies) = explosion(&long, |seconds| Some(Some((3.5 + seconds, 10.0))));
    assert!(
        near(gone, 0.5) && near(flies, RESPAWN_AFTER),
        "{gone:?} {flies:?}"
    );
    assert_eq!(explosion(&long, |_| None), (None, None));
    assert_eq!(explosion(&long, |_| Some(None)), (None, None));
    let at_once = Airframe {
        ground: GroundContact {
            explosion: 0.0,
            ..long.ground
        },
        ..long
    };
    let (gone, flies) = explosion(&at_once, |_| Some(None));
    assert!(
        near(gone, 0.0) && near(flies, RESPAWN_AFTER),
        "{gone:?} {flies:?}"
    );
    let (gone, flies) = explosion(
        &Airframe {
            explodes: false,
            ..long
        },
        |_| None,
    );
    assert_eq!(gone, Some(0.0));
    assert!(near(flies, RESPAWN_AFTER), "{flies:?}");
}

#[test]
fn an_explosion_whose_aircraft_goes_before_it_has_played_is_over_and_the_flight_goes_on() {
    let long = Airframe {
        ground: GroundContact {
            offset: 0.0,
            explosion: 4.0,
        },
        ..PLANE
    };
    let mut flight = level(200.0);
    flight.phase = Phase::Crashed {
        seconds: 0.0,
        eye: [3000.0, 205.0, 995.0],
        exploded: false,
    };
    let mut footprints = Footprints::default();
    for _ in 0..30 {
        let Phase::Crashed { seconds, .. } = flight.phase else {
            unreachable!("{:?}", flight.phase)
        };
        let played = Some(Some((seconds, 10.0)));
        flight.update(&idle(), &base(), &flat, &long, played, &mut footprints);
    }
    assert!(flight.sight(base(), &flat, &long).aircraft.is_some());
    let none = Airframe {
        ground: long.ground,
        ..Airframe::NONE
    };
    let mut elapsed = 0.5;
    while matches!(flight.phase, Phase::Crashed { .. }) {
        flight.update(&idle(), &base(), &flat, &none, None, &mut footprints);
        elapsed += STEP;
        if matches!(flight.phase, Phase::Crashed { .. }) {
            assert!(flight.sight(base(), &flat, &none).aircraft.is_none());
        }
        assert!(elapsed < 20.0, "stranded: {:?}", flight.phase);
    }
    assert_eq!(flight.phase, Phase::Flying);
    assert!(near(Some(elapsed), RESPAWN_AFTER), "{elapsed}");
    assert_eq!(flight.crashes, 1);
}

fn fly_with(
    flight: &mut Flight,
    frame: &Frame,
    floor: Floor<'_>,
    airframe: &Airframe,
    seconds: f32,
) {
    for _ in 0..(seconds / frame.dt).round() as usize {
        flight.step(frame, &base(), floor, airframe);
    }
}
