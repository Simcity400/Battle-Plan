//! Pure CPU reference for the game's full `RestoreScene` pixel law.
//!
//! This module deliberately models the production/full path only. Callers must
//! supply the composed 32^3 color-correction cube, bloom sample, average
//! luminance, and grade constants. There are no feature switches and no
//! identity-resource fallback.

use crate::{ColorCorrectionLut, Result};

/// `gfx/FX/constants.fxh:117-119`.
pub const RESTORE_SCENE_LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];

pub const RESTORE_SCENE_COLOR_CUBE_EDGE: usize = 32;
pub const RESTORE_SCENE_COLOR_CUBE_WIDTH: usize =
    RESTORE_SCENE_COLOR_CUBE_EDGE * RESTORE_SCENE_COLOR_CUBE_EDGE;

/// A checked borrowed view of the exact 1024x32 (32^3) RestoreScene LUT.
///
/// [`ColorCorrectionLut`] remains the owning, game-file-derived resource.
/// Constructing this view fails closed if a different cube shape or a
/// non-finite texel is supplied.
#[derive(Clone, Copy, Debug)]
pub struct RestoreSceneColorCube32<'a> {
    lut: &'a ColorCorrectionLut,
}

impl<'a> RestoreSceneColorCube32<'a> {
    pub fn new(lut: &'a ColorCorrectionLut) -> Result<Self> {
        let expected_texels = RESTORE_SCENE_COLOR_CUBE_WIDTH * RESTORE_SCENE_COLOR_CUBE_EDGE;
        if lut.width != RESTORE_SCENE_COLOR_CUBE_WIDTH
            || lut.height != RESTORE_SCENE_COLOR_CUBE_EDGE
            || lut.slice_size != RESTORE_SCENE_COLOR_CUBE_EDGE
            || lut.texels.len() != expected_texels
        {
            return Err(format!(
                "RestoreScene ColorCube must be {}x{} with slice_size {} and {} texels; got {}x{}, slice_size {}, {} texels",
                RESTORE_SCENE_COLOR_CUBE_WIDTH,
                RESTORE_SCENE_COLOR_CUBE_EDGE,
                RESTORE_SCENE_COLOR_CUBE_EDGE,
                expected_texels,
                lut.width,
                lut.height,
                lut.slice_size,
                lut.texels.len()
            )
            .into());
        }
        if lut.texels.iter().flatten().any(|value| !value.is_finite()) {
            return Err("RestoreScene ColorCube contains a non-finite texel".into());
        }
        Ok(Self { lut })
    }

    pub fn source(self) -> &'a ColorCorrectionLut {
        self.lut
    }

    fn texel(self, red: usize, green: usize, blue: usize) -> [f32; 3] {
        self.lut.texel(red, green, blue)
    }

    fn strip_texel(self, x: usize, y: usize) -> [f32; 3] {
        let blue = x / RESTORE_SCENE_COLOR_CUBE_EDGE;
        let red = x % RESTORE_SCENE_COLOR_CUBE_EDGE;
        self.texel(red, y, blue)
    }
}

impl<'a> TryFrom<&'a ColorCorrectionLut> for RestoreSceneColorCube32<'a> {
    type Error = Box<dyn std::error::Error>;

    fn try_from(lut: &'a ColorCorrectionLut) -> std::result::Result<Self, Self::Error> {
        Self::new(lut)
    }
}

/// Required `RestoreScene` grade inputs.
///
/// These values come from the admitted PostEffect constants and luminance
/// resource. Construction rejects absent/invalid values rather than turning
/// off exposure or grading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestoreSceneGrade {
    hsv: [f32; 4],
    color_balance: [f32; 3],
    middle_grey: f32,
    average_luminance: f32,
}

