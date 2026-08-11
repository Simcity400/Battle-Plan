//! Fog-of-war render law for the game map passes.
//!
//! Shipped shader anchors:
//! - `gfx/FX/fow.fxh:79-88` applies fog as
//!   `lerp(Color, lerp(DARK_FOW_COLOR, BRIGHT_FOW_COLOR, FogColorFactor),
//!   FogAlphaFactor)`.
//! - `gfx/FX/pdxmap.shader:388-389` samples the screen-space `ShadowMap`
//!   with `ApplyFOW(vOut, ShadowMap, Input.vScreenCoord)` and then removes a
//!   fraction of the fog over borders through
//!   `BORDER_FOW_REMOVAL_FACTOR * (1 - vBloomAlpha)`.
//! - The same `ShadowMap` render target is written by the map shadow/FoW
//!   effect as `(shadow, FogColorFactor, FogAlphaFactor, 1)`.
//!
//! Captured DXGI 87 (`B8G8R8A8_UNORM`) staging bytes are in memory order
//! B,G,R,A. HLSL samples them as R,G,B,A, so byte 2 is `.r` (shadow), byte 1
//! is `.g` (fog color factor), and byte 0 is `.b` (fog alpha factor).

use crate::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn fow_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into()).into()
}

pub const FOW_SHADER_SOURCE: &str = "gfx/FX/fow.fxh";
pub const PDXMAP_SHADER_SOURCE: &str = "gfx/FX/pdxmap.shader";
pub const PDXMAP_INCLUDE_SOURCE: &str = "gfx/FX/pdxmap.fxh";
pub const FOW_REQUIRED_SHADER_SOURCES: [&str; 7] = [
    "gfx/FX/fow.fxh",
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
];

// gfx/FX/fow.fxh:11-18.
pub const FOW_TRANSPARENCY_MIN: f32 = 0.3;
pub const FOW_TRANSPARENCY_MAX: f32 = 0.9;
pub const FOW_COLOR_MIN: f32 = 0.2;
pub const FOW_COLOR_MAX: f32 = 1.0;
pub const FOW_NOISE_TILING: f32 = 0.01;
pub const FOW_NOISE_SPEED: f32 = 0.015;

pub const SHADOW_MAP_DXGI_FORMAT: u32 = 87;
/// Exact a729 `CUpdateTextureThreaded` coverage edge for the A8 IntelMap.
pub const INTEL_MAP_PROVINCE_COVERAGE_EDGE: usize = 6;
pub const INTEL_MAP_MAX_PROVINCE_SAMPLES: usize =
    INTEL_MAP_PROVINCE_COVERAGE_EDGE * INTEL_MAP_PROVINCE_COVERAGE_EDGE;
pub const WAVE1_CAP4_FOW_ON_SHADOWMAP_FNV1A64: &str = "35fa63fe96ea5217";
pub const WAVE1_CAP4_FOW_OFF_SHADOWMAP_FNV1A64: &str = "4929cd2b85441701";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactUnitDisclosure {
    Hidden,
    Detected,
    MapVisible,
    ExactDetails,
}

/// Exact a729 land/rail unit disclosure tiers. The land map producer admits
/// counters only at 250, while lower nonzero bytes remain detection/intel
/// state and must not produce a map counter.
pub fn exact_a729_land_unit_disclosure(raw_intel: u8) -> ExactUnitDisclosure {
    match raw_intel {
        0 => ExactUnitDisclosure::Hidden,
        250..=u8::MAX => ExactUnitDisclosure::ExactDetails,
        _ => ExactUnitDisclosure::Detected,
    }
}

/// Exact a729 task-force map-icon disclosure. 150 admits the map producer;
/// 250 admits exact detail. Values 1..149 are detected but not map-visible.
pub fn exact_a729_taskforce_disclosure(raw_intel: u8) -> ExactUnitDisclosure {
    match raw_intel {
        0 => ExactUnitDisclosure::Hidden,
        250..=u8::MAX => ExactUnitDisclosure::ExactDetails,
        150..=249 => ExactUnitDisclosure::MapVisible,
        _ => ExactUnitDisclosure::Detected,
    }
}

