//! CPU mirror of the shipped map bloom chain, derived from the installed content and
//! validated against the captured MainScene->RestoreBloom pair.
//!
//! Source law (files-first; every stage is cited in
//! `documents/BLOOM_LAW.md`):
//!   * `gfx/FX/downsample.shader`  — bright-pass + 2x downsample.
//!   * `gfx/FX/bloom.shader`       — separable linear-sampled Gaussian blur.
//!   * `gfx/FX/posteffect_base.fxh`— PostEffectBase constant layout.
//!   * `gfx/posteffect_volumes.txt`— the `standard` volume that supplies the
//!     bloom parameters actually in effect (BLOOM_WIDTH/BLOOM_SCALE/
//!     BRIGHT_THRESHOLD/tonemap_middlegrey), falling back to
//!     `common/defines/00_graphics.lua`.
//!
//! The blur weights/offsets are NOT stored as literals in any shipped text
//! file — they are the engine's linear-sampled Gaussian, computed here from
//! `BLOOM_WIDTH` (sigma) and `BLOOM_SCALE` (per-pass gain). The derivation
//! reproduces the captured `bloom.shader` ConstantBuffer(2,39) to 7 significant
//! figures, so it stays correct for mods that retune those two constants.
//!
//! This module is deterministic and dependency-free so evidence checks run the
//! same math a GPU port would.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::fs;

/// gfx/FX/constants.fxh:119 — LUMINANCE_VECTOR, used by the bright pass to
/// write log-luminance into alpha (we keep it for a faithful alpha channel even
/// though the blur discards alpha).
pub const BLOOM_LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];

/// Number of symmetric tap pairs in `bloom.shader` (Weights[0..3] / Offsets[0..3]).
const BLOOM_TAP_PAIRS: usize = 4;
/// Discrete Gaussian radius folded into the linear-sampled taps. Pairs cover
/// discrete taps (1,2),(3,4),(5,6),(7,8), so the effective radius is 8.
const BLOOM_DISCRETE_RADIUS: usize = 8;

/// Parameters that drive the shipped bloom chain, resolved from installed content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomParams {
    /// `BLOOM_WIDTH` — the Gaussian sigma (in bloom-buffer texels).
    pub bloom_width: f32,
    /// `BLOOM_SCALE` — the per-pass gain; the two passes multiply to
    /// `bloom_scale^2`.
    pub bloom_scale: f32,
    /// `BRIGHT_THRESHOLD` — the bright-pass knee in the downsample shader.
    pub bright_threshold: f32,
    /// `EMISSIVE_BLOOM_STRENGTH` — additive emissive-alpha contribution in the
    /// bright pass.
    pub emissive_bloom_strength: f32,
}

impl BloomParams {
    /// Validate a fully resolved bloom state before either kernel derivation or
    /// production upload. Public because replay_app composes the per-volume
    /// fields while map_viewer owns the bloom law and its fail-closed contract.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("BLOOM_WIDTH", self.bloom_width),
            ("BLOOM_SCALE", self.bloom_scale),
            ("BRIGHT_THRESHOLD", self.bright_threshold),
            ("EMISSIVE_BLOOM_STRENGTH", self.emissive_bloom_strength),
        ] {
            if !value.is_finite() {
                return Err(format!("bloom chain blocked: {name} is not finite ({value})").into());
            }
        }
        if self.bloom_width <= 0.0 {
            return Err(format!(
                "bloom chain blocked: BLOOM_WIDTH must be > 0, got {}",
                self.bloom_width
            )
            .into());
        }
        if self.bloom_scale < 0.0 {
            return Err(format!(
                "bloom chain blocked: BLOOM_SCALE must be >= 0, got {}",
                self.bloom_scale
            )
            .into());
        }
        Ok(())
    }
}

/// The separable blur kernel, matching `bloom.shader`'s cbuffer(2,39): a center
/// tap plus four symmetric linear-sampled tap pairs, scaled so each pass sums to
/// `BLOOM_SCALE`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomBlurKernel {
    /// `Weight0` — the center-tap weight.
    pub weight0: f32,
    /// `Weights[0..3]` — per-pair combined weight.
    pub weights: [f32; BLOOM_TAP_PAIRS],
    /// `Offsets[0..3]` — per-pair fractional sample offset (in texels).
    pub offsets: [f32; BLOOM_TAP_PAIRS],
}

