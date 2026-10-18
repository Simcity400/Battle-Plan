use super::*;
use crate::controls::Pad;
use eframe::egui;
use viewer_foundation::map::camera::{MapDefines, MapSize};
use viewer_foundation::map::province_map::HeightField;

/// The figure's height at scale 1 in these tests.
const FIGURE: f32 = 2.0;
const STEP: f32 = 1.0 / 60.0;
const MAP: MapSize = MapSize {
    width: 640,
    height: 320,
};

/// Uneven land from row `0` up: every cell's two triangles slope differently, so the drawn
/// surface differs from a bilinear one.
fn land() -> Vec<u8> {
    (0..MAP.width * MAP.height)
        .map(|index| 100 + ((index % MAP.width) * 7 + (index / MAP.width) * 13) as u8 % 40)
        .collect()
}

fn field(values: &[u8]) -> HeightField<'_> {
    HeightField {
        width: MAP.width,
        height: MAP.height,
        values,
    }
}

fn base() -> Camera {
    Camera {
        center: [320.0, 160.0],
        height: 300.0,
        map: MAP,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

fn world<'a>(ground: &'a dyn Fn(f32, f32) -> f32) -> World<'a> {
    World::of(&base(), ground)
}

fn frame(keys: &[egui::Key]) -> Frame {
    Frame {
        dt: STEP,
        held: keys.iter().copied().collect(),
        ..Frame::default()
    }
}

fn pad(pad: Pad) -> Frame {
    Frame {
        dt: STEP,
        pad,
        ..Frame::default()
    }
}

fn double_tap(keys: &[egui::Key]) -> Frame {
    Frame {
        double_tapped: vec![egui::Key::Space],
        ..frame(keys)
    }
}

fn run(walker: &mut Walker, frame: &Frame, seconds: f32, world: &World<'_>) {
    for _ in 0..(seconds / STEP).round() as usize {
        walker.update(frame, 0.0, FIGURE, world);
    }
}

/// The drawn direction of a placed figure: `world_transform` turns local -Z to
/// `(sin r, 0, -cos r)` and a unit mesh faces its own -Z.
fn drawn_facing(instance: ModelInstance) -> [f32; 2] {
    [instance.rotation.sin(), -instance.rotation.cos()]
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn a_walker_walks_where_the_view_faces_on_the_drawn_triangle_surface() {
    let values = land();
    let field = field(&values);
    let ground = |x: f32, z: f32| field.surface_height(x, z);
    let world = world(&ground);
    let east = std::f32::consts::FRAC_PI_2;
    let mut walker = Walker::standing([10.3, 10.7], east, 1.0, &world);
    assert_eq!(walker.feet[1], field.surface_height(10.3, 10.7));
    assert_eq!(walker.instance(FIGURE).animation, IDLE);
    let ahead = frame(&[egui::Key::W]);
    for _ in 0..120 {
        walker.update(&ahead, 0.0, FIGURE, &world);
        // Every step stands exactly on the surface the terrain pass draws.
        assert_eq!(
            walker.feet[1],
            field.surface_height(walker.feet[0], walker.feet[2])
        );
    }
    // Two seconds east at a figure height a second, less the ease in; north unchanged.
    let walked = walker.feet[0] - 10.3;
    assert!(
        walked > WALK_PACE * FIGURE * (2.0 - 2.0 * EASE) && walked < WALK_PACE * FIGURE * 2.0,
        "{walked}"
    );
    assert!(close(walker.feet[2], 10.7, 1e-4));
    // It walks, facing east, with its textures scrolling as a moving division's do.
    let instance = walker.instance(FIGURE);
    assert_eq!(instance.animation, MOVE);
    assert_eq!(instance.native_flags, 3);
    assert_eq!(instance.position, walker.feet);
    let [x, z] = drawn_facing(instance);
    assert!(close(x, 1.0, 1e-3) && close(z, 0.0, 1e-3), "{x} {z}");
    // Stopped, it stands idle where it is.
    run(&mut walker, &frame(&[]), 1.0, &world);
    let instance = walker.instance(FIGURE);
    assert_eq!((instance.animation, instance.native_flags), (IDLE, 1));
}

#[test]
fn the_figure_turns_toward_where_it_walks() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    // The view faces north; walking right turns the figure east, and walking back turns it
    // south, without turning the view.
    let mut walker = Walker::standing([10.0, 10.0], 0.0, 1.0, &world);
    run(&mut walker, &frame(&[egui::Key::D]), 1.0, &world);
    assert!(close(walker.facing, std::f32::consts::FRAC_PI_2, 1e-3));
    assert!(walker.feet[0] > 10.0 && close(walker.feet[2], 10.0, 1e-4));
    run(&mut walker, &frame(&[egui::Key::S]), 1.0, &world);
    assert!(close(walker.facing.abs(), std::f32::consts::PI, 1e-3));
    assert_eq!(walker.view_yaw, 0.0);
}

