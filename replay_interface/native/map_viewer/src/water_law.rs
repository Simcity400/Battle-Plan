
pub mod defines {
    pub const SUN_HEIGHT: f32 = 600.0;
    pub const MOON_HEIGHT: f32 = 600.0;
    pub const SUN_HEIGHT_WATER: f32 = 5000.0;
    pub const MOON_HEIGHT_WATER: f32 = 550.0;
    pub const SUN_LATITUDE: f32 = 848.0;
    pub const SECOND_SUN_LATITUDE: f32 = 100.0;
    pub const SECOND_MOON_LATITUDE: f32 = 100.0;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualLightPacked {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl VirtualLightPacked {
    pub fn water_source_position(self) -> [f32; 3] {
        [self.x, self.w, self.z]
    }

    pub fn terrain_source_position(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn from_cb0_bytes(cb0: &[u8], byte_offset: usize) -> Option<Self> {
        let end = byte_offset.checked_add(16)?;
        let slice = cb0.get(byte_offset..end)?;
        let f = |i: usize| f32::from_le_bytes([slice[i], slice[i + 1], slice[i + 2], slice[i + 3]]);
        Some(Self {
            x: f(0),
            y: f(4),
            z: f(8),
            w: f(12),
        })
    }
}

pub const CB0_OFFSET_VIRTUAL_SUN_POS: usize = 144;
pub const CB0_OFFSET_VIRTUAL_MOON_POS: usize = 160;
pub const CB0_OFFSET_SECOND_VIRTUAL_SUN_POS: usize = 176;
pub const CB0_OFFSET_SECOND_VIRTUAL_MOON_POS: usize = 192;

#[derive(Clone, Debug, PartialEq)]
pub struct PackingLawResidual {
    pub label: &'static str,
    pub terrain_height_residual: f32,
    pub water_height_residual: f32,
    pub latitude_residual: Option<f32>,
}

pub fn validate_packing_law_against_cb0(cb0: &[u8]) -> Option<Vec<PackingLawResidual>> {
    let sun = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_VIRTUAL_SUN_POS)?;
    let moon = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_VIRTUAL_MOON_POS)?;
    let second_sun = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_SECOND_VIRTUAL_SUN_POS)?;
    let second_moon = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_SECOND_VIRTUAL_MOON_POS)?;

    Some(vec![
        PackingLawResidual {
            label: "vVirtualSunPos",
            terrain_height_residual: sun.y - defines::SUN_HEIGHT,
            water_height_residual: sun.w - defines::SUN_HEIGHT_WATER,
            latitude_residual: Some(sun.z - defines::SUN_LATITUDE),
        },
        PackingLawResidual {
            label: "vVirtualMoonPos",
            terrain_height_residual: moon.y - defines::MOON_HEIGHT,
            water_height_residual: moon.w - defines::MOON_HEIGHT_WATER,
            latitude_residual: None,
        },
        PackingLawResidual {
            label: "vSecondVirtualSunPos",
            terrain_height_residual: second_sun.y - defines::SUN_HEIGHT,
            water_height_residual: second_sun.w - defines::SUN_HEIGHT_WATER,
            latitude_residual: Some(second_sun.z - defines::SECOND_SUN_LATITUDE),
        },
        PackingLawResidual {
            label: "vSecondVirtualMoonPos",
            terrain_height_residual: second_moon.y - defines::MOON_HEIGHT,
            water_height_residual: second_moon.w - defines::MOON_HEIGHT_WATER,
            latitude_residual: Some(second_moon.z - defines::SECOND_MOON_LATITUDE),
        },
    ])
}


pub mod underwater_defines {
    pub const WATER_HEIGHT: f32 = 9.5;
    pub const TERRAIN_WATER_CLIP_HEIGHT: f32 = 3.0;
    pub const WATER_COLOR_LIGHTNESS: f32 = 0.5;
    pub const MAP_SPECULAR_WIDTH: f32 = 15.0;
    pub const SHADOW_WEIGHT_TERRAIN: f32 = 0.7;
    pub const DEPTH_FOG_MIN_Y: f32 = 5.3;
    pub const DEPTH_FOG_MAX_Y: f32 = 13.0;
}

pub fn underwater_clip_height_threshold() -> f32 {
    underwater_defines::WATER_HEIGHT + underwater_defines::TERRAIN_WATER_CLIP_HEIGHT
}

pub fn underwater_depth_fog_alpha(prepos_y: f32) -> f32 {
    let span = underwater_defines::DEPTH_FOG_MAX_Y - underwater_defines::DEPTH_FOG_MIN_Y;
    ((prepos_y - underwater_defines::DEPTH_FOG_MIN_Y) / span).clamp(0.0, 1.0)
}

