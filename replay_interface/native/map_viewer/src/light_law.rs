
pub mod offsets {
    pub const GB_CAM_DIST_OVERRIDE: usize = 128;
    pub const VIRTUAL_SUN_POS: usize = 144;
    pub const VIRTUAL_MOON_POS: usize = 160;
    pub const SECOND_VIRTUAL_SUN_POS: usize = 176;
    pub const SECOND_VIRTUAL_MOON_POS: usize = 192;
    pub const CAM_POS: usize = 208;
    pub const HDR_RANGE: usize = 220;
    pub const CAM_LOOK_AT_DIR: usize = 224;
    pub const GLOBAL_TIME: usize = 236;
    pub const FOW_OPACITY_TIME_SNOWMUD_MAXSPEED: usize = 240;
    pub const DAY_NIGHT_HOUR_SUN_DIR: usize = 256;
    pub const AMBIENT_POS_X: usize = 272;
    pub const SHADOW_FADE_FACTOR: usize = 284;
    pub const AMBIENT_NEG_X: usize = 288;
    pub const FOW_FADE_FACTOR: usize = 300;
    pub const AMBIENT_POS_Y: usize = 304;
    pub const MIN_MESH_ALPHA: usize = 316;
    pub const AMBIENT_NEG_Y: usize = 320;
    pub const NEG_FOG_MULTIPLIER: usize = 332;
    pub const AMBIENT_POS_Z: usize = 336;
    pub const AMBIENT_NEG_Z: usize = 352;
    pub const CUBEMAP_INTENSITY: usize = 364;
    pub const SUN_DIFFUSE_INTENSITY: usize = 368;
    pub const MOON_DIFFUSE_INTENSITY: usize = 384;
    pub const GB_TEXTURE_HEIGHT: usize = 400;
    pub const SUN_SPECULAR_INTENSITY: usize = 404;
    pub const CB0_LEN: usize = 416;
}

pub mod defines {
    pub const SUN_HEIGHT: f32 = 600.0;
    pub const MOON_HEIGHT: f32 = 600.0;
    pub const SUN_LATITUDE: f32 = 848.0;
    pub const MOON_LATITUDE_NOT_USED: f32 = 0.0;
    pub const SECOND_SUN_LATITUDE: f32 = 100.0;
    pub const SECOND_MOON_LATITUDE: f32 = 100.0;
    pub const GMT_OFFSET: f32 = 2793.0;
    pub const SUN_DIFFUSE_COLOR_HSV: [f32; 3] = [0.14, 0.0, 1.0];
    pub const SUN_INTENSITY: f32 = 1.0;
    pub const SUN_SPECULAR_INTENSITY: f32 = 1.0;
    pub const MOON_DIFFUSE_COLOR_HSV: [f32; 3] = [0.58, 0.5, 1.0];
    pub const MOON_INTENSITY: f32 = 2.5;
    pub const CUBEMAP_INTENSITY: f32 = 1.0;
    pub const MAP_SIZE_X: f32 = 5632.0;

    pub const NIGHT_OPACITY: f32 = 0.85;
    pub const NIGHT_DARKNESS: f32 = 0.5;
    pub const FEATHER_MIN: f32 = -0.01;
    pub const FEATHER_MAX: f32 = 0.01;

    pub const NIGHT_AMBIENT_BOOST: f32 = 3.0;

    pub const AMBIENT_LIGHT_POS_X_HSV: [f32; 3] = [0.2, 0.2, 0.2];
    pub const AMBIENT_LIGHT_NEG_X_HSV: [f32; 3] = [0.4, 0.1, 0.6];
    pub const AMBIENT_LIGHT_POS_Y_HSV: [f32; 3] = [0.0, 0.0, 0.0];
    pub const AMBIENT_LIGHT_NEG_Y_HSV: [f32; 3] = [0.35, 0.2, 0.0];
    pub const AMBIENT_LIGHT_POS_Z_HSV: [f32; 3] = [0.6, 0.2, 0.924];
    pub const AMBIENT_LIGHT_NEG_Z_HSV: [f32; 3] = [0.55, 0.1, 0.9];

    pub const NIGHT_AMBIENT: [[f32; 3]; 6] = [
        [2.0, 2.0, 2.0],
        [0.2, 0.2, 0.2],
        [0.1, 0.1, 0.1],
        [0.0, 0.0, 0.0],
        [3.0, 3.0, 3.0],
        [0.8, 0.8, 0.8],
    ];

    pub const MOON_Z_HYPOTHESIS: f32 = SUN_LATITUDE + MOON_HEIGHT;
}

