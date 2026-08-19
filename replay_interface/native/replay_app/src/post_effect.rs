
pub(crate) const LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];

const COLOR_CUBE_SIZE: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorCube32 {
    texels: Vec<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RestoreSceneFullParams {
    pub hsv: [f32; 4],
    pub color_balance: [f32; 3],
    pub middle_grey: f32,
    pub average_luminance: Option<f32>,
    pub apply_exposure: bool,
    pub apply_color_lut: bool,
}

impl ColorCube32 {
    pub(crate) fn new(texels: Vec<[f32; 3]>) -> Result<Self, String> {
        let expected = COLOR_CUBE_SIZE * COLOR_CUBE_SIZE * COLOR_CUBE_SIZE;
        if texels.len() != expected {
            return Err(format!(
                "ColorCube32 requires {expected} texels, got {}",
                texels.len()
            ));
        }
        if texels.iter().flatten().any(|value| !value.is_finite()) {
            return Err("ColorCube32 contains a non-finite texel".to_string());
        }
        Ok(Self { texels })
    }

    pub(crate) fn identity() -> Self {
        Self::from_fn(|r, g, b| [r as f32 / 31.0, g as f32 / 31.0, b as f32 / 31.0])
    }

    pub(crate) fn from_fn(mut f: impl FnMut(usize, usize, usize) -> [f32; 3]) -> Self {
        let mut texels = Vec::with_capacity(COLOR_CUBE_SIZE * COLOR_CUBE_SIZE * COLOR_CUBE_SIZE);
        for b in 0..COLOR_CUBE_SIZE {
            for g in 0..COLOR_CUBE_SIZE {
                for r in 0..COLOR_CUBE_SIZE {
                    texels.push(f(r, g, b));
                }
            }
        }
        Self { texels }
    }

    fn texel(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.texels[(b * COLOR_CUBE_SIZE + g) * COLOR_CUBE_SIZE + r]
    }

    fn strip_texel(&self, x: usize, y: usize) -> [f32; 3] {
        let b = x / COLOR_CUBE_SIZE;
        let r = x % COLOR_CUBE_SIZE;
        self.texel(r, y, b)
    }
}

fn uncharted2_tonemap_channel(x: f32) -> f32 {
    let a = 0.22;
    let b = 0.30;
    let c = 0.10;
    let d = 0.20;
    let e = 0.01;
    let f = 0.30;

    ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f
}

pub(crate) fn hoi4_scene_restore_tone_map_rgb(rgb: [f32; 3]) -> [f32; 3] {
    let exposure_bias = 1.0;
    let white_point = 11.2;
    let white_scale = 1.0 / uncharted2_tonemap_channel(white_point);
    rgb.map(|channel| {
        let curr = uncharted2_tonemap_channel(exposure_bias * channel);
        (curr * white_scale)
            .max(0.0)
            .powf(1.0 / 2.2)
            .clamp(0.0, 1.0)
    })
}

pub(crate) fn sample_color_cube(lut: &ColorCube32, rgb: [f32; 3]) -> [f32; 3] {
    let color = saturate_rgb(rgb);
    let cube_size = COLOR_CUBE_SIZE as f32;
    let scale = (cube_size - 1.0) / cube_size;
    let offset = 0.5 / cube_size;

    let x = ((scale * color[0] + offset) / cube_size).clamp(0.0, 1.0);
    let y = (scale * color[1] + offset).clamp(0.0, 1.0);
    let z_floor = ((scale * color[2] + offset) * cube_size).floor();
    let x_offset1 = z_floor / cube_size;
    let x_offset2 = ((cube_size - 1.0).min(z_floor + 1.0)) / cube_size;

    let color1 = sample_color_cube_strip(lut, x + x_offset1, y);
    let color2 = sample_color_cube_strip(lut, x + x_offset2, y);
    lerp3(color1, color2, scale * color[2] * cube_size - z_floor)
}

pub(crate) fn apply_hsv_shift(rgb: [f32; 3], hsv: [f32; 4]) -> [f32; 3] {
    let mut shifted = rgb_to_hsv(rgb);
    shifted[1] *= hsv[1];
    shifted[2] *= hsv[2];
    shifted[0] = shader_mod(shifted[0] + hsv[0], 6.0);
    hsv_to_rgb_post(shifted)
}

pub(crate) fn apply_color_balance(rgb: [f32; 3], color_balance: [f32; 3]) -> [f32; 3] {
    saturate_rgb([
        rgb[0] * color_balance[0],
        rgb[1] * color_balance[1],
        rgb[2] * color_balance[2],
    ])
}

pub(crate) fn apply_levels(rgb: [f32; 3], min_input: f32, max_input: f32) -> [f32; 3] {
    let denom = max_input - min_input;
    rgb.map(|channel| ((channel - min_input).clamp(0.0, 1.0) / denom).clamp(0.0, 1.0))
}

pub(crate) fn exposure(rgb: [f32; 3], middle_grey: f32, average_luminance: f32) -> [f32; 3] {
    let scale = middle_grey / average_luminance;
    rgb.map(|channel| channel * scale)
}

pub(crate) fn restore_scene_full_rgb(
    rgb: [f32; 3],
    lut: &ColorCube32,
    params: RestoreSceneFullParams,
) -> [f32; 3] {
    let mut color = rgb;
    if params.apply_exposure {
        if let Some(average_luminance) = params.average_luminance {
            color = exposure(color, params.middle_grey, average_luminance);
        }
    }
    color = hoi4_scene_restore_tone_map_rgb(color);
    if params.apply_color_lut {
        color = sample_color_cube(lut, saturate_rgb(color));
        color = apply_hsv_shift(color, params.hsv);
        color = apply_color_balance(color, params.color_balance);
        color = apply_levels(color, 0.04, 0.8);
    }
    color
}

#[cfg(test)]
pub(crate) fn restore_scene_full_with_bloom_rgb(
    scene_rgb: [f32; 3],
    bloom_rgb: [f32; 3],
    lut: &ColorCube32,
    params: RestoreSceneFullParams,
) -> [f32; 3] {
    let composited = [
        scene_rgb[0] + bloom_rgb[0],
        scene_rgb[1] + bloom_rgb[1],
        scene_rgb[2] + bloom_rgb[2],
    ];
    restore_scene_full_rgb(composited, lut, params)
}

pub(crate) fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * LUMINANCE_VECTOR[0] + rgb[1] * LUMINANCE_VECTOR[1] + rgb[2] * LUMINANCE_VECTOR[2]
}

