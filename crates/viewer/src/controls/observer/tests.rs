use super::*;
use crate::controls::Pad;
use dashboard::protocol::GamepadButton;
use eframe::egui;
use viewer_foundation::map::camera::{MapDefines, MapSize};

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

fn run(observer: &mut Observer, frame: &Frame, seconds: f32) {
    for _ in 0..(seconds * 60.0) as usize {
        observer.update(frame, base(), &flat);
    }
}

fn step(frame: Frame) -> Frame {
    Frame {
        dt: 1.0 / 60.0,
        ..frame
    }
}

#[test]
fn a_flying_eye_starts_where_the_game_camera_looks_from() {
    let map = base();
    let observer = Observer::flying(&map);
    let camera = observer.camera(map, &flat);
    assert_eq!(camera.eye(), map.eye());
    let [a, b] = [camera.look_direction(), map.look_direction()];
    for axis in 0..3 {
        assert!((a[axis] - b[axis]).abs() < 1e-5);
    }
}

#[test]
fn keys_move_level_relative_to_the_heading_faster_when_higher() {
    let mut observer = Observer::flying(&base());
    observer.yaw = std::f32::consts::FRAC_PI_2;
    let mut held = std::collections::BTreeSet::new();
    held.insert(egui::Key::W);
    let forward = step(Frame {
        held,
        ..Frame::default()
    });
    let start = observer.eye;
    run(&mut observer, &forward, 1.0);
    // East, level, at about 1.2 times the 290.5 height above the water per second.
    assert!(observer.eye[0] - start[0] > 0.8 * 290.5 * SPEED_PER_HEIGHT);
    assert!((observer.eye[1] - start[1]).abs() < 1e-3);
    assert!((observer.eye[2] - start[2]).abs() < 1e-2);
    // Low down it slows to its floor speed.
    let mut low = Observer {
        eye: [3000.0, WATER_HEIGHT + 1.0, 1000.0],
        velocity: [0.0; 3],
        ..observer
    };
    run(&mut low, &forward, 1.0);
    assert!(low.eye[0] - 3000.0 < SLOWEST * 1.01);
}

#[test]
fn the_eye_never_sinks_into_the_ground_and_stays_on_the_map() {
    let mut observer = Observer::flying(&base());
    let sink = step(Frame {
        modifiers: egui::Modifiers::SHIFT,
        ..Frame::default()
    });
    let ridge = |_: f32, _: f32| 20.0;
    for _ in 0..600 {
        observer.update(&sink, base(), &ridge);
    }
    let camera = observer.camera(base(), &ridge);
    assert!(observer.eye[1] > 20.0);
    assert!(observer.eye[1] - 20.0 >= camera.lens().near * CLEARANCE - 1e-3);
    // The near plane shrinks near the ground.
    assert!(camera.lens().near < 0.5);
    let mut north = Observer {
        eye: [3000.0, 100.0, 2040.0],
        yaw: 0.0,
        velocity: [0.0; 3],
        ..observer
    };
    let mut held = std::collections::BTreeSet::new();
    held.insert(egui::Key::W);
    let ahead = step(Frame {
        held,
        ..Frame::default()
    });
    run(&mut north, &ahead, 2.0);
    assert!(north.eye[2] <= MapSize::TEST.y());
}

#[test]
fn the_mouse_looks_only_by_its_motion_and_the_stick_by_its_rate() {
    let mut observer = Observer::flying(&base());
    // Well short of the steepest pitch, so the look is not held there.
    observer.pitch = -0.3;
    let (yaw, pitch) = (observer.yaw, observer.pitch);
    let look = step(Frame {
        look: [100.0, 50.0],
        captured: true,
        ..Frame::default()
    });
    observer.update(&look, base(), &flat);
    assert!((observer.yaw - (yaw + 100.0 * MOUSE_LOOK)).abs() < 1e-5);
    assert!((observer.pitch - (pitch - 50.0 * MOUSE_LOOK)).abs() < 1e-5);
    // Pitch stops short of straight down.
    let down = step(Frame {
        look: [0.0, 1.0e6],
        ..Frame::default()
    });
    observer.update(&down, base(), &flat);
    assert_eq!(observer.pitch, -PITCH_LIMIT);
    let stick = step(Frame {
        pad: Pad {
            right: [1.0, 0.0],
            ..Pad::default()
        },
        ..Frame::default()
    });
    let yaw = observer.yaw;
    run(&mut observer, &stick, 1.0);
    let turned = (observer.yaw - yaw).rem_euclid(std::f32::consts::TAU);
    assert!(
        turned > 0.8 * STICK_LOOK[0] && turned < STICK_LOOK[0],
        "{turned}"
    );
}