pub const UNDERWATER_LIGHT_ABSORPTION: [f32; 3] = [0.133, 0.234, 0.570];

pub fn apply_underwater_light_absorption(cb0: &mut [u8]) -> crate::Result<()> {
    use crate::light_law::offsets;
    if cb0.len() < offsets::CB0_LEN {
        return Err(format!(
            "--water-full underwater pass blocked: scene CB0 has {} bytes, need {} to apply \
             the underwater light absorption",
            cb0.len(),
            offsets::CB0_LEN
        )
        .into());
    }
    let direct_light_offsets = [
        offsets::SUN_DIFFUSE_INTENSITY,
        offsets::MOON_DIFFUSE_INTENSITY,
    ];
    for base in direct_light_offsets {
        for (channel, absorption) in UNDERWATER_LIGHT_ABSORPTION.iter().enumerate() {
            let at = base + channel * 4;
            let raw = f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]]);
            let absorbed = raw * absorption;
            cb0[at..at + 4].copy_from_slice(&absorbed.to_le_bytes());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderwaterLightDelta {
    pub sun_rgb_ratio: [Option<f32>; 3],
    pub moon_rgb_ratio: [Option<f32>; 3],
    pub underwater_sun_rgb: [f32; 3],
    pub scene_sun_rgb: [f32; 3],
    pub underwater_moon_rgb: [f32; 3],
    pub scene_moon_rgb: [f32; 3],
    pub underwater_day_gate: f32,
}

impl UnderwaterLightDelta {
    pub fn is_unabsorbed(&self, tol: f32) -> bool {
        self.sun_rgb_ratio
            .iter()
            .chain(self.moon_rgb_ratio.iter())
            .all(|r| match r {
                Some(v) => (v - 1.0).abs() <= tol,
                None => true,
            })
    }
}

pub fn underwater_cb0_absorption_ratio(
    underwater_cb0: &[u8],
    scene_cb0: &[u8],
) -> Option<UnderwaterLightDelta> {
    use crate::light_law::offsets;
    if underwater_cb0.len() < offsets::CB0_LEN || scene_cb0.len() < offsets::CB0_LEN {
        return None;
    }
    let rgb = |cb0: &[u8], base: usize| -> [f32; 3] {
        let f = |i: usize| {
            let at = base + i * 4;
            f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]])
        };
        [f(0), f(1), f(2)]
    };
    let ratio = |num: [f32; 3], den: [f32; 3]| -> [Option<f32>; 3] {
        [
            if den[0] != 0.0 {
                Some(num[0] / den[0])
            } else {
                None
            },
            if den[1] != 0.0 {
                Some(num[1] / den[1])
            } else {
                None
            },
            if den[2] != 0.0 {
                Some(num[2] / den[2])
            } else {
                None
            },
        ]
    };
    let uw_sun = rgb(underwater_cb0, offsets::SUN_DIFFUSE_INTENSITY);
    let sc_sun = rgb(scene_cb0, offsets::SUN_DIFFUSE_INTENSITY);
    let uw_moon = rgb(underwater_cb0, offsets::MOON_DIFFUSE_INTENSITY);
    let sc_moon = rgb(scene_cb0, offsets::MOON_DIFFUSE_INTENSITY);
    let day_gate = {
        let at = offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 12;
        f32::from_le_bytes([
            underwater_cb0[at],
            underwater_cb0[at + 1],
            underwater_cb0[at + 2],
            underwater_cb0[at + 3],
        ])
    };
    Some(UnderwaterLightDelta {
        sun_rgb_ratio: ratio(uw_sun, sc_sun),
        moon_rgb_ratio: ratio(uw_moon, sc_moon),
        underwater_sun_rgb: uw_sun,
        scene_sun_rgb: sc_sun,
        underwater_moon_rgb: uw_moon,
        scene_moon_rgb: sc_moon,
        underwater_day_gate: day_gate,
    })
}

pub const UNDERWATER_PS_REQUIRED_SRV_SLOTS: &[(&str, u32)] = &[
    ("TerrainDiffuse", 0),
    ("HeightNormal", 1),
    ("TerrainColorTint", 2),
    ("LightIndexMap", 10),
    ("LightDataMap", 11),
    ("ShadowMap", 12),
];


pub fn lean_flattened_det(b: [f32; 2], m: [f32; 3], spec_map_a: f32) -> f32 {
    let vf = spec_map_a;
    let bx = b[0] * vf;
    let by = b[1] * vf;
    let mx = m[0] * vf * vf;
    let my = m[1] * vf * vf;
    let mz = m[2] * vf * vf;
    let sx = mx - bx * bx;
    let sy = my - by * by;
    let sz = mz - bx * by;
    sx * sy - sz * sz
}