impl BloomBlurKernel {
    /// Derive the linear-sampled Gaussian for the given `sigma` (BLOOM_WIDTH)
    /// and per-pass `scale` (BLOOM_SCALE), exactly as the engine does:
    ///   * discrete Gaussian weights `w(x) = exp(-x^2 / (2 sigma^2))` for
    ///     x in 0..=8, normalized over the full symmetric 17-tap kernel;
    ///   * fold each adjacent discrete pair (1,2),(3,4),(5,6),(7,8) into one
    ///     bilinear sample: combined weight `w_a + w_b`, offset
    ///     `(a*w_a + b*w_b)/(w_a + w_b)`;
    ///   * multiply every weight by `scale` so the per-pass sum is `scale`.
    ///
    /// Reproduces the captured `bloom.shader` cbuffer to 7 sig figs.
    pub fn from_gaussian(sigma: f32, scale: f32) -> Result<Self> {
        if !(sigma.is_finite() && sigma > 0.0) {
            return Err(
                format!("bloom blur kernel blocked: sigma must be > 0, got {sigma}").into(),
            );
        }
        if !(scale.is_finite() && scale >= 0.0) {
            return Err(
                format!("bloom blur kernel blocked: scale must be >= 0, got {scale}").into(),
            );
        }
        // Discrete Gaussian taps 0..=BLOOM_DISCRETE_RADIUS (double precision to
        // match the engine's host-side computation, then stored as f32).
        let sigma = f64::from(sigma);
        let mut taps = [0.0f64; BLOOM_DISCRETE_RADIUS + 1];
        for (x, tap) in taps.iter_mut().enumerate() {
            let xf = x as f64;
            *tap = (-(xf * xf) / (2.0 * sigma * sigma)).exp();
        }
        // Full 17-tap normalization: tap0 + 2*sum(tap1..=8).
        let total: f64 = taps[0] + 2.0 * taps[1..].iter().sum::<f64>();
        if !(total.is_finite() && total > 0.0) {
            return Err(
                "bloom blur kernel blocked: Gaussian normalization sum is not positive/finite"
                    .into(),
            );
        }
        let scale64 = f64::from(scale);
        let weight0 = (taps[0] / total) * scale64;
        let mut weights = [0.0f32; BLOOM_TAP_PAIRS];
        let mut offsets = [0.0f32; BLOOM_TAP_PAIRS];
        for pair in 0..BLOOM_TAP_PAIRS {
            let a = 2 * pair + 1; // discrete offsets 1,3,5,7
            let b = 2 * pair + 2; // discrete offsets 2,4,6,8
            let wa = taps[a] / total;
            let wb = taps[b] / total;
            let combined = wa + wb;
            if !(combined.is_finite() && combined > 0.0) {
                return Err(format!(
                    "bloom blur kernel blocked: tap pair {pair} combined weight is not positive"
                )
                .into());
            }
            let offset = (a as f64 * wa + b as f64 * wb) / combined;
            weights[pair] = (combined * scale64) as f32;
            offsets[pair] = offset as f32;
        }
        Ok(Self {
            weight0: weight0 as f32,
            weights,
            offsets,
        })
    }

    /// Per-pass weight sum (`Weight0 + 2*sum(Weights)`), which equals
    /// `BLOOM_SCALE` by construction.
    pub fn pass_weight_sum(&self) -> f32 {
        self.weight0 + 2.0 * self.weights.iter().sum::<f32>()
    }
}

/// An RGBA half-float image, decoded to f32 and stored row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct BloomImageRgba {
    pub width: usize,
    pub height: usize,
    /// `width*height` RGBA texels.
    pub texels: Vec<[f32; 4]>,
}

impl BloomImageRgba {
    pub fn new(width: usize, height: usize, texels: Vec<[f32; 4]>) -> Result<Self> {
        let expected = width
            .checked_mul(height)
            .ok_or("bloom image blocked: width*height overflow")?;
        if texels.len() != expected {
            return Err(format!(
                "bloom image blocked: expected {expected} texels for {width}x{height}, got {}",
                texels.len()
            )
            .into());
        }
        Ok(Self {
            width,
            height,
            texels,
        })
    }

    /// Decode a tightly-packed `R16G16B16A16_FLOAT` payload (DXGI format 10)
    /// with the given row pitch in bytes. Fails closed on any size mismatch or
    /// non-finite texel.
    pub fn from_r16g16b16a16_float(
        bytes: &[u8],
        width: usize,
        height: usize,
        row_pitch_bytes: usize,
    ) -> Result<Self> {
        let bytes_per_texel = 8usize; // 4 channels * 2 bytes
        let min_row = width
            .checked_mul(bytes_per_texel)
            .ok_or("bloom image blocked: row byte width overflow")?;
        if row_pitch_bytes < min_row {
            return Err(format!(
                "bloom image blocked: row_pitch {row_pitch_bytes} is smaller than {min_row} bytes for width {width}"
            )
            .into());
        }
        let needed = row_pitch_bytes
            .checked_mul(height)
            .ok_or("bloom image blocked: payload byte count overflow")?;
        if bytes.len() < needed {
            return Err(format!(
                "bloom image blocked: payload has {} bytes, need at least {needed} for {width}x{height} @ pitch {row_pitch_bytes}",
                bytes.len()
            )
            .into());
        }
        let mut texels = Vec::with_capacity(width * height);
        for row in 0..height {
            let row_start = row * row_pitch_bytes;
            for col in 0..width {
                let base = row_start + col * bytes_per_texel;
                let mut texel = [0.0f32; 4];
                for (channel, slot) in texel.iter_mut().enumerate() {
                    let off = base + channel * 2;
                    let bits = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
                    let value = half_float_to_f32(bits);
                    if !value.is_finite() {
                        return Err(format!(
                            "bloom image blocked: non-finite half-float texel at row {row} col {col} channel {channel}"
                        )
                        .into());
                    }
                    *slot = value;
                }
                texels.push(texel);
            }
        }
        Self::new(width, height, texels)
    }

    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }
}

