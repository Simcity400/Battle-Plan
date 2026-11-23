use super::*;
use crate::camera::Pose;

const REFERENCE_HOUR: i32 = 60_826_314;

#[test]
fn playback_lighting_moves_on_every_frame_at_each_rate() {
    for speed in [0.25, 1.0, 4.0, 24.0, 168.0, 720.0, 1440.0] {
        let mut clock = DayNightPlayback::default();
        let initial = DayNight::at_hour(18);
        assert_eq!(clock.update(18, initial, 0.0, 1.0 / speed), initial);
        let target = DayNight::at_hour(19);
        let mut previous = initial.hour_fraction;
        for frame in 1..=120 {
            let shown = clock.update(19, target, f64::from(frame) / (120.0 * speed), 1.0 / speed);
            assert!(shown.hour_fraction > previous);
            assert!(shown.hour_fraction <= target.hour_fraction);
            previous = shown.hour_fraction;
        }
        assert_eq!(
            clock
                .update(19, target, 2.0 / speed, 1.0 / speed)
                .hour_fraction,
            target.hour_fraction
        );
    }
}

#[test]
fn playback_lighting_wraps_midnight_forward_and_resets_without_history() {
    let mut clock = DayNightPlayback::default();
    clock.update(23, DayNight::at_hour(23), 0.0, 1.0);
    let middle = clock.update(24, DayNight::at_hour(24), 0.25, 1.0);
    assert!(middle.hour_fraction > 23.0 / 24.0);
    assert_eq!(
        clock
            .update(24, DayNight::at_hour(24), 2.0, 1.0)
            .hour_fraction,
        0.0
    );
    assert_eq!(
        clock
            .update(100, DayNight::at_hour(100), 2.1, 1.0)
            .hour_fraction,
        hour_fraction(100)
    );
    let mut sought = DayNightPlayback::default();
    let target = DayNight::at_hour(4).with_cycle(false);
    assert_eq!(sought.update(4, target, 10.0, 1.0), target);
}

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
    assert!((direction[2] - -0.0282).abs() < 4e-3, "{direction:?}");
    assert!((direction[3] - 0.99960).abs() < 1e-3, "{direction:?}");
}