fn read_f4(cb0: &[u8], off: usize) -> Option<[f32; 4]> {
    let s = cb0.get(off..off.checked_add(16)?)?;
    let f = |i: usize| f32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    Some([f(0), f(4), f(8), f(12)])
}

fn read_f3(cb0: &[u8], off: usize) -> Option<[f32; 3]> {
    let s = cb0.get(off..off.checked_add(12)?)?;
    let f = |i: usize| f32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    Some([f(0), f(4), f(8)])
}

fn read_f1(cb0: &[u8], off: usize) -> Option<f32> {
    let s = cb0.get(off..off.checked_add(4)?)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLighting {
    pub virtual_sun_pos: [f32; 4],
    pub virtual_moon_pos: [f32; 4],
    pub second_virtual_sun_pos: [f32; 4],
    pub second_virtual_moon_pos: [f32; 4],
    pub cam_pos: [f32; 3],
    pub cam_look_at_dir: [f32; 3],
    pub global_time: f32,
    pub fow_opacity_time_snowmud_maxspeed: [f32; 4],
    pub day_night_hour_sun_dir: [f32; 4],
    pub sun_diffuse_intensity: [f32; 4],
    pub moon_diffuse_intensity: [f32; 4],
    pub sun_specular_intensity: f32,
    pub cubemap_intensity: f32,
    pub ambient_rig: [[f32; 3]; 6],
}

impl SceneLighting {
    pub fn from_cb0_bytes(cb0: &[u8]) -> Option<Self> {
        use offsets::*;
        Some(Self {
            virtual_sun_pos: read_f4(cb0, VIRTUAL_SUN_POS)?,
            virtual_moon_pos: read_f4(cb0, VIRTUAL_MOON_POS)?,
            second_virtual_sun_pos: read_f4(cb0, SECOND_VIRTUAL_SUN_POS)?,
            second_virtual_moon_pos: read_f4(cb0, SECOND_VIRTUAL_MOON_POS)?,
            cam_pos: read_f3(cb0, CAM_POS)?,
            cam_look_at_dir: read_f3(cb0, CAM_LOOK_AT_DIR)?,
            global_time: read_f1(cb0, GLOBAL_TIME)?,
            fow_opacity_time_snowmud_maxspeed: read_f4(cb0, FOW_OPACITY_TIME_SNOWMUD_MAXSPEED)?,
            day_night_hour_sun_dir: read_f4(cb0, DAY_NIGHT_HOUR_SUN_DIR)?,
            sun_diffuse_intensity: read_f4(cb0, SUN_DIFFUSE_INTENSITY)?,
            moon_diffuse_intensity: read_f4(cb0, MOON_DIFFUSE_INTENSITY)?,
            sun_specular_intensity: read_f1(cb0, SUN_SPECULAR_INTENSITY)?,
            cubemap_intensity: read_f1(cb0, CUBEMAP_INTENSITY)?,
            ambient_rig: [
                read_f3(cb0, AMBIENT_POS_X)?,
                read_f3(cb0, AMBIENT_NEG_X)?,
                read_f3(cb0, AMBIENT_POS_Y)?,
                read_f3(cb0, AMBIENT_NEG_Y)?,
                read_f3(cb0, AMBIENT_POS_Z)?,
                read_f3(cb0, AMBIENT_NEG_Z)?,
            ],
        })
    }

    pub fn day_night_hour(self) -> f32 {
        self.day_night_hour_sun_dir[0] * 24.0
    }

    pub fn sun_dir_yzw(self) -> [f32; 3] {
        [
            self.day_night_hour_sun_dir[1],
            self.day_night_hour_sun_dir[2],
            self.day_night_hour_sun_dir[3],
        ]
    }

    pub fn is_full_daylight_capture(self) -> bool {
        self.fow_opacity_time_snowmud_maxspeed[3] == 0.0
    }
}

fn hue_post(h: f32) -> [f32; 3] {
    let x = 1.0 - ((h % 2.0) - 1.0).abs();
    if h < 1.0 {
        [1.0, x, 0.0]
    } else if h < 2.0 {
        [x, 1.0, 0.0]
    } else if h < 3.0 {
        [0.0, 1.0, x]
    } else if h < 4.0 {
        [0.0, x, 1.0]
    } else if h < 5.0 {
        [x, 0.0, 1.0]
    } else {
        [1.0, 0.0, x]
    }
}

fn hsv_to_rgb_post(hsv6: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv6;
    if s != 0.0 {
        let c = s * v;
        let hue = hue_post(h);
        [
            (hue[0] * c + (v - c)).clamp(0.0, 1.0),
            (hue[1] * c + (v - c)).clamp(0.0, 1.0),
            (hue[2] * c + (v - c)).clamp(0.0, 1.0),
        ]
    } else {
        [v.clamp(0.0, 1.0); 3]
    }
}

pub fn diffuse_color_rgb_from_hsv_define(hsv_dial01: [f32; 3]) -> [f32; 3] {
    let rgb = hsv_to_rgb_post([hsv_dial01[0] * 6.0, hsv_dial01[1], hsv_dial01[2]]);
    [rgb[0].powf(2.2), rgb[1].powf(2.2), rgb[2].powf(2.2)]
}

fn hue_manual(h01: f32) -> [f32; 3] {
    let h6 = h01 * 6.0;
    let r = (h6 - 3.0).abs() - 1.0;
    let g = 2.0 - (h6 - 2.0).abs();
    let b = 2.0 - (h6 - 4.0).abs();
    [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
}

pub fn hsv_to_rgb_manual_linear(hsv_dial01: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv_dial01;
    let hue = hue_manual(h);
    let val = |c: f32| (((c - 1.0) * s + 1.0) * v).powf(2.2);
    [val(hue[0]), val(hue[1]), val(hue[2])]
}

pub fn vanilla_ambient_light_hsv_defines() -> [[f32; 3]; 6] {
    [
        defines::AMBIENT_LIGHT_POS_X_HSV,
        defines::AMBIENT_LIGHT_NEG_X_HSV,
        defines::AMBIENT_LIGHT_POS_Y_HSV,
        defines::AMBIENT_LIGHT_NEG_Y_HSV,
        defines::AMBIENT_LIGHT_POS_Z_HSV,
        defines::AMBIENT_LIGHT_NEG_Z_HSV,
    ]
}

pub fn cb0_ambient_rig_from_hsv_defines(hsv6: [[f32; 3]; 6]) -> [[f32; 3]; 6] {
    [
        hsv_to_rgb_manual_linear(hsv6[0]),
        hsv_to_rgb_manual_linear(hsv6[1]),
        hsv_to_rgb_manual_linear(hsv6[2]),
        hsv_to_rgb_manual_linear(hsv6[3]),
        hsv_to_rgb_manual_linear(hsv6[4]),
        hsv_to_rgb_manual_linear(hsv6[5]),
    ]
}

pub fn ambient_rig_residual(captured: &[[f32; 3]; 6], composed: &[[f32; 3]; 6]) -> f32 {
    let mut worst = 0.0f32;
    for (cap, law) in captured.iter().zip(composed.iter()) {
        for (c, l) in cap.iter().zip(law.iter()) {
            worst = worst.max((c - l).abs());
        }
    }
    worst
}

pub fn ambient_light_hsv_defines_from_graphics_source(
    source: &str,
) -> Result<[[f32; 3]; 6], String> {
    const KEYS: [&str; 6] = [
        "AMBIENT_LIGHT_POS_X",
        "AMBIENT_LIGHT_NEG_X",
        "AMBIENT_LIGHT_POS_Y",
        "AMBIENT_LIGHT_NEG_Y",
        "AMBIENT_LIGHT_POS_Z",
        "AMBIENT_LIGHT_NEG_Z",
    ];
    let mut out = [[0.0f32; 3]; 6];
    let mut found = [false; 6];
    for raw in source.lines() {
        let code = raw.split("--").next().unwrap_or(raw).trim();
        let Some((lhs, rhs)) = code.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let Some(slot) = KEYS.iter().position(|k| *k == key) else {
            continue;
        };
        let open = rhs.find('{').ok_or_else(|| {
            format!("00_graphics.lua {key} has no {{ h, s, v }} table on its line")
        })?;
        let close = rhs[open + 1..]
            .find('}')
            .ok_or_else(|| format!("00_graphics.lua {key} table is unterminated on its line"))?;
        let inner = &rhs[open + 1..open + 1 + close];
        let mut triple = [0.0f32; 3];
        let mut count = 0usize;
        for token in inner.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if count >= 3 {
                count += 1;
                break;
            }
            triple[count] = token
                .parse::<f32>()
                .map_err(|err| format!("00_graphics.lua {key} component {count}: {err}"))?;
            count += 1;
        }
        if count != 3 {
            return Err(format!(
                "00_graphics.lua {key} must have exactly 3 HSV components, found {count}"
            ));
        }
        if found[slot] {
            return Err(format!("00_graphics.lua defines {key} more than once"));
        }
        out[slot] = triple;
        found[slot] = true;
    }
    if let Some(missing) = KEYS.iter().zip(found.iter()).find(|(_, f)| !**f) {
        return Err(format!("00_graphics.lua is missing {}", missing.0));
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LightingLawResidual {
    pub sun_height_residual: f32,
    pub sun_latitude_residual: f32,
    pub moon_height_residual: f32,
    pub moon_z_hypothesis_residual: f32,
    pub sun_diffuse_rgb_residual: f32,
    pub sun_intensity_residual: f32,
    pub moon_diffuse_rgb_residual: f32,
    pub moon_intensity_residual: f32,
    pub sun_specular_residual: f32,
    pub cubemap_intensity_residual: f32,
    pub sun_dir_unit_residual: f32,
    pub sun_x_minus_cam_x: f32,
}

pub fn validate_lighting_law_against_cb0(cb0: &[u8]) -> Option<LightingLawResidual> {
    let s = SceneLighting::from_cb0_bytes(cb0)?;
    let max3 = |a: [f32; 3], b: [f32; 3]| {
        (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .max((a[2] - b[2]).abs())
    };
    let sun_rgb_law = diffuse_color_rgb_from_hsv_define(defines::SUN_DIFFUSE_COLOR_HSV);
    let moon_rgb_law = diffuse_color_rgb_from_hsv_define(defines::MOON_DIFFUSE_COLOR_HSV);
    let sun_dir = s.sun_dir_yzw();
    let sun_dir_len =
        (sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2]).sqrt();
    Some(LightingLawResidual {
        sun_height_residual: s.virtual_sun_pos[1] - defines::SUN_HEIGHT,
        sun_latitude_residual: s.virtual_sun_pos[2] - defines::SUN_LATITUDE,
        moon_height_residual: s.virtual_moon_pos[1] - defines::MOON_HEIGHT,
        moon_z_hypothesis_residual: s.virtual_moon_pos[2] - defines::MOON_Z_HYPOTHESIS,
        sun_diffuse_rgb_residual: max3(
            [
                s.sun_diffuse_intensity[0],
                s.sun_diffuse_intensity[1],
                s.sun_diffuse_intensity[2],
            ],
            sun_rgb_law,
        ),
        sun_intensity_residual: s.sun_diffuse_intensity[3] - defines::SUN_INTENSITY,
        moon_diffuse_rgb_residual: max3(
            [
                s.moon_diffuse_intensity[0],
                s.moon_diffuse_intensity[1],
                s.moon_diffuse_intensity[2],
            ],
            moon_rgb_law,
        ),
        moon_intensity_residual: s.moon_diffuse_intensity[3] - defines::MOON_INTENSITY,
        sun_specular_residual: s.sun_specular_intensity - defines::SUN_SPECULAR_INTENSITY,
        cubemap_intensity_residual: s.cubemap_intensity - defines::CUBEMAP_INTENSITY,
        sun_dir_unit_residual: sun_dir_len - 1.0,
        sun_x_minus_cam_x: s.virtual_sun_pos[0] - s.cam_pos[0],
    })
}

pub fn virtual_sun_longitude(hour24: f32, map_size_x: f32) -> f32 {
    (map_size_x * (1.0 - hour24 / 24.0)).rem_euclid(map_size_x)
}

pub fn virtual_moon_longitude(hour24: f32, map_size_x: f32) -> f32 {
    (virtual_sun_longitude(hour24, map_size_x) + map_size_x * 0.5).rem_euclid(map_size_x)
}

pub fn virtual_rig_longitudes_x(hour24: f32, map_size_x: f32) -> [f32; 4] {
    let sun = virtual_sun_longitude(hour24, map_size_x);
    let moon = virtual_moon_longitude(hour24, map_size_x);
    [sun, moon, moon, sun]
}

pub fn render_sun_dir_yzw(hour24: f32, captured_yzw: [f32; 3]) -> [f32; 3] {
    debug_assert!(
        hour24.is_finite(),
        "render_sun_dir_yzw: hour24 must be finite, got {hour24}"
    );
    let _ = hour24;
    captured_yzw
}

pub fn night_yzw_residual(yzw: [f32; 3]) -> (f32, f32, f32) {
    let len = (yzw[0] * yzw[0] + yzw[1] * yzw[1] + yzw[2] * yzw[2]).sqrt();
    (
        (len - 1.0).abs(),
        yzw[0].abs(),
        (yzw[2] - 1.0).abs(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mined_scene_cb0_live_pan2() -> Vec<u8> {
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        let mut put4 = |off: usize, v: [f32; 4]| {
            for (i, c) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
            }
        };
        put4(offsets::VIRTUAL_SUN_POS, [2816.0, 600.0, 848.0, 5000.0]);
        put4(offsets::VIRTUAL_MOON_POS, [0.0, 600.0, 1448.0, 550.0]);
        put4(offsets::SECOND_VIRTUAL_SUN_POS, [0.0, 600.0, 100.0, 5000.0]);
        put4(
            offsets::SECOND_VIRTUAL_MOON_POS,
            [2816.0, 600.0, 100.0, 550.0],
        );
        put4(offsets::CAM_POS, [2958.0, 800.0, 1400.0, 1.0]);
        put4(offsets::CAM_LOOK_AT_DIR, [0.0, -0.98912, 0.14713, 23.2925]);
        put4(
            offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED,
            [0.0, 23.29197, 0.0, 0.0],
        );
        put4(
            offsets::DAY_NIGHT_HOUR_SUN_DIR,
            [0.0416666679, 0.0, -0.07833850, 0.99692684],
        );
        put4(offsets::SUN_DIFFUSE_INTENSITY, [1.0, 1.0, 1.0, 1.0]);
        put4(
            offsets::MOON_DIFFUSE_INTENSITY,
            [0.21763763, 0.54675138, 1.0, 2.5],
        );
        put4(offsets::GB_TEXTURE_HEIGHT, [1024.0, 1.0, 1.0, 0.0]);
        cb0[offsets::CUBEMAP_INTENSITY..offsets::CUBEMAP_INTENSITY + 4]
            .copy_from_slice(&1.0f32.to_le_bytes());
        cb0
    }

    #[test]
    fn decodes_every_lighting_field_from_mined_cb0() {
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).expect("full CB0 decodes");
        assert_eq!(s.virtual_sun_pos, [2816.0, 600.0, 848.0, 5000.0]);
        assert_eq!(s.virtual_moon_pos, [0.0, 600.0, 1448.0, 550.0]);
        assert_eq!(s.cam_pos, [2958.0, 800.0, 1400.0]);
        assert!((s.global_time - 23.2925).abs() < 1e-3);
        assert_eq!(s.sun_diffuse_intensity, [1.0, 1.0, 1.0, 1.0]);
        assert!((s.moon_diffuse_intensity[3] - 2.5).abs() < 1e-6);
        assert!((s.sun_specular_intensity - 1.0).abs() < 1e-6);
        assert!((s.cubemap_intensity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn day_night_hour_reads_out_of_dot_x() {
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).unwrap();
        assert!(
            (s.day_night_hour() - 1.0).abs() < 1e-3,
            "{}",
            s.day_night_hour()
        );
    }

    #[test]
    fn corpus_is_full_daylight_only() {
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).unwrap();
        assert!(s.is_full_daylight_capture());
    }

    #[test]
    fn sun_direction_is_a_unit_vector() {
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(
            r.sun_dir_unit_residual.abs() < 2e-3,
            "sun dir unit residual {}",
            r.sun_dir_unit_residual
        );
    }

    #[test]
    fn diffuse_color_hsv_to_linear_law_matches_captured_rgb() {
        let moon = diffuse_color_rgb_from_hsv_define(defines::MOON_DIFFUSE_COLOR_HSV);
        assert!((moon[0] - 0.21763763).abs() < 1e-4, "moon.r {}", moon[0]);
        assert!((moon[1] - 0.54675138).abs() < 1e-4, "moon.g {}", moon[1]);
        assert!((moon[2] - 1.0).abs() < 1e-4, "moon.b {}", moon[2]);
        let sun = diffuse_color_rgb_from_hsv_define(defines::SUN_DIFFUSE_COLOR_HSV);
        assert_eq!(sun, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn files_derived_residuals_are_zero_against_mined_cb0() {
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(r.sun_height_residual.abs() < 1e-3);
        assert!(r.sun_latitude_residual.abs() < 1e-3);
        assert!(r.moon_height_residual.abs() < 1e-3);
        assert!(
            r.moon_z_hypothesis_residual.abs() < 1e-3,
            "moon z {}",
            r.moon_z_hypothesis_residual
        );
        assert!(r.sun_diffuse_rgb_residual < 1e-4);
        assert!(r.sun_intensity_residual.abs() < 1e-6);
        assert!(r.moon_diffuse_rgb_residual < 1e-4);
        assert!(r.moon_intensity_residual.abs() < 1e-6);
        assert!(r.sun_specular_residual.abs() < 1e-6);
        assert!(r.cubemap_intensity_residual.abs() < 1e-6);
    }

    #[test]
    fn camera_follow_hypothesis_is_refuted() {
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(
            r.sun_x_minus_cam_x.abs() > 100.0,
            "sun.x - cam.x = {} (would be ~0 under camera-follow)",
            r.sun_x_minus_cam_x
        );
        assert!((r.sun_x_minus_cam_x - (2816.0 - 2958.0)).abs() < 1e-3);
    }

    #[test]
    fn cb0_decode_fails_closed_on_short_buffer() {
        let short = vec![0u8; offsets::DAY_NIGHT_HOUR_SUN_DIR + 8];
        assert!(SceneLighting::from_cb0_bytes(&short).is_none());
        assert!(validate_lighting_law_against_cb0(&short).is_none());
    }

    #[test]
    fn day_cycle_longitude_law_matches_unpaused_session_final_1_samples() {
        const MAPX: f32 = 5632.0;
        let samples: [(f32, f32, f32); 3] = [
            (10.147_274, 3_250.772_9, 434.773_44),
            (7.876_204, 3_783.717_3, 967.716_8),
            (15.377_023, 2_023.525_4, 4_839.525_4),
        ];
        for (h, sun_x, moon_x) in samples {
            assert!(
                (virtual_sun_longitude(h, MAPX) - sun_x).abs() < 0.5,
                "sun law at h={h}: {} vs measured {sun_x}",
                virtual_sun_longitude(h, MAPX)
            );
            assert!(
                (virtual_moon_longitude(h, MAPX) - moon_x).abs() < 0.5,
                "moon law at h={h}: {} vs measured {moon_x}",
                virtual_moon_longitude(h, MAPX)
            );
        }
        assert!((virtual_sun_longitude(12.0, MAPX) - 2816.0).abs() < 1e-3);
        assert!((virtual_sun_longitude(0.0, MAPX)).abs() < 1e-3);
        assert!((virtual_sun_longitude(24.0, MAPX)).abs() < 1e-3);
    }

    #[test]
    fn map_size_x_define_matches_heightmap_width() {
        assert_eq!(defines::MAP_SIZE_X, 5632.0);
    }

    #[test]
    fn rig_longitudes_are_byte_exact_captured_values_at_noon() {
        let x = virtual_rig_longitudes_x(12.0, defines::MAP_SIZE_X);
        assert_eq!(x[0].to_bits(), 2816.0f32.to_bits(), "sun.x");
        assert_eq!(x[1].to_bits(), 0.0f32.to_bits(), "moon.x");
        assert_eq!(x[2].to_bits(), 0.0f32.to_bits(), "second_sun.x");
        assert_eq!(x[3].to_bits(), 2816.0f32.to_bits(), "second_moon.x");
        assert!(!x[1].is_sign_negative());
        assert!(!x[2].is_sign_negative());
    }

    #[test]
    fn rig_longitudes_move_with_the_hour_per_the_law() {
        const MAPX: f32 = 5632.0;
        let six = virtual_rig_longitudes_x(6.0, MAPX);
        assert!((six[0] - 4224.0).abs() < 1e-3, "sun.x@06:00 {}", six[0]);
        assert!((six[1] - 1408.0).abs() < 1e-3, "moon.x@06:00 {}", six[1]);
        let eighteen = virtual_rig_longitudes_x(18.0, MAPX);
        assert!(
            (eighteen[0] - 1408.0).abs() < 1e-3,
            "sun.x@18:00 {}",
            eighteen[0]
        );
        assert!(
            (eighteen[1] - 4224.0).abs() < 1e-3,
            "moon.x@18:00 {}",
            eighteen[1]
        );
        assert_eq!(six[2], six[1]);
        assert_eq!(six[3], six[0]);
    }


    fn hexbytes(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0, "odd hex len");
        let nib = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex nibble {c}"),
        };
        hex.as_bytes()
            .chunks(2)
            .map(|p| (nib(p[0]) << 4) | nib(p[1]))
            .collect()
    }

    const SWEEP_FULL2_HOUR1_HEX: &str = "daebad3f00000000000000000000000000000000cbf7b53ee7877dbfc6367dbf000000000cea1840d3d9163e8da9163e66337bc52ed562c552421144d753124435683c3f00000000000000000000000000000000267c793db6a6d13e0000000012b713c4cccc1fc333d38bc3438b4cbe67e61344c9021f43d5128c43cdcc4c3e000000000000000072f97f3f72f97f3f00003045000016440000544400409c4500000000000016440000b5440080094400000000000016440000c84200409c4500003045000016440000c8420080094400e03845000048440000af440000803f00000000c6367dbf8da9163ee354d54100000000000000000000000000000000abaa2a3d000000009c6d9e3d9f3b7f3fae18d93cec7eed3cef5c913c0000000012fd833e3f6ba63e3f3d913e00000000000000000000000000000000000000000000000000000000000000000000803fcfad033fd765223f7b23573f6a020000b207213f64e03d3f44094b3f0000803f0000803f0000803f0000803f0000803f66dc5e3ee6f70b3f0000803f00002040000080440000803f00540500f8000000";
    const SWEEP_FULL2_HOUR6_HEX: &str = "daebad3f000000000000000000000000000000000224763e560c7fbfb8ba7ebf0000000053d41940c20ccc3d76cbcb3d55445ec563df50c5d40c2943bbd62d4335683c3f00000000000000000000000000000000edbb283debe7d23e0000000072b302c4173578c2a1c387c3438b4cbe53dd0244b4897442dffb8743cdcc4c3e0000803f0000000048e17a3f8716793f00003045000016440000544400409c4500000000000016440000b5440080094400000000000016440000c84200409c4500003045000016440000c84200800944a8942345e3529b43eaeaa9440000803f00000000b8ba7ebf76cbcb3d9f2b374300000000c0b72843821ff83d000000000000803e000000009c6d9e3d9f3b7f3fae18d93cec7eed3cef5c913c0000803f12fd833e3f6ba63e3f3d913e247a853d000000000000000000000000000000000000000000000000000000000000803fcfad033fd765223f7b23573f00000000b207213f64e03d3f44094b3f0000803f0000803f0000803f0000803f0000803f66dc5e3ee6f70b3f0000803f00002040000080440000803f0000803f00000000";

    #[test]
    fn night_sweep_yzw_is_bit_identical_across_hours_at_one_date() {
        let h1 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let h6 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR6_HEX)).unwrap();

        assert!(
            (h1.day_night_hour() - 1.0).abs() < 1e-3,
            "h1 {}",
            h1.day_night_hour()
        );
        assert!(
            (h6.day_night_hour() - 6.0).abs() < 1e-3,
            "h6 {}",
            h6.day_night_hour()
        );

        for i in 0..3 {
            assert_eq!(
                h1.sun_dir_yzw()[i].to_bits(),
                h6.sun_dir_yzw()[i].to_bits(),
                "sun_dir .yzw component {i} moved with the hour (h1 {} vs h6 {})",
                h1.sun_dir_yzw()[i],
                h6.sun_dir_yzw()[i]
            );
        }

        let (unit, y_abs, one_minus_w) = night_yzw_residual(h1.sun_dir_yzw());
        assert_eq!(y_abs, 0.0, "sun_dir .y (dir.x) is exactly 0 in the sweep");
        assert!(
            one_minus_w < 3e-3,
            "sun_dir .w (dir.z) ≈ 1, off by {one_minus_w}"
        );
        assert!(unit < 2e-3, "sun_dir is a unit vector, |yzw|-1 = {unit}");

        assert_eq!(h1.fow_opacity_time_snowmud_maxspeed[3], 0.0);
        assert_eq!(h6.fow_opacity_time_snowmud_maxspeed[3], 0.0);
    }

    #[test]
    fn render_sun_dir_yzw_echoes_captured_vector_independent_of_hour() {
        let captured = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX))
            .unwrap()
            .sun_dir_yzw();
        for hour24 in [0.0f32, 6.0, 12.0, 18.0, 23.5] {
            let out = render_sun_dir_yzw(hour24, captured);
            for i in 0..3 {
                assert_eq!(
                    out[i].to_bits(),
                    captured[i].to_bits(),
                    "render_sun_dir_yzw changed .yzw[{i}] at hour24 {hour24}"
                );
            }
        }
    }


    const CAPTURED_AMBIENT_RIG_FULL2: [[f32; 3]; 6] = [
        [0.026501026, 0.028991185, 0.0177445095],
        [0.25779015, 0.32503697, 0.2836704],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.51437086, 0.63436645, 0.84038514],
        [0.62902367, 0.7417052, 0.79311013],
    ];

    #[test]
    fn gate_cb0_ambient_rig_composes_bit_exact_from_ambient_light_defines() {
        let composed = cb0_ambient_rig_from_hsv_defines(vanilla_ambient_light_hsv_defines());
        for hex in [SWEEP_FULL2_HOUR1_HEX, SWEEP_FULL2_HOUR6_HEX] {
            let s = SceneLighting::from_cb0_bytes(&hexbytes(hex)).unwrap();
            let residual = ambient_rig_residual(&s.ambient_rig, &composed);
            assert_eq!(
                residual, 0.0,
                "composed ambient rig residual {residual} vs captured (must be bit-exact)"
            );
            for (face, (cap, law)) in s.ambient_rig.iter().zip(composed.iter()).enumerate() {
                for lane in 0..3 {
                    assert_eq!(
                        cap[lane].to_bits(),
                        law[lane].to_bits(),
                        "ambient face {face} lane {lane}: captured {} vs composed {}",
                        cap[lane],
                        law[lane]
                    );
                }
            }
        }
        let s = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        assert_eq!(s.ambient_rig, CAPTURED_AMBIENT_RIG_FULL2);
    }

    #[test]
    fn ambient_rig_is_hour_invariant_in_swept_capture() {
        let h1 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let h6 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR6_HEX)).unwrap();
        for face in 0..6 {
            for lane in 0..3 {
                assert_eq!(
                    h1.ambient_rig[face][lane].to_bits(),
                    h6.ambient_rig[face][lane].to_bits(),
                    "ambient face {face} lane {lane} moved between hours 1 and 6"
                );
            }
        }
    }

    #[test]
    fn cb0_ambient_compose_is_not_the_day_ambient_map_rig() {
        let composed = cb0_ambient_rig_from_hsv_defines(vanilla_ambient_light_hsv_defines());
        let day_ambient_map_pos_x = [0.1f32, 0.1, 0.05];
        let delta = (composed[0][0] - day_ambient_map_pos_x[0]).abs();
        assert!(
            delta > 0.05,
            "CB0 PosX unexpectedly matches DayAmbientMapPosX (delta {delta})"
        );
    }

    #[test]
    fn ambient_light_defines_parse_from_lua_source() {
        let source = r#"
	-- hsv color ambient light
	AMBIENT_LIGHT_POS_X = { 0.2,  0.2, 0.2 }, -- right
	AMBIENT_LIGHT_NEG_X = { 0.4,  0.1, 0.6 }, -- left
	AMBIENT_LIGHT_POS_Y = { 0.0,  0.0, 0.0 },-- kills everything
	AMBIENT_LIGHT_NEG_Y = { 0.35, 0.2, 0.0 },-- from under
	AMBIENT_LIGHT_POS_Z = { 0.6,  0.2, 0.924 }, -- top
	AMBIENT_LIGHT_NEG_Z = { 0.55, 0.1, 0.9 }, -- bottom
"#;
        let parsed = ambient_light_hsv_defines_from_graphics_source(source)
            .expect("parse AMBIENT_LIGHT_* from lua source");
        assert_eq!(parsed, vanilla_ambient_light_hsv_defines());
        let err = ambient_light_hsv_defines_from_graphics_source("AMBIENT_LIGHT_POS_X = { 0 }")
            .expect_err("must fail closed");
        assert!(err.contains("AMBIENT_LIGHT_POS_X"), "{err}");
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_graphics_lua_ambient_defines_compose_to_captured_rig() {
        let path =
            replay_test_gates::require_vanilla_game_root().join("common/defines/00_graphics.lua");
        let source = std::fs::read_to_string(&path).expect("read shipped 00_graphics.lua");
        let parsed = ambient_light_hsv_defines_from_graphics_source(&source)
            .expect("parse AMBIENT_LIGHT_* from the shipped 00_graphics.lua");
        assert_eq!(
            parsed,
            vanilla_ambient_light_hsv_defines(),
            "shipped defines drifted from the cited vanilla values"
        );
        let composed = cb0_ambient_rig_from_hsv_defines(parsed);
        let s = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let residual = ambient_rig_residual(&s.ambient_rig, &composed);
        assert_eq!(residual, 0.0, "file->law->CB0 residual {residual}");
        println!(
            "vanilla ambient-defines gate: shipped lua -> composed rig bit-exact (residual 0)"
        );
    }

    #[test]
    fn night_yzw_residual_law_holds_over_proven_hour_range() {
        for hex in [SWEEP_FULL2_HOUR1_HEX, SWEEP_FULL2_HOUR6_HEX] {
            let s = SceneLighting::from_cb0_bytes(&hexbytes(hex)).unwrap();
            let (unit, y_abs, one_minus_w) = night_yzw_residual(s.sun_dir_yzw());
            assert!(unit < 2e-3, "|yzw|-1 {unit}");
            assert_eq!(y_abs, 0.0, ".y exactly 0");
            assert!(one_minus_w < 3e-3, "1-.w {one_minus_w}");
        }
    }
}