/// Full CPU bloom chain output plus the derived kernel, for reporting.
#[derive(Clone, Debug)]
pub struct BloomChainOutput {
    pub params: BloomParams,
    pub kernel: BloomBlurKernel,
    /// The bright-pass / downsample output (half the input size), RGB only in
    /// the color channels; alpha carries the shader's log-luminance.
    pub bright_pass: BloomImageRgba,
    /// The final bloom buffer after horizontal+vertical blur. Alpha is forced to
    /// 1.0, matching `bloom.shader`'s `return float4(color, 1.0)`.
    pub bloom: BloomImageRgba,
}

/// The posteffect volume that supplies the day/base bloom parameters and is the
/// `inherit` root for the other volumes. It matches the captured PostEffectBase
/// CB for `restore_gate_1`. Other volumes (e.g. `green_night`, `medither_night`)
/// carry their own active bloom overrides, so the render-wiring job that knows
/// the frame's active volume should pass its name to
/// [`resolve_bloom_params_for_volume`]; this default is correct for the standard
/// map view.
pub const BLOOM_DEFAULT_VOLUME: &str = "standard";

/// Resolve the shipped bloom parameters for [`BLOOM_DEFAULT_VOLUME`] through the
/// mod-aware resolver. See [`resolve_bloom_params_for_volume`].
pub fn resolve_bloom_params(resolver: &GameFileResolver) -> Result<BloomParams> {
    resolve_bloom_params_for_volume(resolver, BLOOM_DEFAULT_VOLUME)
}

/// Resolve the shipped bloom parameters for a named posteffect volume through
/// the mod-aware resolver.
///
/// Primary source is `gfx/posteffect_volumes.txt` (`volume_name`, with its
/// `inherit` chain resolved so a child overrides its parent).
/// `EMISSIVE_BLOOM_STRENGTH` is not part of the volume schema, so it comes from
/// `common/defines/00_graphics.lua`. If the volume omits a bloom key, the define
/// file is the fail-closed fallback.
pub fn resolve_bloom_params_for_volume(
    resolver: &GameFileResolver,
    volume_name: &str,
) -> Result<BloomParams> {
    let volumes_path =
        resolver.resolve_required("gfx/posteffect_volumes.txt", "posteffect volumes")?;
    let volumes_src = fs::read_to_string(&volumes_path).map_err(|err| {
        format!(
            "bloom chain blocked: {} is not readable: {err}",
            volumes_path.display()
        )
    })?;
    let volume = parse_posteffect_volume(&volumes_src, volume_name)?;

    let defines_path =
        resolver.resolve_required("common/defines/00_graphics.lua", "graphics defines")?;
    let defines_src = fs::read_to_string(&defines_path).map_err(|err| {
        format!(
            "bloom chain blocked: {} is not readable: {err}",
            defines_path.display()
        )
    })?;

    let bloom_width = volume
        .get("BLOOM_WIDTH")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| parse_define_number(&defines_src, "BLOOM_WIDTH"))?;
    let bloom_scale = volume
        .get("BLOOM_SCALE")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| parse_define_number(&defines_src, "BLOOM_SCALE"))?;
    let bright_threshold = volume
        .get("BRIGHT_THRESHOLD")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| parse_define_number(&defines_src, "BRIGHT_THRESHOLD"))?;
    // Emissive strength is only in the defines file.
    let emissive_bloom_strength = parse_define_number(&defines_src, "EMISSIVE_BLOOM_STRENGTH")?;

    let params = BloomParams {
        bloom_width,
        bloom_scale,
        bright_threshold,
        emissive_bloom_strength,
    };
    params.validate()?;
    Ok(params)
}

/// Parse a `KEY = value` numeric assignment from a Lua/Clausewitz-style source,
/// tolerating trailing commas and `--` comments. Fails closed if absent or
/// non-numeric.
pub fn parse_define_number(source: &str, key: &str) -> Result<f32> {
    let mut found = None;
    for (line_index, line) in source.lines().enumerate() {
        let code = line.split("--").next().unwrap_or(line).trim();
        // Match the key as a whole token followed by '='.
        let Some(rest) = code.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..]
            .trim_start()
            .trim_end_matches(',')
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("bloom chain blocked: {key} has no value"))?
            .trim_end_matches(',')
            .parse::<f32>()
            .map_err(|err| {
                format!(
                    "bloom chain blocked: {key} on line {} is not a number: {err}",
                    line_index + 1
                )
            })?;
        if found.replace(value).is_some() {
            return Err(format!("bloom chain blocked: {key} is defined more than once").into());
        }
    }
    found.ok_or_else(|| format!("bloom chain blocked: source is missing {key}").into())
}

/// Parse a single `posteffect_values = { name = <target> ... }` block from
/// `posteffect_volumes.txt`, returning its bloom-relevant numeric keys with
/// `inherit` resolved (the child overrides the parent). Only active (non-`#`
/// commented) lines contribute.
fn parse_posteffect_volume(
    source: &str,
    target: &str,
) -> Result<std::collections::BTreeMap<String, f32>> {
    let blocks = split_posteffect_blocks(source)?;
    let mut chain = Vec::new();
    let mut name = target.to_string();
    let mut guard = 0;
    loop {
        let block = blocks.get(&name).ok_or_else(|| {
            format!("bloom chain blocked: posteffect_volumes.txt has no volume named {name}")
        })?;
        chain.push(block);
        match &block.inherit {
            Some(parent) => name = parent.clone(),
            None => break,
        }
        guard += 1;
        if guard > blocks.len() {
            return Err(
                "bloom chain blocked: posteffect_volumes.txt inherit chain is cyclic".into(),
            );
        }
    }
    // Apply from the root of the inherit chain down to the target so children win.
    let mut merged = std::collections::BTreeMap::new();
    for block in chain.iter().rev() {
        for (key, value) in &block.values {
            merged.insert(key.clone(), *value);
        }
    }
    Ok(merged)
}