#[test]
fn the_walker_walks_on_seas_and_lakes() {
    let sea = vec![0u8; (MAP.width * MAP.height) as usize];
    let field = field(&sea);
    let ground = |x: f32, z: f32| field.surface_height(x, z);
    let world = world(&ground);
    let mut walker = Walker::standing([20.0, 20.0], 0.0, 1.0, &world);
    run(&mut walker, &frame(&[egui::Key::W]), 1.0, &world);
    assert_eq!(walker.feet[1], WATER_HEIGHT);
    assert!(walker.feet[2] > 21.0);
    assert_eq!(walker.gait, Gait::Ground);
    assert_eq!(walker.instance(FIGURE).animation, MOVE);
}

#[test]
fn a_double_tap_flies_space_and_shift_rise_and_descend_and_another_falls_back() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    let mut walker = Walker::standing([10.0, 10.0], 0.0, 1.0, &world);
    // Space alone neither jumps nor flies.
    run(&mut walker, &frame(&[egui::Key::Space]), 0.5, &world);
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Ground, 20.0));
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    assert_eq!(walker.gait, Gait::Flying);
    // Flying holds its height until Space rises, at the flying pace.
    run(&mut walker, &frame(&[]), 1.0, &world);
    assert_eq!(walker.feet[1], 20.0);
    run(&mut walker, &frame(&[egui::Key::Space]), 1.0, &world);
    let risen = walker.feet[1] - 20.0;
    assert!(
        risen > FLY_PACE * FIGURE * (1.0 - 2.0 * EASE) && risen < FLY_PACE * FIGURE,
        "{risen}"
    );
    // Let go, it eases to a stop in the air. W then moves level, faster than walking, and
    // plays idle rather than the walk.
    run(&mut walker, &frame(&[]), 1.0, &world);
    let (x, height) = (walker.feet[2], walker.feet[1]);
    run(&mut walker, &frame(&[egui::Key::W]), 1.0, &world);
    let moved = walker.feet[2] - x;
    assert!(moved > WALK_PACE * FIGURE * 1.5, "{moved}");
    assert!(close(walker.feet[1], height, 1e-2));
    assert_eq!(walker.instance(FIGURE).animation, IDLE);
    // Shift descends; reaching the ground lands.
    let shift = Frame {
        modifiers: egui::Modifiers::SHIFT,
        ..frame(&[])
    };
    run(&mut walker, &shift, 4.0, &world);
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Ground, 20.0));
    // Up again, then a double tap stops flying: the figure falls faster and faster to the
    // ground.
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    run(&mut walker, &frame(&[egui::Key::Space]), 3.0, &world);
    let top = walker.feet[1];
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    assert_eq!(walker.gait, Gait::Falling);
    run(&mut walker, &frame(&[]), 0.25, &world);
    let first = top - walker.feet[1];
    run(&mut walker, &frame(&[]), 0.25, &world);
    let second = top - walker.feet[1] - first;
    assert!(first > 0.0 && second > 2.0 * first, "{first} {second}");
    // A double tap in the fall flies again and holds there.
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    let held = walker.feet[1];
    run(&mut walker, &frame(&[]), 0.5, &world);
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Flying, held));
    // Stopping again falls all the way back to the ground.
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    run(&mut walker, &frame(&[]), 5.0, &world);
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Ground, 20.0));
}

