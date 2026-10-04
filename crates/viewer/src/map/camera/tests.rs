use super::*;

fn reference() -> Camera {
    Camera {
        center: [3010.0, 1568.0],
        height: 310.178,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    }
}
const VIEWPORT: [u32; 2] = [2048, 1152];

/// R1 section 1.5 lists measured icon centres in the 2560x1440 screenshot,
/// which is the 2048x1152 frame upscaled by 1.25 (R1 section 0). The world
/// anchors are `map/unitstacks.txt` type 38 positions.
const FITTED_CITIES: [([f32; 3], [f32; 2]); 6] = [
    ([2956.37, 10.35, 1569.00], [982.1, 711.9]),
    ([3068.02, 10.40, 1541.38], [1629.1, 871.9]),
    ([3129.02, 9.82, 1561.00], [1979.1, 757.9]),
    ([3000.72, 9.70, 1638.29], [1238.1, 324.9]),
    ([2884.07, 9.70, 1564.86], [564.1, 735.9]),
    ([3103.20, 9.70, 1464.00], [1847.1, 1335.9]),
];

/// R1 section 1.5 fixes the icon centre at `anchor + (9.5, -1.5)` GUI pixels.
const ICON_OFFSET: [f32; 2] = [9.5, -1.5];

#[test]
fn projection_reproduces_the_fitted_city_positions() {
    for (world, measured) in FITTED_CITIES {
        let screen = reference().project(world, VIEWPORT).unwrap();
        let icon = [screen[0] + ICON_OFFSET[0], screen[1] + ICON_OFFSET[1]];
        // The report measures in the 1.25x screenshot; compare there.
        let residual = [icon[0] * 1.25 - measured[0], icon[1] * 1.25 - measured[1]];
        assert!(
            residual[0].abs() < 2.0 && residual[1].abs() < 2.0,
            "{world:?} projected to {icon:?}, residual {residual:?}"
        );
    }
}

#[test]
fn the_reference_view_matrix_matches_the_report() {
    let view = reference().view();
    let expected = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.099282, -0.995059, 0.0],
        [0.0, 0.995059, 0.099282, 0.0],
        [-3010.0, -1561.1963, 155.95049, 1.0],
    ];
    for (row, values) in expected.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            assert!(
                (view[row][column] - value).abs() < 1e-3,
                "view[{row}][{column}] = {} not {value}",
                view[row][column]
            );
        }
    }
    let projection = projection(aspect(VIEWPORT));
    assert!((projection[0][0] - 1.35876).abs() < 1e-5);
    assert!((projection[1][1] - 2.415574).abs() < 1e-5);
    assert!((projection[2][2] - 1.001252).abs() < 1e-5);
    assert!((projection[3][2] - -5.006258).abs() < 1e-5);
}

#[test]
fn unprojecting_a_projected_point_returns_it() {
    for point in [[2900.0, 9.5, 1600.0], [3100.0, 9.5, 1500.0]] {
        let screen = reference().project(point, VIEWPORT).unwrap();
        let ground = reference().unproject_ground(screen, VIEWPORT).unwrap();
        assert!((ground[0] - point[0]).abs() < 0.01, "{ground:?}");
        assert!((ground[1] - point[2]).abs() < 0.01, "{ground:?}");
    }
}

#[test]
fn the_inverse_undoes_the_view_projection() {
    let m = reference().view_projection(VIEWPORT);
    let inverse = reference().inverse(VIEWPORT).unwrap();
    let identity = multiply(m, inverse);
    for row in 0..4 {
        for column in 0..4 {
            let expected = if row == column { 1.0 } else { 0.0 };
            // The round trip is checked in f32, where the translation row
            // carries about four thousand units of magnitude.
            assert!(
                (identity[row][column] - expected).abs() < 1e-2,
                "{identity:?}"
            );
        }
    }
}