#[derive(Default)]
struct PostEffectBlock {
    inherit: Option<String>,
    values: std::collections::BTreeMap<String, f32>,
}

/// Split the file into named `posteffect_values` blocks. The parser only reads
/// `name`, `inherit`, and the numeric bloom keys; unknown keys are ignored.
fn split_posteffect_blocks(
    source: &str,
) -> Result<std::collections::BTreeMap<String, PostEffectBlock>> {
    const BLOOM_KEYS: [&str; 3] = ["BLOOM_WIDTH", "BLOOM_SCALE", "BRIGHT_THRESHOLD"];
    let mut blocks = std::collections::BTreeMap::new();
    let mut in_block = false;
    let mut current_name: Option<String> = None;
    let mut current = PostEffectBlock::default();
    for raw in source.lines() {
        // Strip `#` comments (Clausewitz comment marker in this file).
        let line = raw.split('#').next().unwrap_or(raw).trim();
        if line.is_empty() {
            continue;
        }
        if !in_block {
            if line.starts_with("posteffect_values") && line.contains('{') {
                in_block = true;
                current = PostEffectBlock::default();
                current_name = None;
            }
            continue;
        }
        // Per-line brace accounting on the comment-stripped line (same
        // convention as parse_posteffect_volume_table in grading_law.rs).
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if opens > closes {
            // A nested sub-block would silently swallow every bloom key after
            // it under the old `contains('}')` close rule; refuse instead.
            return Err(format!(
                "bloom chain blocked: posteffect_volumes.txt volume {} contains a nested block, \
                 which this parser does not support",
                current_name.as_deref().unwrap_or("<unnamed>")
            )
            .into());
        }
        if closes > opens {
            // Close the block; require a name.
            let name = current_name
                .take()
                .ok_or("bloom chain blocked: posteffect_volumes.txt block is missing a name")?;
            blocks.insert(name, std::mem::take(&mut current));
            in_block = false;
            continue;
        }
        // opens == closes: either a plain key/value line or an inline
        // net-zero brace line (`foo = { 1 2 3 }`) — the latter is not a block
        // terminator. `name`/`inherit` only make sense on brace-free lines; a
        // bloom key with a brace value still fails closed below via the
        // not-a-number parse error.
        if opens == 0 {
            if let Some(value) = strip_kv(line, "name") {
                current_name = Some(value.to_string());
                continue;
            }
            if let Some(value) = strip_kv(line, "inherit") {
                current.inherit = Some(value.to_string());
                continue;
            }
        }
        for key in BLOOM_KEYS {
            if let Some(value) = strip_kv(line, key) {
                let parsed = value.trim_end_matches(',').parse::<f32>().map_err(|err| {
                    format!(
                        "bloom chain blocked: posteffect_volumes.txt {key} is not a number: {err}"
                    )
                })?;
                current.values.insert(key.to_string(), parsed);
            }
        }
    }
    if in_block {
        return Err(
            "bloom chain blocked: posteffect_volumes.txt has an unterminated posteffect_values block"
                .into(),
        );
    }
    Ok(blocks)
}

/// Parse `key = value` (whitespace tolerant), returning the value token.
fn strip_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let value = rest.trim().split_whitespace().next()?;
    Some(value)
}

/// Run the full file-derived bloom chain over an `R16G16B16A16_FLOAT` MainScene.
///
/// The output bloom is `main.width/2 x main.height/2` — the shipped downsample
/// factor. Fails closed if the input dimensions are not even.
pub fn run_bloom_chain(
    main_scene: &BloomImageRgba,
    params: BloomParams,
) -> Result<BloomChainOutput> {
    params.validate()?;
    if main_scene.width < 2 || main_scene.height < 2 {
        return Err(format!(
            "bloom chain blocked: MainScene {}x{} is too small to downsample",
            main_scene.width, main_scene.height
        )
        .into());
    }
    if main_scene.width % 2 != 0 || main_scene.height % 2 != 0 {
        return Err(format!(
            "bloom chain blocked: MainScene {}x{} is not an even size for the 2x downsample",
            main_scene.width, main_scene.height
        )
        .into());
    }
    let kernel = BloomBlurKernel::from_gaussian(params.bloom_width, params.bloom_scale)?;
    let bright_pass = bright_pass_downsample(main_scene, params)?;
    // bloom.shader: horizontal pass (Axis=1) then vertical pass (Axis=0), each
    // sampling the previous half-float render target. We round to half-float
    // between passes to mirror the engine's R16G16B16A16_FLOAT intermediate.
    let horizontal = blur_pass(&bright_pass, &kernel, BlurAxis::Horizontal);
    let horizontal = quantize_half(&horizontal);
    let mut bloom = blur_pass(&horizontal, &kernel, BlurAxis::Vertical);
    // bloom.shader returns float4(color, 1.0).
    for texel in &mut bloom.texels {
        texel[3] = 1.0;
    }
    Ok(BloomChainOutput {
        params,
        kernel,
        bright_pass,
        bloom,
    })
}