impl RestoreSceneGrade {
    pub fn new(
        hsv: [f32; 4],
        color_balance: [f32; 3],
        middle_grey: f32,
        average_luminance: f32,
    ) -> Result<Self> {
        if hsv.into_iter().any(|value| !value.is_finite()) {
            return Err("RestoreScene HSV contains a non-finite value".into());
        }
        if color_balance.into_iter().any(|value| !value.is_finite()) {
            return Err("RestoreScene color balance contains a non-finite value".into());
        }
        if !middle_grey.is_finite() || middle_grey <= 0.0 {
            return Err(format!(
                "RestoreScene middle grey must be finite and positive, got {middle_grey}"
            )
            .into());
        }
        if !average_luminance.is_finite() || average_luminance <= 0.0 {
            return Err(format!(
                "RestoreScene average luminance must be finite and positive, got {average_luminance}"
            )
            .into());
        }
        Ok(Self {
            hsv,
            color_balance,
            middle_grey,
            average_luminance,
        })
    }

    pub fn hsv(self) -> [f32; 4] {
        self.hsv
    }

    pub fn color_balance(self) -> [f32; 3] {
        self.color_balance
    }

    pub fn middle_grey(self) -> f32 {
        self.middle_grey
    }

    pub fn average_luminance(self) -> f32 {
        self.average_luminance
    }
}

/// Full `restorescene.shader` pixel order:
///
/// 1. add bloom to the linear MainScene sample;
/// 2. apply exposure using MiddleGrey / AverageLuminance;
/// 3. apply the Uncharted 2 tone curve;
/// 4. saturate, sample the 32^3 LUT, shift HSV, balance color, and apply
///    levels (0.04, 0.8).
///
/// Bloom is intentionally required and is added before the nonlinear tone map.
pub fn restore_scene_pixel_rgb(
    scene_rgb: [f32; 3],
    bloom_rgb: [f32; 3],
    color_cube: RestoreSceneColorCube32<'_>,
    grade: RestoreSceneGrade,
) -> Result<[f32; 3]> {
    require_finite_rgb("RestoreScene MainScene sample", scene_rgb)?;
    require_finite_rgb("RestoreScene bloom sample", bloom_rgb)?;

    let mut color = [
        scene_rgb[0] + bloom_rgb[0],
        scene_rgb[1] + bloom_rgb[1],
        scene_rgb[2] + bloom_rgb[2],
    ];
    color = restore_scene_exposure_rgb(color, grade.middle_grey(), grade.average_luminance());
    color = restore_scene_tone_map_rgb(color);
    color = sample_restore_scene_color_cube(color_cube, saturate_rgb(color));
    color = apply_hsv_shift(color, grade.hsv());
    color = apply_color_balance(color, grade.color_balance());
    Ok(apply_levels(color, 0.04, 0.8))
}

/// `gfx/FX/restorescene.shader:135-140`, `ADJUSTED_EXPOSURE` branch.
pub fn restore_scene_exposure_rgb(
    rgb: [f32; 3],
    middle_grey: f32,
    average_luminance: f32,
) -> [f32; 3] {
    let scale = middle_grey / average_luminance;
    rgb.map(|channel| channel * scale)
}