pub fn lean_specular_peak(det: f32) -> f32 {
    if det <= 0.0 {
        0.0
    } else {
        1.0 / det.sqrt()
    }
}

pub fn lean_specular_peak_glossiness(det: f32, spec_map_a: f32) -> f32 {
    lean_specular_peak(det) / 9.0 * (1.0 - spec_map_a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured_cb0_from_restore_gate_1_and_live_pan2() -> Vec<u8> {
        let mut cb0 = vec![0u8; 272];
        let mut put = |off: usize, v: [f32; 4]| {
            for (i, comp) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&comp.to_le_bytes());
            }
        };
        put(CB0_OFFSET_VIRTUAL_SUN_POS, [2816.0, 600.0, 848.0, 5000.0]);
        put(CB0_OFFSET_VIRTUAL_MOON_POS, [0.0, 600.0, 1448.0, 550.0]);
        put(
            CB0_OFFSET_SECOND_VIRTUAL_SUN_POS,
            [0.0, 600.0, 100.0, 5000.0],
        );
        put(
            CB0_OFFSET_SECOND_VIRTUAL_MOON_POS,
            [2816.0, 600.0, 100.0, 550.0],
        );
        cb0
    }

    #[test]
    fn water_swizzle_takes_w_as_height() {
        let sun = VirtualLightPacked {
            x: 2816.0,
            y: defines::SUN_HEIGHT,
            z: defines::SUN_LATITUDE,
            w: defines::SUN_HEIGHT_WATER,
        };
        assert_eq!(sun.water_source_position(), [2816.0, 5000.0, 848.0]);
        assert_eq!(sun.terrain_source_position(), [2816.0, 600.0, 848.0]);
    }

    #[test]
    fn packing_law_reproduces_captured_cb0_vectors() {
        let cb0 = captured_cb0_from_restore_gate_1_and_live_pan2();
        let residuals =
            validate_packing_law_against_cb0(&cb0).expect("CB0 long enough for all four lights");
        assert_eq!(residuals.len(), 4);
        for r in &residuals {
            assert!(
                r.terrain_height_residual.abs() < 1e-3,
                "{}: terrain height residual {} exceeds tolerance",
                r.label,
                r.terrain_height_residual
            );
            assert!(
                r.water_height_residual.abs() < 1e-3,
                "{}: water height (.w) residual {} exceeds tolerance",
                r.label,
                r.water_height_residual
            );
            if let Some(lat) = r.latitude_residual {
                assert!(
                    lat.abs() < 1e-3,
                    "{}: latitude (.z) residual {} exceeds tolerance",
                    r.label,
                    lat
                );
            }
        }
        let sun = VirtualLightPacked::from_cb0_bytes(&cb0, CB0_OFFSET_VIRTUAL_SUN_POS).unwrap();
        assert_eq!(sun.water_source_position(), [2816.0, 5000.0, 848.0]);
    }

    #[test]
    fn cb0_reader_fails_closed_on_short_buffer() {
        let short = vec![0u8; CB0_OFFSET_VIRTUAL_MOON_POS + 8];
        assert!(VirtualLightPacked::from_cb0_bytes(&short, CB0_OFFSET_VIRTUAL_SUN_POS).is_some());
        assert!(
            VirtualLightPacked::from_cb0_bytes(&short, CB0_OFFSET_SECOND_VIRTUAL_MOON_POS)
                .is_none()
        );
        assert!(validate_packing_law_against_cb0(&short).is_none());
    }


    #[test]
    fn underwater_clip_threshold_from_shipped_defines() {
        assert_eq!(underwater_clip_height_threshold(), 12.5);
        assert_eq!(underwater_defines::WATER_HEIGHT, 9.5);
        assert_eq!(underwater_defines::TERRAIN_WATER_CLIP_HEIGHT, 3.0);
    }

    #[test]
    fn underwater_depth_fog_alpha_matches_shader_lerp() {
        assert_eq!(underwater_depth_fog_alpha(5.3), 0.0);
        assert_eq!(underwater_depth_fog_alpha(0.0), 0.0);
        assert_eq!(underwater_depth_fog_alpha(13.0), 1.0);
        assert_eq!(underwater_depth_fog_alpha(20.0), 1.0);
        let mid = underwater_depth_fog_alpha(9.15);
        assert!((mid - 0.5).abs() < 1e-4, "midpoint alpha {mid}");
        assert!(underwater_depth_fog_alpha(7.0) < underwater_depth_fog_alpha(10.0));
    }

    #[test]
    fn underwater_required_srv_slots_are_the_shader_sampled_set() {
        let slots: Vec<(&str, u32)> = UNDERWATER_PS_REQUIRED_SRV_SLOTS.to_vec();
        assert_eq!(
            slots,
            vec![
                ("TerrainDiffuse", 0),
                ("HeightNormal", 1),
                ("TerrainColorTint", 2),
                ("LightIndexMap", 10),
                ("LightDataMap", 11),
                ("ShadowMap", 12),
            ]
        );
    }


    #[test]
    fn underwater_light_absorption_fallback_is_blue_dominant_and_bounded() {
        let [r, g, b] = UNDERWATER_LIGHT_ABSORPTION;
        assert!(r > 0.0 && r < 1.0, "red absorption {r}");
        assert!(g > 0.0 && g < 1.0, "green absorption {g}");
        assert!(b > 0.0 && b < 1.0, "blue absorption {b}");
        assert!(
            r < g && g < b,
            "absorption must be blue-dominant: {r} {g} {b}"
        );
    }

    #[test]
    fn underwater_light_absorption_patches_direct_light_only() {
        use crate::light_law::offsets;
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        for chunk in cb0.chunks_exact_mut(4) {
            chunk.copy_from_slice(&1.0f32.to_le_bytes());
        }
        apply_underwater_light_absorption(&mut cb0).expect("CB0 long enough");
        let read = |at: usize| f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]]);
        for base in [
            offsets::SUN_DIFFUSE_INTENSITY,
            offsets::MOON_DIFFUSE_INTENSITY,
        ] {
            for (channel, absorption) in UNDERWATER_LIGHT_ABSORPTION.iter().enumerate() {
                let value = read(base + channel * 4);
                assert!(
                    (value - absorption).abs() < 1e-6,
                    "direct light color at {base}+{channel} must be absorbed: {value} vs {absorption}"
                );
            }
            let a = read(base + 12);
            assert_eq!(a, 1.0, "intensity .a at {base}+12 must be preserved");
        }
        for at in [
            offsets::AMBIENT_POS_X,
            offsets::AMBIENT_NEG_X,
            offsets::AMBIENT_POS_Y,
            offsets::AMBIENT_NEG_Y,
            offsets::AMBIENT_POS_Z,
            offsets::AMBIENT_NEG_Z,
            offsets::HDR_RANGE,
            offsets::CAM_POS,
            offsets::VIRTUAL_SUN_POS,
            offsets::DAY_NIGHT_HOUR_SUN_DIR,
            offsets::SUN_SPECULAR_INTENSITY,
            offsets::SHADOW_FADE_FACTOR,
        ] {
            assert_eq!(read(at), 1.0, "field at {at} must be preserved");
        }
    }

    #[test]
    fn underwater_light_absorption_fails_closed_on_short_cb0() {
        let mut short = vec![0u8; crate::light_law::offsets::CB0_LEN - 16];
        assert!(apply_underwater_light_absorption(&mut short).is_err());
    }


    fn cb0_with_light(sun: [f32; 4], moon: [f32; 4], day_gate: f32) -> Vec<u8> {
        use crate::light_law::offsets;
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        let put4 = |cb0: &mut [u8], off: usize, v: [f32; 4]| {
            for (i, c) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
            }
        };
        put4(&mut cb0, offsets::SUN_DIFFUSE_INTENSITY, sun);
        put4(&mut cb0, offsets::MOON_DIFFUSE_INTENSITY, moon);
        cb0[offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 12
            ..offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 16]
            .copy_from_slice(&day_gate.to_le_bytes());
        cb0
    }

    const CAPFIX2_UNDERWATER_SUN: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const CAPFIX2_UNDERWATER_MOON: [f32; 4] = [0.217_637_63, 0.546_751_38, 1.0, 2.5];
    const CAPFIX2_UNDERWATER_DAY_GATE: f32 = 0.0;

    #[test]
    fn engine_underwater_cb0_carries_unabsorbed_white_sun() {
        let underwater = cb0_with_light(
            CAPFIX2_UNDERWATER_SUN,
            CAPFIX2_UNDERWATER_MOON,
            CAPFIX2_UNDERWATER_DAY_GATE,
        );
        let scene = cb0_with_light(
            CAPFIX2_UNDERWATER_SUN,
            CAPFIX2_UNDERWATER_MOON,
            CAPFIX2_UNDERWATER_DAY_GATE,
        );
        let delta = underwater_cb0_absorption_ratio(&underwater, &scene)
            .expect("both CB0s are full length");

        assert_eq!(delta.underwater_sun_rgb, [1.0, 1.0, 1.0]);
        assert_eq!(delta.underwater_day_gate, 0.0);
        for r in delta
            .sun_rgb_ratio
            .iter()
            .chain(delta.moon_rgb_ratio.iter())
        {
            let v = r.expect("scene light channel is non-zero");
            assert!(
                (v - 1.0).abs() < 1e-6,
                "underwater/scene light ratio {v} must be 1.0 (no engine absorption)"
            );
        }
        assert!(
            delta.is_unabsorbed(1e-6),
            "engine underwater pass must carry an un-absorbed light rig"
        );
    }

    #[test]
    fn recovered_absorption_is_not_the_engine_cb0_ratio() {
        let scene = cb0_with_light([1.0, 1.0, 1.0, 1.0], CAPFIX2_UNDERWATER_MOON, 0.0);
        let mut hypothetical_absorbed = scene.clone();
        apply_underwater_light_absorption(&mut hypothetical_absorbed).unwrap();
        let hypo = underwater_cb0_absorption_ratio(&hypothetical_absorbed, &scene).unwrap();
        for (i, r) in hypo.sun_rgb_ratio.iter().enumerate() {
            let v = r.unwrap();
            assert!(
                (v - UNDERWATER_LIGHT_ABSORPTION[i]).abs() < 1e-6,
                "app-absorbed sun ratio {v} must equal the absorption {}",
                UNDERWATER_LIGHT_ABSORPTION[i]
            );
        }
        let real = cb0_with_light(CAPFIX2_UNDERWATER_SUN, CAPFIX2_UNDERWATER_MOON, 0.0);
        let real_delta = underwater_cb0_absorption_ratio(&real, &scene).unwrap();
        assert!(
            real_delta.is_unabsorbed(1e-6),
            "the captured engine underwater CB0 is un-absorbed, unlike the app's clone"
        );
        assert!(
            !hypo.is_unabsorbed(1e-6),
            "the app's absorbed clone is (correctly) detected as absorbed"
        );
    }

    #[test]
    fn underwater_cb0_absorption_ratio_fails_closed_on_short_cb0() {
        let short = vec![0u8; crate::light_law::offsets::CB0_LEN - 4];
        let full = vec![0u8; crate::light_law::offsets::CB0_LEN];
        assert!(underwater_cb0_absorption_ratio(&short, &full).is_none());
        assert!(underwater_cb0_absorption_ratio(&full, &short).is_none());
    }


    #[test]
    fn lean_specular_peak_is_unbounded_as_det_goes_to_zero() {
        let p_1 = lean_specular_peak(1.0);
        let p_1e2 = lean_specular_peak(1.0e-2);
        let p_1e6 = lean_specular_peak(1.0e-6);
        assert!((p_1 - 1.0).abs() < 1e-6);
        assert!((p_1e2 - 10.0).abs() < 1e-3, "peak at det=1e-2 was {p_1e2}");
        assert!(
            (p_1e6 - 1000.0).abs() < 1e-1,
            "peak at det=1e-6 was {p_1e6}"
        );
        assert!(
            p_1e6 > p_1e2 && p_1e2 > p_1,
            "peak must grow as det shrinks"
        );
        assert_eq!(lean_specular_peak(0.0), 0.0);
        assert_eq!(lean_specular_peak(-1.0), 0.0);
    }

    #[test]
    fn open_sea_specmap_collapses_glint_the_shader_implied_bound() {
        let huge_peak_det = 1.0e-8;
        let gloss_open_sea = lean_specular_peak_glossiness(huge_peak_det, 1.0);
        assert_eq!(gloss_open_sea, 0.0, "vSpecMap=1 must collapse glossiness");
        let gloss_near_open = lean_specular_peak_glossiness(huge_peak_det, 0.99);
        let gloss_calm = lean_specular_peak_glossiness(huge_peak_det, 0.0);
        assert!(
            gloss_near_open < gloss_calm * 0.02,
            "near-open (a=0.99) gloss {gloss_near_open} must be <<2% of a=0 gloss {gloss_calm}",
        );
    }

    #[test]
    fn specmap_flatten_shrinks_lean_covariance_toward_flat() {
        let b = [0.30_f32, -0.20];
        let m = [0.50_f32, 0.40, 0.05];
        let det_full = lean_flattened_det(b, m, 1.0);
        let det_flat = lean_flattened_det(b, m, 0.1);
        assert!(det_full > 0.0, "reference det must be positive: {det_full}");
        assert!(
            det_flat.abs() < det_full * 0.05,
            "flattened det {det_flat} must be <<5% of full det {det_full}",
        );
    }
}