#[test]
fn panning_keeps_the_dragged_ground_point_under_the_pointer() {
    let mut camera = reference();
    let from = [700.0, 400.0];
    let to = [900.0, 620.0];
    let grabbed = camera.unproject_ground(from, VIEWPORT).unwrap();
    camera.pan(from, to, VIEWPORT);
    let now = camera.unproject_ground(to, VIEWPORT).unwrap();
    assert!((now[0] - grabbed[0]).abs() < 0.01 && (now[1] - grabbed[1]).abs() < 0.01);
}

#[test]
fn zooming_keeps_the_ground_point_under_the_cursor_and_clamps_height() {
    let mut camera = reference();
    let cursor = [1500.0, 300.0];
    let grabbed = camera.unproject_ground(cursor, VIEWPORT).unwrap();
    camera.zoom(2.0, cursor, VIEWPORT);
    let now = camera.unproject_ground(cursor, VIEWPORT).unwrap();
    assert!((now[0] - grabbed[0]).abs() < 0.01 && (now[1] - grabbed[1]).abs() < 0.01);
    assert!((camera.height - 310.178 / 2.0).abs() < 1e-3);
    camera.zoom(0.0001, cursor, VIEWPORT);
    assert_eq!(camera.height, MapDefines::test().heights[1]);
    camera.zoom(1000.0, cursor, VIEWPORT);
    assert_eq!(camera.height, MapDefines::test().heights[0]);
}