fn sample_color_cube_strip(lut: &ColorCube32, u: f32, v: f32) -> [f32; 3] {
    let width = (COLOR_CUBE_SIZE * COLOR_CUBE_SIZE) as f32;
    let height = COLOR_CUBE_SIZE as f32;
    let x = (u * width - 0.5).clamp(0.0, width - 1.0);
    let y = (v * height - 0.5).clamp(0.0, height - 1.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(COLOR_CUBE_SIZE * COLOR_CUBE_SIZE - 1);
    let y1 = (y0 + 1).min(COLOR_CUBE_SIZE - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let c00 = lut.strip_texel(x0, y0);
    let c10 = lut.strip_texel(x1, y0);
    let c01 = lut.strip_texel(x0, y1);
    let c11 = lut.strip_texel(x1, y1);
    lerp3(lerp3(c00, c10, tx), lerp3(c01, c11, tx), ty)
}

fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let cmax = rgb[0].max(rgb[1].max(rgb[2]));
    let cmin = rgb[0].min(rgb[1].min(rgb[2]));
    let diff = cmax - cmin;

    let mut h = 0.0;
    let mut s = 0.0;
    if diff != 0.0 {
        s = diff / cmax;
        if cmax == rgb[0] {
            h = (rgb[1] - rgb[2]) / diff + 6.0;
        } else if cmax == rgb[1] {
            h = (rgb[2] - rgb[0]) / diff + 2.0;
        } else {
            h = (rgb[0] - rgb[1]) / diff + 4.0;
        }
    }
    [h, s, cmax]
}

fn hue_post(h: f32) -> [f32; 3] {
    let x = 1.0 - (shader_mod(h, 2.0) - 1.0).abs();
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

fn hsv_to_rgb_post(hsv: [f32; 3]) -> [f32; 3] {
    if hsv[1] != 0.0 {
        let c = hsv[1] * hsv[2];
        let hue = hue_post(hsv[0]);
        return [
            (hue[0] * c + (hsv[2] - c)).clamp(0.0, 1.0),
            (hue[1] * c + (hsv[2] - c)).clamp(0.0, 1.0),
            (hue[2] * c + (hsv[2] - c)).clamp(0.0, 1.0),
        ];
    }
    saturate_rgb([hsv[2], hsv[2], hsv[2]])
}

fn shader_mod(value: f32, modulus: f32) -> f32 {
    value - modulus * (value / modulus).floor()
}

fn saturate_rgb(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|channel| channel.clamp(0.0, 1.0))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_map_matches_shader_mid_gray_sample() {
        let out = hoi4_scene_restore_tone_map_rgb([0.23, 0.23, 0.23]);
        for channel in out {
            assert!((channel - 0.395515).abs() < 0.00001, "{channel}");
        }
    }

    #[test]
    fn tone_map_brightens_hdr_midtones_without_clipping() {
        let input = [0.23, 0.23, 0.23];
        let output = hoi4_scene_restore_tone_map_rgb(input);
        assert!(luminance(output) > luminance(input));
        assert!(output.iter().all(|channel| *channel < 1.0));
    }

    #[test]
    fn identity_color_cube_is_passthrough() {
        let lut = ColorCube32::new(ColorCube32::identity().texels).expect("identity cube is valid");
        for rgb in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.5, 0.25, 0.75]] {
            let out = sample_color_cube(&lut, rgb);
            for channel in 0..3 {
                assert!((out[channel] - rgb[channel]).abs() < 0.00001, "{out:?}");
            }
        }
    }

    #[test]
    fn color_cube_sampling_matches_shader_strip_coordinates() {
        let lut = ColorCube32::from_fn(|r, g, b| [r as f32, g as f32, b as f32]);
        assert_eq!(sample_color_cube(&lut, [0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(sample_color_cube(&lut, [1.0, 1.0, 1.0]), [31.0, 31.0, 31.0]);

        let out = sample_color_cube(&lut, [0.5, 0.5, 0.5]);
        assert!((out[0] - 15.5).abs() < 0.00001, "{out:?}");
        assert!((out[1] - 15.5).abs() < 0.00001, "{out:?}");
        assert!((out[2] - 15.5).abs() < 0.00001, "{out:?}");
    }

    #[test]
    fn hsv_identity_is_noop() {
        let rgb = [0.2, 0.4, 0.8];
        let out = apply_hsv_shift(rgb, [0.0, 1.0, 1.0, 0.0]);
        for channel in 0..3 {
            assert!((out[channel] - rgb[channel]).abs() < 0.00001, "{out:?}");
        }
    }

    #[test]
    fn levels_match_shader_endpoints() {
        let out = apply_levels([0.04, 0.8, 0.42], 0.04, 0.8);
        assert!((out[0] - 0.0).abs() < 0.00001, "{out:?}");
        assert!((out[1] - 1.0).abs() < 0.00001, "{out:?}");
        assert!((out[2] - 0.5).abs() < 0.00001, "{out:?}");
    }

    #[test]
    fn exposure_uses_middle_grey_over_average_luminance() {
        let out = exposure([2.0, 4.0, 8.0], 0.5, 2.0);
        assert_eq!(out, [0.5, 1.0, 2.0]);
    }

    #[test]
    fn full_chain_with_identity_evidence_is_plain_tonemap() {
        let rgb = [0.23, 0.31, 0.4];
        let lut = ColorCube32::identity();
        let params = RestoreSceneFullParams {
            hsv: [0.0, 1.0, 1.0, 0.0],
            color_balance: [1.0, 1.0, 1.0],
            middle_grey: 1.0,
            average_luminance: None,
            apply_exposure: false,
            apply_color_lut: false,
        };
        let out = restore_scene_full_rgb(rgb, &lut, params);
        let expected = hoi4_scene_restore_tone_map_rgb(rgb);
        for channel in 0..3 {
            assert!(
                (out[channel] - expected[channel]).abs() < 0.00001,
                "{out:?}"
            );
        }
    }

    fn no_grade_params() -> RestoreSceneFullParams {
        RestoreSceneFullParams {
            hsv: [0.0, 1.0, 1.0, 0.0],
            color_balance: [1.0, 1.0, 1.0],
            middle_grey: 1.0,
            average_luminance: None,
            apply_exposure: false,
            apply_color_lut: false,
        }
    }

    #[test]
    fn bloom_composites_additively_before_tonemap_hand_computed() {
        let scene = [0.20f32, 0.30, 0.40];
        let bloom = [0.05f32, 0.10, 0.15];
        let lut = ColorCube32::identity();
        let out = restore_scene_full_with_bloom_rgb(scene, bloom, &lut, no_grade_params());
        let hand = hoi4_scene_restore_tone_map_rgb([
            scene[0] + bloom[0],
            scene[1] + bloom[1],
            scene[2] + bloom[2],
        ]);
        for channel in 0..3 {
            assert!(
                (out[channel] - hand[channel]).abs() < 1e-6,
                "channel {channel}: {out:?} vs {hand:?}"
            );
        }
    }

    #[test]
    fn adding_bloom_before_tonemap_differs_from_adding_after() {
        let scene = [0.40f32, 0.50, 0.60];
        let bloom = [0.20f32, 0.20, 0.20];
        let lut = ColorCube32::identity();
        let correct = restore_scene_full_with_bloom_rgb(scene, bloom, &lut, no_grade_params());
        let wrong_post = {
            let toned = hoi4_scene_restore_tone_map_rgb(scene);
            [
                toned[0] + bloom[0],
                toned[1] + bloom[1],
                toned[2] + bloom[2],
            ]
        };
        let mut any_channel_differs = false;
        for channel in 0..3 {
            if (correct[channel] - wrong_post[channel]).abs() > 1e-3 {
                any_channel_differs = true;
            }
        }
        assert!(
            any_channel_differs,
            "pre-tonemap and post-tonemap bloom must differ (non-linear tone map): {correct:?} vs {wrong_post:?}"
        );
    }

    #[test]
    fn zero_bloom_is_identical_to_no_bloom_path() {
        let scene = [0.23f32, 0.31, 0.40];
        let lut = ColorCube32::identity();
        let params = RestoreSceneFullParams {
            hsv: [0.0, 1.0, 1.0, 0.0],
            color_balance: [1.0, 1.0, 1.0],
            middle_grey: 0.55,
            average_luminance: Some(0.5),
            apply_exposure: true,
            apply_color_lut: true,
        };
        let with_zero_bloom =
            restore_scene_full_with_bloom_rgb(scene, [0.0, 0.0, 0.0], &lut, params);
        let no_bloom = restore_scene_full_rgb(scene, &lut, params);
        assert_eq!(with_zero_bloom, no_bloom);
    }
}
