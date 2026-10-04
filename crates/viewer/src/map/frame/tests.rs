use super::*;

const REFERENCE_HOUR: i32 = 60_826_314;

#[test]
fn the_reference_hour_reproduces_the_report_sun_and_moon() {
    let day_night = DayNight::at_hour(REFERENCE_HOUR);
    assert_eq!(day_night.hour_fraction, 0.75);
    assert_eq!(
        day_night.sun_position(MapSize::TEST, &MapDefines::test()),
        [1408.0, 600.0, 848.0]
    );
    assert_eq!(
        day_night.moon_position(MapSize::TEST, &MapDefines::test()),
        [4224.0, 600.0, 1448.0]
    );
    let direction = day_night.hour_sun_dir();
    assert_eq!(direction[0], 0.75);
    assert_eq!(direction[1], 0.0);
    // The independent R1 terminator fit is within its 0.2-degree precision.
    assert!((direction[2] - -0.0282).abs() < 4e-3, "{direction:?}");
    assert!((direction[3] - 0.99960).abs() < 1e-3, "{direction:?}");
}

#[test]
fn the_hour_fraction_wraps_the_way_the_engine_does() {
    assert_eq!(hour_fraction(0), 0.0);
    assert_eq!(hour_fraction(43_800_000), 0.0);
    assert_eq!(hour_fraction(REFERENCE_HOUR + 1), 19.0 / 24.0);
    assert_eq!(hour_fraction(60_832_933), 13.0 / 24.0);
    assert_eq!(hour_fraction(-1), 23.0 / 24.0);
}

#[test]
fn the_shadow_fade_follows_the_camera_past_the_cutoff() {
    // Flat below the start of the fade, linear across it, zero above the
    // cutoff. The four fitted `pairE` framings straddle it: 50 and 164 are
    // heights where the engine draws map shadows, 627 and 1590 heights
    // where `GetShadowScaled` saturates to a shadow term of exactly 1.
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 50.178), 1.0);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 163.934), 1.0);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 350.0), 1.0);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 375.0), 0.5);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 400.0), 0.0);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 627.404), 0.0);
    assert_eq!(shadow_fade_factor(&MapDefines::test(), 1589.602), 0.0);
}