#[test]
fn flying_rises_over_the_ground_and_stays_under_the_highest_camera() {
    let ridge = |x: f32, _: f32| if x > 12.0 { 40.0 } else { 20.0 };
    let world = world(&ridge);
    let east = std::f32::consts::FRAC_PI_2;
    let mut walker = Walker::standing([10.0, 10.0], east, 1.0, &world);
    walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
    // Flying level into higher ground carries the figure up onto it, still flying.
    run(&mut walker, &frame(&[egui::Key::W]), 2.0, &world);
    assert!(walker.feet[0] > 12.0);
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Flying, 40.0));
    // It never rises so high that it or the view behind it passes the content's highest
    // camera.
    let ceiling = MapDefines::test().heights[1];
    walker.scale = 20.0;
    let boost = Frame {
        modifiers: egui::Modifiers::CTRL,
        ..frame(&[egui::Key::Space])
    };
    run(&mut walker, &boost, 30.0, &world);
    let reach = 20.0 * FIGURE * (1.0 + walker.distance);
    assert!(
        close(walker.feet[1], ceiling - reach, 1e-2),
        "{}",
        walker.feet[1]
    );
    let camera = walker.camera(base(), FIGURE, &world);
    assert!(camera.height <= ceiling);
    camera.validate().unwrap();
}

#[test]
fn a_controller_walks_flies_rises_descends_grows_and_turns_the_view() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    let mut walker = Walker::standing([10.0, 10.0], 0.0, 1.0, &world);
    // The left stick walks.
    run(
        &mut walker,
        &pad(Pad {
            left: [0.0, 1.0],
            ..Pad::default()
        }),
        1.0,
        &world,
    );
    assert!(walker.feet[2] > 10.0 + WALK_PACE * FIGURE * 0.5);
    assert_eq!(walker.instance(FIGURE).animation, MOVE);
    // The south button twice flies; held, it rises; the east button descends and lands.
    let south = GamepadButton::South.bit();
    walker.update(
        &pad(Pad {
            double_tapped: south,
            ..Pad::default()
        }),
        0.0,
        FIGURE,
        &world,
    );
    assert_eq!(walker.gait, Gait::Flying);
    run(
        &mut walker,
        &pad(Pad {
            held: south,
            ..Pad::default()
        }),
        1.0,
        &world,
    );
    assert!(walker.feet[1] > 20.0 + FLY_PACE * FIGURE * 0.5);
    // The triggers rise and sink by how far they are pulled.
    let before = walker.feet[1];
    run(
        &mut walker,
        &pad(Pad {
            triggers: [0.0, 1.0],
            ..Pad::default()
        }),
        0.5,
        &world,
    );
    assert!(walker.feet[1] > before);
    run(
        &mut walker,
        &pad(Pad {
            held: GamepadButton::East.bit(),
            ..Pad::default()
        }),
        5.0,
        &world,
    );
    assert_eq!((walker.gait, walker.feet[1]), (Gait::Ground, 20.0));
    // The d-pad grows and shrinks the figure.
    run(
        &mut walker,
        &pad(Pad {
            held: GamepadButton::DpadUp.bit(),
            ..Pad::default()
        }),
        1.0,
        &world,
    );
    assert!(close(walker.scale, 2.0, 1e-3), "{}", walker.scale);
    run(
        &mut walker,
        &pad(Pad {
            held: GamepadButton::DpadDown.bit(),
            ..Pad::default()
        }),
        1.0,
        &world,
    );
    assert!(close(walker.scale, 1.0, 1e-3), "{}", walker.scale);
    // The right stick turns the view by its rate; the bumpers move it nearer and farther; the
    // right stick pressed puts it behind the figure again.
    run(
        &mut walker,
        &pad(Pad {
            right: [1.0, 0.0],
            ..Pad::default()
        }),
        1.0,
        &world,
    );
    assert!(walker.view_yaw > 0.8 * STICK_LOOK[0] && walker.view_yaw < STICK_LOOK[0]);
    let distance = walker.distance;
    run(
        &mut walker,
        &pad(Pad {
            held: GamepadButton::RightBumper.bit(),
            ..Pad::default()
        }),
        0.5,
        &world,
    );
    assert!(walker.distance > distance * 1.3);
    walker.update(
        &pad(Pad {
            pressed: GamepadButton::RightStick.bit(),
            ..Pad::default()
        }),
        0.0,
        FIGURE,
        &world,
    );
    assert_eq!(
        (walker.view_yaw, walker.view_pitch),
        (walker.facing, VIEW_PITCH_START)
    );
}