#[test]
fn disabling_the_display_cycle_fixes_lights_without_changing_the_hour() {
    let map = MapSize::TEST;
    let defines = MapDefines::test();
    let active = DayNight::at_hour(REFERENCE_HOUR);
    let off = active.with_cycle(false);
    assert_eq!(off.strength, 0.0);
    assert_eq!(off.hour_sun_dir(), active.hour_sun_dir());
    assert_eq!(off.sun_position(map, &defines), [2816.0, 600.0, 848.0]);
    assert_eq!(off.moon_position(map, &defines), [0.0, 600.0, 1448.0]);
    let later = DayNight::at_hour(REFERENCE_HOUR + 12).with_cycle(false);
    assert_ne!(later.hour_fraction, off.hour_fraction);
    assert_eq!(
        later.sun_position(map, &defines),
        off.sun_position(map, &defines)
    );
    assert_eq!(
        later.moon_position(map, &defines),
        off.moon_position(map, &defines)
    );
    assert_eq!(off.with_cycle(true), active);
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
        pose: Pose::Map,
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
        pose: Pose::Map,
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

#[test]
fn the_capture_frame_constants_are_reproduced() {
    let camera = super::Camera {
        center: [3011.74, 1570.27],
        height: 263.493,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let constants = FrameConstants::try_new(
        camera,
        [2048, 1152],
        DayNight::at_hour(60_828_490),
        0.0,
        false,
    )
    .expect("valid reference camera");
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
fn the_sun_direction_normalizes_the_simulation_target() {
    for day in 0..365 {
        let hour = day * 24;
        let [x, y, z] = seasonal_direction(hour);
        assert_eq!(x, 0.0);
        assert!((y * y + z * z - 1.0).abs() <= 3.0 * f32::EPSILON);
    }
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

fn game_speeds(seconds: &str) -> Result<GameSpeeds, String> {
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    let source = format!("NDefines = {{ NGame = {{ GAME_SPEED_SECONDS = {{ {seconds} }} }} }}");
    defines.load("test.lua", source.as_bytes()).unwrap();
    GameSpeeds::load(&defines)
}

#[test]
fn the_cycle_shows_below_the_games_fastest_speed_and_while_paused() {
    let speeds = game_speeds("2.0, 0.5, 0.2, 0.1, 0.0").unwrap();
    assert_eq!(speeds, GameSpeeds::test());
    let rates = [0.25, 0.5, 1.0, 2.0, 4.0, 4.99, 5.0, 9.99, 10.0, 24.0, 1440.0];
    let levels = rates.map(|rate| speeds.level(rate));
    assert_eq!(levels, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 4]);
    assert!(speeds.shows_cycle(true, 9.99));
    assert!(!speeds.shows_cycle(true, 24.0));
    assert!(speeds.shows_cycle(false, 1440.0));
    let slow = game_speeds("2.0, 0.5, 0.2, 0.01, 0.0").unwrap();
    assert!(slow.shows_cycle(true, 24.0));
    let unbounded = game_speeds("2.0, 0.5, 0.2, 0.0, 0.0").unwrap();
    assert!(unbounded.shows_cycle(true, 1440.0));
}

#[test]
fn speeds_load_as_the_game_resizes_them() {
    let short = game_speeds("2.0, 0.5").unwrap();
    assert_eq!(short.level(1440.0), 2);
    assert!(short.shows_cycle(true, 1440.0));
    let long = game_speeds("2.0, 0.5, 0.2, 0.1, 0.0, 5.0").unwrap();
    assert_eq!(long, GameSpeeds::test());
    let negative = game_speeds("2.0, -0.5, 0.2, 0.1, 0.0").unwrap();
    assert_eq!(negative.level(1440.0), 1);
    let error = game_speeds(r#"2.0, "fast", 0.2, 0.1, 0.0"#).unwrap_err();
    assert!(error.contains("NDefines.NGame.GAME_SPEED_SECONDS"), "{error}");
}

#[test]
fn a_hidden_cycle_fades_out_rather_than_snapping() {
    let mut clock = DayNightPlayback::default();
    let shown = DayNight::at_hour(30);
    clock.update(30, shown, 0.0, 1.0);
    let hidden = shown.with_cycle(false);
    let first = clock.update(30, hidden, 1.0 / 60.0, 1.0).strength;
    assert!((first - 0.8).abs() < 1e-6, "{first}");
    assert_eq!(clock.update(30, hidden, 1.0, 1.0).strength, 0.0);
}

#[test]
fn a_mods_game_speeds_move_where_the_cycle_hides() {
    use content_reader_layers::ResolvedLayers;
    let root = std::env::temp_dir().join(format!("mod-game-speeds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (base, addon) = (root.join("base"), root.join("mod"));
    let write = |folder: &std::path::Path, text: &str| {
        std::fs::create_dir_all(folder.join("common/defines")).unwrap();
        std::fs::write(folder.join("common/defines/speeds.lua"), text).unwrap();
    };
    write(
        &base,
        "NDefines = { NGame = { GAME_SPEED_SECONDS = { 2.0, 0.5, 0.2, 0.1, 0.0 } } }",
    );
    let speeds = |layers: &ResolvedLayers| {
        GameSpeeds::load(&content_reader_defines::defines::read(layers).unwrap())
    };
    let vanilla = speeds(&ResolvedLayers::from_folders(&[(&base, &[])]).unwrap()).unwrap();
    assert!(!vanilla.shows_cycle(true, 24.0));
    std::fs::create_dir_all(addon.join("common/defines")).unwrap();
    std::fs::write(
        addon.join("common/defines/zz_speeds.lua"),
        "NDefines.NGame.GAME_SPEED_SECONDS = { 2.0, 0.5, 0.2, 0.01, 0.0 }",
    )
    .unwrap();
    let modded = ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    let modded = speeds(&modded).unwrap();
    assert_eq!(modded.level(24.0), 3);
    assert!(modded.shows_cycle(true, 24.0));
    std::fs::write(
        addon.join("common/defines/zz_speeds.lua"),
        r#"NDefines.NGame.GAME_SPEED_SECONDS = { 2.0, "fast", 0.2, 0.1, 0.0 }"#,
    )
    .unwrap();
    let unreadable = ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
    let why = speeds(&unreadable).unwrap_err();
    assert!(why.contains("NDefines.NGame.GAME_SPEED_SECONDS"), "{why}");
    std::fs::remove_dir_all(&root).unwrap();
}
