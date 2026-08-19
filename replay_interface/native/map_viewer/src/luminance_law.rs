
use crate::constants::DXGI_FORMAT_R16G16B16A16_FLOAT;

pub const POSTFX_REQUIRED_SHADER_SOURCES: [&str; 5] = [
    "gfx/FX/downsample.shader",
    "gfx/FX/downsample_luminance.shader",
    "gfx/FX/bloom.shader",
    "gfx/FX/restorescene.shader",
    "gfx/FX/lut_blender.shader",
];

pub const LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];
pub const LOG_LUMINANCE_EPSILON: f32 = 0.0001;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LuminanceClampBounds {
    pub min_hdr_adjustment: f32,
    pub max_hdr_adjustment: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AverageLuminanceReduction {
    pub geometric_mean_luminance: f32,
    pub clamped: f32,
    pub clamped_to_bound: bool,
    pub r16_quantized: f32,
    pub r16_bits: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporalLuminanceState {
    pub r16_bits: u16,
}

impl TemporalLuminanceState {
    pub fn value(self) -> f32 {
        f16_bits_to_f32(self.r16_bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalLuminanceContinuity {
    ConsecutiveAuthenticated {
        prior: TemporalLuminanceState,
    },
    EngineResourceReset {
        reason: TemporalLuminanceResetReason,
    },
    InitializationUnproven,
    SeekResetUnproven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalLuminanceResetReason {
    ResourcesAllocated,
    ResourcesRecreated,
    HdrModeChanged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthenticatedLuminanceCadence {
    pub delta_time_seconds: f32,
    pub adjustment_speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporalLuminanceReceipt {
    pub reset_reason: Option<TemporalLuminanceResetReason>,
    pub prior_r16_bits: u16,
    pub current_target_f32_bits: u32,
    pub delta_time_f32_bits: u32,
    pub adjustment_speed_f32_bits: u32,
    pub tau_delta_time_f32_bits: u32,
    pub blend_factor_f32_bits: u32,
    pub unquantized_result_f32_bits: u32,
    pub output_r16_bits: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporalLuminanceStep {
    pub state: TemporalLuminanceState,
    pub receipt: TemporalLuminanceReceipt,
}

pub fn adapt_luminance_temporal(
    continuity: TemporalLuminanceContinuity,
    current_target: f32,
    cadence: Option<AuthenticatedLuminanceCadence>,
) -> Result<TemporalLuminanceStep, String> {
    let (prior, reset_reason) = match continuity {
        TemporalLuminanceContinuity::ConsecutiveAuthenticated { prior } => (prior, None),
        TemporalLuminanceContinuity::EngineResourceReset { reason } => (
            TemporalLuminanceState {
                r16_bits: f32_to_f16_bits(0.0),
            },
            Some(reason),
        ),
        TemporalLuminanceContinuity::InitializationUnproven => {
            return Err(
                "temporal luminance initialization is unproven; an authenticated prior R16 texel is required"
                    .into(),
            );
        }
        TemporalLuminanceContinuity::SeekResetUnproven => {
            return Err(
                "temporal luminance seek/reset semantics are unproven; an authenticated consecutive prior is required"
                    .into(),
            );
        }
    };
    let (delta_time_seconds, adjustment_speed, tau_delta_time) = if reset_reason.is_some() {
        if cadence.is_some() {
            return Err(
                "temporal luminance engine-reset path must not carry unused frame cadence".into(),
            );
        }
        (0.0_f32, 0.0_f32, 100.0_f32)
    } else {
        let cadence = cadence.ok_or(
            "temporal luminance cadence is absent; authenticated delta time and adjustment speed are required",
        )?;
        if !cadence.delta_time_seconds.is_finite() || cadence.delta_time_seconds < 0.0 {
            return Err(format!(
                "temporal luminance delta time is invalid: {}",
                cadence.delta_time_seconds
            ));
        }
        if !cadence.adjustment_speed.is_finite() || cadence.adjustment_speed < 0.0 {
            return Err(format!(
                "temporal luminance adjustment speed is invalid: {}",
                cadence.adjustment_speed
            ));
        }
        let tau_delta_time = cadence.adjustment_speed * cadence.delta_time_seconds;
        (
            cadence.delta_time_seconds,
            cadence.adjustment_speed,
            tau_delta_time,
        )
    };
    let prior_value = prior.value();
    if !prior_value.is_finite() || (reset_reason.is_none() && prior_value <= 0.0) {
        return Err(format!(
            "temporal luminance prior R16 value is invalid: bits {:#06x} value {}",
            prior.r16_bits, prior_value
        ));
    }
    if !current_target.is_finite() || current_target <= 0.0 {
        return Err(format!(
            "temporal luminance current target is invalid: {current_target}"
        ));
    }
    if !tau_delta_time.is_finite() {
        return Err(format!(
            "temporal luminance tau-delta product is non-finite: {} * {}",
            adjustment_speed, delta_time_seconds
        ));
    }
    let blend_factor = 1.0_f32 - (-tau_delta_time).exp();
    let unquantized_result = prior_value + (current_target - prior_value) * blend_factor;
    if !unquantized_result.is_finite() || unquantized_result <= 0.0 {
        return Err(format!(
            "temporal luminance adapted result is invalid: {unquantized_result}"
        ));
    }
    let output_r16_bits = f32_to_f16_bits(unquantized_result);
    let output_value = f16_bits_to_f32(output_r16_bits);
    if !output_value.is_finite() || output_value <= 0.0 {
        return Err(format!(
            "temporal luminance R16 quantization produced an invalid state: bits {output_r16_bits:#06x} value {output_value}"
        ));
    }

    Ok(TemporalLuminanceStep {
        state: TemporalLuminanceState {
            r16_bits: output_r16_bits,
        },
        receipt: TemporalLuminanceReceipt {
            reset_reason,
            prior_r16_bits: prior.r16_bits,
            current_target_f32_bits: current_target.to_bits(),
            delta_time_f32_bits: delta_time_seconds.to_bits(),
            adjustment_speed_f32_bits: adjustment_speed.to_bits(),
            tau_delta_time_f32_bits: tau_delta_time.to_bits(),
            blend_factor_f32_bits: blend_factor.to_bits(),
            unquantized_result_f32_bits: unquantized_result.to_bits(),
            output_r16_bits,
        },
    })
}

pub fn scene_average_luminance_r16g16b16a16_float(
    raw: &[u8],
    width: usize,
    height: usize,
    row_pitch: usize,
    bounds: LuminanceClampBounds,
) -> Result<AverageLuminanceReduction, String> {
    if width == 0 || height == 0 {
        return Err("luminance reduction needs a non-empty scene".into());
    }
    let min_row = width
        .checked_mul(8)
        .ok_or("luminance reduction row-byte overflow")?;
    if row_pitch < min_row {
        return Err(format!(
            "luminance reduction row_pitch {row_pitch} is smaller than {min_row} bytes for {width} RGBA16F texels"
        ));
    }
    let needed = row_pitch
        .checked_mul(height)
        .ok_or("luminance reduction size overflow")?;
    if raw.len() < needed {
        return Err(format!(
            "luminance reduction scene payload is {} bytes, needs at least {needed} ({row_pitch} x {height})",
            raw.len()
        ));
    }
    if !(bounds.min_hdr_adjustment.is_finite()
        && bounds.max_hdr_adjustment.is_finite()
        && bounds.min_hdr_adjustment > 0.0
        && bounds.max_hdr_adjustment > 0.0)
    {
        return Err(format!(
            "luminance clamp bounds are invalid: min {} max {}",
            bounds.min_hdr_adjustment, bounds.max_hdr_adjustment
        ));
    }

    let texel_luminance_input = |x: usize, y: usize| -> [f32; 3] {
        let base = y * row_pitch + x * 8;
        let r = f16_bits_to_f32(u16::from_le_bytes([raw[base], raw[base + 1]]));
        let g = f16_bits_to_f32(u16::from_le_bytes([raw[base + 2], raw[base + 3]]));
        let b = f16_bits_to_f32(u16::from_le_bytes([raw[base + 4], raw[base + 5]]));
        [r, g, b]
    };

    let mut sum_log = 0.0f64;
    let mut count = 0u64;

    let hw = width / 2;
    let hh = height / 2;
    if hw == 0 || hh == 0 {
        return Err("luminance reduction scene is too small for the floor-half bright pass".into());
    }
    for by in 0..hh {
        let y0 = by * 2;
        let y1 = (y0 + 1).min(height - 1);
        for bx in 0..hw {
            let x0 = bx * 2;
            let x1 = (x0 + 1).min(width - 1);
            let c00 = texel_luminance_input(x0, y0);
            let c10 = texel_luminance_input(x1, y0);
            let c01 = texel_luminance_input(x0, y1);
            let c11 = texel_luminance_input(x1, y1);
            let mut box_avg = [0.0f32; 3];
            for ch in 0..3 {
                box_avg[ch] = 0.25 * (c00[ch] + c10[ch] + c01[ch] + c11[ch]);
            }
            let dot = box_avg[0] * LUMINANCE_VECTOR[0]
                + box_avg[1] * LUMINANCE_VECTOR[1]
                + box_avg[2] * LUMINANCE_VECTOR[2];
            let log_luminance = (dot.max(0.0) + LOG_LUMINANCE_EPSILON).ln();
            sum_log += f64::from(log_luminance);
            count += 1;
        }
    }

    let mean_log = (sum_log / count as f64) as f32;
    let geometric_mean_luminance = mean_log.exp();
    let clamped = geometric_mean_luminance
        .max(bounds.min_hdr_adjustment)
        .min(bounds.max_hdr_adjustment);
    let clamped_to_bound = clamped.to_bits() != geometric_mean_luminance.to_bits();
    let r16_bits = f32_to_f16_bits(clamped);
    let r16_quantized = f16_bits_to_f32(r16_bits);

    Ok(AverageLuminanceReduction {
        geometric_mean_luminance,
        clamped,
        clamped_to_bound,
        r16_quantized,
        r16_bits,
    })
}

pub fn scene_average_luminance(
    raw: &[u8],
    width: usize,
    height: usize,
    row_pitch: usize,
    dxgi_format: u32,
    bounds: LuminanceClampBounds,
) -> Result<AverageLuminanceReduction, String> {
    if dxgi_format != DXGI_FORMAT_R16G16B16A16_FLOAT {
        return Err(format!(
            "luminance reduction expects the R16G16B16A16_FLOAT MainScene (DXGI {DXGI_FORMAT_R16G16B16A16_FLOAT}), got DXGI {dxgi_format}"
        ));
    }
    scene_average_luminance_r16g16b16a16_float(raw, width, height, row_pitch, bounds)
}

pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x3FF);
    let magnitude = match (exponent, mantissa) {
        (0, 0) => 0,
        (0, m) => {
            let top = 31 - m.leading_zeros();
            let f32_exp = (127 + (top as i32 - 24)) as u32;
            let frac = (m & ((1 << top) - 1)) << (23 - top);
            (f32_exp << 23) | frac
        }
        (0x1F, 0) => 0xFF << 23,
        (0x1F, m) => (0xFF << 23) | (m << 13),
        (e, m) => ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(sign | magnitude)
}

pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007F_FFFF;

    if exp == 0xFF {
        return if mantissa != 0 {
            sign | 0x7E00
        } else {
            sign | 0x7C00
        };
    }

    let unbiased = exp - 127;
    let half_exp = unbiased + 15;

    if half_exp >= 0x1F {
        return sign | 0x7C00;
    }

    if half_exp <= 0 {
        if exp == 0 || half_exp < -10 {
            return sign;
        }
        let significand = mantissa | 0x0080_0000;
        let shift = (14 - half_exp) as u32;
        let result = round_shift_right_nearest_even(significand, shift);
        return sign | result as u16;
    }

    let mut he = half_exp as u32;
    let mut hm = round_shift_right_nearest_even(mantissa, 13);
    if hm == 0x0400 {
        hm = 0;
        he += 1;
        if he >= 0x1F {
            return sign | 0x7C00;
        }
    }
    sign | ((he << 10) as u16) | hm as u16
}

fn round_shift_right_nearest_even(value: u32, shift: u32) -> u32 {
    let kept = value >> shift;
    let remainder = value & ((1u32 << shift) - 1);
    let halfway = 1u32 << (shift - 1);
    if remainder > halfway || (remainder == halfway && (kept & 1) == 1) {
        kept + 1
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: LuminanceClampBounds = LuminanceClampBounds {
        min_hdr_adjustment: 0.5,
        max_hdr_adjustment: 1.0,
    };

    fn rgba16f_texel(r: f32, g: f32, b: f32) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&f32_to_f16_bits(r).to_le_bytes());
        out[2..4].copy_from_slice(&f32_to_f16_bits(g).to_le_bytes());
        out[4..6].copy_from_slice(&f32_to_f16_bits(b).to_le_bytes());
        out[6..8].copy_from_slice(&f32_to_f16_bits(1.0).to_le_bytes());
        out
    }

    fn solid_scene(width: usize, height: usize, rgb: [f32; 3]) -> Vec<u8> {
        let texel = rgba16f_texel(rgb[0], rgb[1], rgb[2]);
        let mut out = Vec::with_capacity(width * height * 8);
        for _ in 0..(width * height) {
            out.extend_from_slice(&texel);
        }
        out
    }

    fn temporal_step(
        prior: f32,
        target: f32,
        delta_time_seconds: f32,
        adjustment_speed: f32,
    ) -> Result<TemporalLuminanceStep, String> {
        adapt_luminance_temporal(
            TemporalLuminanceContinuity::ConsecutiveAuthenticated {
                prior: TemporalLuminanceState {
                    r16_bits: f32_to_f16_bits(prior),
                },
            },
            target,
            Some(AuthenticatedLuminanceCadence {
                delta_time_seconds,
                adjustment_speed,
            }),
        )
    }

    #[test]
    fn temporal_adaptation_moves_toward_dark_and_bright_targets() {
        let dark = temporal_step(1.0, 0.5, 1.0 / 60.0, 15.0).expect("dark step");
        assert!(
            (0.5..1.0).contains(&dark.state.value()),
            "dark target must move the prior downward without overshoot: {}",
            dark.state.value()
        );

        let bright = temporal_step(0.5, 0.9, 1.0 / 60.0, 15.0).expect("bright step");
        assert!(
            (0.5..0.9).contains(&bright.state.value()),
            "bright target must move the prior upward without overshoot: {}",
            bright.state.value()
        );

        let expected_tau_delta = 15.0_f32 * (1.0_f32 / 60.0_f32);
        let expected_blend = 1.0_f32 - (-expected_tau_delta).exp();
        let expected_unquantized = 0.5_f32 + (0.9_f32 - 0.5_f32) * expected_blend;
        assert_eq!(
            bright.receipt.tau_delta_time_f32_bits,
            expected_tau_delta.to_bits()
        );
        assert_eq!(
            bright.receipt.blend_factor_f32_bits,
            expected_blend.to_bits()
        );
        assert_eq!(
            bright.receipt.unquantized_result_f32_bits,
            expected_unquantized.to_bits()
        );
        assert_eq!(
            bright.receipt.output_r16_bits,
            f32_to_f16_bits(expected_unquantized)
        );
    }

    #[test]
    fn temporal_output_r16_is_the_next_frames_prior() {
        let first = temporal_step(0.5, 0.9, 1.0 / 60.0, 15.0).expect("first");
        let second = adapt_luminance_temporal(
            TemporalLuminanceContinuity::ConsecutiveAuthenticated { prior: first.state },
            0.9,
            Some(AuthenticatedLuminanceCadence {
                delta_time_seconds: 1.0 / 60.0,
                adjustment_speed: 15.0,
            }),
        )
        .expect("second");

        assert_eq!(second.receipt.prior_r16_bits, first.state.r16_bits);
        assert!(
            second.state.value() > first.state.value(),
            "persisted prior must continue adapting toward the bright target"
        );
    }

    #[test]
    fn temporal_zero_delta_preserves_prior_and_receipt_is_deterministic() {
        let a = temporal_step(0.7001, 0.5, 0.0, 15.0).expect("paused step");
        let b = temporal_step(0.7001, 0.5, 0.0, 15.0).expect("repeat paused step");
        assert_eq!(
            a, b,
            "identical authenticated inputs need identical receipts"
        );
        assert_eq!(
            a.state.r16_bits,
            f32_to_f16_bits(0.7001),
            "zero delta must retain the already-quantized prior"
        );
        assert_eq!(a.receipt.blend_factor_f32_bits, 0.0_f32.to_bits());
    }

    #[test]
    fn authenticated_engine_reset_uses_zero_history_and_forced_tau_100() {
        let reset = adapt_luminance_temporal(
            TemporalLuminanceContinuity::EngineResourceReset {
                reason: TemporalLuminanceResetReason::ResourcesAllocated,
            },
            0.7,
            None,
        )
        .expect("exact engine reset");
        assert_eq!(
            reset.receipt.reset_reason,
            Some(TemporalLuminanceResetReason::ResourcesAllocated)
        );
        assert_eq!(reset.receipt.prior_r16_bits, f32_to_f16_bits(0.0));
        assert_eq!(reset.receipt.tau_delta_time_f32_bits, 100.0_f32.to_bits());
        assert_eq!(reset.receipt.delta_time_f32_bits, 0.0_f32.to_bits());
        assert_eq!(reset.receipt.adjustment_speed_f32_bits, 0.0_f32.to_bits());
        assert_eq!(reset.state.r16_bits, f32_to_f16_bits(0.7));

        let with_unused_cadence = adapt_luminance_temporal(
            TemporalLuminanceContinuity::EngineResourceReset {
                reason: TemporalLuminanceResetReason::HdrModeChanged,
            },
            0.7,
            Some(AuthenticatedLuminanceCadence {
                delta_time_seconds: 1.0 / 60.0,
                adjustment_speed: 15.0,
            }),
        )
        .expect_err("reset path must reject unused cadence");
        assert!(
            with_unused_cadence.contains("must not carry unused frame cadence"),
            "{with_unused_cadence}"
        );
    }

    #[test]
    fn temporal_initialization_seek_and_absent_cadence_fail_closed() {
        let cadence = Some(AuthenticatedLuminanceCadence {
            delta_time_seconds: 1.0 / 60.0,
            adjustment_speed: 15.0,
        });
        let initial = adapt_luminance_temporal(
            TemporalLuminanceContinuity::InitializationUnproven,
            0.7,
            cadence,
        )
        .expect_err("unproved initialization must fail");
        assert!(initial.contains("initialization is unproven"), "{initial}");

        let seek =
            adapt_luminance_temporal(TemporalLuminanceContinuity::SeekResetUnproven, 0.7, cadence)
                .expect_err("unproved seek reset must fail");
        assert!(seek.contains("seek/reset semantics are unproven"), "{seek}");

        let absent = adapt_luminance_temporal(
            TemporalLuminanceContinuity::ConsecutiveAuthenticated {
                prior: TemporalLuminanceState { r16_bits: 0x3800 },
            },
            0.7,
            None,
        )
        .expect_err("absent cadence must fail");
        assert!(absent.contains("cadence is absent"), "{absent}");
    }

    #[test]
    fn temporal_nonfinite_and_negative_inputs_fail_closed() {
        for target in [f32::NAN, f32::INFINITY, -0.5, 0.0] {
            let err =
                temporal_step(0.5, target, 1.0 / 60.0, 15.0).expect_err("invalid target must fail");
            assert!(err.contains("current target is invalid"), "{err}");
        }
        for delta in [f32::NAN, f32::INFINITY, -0.01] {
            let err = temporal_step(0.5, 0.7, delta, 15.0).expect_err("invalid delta must fail");
            assert!(err.contains("delta time is invalid"), "{err}");
        }
        for speed in [f32::NAN, f32::INFINITY, -15.0] {
            let err =
                temporal_step(0.5, 0.7, 1.0 / 60.0, speed).expect_err("invalid speed must fail");
            assert!(err.contains("adjustment speed is invalid"), "{err}");
        }

        let negative_prior = adapt_luminance_temporal(
            TemporalLuminanceContinuity::ConsecutiveAuthenticated {
                prior: TemporalLuminanceState {
                    r16_bits: f32_to_f16_bits(-0.5),
                },
            },
            0.7,
            Some(AuthenticatedLuminanceCadence {
                delta_time_seconds: 1.0 / 60.0,
                adjustment_speed: 15.0,
            }),
        )
        .expect_err("negative prior must fail");
        assert!(negative_prior.contains("prior R16 value is invalid"));
    }

    #[test]
    fn f16_round_trip_known_values() {
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0x3800), 0.5);
        assert_eq!(f16_bits_to_f32(0x4000), 2.0);
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(2.0), 0x4000);
        assert_eq!(f32_to_f16_bits(-1.0), 0xBC00);
    }

    #[test]
    fn f16_encode_matches_decode_round_trip_over_grid() {
        for bits in 0u16..=0xFFFF {
            let exp = (bits >> 10) & 0x1F;
            let mant = bits & 0x3FF;
            if exp == 0x1F {
                continue;
            }
            let _ = mant;
            let value = f16_bits_to_f32(bits);
            let reencoded = f32_to_f16_bits(value);
            assert_eq!(
                f16_bits_to_f32(reencoded),
                value,
                "half bits {bits:#06x} did not survive decode/encode round-trip"
            );
        }
    }

    #[test]
    fn f16_encode_rounds_to_nearest_even() {
        assert_eq!(f32_to_f16_bits(0.1), 0x2E66);
        let mid = f16_bits_to_f32(0x3801);
        let below = f16_bits_to_f32(0x3800);
        let between = (mid + below) * 0.5;
        let enc = f32_to_f16_bits(between);
        assert_eq!(enc, 0x3800);
    }

    #[test]
    fn uniform_dim_scene_clamps_to_floor() {
        let scene = solid_scene(8, 8, [0.1, 0.1, 0.1]);
        let out =
            scene_average_luminance(&scene, 8, 8, 8 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        assert!(
            (out.geometric_mean_luminance - (0.1 + 0.0001)).abs() < 2e-3,
            "geo mean {} not ~0.1",
            out.geometric_mean_luminance
        );
        assert!(out.clamped_to_bound);
        assert_eq!(out.clamped, 0.5);
        assert_eq!(
            out.r16_bits, 0x3800,
            "clamped 0.5 must store as half 0x3800"
        );
        assert_eq!(out.r16_quantized, 0.5);
    }

    #[test]
    fn uniform_bright_scene_does_not_clamp() {
        let scene = solid_scene(8, 8, [0.7, 0.7, 0.7]);
        let out =
            scene_average_luminance(&scene, 8, 8, 8 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        assert!(!out.clamped_to_bound, "0.7 scene must not clamp");
        assert!(
            (out.clamped - 0.7).abs() < 3e-3,
            "clamped {} not ~0.7",
            out.clamped
        );
        assert!((out.r16_quantized - 0.7).abs() < 3e-3);
    }

    #[test]
    fn uniform_overbright_scene_clamps_to_ceiling() {
        let scene = solid_scene(4, 4, [2.0, 2.0, 2.0]);
        let out =
            scene_average_luminance(&scene, 4, 4, 4 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        assert!(out.clamped_to_bound);
        assert_eq!(out.clamped, 1.0);
        assert_eq!(out.r16_bits, 0x3C00);
    }

    #[test]
    fn reversed_shipped_hdr_bounds_preserve_hlsl_clamp_order() {
        let scene = solid_scene(4, 4, [0.7, 0.7, 0.7]);
        let out = scene_average_luminance(
            &scene,
            4,
            4,
            4 * 8,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            LuminanceClampBounds {
                min_hdr_adjustment: 1.0,
                max_hdr_adjustment: 0.3,
            },
        )
        .expect("HLSL clamp permits reversed shipped bounds");
        assert_eq!(out.clamped.to_bits(), 0.3_f32.to_bits());
    }

    #[test]
    fn box_average_is_order_of_operations_engine_faithful() {
        let bright = rgba16f_texel(0.8, 0.8, 0.8);
        let dark = rgba16f_texel(0.0, 0.0, 0.0);
        let mut scene = Vec::new();
        scene.extend_from_slice(&bright);
        scene.extend_from_slice(&dark);
        scene.extend_from_slice(&dark);
        scene.extend_from_slice(&dark);
        let out =
            scene_average_luminance(&scene, 2, 2, 2 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        assert!(
            (out.geometric_mean_luminance - 0.2001).abs() < 3e-3,
            "geo mean {} not ~0.2 (box-average-then-log)",
            out.geometric_mean_luminance
        );
    }

    #[test]
    fn row_pitch_padding_is_respected() {
        let bright = rgba16f_texel(0.6, 0.6, 0.6);
        let pad = [0u8; 8];
        let mut scene = Vec::new();
        for _ in 0..2 {
            scene.extend_from_slice(&bright);
            scene.extend_from_slice(&bright);
            scene.extend_from_slice(&pad);
        }
        let row_pitch = 2 * 8 + 8;
        let out = scene_average_luminance(
            &scene,
            2,
            2,
            row_pitch,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            BOUNDS,
        )
        .expect("reduce");
        assert!(
            (out.clamped - 0.6).abs() < 3e-3,
            "padding leaked into the reduction: clamped {}",
            out.clamped
        );
    }

    #[test]
    fn rejects_non_rgba16f_format() {
        let scene = solid_scene(2, 2, [0.5, 0.5, 0.5]);
        let err = scene_average_luminance(&scene, 2, 2, 2 * 8, 28, BOUNDS)
            .expect_err("must reject non-RGBA16F");
        assert!(err.contains("R16G16B16A16_FLOAT"), "{err}");
    }

    #[test]
    fn rejects_short_payload() {
        let scene = vec![0u8; 8];
        let err = scene_average_luminance_r16g16b16a16_float(&scene, 2, 2, 16, BOUNDS)
            .expect_err("must reject short payload");
        assert!(err.contains("needs at least"), "{err}");
    }

    #[test]
    fn rejects_row_pitch_below_row_width() {
        let scene = vec![0u8; 4 * 8];
        let err = scene_average_luminance_r16g16b16a16_float(&scene, 2, 2, 8, BOUNDS)
            .expect_err("row_pitch too small");
        assert!(err.contains("row_pitch"), "{err}");
    }

    const RESTORE_GATE_DIR: &str = r"C:\hoi4_evidence\capture_corpus_preserved\restore_gate_1";
    const CAPTURED_MAIN_SCENE: &str =
        "restore_frame_draw_map_frame_shared_payload_ps_srv_MainScene_b811e76916a16e8c.d3d11";
    const CAPTURED_AVG_LUM: &str =
        "restore_frame_draw_map_frame_shared_payload_ps_srv_AverageLuminanceTexture_a86bb8b3d96548fd.d3d11";
    const CAPTURED_SCENE_W: usize = 1920;
    const CAPTURED_SCENE_H: usize = 1080;
    const CAPTURED_SCENE_ROW_PITCH: usize = 15360;

    fn restore_gate_dir() -> std::path::PathBuf {
        if let Some(root) = std::env::var_os("BATTLE_PLAN_RESTORE_GATE_1_ROOT").or_else(|| std::env::var_os("HOI4_RESTORE_GATE_1_ROOT")) {
            return std::path::PathBuf::from(root);
        }
        std::env::var_os("BATTLE_PLAN_CAPTURES_ROOT").or_else(|| std::env::var_os("HOI4_CAPTURES_ROOT"))
            .map(std::path::PathBuf::from)
            .map(|root| root.join("restore_gate_1"))
            .unwrap_or_else(|| std::path::PathBuf::from(RESTORE_GATE_DIR))
    }

    #[test]
    #[cfg_attr(
        not(gate_restore_gate_1_capture),
        ignore = "BATTLE_PLAN_RESTORE_GATE_1_ROOT (or BATTLE_PLAN_CAPTURES_ROOT/restore_gate_1 or C:\\hoi4_evidence\\capture_corpus_preserved\\restore_gate_1) with the captured MainScene and AverageLuminanceTexture payloads"
    )]
    fn gate_captured_scene_reproduces_captured_average_luminance() {
        let dir = replay_test_gates::require_existing_dir(restore_gate_dir());
        let scene_path = dir.join(CAPTURED_MAIN_SCENE);
        let avg_path = dir.join(CAPTURED_AVG_LUM);
        replay_test_gates::require_existing_file(&scene_path);
        replay_test_gates::require_existing_file(&avg_path);
        let scene = std::fs::read(&scene_path).expect("read captured MainScene");
        assert_eq!(
            scene.len(),
            CAPTURED_SCENE_ROW_PITCH * CAPTURED_SCENE_H,
            "captured MainScene byte count"
        );

        let out = scene_average_luminance(
            &scene,
            CAPTURED_SCENE_W,
            CAPTURED_SCENE_H,
            CAPTURED_SCENE_ROW_PITCH,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            BOUNDS,
        )
        .expect("reduce captured scene");

        assert!(
            out.clamped_to_bound && out.geometric_mean_luminance < BOUNDS.min_hdr_adjustment,
            "captured scene did NOT clamp: geo mean {} (expected < {})",
            out.geometric_mean_luminance,
            BOUNDS.min_hdr_adjustment
        );
        assert_eq!(out.clamped, 0.5, "clamp must produce exactly 0.5");

        let avg_raw = std::fs::read(&avg_path).expect("read captured AvgLum texel");
        assert!(avg_raw.len() >= 2, "captured AvgLum payload too short");
        let captured_bits = u16::from_le_bytes([avg_raw[0], avg_raw[1]]);
        let captured_value = f16_bits_to_f32(captured_bits);
        assert_eq!(
            captured_bits, 0x3800,
            "captured AvgLum half bits are {captured_bits:#06x}, expected 0x3800 (0.5)"
        );
        assert_eq!(
            out.r16_bits, captured_bits,
            "ported chain R16 bits {:#06x} != captured {:#06x}",
            out.r16_bits, captured_bits
        );
        assert_eq!(out.r16_quantized, captured_value);
        println!(
            "captured-AvgLum reproduction gate: geo mean {:.6} -> clamp {:.3} -> R16 {:#06x} == captured {:#06x} ({:.3}) [BYTE-EXACT, clamped branch]",
            out.geometric_mean_luminance,
            out.clamped,
            out.r16_bits,
            captured_bits,
            captured_value
        );
    }

    const CAP4_TREES_POST_NEEDLE: &str =
        "trees_frame_draw_map_frame_shared_payload_om_post_pass_render_target";

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    fn cap4_rounds_root() -> std::path::PathBuf {
        corpus::wave1_cap4_rounds_dir(&["fow_on_pov_front", "fow_off_observer_front"]).expect(
            "the CAP4 round pair (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved corpus) is absent, \
             but this test was compiled as runnable",
        )
    }

    fn find_cap4_payload(dir: &std::path::Path, needle: &str) -> Option<std::path::PathBuf> {
        let mut hit = None;
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".d3d11") && name.contains(needle) {
                hit = Some(entry.path());
            }
        }
        hit
    }

    #[test]
    #[cfg_attr(
        not(gate_wave1_cap4_fow_pair),
        ignore = "the CAP4 round corpus with fow_on_pov_front and fow_off_observer_front (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved corpus)"
    )]
    fn gate_wave1_cap4_scene_clamps_to_floor_unclamped_branch_unexercised() {
        let mut rounds_exercised = 0usize;
        for round in ["fow_off_observer_front", "fow_on_pov_front"] {
            let dir = cap4_rounds_root().join(round);
            let scene_path = (dir.is_dir())
                .then(|| find_cap4_payload(&dir, CAP4_TREES_POST_NEEDLE))
                .flatten()
                .unwrap_or_else(|| {
                    panic!(
                        "CAP4 round {round}: no trees post-pass render target under {}, but this \
                         test was compiled as runnable",
                        dir.display()
                    )
                });
            let scene = std::fs::read(&scene_path).expect("read CAP4 trees post-pass RT");
            assert_eq!(
                scene.len(),
                1920 * 1080 * 8,
                "CAP4 scene RT is not 1920x1080 RGBA16F"
            );
            let out = scene_average_luminance(
                &scene,
                1920,
                1080,
                1920 * 8,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                BOUNDS,
            )
            .expect("reduce CAP4 scene");
            eprintln!(
                "CAP4 {round}: scene geo-mean {:.5} -> clamp {:.3} (clamped_to_bound={})",
                out.geometric_mean_luminance, out.clamped, out.clamped_to_bound
            );
            assert!(
                out.geometric_mean_luminance < BOUNDS.min_hdr_adjustment,
                "CAP4 {round} geo-mean {} unexpectedly reached the unclamped branch (>= {})",
                out.geometric_mean_luminance,
                BOUNDS.min_hdr_adjustment
            );
            assert!(out.clamped_to_bound && out.clamped == 0.5);
            assert!(
                (0.15..0.30).contains(&out.geometric_mean_luminance),
                "CAP4 {round} geo-mean {} outside the expected dim band",
                out.geometric_mean_luminance
            );
            rounds_exercised += 1;
        }
        assert_eq!(
            rounds_exercised, 2,
            "both CAP4 rounds must be measured; a partially-executed loop would pass vacuously"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_restore_gate_1_scene),
        ignore = "BATTLE_PLAN_RESTORE_GATE_1_ROOT (or BATTLE_PLAN_CAPTURES_ROOT/restore_gate_1 or C:\\hoi4_evidence\\capture_corpus_preserved\\restore_gate_1) with the captured MainScene payload"
    )]
    #[cfg_attr(
        gate_restore_gate_1_scene,
        ignore = "performance test; run explicitly with --ignored"
    )]
    fn perf_reduction_over_full_res_scene() {
        let dir = replay_test_gates::require_existing_dir(restore_gate_dir());
        let scene_path = dir.join(CAPTURED_MAIN_SCENE);
        replay_test_gates::require_existing_file(&scene_path);
        let scene = std::fs::read(&scene_path).expect("read scene");
        let iters = 20;
        let start = std::time::Instant::now();
        let mut last = 0.0f32;
        for _ in 0..iters {
            let out = scene_average_luminance(
                &scene,
                CAPTURED_SCENE_W,
                CAPTURED_SCENE_H,
                CAPTURED_SCENE_ROW_PITCH,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                BOUNDS,
            )
            .expect("reduce");
            last = out.clamped;
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "perf: reduction over 1920x1080 = {per:.3} ms/frame (clamped {last}); budget 100 ms at speed 5"
        );
        assert!(
            per < 100.0,
            "reduction {per:.3} ms exceeds speed-5 frame budget"
        );
    }
}