/// gfx/FX/downsample.shader:62-73. Bilinear 2x reduction (exact-2x hardware
/// bilinear == 2x2 box average) followed by the bright-pass knee. Alpha carries
/// the shader's `logLuminance`.
fn bright_pass_downsample(main: &BloomImageRgba, params: BloomParams) -> Result<BloomImageRgba> {
    let out_w = main.width / 2;
    let out_h = main.height / 2;
    let mut texels = Vec::with_capacity(out_w * out_h);
    for y in 0..out_h {
        for x in 0..out_w {
            // 2x2 box average of the four source texels (hardware bilinear at
            // an exact 2x reduction lands exactly on the shared corner).
            let sx = x * 2;
            let sy = y * 2;
            let c00 = main.texels[main.index(sx, sy)];
            let c10 = main.texels[main.index(sx + 1, sy)];
            let c01 = main.texels[main.index(sx, sy + 1)];
            let c11 = main.texels[main.index(sx + 1, sy + 1)];
            let mut avg = [0.0f32; 4];
            for channel in 0..4 {
                avg[channel] = 0.25 * (c00[channel] + c10[channel] + c01[channel] + c11[channel]);
            }
            let (r, g, b, a) = (avg[0], avg[1], avg[2], avg[3]);
            // vMax = max(0, max(max(r,g),b) - BrightThreshold)
            let mut vmax = (r.max(g).max(b) - params.bright_threshold).max(0.0);
            // vMax /= (0.5 + vMax)
            vmax /= 0.5 + vmax;
            // vMax += a * EmissiveBloomStrength
            vmax += a * params.emissive_bloom_strength;
            let color = [r * vmax, g * vmax, b * vmax];
            // logLuminance = log(max(0, dot(rgb, LUMINANCE_VECTOR)) + 1e-4)
            let lum = (color_dot(avg, BLOOM_LUMINANCE_VECTOR)).max(0.0);
            let log_luminance = (lum + 0.0001).ln();
            texels.push([color[0], color[1], color[2], log_luminance]);
        }
    }
    BloomImageRgba::new(out_w, out_h, texels)
}

fn color_dot(rgba: [f32; 4], v: [f32; 3]) -> f32 {
    rgba[0] * v[0] + rgba[1] * v[1] + rgba[2] * v[2]
}

#[derive(Clone, Copy)]
enum BlurAxis {
    Horizontal,
    Vertical,
}

/// gfx/FX/bloom.shader PixelShader: center tap plus four symmetric linear-sampled
/// tap pairs. Sampling uses hardware bilinear with Clamp addressing (edge
/// replicate), which for fractional offsets interpolates between adjacent
/// texels.
fn blur_pass(src: &BloomImageRgba, kernel: &BloomBlurKernel, axis: BlurAxis) -> BloomImageRgba {
    let mut texels = Vec::with_capacity(src.width * src.height);
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = [0.0f32; 4];
            let center = src.texels[src.index(x, y)];
            for channel in 0..4 {
                acc[channel] = kernel.weight0 * center[channel];
            }
            for pair in 0..BLOOM_TAP_PAIRS {
                let offset = kernel.offsets[pair];
                let weight = kernel.weights[pair];
                let plus = sample_bilinear_axis(src, x, y, offset, axis);
                let minus = sample_bilinear_axis(src, x, y, -offset, axis);
                for channel in 0..4 {
                    acc[channel] += weight * (plus[channel] + minus[channel]);
                }
            }
            texels.push(acc);
        }
    }
    BloomImageRgba {
        width: src.width,
        height: src.height,
        texels,
    }
}

/// Bilinear sample offset by `offset` texels along `axis`, Clamp addressing.
/// The offset is a texel-space displacement of the sample center; we interpolate
/// between the two straddling texels, matching hardware linear filtering.
fn sample_bilinear_axis(
    src: &BloomImageRgba,
    x: usize,
    y: usize,
    offset: f32,
    axis: BlurAxis,
) -> [f32; 4] {
    let (coord, extent) = match axis {
        BlurAxis::Horizontal => (x as f32, src.width),
        BlurAxis::Vertical => (y as f32, src.height),
    };
    let sample = coord + offset;
    let base = sample.floor();
    let frac = sample - base;
    let i0 = clamp_index(base, extent);
    let i1 = clamp_index(base + 1.0, extent);
    let (t0, t1) = match axis {
        BlurAxis::Horizontal => (src.texels[src.index(i0, y)], src.texels[src.index(i1, y)]),
        BlurAxis::Vertical => (src.texels[src.index(x, i0)], src.texels[src.index(x, i1)]),
    };
    let mut out = [0.0f32; 4];
    for channel in 0..4 {
        out[channel] = t0[channel] + (t1[channel] - t0[channel]) * frac;
    }
    out
}

fn clamp_index(value: f32, extent: usize) -> usize {
    if value < 0.0 {
        0
    } else {
        let idx = value as usize;
        idx.min(extent - 1)
    }
}

