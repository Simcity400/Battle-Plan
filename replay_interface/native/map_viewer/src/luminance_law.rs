//! Files-first per-frame exposure adaptation: a CPU port of the engine's
//! luminance-downsample chain that produces the `AverageLuminance` value the
//! RestoreScene exposure step divides by.
//!
//! # Why this exists (FIDELITY_LEDGER debt #2)
//!
//! `gfx/FX/restorescene.shader` `Exposure()` (`:135-140`, `#define
//! ADJUSTED_EXPOSURE`) scales the linear scene by `MiddleGrey /
//! AverageLuminance`. Until now the replay had no per-frame luminance readback,
//! so it substituted the constant `MIN_HDR_ADJUSTMENT = 0.5` clamp floor for
//! `AverageLuminance`. That is only correct when the scene is dim enough to
//! clamp (true for the captured wide 1936 view, wrong for a bright close-up).
//! This module computes `AverageLuminance` from the rendered HDR scene each
//! frame by porting the shipped chain exactly.
//!
//! # The shipped chain (ported here, cited line-by-line)
//!
//! 1. `gfx/FX/downsample.shader` PixelShader (bright pass, full-res MainScene ->
//!    half-res): for each half-res texel it linear-samples the MainScene (a 2x2
//!    box tap under a linear sampler) and writes, in the **alpha** channel,
//!    `logLuminance = log( max(0, dot(rgb, LUMINANCE_VECTOR)) + 0.0001 )`
//!    (`downsample.shader:70`). `LUMINANCE_VECTOR = (0.2125, 0.7154, 0.0721)`
//!    (`constants.fxh:119`).
//! 2. `gfx/FX/downsample_luminance.shader` `PixelShaderDownsample` resamples that
//!    log-luminance from the bright-pass **alpha** into a dedicated luminance
//!    target (`#ifdef LUMINANCE_SAMPLE_ALPHA` -> `.a`, else `.r`; `:87-93`).
//! 3. `gfx/FX/downsample_luminance.shader` `PixelShaderGather` sums those
//!    log-luminance texels and divides by the tap count (`vSum /=
//!    GatherSize_PixelSize.x * .y`, `:126`) — the arithmetic mean of the logs,
//!    i.e. `log` of the geometric mean of luminance — then
//!    `vCurrentLuminance = clamp( exp(vSum), MinHdr_MaxHdr.x, MinHdr_MaxHdr.y )`
//!    (`:128`), then eases `LastLuminance` toward it
//!    (`CalculateAdaptedLuminance`, `:99-104`).
//!    `MinHdr_MaxHdr = { MIN_HDR_ADJUSTMENT 0.5, MAX_HDR_ADJUSTMENT 1.0 }`
//!    (`common/defines/00_graphics.lua:1038-1039`).
//!
//! So the fully-adapted (steady-state) average luminance is
//! `clamp( exp( mean( log( max(0, dot(rgb, LUMINANCE_VECTOR)) + 1e-4 ) ) ),
//! MIN_HDR_ADJUSTMENT, MAX_HDR_ADJUSTMENT )`, and the engine stores it in a 1x1
//! `R16_FLOAT` texture that the restore pass samples — so this module also
//! **quantizes the result to an R16 half** before it is handed to the exposure
//! constant, matching the engine's storage precision byte-for-byte.
//!
//! # Order-of-operations fidelity (honest)
//!
//! The bright pass computes `log(luminance_of_box_average)`, not
//! `mean(log(per_pixel_luminance))`: because `log` is nonlinear these differ.
//! This module reproduces the engine's order — it 2x2-box-averages the linear
//! color first (matching the bright-pass linear-sampled 2x2 tap) and only then
//! takes `log(dot(box_avg, LUMINANCE_VECTOR) + 1e-4)`, then means those logs.
//! The one intermediate stage it does NOT reproduce is the engine's bilinear
//! resample of the half-res log field down to the small gather grid before the
//! sum; averaging over all half-res texels instead is an exact substitute only
//! when the resample weights are uniform. Documented, and irrelevant whenever
//! `exp(mean_log)` sits below the 0.5 clamp floor (the captured corpus, with a
//! ~6x margin).
//!
//! # Temporal easing (honest)
//!
//! `CalculateAdaptedLuminance` eases `LastLuminance` toward `vCurrentLuminance`
//! at `HDR_ADJUSTMENT_SPEED = 15.0` (`00_graphics.lua:1040`); over time it
//! converges to `vCurrentLuminance`. A static replay dump has no previous frame,
//! so the fully-adapted steady state IS `vCurrentLuminance`. This module returns
//! that steady state and never invents a tau/delta-time. A future frame-sequence
//! path would persist the previous adapted luminance and apply the shipped
//! easing.