/// `gfx/FX/restorescene.shader:194-202`, followed by the final saturate at
/// line 223.
pub fn restore_scene_tone_map_rgb(rgb: [f32; 3]) -> [f32; 3] {
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

/// `gfx/FX/restorescene.shader:90-109`.
///
/// The cube is a 1024x32 strip: 32 blue slices laid out horizontally.
/// Hardware bilinear filtering is mirrored within the strip and the shader's
/// explicit interpolation is mirrored between blue slices.
pub fn sample_restore_scene_color_cube(
    lut: RestoreSceneColorCube32<'_>,
    rgb: [f32; 3],
) -> [f32; 3] {
    let color = saturate_rgb(rgb);
    let cube_size = RESTORE_SCENE_COLOR_CUBE_EDGE as f32;
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

pub fn restore_scene_luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * RESTORE_SCENE_LUMINANCE_VECTOR[0]
        + rgb[1] * RESTORE_SCENE_LUMINANCE_VECTOR[1]
        + rgb[2] * RESTORE_SCENE_LUMINANCE_VECTOR[2]
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

fn sample_color_cube_strip(lut: RestoreSceneColorCube32<'_>, u: f32, v: f32) -> [f32; 3] {
    let width = RESTORE_SCENE_COLOR_CUBE_WIDTH as f32;
    let height = RESTORE_SCENE_COLOR_CUBE_EDGE as f32;
    let x = (u * width - 0.5).clamp(0.0, width - 1.0);
    let y = (v * height - 0.5).clamp(0.0, height - 1.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(RESTORE_SCENE_COLOR_CUBE_WIDTH - 1);
    let y1 = (y0 + 1).min(RESTORE_SCENE_COLOR_CUBE_EDGE - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let c00 = lut.strip_texel(x0, y0);
    let c10 = lut.strip_texel(x1, y0);
    let c01 = lut.strip_texel(x0, y1);
    let c11 = lut.strip_texel(x1, y1);
    lerp3(lerp3(c00, c10, tx), lerp3(c01, c11, tx), ty)
}

fn apply_hsv_shift(rgb: [f32; 3], hsv: [f32; 4]) -> [f32; 3] {
    let mut shifted = rgb_to_hsv(rgb);
    shifted[1] *= hsv[1];
    shifted[2] *= hsv[2];
    shifted[0] = shader_mod(shifted[0] + hsv[0], 6.0);
    hsv_to_rgb_post(shifted)
}

fn apply_color_balance(rgb: [f32; 3], color_balance: [f32; 3]) -> [f32; 3] {
    saturate_rgb([
        rgb[0] * color_balance[0],
        rgb[1] * color_balance[1],
        rgb[2] * color_balance[2],
    ])
}

/// `standardfuncsgfx.fxh:172-176`.
fn apply_levels(rgb: [f32; 3], min_input: f32, max_input: f32) -> [f32; 3] {
    let denom = max_input - min_input;
    rgb.map(|channel| ((channel - min_input).clamp(0.0, 1.0) / denom).clamp(0.0, 1.0))
}

fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let cmax = rgb[0].max(rgb[1].max(rgb[2]));
    let cmin = rgb[0].min(rgb[1].min(rgb[2]));
    let diff = cmax - cmin;

    let mut hue = 0.0;
    let mut saturation = 0.0;
    if diff != 0.0 {
        saturation = diff / cmax;
        if cmax == rgb[0] {
            hue = (rgb[1] - rgb[2]) / diff + 6.0;
        } else if cmax == rgb[1] {
            hue = (rgb[2] - rgb[0]) / diff + 2.0;
        } else {
            hue = (rgb[0] - rgb[1]) / diff + 4.0;
        }
    }
    [hue, saturation, cmax]
}

fn hue_post(hue: f32) -> [f32; 3] {
    let x = 1.0 - (shader_mod(hue, 2.0) - 1.0).abs();
    if hue < 1.0 {
        [1.0, x, 0.0]
    } else if hue < 2.0 {
        [x, 1.0, 0.0]
    } else if hue < 3.0 {
        [0.0, 1.0, x]
    } else if hue < 4.0 {
        [0.0, x, 1.0]
    } else if hue < 5.0 {
        [x, 0.0, 1.0]
    } else {
        [1.0, 0.0, x]
    }
}

fn hsv_to_rgb_post(hsv: [f32; 3]) -> [f32; 3] {
    if hsv[1] != 0.0 {
        let chroma = hsv[1] * hsv[2];
        let hue = hue_post(hsv[0]);
        return [
            (hue[0] * chroma + (hsv[2] - chroma)).clamp(0.0, 1.0),
            (hue[1] * chroma + (hsv[2] - chroma)).clamp(0.0, 1.0),
            (hue[2] * chroma + (hsv[2] - chroma)).clamp(0.0, 1.0),
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

fn require_finite_rgb(label: &str, rgb: [f32; 3]) -> Result<()> {
    if rgb.into_iter().any(|value| !value.is_finite()) {
        return Err(format!("{label} contains a non-finite value").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_lut() -> ColorCorrectionLut {
        let edge = RESTORE_SCENE_COLOR_CUBE_EDGE;
        let mut texels = Vec::with_capacity(edge * edge * edge);
        for green in 0..edge {
            for blue in 0..edge {
                for red in 0..edge {
                    texels.push([
                        red as f32 / (edge - 1) as f32,
                        green as f32 / (edge - 1) as f32,
                        blue as f32 / (edge - 1) as f32,
                    ]);
                }
            }
        }
        ColorCorrectionLut {
            width: edge * edge,
            height: edge,
            slice_size: edge,
            texels,
        }
    }

    fn neutral_grade() -> RestoreSceneGrade {
        RestoreSceneGrade::new([0.0, 1.0, 1.0, 0.0], [1.0; 3], 1.0, 1.0)
            .expect("valid neutral grade")
    }

    #[test]
    fn cube_view_rejects_wrong_shape_and_non_finite_texels() {
        let mut lut = identity_lut();
        lut.width = 64;
        assert!(RestoreSceneColorCube32::new(&lut)
            .expect_err("wrong shape")
            .to_string()
            .contains("must be 1024x32"));

        let mut lut = identity_lut();
        lut.texels[0][1] = f32::NAN;
        assert!(RestoreSceneColorCube32::new(&lut)
            .expect_err("non-finite texel")
            .to_string()
            .contains("non-finite"));
    }

    #[test]
    fn grade_rejects_missing_or_invalid_luminance_semantics() {
        assert!(RestoreSceneGrade::new([0.0, 1.0, 1.0, 0.0], [1.0; 3], 0.55, 0.0).is_err());
        assert!(RestoreSceneGrade::new([0.0, 1.0, 1.0, 0.0], [1.0; 3], 0.55, f32::NAN).is_err());
    }

    #[test]
    fn tone_map_matches_shader_mid_gray_sample() {
        let output = restore_scene_tone_map_rgb([0.23; 3]);
        for channel in output {
            assert!((channel - 0.395515).abs() < 0.00001, "{channel}");
        }
    }

    #[test]
    fn exact_strip_sampling_preserves_identity_cube() {
        let lut = identity_lut();
        let cube = RestoreSceneColorCube32::new(&lut).expect("identity LUT has exact shape");
        for rgb in [[0.0; 3], [1.0; 3], [0.5, 0.25, 0.75]] {
            let output = sample_restore_scene_color_cube(cube, rgb);
            for channel in 0..3 {
                assert!(
                    (output[channel] - rgb[channel]).abs() < 0.00001,
                    "{output:?}"
                );
            }
        }
    }

    #[test]
    fn bloom_is_added_before_exposure_and_tonemap() {
        let lut = identity_lut();
        let cube = RestoreSceneColorCube32::new(&lut).expect("identity LUT has exact shape");
        let scene = [0.20, 0.30, 0.40];
        let bloom = [0.05, 0.10, 0.15];
        let output =
            restore_scene_pixel_rgb(scene, bloom, cube, neutral_grade()).expect("finite pixel");

        let toned = restore_scene_tone_map_rgb([
            scene[0] + bloom[0],
            scene[1] + bloom[1],
            scene[2] + bloom[2],
        ]);
        let expected = apply_levels(toned, 0.04, 0.8);
        for channel in 0..3 {
            assert!(
                (output[channel] - expected[channel]).abs() < 0.00001,
                "{output:?} vs {expected:?}"
            );
        }

        let wrong_post_tonemap = restore_scene_tone_map_rgb(scene);
        assert!(
            (0..3).any(|channel| {
                (output[channel] - (wrong_post_tonemap[channel] + bloom[channel])).abs() > 0.001
            }),
            "nonlinear pre-tonemap bloom order must differ from post-tonemap addition"
        );
    }

    #[test]
    fn exposure_uses_middle_grey_over_average_luminance() {
        assert_eq!(
            restore_scene_exposure_rgb([2.0, 4.0, 8.0], 0.5, 2.0),
            [0.5, 1.0, 2.0]
        );
    }
}