/// Round every channel through `R16G16B16A16_FLOAT` to mirror the engine's
/// half-float render-target write between blur passes.
fn quantize_half(image: &BloomImageRgba) -> BloomImageRgba {
    let texels = image
        .texels
        .iter()
        .map(|texel| {
            let mut out = [0.0f32; 4];
            for channel in 0..4 {
                out[channel] = half_float_to_f32(f32_to_half_float(texel[channel]));
            }
            out
        })
        .collect();
    BloomImageRgba {
        width: image.width,
        height: image.height,
        texels,
    }
}

/// IEEE-754 half-float bit pattern to f32.
pub fn half_float_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 => {
            if fraction == 0 {
                sign * 0.0
            } else {
                sign * 2.0f32.powi(-14) * (f32::from(fraction) / 1024.0)
            }
        }
        0x1f => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * 2.0f32.powi(i32::from(exponent) - 15) * (1.0 + f32::from(fraction) / 1024.0),
    }
}

/// f32 to nearest `R16G16B16A16_FLOAT` half bit pattern (round-to-nearest-even),
/// with saturation to +/-inf on overflow. This is the standard IEEE-754
/// binary32 -> binary16 conversion; used only to model the engine's half-float
/// intermediate render target.
pub fn f32_to_half_float(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exp == 0xff {
        // Inf/NaN.
        return if mantissa == 0 {
            sign | 0x7c00
        } else {
            // Preserve NaN-ness; canonicalize the payload.
            sign | 0x7e00
        };
    }
    // Unbias f32 exponent (127) and rebias to half (15).
    let unbiased = exp - 127;
    if unbiased > 15 {
        // Overflow -> inf.
        return sign | 0x7c00;
    }
    if unbiased < -14 {
        // Subnormal or zero in half. Represent value as a subnormal.
        if unbiased < -24 {
            return sign; // Rounds to zero.
        }
        // Add implicit leading 1, then shift to align to the subnormal range.
        let mant_with_implicit = mantissa | 0x0080_0000;
        // bits to drop from the 24-bit mantissa; unbiased in -24..-14 here, so
        // this is in 13..=23 and safely non-negative.
        let shift = ((-unbiased - 14) + 13) as u32;
        let half_mant = round_shift(mant_with_implicit, shift);
        return sign | (half_mant as u16);
    }
    // Normalized half.
    let half_exp = ((unbiased + 15) as u32) << 10;
    let half_mant = round_shift(mantissa, 13);
    // Rounding may overflow the mantissa into the exponent, which is the
    // intended IEEE behavior (e.g. 0x3ff + 1 -> next exponent).
    (sign | ((half_exp + half_mant) as u16)) as u16
}