#[test]
fn the_centre_is_clamped_to_the_map_plus_the_outside_distance() {
    // A camera looking past the top edge is pulled back until the visible
    // ground stops at the map height + MapDefines::test().outside[0].
    let mut camera = Camera {
        center: [3010.0, 2200.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    camera.clamp_center(VIEWPORT);
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(
        (rect[3] - (MapSize::TEST.y() + MapDefines::test().outside[0])).abs() < 1e-2,
        "{rect:?}"
    );
    // And past the bottom edge, back to -MapDefines::test().outside[0].
    let mut camera = Camera {
        center: [3010.0, -400.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    camera.clamp_center(VIEWPORT);
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(
        (rect[1] - -MapDefines::test().outside[0]).abs() < 1e-2,
        "{rect:?}"
    );
    // A view taller than the allowance is centred on it instead, which is
    // what `pairD_out40` fits: at CAMERA_MAX_HEIGHT the clamp lands within
    // 0.9 world units of that frame's fitted centre of 1017.96.
    let mut camera = Camera {
        center: [3011.2, 1600.0],
        height: MapDefines::test().heights[1],
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    camera.clamp_center(VIEWPORT);
    assert!((camera.center[1] - 1017.96).abs() < 1.5, "{camera:?}");
    // A camera already inside is not moved at all: `pairD_out20`'s fit.
    let mut camera = Camera {
        center: [3011.86, 1364.30],
        height: 2122.295,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    let before = camera.center;
    camera.clamp_center(VIEWPORT);
    assert_eq!(camera.center, before);
}

#[test]
fn fit_centres_the_map_and_saturates_at_the_engine_ceiling() {
    let camera = Camera::fit(MapSize::TEST, MapDefines::test(), VIEWPORT);
    assert_eq!(camera.center, [2816.0, 1024.0]);
    // 3000 is CAMERA_MAX_HEIGHT and shows about 4400 x 2475 units at 16:9.
    assert_eq!(camera.height, MapDefines::test().heights[1]);
    let camera = Camera::fit(
        MapSize {
            width: 400,
            height: 200,
        },
        MapDefines::test(),
        VIEWPORT,
    );
    assert!(
        camera.height > MapDefines::test().heights[0]
            && camera.height < MapDefines::test().heights[1]
    );
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(rect[0] <= 0.0 && rect[1] <= 0.0 && rect[2] >= 400.0 && rect[3] >= 200.0);
}

#[test]
fn an_anchor_across_the_seam_is_projected_through_the_wrap_copy() {
    // Just west of the seam, so the frame runs past the map width.
    let camera = Camera {
        center: [MapSize::TEST.x() - 32.0, 1000.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    // A province 40 units east of the seam: its own x is 40, but it is
    // seen at 5672, which only the plus-width candidate reaches.
    let anchor = [40.0, WATER_HEIGHT, 1000.0];
    let plain = camera.project(anchor, VIEWPORT).unwrap();
    let wrapped = camera.project_wrapped(anchor, VIEWPORT).unwrap();
    assert!(
        plain[0] < 0.0,
        "the unshifted anchor is off the frame: {plain:?}"
    );
    let expected = camera
        .project([40.0 + MapSize::TEST.x(), WATER_HEIGHT, 1000.0], VIEWPORT)
        .unwrap();
    assert_eq!(wrapped, expected);
    assert!(
        wrapped[0] > 0.0 && wrapped[0] < VIEWPORT[0] as f32,
        "{wrapped:?}"
    );
}

#[test]
fn an_anchor_across_the_left_seam_is_projected_through_the_wrap_copy() {
    let camera = Camera {
        center: [40.0, 1000.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    let anchor = [MapSize::TEST.x() - 40.0, WATER_HEIGHT, 1000.0];
    let wrapped = camera.project_wrapped(anchor, VIEWPORT).unwrap();
    let expected = camera
        .project([-40.0, WATER_HEIGHT, 1000.0], VIEWPORT)
        .unwrap();
    assert_eq!(wrapped, expected);
    assert!(
        wrapped[0] > 0.0 && wrapped[0] < VIEWPORT[0] as f32,
        "{wrapped:?}"
    );
}

#[test]
fn an_anchor_the_camera_already_sees_keeps_its_own_position() {
    for (world, _) in FITTED_CITIES {
        assert_eq!(
            reference().project_wrapped(world, VIEWPORT),
            reference().project(world, VIEWPORT)
        );
    }
}

#[test]
fn a_degenerate_camera_is_refused() {
    assert!(
        Camera {
            center: [f32::NAN, 0.0],
            height: 310.0,
            map: MapSize::TEST,
            defines: MapDefines::test(),
        }
        .validate()
        .is_err()
    );
    assert!(
        Camera {
            center: [0.0, 0.0],
            height: 9.5,
            map: MapSize::TEST,
            defines: MapDefines::test(),
        }
        .validate()
        .is_err()
    );
    assert!(reference().validate().is_ok());
}

/// Finding 5: a map icon's projection (`14117BC60` from `14162D5B0`) is never refused. A
/// point behind the camera, which [`Camera::project`] refuses, lands where the division by
/// its negative w puts it; a point in front lands where [`Camera::project`] puts it; and an
/// anchor a map width away is first moved to the copy within half a width of the camera
#[test]
fn a_map_icon_projection_divides_by_any_w() {
    let camera = reference();
    let behind = [camera.center[0] + 10.0, 0.0, camera.center[1] - 5000.0];
    assert!(camera.project(behind, VIEWPORT).is_none());
    let extent = VIEWPORT.map(|value| value as i32);
    let point = camera.project_map_icon(behind, VIEWPORT, extent).position;
    assert!(point.iter().all(|value| value.is_finite()), "{point:?}");
    let front = [camera.center[0], 0.0, camera.center[1]];
    assert_eq!(
        Some(camera.project_map_icon(front, VIEWPORT, extent).position),
        camera.project(front, VIEWPORT)
    );
    let width = camera.map.x();
    assert_eq!(
        camera.project_map_icon([front[0] + width, front[1], front[2]], VIEWPORT, extent),
        camera.project_map_icon(front, VIEWPORT, extent)
    );
    // Scaled by the integer GUI viewport, not the device frame over the UI scale: a 1366
    // pixel frame at scale 1.25 is 1092 GUI pixels wide, never 1092.8.
    let ndc = camera.project(front, VIEWPORT).unwrap()[0] / VIEWPORT[0] as f32;
    let gui = camera
        .project_map_icon(front, VIEWPORT, [1092, 614])
        .position;
    assert!((gui[0] - ndc * 1092.0).abs() < 1e-3, "{gui:?} {ndc}");
}