#[test]
fn the_scale_grows_and_shrinks_the_figure_its_pace_and_its_view() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    let mut walker = Walker::standing([10.0, 10.0], 0.0, 1.0, &world);
    // Plus held doubles the scale every second; minus halves it.
    run(&mut walker, &frame(&[egui::Key::Plus]), 1.0, &world);
    assert!(close(walker.scale, 2.0, 1e-3), "{}", walker.scale);
    run(&mut walker, &frame(&[egui::Key::Equals]), 1.0, &world);
    assert!(close(walker.scale, 4.0, 2e-3), "{}", walker.scale);
    run(&mut walker, &frame(&[egui::Key::Minus]), 2.0, &world);
    assert!(close(walker.scale, 1.0, 1e-3), "{}", walker.scale);
    // A figure four times the size walks four times as far, and its view stands four times
    // as far away with a near plane four times as deep.
    let walk = |scale: f32| {
        let mut walker = Walker::standing([100.0, 100.0], 0.0, scale, &world);
        run(&mut walker, &frame(&[egui::Key::W]), 1.0, &world);
        let camera = walker.camera(base(), FIGURE, &world);
        let eye = camera.eye();
        let target = [
            walker.feet[0],
            walker.feet[1] + LOOK_AT * FIGURE * scale,
            walker.feet[2],
        ];
        let reach = (0..3)
            .map(|axis| (eye[axis] - target[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        (walker.feet[2] - 100.0, reach, camera.lens().near)
    };
    let (small, large) = (walk(1.0), walk(4.0));
    assert!(close(large.0, 4.0 * small.0, 1e-3), "{small:?} {large:?}");
    assert!(close(large.1, 4.0 * small.1, 1e-2), "{small:?} {large:?}");
    assert!(close(large.2, 4.0 * small.2, 1e-3), "{small:?} {large:?}");
    // Flying keeps the same rule: four times the pace up.
    let rise = |scale: f32| {
        let mut walker = Walker::standing([10.0, 10.0], 0.0, scale, &world);
        walker.update(&double_tap(&[]), 0.0, FIGURE, &world);
        run(&mut walker, &frame(&[egui::Key::Space]), 1.0, &world);
        walker.feet[1] - 20.0
    };
    assert!(close(rise(4.0), 4.0 * rise(1.0), 1e-2));
    // The scale stops at one height-map step tall and where the figure and its farthest view
    // reach the content's highest camera.
    let [smallest, largest] = scale_limits(FIGURE, MapDefines::test().heights[1]);
    assert!(close(smallest * FIGURE, HEIGHT_SCALE, 1e-6));
    assert!(close(
        largest * FIGURE * (1.0 + VIEW_DISTANCE[1]),
        MapDefines::test().heights[1] - WATER_HEIGHT,
        1e-2
    ));
    run(&mut walker, &frame(&[egui::Key::Minus]), 10.0, &world);
    assert_eq!(walker.scale, smallest);
    run(&mut walker, &frame(&[egui::Key::Plus]), 20.0, &world);
    assert_eq!(walker.scale, largest);
    assert_eq!(walker.instance(FIGURE).scale, largest);
}

#[test]
fn the_view_orbits_the_figure_looking_at_it_above_the_ground() {
    let values = land();
    let field = field(&values);
    let ground = |x: f32, z: f32| field.surface_height(x, z);
    let world = world(&ground);
    let mut walker = Walker::standing([300.0, 160.0], 0.0, 1.0, &world);
    // The mouse turns the view by its motion, within its pitch limits.
    let look = |dx: f32, dy: f32| Frame {
        look: [dx, dy],
        captured: true,
        ..frame(&[])
    };
    walker.update(&look(100.0, -50.0), 0.0, FIGURE, &world);
    assert!(close(walker.view_yaw, 100.0 * MOUSE_LOOK, 1e-5));
    assert!(close(
        walker.view_pitch,
        VIEW_PITCH_START + 50.0 * MOUSE_LOOK,
        1e-5
    ));
    for (dy, limit) in [(1.0e6, VIEW_PITCH[0]), (-1.0e6, VIEW_PITCH[1])] {
        walker.update(&look(0.0, dy), 0.0, FIGURE, &world);
        assert_eq!(walker.view_pitch, limit);
        let camera = walker.camera(base(), FIGURE, &world);
        camera.validate().unwrap();
        // It looks at the figure's chest, from at least two near planes above the ground.
        let eye = camera.eye();
        let target = [
            walker.feet[0],
            walker.feet[1] + LOOK_AT * FIGURE,
            walker.feet[2],
        ];
        let toward: Vec<f32> = (0..3).map(|axis| target[axis] - eye[axis]).collect();
        let length = toward.iter().map(|v| v * v).sum::<f32>().sqrt();
        let looking = camera.look_direction();
        for axis in 0..3 {
            assert!(
                close(looking[axis], toward[axis] / length, 1e-3),
                "{looking:?}"
            );
        }
        let near = camera.lens().near;
        assert!(near > 0.0);
        assert!(eye[1] >= field.surface_height(eye[0], eye[2]) + near * CLEARANCE - 1e-3);
    }
    // The wheel brings the view nearer and sends it farther, within its limits.
    walker.update(&frame(&[]), 1.0e5, FIGURE, &world);
    assert_eq!(walker.distance, VIEW_DISTANCE[0]);
    walker.update(&frame(&[]), -1.0e5, FIGURE, &world);
    assert_eq!(walker.distance, VIEW_DISTANCE[1]);
}

#[test]
fn the_walker_wraps_east_and_west_and_stops_at_the_top_and_bottom() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    let east = std::f32::consts::FRAC_PI_2;
    let mut walker = Walker::standing([639.5, 319.0], east, 1.0, &world);
    run(&mut walker, &frame(&[egui::Key::W]), 1.0, &world);
    assert!(walker.feet[0] < 2.0, "{}", walker.feet[0]);
    // Left of east is north, where the map stops.
    run(&mut walker, &frame(&[egui::Key::A]), 2.0, &world);
    assert_eq!(walker.feet[2], MAP.y());
    // The view behind the figure stays on the map too.
    let camera = walker.camera(base(), FIGURE, &world);
    assert!(camera.center[0] >= 0.0 && camera.center[0] < MAP.x());
    assert!(camera.center[1] <= MAP.y());
    let mut south = Walker::standing([5.0, 1.0], east, 1.0, &world);
    run(&mut south, &frame(&[egui::Key::D]), 2.0, &world);
    assert_eq!(south.feet[2], 0.0);
}

/// The angle between `camera`'s look and the direction from its eye to `point`.
fn off_look(camera: &Camera, point: [f32; 3]) -> f32 {
    let eye = camera.eye();
    let toward: [f32; 3] = std::array::from_fn(|axis| point[axis] - eye[axis]);
    let length = toward.iter().map(|value| value * value).sum::<f32>().sqrt();
    let look = camera.look_direction();
    let cosine = (0..3)
        .map(|axis| look[axis] * toward[axis] / length)
        .sum::<f32>();
    cosine.clamp(-1.0, 1.0).acos()
}

#[test]
fn at_the_map_s_edge_the_view_rises_and_still_looks_at_the_figure() {
    let ground = |_: f32, _: f32| 20.0;
    let world = world(&ground);
    // Facing north on the map's bottom row, the view behind the figure would stand off the map.
    for (at, yaw) in [
        ([100.0, 0.0], 0.0),
        ([100.0, MAP.y()], std::f32::consts::PI),
    ] {
        let walker = Walker::standing(at, yaw, 1.0, &world);
        let camera = walker.camera(base(), FIGURE, &world);
        camera.validate().unwrap();
        let eye = camera.eye();
        assert!((0.0..=MAP.y()).contains(&eye[2]), "{eye:?}");
        let target = [at[0], 20.0 + LOOK_AT * FIGURE, at[1]];
        assert!(
            off_look(&camera, target) < 0.01,
            "{}",
            off_look(&camera, target)
        );
        // At the view's distance, from above.
        let reach = (0..3)
            .map(|axis| (eye[axis] - target[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(close(reach, VIEW_DISTANCE_START * FIGURE, 1e-3), "{reach}");
        assert!(eye[1] > target[1]);
    }
    // Away from the edges the view is the one the pitch asks for.
    let walker = Walker::standing([100.0, 100.0], 0.0, 1.0, &world);
    let camera = walker.camera(base(), FIGURE, &world);
    assert!(close(camera.free().unwrap().pitch, VIEW_PITCH_START, 1e-4));
}