fn held(keys: &[egui::Key]) -> Frame {
    step(Frame {
        held: keys.iter().copied().collect(),
        ..Frame::default()
    })
}

#[test]
fn a_walker_stands_on_the_ground_or_water_and_walks_at_its_pace() {
    // Land three high to the east of x = 3010, sea to the west.
    let coast = |x: f32, _: f32| if x > 3010.0 { 30.0 } else { WATER_HEIGHT - 2.0 };
    let mut walker = Observer::walking([3000.0, 1000.0], std::f32::consts::FRAC_PI_2, &coast);
    assert_eq!(walker.gait, Gait::Walk { flying: false });
    assert_eq!(walker.eye[1], WATER_HEIGHT + EYE_HEIGHT);
    assert_eq!(walker.pitch, 0.0);
    // Walking east, up onto the land: on the water surface first, then on the ground.
    let east = held(&[egui::Key::W]);
    for _ in 0..30 {
        walker.update(&east, base(), &coast);
    }
    assert!(walker.eye[0] > 3000.0 && walker.eye[0] < 3000.0 + WALK_SPEED * 0.5);
    for _ in 0..120 {
        walker.update(&east, base(), &coast);
    }
    assert!(walker.eye[0] > 3010.0);
    assert_eq!(walker.eye[1], 30.0 + EYE_HEIGHT);
    // Space alone neither jumps nor flies.
    let space = held(&[egui::Key::Space]);
    walker.update(&space, base(), &coast);
    assert_eq!(walker.eye[1], 30.0 + EYE_HEIGHT);
}

#[test]
fn a_double_tap_flies_from_a_walk_and_another_falls_back_to_the_ground() {
    let mut walker = Observer::walking([3000.0, 1000.0], 0.0, &flat);
    let tap = |key_tap: bool, pad_tap: bool, rise: bool| {
        step(Frame {
            held: if rise {
                [egui::Key::Space].into_iter().collect()
            } else {
                Default::default()
            },
            double_tapped: if key_tap { vec![egui::Key::Space] } else { Vec::new() },
            pad: Pad {
                double_tapped: if pad_tap {
                    GamepadButton::South.bit()
                } else {
                    0
                },
                ..Pad::default()
            },
            ..Frame::default()
        })
    };
    walker.update(&tap(true, false, true), base(), &flat);
    assert_eq!(walker.gait, Gait::Walk { flying: true });
    // Flying, Space rises at walking pace.
    for _ in 0..60 {
        walker.update(&tap(false, false, true), base(), &flat);
    }
    let high = walker.eye[1];
    assert!(high > WATER_HEIGHT + EYE_HEIGHT + 0.5 * WALK_SPEED, "{high}");
    assert!(high < WATER_HEIGHT + EYE_HEIGHT + WALK_SPEED);
    // The controller's double tap stops flying; the walker falls back to the ground.
    walker.update(&tap(false, true, false), base(), &flat);
    assert_eq!(walker.gait, Gait::Walk { flying: false });
    for _ in 0..60 {
        walker.update(&tap(false, false, false), base(), &flat);
    }
    assert_eq!(walker.eye[1], WATER_HEIGHT + EYE_HEIGHT);
    // Flying mode has no walk to return to.
    let mut flyer = Observer::flying(&base());
    flyer.update(&tap(true, true, false), base(), &flat);
    assert_eq!(flyer.gait, Gait::Fly);
}