#[test]
fn invalid_camera_never_gets_identity_inverse_constants() {
    let mut camera = Camera {
        center: [100.0, 100.0],
        height: super::super::camera::WATER_HEIGHT,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    camera.defines.tilt_lead = 0.0;
    assert!(
        FrameConstants::try_new(
            camera,
            [32, 32],
            DayNight::at_hour(REFERENCE_HOUR),
            0.0,
            false
        )
        .is_none()
    );
    camera.height = f32::NAN;
    assert!(
        FrameConstants::try_new(
            camera,
            [32, 32],
            DayNight::at_hour(REFERENCE_HOUR),
            0.0,
            false
        )
        .is_none()
    );
}

#[test]
fn the_constant_buffer_matches_the_reference_frame() {
    let camera = super::Camera {
        center: [3010.0, 1568.0],
        height: 310.178,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    let constants = FrameConstants::try_new(
        camera,
        [2048, 1152],
        DayNight::at_hour(REFERENCE_HOUR),
        0.0,
        false,
    )
    .expect("valid reference camera");
    assert_eq!(constants.virtual_sun_pos, [1408.0, 600.0, 848.0, 5000.0]);
    assert_eq!(constants.virtual_moon_pos, [4224.0, 600.0, 1448.0, 550.0]);
    assert_eq!(
        constants.second_virtual_sun_pos,
        [4224.0, 600.0, 100.0, 5000.0]
    );
    assert_eq!(
        constants.second_virtual_moon_pos,
        [1408.0, 600.0, 100.0, 550.0]
    );
    assert_eq!(constants.cam_pos_hdr_range, [3010.0, 310.178, 1538.0, 1.0]);
    let look = constants.cam_look_at_dir_global_time;
    assert!((look[0]).abs() < 1e-6);
    assert!((look[1] - -0.995059).abs() < 1e-5, "{look:?}");
    assert!((look[2] - 0.099282).abs() < 1e-5, "{look:?}");
}

/// The game's own `cb0`, read out of `hoi4_frame33840.rdc` at draw event
/// 2777 (the `terrain` effect), against what this file writes for the same
/// hour and the camera that capture's `vCamPos` and `vCamLookAtDir` name.
///
/// Every field is reproduced, `vGBCamDistOverride_GBOutlineCutoff` included:
/// its `(1, 0, 0.98, 0.973)` is `STRATEGIC_REGIONS` in layer 1 and `COUNTRY`
/// in layer 2, and [`super::political`] now builds those two layers in that
/// order.
#[test]
fn the_capture_frame_constants_are_reproduced() {
    // `vCamPos` is the eye and the camera model puts the eye at
    // `(cx, h, cz - CAMERA_TILT)`, so the capture's camera is
    // (3011.74, 1570.27, 263.493); it reproduces the capture's own
    // `ViewProjectionMatrix` to the precision that is read at.
    let camera = super::Camera {
        center: [3011.74, 1570.27],
        height: 263.493,
        map: MapSize::TEST,
        defines: MapDefines::test(),
    };
    let constants = FrameConstants::try_new(
        camera,
        [2048, 1152],
        DayNight::at_hour(60_828_490),
        0.0,
        false,
    )
    .expect("valid reference camera");
    // The capture prints six significant figures, so the tolerance is
    // absolute near zero and relative for the world-space fields.
    let close = |left: f32, right: f32, name: &str| {
        let slack = 1e-3 + 1e-5 * right.abs();
        assert!((left - right).abs() <= slack, "{name}: {left} != {right}");
    };
    let all = |got: [f32; 4], want: [f32; 4], name: &str| {
        for (index, value) in want.into_iter().enumerate() {
            close(got[index], value, name);
        }
    };
    all(
        constants.virtual_sun_pos,
        [3285.33, 600.0, 848.0, 5000.0],
        "vVirtualSunPos",
    );
    all(
        constants.virtual_moon_pos,
        [469.334, 600.0, 1448.0, 550.0],
        "vVirtualMoonPos",
    );
    all(
        constants.second_virtual_sun_pos,
        [469.333, 600.0, 100.0, 5000.0],
        "vSecondVirtualSunPos",
    );
    all(
        constants.second_virtual_moon_pos,
        [3285.33, 600.0, 100.0, 550.0],
        "vSecondVirtualMoonPos",
    );
    all(
        constants.cam_pos_hdr_range,
        [3011.74, 263.493, 1540.27, 1.0],
        "vCamPos and HdrRange",
    );
    for (index, value) in [0.0, -0.993_097, 0.117_298].into_iter().enumerate() {
        close(
            constants.cam_look_at_dir_global_time[index],
            value,
            "vCamLookAtDir",
        );
    }
    // `vFoWOpacity` 0 with no fog, `FoWTime` the engine's animation clock
    // (0 here, for the reason that field's own documentation gives),
    // `SnowMudFade` 1 and `MaxGameSpeed` 1.
    assert_eq!(
        constants.fow_opacity_time_snow_mud_day_night,
        [0.0, 0.0, 1.0, 1.0]
    );
    close(
        constants.day_night_hour_sun_dir[0],
        0.416_667,
        "DayNight hour",
    );
    close(constants.day_night_hour_sun_dir[1], 0.0, "sun direction y");
    // The captured 0.0585195 now follows from the integer weather target,
    // including the engine's approximate trig and renderer normalization.
    assert!((constants.day_night_hour_sun_dir[2] - 0.058_519_5).abs() < 1e-7);
    all(
        constants.ambient_pos_x_shadow_fade,
        [0.026_501, 0.028_991_2, 0.017_744_5, 1.0],
        "AmbientPosX and ShadowFadeFactor",
    );
    all(
        constants.ambient_neg_x_fow_fade,
        [0.257_79, 0.325_037, 0.283_67, 0.0],
        "AmbientNegX and FOWFadeFactor",
    );
    assert_eq!(constants.ambient_pos_y_min_mesh_alpha, [0.0; 4]);
    assert_eq!(constants.ambient_neg_y_neg_fog, [0.0, 0.0, 0.0, 1.0]);
    all(
        constants.ambient_pos_z,
        [0.514_371, 0.634_366, 0.840_385, 0.0],
        "AmbientPosZ",
    );
    all(
        constants.ambient_neg_z_cubemap,
        [0.629_024, 0.741_705, 0.793_11, 1.0],
        "AmbientNegZ and CubemapIntensity",
    );
    assert_eq!(constants.sun_diffuse_intensity, [1.0; 4]);
    all(
        constants.moon_diffuse_intensity,
        [0.217_638, 0.546_751, 1.0, 2.5],
        "MoonDiffuseIntensity",
    );
    assert_eq!(constants.gb_texture_height_sun_specular[0], 1024.0);
    assert_eq!(constants.gb_texture_height_sun_specular[1], 1.0);
    // `FOWFadeFactor` with the fog on: the engine writes 0.152116 at this
    // camera and 0.9993 at the minimum-height capture's 50.1225.
    close(
        fow_fade_factor(&MapDefines::test(), 263.493, true),
        0.152_116,
        "FOWFadeFactor h290",
    );
    close(
        fow_fade_factor(&MapDefines::test(), 50.1225, true),
        0.9993,
        "FOWFadeFactor hmin",
    );
    assert_eq!(
        constants.gb_cam_dist_override_outline_cutoff,
        [1.0, 0.0, 0.98, 0.973]
    );
}

#[test]
fn seasonal_turns_use_zero_based_dates_and_truncated_fixed_point() {
    assert_eq!(seasonal_rotation(172 * 24), -500_000);
    assert_eq!(seasonal_rotation(355 * 24), 500_000);
    assert_eq!(seasonal_rotation(173 * 24), -494_540);
    assert_eq!(seasonal_rotation(356 * 24), 494_510);
    assert_eq!(seasonal_rotation(0), 445_060);
    for day in 0..365 {
        let hour = day * 24;
        assert_eq!(
            seasonal_direction_raw(hour),
            seasonal_direction_raw(hour + 23)
        );
        assert_eq!(
            seasonal_direction_raw(hour),
            seasonal_direction_raw(hour + 365 * 24)
        );
        assert_eq!(
            seasonal_direction_raw(hour),
            seasonal_direction_raw(hour - 365 * 24)
        );
        assert!(seasonal_rotation(hour).abs() <= 500_000);
        let [x, y, z] = seasonal_direction(hour);
        assert_eq!(x, 0.0);
        // Squaring the rounded f32 components adds another rounding step.
        assert!((y * y + z * z - 1.0).abs() <= 3.0 * f32::EPSILON);
    }
}

#[test]
fn seasonal_trig_keeps_native_quadrants_and_polynomial_error() {
    assert_eq!(seasonal_sine(0), 0);
    assert_eq!(seasonal_sine(157_080), 100_453);
    assert_eq!(seasonal_sine(314_159), 1);
    assert_eq!(seasonal_sine(628_319), 0);
    assert_eq!(seasonal_sine(-157_080), -100_453);
    assert_eq!(seasonal_direction_raw(264 * 24)[2], SEASON_SCALE);
}

#[test]
fn the_buffer_layout_matches_the_hlsl_registers() {
    assert_eq!(std::mem::size_of::<FrameConstants>(), 416);
    let zero = FrameConstants::zeroed();
    let base = std::ptr::from_ref(&zero) as usize;
    let offset = |field: &[f32; 4]| std::ptr::from_ref(field) as usize - base;
    assert_eq!(offset(&zero.gb_cam_dist_override_outline_cutoff), 128);
    assert_eq!(offset(&zero.cam_pos_hdr_range), 208);
    assert_eq!(offset(&zero.day_night_hour_sun_dir), 256);
    assert_eq!(offset(&zero.ambient_pos_z), 336);
    assert_eq!(offset(&zero.gb_texture_height_sun_specular), 400);
}