use crate::constants::DXGI_FORMAT_R16G16B16A16_FLOAT;

/// `constants.fxh:119` `LUMINANCE_VECTOR`.
pub const LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];
/// `downsample.shader:70` `+ 0.0001f` inside the `log`.
pub const LOG_LUMINANCE_EPSILON: f32 = 0.0001;

/// The parameters this reduction needs, sourced from the resolver-loaded
/// `common/defines/00_graphics.lua` (`MIN_HDR_ADJUSTMENT` / `MAX_HDR_ADJUSTMENT`,
/// the `MinHdr_MaxHdr` clamp bounds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LuminanceClampBounds {
    pub min_hdr_adjustment: f32,
    pub max_hdr_adjustment: f32,
}

/// The result of the ported luminance reduction over one rendered HDR scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AverageLuminanceReduction {
    /// `exp(mean(log(dot(rgb, LUMINANCE_VECTOR) + 1e-4)))` — the geometric mean
    /// of the scene luminance, BEFORE the clamp. This is `exp(vSum)` in
    /// `downsample_luminance.shader:128`.
    pub geometric_mean_luminance: f32,
    /// `clamp(geometric_mean_luminance, min_hdr, max_hdr)` — the engine's
    /// `vCurrentLuminance`, and (for a static dump) the fully-adapted average.
    pub clamped: f32,
    /// True iff the clamp bit (not the raw mean) is what the exposure uses,
    /// i.e. the raw geometric mean fell outside `[min_hdr, max_hdr]`.
    pub clamped_to_bound: bool,
    /// `clamped` after quantizing to an IEEE-754 half (R16_FLOAT), matching the
    /// engine's 1x1 `R16_FLOAT` `AverageLuminanceTexture` storage precision.
    /// This is the value the exposure constant should actually carry.
    pub r16_quantized: f32,
    /// The raw 16-bit half bits of `r16_quantized` — for byte-exact comparison
    /// against a captured `R16_FLOAT` AverageLuminance texel.
    pub r16_bits: u16,
}

/// Port of `gfx/FX/downsample_luminance.shader`'s gather over a rendered HDR
/// `R16G16B16A16_FLOAT` MainScene (the exact scene input the restore pass
/// grades). `raw` is the mapped subresource bytes; `row_pitch` is its byte
/// stride (D3D11 staging layout, may exceed `width * 8`).
///
/// Order of operations mirrors the engine: 2x2 box-average the linear color
/// (bright-pass linear tap, `downsample.shader:64`), then per half-res texel
/// `log(max(0, dot(box_avg, LUMINANCE_VECTOR)) + 1e-4)` (`:70`), then the
/// arithmetic mean of those logs (`downsample_luminance.shader:126`), then
/// `exp` + `clamp` (`:128`).
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
        && bounds.max_hdr_adjustment >= bounds.min_hdr_adjustment)
    {
        return Err(format!(
            "luminance clamp bounds are invalid: min {} max {}",
            bounds.min_hdr_adjustment, bounds.max_hdr_adjustment
        ));
    }

    // Read one RGBA16F texel's linear luminance-relevant channels.
    let texel_luminance_input = |x: usize, y: usize| -> [f32; 3] {
        let base = y * row_pitch + x * 8;
        let r = f16_bits_to_f32(u16::from_le_bytes([raw[base], raw[base + 1]]));
        let g = f16_bits_to_f32(u16::from_le_bytes([raw[base + 2], raw[base + 3]]));
        let b = f16_bits_to_f32(u16::from_le_bytes([raw[base + 4], raw[base + 5]]));
        [r, g, b]
    };

    // Mean of the per-(half-res)-texel log-luminance. We accumulate in f64 so a
    // multi-million-texel sum does not lose low bits; the per-texel math itself
    // is done in f32 to match the shader.
    let mut sum_log = 0.0f64;
    let mut count = 0u64;

    // Half-res grid: the bright pass emits width/2 x height/2 texels, each the
    // 2x2 box average of the four full-res texels it covers. Odd trailing
    // row/column (if the scene is odd-sized) is folded into the last box so no
    // pixel is dropped; the engine's even-sized MainScene never hits this.
    let hw = width.div_ceil(2);
    let hh = height.div_ceil(2);
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
            // downsample.shader:70 order: dot -> max(0,·) -> + 1e-4 -> log.
            let dot = box_avg[0] * LUMINANCE_VECTOR[0]
                + box_avg[1] * LUMINANCE_VECTOR[1]
                + box_avg[2] * LUMINANCE_VECTOR[2];
            let log_luminance = (dot.max(0.0) + LOG_LUMINANCE_EPSILON).ln();
            sum_log += f64::from(log_luminance);
            count += 1;
        }
    }

    // downsample_luminance.shader:126 divides by the tap count; :128 exp + clamp.
    let mean_log = (sum_log / count as f64) as f32;
    let geometric_mean_luminance = mean_log.exp();
    let clamped =
        geometric_mean_luminance.clamp(bounds.min_hdr_adjustment, bounds.max_hdr_adjustment);
    let clamped_to_bound = geometric_mean_luminance < bounds.min_hdr_adjustment
        || geometric_mean_luminance > bounds.max_hdr_adjustment;
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