/// Rebuild one a729 A8 IntelMap texel from the engine's province-id coverage
/// cells and an admitted complete current-POV visibility vector.
///
/// Each coverage sample contributes 255 for an invalid/non-land province or
/// while FOW is disabled. Otherwise it contributes the exact engine byte at
/// the province database ID. The stored A8 byte is the bitwise inverse of the
/// approximation and must not consume a stale or partial visibility vector.
pub fn exact_a729_intel_map_texel(
    province_ids: &[u16],
    province_is_eligible_land: &[bool],
    province_visibility: &[u8],
    fow_enabled: bool,
) -> Result<u8> {
    if province_ids.is_empty() || province_ids.len() > INTEL_MAP_MAX_PROVINCE_SAMPLES {
        return Err(fow_error(format!(
            "IntelMap coverage sample count must be 1..={INTEL_MAP_MAX_PROVINCE_SAMPLES}"
        )));
    }
    if province_is_eligible_land.len() != province_visibility.len() {
        return Err(fow_error(
            "IntelMap province eligibility/visibility vector length mismatch",
        ));
    }
    let mut sum = 0u32;
    for &province_id in province_ids {
        let index = usize::from(province_id);
        let sample = if !fow_enabled
            || index >= province_visibility.len()
            || !province_is_eligible_land[index]
        {
            u8::MAX
        } else {
            province_visibility[index]
        };
        sum += u32::from(sample);
    }
    let average = sum
        / u32::try_from(province_ids.len())
            .map_err(|_| fow_error("IntelMap coverage count overflow"))?;
    Ok(!(average as u8))
}

