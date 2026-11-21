use super::*;

fn reference() -> Camera {
    Camera {
        center: [3010.0, 1568.0],
        height: 310.178,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}
const VIEWPORT: [u32; 2] = [2048, 1152];

const FITTED_CITIES: [([f32; 3], [f32; 2]); 6] = [
    ([2956.37, 10.35, 1569.00], [982.1, 711.9]),
    ([3068.02, 10.40, 1541.38], [1629.1, 871.9]),
    ([3129.02, 9.82, 1561.00], [1979.1, 757.9]),
    ([3000.72, 9.70, 1638.29], [1238.1, 324.9]),
    ([2884.07, 9.70, 1564.86], [564.1, 735.9]),
    ([3103.20, 9.70, 1464.00], [1847.1, 1335.9]),
];

const ICON_OFFSET: [f32; 2] = [9.5, -1.5];

#[test]
fn projection_reproduces_the_fitted_city_positions() {
    for (world, measured) in FITTED_CITIES {
        let screen = reference().project(world, VIEWPORT).unwrap();
        let icon = [screen[0] + ICON_OFFSET[0], screen[1] + ICON_OFFSET[1]];
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
    let projection = projection(Lens::MAP, aspect(VIEWPORT));
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
fn the_centre_is_clamped_to_the_map_plus_the_outside_distance() {
    let mut camera = Camera {
        center: [3010.0, 2200.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    camera.clamp_center(VIEWPORT);
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(
        (rect[3] - (MapSize::TEST.y() + MapDefines::test().outside[0])).abs() < 1e-2,
        "{rect:?}"
    );
    let mut camera = Camera {
        center: [3010.0, -400.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    camera.clamp_center(VIEWPORT);
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(
        (rect[1] - -MapDefines::test().outside[0]).abs() < 1e-2,
        "{rect:?}"
    );
    let mut camera = Camera {
        center: [3011.2, 1600.0],
        height: MapDefines::test().heights[1],
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    camera.clamp_center(VIEWPORT);
    assert!((camera.center[1] - 1017.96).abs() < 1.5, "{camera:?}");
    let mut camera = Camera {
        center: [3011.86, 1364.30],
        height: 2122.295,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let before = camera.center;
    camera.clamp_center(VIEWPORT);
    assert_eq!(camera.center, before);
}

#[test]
fn fit_centres_the_map_and_saturates_at_the_engine_ceiling() {
    let camera = Camera::fit(MapSize::TEST, MapDefines::test(), VIEWPORT);
    assert_eq!(camera.center, [2816.0, 1024.0]);
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
    let camera = Camera {
        center: [MapSize::TEST.x() - 32.0, 1000.0],
        height: 600.0,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
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
        pose: Pose::Map,
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
            pose: Pose::Map,
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
            pose: Pose::Map,
        }
        .validate()
        .is_err()
    );
    assert!(reference().validate().is_ok());
}

#[test]
fn a_map_icon_projection_divides_by_any_w() {
    let camera = reference();
    let behind = [camera.center[0] + 10.0, 0.0, camera.center[1] - 5000.0];
    assert!(camera.project(behind, VIEWPORT).is_none());
    let extent = VIEWPORT.map(|value| value as i32);
    let point = camera
        .project_map_icon(behind, VIEWPORT, extent)
        .unwrap()
        .position;
    assert!(point.iter().all(|value| value.is_finite()), "{point:?}");
    let front = [camera.center[0], 0.0, camera.center[1]];
    assert_eq!(
        camera
            .project_map_icon(front, VIEWPORT, extent)
            .map(|icon| icon.position),
        camera.project(front, VIEWPORT)
    );
    let width = camera.map.x();
    assert_eq!(
        camera.project_map_icon([front[0] + width, front[1], front[2]], VIEWPORT, extent),
        camera.project_map_icon(front, VIEWPORT, extent)
    );
    let ndc = camera.project(front, VIEWPORT).unwrap()[0] / VIEWPORT[0] as f32;
    let gui = camera
        .project_map_icon(front, VIEWPORT, [1092, 614])
        .unwrap()
        .position;
    assert!((gui[0] - ndc * 1092.0).abs() < 1e-3, "{gui:?} {ndc}");
}

#[test]
fn the_stored_tilt_is_held_within_five_and_one_hundred() {
    assert_eq!(stored_tilt(0.0), 5.0);
    assert_eq!(stored_tilt(150.0), 100.0);
    assert_eq!(stored_tilt(70.0), 70.0);
}

#[test]
fn the_pan_direction_is_the_cursor_ray_across_the_view_without_height() {
    let camera = reference();
    let centre = camera.pan_direction([1024.0, 576.0], VIEWPORT);
    assert!(centre.iter().all(|value| value.abs() < 1e-3), "{centre:?}");
    let right = camera.pan_direction([2048.0, 576.0], VIEWPORT);
    assert!(
        right[0] > 0.0 && right[1] == 0.0 && right[2].abs() < 1e-3,
        "{right:?}"
    );
    let top = camera.pan_direction([1024.0, 0.0], VIEWPORT);
    assert!(
        top[2] > 0.0 && top[1] == 0.0 && top[0].abs() < 1e-3,
        "{top:?}"
    );
    assert!(right[0] > top[2]);
}

fn free(yaw: f32, pitch: f32, height: f32, lens: Lens) -> Camera {
    let eye = reference().eye();
    Camera {
        center: [eye[0], eye[2]],
        height,
        pose: Pose::Free(Free::new(yaw, pitch, lens)),
        ..reference()
    }
}

#[test]
fn a_free_pose_built_from_the_game_camera_sees_what_it_sees() {
    let map = reference();
    let pose = Free::of_map(0.0, map.height, map.defines.tilt_lead);
    let eye = map.eye();
    let camera = Camera {
        center: [eye[0], eye[2]],
        pose: Pose::Free(pose),
        ..map
    };
    assert_eq!(camera.eye(), eye);
    for point in [
        [2900.0, 9.5, 1600.0],
        [3100.0, 20.0, 1500.0],
        [3010.0, 9.5, 1568.0],
    ] {
        let [a, b] = [map, camera].map(|camera| camera.project(point, VIEWPORT).unwrap());
        assert!(
            (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05,
            "{a:?} {b:?}"
        );
    }
    let [forward, free_forward] = [map.look_direction(), camera.look_direction()];
    for axis in 0..3 {
        assert!((forward[axis] - free_forward[axis]).abs() < 1e-5);
    }
}

#[test]
fn free_axes_are_orthonormal_and_a_positive_roll_lowers_the_right_side() {
    let level = Free::new(0.7, -0.3, Lens::MAP);
    let rolled = Free { roll: 0.5, ..level };
    for pose in [level, rolled] {
        let [right, up, forward] = pose.axes();
        for (a, b) in [(right, up), (up, forward), (forward, right)] {
            assert!(dot(a, b).abs() < 1e-5);
        }
        for axis in [right, up, forward] {
            assert!((dot(axis, axis) - 1.0).abs() < 1e-5);
        }
    }
    assert!(rolled.axes()[0][1] < level.axes()[0][1]);
    let east = Free::new(std::f32::consts::FRAC_PI_2, 0.0, Lens::MAP).forward();
    assert!((east[0] - 1.0).abs() < 1e-6 && east[2].abs() < 1e-6);
    let looking = Free::looking_at([0.0, 50.0, 0.0], [100.0, 0.0, 100.0], Lens::MAP).forward();
    let expected = normalize([100.0, -50.0, 100.0]);
    for axis in 0..3 {
        assert!((looking[axis] - expected[axis]).abs() < 1e-5);
    }
}

#[test]
fn a_level_free_eye_reaches_its_object_range_and_sees_sky_above_the_horizon() {
    let lens = Lens {
        fov_y: FOV_Y,
        near: 1.0,
        far: 20_000.0,
    };
    let camera = free(0.0, 0.0, 30.0, lens);
    let eye = camera.eye();
    let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
    assert!(rect.iter().all(|value| value.is_finite()), "{rect:?}");
    let range = MapDefines::test().map_objects_cutoff;
    assert_eq!(camera.object_range(), range);
    assert!((rect[3] - (eye[2] + range)).abs() < 1.0, "{rect:?}");
    assert!(rect[1] >= eye[2], "nothing behind the eye: {rect:?}");
    assert!(camera.unproject_ground([1024.0, 0.0], VIEWPORT).is_none());
    assert!(
        camera
            .unproject_ground([1024.0, 1151.0], VIEWPORT)
            .is_some()
    );
    let terrain = camera
        .reach(VIEWPORT, lens.far, [0.0, frustum::TERRAIN_TOP])
        .unwrap();
    assert!(
        (terrain[3] - (eye[2] + lens.far)).abs() < 1.0,
        "{terrain:?}"
    );
}

#[test]
fn a_free_eye_looking_at_the_sky_reaches_no_ground() {
    let camera = free(0.0, 1.2, 100.0, Lens::MAP);
    assert!(camera.reach(VIEWPORT, 4000.0, [0.0, 70.0]).is_none());
    let eye = camera.eye();
    let rect = camera.visible_ground(VIEWPORT, 16.0).unwrap();
    assert_eq!(
        rect,
        [eye[0] - 16.0, eye[2] - 16.0, eye[0] + 16.0, eye[2] + 16.0]
    );
}

#[test]
fn map_icons_behind_a_free_eye_are_refused() {
    let camera = free(0.0, -0.2, 40.0, Lens::MAP);
    let eye = camera.eye();
    let extent = VIEWPORT.map(|value| value as i32);
    assert!(
        camera
            .project_map_icon([eye[0], 9.5, eye[2] - 100.0], VIEWPORT, extent)
            .is_none()
    );
    let front = [eye[0] + 5.0, 9.5, eye[2] + 100.0];
    assert_eq!(
        camera
            .project_map_icon(front, VIEWPORT, extent)
            .map(|icon| icon.position),
        camera.project(front, VIEWPORT)
    );
    assert!(
        reference()
            .project_map_icon([eye[0], 9.5, eye[2] - 5000.0], VIEWPORT, extent)
            .is_some()
    );
}

#[test]
fn the_lens_sets_the_projection_and_the_centre_ray_is_forward() {
    let lens = Lens {
        fov_y: 1.2,
        near: 0.25,
        far: 9000.0,
    };
    let camera = free(1.0, -0.4, 60.0, lens);
    let projection = camera.projection(VIEWPORT);
    assert!((projection[1][1] - 1.0 / 0.6f32.tan()).abs() < 1e-5);
    assert!((projection[3][2] - -0.25 * 9000.0 / (9000.0 - 0.25)).abs() < 1e-4);
    let ray = normalize(camera.ray([1024.0, 576.0], VIEWPORT));
    let forward = camera.look_direction();
    for axis in 0..3 {
        assert!((ray[axis] - forward[axis]).abs() < 1e-5);
    }
    let picked = camera.ray_at_ndc([0.0, 0.0], VIEWPORT);
    for axis in 0..3 {
        assert!((picked[axis] - forward[axis]).abs() < 1e-5);
    }
    let eye = camera.eye();
    let inside = |point: [f32; 3]| {
        camera
            .frustum_planes(VIEWPORT)
            .iter()
            .all(|[a, b, c, d]| a * point[0] + b * point[1] + c * point[2] + d >= 0.0)
    };
    let along = |distance: f32| std::array::from_fn(|axis| eye[axis] + forward[axis] * distance);
    assert!(inside(along(100.0)));
    assert!(!inside(along(9100.0)));
    assert!(!inside(along(0.1)));
}

fn turned(camera: Camera, [yaw, pitch, roll]: [f32; 3], travel: [f32; 2]) -> Camera {
    let Pose::Free(pose) = camera.pose else { panic!("a free eye") };
    Camera {
        center: [camera.center[0] + travel[0], camera.center[1] + travel[1]],
        pose: Pose::Free(Free { yaw: pose.yaw + yaw, pitch: pose.pitch + pitch, roll: pose.roll + roll, ..pose }),
        ..camera
    }
}

#[test]
fn a_turning_free_eye_gathers_from_a_held_eye_whose_ground_holds_every_view_it_covers() {
    let lens = Lens { fov_y: 0.96, near: 1.0, far: 20_000.0 };
    let start = free(std::f32::consts::FRAC_PI_2, -0.21, 100.0, lens);
    assert_eq!(start.turned_from(&start), Some((0.0, 0.0)));
    let (travel, turn) = start.turned_from(&turned(start, [0.03, 0.0, 0.0], [3.0, 4.0])).unwrap();
    assert!((travel - 5.0).abs() < 1e-4 && (turn - 2.0 * 0.015f32.sin()).abs() < 1e-4, "{travel} {turn}");
    let (_, small) = start.turned_from(&turned(start, [1e-4, 0.0, 0.0], [0.0, 0.0])).unwrap();
    assert!((small - 1e-4).abs() < 2e-6, "a ten-thousandth of a radian turns {small}");
    assert_eq!(start.turned_from(&Camera { height: 101.0, ..start }), None, "a climb is another shape");

    let mut hold = Hold::default();
    assert_eq!(hold.at(start, VIEWPORT), (start, Slack::NONE));
    let first = turned(start, [0.004, 0.0, 0.0], [0.0, 0.0]);
    assert_eq!(hold.at(first, VIEWPORT), (first, Slack::TURNING));
    let (mut camera, mut kept) = (first, 0);
    loop {
        camera = turned(camera, [0.004, 0.0, 0.0], [0.0, 0.0]);
        if hold.at(camera, VIEWPORT).0 != first {
            break;
        }
        kept += 1;
    }
    assert!(kept >= 10, "a turn of a quarter degree a frame keeps what it gathered {kept} frames");
    assert_eq!(hold.at(camera, VIEWPORT), (camera, Slack::NONE), "a stop gathers what it sees");
    let mut panning = Hold::default();
    panning.at(start, VIEWPORT);
    let east = Camera { center: [start.center[0] + 1.0, start.center[1]], ..start };
    assert_eq!(panning.at(east, VIEWPORT), (east, Slack::PAN));
    let further = Camera { center: [east.center[0] + 1.0, east.center[1]], ..east };
    assert_eq!(panning.at(further, VIEWPORT), (east, Slack::PAN));

    let mut tested = 0;
    for (yaw, pitch, height) in [(0.3, -0.21, 100.0), (2.0, -0.6, 100.0), (4.0, 0.25, 100.0), (5.5, -1.2, 100.0), (0.0, 0.481, 110.0)] {
        let held = free(yaw, pitch, height, lens);
        let ground = held.held_ground(VIEWPORT, 0.0, Slack::TURNING).unwrap();
        let a = 0.5 * TURN;
        for delta in [[a, 0.0, 0.0], [-a, 0.0, 0.0], [0.0, a, 0.0], [0.0, -a, 0.0], [0.0, 0.0, 2.0 * a], [0.6 * a, -0.6 * a, 0.6 * a]] {
            for travel in [[0.0, 0.0], [0.7 * PAD, 0.7 * PAD], [-0.7 * PAD, 0.7 * PAD]] {
                let moved = turned(held, delta, travel);
                let (travelled, turn) = held.turned_from(&moved).unwrap();
                assert!(travelled <= PAD && turn <= TURN, "{delta:?} {travel:?} is inside");
                let seen = moved.visible_ground(VIEWPORT, 0.0).unwrap();
                assert!(
                    ground[0] <= seen[0] && ground[1] <= seen[1] && seen[2] <= ground[2] && seen[3] <= ground[3],
                    "{yaw} {pitch} {delta:?} {travel:?}: {seen:?} outside {ground:?}"
                );
                tested += 1;
            }
        }
    }
    assert_eq!(tested, 90);
}

#[test]
fn holding_a_free_eye_wraps_x_keeps_z_on_the_map_and_lifts_it_off_the_floor() {
    let mut camera = free(0.0, 0.0, 12.0, Lens::MAP);
    camera.center = [-10.0, 5000.0];
    camera.hold_free(&|_, _| 20.0, 2.0);
    assert_eq!(camera.center, [MapSize::TEST.x() - 10.0, MapSize::TEST.y()]);
    assert_eq!(camera.height, 22.0);
    camera.height = 1.0e6;
    camera.hold_free(&|_, _| 20.0, 2.0);
    assert_eq!(camera.height, MapDefines::test().heights[1]);
    let mut map = reference();
    map.center[0] = -10.0;
    map.hold_free(&|_, _| 20.0, 2.0);
    assert_eq!(map.center[0], -10.0);
}

#[test]
fn a_free_pose_with_an_unusable_lens_or_orientation_is_refused() {
    for lens in [
        Lens {
            near: 0.0,
            ..Lens::MAP
        },
        Lens {
            far: 1.0,
            near: 2.0,
            ..Lens::MAP
        },
        Lens {
            fov_y: 0.0,
            ..Lens::MAP
        },
        Lens {
            fov_y: 3.2,
            ..Lens::MAP
        },
    ] {
        assert!(free(0.0, 0.0, 40.0, lens).validate().is_err(), "{lens:?}");
    }
    let mut camera = free(0.0, 0.0, 40.0, Lens::MAP);
    assert!(camera.validate().is_ok());
    if let Pose::Free(pose) = &mut camera.pose {
        pose.yaw = f32::NAN;
    }
    assert!(camera.validate().is_err());
}

#[test]
fn fixed5_camera_distances_convert_the_stored_integer_before_dividing_in_single_precision() {
    assert_eq!(fixed5_camera_distance(1_074_218_815), 10742.1875);
    assert_ne!(
        fixed5_camera_distance(1_074_218_815),
        (1_074_218_815_f64 / 100_000.0) as f32
    );
    assert_eq!(fixed5_camera_distance(90_000_000), 900.0);
}
