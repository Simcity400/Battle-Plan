use super::*;
use crate::camera::{MapDefines, MapSize, Pose};

fn flat(_: f32, _: f32) -> f32 {
    9.5
}

fn camera(height: f32) -> Camera {
    Camera {
        center: [3000.0, 1500.0],
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

const VIEWPORT: [u32; 2] = [2048, 1152];

fn speeds(camera_speed: f32) -> Speeds {
    Speeds {
        zoom_speed: 50.0,
        camera_speed,
        scroll_speed: 22.0,
        edge_scrolling: true,
    }
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

fn step(law: &CameraLaw, height: f32, frame: f32) -> f32 {
    let t = (height - 50.0) / (3000.0 - 50.0);
    let base = frame * law.zoom_speed * law.speed;
    ((t * FRAC_PI_2 - FRAC_PI_2).sin() + 1.0) * law.distance_mult * base + base
}

#[test]
fn camera_speed_is_held_as_the_game_reads_it() {
    assert_eq!(
        CameraLaw::new([50.0, 0.02, 6.0, 0.19], speeds(0.0)).camera_speed,
        5.0
    );
    assert_eq!(
        CameraLaw::new([50.0, 0.02, 6.0, 0.19], speeds(100.0)).camera_speed,
        95.0
    );
    assert_eq!(
        CameraLaw::new([50.0, 0.02, 6.0, 0.19], speeds(42.0)).camera_speed,
        42.0
    );
}

#[test]
fn wheel_lines_turn_whole_notches_and_keep_the_rest() {
    let mut motion = Motion::default();
    assert_eq!(motion.notches(0.5), 0);
    assert_eq!(motion.notches(0.5), 1);
    assert_eq!(motion.notches(2.0), 2);
    assert_eq!(motion.notches(-0.25), 0);
    assert_eq!(motion.notches(-0.75), -1);
    assert_eq!(motion.notches(f32::NAN), 0);
}

#[test]
fn a_notch_moves_the_target_along_the_cursor_ray_by_the_height_law() {
    let law = CameraLaw::test();
    let shown = camera(1000.0);
    let eye = shown.eye();
    let ground = [eye[0], 9.5, eye[2] + 400.0];
    let frame = 1.0 / 60.0;
    let mut motion = Motion::default();
    motion.wheel(&shown, &law, frame, &[1], Some(ground), &flat);
    let target = motion.target().unwrap();
    let t: f32 = (1000.0 - 50.0) / 2950.0;
    let forward = 400.0 - 2048.0 * 0.015 * (1.0 - t);
    let down: f32 = 9.5 - 1000.0;
    let length = (forward * forward + down * down).sqrt();
    let s = step(&law, 1000.0, frame);
    assert!(
        close(target.height, 1000.0 + down / length * s, 1e-3),
        "{target:?}"
    );
    assert!(
        close(target.center[1], 1500.0 + forward / length * s, 1e-3),
        "{target:?}"
    );
    assert_eq!(target.center[0], 3000.0);
    let mut out = Motion::default();
    out.wheel(&shown, &law, frame, &[-1, -1], Some(ground), &flat);
    assert!(close(
        out.target().unwrap().height,
        1000.0 - 2.0 * down / length * s,
        1e-2
    ));
    assert_eq!(shown.height, 1000.0);
}

#[test]
fn the_step_grows_towards_the_top_and_reads_the_frame_time() {
    let law = CameraLaw::test();
    assert!(close(step(&law, 50.0, 0.5), 0.5 * 50.0 * 50.0, 1e-2));
    assert!(close(
        step(&law, 3000.0, 0.5),
        7.0 * 0.5 * 50.0 * 50.0,
        1e-1
    ));
    let shown = camera(1500.0);
    let ground = [shown.eye()[0], 9.5, shown.eye()[2] + 200.0];
    let moved = |frame: f32| {
        let mut motion = Motion::default();
        motion.wheel(&shown, &law, frame, &[1], Some(ground), &flat);
        1500.0 - motion.target().unwrap().height
    };
    assert!(close(moved(1.0 / 30.0), 2.0 * moved(1.0 / 60.0), 1e-3));
    assert_eq!(moved(5.0), moved(0.7));
}

#[test]
fn no_notch_zooms_out_at_the_top_or_in_at_the_floor() {
    let law = CameraLaw::test();
    for (height, notch) in [(3000.0, -1), (50.0, 1)] {
        let shown = camera(height);
        let ground = [shown.eye()[0], 9.5, shown.eye()[2] + 200.0];
        let mut motion = Motion::default();
        motion.wheel(&shown, &law, 0.1, &[notch], Some(ground), &flat);
        assert_eq!(motion.target(), None, "{height} {notch}");
    }
}

#[test]
fn a_frame_reaching_below_the_floor_is_held_there_and_damps_its_sideways_move() {
    let law = CameraLaw::test();
    let shown = camera(60.0);
    let eye = shown.eye();
    let ground = [eye[0] + 30.0, 9.5, eye[2] + 30.0];
    let mut motion = Motion::default();
    motion.wheel(&shown, &law, 0.7, &[1], Some(ground), &flat);
    let target = motion.target().unwrap();
    assert!(close(target.height, 50.0, 1e-3), "{target:?}");
    let t: f32 = 10.0 / 2950.0;
    let correction = 2048.0 * 0.015 * (1.0 - t);
    let toward = normalize([30.0, 9.5 - 60.0, 30.0 - correction]);
    let s = step(&law, 60.0, 0.7);
    assert!(
        close(target.center[0] - 3000.0, toward[0] * s * (t + 0.1), 1e-3),
        "{target:?}"
    );
}

#[test]
fn a_zoom_key_sets_the_target_height_from_the_shown_height() {
    let law = CameraLaw::test();
    let mut shown = camera(1000.0);
    let mut motion = Motion::default();
    motion.key(&mut shown, &law, false, &flat);
    assert_eq!(motion.target().unwrap().height, 1050.0);
    motion.key(&mut shown, &law, true, &flat);
    assert_eq!(motion.target().unwrap().height, 950.0);
    let mut top = camera(2999.5);
    let mut held = Motion::default();
    held.key(&mut top, &law, false, &flat);
    assert_eq!(held.target().unwrap().height, 2999.5);
}

#[test]
fn the_shown_camera_eases_towards_the_target_until_it_settles() {
    let law = CameraLaw::test();
    let mut shown = camera(1000.0);
    let mut motion = Motion::default();
    motion.key(&mut shown, &law, false, &flat);
    assert!(motion.ease(&mut shown, &law, 0.05, VIEWPORT, &flat));
    assert!(
        close(shown.height, 1000.0 + 50.0 * 0.475, 1e-3),
        "{shown:?}"
    );
    assert!(motion.ease(&mut shown, &law, 0.7, VIEWPORT, &flat));
    assert_eq!(shown.height, 1050.0);
    assert!(!motion.ease(&mut shown, &law, 0.05, VIEWPORT, &flat));
    assert_eq!(motion.target().unwrap().height, 1050.0);
}

#[test]
fn heights_stay_within_the_floor_and_the_truncated_highest_height() {
    let law = CameraLaw::test();
    let mut shown = camera(1000.0);
    shown.defines.heights = [50.9, 2000.9];
    let mut motion = Motion {
        target: Some(Placement {
            center: [3000.0, -40.0],
            height: 2500.0,
            pose: Pose::Map,
        }),
        lines: 0.0,
    };
    motion.ease(&mut shown, &law, 0.7, VIEWPORT, &flat);
    assert_eq!(shown.height, 2000.0);
    assert_eq!(motion.target().unwrap().height, 2000.0);
    assert!(shown.center[1] > 0.0, "{shown:?}");
    assert_eq!(motion.target().unwrap().center[1], shown.center[1]);
    let high = |_: f32, _: f32| 1500.0;
    let mut low = Motion {
        target: Some(Placement {
            center: [3000.0, 1500.0],
            height: 100.0,
            pose: Pose::Map,
        }),
        lines: 0.0,
    };
    let mut shown = camera(1000.0);
    low.ease(&mut shown, &law, 0.7, VIEWPORT, &high);
    assert_eq!(shown.height, 1500.0);
    assert_eq!(low.target().unwrap().height, 1500.0);
}

#[test]
fn a_pan_moves_the_target_with_the_camera() {
    let law = CameraLaw::test();
    let mut shown = camera(1000.0);
    let mut motion = Motion::default();
    motion.key(&mut shown, &law, true, &flat);
    motion.shift([10.0, -5.0]);
    let target = motion.target().unwrap();
    assert_eq!(target.center, [3010.0, 1495.0]);
    motion.reset();
    assert_eq!(motion.target(), None);
}

#[test]
fn a_target_beyond_the_boundary_is_reset_to_the_camera() {
    let law = CameraLaw::test();
    let mut shown = camera(1000.0);
    shown.center[1] = 300.0;
    let mut motion = Motion {
        target: Some(Placement {
            center: [3000.0, 0.0],
            height: 1000.0,
            pose: Pose::Map,
        }),
        lines: 0.0,
    };
    motion.ease(&mut shown, &law, 0.7, VIEWPORT, &flat);
    let held = shown.center[1];
    assert_eq!(motion.target().unwrap().center[1], held);
    assert!(!motion.ease(&mut shown, &law, 0.05, VIEWPORT, &flat));
    assert_eq!(shown.center[1], held);
}

#[test]
fn wheel_events_turn_into_notches_in_order() {
    let mut motion = Motion::default();
    assert_eq!(motion.wheel_events([1.0, -1.0]), [1, -1]);
    assert_eq!(motion.wheel_events([0.5, 0.5, 2.0]), [1, 1, 1]);
    assert_eq!(motion.wheel_events([-0.25, -0.75]), [-1]);
}

#[test]
fn a_blocked_notch_does_not_cancel_the_next_one() {
    let law = CameraLaw::test();
    let shown = camera(50.0);
    let ground = [shown.eye()[0], 9.5, shown.eye()[2] + 200.0];
    let mut motion = Motion::default();
    motion.wheel(&shown, &law, 0.1, &[1, -1], Some(ground), &flat);
    assert!(motion.target().unwrap().height > 50.0);
    let top = camera(3000.0);
    let ground = [top.eye()[0], 9.5, top.eye()[2] + 200.0];
    let mut motion = Motion::default();
    motion.wheel(&top, &law, 0.1, &[-1, 1], Some(ground), &flat);
    assert!(motion.target().unwrap().height < 3000.0);
}

fn held(keys: Pan) -> Input<'static> {
    Input {
        keys,
        ..Input::default()
    }
}

#[test]
fn an_arrow_key_pans_towards_its_edge_by_the_height_and_frame() {
    let law = CameraLaw::test();
    let shown = camera(1000.0);
    let mut motion = Motion::default();
    let right = Pan {
        right: true,
        ..Pan::default()
    };
    motion.step(&shown, &law, 0.1, VIEWPORT, &held(right), &flat);
    let target = motion.target().unwrap();
    let direction = shown.pan_direction([2048.0, 576.0], VIEWPORT);
    let pan = 0.1 * 22.0 * (0.1 * 1000.0);
    assert!(
        close(target.center[0], 3000.0 + direction[0] * pan, 1e-3),
        "{target:?}"
    );
    assert!(target.center[0] > 3000.0);
    assert_eq!(target.height, 1000.0);
    let mut high = Motion::default();
    high.step(&camera(2000.0), &law, 0.1, VIEWPORT, &held(right), &flat);
    let far = high.target().unwrap().center[0] - 3000.0;
    assert!(close(far, 2.0 * (target.center[0] - 3000.0), 1e-2), "{far}");
    let mut long = Motion::default();
    long.step(&shown, &law, 5.0, VIEWPORT, &held(right), &flat);
    let mut capped = Motion::default();
    capped.step(&shown, &law, 0.7, VIEWPORT, &held(right), &flat);
    assert_eq!(long.target(), capped.target());
}

#[test]
fn diagonal_keys_add_their_directions_and_opposites_cancel() {
    let law = CameraLaw::test();
    let shown = camera(1000.0);
    let moved = |keys: Pan| {
        let mut motion = Motion::default();
        motion.step(&shown, &law, 0.1, VIEWPORT, &held(keys), &flat);
        let target = motion.target().unwrap_or(shown.placement());
        [target.center[0] - 3000.0, target.center[1] - 1500.0]
    };
    let up = moved(Pan {
        up: true,
        ..Pan::default()
    });
    let left = moved(Pan {
        left: true,
        ..Pan::default()
    });
    let both = moved(Pan {
        up: true,
        left: true,
        ..Pan::default()
    });
    assert!(up[1] > 0.0 && left[0] < 0.0, "{up:?} {left:?}");
    assert!(close(both[0], up[0] + left[0], 1e-3) && close(both[1], up[1] + left[1], 1e-3));
    let still = moved(Pan {
        left: true,
        right: true,
        ..Pan::default()
    });
    assert!(
        close(still[0], 0.0, 1e-3) && close(still[1], 0.0, 1e-3),
        "{still:?}"
    );
}

#[test]
fn edge_scrolling_reads_the_two_pixel_band_and_the_setting() {
    let law = CameraLaw::test();
    let shown = camera(1000.0);
    let edge = |law: &CameraLaw, cursor: [f32; 2]| {
        let mut motion = Motion::default();
        let input = Input {
            cursor: Some(cursor),
            ..Input::default()
        };
        motion.step(&shown, law, 0.1, VIEWPORT, &input, &flat);
        motion.target()
    };
    let left = edge(&law, [1.9, 576.0]).unwrap();
    assert!(left.center[0] < 3000.0, "{left:?}");
    assert_eq!(edge(&law, [2.0, 576.0]), None);
    let bottom = edge(&law, [1024.0, 1150.0]).unwrap();
    assert!(bottom.center[1] < 1500.0, "{bottom:?}");
    assert_eq!(edge(&law, [1024.0, 1149.0]), None);
    let off = CameraLaw {
        edge_scrolling: false,
        ..law
    };
    assert_eq!(edge(&off, [1.0, 576.0]), None);
}

#[test]
fn a_frame_without_motion_leaves_the_camera_fitted() {
    let law = CameraLaw::test();
    let shown = camera(1000.0);
    let mut motion = Motion::default();
    let inside = Input {
        cursor: Some([1024.0, 576.0]),
        ..Input::default()
    };
    assert!(!motion.step(&shown, &law, 0.1, VIEWPORT, &inside, &flat));
    let off = CameraLaw {
        edge_scrolling: false,
        ..law
    };
    let edge = Input {
        cursor: Some([0.0, 576.0]),
        ..Input::default()
    };
    assert!(!motion.step(&shown, &off, 0.1, VIEWPORT, &edge, &flat));
    assert_eq!(motion.target(), None);
    assert!(motion.step(&shown, &law, 0.1, VIEWPORT, &edge, &flat));
}

#[test]
fn placing_keeps_the_height_and_a_zero_height_lands_on_the_floor() {
    let mut camera = camera(800.0);
    let mut motion = Motion::default();
    motion.place(&mut camera, [100.0, 200.0], None, &flat);
    let target = motion.target().unwrap();
    assert_eq!(target.center, [100.0, 200.0]);
    assert_eq!(target.height, 800.0);
    motion.place(&mut camera, [100.0, 200.0], Some(0.0), &flat);
    assert_eq!(motion.target().unwrap().height, 50.0);
    motion.place(&mut camera, [100.0, -40.0], None, &flat);
    assert_eq!(motion.target().unwrap().center[1], 0.0);
}

#[test]
fn a_minimap_zoom_step_halves_in_and_grows_by_three_quarters_out() {
    let mut camera = camera(800.0);
    let mut motion = Motion::default();
    motion.zoom_step(&mut camera, true, &flat);
    assert_eq!(motion.target().unwrap().height, 400.0);
    motion.zoom_step(&mut camera, false, &flat);
    assert_eq!(motion.target().unwrap().height, 700.0);
    let mut low = Motion::default();
    let mut near = self::camera(60.0);
    low.zoom_step(&mut near, true, &flat);
    assert_eq!(low.target().unwrap().height, 50.0);
    let mut floor = Motion::default();
    let mut grounded = self::camera(50.5);
    floor.zoom_step(&mut grounded, true, &flat);
    assert_eq!(floor.target().unwrap().height, 50.5);
}
