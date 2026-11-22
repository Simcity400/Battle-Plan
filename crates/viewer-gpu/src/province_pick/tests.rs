use super::*;
use crate::camera::{MapDefines, MapSize, Pose};

fn fixture() -> MapData {
    MapData {
        width: 128,
        height: 64,
        province_ids: (0..64)
            .flat_map(|row| std::iter::repeat_n(if row < 34 { 1 } else { 2 }, 128))
            .collect(),
        definitions: Default::default(),
        country_colors: Default::default(),
    }
}

#[test]
fn elevated_hover_uses_height_cells_instead_of_the_water_plane() {
    let map = fixture();
    let camera = Camera {
        center: [64.25, 32.0],
        height: 100.0,
        map: MapSize::of(&map),
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let screen = [320.0, 240.0];
    let viewport = [640, 480];
    assert_eq!(raster(&map, [64.25, 9.5, 32.0]), Some(1));
    assert_eq!(
        pick(&camera, screen, viewport, &map, &vec![255; 128 * 64]).unwrap(),
        Some(2)
    );
}

#[test]
fn raster_is_bottom_up_and_truncates_before_bounds_checks() {
    let mut map = fixture();
    map.province_ids[63 * 128] = 19;
    assert_eq!(raster(&map, [-0.25, 0.0, -0.25]), Some(19));
    assert_eq!(raster(&map, [-1.0, 0.0, 0.0]), None);
    assert_eq!(raster(&map, [128.0, 0.0, 0.0]), None);
    assert_eq!(raster(&map, [0.0, 0.0, 64.0]), None);
    map.province_ids[63 * 128] = 0;
    assert_eq!(raster(&map, [0.0; 3]), None);
}

#[test]
fn height_rounds_to_nearest_clamps_and_keeps_sea_floor() {
    let map = fixture();
    let mut texels = vec![0; 128 * 64];
    texels[63 * 128 + 1] = 200;
    assert_eq!(terrain_height(&map, &texels, [0.49, 0.0]), 9.5);
    assert_eq!(terrain_height(&map, &texels, [0.5, 0.0]), 20.0);
    assert_eq!(terrain_height(&map, &texels, [-10.0, 90.0]), 9.5);
}

#[test]
fn seam_hover_selects_the_same_province_after_one_width_shift() {
    let mut map = fixture();
    for row in map.province_ids.chunks_exact_mut(128) {
        row[..64].fill(7);
        row[64..].fill(8);
    }
    let camera = Camera {
        center: [-0.25, 32.0],
        height: 100.0,
        map: MapSize::of(&map),
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let mut east = camera;
    east.center[0] += map.width as f32;
    let texels = vec![0; 128 * 64];
    assert_eq!(
        pick(&camera, [320.0, 240.0], [640, 480], &map, &texels).unwrap(),
        Some(8)
    );
    assert_eq!(
        pick(&east, [320.0, 240.0], [640, 480], &map, &texels).unwrap(),
        Some(8)
    );
}

#[test]
fn mismatched_rasters_return_a_named_refusal() {
    let mut map = fixture();
    let camera = Camera {
        center: [64.25, 32.0],
        height: 100.0,
        map: MapSize::of(&map),
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let texels = vec![0; map.province_ids.len()];
    assert!(
        pick(&camera, [320.0, 240.0], [640, 480], &map, &texels[..1])
            .unwrap_err()
            .contains("province pick: raster size mismatch")
    );
    map.province_ids.pop();
    assert!(
        pick(&camera, [320.0, 240.0], [640, 480], &map, &texels)
            .unwrap_err()
            .contains("province pick: raster size mismatch")
    );
}

#[test]
fn the_motion_pick_keeps_the_eye_coordinates_across_both_seams() {
    let map = fixture();
    let flat = vec![0; 128 * 64];
    for (centre, canonical) in [(138.0, 10.0), (-10.0, 118.0)] {
        let camera = Camera {
            center: [centre, 32.0],
            height: 100.0,
            map: MapSize::of(&map),
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        let screen = [320.0, 240.0];
        let viewport = [640, 480];
        let own = motion_ground_point(&camera, screen, viewport, &map, &flat)
            .unwrap()
            .unwrap();
        assert!((own[0] - centre).abs() < 0.5, "{centre}: {own:?}");
        let seams = ground_point(&camera, screen, viewport, &map, &flat)
            .unwrap()
            .unwrap();
        assert!((seams[0] - canonical).abs() < 0.5, "{centre}: {seams:?}");
    }
}

fn free_camera(map: &MapData, eye: [f32; 3], yaw: f32, pitch: f32) -> Camera {
    use crate::camera::{Free, Lens};
    Camera {
        center: [eye[0], eye[2]],
        height: eye[1],
        map: MapSize::of(map),
        defines: MapDefines::test(),
        pose: Pose::Free(Free::new(
            yaw,
            pitch,
            Lens {
                near: 0.5,
                far: 2000.0,
                ..Lens::MAP
            },
        )),
    }
}

#[test]
fn a_free_eye_picks_the_first_cell_its_ray_meets_and_nothing_above_the_horizon() {
    let map = fixture();
    let mut texels = vec![0; 128 * 64];
    for z in [40, 41] {
        for x in 0..128 {
            texels[(63 - z) * 128 + x] = 200;
        }
    }
    let viewport = [640, 480];
    let camera = free_camera(&map, [64.5, 15.0, 20.5], 0.0, 0.0);
    let hit = free_ground(&camera, [320.0, 240.0], viewport, &map, &texels).unwrap();
    assert!((hit[2] - 40.0).abs() < 1e-3, "{hit:?}");
    assert_eq!(hit[1], 20.0);
    assert_eq!(
        pick(&camera, [320.0, 240.0], viewport, &map, &texels).unwrap(),
        Some(1)
    );
    assert_eq!(
        free_ground(&camera, [320.0, 10.0], viewport, &map, &texels),
        None
    );
    assert_eq!(
        pick(&camera, [320.0, 10.0], viewport, &map, &texels).unwrap(),
        None
    );
    let down = free_camera(&map, [10.5, 60.0, 10.5], 0.0, -1.569);
    let sea = free_ground(&down, [320.0, 240.0], viewport, &map, &texels).unwrap();
    assert_eq!(sea[1], 9.5);
    assert!(
        (sea[0] - 10.5).abs() < 1.0 && (sea[2] - 10.5).abs() < 1.0,
        "{sea:?}"
    );
}

#[test]
fn a_free_eye_picks_across_the_seam_and_stops_past_the_map_edge() {
    let map = fixture();
    let mut texels = vec![0; 128 * 64];
    texels[(63 - 30) * 128 + 2] = 250;
    let viewport = [640, 480];
    let camera = free_camera(&map, [126.5, 20.0, 30.5], std::f32::consts::FRAC_PI_2, 0.0);
    let hit = free_ground(&camera, [320.0, 240.0], viewport, &map, &texels).unwrap();
    assert!((hit[0] - 130.0).abs() < 1e-3, "{hit:?}");
    let screen = [320.0, 240.0];
    let seams = ground_point(&camera, screen, viewport, &map, &texels)
        .unwrap()
        .unwrap();
    assert!((seams[0] - 2.0).abs() < 1e-3, "{seams:?}");
    let own = motion_ground_point(&camera, screen, viewport, &map, &texels)
        .unwrap()
        .unwrap();
    assert!((own[0] - 130.0).abs() < 1e-3, "{own:?}");
    assert_eq!(
        pick(&camera, [320.0, 240.0], viewport, &map, &texels).unwrap(),
        Some(1)
    );
    let north = free_camera(&map, [64.5, 10.0, 60.5], 0.0, 0.0);
    assert_eq!(
        free_ground(&north, [320.0, 240.0], viewport, &map, &texels),
        None
    );
}

#[test]
fn a_free_eye_picks_visible_ground_out_to_the_far_planes_depth_at_the_edges() {
    use crate::camera::{Free, Lens, WATER_HEIGHT};
    let map = MapData {
        width: 256,
        height: 256,
        province_ids: vec![1; 256 * 256],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let sea = vec![0; 256 * 256];
    let viewport = [640, 480];
    let pitch = -(20.0f32 / 90.0).asin();
    let camera = Camera {
        center: [128.5, 10.0],
        height: WATER_HEIGHT + 20.0,
        map: MapSize::of(&map),
        defines: MapDefines::test(),
        pose: Pose::Free(Free::new(
            0.0,
            pitch,
            Lens {
                near: 0.5,
                far: 95.0,
                ..Lens::MAP
            },
        )),
    };
    let eye = camera.eye();
    let along = |point: [f32; 3]| {
        (0..3)
            .map(|axis| (point[axis] - eye[axis]).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    let centre = free_ground(&camera, [320.0, 240.0], viewport, &map, &sea).unwrap();
    assert!((along(centre) - 90.0).abs() < 2.0, "{centre:?}");
    let edge = free_ground(&camera, [0.0, 240.0], viewport, &map, &sea).unwrap();
    assert!(along(edge) > 95.0, "{edge:?}");
    assert_eq!(
        pick(&camera, [0.0, 240.0], viewport, &map, &sea).unwrap(),
        Some(1)
    );
}