/// Round-to-nearest-even shift of `value` right by `shift` bits.
fn round_shift(value: u32, shift: u32) -> u32 {
    if shift == 0 {
        return value;
    }
    if shift >= 32 {
        return 0;
    }
    let dropped = value & ((1u32 << shift) - 1);
    let kept = value >> shift;
    let halfway = 1u32 << (shift - 1);
    if dropped > halfway || (dropped == halfway && (kept & 1) == 1) {
        kept + 1
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_kernel_matches_captured_bloom_constant_buffer() {
        // Captured bloom.shader ConstantBuffer(2,39) from restore_gate_1
        // (the standard volume: BLOOM_WIDTH=1.5, BLOOM_SCALE=0.9). These are
        // the engine's own linear-sampled Gaussian weights/offsets; our
        // derivation must reproduce them to f32 rounding.
        let kernel = BloomBlurKernel::from_gaussian(1.5, 0.9).expect("kernel");
        let cap_w0 = 0.2393653690814972_f32;
        let cap_weights = [
            0.29007482528686523_f32,
            0.039232172071933746,
            0.0010056657483801246,
            4.627003818313824e-06,
        ];
        let cap_offsets = [
            1.3392436504364014_f32,
            3.174285411834717,
            5.079845905303955,
            7.034444808959961,
        ];
        assert!(
            (kernel.weight0 - cap_w0).abs() < 1e-6,
            "weight0 {} vs {}",
            kernel.weight0,
            cap_w0
        );
        for i in 0..BLOOM_TAP_PAIRS {
            assert!(
                (kernel.weights[i] - cap_weights[i]).abs() < 1e-6,
                "weight[{i}] {} vs {}",
                kernel.weights[i],
                cap_weights[i]
            );
            assert!(
                (kernel.offsets[i] - cap_offsets[i]).abs() < 1e-4,
                "offset[{i}] {} vs {}",
                kernel.offsets[i],
                cap_offsets[i]
            );
        }
    }

    #[test]
    fn kernel_pass_weight_sum_equals_bloom_scale() {
        for scale in [0.5f32, 0.9, 1.0] {
            let kernel = BloomBlurKernel::from_gaussian(1.5, scale).expect("kernel");
            assert!(
                (kernel.pass_weight_sum() - scale).abs() < 1e-6,
                "pass sum {} vs scale {scale}",
                kernel.pass_weight_sum()
            );
        }
    }

    #[test]
    fn bright_pass_knee_matches_shader_on_known_texels() {
        // A uniform 2x2 block so the box average is the block value.
        let make = |rgb: [f32; 3], a: f32| {
            let t = [rgb[0], rgb[1], rgb[2], a];
            BloomImageRgba::new(2, 2, vec![t, t, t, t]).unwrap()
        };
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.9,
            emissive_bloom_strength: 1.0,
        };
        // Below threshold, no emissive: color -> 0.
        let below = bright_pass_downsample(&make([0.5, 0.5, 0.5], 0.0), params).unwrap();
        for channel in 0..3 {
            assert!(below.texels[0][channel].abs() < 1e-6);
        }
        // Above threshold: maxc=1.4, knee = (1.4-0.9)=0.5; 0.5/(0.5+0.5)=0.5.
        let above = bright_pass_downsample(&make([1.4, 0.2, 0.2], 0.0), params).unwrap();
        // r channel = 1.4 * 0.5 = 0.7.
        assert!(
            (above.texels[0][0] - 0.7).abs() < 1e-5,
            "{}",
            above.texels[0][0]
        );
        // Emissive-only: maxc below threshold, alpha adds a*strength.
        let emissive = bright_pass_downsample(&make([0.1, 0.1, 0.1], 0.3), params).unwrap();
        // vmax = 0 + 0.3*1.0 = 0.3 -> r = 0.1*0.3 = 0.03.
        assert!(
            (emissive.texels[0][0] - 0.03).abs() < 1e-6,
            "{}",
            emissive.texels[0][0]
        );
    }

    #[test]
    fn downsample_is_two_by_two_box_average() {
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.0,
            emissive_bloom_strength: 0.0,
        };
        // 2x2 with distinct values; box avg of r = (0.1+0.2+0.3+0.4)/4 = 0.25.
        let img = BloomImageRgba::new(
            2,
            2,
            vec![
                [0.1, 0.0, 0.0, 0.0],
                [0.2, 0.0, 0.0, 0.0],
                [0.3, 0.0, 0.0, 0.0],
                [0.4, 0.0, 0.0, 0.0],
            ],
        )
        .unwrap();
        let out = bright_pass_downsample(&img, params).unwrap();
        assert_eq!(out.width, 1);
        assert_eq!(out.height, 1);
        // bright_threshold 0 => vmax = maxc/(0.5+maxc); maxc = 0.25.
        // vmax = 0.25/0.75 = 1/3; r = 0.25 * 1/3.
        let expect = 0.25f32 * (0.25 / 0.75);
        assert!(
            (out.texels[0][0] - expect).abs() < 1e-6,
            "{}",
            out.texels[0][0]
        );
    }

    #[test]
    fn blur_of_constant_field_scales_by_bloom_scale() {
        // A constant image blurred is itself times the per-pass sum (clamp edges
        // keep it constant). Two passes -> bloom_scale^2.
        let kernel = BloomBlurKernel::from_gaussian(1.5, 0.9).expect("kernel");
        let img = BloomImageRgba::new(16, 16, vec![[1.0, 1.0, 1.0, 1.0]; 256]).unwrap();
        let h = blur_pass(&img, &kernel, BlurAxis::Horizontal);
        let v = blur_pass(&h, &kernel, BlurAxis::Vertical);
        // Center pixel far from edges.
        let center = v.texels[v.index(8, 8)];
        let expect = 0.9f32 * 0.9;
        assert!((center[0] - expect).abs() < 1e-5, "{}", center[0]);
    }

    #[test]
    fn bilinear_axis_sample_interpolates_and_clamps() {
        let img = BloomImageRgba::new(
            3,
            1,
            vec![
                [0.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0, 0.0],
            ],
        )
        .unwrap();
        // From x=1, offset +0.5 -> between texel 1 (1.0) and texel 2 (2.0) => 1.5.
        let s = sample_bilinear_axis(&img, 1, 0, 0.5, BlurAxis::Horizontal);
        assert!((s[0] - 1.5).abs() < 1e-6, "{}", s[0]);
        // From x=0, offset -3 clamps to texel 0.
        let c = sample_bilinear_axis(&img, 0, 0, -3.0, BlurAxis::Horizontal);
        assert!((c[0] - 0.0).abs() < 1e-6, "{}", c[0]);
        // From x=2, offset +3 clamps to texel 2 (2.0).
        let e = sample_bilinear_axis(&img, 2, 0, 3.0, BlurAxis::Horizontal);
        assert!((e[0] - 2.0).abs() < 1e-6, "{}", e[0]);
    }

    #[test]
    fn half_float_roundtrip_matches_known_values() {
        for (bits, value) in [
            (0x0000u16, 0.0f32),
            (0x3c00, 1.0),
            (0xbc00, -1.0),
            (0x4000, 2.0),
            (0x3800, 0.5),
        ] {
            assert_eq!(half_float_to_f32(bits), value);
            assert_eq!(f32_to_half_float(value), bits, "encode {value}");
        }
        // Round-trip a spread of finite values through f32->half->f32.
        for v in [0.1f32, 0.239, 0.545, 1.79, 0.0001, 0.03125] {
            let rt = half_float_to_f32(f32_to_half_float(v));
            // Half precision ~ 3-4 significant digits.
            assert!(
                (rt - v).abs() <= v.abs() * 0.001 + 1e-4,
                "roundtrip {v} -> {rt}"
            );
        }
    }

    #[test]
    fn image_decode_rejects_short_payload() {
        let bytes = vec![0u8; 8]; // one texel, but claim 2x1
        let err = BloomImageRgba::from_r16g16b16a16_float(&bytes, 2, 1, 16).unwrap_err();
        assert!(err.to_string().contains("bloom image blocked"), "{err}");
    }

    #[test]
    fn run_bloom_chain_rejects_odd_dimensions() {
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.9,
            emissive_bloom_strength: 1.0,
        };
        let img = BloomImageRgba::new(3, 2, vec![[0.0; 4]; 6]).unwrap();
        let err = run_bloom_chain(&img, params).unwrap_err();
        assert!(err.to_string().contains("even size"), "{err}");
    }

    #[test]
    fn parse_posteffect_volume_reads_standard_bloom_keys() {
        let src = "\
posteffect_values = {
    name = standard
    lut = \"gfx/world/colorcorrection.tga\"
    BLOOM_WIDTH = 1.5
    BLOOM_SCALE = 0.9
    BRIGHT_THRESHOLD = 0.9
    tonemap_middlegrey = 0.55
}

