use super::*;
use crate::map::camera::{MapDefines, MapSize};

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
