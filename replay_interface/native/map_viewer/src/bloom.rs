
use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::fs;

pub const BLOOM_LUMINANCE_VECTOR: [f32; 3] = [0.2125, 0.7154, 0.0721];

const BLOOM_TAP_PAIRS: usize = 4;
const BLOOM_DISCRETE_RADIUS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomParams {
    pub bloom_width: f32,
    pub bloom_scale: f32,
    pub bright_threshold: f32,
    pub emissive_bloom_strength: f32,
}

impl BloomParams {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomBlurKernel {
    pub weight0: f32,
    pub weights: [f32; BLOOM_TAP_PAIRS],
    pub offsets: [f32; BLOOM_TAP_PAIRS],
}

impl BloomBlurKernel {
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
        let sigma = f64::from(sigma);
        let mut taps = [0.0f64; BLOOM_DISCRETE_RADIUS + 1];
        for (x, tap) in taps.iter_mut().enumerate() {
            let xf = x as f64;
            *tap = (-(xf * xf) / (2.0 * sigma * sigma)).exp();
        }
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
            let a = 2 * pair + 1;
            let b = 2 * pair + 2;
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

    pub fn pass_weight_sum(&self) -> f32 {
        self.weight0 + 2.0 * self.weights.iter().sum::<f32>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BloomImageRgba {
    pub width: usize,
    pub height: usize,
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

    pub fn from_r16g16b16a16_float(
        bytes: &[u8],
        width: usize,
        height: usize,
        row_pitch_bytes: usize,
    ) -> Result<Self> {
        let bytes_per_texel = 8usize;
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

#[derive(Clone, Debug)]
pub struct BloomChainOutput {
    pub params: BloomParams,
    pub kernel: BloomBlurKernel,
    pub bright_pass: BloomImageRgba,
    pub bloom: BloomImageRgba,
}

pub const BLOOM_DEFAULT_VOLUME: &str = "standard";

pub fn resolve_bloom_params(resolver: &GameFileResolver) -> Result<BloomParams> {
    resolve_bloom_params_for_volume(resolver, BLOOM_DEFAULT_VOLUME)
}

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

    let (sources, _) = crate::xmap09_lighting_law::admitted_effective_define_sources(resolver, "bloom defines")?;
    let defines = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;

    let bloom_width = volume
        .get("BLOOM_WIDTH")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| defines.scalar("NGraphics", "BLOOM_WIDTH"))?;
    let bloom_scale = volume
        .get("BLOOM_SCALE")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| defines.scalar("NGraphics", "BLOOM_SCALE"))?;
    let bright_threshold = volume
        .get("BRIGHT_THRESHOLD")
        .copied()
        .map(Ok)
        .unwrap_or_else(|| defines.scalar("NGraphics", "BRIGHT_THRESHOLD"))?;
    let emissive_bloom_strength = defines.scalar("NGraphics", "EMISSIVE_BLOOM_STRENGTH")?;

    let params = BloomParams {
        bloom_width,
        bloom_scale,
        bright_threshold,
        emissive_bloom_strength,
    };
    params.validate()?;
    Ok(params)
}

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

fn split_posteffect_blocks(
    source: &str,
) -> Result<std::collections::BTreeMap<String, PostEffectBlock>> {
    const BLOOM_KEYS: [&str; 3] = ["BLOOM_WIDTH", "BLOOM_SCALE", "BRIGHT_THRESHOLD"];
    let mut blocks = std::collections::BTreeMap::new();
    let mut in_block = false;
    let mut current_name: Option<String> = None;
    let mut current = PostEffectBlock::default();
    for raw in source.lines() {
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
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if opens > closes {
            return Err(format!(
                "bloom chain blocked: posteffect_volumes.txt volume {} contains a nested block, \
                 which this parser does not support",
                current_name.as_deref().unwrap_or("<unnamed>")
            )
            .into());
        }
        if closes > opens {
            let name = current_name
                .take()
                .ok_or("bloom chain blocked: posteffect_volumes.txt block is missing a name")?;
            blocks.insert(name, std::mem::take(&mut current));
            in_block = false;
            continue;
        }
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

fn strip_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let value = rest.trim().split_whitespace().next()?;
    Some(value)
}

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
    let horizontal = blur_pass(&bright_pass, &kernel, BlurAxis::Horizontal);
    let horizontal = quantize_half(&horizontal);
    let mut bloom = blur_pass(&horizontal, &kernel, BlurAxis::Vertical);
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

fn bright_pass_downsample(main: &BloomImageRgba, params: BloomParams) -> Result<BloomImageRgba> {
    let out_w = main.width / 2;
    let out_h = main.height / 2;
    let mut texels = Vec::with_capacity(out_w * out_h);
    for y in 0..out_h {
        for x in 0..out_w {
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
            let mut vmax = (r.max(g).max(b) - params.bright_threshold).max(0.0);
            vmax /= 0.5 + vmax;
            vmax += a * params.emissive_bloom_strength;
            let color = [r * vmax, g * vmax, b * vmax];
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

pub fn f32_to_half_float(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exp == 0xff {
        return if mantissa == 0 {
            sign | 0x7c00
        } else {
            sign | 0x7e00
        };
    }
    let unbiased = exp - 127;
    if unbiased > 15 {
        return sign | 0x7c00;
    }
    if unbiased < -14 {
        if unbiased < -24 {
            return sign;
        }
        let mant_with_implicit = mantissa | 0x0080_0000;
        let shift = ((-unbiased - 14) + 13) as u32;
        let half_mant = round_shift(mant_with_implicit, shift);
        return sign | (half_mant as u16);
    }
    let half_exp = ((unbiased + 15) as u32) << 10;
    let half_mant = round_shift(mantissa, 13);
    (sign | ((half_exp + half_mant) as u16)) as u16
}

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
    fn bloom_resolves_mod_expressions_and_keeps_volume_precedence() {
        let root = crate::test_support::temp_test_dir("bloom-effective-defines");
        let files = [
            ("gfx/posteffect_volumes.txt", "posteffect_values = {\nname = standard\nBLOOM_WIDTH = 7\n}\n"),
            ("common/defines/00_graphics.lua", "NDefines = { NGraphics = { BLOOM_WIDTH = 1, BLOOM_SCALE = 2, BRIGHT_THRESHOLD = 3, EMISSIVE_BLOOM_STRENGTH = 4 } }"),
            ("common/defines/zz_mod.lua", "local g = NDefines.NGraphics; g.BLOOM_WIDTH = nil; g.BLOOM_SCALE = g.BLOOM_SCALE / 4; g.BRIGHT_THRESHOLD = 3 / 10; g.EMISSIVE_BLOOM_STRENGTH = math.floor(1.9)"),
        ];
        for (path, source) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        let resolver = GameFileResolver::from_options(root, crate::GameFileResolverOptions {
            mod_paths: Vec::new(), use_dlc_load: false, userdata: None,
        }).unwrap();
        let paths = files.iter().map(|(path, _)| path.to_string()).collect();
        let (resolver, _) = crate::test_support::admitted_vanilla_resolver(&resolver, &paths, "bloom-effective-defines");
        let params = resolve_bloom_params(&resolver).unwrap();
        assert_eq!(params.bloom_width, 7.0);
        assert_eq!(params.bloom_scale, 0.5);
        assert_eq!(params.bright_threshold, 0.3);
        assert_eq!(params.emissive_bloom_strength, 1.0);
    }

    #[test]
    fn gaussian_kernel_matches_captured_bloom_constant_buffer() {
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
        let below = bright_pass_downsample(&make([0.5, 0.5, 0.5], 0.0), params).unwrap();
        for channel in 0..3 {
            assert!(below.texels[0][channel].abs() < 1e-6);
        }
        let above = bright_pass_downsample(&make([1.4, 0.2, 0.2], 0.0), params).unwrap();
        assert!(
            (above.texels[0][0] - 0.7).abs() < 1e-5,
            "{}",
            above.texels[0][0]
        );
        let emissive = bright_pass_downsample(&make([0.1, 0.1, 0.1], 0.3), params).unwrap();
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
        let expect = 0.25f32 * (0.25 / 0.75);
        assert!(
            (out.texels[0][0] - expect).abs() < 1e-6,
            "{}",
            out.texels[0][0]
        );
    }

    #[test]
    fn blur_of_constant_field_scales_by_bloom_scale() {
        let kernel = BloomBlurKernel::from_gaussian(1.5, 0.9).expect("kernel");
        let img = BloomImageRgba::new(16, 16, vec![[1.0, 1.0, 1.0, 1.0]; 256]).unwrap();
        let h = blur_pass(&img, &kernel, BlurAxis::Horizontal);
        let v = blur_pass(&h, &kernel, BlurAxis::Vertical);
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
        let s = sample_bilinear_axis(&img, 1, 0, 0.5, BlurAxis::Horizontal);
        assert!((s[0] - 1.5).abs() < 1e-6, "{}", s[0]);
        let c = sample_bilinear_axis(&img, 0, 0, -3.0, BlurAxis::Horizontal);
        assert!((c[0] - 0.0).abs() < 1e-6, "{}", c[0]);
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
        for v in [0.1f32, 0.239, 0.545, 1.79, 0.0001, 0.03125] {
            let rt = half_float_to_f32(f32_to_half_float(v));
            assert!(
                (rt - v).abs() <= v.abs() * 0.001 + 1e-4,
                "roundtrip {v} -> {rt}"
            );
        }
    }

    #[test]
    fn image_decode_rejects_short_payload() {
        let bytes = vec![0u8; 8];
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
        let default = parse_posteffect_volume(src, "default").unwrap();
        assert!((default["BLOOM_WIDTH"] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn parse_posteffect_volume_refuses_nested_block() {
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
    fn all_below_threshold_scene_produces_zero_bloom() {
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
        let params = BloomParams {
            bloom_width: 1.5,
            bloom_scale: 0.9,
            bright_threshold: 0.9,
            emissive_bloom_strength: 0.0,
        };
        let w = 8;
        let h = 8;
        let mut texels = vec![[0.1f32, 0.1, 0.1, 0.0]; w * h];
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            texels[y * w + x] = [3.0, 3.0, 3.0, 0.0];
        }
        let scene = BloomImageRgba::new(w, h, texels).unwrap();
        let out = run_bloom_chain(&scene, params).unwrap();
        let idx00 = out.bloom.index(0, 0);
        assert!(out.bloom.texels[idx00][0] > 0.0);
        let idx10 = out.bloom.index(1, 0);
        assert!(
            out.bloom.texels[idx10][0] > 0.0,
            "bloom must spread to the neighbor via the blur: {:?}",
            out.bloom.texels[idx10]
        );
    }
}