posteffect_values = {
    name = default
    inherit = standard
    #BLOOM_WIDTH = 1.5
}
";
        let standard = parse_posteffect_volume(src, "standard").unwrap();
        assert!((standard["BLOOM_WIDTH"] - 1.5).abs() < 1e-6);
        assert!((standard["BLOOM_SCALE"] - 0.9).abs() < 1e-6);
        assert!((standard["BRIGHT_THRESHOLD"] - 0.9).abs() < 1e-6);
        // The child inherits the parent's bloom keys since its own are commented.
        let default = parse_posteffect_volume(src, "default").unwrap();
        assert!((default["BLOOM_WIDTH"] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn parse_posteffect_volume_refuses_nested_block() {
        // A nested sub-block must be a typed refusal naming the volume, not a
        // silent early close that drops the trailing bloom keys.
        let src = "\
posteffect_values = {
    name = standard
    BLOOM_WIDTH = 1.5
    nested_table = {
        foo = 1.0
    }
    BLOOM_SCALE = 0.9
}
";
        let err = parse_posteffect_volume(src, "standard").unwrap_err();
        assert!(err.to_string().contains("nested block"), "{err}");
        assert!(err.to_string().contains("standard"), "{err}");
    }

    #[test]
    fn parse_posteffect_volume_inline_brace_line_does_not_close_block() {
        // An inline net-zero brace line (`foo = { 1 2 3 }`) is not a block
        // terminator; bloom keys after it must still be read.
        let src = "\
posteffect_values = {
    name = standard
    fog_color = { 0.5 0.6 0.7 }
    BLOOM_WIDTH = 1.5
    BLOOM_SCALE = 0.9
    BRIGHT_THRESHOLD = 0.9
}
";
        let standard = parse_posteffect_volume(src, "standard").unwrap();
        assert!((standard["BLOOM_WIDTH"] - 1.5).abs() < 1e-6);
        assert!((standard["BLOOM_SCALE"] - 0.9).abs() < 1e-6);
        assert!((standard["BRIGHT_THRESHOLD"] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn parse_define_number_reads_and_fails_closed() {
        let src = "NDefines = {\n\tBLOOM_WIDTH = 1.5,\n\tEMISSIVE_BLOOM_STRENGTH = 1.0,\n}\n";
        assert!((parse_define_number(src, "EMISSIVE_BLOOM_STRENGTH").unwrap() - 1.0).abs() < 1e-6);
        assert!(parse_define_number(src, "MISSING_KEY").is_err());
    }

    #[test]
    fn all_below_threshold_scene_produces_zero_bloom() {
        // If every MainScene channel is below BRIGHT_THRESHOLD (and alpha 0), the
        // bright pass writes 0, so the whole bloom buffer is 0 — the composite
        // then adds nothing. This is the day-map case (max channel < 0.9).
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.9,
            emissive_bloom_strength: 1.0,
        };
        let scene = BloomImageRgba::new(8, 8, vec![[0.6, 0.5, 0.4, 0.0]; 64]).unwrap();
        let out = run_bloom_chain(&scene, params).unwrap();
        for texel in &out.bloom.texels {
            for channel in 0..3 {
                assert!(
                    texel[channel].abs() < 1e-6,
                    "expected zero bloom, got {texel:?}"
                );
            }
        }
    }

    #[test]
    fn bright_pixel_blooms_and_spreads_to_neighbors() {
        // A localized bright block above threshold produces non-zero bloom that
        // spreads (the blur), so a neighboring half-res texel that was dark in the
        // bright pass becomes non-zero after the separable blur.
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.9,
            emissive_bloom_strength: 0.0,
        };
        let w = 8;
        let h = 8;
        let mut texels = vec![[0.1f32, 0.1, 0.1, 0.0]; w * h];
        // Bright 2x2 block at the top-left -> one bright half-res texel at (0,0).
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            texels[y * w + x] = [3.0, 3.0, 3.0, 0.0];
        }
        let scene = BloomImageRgba::new(w, h, texels).unwrap();
        let out = run_bloom_chain(&scene, params).unwrap();
        // The bright half-res source texel (0,0) is well above zero.
        let idx00 = out.bloom.index(0, 0);
        assert!(out.bloom.texels[idx00][0] > 0.0);
        // A half-res texel one step away picked up spread from the blur.
        let idx10 = out.bloom.index(1, 0);
        assert!(
            out.bloom.texels[idx10][0] > 0.0,
            "bloom must spread to the neighbor via the blur: {:?}",
            out.bloom.texels[idx10]
        );
    }
}
