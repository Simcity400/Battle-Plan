use super::*;
use crate::controls::{Frame, Pad};
use viewer_gpu::camera::{MapDefines, MapSize};

const VIEWPORT: [u32; 2] = [1600, 900];

fn map() -> Camera {
    Camera {
        center: [3000.0, 1000.0],
        height: 300.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

fn frame(pad: Pad, dt: f32) -> Frame {
    Frame {
        dt,
        pad,
        ..Frame::default()
    }
}

fn hold_for(orbit: &mut Orbit, map: &mut Camera, pad: Pad, seconds: f32) {
    for _ in 0..(seconds * 60.0) as usize {
        orbit.update(&frame(pad, 1.0 / 60.0), map, [0.0; 2]);
    }
}

#[test]
fn without_a_heading_or_tilt_the_camera_is_the_games() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    assert_eq!(orbit.view(camera), camera);
    camera.center[0] = -20.0;
    assert!(!orbit.update(&frame(Pad::default(), 0.016), &mut camera, [0.0; 2]));
    assert_eq!(camera.center[0], -20.0);
}

#[test]
fn the_right_stick_turns_and_tilts_around_the_point_looked_at() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    let point = camera.look_at();
    let turn = Pad {
        right: [1.0, 0.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, turn, 0.5);
    assert!(orbit.yaw > 0.5, "{}", orbit.yaw);
    let view = orbit.view(camera);
    assert!(view.free().is_some());
    let centre = view
        .unproject_ground([800.0, 450.0], VIEWPORT)
        .expect("ground ahead");
    assert!((centre[0] - point[0]).abs() < 0.5 && (centre[1] - point[2]).abs() < 0.5);
    assert_eq!(view.eye()[1], camera.height);
    let up = Pad {
        right: [0.0, 1.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, up, 4.0);
    let view = orbit.view(camera);
    let pitch = view.look_direction()[1].asin();
    assert!((pitch + LOWEST_ELEVATION).abs() < 1e-3, "{pitch}");
    assert!(view.unproject_ground([800.0, 0.0], VIEWPORT).is_none());
}

#[test]
fn the_right_stick_press_eases_back_to_the_games_view_exactly() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    let both = Pad {
        right: [0.8, 0.6],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, both, 1.0);
    hold_for(&mut orbit, &mut camera, Pad::default(), 0.5);
    assert!(!orbit.is_native());
    let press = Pad {
        held: GamepadButton::RightStick.bit(),
        pressed: GamepadButton::RightStick.bit(),
        ..Pad::default()
    };
    orbit.update(&frame(press, 1.0 / 60.0), &mut camera, [0.0; 2]);
    hold_for(&mut orbit, &mut camera, Pad::default(), 2.0);
    assert!(orbit.is_native());
    assert!(!orbit.moving());
    assert_eq!(orbit.view(camera), camera);
}

#[test]
fn a_flick_then_the_right_stick_press_returns_for_good() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    let flick = Pad {
        right: [1.0, 1.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, flick, 0.05);
    let press = Pad {
        held: GamepadButton::RightStick.bit(),
        pressed: GamepadButton::RightStick.bit(),
        ..Pad::default()
    };
    orbit.update(&frame(press, 1.0 / 60.0), &mut camera, [0.0; 2]);
    for seconds in [0.5, 1.0, 3.0] {
        hold_for(&mut orbit, &mut camera, Pad::default(), seconds);
        assert!(orbit.is_native(), "{orbit:?}");
        assert_eq!(orbit.view(camera), camera);
    }
    assert!(!orbit.moving());
}

#[test]
fn the_left_stick_moves_relative_to_the_heading_at_a_pace_set_by_the_height() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    let ahead = Pad {
        left: [0.0, 1.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, ahead, 1.0);
    let moved = camera.center[1] - 1000.0;
    let rate = PAN_SPEED * (300.0 - WATER_HEIGHT);
    assert!(moved > 0.8 * rate && moved < rate, "{moved}");
    assert!((camera.center[0] - 3000.0).abs() < 1e-3);
    let mut turned = Orbit {
        yaw: std::f32::consts::FRAC_PI_2,
        ..Orbit::default()
    };
    let mut camera = map();
    hold_for(&mut turned, &mut camera, ahead, 1.0);
    assert!(camera.center[0] - 3000.0 > 0.8 * rate);
    assert!((camera.center[1] - 1000.0).abs() < 1.0);
}

#[test]
fn keys_move_like_the_left_stick_relative_to_the_heading_and_never_faster() {
    let rate = PAN_SPEED * (300.0 - WATER_HEIGHT);
    let mut turned = Orbit {
        yaw: std::f32::consts::FRAC_PI_2,
        ..Orbit::default()
    };
    let mut camera = map();
    for _ in 0..60 {
        turned.update(&frame(Pad::default(), 1.0 / 60.0), &mut camera, [0.0, 1.0]);
    }
    let east = camera.center[0] - 3000.0;
    assert!(east > 0.8 * rate && east < rate, "{east}");
    assert!((camera.center[1] - 1000.0).abs() < 1.0);
    let ahead = Pad {
        left: [0.0, 1.0],
        ..Pad::default()
    };
    let mut camera = map();
    let mut orbit = Orbit::default();
    for _ in 0..60 {
        orbit.update(&frame(ahead, 1.0 / 60.0), &mut camera, [0.0, 1.0]);
    }
    assert!(camera.center[1] - 1000.0 < rate);
    for _ in 0..120 {
        orbit.update(&frame(Pad::default(), 1.0 / 60.0), &mut camera, [0.0; 2]);
    }
    assert!(!orbit.moving());
}

#[test]
fn the_triggers_zoom_within_the_contents_heights() {
    let mut orbit = Orbit::default();
    let mut camera = map();
    let zoom_in = Pad {
        triggers: [0.0, 1.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, zoom_in, 0.5);
    assert!(camera.height < 300.0);
    hold_for(&mut orbit, &mut camera, zoom_in, 10.0);
    assert_eq!(camera.height, MapDefines::test().heights[0]);
    let zoom_out = Pad {
        triggers: [1.0, 0.0],
        ..Pad::default()
    };
    hold_for(&mut orbit, &mut camera, zoom_out, 20.0);
    assert_eq!(camera.height, MapDefines::test().heights[1]);
    assert!(orbit.is_native());
    assert_eq!(orbit.view(camera), camera);
}

#[test]
fn a_mouse_drag_and_wheel_keep_their_ground_point_while_orbiting() {
    let orbit = Orbit {
        yaw: 0.9,
        tilt: 0.3,
        ..Orbit::default()
    };
    let mut camera = map();
    let cursor = [1100.0, 600.0];
    let grabbed = orbit
        .view(camera)
        .unproject_ground(cursor, VIEWPORT)
        .unwrap();
    orbit.zoom(&mut camera, 1.5, cursor, VIEWPORT);
    assert!((camera.height - 200.0).abs() < 1e-3);
    let still = orbit
        .view(camera)
        .unproject_ground(cursor, VIEWPORT)
        .unwrap();
    assert!((still[0] - grabbed[0]).abs() < 0.05 && (still[1] - grabbed[1]).abs() < 0.05);
    let from = [900.0, 500.0];
    let to = [700.0, 650.0];
    let under = orbit.view(camera).unproject_ground(from, VIEWPORT).unwrap();
    orbit.pan(&mut camera, from, to, VIEWPORT);
    let moved = orbit.view(camera).unproject_ground(to, VIEWPORT).unwrap();
    assert!((moved[0] - under[0]).abs() < 0.05 && (moved[1] - under[1]).abs() < 0.05);
}
