use super::*;
use viewer_gpu::camera::{FOV_Y, Free, Lens, MapDefines, MapSize, Pose};
use viewer_gpu::province_admission::views;

const VIEWPORT: [u32; 2] = [1600, 900];

fn eye(x: f32, height: f32, pitch: f32) -> Camera {
    Camera {
        center: [x, 1000.0],
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Free(Free::new(
            std::f32::consts::FRAC_PI_2,
            pitch,
            Lens {
                fov_y: FOV_Y,
                near: 1.0,
                far: 20_000.0,
            },
        )),
    }
}

#[test]
fn a_player_high_across_the_seam_takes_its_own_copy() {
    let width = MapSize::TEST.x();
    let camera = eye(width - 5.0, 200.0, 0.5);
    let plane = ([60.0, 260.0, 1000.0], 2.0);
    assert_eq!(views(camera, VIEWPORT).len(), 1);
    let mut offsets = Vec::new();
    copies(&camera, VIEWPORT, plane, &mut offsets);
    assert_eq!(offsets, [width]);
    copies(
        &eye(100.0, 200.0, 0.5),
        VIEWPORT,
        ([180.0, 260.0, 1000.0], 2.0),
        &mut offsets,
    );
    assert_eq!(offsets, [0.0]);
    copies(
        &eye(100.0, 200.0, 0.5),
        VIEWPORT,
        ([20.0, 260.0, 1000.0], 2.0),
        &mut offsets,
    );
    assert!(offsets.is_empty(), "{offsets:?}");
}

#[test]
fn an_oblique_view_finds_copies_farther_in_x_than_its_depth() {
    let map = MapSize {
        width: 1024,
        height: 4096,
    };
    let camera = Camera {
        center: [500.0, 3000.0],
        height: 100.0,
        map,
        defines: MapDefines::test(),
        pose: Pose::Free(Free::new(
            60f32.to_radians(),
            0.0,
            Lens {
                fov_y: FOV_Y,
                near: 1.0,
                far: 20_000.0,
            },
        )),
    };
    let [right, _, forward] = camera.axes();
    let tangent_x = (FOV_Y * 0.5).tan() * VIEWPORT[0] as f32 / VIEWPORT[1] as f32;
    let depth = 18_050.0;
    let seen: [f32; 3] = std::array::from_fn(|axis| {
        camera.eye()[axis] + forward[axis] * depth + right[axis] * 0.95 * tangent_x * depth
    });
    let copy = (seen[0] / 1024.0).floor();
    assert_eq!(copy, 21.0);
    let figure = ([seen[0] - copy * 1024.0, seen[1], seen[2]], 5.0);
    assert!(seen[0] - camera.eye()[0] > camera.lens().far + figure.1);
    let mut offsets = Vec::new();
    copies(&camera, VIEWPORT, figure, &mut offsets);
    assert!(offsets.contains(&(copy * 1024.0)), "{offsets:?}");
}