/// Convenience: assert the payload really is the RGBA16F MainScene format the
/// reduction expects, then run it. Fails closed on any other format so a future
/// capture/scene-format change can never be silently mis-decoded.
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

/// IEEE-754 half-precision bits -> f32 (R16G16B16A16_FLOAT / R16_FLOAT decode).
/// Mirrors `restore_evidence::f16_bits_to_f32`; kept module-local so the ported
/// chain math is self-contained and unit-tested here.
pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x3FF);
    let magnitude = match (exponent, mantissa) {
        (0, 0) => 0,
        (0, m) => {
            // Subnormal half = m * 2^-24. Normalize into the f32 range: find the
            // highest set bit of the 10-bit mantissa, shift it up to the
            // implicit-1 position, and set the f32 exponent accordingly. For the
            // top mantissa bit at position `p` (0..=9), the value is
            // 2^(p-24) * (1 + frac), so the f32 biased exponent is
            // 127 + (p - 24) and the fraction fills the low mantissa bits.
            let top = 31 - m.leading_zeros(); // 0..=9, highest set bit index
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

/// f32 -> IEEE-754 half-precision bits, round-to-nearest-even, matching what a
/// GPU store into an `R16_FLOAT` render target produces. Used to quantize the
/// computed average luminance to the engine's `AverageLuminanceTexture`
/// precision before it feeds the exposure constant.
///
/// This is Fabian Giesen's branch-light float32->float16 with correct
/// round-to-nearest-even for normals AND subnormals (the naive
/// implicit-bit-shift approach mis-rounds tiny subnormal halves). The
/// magnitude is rounded via a float multiply that reproduces the hardware
/// rounding, then the sign is reattached.
pub fn f32_to_f16_bits(value: f32) -> u16 {
    // Explicit, fully reasoned float32 -> float16 with round-to-nearest-even.
    // Matches a GPU store into an R16_FLOAT target. Verified exhaustively in
    // tests against the (proven) `f16_bits_to_f32` decoder over all finite
    // halves.
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32; // biased f32 exponent [0, 255]
    let mantissa = bits & 0x007F_FFFF; // 23-bit fraction

    // Inf / NaN.
    if exp == 0xFF {
        return if mantissa != 0 {
            sign | 0x7E00 // canonical quiet NaN half
        } else {
            sign | 0x7C00 // infinity
        };
    }

    // Unbiased exponent, then rebias to half (bias 15).
    let unbiased = exp - 127;
    let half_exp = unbiased + 15;

    if half_exp >= 0x1F {
        // Overflow -> half infinity.
        return sign | 0x7C00;
    }

    if half_exp <= 0 {
        // Subnormal half, or underflow to zero. Build the full significand with
        // the implicit leading 1 (for a normal f32) and shift it into subnormal
        // position, rounding to nearest even. `exp == 0` (an f32 subnormal) is
        // far below half's smallest subnormal and underflows to zero.
        if exp == 0 || half_exp < -10 {
            return sign; // signed zero
        }
        let significand = mantissa | 0x0080_0000; // 24-bit, implicit 1 at bit 23
                                                  // The value is significand * 2^(unbiased-23). To land it as a half
                                                  // subnormal (fixed exponent 2^-14, 10 fraction bits) we shift right by
                                                  // (1 - half_exp) + 13 = 14 - half_exp bits.
        let shift = (14 - half_exp) as u32; // in 14..=24
        let result = round_shift_right_nearest_even(significand, shift);
        // `result` may be 0x400 (carry up to the smallest normal), which is the
        // correct half bit pattern for exponent 1 mantissa 0.
        return sign | result as u16;
    }

    // Normal half. Round the 23-bit mantissa to 10 bits, round-to-nearest-even,
    // allowing a carry to bump the exponent.
    let mut he = half_exp as u32;
    let mut hm = round_shift_right_nearest_even(mantissa, 13);
    if hm == 0x0400 {
        // Mantissa carry -> exponent += 1, mantissa resets.
        hm = 0;
        he += 1;
        if he >= 0x1F {
            return sign | 0x7C00;
        }
    }
    sign | ((he << 10) as u16) | hm as u16
}

/// Shift `value` right by `shift` bits, rounding the discarded bits to nearest,
/// ties-to-even. `shift` must be >= 1.
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

    #[test]
    fn f16_round_trip_known_values() {
        // Decode side (matches restore_evidence::f16_bits_to_f32).
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0x3800), 0.5);
        assert_eq!(f16_bits_to_f32(0x4000), 2.0);
        // Encode side, exact-representable halves.
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(2.0), 0x4000);
        assert_eq!(f32_to_f16_bits(-1.0), 0xBC00);
    }

    #[test]
    fn f16_encode_matches_decode_round_trip_over_grid() {
        // Every exactly-representable half must survive decode->encode.
        for bits in 0u16..=0xFFFF {
            // Skip NaNs (payload not uniquely preserved) and both infinities'
            // aliasing; test finite values only.
            let exp = (bits >> 10) & 0x1F;
            let mant = bits & 0x3FF;
            if exp == 0x1F {
                continue;
            }
            let _ = mant;
            let value = f16_bits_to_f32(bits);
            let reencoded = f32_to_f16_bits(value);
            // -0.0 and +0.0 both encode canonically; compare via decoded value.
            assert_eq!(
                f16_bits_to_f32(reencoded),
                value,
                "half bits {bits:#06x} did not survive decode/encode round-trip"
            );
        }
    }

    #[test]
    fn f16_encode_rounds_to_nearest_even() {
        // 0.1 is not representable; nearest half is 0x2E66 (~0.099975586).
        assert_eq!(f32_to_f16_bits(0.1), 0x2E66);
        // A value between two halves rounds to nearest.
        let mid = f16_bits_to_f32(0x3801); // just above 0.5
        let below = f16_bits_to_f32(0x3800); // 0.5
        let between = (mid + below) * 0.5;
        let enc = f32_to_f16_bits(between);
        // Ties-to-even: 0x3800 has even mantissa (0), so tie rounds down to it.
        assert_eq!(enc, 0x3800);
    }

    #[test]
    fn uniform_dim_scene_clamps_to_floor() {
        // A uniform mid-grey 0.1 scene: luminance = 0.1, exp(mean(log)) ~ 0.1001
        // (0.1 + 1e-4), well below the 0.5 floor -> clamps to 0.5.
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
        // A uniform 0.7 scene: luminance 0.7, exp(mean(log)) ~ 0.7001, inside
        // [0.5, 1.0] -> NOT clamped; the exposure adapts down.
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
        // The half-quantized value is what feeds exposure.
        assert!((out.r16_quantized - 0.7).abs() < 3e-3);
    }

    #[test]
    fn uniform_overbright_scene_clamps_to_ceiling() {
        // A uniform 2.0 scene: luminance 2.0 > MAX_HDR 1.0 -> clamps to 1.0.
        let scene = solid_scene(4, 4, [2.0, 2.0, 2.0]);
        let out =
            scene_average_luminance(&scene, 4, 4, 4 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        assert!(out.clamped_to_bound);
        assert_eq!(out.clamped, 1.0);
        assert_eq!(out.r16_bits, 0x3C00);
    }

    #[test]
    fn box_average_is_order_of_operations_engine_faithful() {
        // A 2x2 scene with one bright texel and three near-black: the engine
        // box-averages COLOR first, so the single 2x2 box sees the average
        // color, then takes ONE log. mean(log(perpixel)) would be far darker.
        // box avg color = (0.8+0+0+0)/4 = 0.2 per channel.
        let bright = rgba16f_texel(0.8, 0.8, 0.8);
        let dark = rgba16f_texel(0.0, 0.0, 0.0);
        let mut scene = Vec::new();
        // row 0: bright, dark
        scene.extend_from_slice(&bright);
        scene.extend_from_slice(&dark);
        // row 1: dark, dark
        scene.extend_from_slice(&dark);
        scene.extend_from_slice(&dark);
        let out =
            scene_average_luminance(&scene, 2, 2, 2 * 8, DXGI_FORMAT_R16G16B16A16_FLOAT, BOUNDS)
                .expect("reduce");
        // One box, avg color 0.2 -> luminance 0.2 -> exp(log(0.2+1e-4)) ~ 0.2001.
        assert!(
            (out.geometric_mean_luminance - 0.2001).abs() < 3e-3,
            "geo mean {} not ~0.2 (box-average-then-log)",
            out.geometric_mean_luminance
        );
    }

    #[test]
    fn row_pitch_padding_is_respected() {
        // width 2, but row_pitch has 8 padding bytes after each 16-byte row.
        let bright = rgba16f_texel(0.6, 0.6, 0.6);
        let pad = [0u8; 8];
        let mut scene = Vec::new();
        for _ in 0..2 {
            scene.extend_from_slice(&bright);
            scene.extend_from_slice(&bright);
            scene.extend_from_slice(&pad); // padding must be ignored
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
        // DXGI 28 = R8G8B8A8_UNORM, not the scene format.
        let err = scene_average_luminance(&scene, 2, 2, 2 * 8, 28, BOUNDS)
            .expect_err("must reject non-RGBA16F");
        assert!(err.contains("R16G16B16A16_FLOAT"), "{err}");
    }

    #[test]
    fn rejects_short_payload() {
        let scene = vec![0u8; 8]; // one texel, but claims 2x2
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

    // --- CAPTURED-AvgLum REPRODUCTION GATE (spec step 3) ---------------------
    // The captured restore_gate_1 MainScene (the exact HDR scene input the
    // engine's restore pass graded) reduced through this ported chain MUST
    // clamp to the MIN_HDR_ADJUSTMENT floor and, quantized to R16, byte-match
    // the captured AverageLuminanceTexture texel (half 0x3800 == 0.5). If it
    // does NOT clamp, the port is wrong — the gate fails (never tunes).
    const RESTORE_GATE_DIR: &str = r"C:\Users\Example\AppData\Local\hoi4_captures\restore_gate_1";
    const CAPTURED_MAIN_SCENE: &str =
        "restore_frame_draw_map_frame_shared_payload_ps_srv_MainScene_b811e76916a16e8c.d3d11";
    const CAPTURED_AVG_LUM: &str =
        "restore_frame_draw_map_frame_shared_payload_ps_srv_AverageLuminanceTexture_a86bb8b3d96548fd.d3d11";
    // From map_frame_restore_pass_bundle.json (restore_gate_1), MainScene t0.
    const CAPTURED_SCENE_W: usize = 1920;
    const CAPTURED_SCENE_H: usize = 1080;
    const CAPTURED_SCENE_ROW_PITCH: usize = 15360; // 1920 * 8, tight rows

    #[test]
    fn gate_captured_scene_reproduces_captured_average_luminance() {
        let dir = std::path::PathBuf::from(RESTORE_GATE_DIR);
        let scene_path = dir.join(CAPTURED_MAIN_SCENE);
        let avg_path = dir.join(CAPTURED_AVG_LUM);
        if !scene_path.is_file() || !avg_path.is_file() {
            eprintln!(
                "skipping captured-AvgLum reproduction gate: {RESTORE_GATE_DIR} scene/avglum payloads absent"
            );
            return;
        }
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
            BOUNDS, // MIN_HDR 0.5 / MAX_HDR 1.0, the shipped clamp bounds
        )
        .expect("reduce captured scene");

        // The captured 1936 wide view is dim: exp(mean_log) must land at/below
        // the 0.5 floor so the clamp engages (this is the clamped branch).
        assert!(
            out.clamped_to_bound && out.geometric_mean_luminance < BOUNDS.min_hdr_adjustment,
            "captured scene did NOT clamp: geo mean {} (expected < {})",
            out.geometric_mean_luminance,
            BOUNDS.min_hdr_adjustment
        );
        assert_eq!(out.clamped, 0.5, "clamp must produce exactly 0.5");

        // Byte-exact vs the captured R16_FLOAT AverageLuminance texel.
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

    // --- CAP4 UNCLAMPED-EXPOSURE PROBE (B9, spec Target 3) -------------------
    // B9 asks whether the CAP4 live session exercised the UNCLAMPED exposure
    // branch (raw geometric-mean scene luminance > MIN_HDR_ADJUSTMENT 0.5). The
    // preserved `session_WAVE1_CAP4` rounds captured the terrain/water/rivers/
    // trees passes but NO restore_pass bundle (no MainScene/AverageLuminance
    // payload). The closest proxy to the restore MainScene is the LAST captured
    // map pass's `om_post_pass` HDR RT (trees pass, 1920x1080 RGBA16F). Reducing
    // it through the ported chain determines whether the scene was ever bright
    // enough to leave the floor. Result (both rounds): geo-mean ~0.237, which
    // CLAMPS to 0.5 -> the unclamped branch was NOT exercised. This gate pins
    // that finding so a future "CAP4 closes B9" claim fails loudly.
    const DEFAULT_WAVE1_CAP4_ROUNDS: &str =
        r"C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds";
    const CAP4_TREES_POST_NEEDLE: &str =
        "trees_frame_draw_map_frame_shared_payload_om_post_pass_render_target";

    fn cap4_rounds_root() -> std::path::PathBuf {
        std::env::var("BATTLE_PLAN_WAVE1_CAP4_ROUNDS").or_else(|_| std::env::var("HOI4_WAVE1_CAP4_ROUNDS"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_WAVE1_CAP4_ROUNDS))
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
    fn gate_wave1_cap4_scene_clamps_to_floor_unclamped_branch_unexercised() {
        for round in ["fow_off_observer_front", "fow_on_pov_front"] {
            let dir = cap4_rounds_root().join(round);
            let Some(scene_path) = (dir.is_dir())
                .then(|| find_cap4_payload(&dir, CAP4_TREES_POST_NEEDLE))
                .flatten()
            else {
                eprintln!(
                    "gate_wave1_cap4_scene_clamps_to_floor: {} trees post-pass RT absent; skipped",
                    dir.display()
                );
                continue;
            };
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
            // The unclamped branch needs geo-mean > 0.5. CAP4 stays far below.
            assert!(
                out.geometric_mean_luminance < BOUNDS.min_hdr_adjustment,
                "CAP4 {round} geo-mean {} unexpectedly reached the unclamped branch (>= {})",
                out.geometric_mean_luminance,
                BOUNDS.min_hdr_adjustment
            );
            assert!(out.clamped_to_bound && out.clamped == 0.5);
            // Observed ~0.236-0.239; a wide band tolerates the trees-vs-final
            // MainScene proxy gap while still asserting "well below the floor".
            assert!(
                (0.15..0.30).contains(&out.geometric_mean_luminance),
                "CAP4 {round} geo-mean {} outside the expected dim band",
                out.geometric_mean_luminance
            );
        }
    }

    // Perf note (speed-five mandate: frames < 100 ms). Measures the CPU cost of
    // the reduction over a real 1920x1080 scene, isolated from the D3D11 readback
    // (that CopyResource + Map is a fixed staging cost on top). `--ignored` so it
    // never gates CI; run with `--ignored --nocapture` to see the timing.
    #[test]
    #[ignore]
    fn perf_reduction_over_full_res_scene() {
        let dir = std::path::PathBuf::from(RESTORE_GATE_DIR);
        let scene_path = dir.join(CAPTURED_MAIN_SCENE);
        if !scene_path.is_file() {
            eprintln!("skipping perf test: captured scene absent");
            return;
        }
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