/// Build the complete a729 A8 IntelMap from the province-id map raster.
///
/// The output extent is the positive map extent divided by six in each axis;
/// remainder source pixels are not represented, matching the engine staging
/// allocation and threaded rasterizer.
pub fn exact_a729_intel_map(
    province_id_raster: &[u16],
    map_width: usize,
    map_height: usize,
    province_is_eligible_land: &[bool],
    province_visibility: &[u8],
    fow_enabled: bool,
) -> Result<(usize, usize, Vec<u8>)> {
    if map_width == 0
        || map_height == 0
        || province_id_raster.len()
            != map_width
                .checked_mul(map_height)
                .ok_or_else(|| fow_error("province-id raster extent overflow"))?
    {
        return Err(fow_error("province-id raster extent/length mismatch"));
    }
    let width = map_width / INTEL_MAP_PROVINCE_COVERAGE_EDGE;
    let height = map_height / INTEL_MAP_PROVINCE_COVERAGE_EDGE;
    if width == 0 || height == 0 {
        return Err(fow_error(
            "province-id raster is smaller than one IntelMap coverage cell",
        ));
    }
    let mut out = Vec::with_capacity(
        width
            .checked_mul(height)
            .ok_or_else(|| fow_error("IntelMap output extent overflow"))?,
    );
    let mut coverage = [0u16; INTEL_MAP_MAX_PROVINCE_SAMPLES];
    for texel_y in 0..height {
        for texel_x in 0..width {
            let mut cursor = 0usize;
            let source_y = texel_y * INTEL_MAP_PROVINCE_COVERAGE_EDGE;
            let source_x = texel_x * INTEL_MAP_PROVINCE_COVERAGE_EDGE;
            for dy in 0..INTEL_MAP_PROVINCE_COVERAGE_EDGE {
                let row = (source_y + dy) * map_width + source_x;
                for dx in 0..INTEL_MAP_PROVINCE_COVERAGE_EDGE {
                    coverage[cursor] = province_id_raster[row + dx];
                    cursor += 1;
                }
            }
            out.push(exact_a729_intel_map_texel(
                &coverage[..cursor],
                province_is_eligible_land,
                province_visibility,
                fow_enabled,
            )?);
        }
    }
    Ok((width, height, out))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FowFogFactors {
    pub fog_color_factor: f32,
    pub fog_alpha_factor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowMapFogSample {
    pub shadow_factor: f32,
    pub fog_color_factor: f32,
    pub fog_alpha_factor: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FowChannelDiffCounts {
    pub shadow_r: usize,
    pub fog_color_g: usize,
    pub fog_alpha_b: usize,
    pub alpha_a: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FowPairDrawDiffReport {
    pub on_draws: usize,
    pub off_draws: usize,
    pub shared_draw_signatures_by_index: usize,
    pub on_only_draw_signatures: usize,
    pub off_only_draw_signatures: usize,
    pub non_shadow_srv_diffs: usize,
    pub shadow_srv_diffs: usize,
    pub constant_buffer_diffs: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FowShadowMapReport {
    pub width: usize,
    pub height: usize,
    pub dxgi_format: u32,
    pub byte_len: usize,
    pub on_fnv1a64: String,
    pub off_fnv1a64: String,
    pub channel_diffs: FowChannelDiffCounts,
    pub on_fog_alpha_nonzero_pixels: usize,
    pub off_fog_alpha_nonzero_pixels: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FowParityStats {
    pub pixels: usize,
    pub channels: usize,
    pub exact_half_channels: usize,
    pub near_one_ulp_half_channels: usize,
    pub mean_abs: f32,
    pub max_abs: f32,
    pub p95_abs: f32,
    pub p99_abs: f32,
}

impl FowParityStats {
    pub fn exact_percent(&self) -> f32 {
        percent(self.exact_half_channels, self.channels)
    }

    pub fn near_one_ulp_percent(&self) -> f32 {
        percent(self.near_one_ulp_half_channels, self.channels)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FowCapturePairReport {
    pub draw_diff: FowPairDrawDiffReport,
    pub shadow_map: FowShadowMapReport,
    pub observer_to_fow_all_pixels: FowParityStats,
    pub observer_to_fow_clear_low_alpha_pixels: FowParityStats,
    pub observer_to_fow_fogged_changed_pixels: FowParityStats,
    pub adversarial_forced_fog_on_clear_pixels: FowParityStats,
}

pub fn shadow_map_bgra8_unorm_sample(pixel: &[u8]) -> Result<ShadowMapFogSample> {
    if pixel.len() < 4 {
        return Err("ShadowMap BGRA8 sample requires at least 4 bytes".into());
    }
    Ok(ShadowMapFogSample {
        shadow_factor: f32::from(pixel[2]) / 255.0,
        fog_color_factor: f32::from(pixel[1]) / 255.0,
        fog_alpha_factor: f32::from(pixel[0]) / 255.0,
    })
}

pub(crate) fn fow_fog_color(fog_color_factor: f32) -> Vec3 {
    lerp_vec3(DARK_FOW_COLOR, BRIGHT_FOW_COLOR, fog_color_factor)
}

pub(crate) fn apply_fow(color: Vec3, factors: FowFogFactors) -> Vec3 {
    lerp_vec3(
        color,
        fow_fog_color(factors.fog_color_factor),
        factors.fog_alpha_factor,
    )
}

/// Effective fog alpha after `pdxmap.shader:389`.
///
/// The shader computes `lerp(vFOW, vOut, BORDER_FOW_REMOVAL_FACTOR *
/// (1 - vBloomAlpha))`, so the fog contribution is scaled by
/// `1 - BORDER_FOW_REMOVAL_FACTOR * (1 - vBloomAlpha)`.
pub fn pdxmap_fow_effective_alpha(fog_alpha_factor: f32, bloom_alpha: f32) -> f32 {
    let removal = BORDER_FOW_REMOVAL_FACTOR * (1.0 - bloom_alpha);
    fog_alpha_factor * (1.0 - removal)
}

pub(crate) fn apply_pdxmap_fow_tail(color: Vec3, factors: FowFogFactors, bloom_alpha: f32) -> Vec3 {
    let effective = FowFogFactors {
        fog_color_factor: factors.fog_color_factor,
        fog_alpha_factor: pdxmap_fow_effective_alpha(factors.fog_alpha_factor, bloom_alpha),
    };
    apply_fow(color, effective)
}

pub(crate) fn remove_pdxmap_fow_tail(
    fogged_color: Vec3,
    factors: FowFogFactors,
    bloom_alpha: f32,
) -> Option<Vec3> {
    let alpha = pdxmap_fow_effective_alpha(factors.fog_alpha_factor, bloom_alpha);
    let denom = 1.0 - alpha;
    if denom.abs() < 1e-6 {
        return None;
    }
    let fog = fow_fog_color(factors.fog_color_factor);
    Some(Vec3 {
        x: (fogged_color.x - fog.x * alpha) / denom,
        y: (fogged_color.y - fog.y * alpha) / denom,
        z: (fogged_color.z - fog.z * alpha) / denom,
    })
}

pub fn analyze_wave1_cap4_fow_pair(rounds_root: &Path) -> Result<FowCapturePairReport> {
    analyze_fow_capture_pair(
        &rounds_root.join("fow_on_pov_front"),
        &rounds_root.join("fow_off_observer_front"),
    )
}

pub fn analyze_fow_capture_pair(on_dir: &Path, off_dir: &Path) -> Result<FowCapturePairReport> {
    let on_bundle_path = on_dir.join(MAP_FRAME_BUNDLE_FILE);
    let off_bundle_path = off_dir.join(MAP_FRAME_BUNDLE_FILE);
    let on_bundle: Value = serde_json::from_slice(&fs::read(&on_bundle_path)?)?;
    let off_bundle: Value = serde_json::from_slice(&fs::read(&off_bundle_path)?)?;

    let draw_diff = compare_draw_sets(&on_bundle, &off_bundle)?;
    let on_shadow = shadow_resource(&on_bundle)?;
    let off_shadow = shadow_resource(&off_bundle)?;
    let on_shadow_file = string_field(on_shadow, "file")?;
    let off_shadow_file = string_field(off_shadow, "file")?;
    let on_shadow_bytes = fs::read(on_dir.join(on_shadow_file))?;
    let off_shadow_bytes = fs::read(off_dir.join(off_shadow_file))?;
    let shadow_map = shadow_map_report(on_shadow, off_shadow, &on_shadow_bytes, &off_shadow_bytes)?;

    let on_rt = post_pass_render_target_bytes(on_dir, &on_bundle)?;
    let off_rt = post_pass_render_target_bytes(off_dir, &off_bundle)?;
    let masks = build_pair_masks(&on_shadow_bytes, &off_shadow_bytes)?;

    // `bloom_alpha = 0` is the non-border branch explicitly initialized in
    // `pdxmap.shader:324`. The residual on changed/fogged pixels is therefore
    // a measured missing-evidence signal for per-pixel `vBloomAlpha` and the
    // pre-FoW color, not an invitation to fit the factor.
    let observer_to_fow_all_pixels = parity_from_observer_to_fow(
        &off_rt,
        &on_rt,
        &off_shadow_bytes,
        &on_shadow_bytes,
        &masks.all,
        0.0,
    )?;
    let observer_to_fow_clear_low_alpha_pixels = parity_from_observer_to_fow(
        &off_rt,
        &on_rt,
        &off_shadow_bytes,
        &on_shadow_bytes,
        &masks.clear_low_alpha,
        0.0,
    )?;
    let observer_to_fow_fogged_changed_pixels = parity_from_observer_to_fow(
        &off_rt,
        &on_rt,
        &off_shadow_bytes,
        &on_shadow_bytes,
        &masks.fogged_changed,
        0.0,
    )?;
    let adversarial_forced_fog_on_clear_pixels =
        forced_fog_on_clear_parity(&off_rt, &on_rt, &on_shadow_bytes, &masks.clear_low_alpha)?;

    Ok(FowCapturePairReport {
        draw_diff,
        shadow_map,
        observer_to_fow_all_pixels,
        observer_to_fow_clear_low_alpha_pixels,
        observer_to_fow_fogged_changed_pixels,
        adversarial_forced_fog_on_clear_pixels,
    })
}

fn compare_draw_sets(on: &Value, off: &Value) -> Result<FowPairDrawDiffReport> {
    let on_draws = array_field(on, "draws")?;
    let off_draws = array_field(off, "draws")?;
    let mut shared_draw_signatures_by_index = 0usize;
    for (left, right) in on_draws.iter().zip(off_draws.iter()) {
        if draw_signature(left)? == draw_signature(right)? {
            shared_draw_signatures_by_index += 1;
        }
    }

    let on_counts = signature_counts(on_draws)?;
    let off_counts = signature_counts(off_draws)?;
    let on_only_draw_signatures = count_multiset_delta(&on_counts, &off_counts);
    let off_only_draw_signatures = count_multiset_delta(&off_counts, &on_counts);

    let mut non_shadow_srv_diffs = 0usize;
    let mut shadow_srv_diffs = 0usize;
    let mut constant_buffer_diffs = 0usize;
    for (left, right) in on_draws.iter().zip(off_draws.iter()) {
        let left_srvs = resource_hashes(left, "shader_resources")?;
        let right_srvs = resource_hashes(right, "shader_resources")?;
        for (key, left_hash) in &left_srvs {
            let right_hash = right_srvs.get(key);
            if Some(left_hash) != right_hash {
                if key.2.as_deref() == Some(SHADOW_MAP_SYMBOL) {
                    shadow_srv_diffs += 1;
                } else {
                    non_shadow_srv_diffs += 1;
                }
            }
        }
        for key in right_srvs.keys() {
            if !left_srvs.contains_key(key) {
                if key.2.as_deref() == Some(SHADOW_MAP_SYMBOL) {
                    shadow_srv_diffs += 1;
                } else {
                    non_shadow_srv_diffs += 1;
                }
            }
        }

        let left_cbs = resource_hashes(left, "constant_buffers")?;
        let right_cbs = resource_hashes(right, "constant_buffers")?;
        for (key, left_hash) in &left_cbs {
            if Some(left_hash) != right_cbs.get(key) {
                constant_buffer_diffs += 1;
            }
        }
        for key in right_cbs.keys() {
            if !left_cbs.contains_key(key) {
                constant_buffer_diffs += 1;
            }
        }
    }

    Ok(FowPairDrawDiffReport {
        on_draws: on_draws.len(),
        off_draws: off_draws.len(),
        shared_draw_signatures_by_index,
        on_only_draw_signatures,
        off_only_draw_signatures,
        non_shadow_srv_diffs,
        shadow_srv_diffs,
        constant_buffer_diffs,
    })
}

fn draw_signature(draw: &Value) -> Result<String> {
    let mut parts = Vec::new();
    for key in [
        "draw_call",
        "draw_count",
        "draw_start",
        "base_vertex_location",
        "topology",
    ] {
        parts.push(value_key(draw.get(key)));
    }
    parts.push(
        draw.get("shader")
            .and_then(|shader| shader.get("fnv1a64"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    );
    parts.push(
        draw.get("vertex_shader")
            .and_then(|shader| shader.get("fnv1a64"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    );
    parts.push(
        draw.get("index_buffer")
            .and_then(|ib| ib.get("fnv1a64"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    );
    if let Some(vertex_buffers) = draw.get("vertex_buffers").and_then(Value::as_object) {
        for (slot, binding) in vertex_buffers {
            parts.push(format!(
                "vb{slot}:{}:{}:{}",
                binding.get("fnv1a64").and_then(Value::as_str).unwrap_or(""),
                value_key(binding.get("stride")),
                value_key(binding.get("offset"))
            ));
        }
    }
    Ok(parts.join("|"))
}

fn signature_counts(draws: &[Value]) -> Result<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for draw in draws {
        *counts.entry(draw_signature(draw)?).or_insert(0) += 1;
    }
    Ok(counts)
}

fn count_multiset_delta(left: &BTreeMap<String, usize>, right: &BTreeMap<String, usize>) -> usize {
    left.iter()
        .map(|(key, count)| count.saturating_sub(*right.get(key).unwrap_or(&0)))
        .sum()
}

fn resource_hashes(
    draw: &Value,
    field: &str,
) -> Result<BTreeMap<(String, i64, Option<String>), Option<String>>> {
    let mut out = BTreeMap::new();
    for resource in array_field(draw, field)? {
        let stage = resource
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let slot = resource.get("slot").and_then(Value::as_i64).unwrap_or(-1);
        let declared = resource
            .get("declared_name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let hash = resource
            .get("fnv1a64")
            .and_then(Value::as_str)
            .map(str::to_string);
        out.insert((stage, slot, declared), hash);
    }
    Ok(out)
}

fn shadow_resource(bundle: &Value) -> Result<&Value> {
    let first_draw = array_field(bundle, "draws")?
        .first()
        .ok_or("FoW pair bundle has no draws")?;
    array_field(first_draw, "shader_resources")?
        .iter()
        .find(|resource| {
            resource.get("declared_name").and_then(Value::as_str) == Some(SHADOW_MAP_SYMBOL)
        })
        .ok_or("FoW pair terrain draw has no ShadowMap SRV".into())
}

fn shadow_map_report(
    on_shadow: &Value,
    off_shadow: &Value,
    on: &[u8],
    off: &[u8],
) -> Result<FowShadowMapReport> {
    if on.len() != off.len() || on.len() % 4 != 0 {
        return Err("FoW ShadowMap pair must have equal BGRA8 lengths".into());
    }
    let width = usize_field(on_shadow, "width")?;
    let height = usize_field(on_shadow, "height")?;
    let dxgi_format = u32_field(on_shadow, "dxgi_format")?;
    if width != usize_field(off_shadow, "width")?
        || height != usize_field(off_shadow, "height")?
        || dxgi_format != u32_field(off_shadow, "dxgi_format")?
    {
        return Err("FoW ShadowMap descriptors differ beyond payload".into());
    }
    let mut channel_diffs = FowChannelDiffCounts {
        shadow_r: 0,
        fog_color_g: 0,
        fog_alpha_b: 0,
        alpha_a: 0,
    };
    let mut on_fog_alpha_nonzero_pixels = 0usize;
    let mut off_fog_alpha_nonzero_pixels = 0usize;
    for i in (0..on.len()).step_by(4) {
        if on[i] != off[i] {
            channel_diffs.fog_alpha_b += 1;
        }
        if on[i + 1] != off[i + 1] {
            channel_diffs.fog_color_g += 1;
        }
        if on[i + 2] != off[i + 2] {
            channel_diffs.shadow_r += 1;
        }
        if on[i + 3] != off[i + 3] {
            channel_diffs.alpha_a += 1;
        }
        if on[i] != 0 {
            on_fog_alpha_nonzero_pixels += 1;
        }
        if off[i] != 0 {
            off_fog_alpha_nonzero_pixels += 1;
        }
    }
    Ok(FowShadowMapReport {
        width,
        height,
        dxgi_format,
        byte_len: on.len(),
        on_fnv1a64: string_field(on_shadow, "fnv1a64")?.to_string(),
        off_fnv1a64: string_field(off_shadow, "fnv1a64")?.to_string(),
        channel_diffs,
        on_fog_alpha_nonzero_pixels,
        off_fog_alpha_nonzero_pixels,
    })
}

struct PairMasks {
    all: Vec<bool>,
    clear_low_alpha: Vec<bool>,
    fogged_changed: Vec<bool>,
}

fn build_pair_masks(on_shadow: &[u8], off_shadow: &[u8]) -> Result<PairMasks> {
    if on_shadow.len() != off_shadow.len() || on_shadow.len() % 4 != 0 {
        return Err("FoW ShadowMap pair must have equal BGRA8 lengths".into());
    }
    let pixels = on_shadow.len() / 4;
    let mut all = vec![true; pixels];
    let mut clear_low_alpha = Vec::with_capacity(pixels);
    let mut fogged_changed = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let i = pixel * 4;
        let on_alpha = on_shadow[i];
        let off_alpha = off_shadow[i];
        clear_low_alpha.push(on_alpha <= 1 && off_alpha <= 1);
        fogged_changed
            .push(on_alpha > 8 && (i16::from(on_alpha) - i16::from(off_alpha)).unsigned_abs() > 2);
    }
    // Keep `all` explicit so the report makes the mask contract visible.
    all.shrink_to_fit();
    Ok(PairMasks {
        all,
        clear_low_alpha,
        fogged_changed,
    })
}

fn parity_from_observer_to_fow(
    off_rt: &[u8],
    on_rt: &[u8],
    off_shadow: &[u8],
    on_shadow: &[u8],
    mask: &[bool],
    bloom_alpha: f32,
) -> Result<FowParityStats> {
    validate_pair_payload_shapes(off_rt, on_rt, off_shadow, on_shadow, mask)?;
    parity_stats_for_mask(mask, |pixel| {
        let off_sample = shadow_map_bgra8_unorm_sample(&off_shadow[pixel * 4..pixel * 4 + 4])?;
        let on_sample = shadow_map_bgra8_unorm_sample(&on_shadow[pixel * 4..pixel * 4 + 4])?;
        let off_factors = FowFogFactors {
            fog_color_factor: off_sample.fog_color_factor,
            fog_alpha_factor: off_sample.fog_alpha_factor,
        };
        let on_factors = FowFogFactors {
            fog_color_factor: on_sample.fog_color_factor,
            fog_alpha_factor: on_sample.fog_alpha_factor,
        };
        let off_color = read_half_rgb(off_rt, pixel);
        let base = remove_pdxmap_fow_tail(off_color, off_factors, bloom_alpha).unwrap_or(off_color);
        let predicted = apply_pdxmap_fow_tail(base, on_factors, bloom_alpha);
        Ok((predicted, read_half_rgb(on_rt, pixel)))
    })
}

fn forced_fog_on_clear_parity(
    off_rt: &[u8],
    on_rt: &[u8],
    on_shadow: &[u8],
    mask: &[bool],
) -> Result<FowParityStats> {
    let forced = FowFogFactors {
        fog_color_factor: 1.0,
        fog_alpha_factor: 1.0,
    };
    let dummy_shadow = on_shadow;
    validate_pair_payload_shapes(off_rt, on_rt, dummy_shadow, dummy_shadow, mask)?;
    parity_stats_for_mask(mask, |pixel| {
        let predicted = apply_pdxmap_fow_tail(read_half_rgb(off_rt, pixel), forced, 0.0);
        Ok((predicted, read_half_rgb(on_rt, pixel)))
    })
}

fn parity_stats_for_mask<F>(mask: &[bool], mut values: F) -> Result<FowParityStats>
where
    F: FnMut(usize) -> Result<(Vec3, Vec3)>,
{
    let pixels = mask.iter().filter(|selected| **selected).count();
    if pixels == 0 {
        return Err("FoW parity mask selected zero pixels".into());
    }
    let mut exact_half_channels = 0usize;
    let mut near_one_ulp_half_channels = 0usize;
    let mut errors = Vec::with_capacity(pixels * 3);
    for (pixel, selected) in mask.iter().copied().enumerate() {
        if !selected {
            continue;
        }
        let (predicted, observed) = values(pixel)?;
        for (predicted, observed) in [
            (predicted.x, observed.x),
            (predicted.y, observed.y),
            (predicted.z, observed.z),
        ] {
            let predicted_bits = f32_to_half_float(predicted);
            let observed_bits = f32_to_half_float(observed);
            let predicted_quantized = half_float_to_f32(predicted_bits);
            let bit_delta = predicted_bits.abs_diff(observed_bits);
            if bit_delta == 0 {
                exact_half_channels += 1;
            }
            if bit_delta <= 1 {
                near_one_ulp_half_channels += 1;
            }
            errors.push((predicted_quantized - observed).abs());
        }
    }
    errors.sort_by(|a, b| a.total_cmp(b));
    let channels = errors.len();
    let mean_abs = errors.iter().sum::<f32>() / channels as f32;
    Ok(FowParityStats {
        pixels,
        channels,
        exact_half_channels,
        near_one_ulp_half_channels,
        mean_abs,
        max_abs: *errors.last().unwrap_or(&0.0),
        p95_abs: quantile_sorted(&errors, 0.95),
        p99_abs: quantile_sorted(&errors, 0.99),
    })
}

fn validate_pair_payload_shapes(
    off_rt: &[u8],
    on_rt: &[u8],
    off_shadow: &[u8],
    on_shadow: &[u8],
    mask: &[bool],
) -> Result<()> {
    if off_rt.len() != on_rt.len() || off_rt.len() % 8 != 0 {
        return Err("FoW render-target pair must have equal R16G16B16A16_FLOAT lengths".into());
    }
    if off_shadow.len() != on_shadow.len() || off_shadow.len() % 4 != 0 {
        return Err("FoW ShadowMap pair must have equal BGRA8 lengths".into());
    }
    let pixels = off_rt.len() / 8;
    if pixels != off_shadow.len() / 4 || pixels != mask.len() {
        return Err("FoW RT, ShadowMap, and mask pixel counts differ".into());
    }
    Ok(())
}

fn read_half_rgb(bytes: &[u8], pixel: usize) -> Vec3 {
    let base = pixel * 8;
    Vec3 {
        x: half_float_to_f32(u16::from_le_bytes([bytes[base], bytes[base + 1]])),
        y: half_float_to_f32(u16::from_le_bytes([bytes[base + 2], bytes[base + 3]])),
        z: half_float_to_f32(u16::from_le_bytes([bytes[base + 4], bytes[base + 5]])),
    }
}

fn post_pass_render_target_bytes(bundle_dir: &Path, bundle: &Value) -> Result<Vec<u8>> {
    let payload = bundle
        .get("post_pass_output_merger")
        .and_then(|om| om.get("render_targets"))
        .and_then(Value::as_array)
        .and_then(|targets| targets.first())
        .and_then(|target| target.get("post_pass_payload"))
        .ok_or("FoW pair bundle has no post-pass render-target payload")?;
    Ok(fs::read(bundle_dir.join(string_field(payload, "file")?))?)
}

fn array_field<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing JSON array field {field}").into())
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing JSON string field {field}").into())
}

fn usize_field(value: &Value, field: &str) -> Result<usize> {
    let raw = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing JSON usize field {field}"))?;
    usize::try_from(raw).map_err(|_| format!("JSON field {field} overflows usize").into())
}

fn u32_field(value: &Value, field: &str) -> Result<u32> {
    let raw = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing JSON u32 field {field}"))?;
    u32::try_from(raw).map_err(|_| format!("JSON field {field} overflows u32").into())
}

fn value_key(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

fn quantile_sorted(values: &[f32], quantile: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let idx = ((values.len() - 1) as f32 * quantile).round() as usize;
    values[idx.min(values.len() - 1)]
}

fn percent(part: usize, whole: usize) -> f32 {
    if whole == 0 {
        0.0
    } else {
        part as f32 * 100.0 / whole as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's gate table, included so this test module resolves the CAP4
    /// corpus through EXACTLY the candidate list the build-time probe used.
    /// Two copies of a candidate list drift; one cannot.
    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    #[test]
    fn apply_fow_matches_shader_closed_form() {
        let color = Vec3 {
            x: 0.25,
            y: 0.5,
            z: 0.75,
        };
        let clear = apply_fow(
            color,
            FowFogFactors {
                fog_color_factor: 0.5,
                fog_alpha_factor: 0.0,
            },
        );
        assert_eq!(clear, color, "alpha 0 must be a byte-safe no-op");

        let full = apply_fow(
            color,
            FowFogFactors {
                fog_color_factor: 1.0,
                fog_alpha_factor: 1.0,
            },
        );
        assert_eq!(full, BRIGHT_FOW_COLOR);

        let half = apply_fow(
            color,
            FowFogFactors {
                fog_color_factor: 0.0,
                fog_alpha_factor: 0.5,
            },
        );
        let expected = lerp_vec3(color, DARK_FOW_COLOR, 0.5);
        assert_eq!(half, expected);
    }

    #[test]
    fn shadow_map_bgra_decode_maps_to_shader_channels() {
        let sample = shadow_map_bgra8_unorm_sample(&[64, 128, 255, 7]).unwrap();
        assert_eq!(sample.shadow_factor, 1.0);
        assert!((sample.fog_color_factor - (128.0 / 255.0)).abs() < 1e-7);
        assert!((sample.fog_alpha_factor - (64.0 / 255.0)).abs() < 1e-7);
    }

    #[test]
    fn pdxmap_border_fow_removal_uses_file_derived_factor() {
        assert!((pdxmap_fow_effective_alpha(0.5, 0.0) - 0.1).abs() < 1e-6);
        assert!((pdxmap_fow_effective_alpha(0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    #[cfg_attr(
        not(gate_wave1_cap4_fow_pair),
        ignore = "BATTLE_PLAN_WAVE1_CAP4_ROUNDS naming a capture corpus with fow_on_pov_front and fow_off_observer_front"
    )]
    fn gate_wave1_cap4_fow_pair_carrier_and_parity() {
        let root = corpus::wave1_cap4_rounds_dir(&["fow_on_pov_front", "fow_off_observer_front"])
            .expect(
                "the CAP4 FoW round pair (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved copy) is \
                 absent, but this test was compiled as runnable",
            );

        let report = analyze_wave1_cap4_fow_pair(&root).expect("analyze CAP4 FoW pair");
        eprintln!("CAP4 FoW report: {report:#?}");

        assert_eq!(report.draw_diff.on_draws, 15);
        assert_eq!(report.draw_diff.off_draws, 15);
        assert_eq!(report.draw_diff.shared_draw_signatures_by_index, 15);
        assert_eq!(report.draw_diff.on_only_draw_signatures, 0);
        assert_eq!(report.draw_diff.off_only_draw_signatures, 0);
        assert_eq!(report.draw_diff.non_shadow_srv_diffs, 0);
        assert_eq!(report.draw_diff.shadow_srv_diffs, 15);

        assert_eq!(report.shadow_map.width, 1920);
        assert_eq!(report.shadow_map.height, 1080);
        assert_eq!(report.shadow_map.dxgi_format, SHADOW_MAP_DXGI_FORMAT);
        assert_eq!(report.shadow_map.byte_len, 8_294_400);
        assert_eq!(
            report.shadow_map.on_fnv1a64,
            WAVE1_CAP4_FOW_ON_SHADOWMAP_FNV1A64
        );
        assert_eq!(
            report.shadow_map.off_fnv1a64,
            WAVE1_CAP4_FOW_OFF_SHADOWMAP_FNV1A64
        );
        assert_eq!(report.shadow_map.channel_diffs.fog_alpha_b, 547_006);
        assert_eq!(report.shadow_map.channel_diffs.fog_color_g, 2_018_110);
        assert_eq!(report.shadow_map.channel_diffs.shadow_r, 10_813);
        assert_eq!(report.shadow_map.channel_diffs.alpha_a, 0);

        assert_eq!(report.observer_to_fow_all_pixels.pixels, 2_073_600);
        assert!(
            report.observer_to_fow_all_pixels.mean_abs < 0.0012,
            "full-frame transfer residual drifted: {:?}",
            report.observer_to_fow_all_pixels
        );
        assert!(
            report
                .observer_to_fow_clear_low_alpha_pixels
                .exact_percent()
                > 94.0,
            "clear low-alpha pixels should remain near-byte identical"
        );
        assert!(
            report.observer_to_fow_fogged_changed_pixels.mean_abs > 0.004,
            "changed/fogged pixels unexpectedly became byte-close; re-check missing pre-FoW/vBloomAlpha evidence"
        );
        assert!(
            report.adversarial_forced_fog_on_clear_pixels.mean_abs > 0.08,
            "forcing full fog onto clear pixels should not match the observed clear region"
        );
    }
}
