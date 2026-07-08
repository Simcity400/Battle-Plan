use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const RESTORE_MAIN_SCENE: &str = "MainScene";
const RESTORE_BLOOM: &str = "RestoreBloom";
const RESTORE_COLOR_CUBE: &str = "ColorCube";
const RESTORE_AVERAGE_LUMINANCE: &str = "AverageLuminanceTexture";
pub const RESTORE_PASS_BUNDLE_FILE: &str = "map_frame_restore_pass_bundle.json";
const RESTORE_PASS_MOMENT: &str = "restore_pass";
const RESTORE_POSTEFFECT_CB_BYTES: usize = 80;
const RESTORE_COLOR_CUBE_WIDTH: u32 = 1024;
const RESTORE_COLOR_CUBE_HEIGHT: u32 = 32;
const RESTORE_COLOR_CUBE_BYTES: usize = 1024 * 32 * 4;
const RESTORE_AVERAGE_LUMINANCE_WIDTH: u32 = 1;
const RESTORE_AVERAGE_LUMINANCE_HEIGHT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneEvidenceProbeReport {
    pub session_dir: PathBuf,
    pub checked_bundles: Vec<String>,
    pub missing_bundles: Vec<String>,
    pub evidence: Option<RestoreSceneEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneEvidence {
    pub bundle_file: String,
    pub draw_index: u32,
    pub draw_call: String,
    pub match_reasons: Vec<String>,
    pub uniforms: Option<RestoreSceneUniforms>,
    pub average_luminance: Option<f32>,
    pub color_cube_payload: Option<RestoreSceneTexturePayloadSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneUniforms {
    pub hsv: [f32; 4],
    pub color_balance: [f32; 3],
    pub emissive_bloom_strength: f32,
    pub inv_down_sample_size: [f32; 2],
    pub inv_window_size: [f32; 2],
    pub bloom_to_screen_scale: [f32; 2],
    pub bright_threshold: f32,
    pub middle_grey: f32,
    pub lum_white2: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneTexturePayloadSummary {
    pub slot: u32,
    pub declared_name: String,
    pub file: Option<String>,
    pub byte_len: usize,
    pub fnv1a64: Option<String>,
    pub dxgi_format: u32,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub payload_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneBundle {
    pub bundle_file: String,
    pub draw_index: u32,
    pub draw_call: String,
    pub color_cube: RestoreSceneColorCube,
    pub posteffect_cb: RestoreSceneUniforms,
    pub average_luminance: RestoreSceneAverageLuminance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneColorCube {
    pub slot: u32,
    pub declared_name: String,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub fnv1a64: Option<String>,
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreSceneAverageLuminance {
    pub slot: u32,
    pub declared_name: String,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub value: f32,
    pub raw: Vec<u8>,
}

pub fn load_restore_scene_bundle_session_dir(
    session_dir: &Path,
) -> Result<Option<RestoreSceneBundle>> {
    let bundle_path = session_dir.join(RESTORE_PASS_BUNDLE_FILE);
    if !bundle_path.is_file() {
        return Ok(None);
    }
    Ok(Some(load_restore_scene_bundle(&bundle_path)?))
}

pub fn load_restore_scene_bundle(bundle_path: &Path) -> Result<RestoreSceneBundle> {
    let bundle_dir = bundle_path.parent().unwrap_or_else(|| Path::new("."));
    let bundle: MapFrameRuntimeBundle = serde_json::from_slice(&fs::read(bundle_path)?)?;
    if bundle.schema != MAP_FRAME_BUNDLE_SCHEMA {
        return Err(format!(
            "restore-pass bundle {} has schema {}, expected {}",
            bundle_path.display(),
            bundle.schema,
            MAP_FRAME_BUNDLE_SCHEMA
        )
        .into());
    }
    if bundle.capture_moment != RESTORE_PASS_MOMENT {
        return Err(format!(
            "restore-pass bundle {} has capture_moment {}, expected {}",
            bundle_path.display(),
            bundle.capture_moment,
            RESTORE_PASS_MOMENT
        )
        .into());
    }
    // Codex review 2026-07-06: a blocked/in-progress producer snapshot can
    // carry blockers (and accumulate extra draws) while every field this
    // loader reads still parses — grading must never run from incomplete
    // capture evidence, so any recorded blocker fails the whole bundle.
    if !bundle.blockers.is_empty() {
        return Err(format!(
            "restore-pass bundle {} has {} capture blocker(s), refusing incomplete evidence: {}",
            bundle_path.display(),
            bundle.blockers.len(),
            bundle.blockers.join("; ")
        )
        .into());
    }
    if bundle.draws.len() != 1 {
        return Err(format!(
            "restore-pass bundle {} has {} draw record(s), expected 1 Draw:3 restore draw",
            bundle_path.display(),
            bundle.draws.len()
        )
        .into());
    }
    let draw = &bundle.draws[0];
    if draw.draw_call != "Draw" || draw.draw_count != 3 || draw.draw_start != 0 {
        return Err(format!(
            "restore-pass draw {} is {}:{} start {}, expected Draw:3 start 0",
            draw.draw_index, draw.draw_call, draw.draw_count, draw.draw_start
        )
        .into());
    }

    require_restore_resource(draw, RESTORE_MAIN_SCENE, 0)?;
    require_restore_resource(draw, RESTORE_BLOOM, 1)?;
    let color_cube = load_restore_color_cube(bundle_dir, draw)?;
    let average_luminance = load_restore_average_luminance(bundle_dir, draw)?;
    let posteffect_cb = load_restore_posteffect_cb(bundle_dir, draw)?;

    Ok(RestoreSceneBundle {
        bundle_file: bundle_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| bundle_path.display().to_string()),
        draw_index: draw.draw_index,
        draw_call: draw.draw_call.clone(),
        color_cube,
        posteffect_cb,
        average_luminance,
    })
}

pub fn probe_restore_scene_evidence_session_dir(
    session_dir: &Path,
) -> Result<RestoreSceneEvidenceProbeReport> {
    let bundle_files = restore_probe_bundle_files(session_dir)?;
    let mut checked_bundles = Vec::new();
    let mut missing_bundles = Vec::new();
    for bundle_file in bundle_files {
        let bundle_path = session_dir.join(&bundle_file);
        if !bundle_path.is_file() {
            missing_bundles.push(bundle_file);
            continue;
        }
        checked_bundles.push(bundle_file.clone());
        let bundle: MapFrameRuntimeBundle = serde_json::from_slice(&fs::read(&bundle_path)?)?;
        if let Some(evidence) =
            probe_restore_scene_evidence_bundle(session_dir, &bundle_file, &bundle)?
        {
            return Ok(RestoreSceneEvidenceProbeReport {
                session_dir: session_dir.to_path_buf(),
                checked_bundles,
                missing_bundles,
                evidence: Some(evidence),
            });
        }
    }
    Ok(RestoreSceneEvidenceProbeReport {
        session_dir: session_dir.to_path_buf(),
        checked_bundles,
        missing_bundles,
        evidence: None,
    })
}

pub(crate) fn probe_restore_scene_evidence_bundle(
    bundle_dir: &Path,
    bundle_file: &str,
    bundle: &MapFrameRuntimeBundle,
) -> Result<Option<RestoreSceneEvidence>> {
    for draw in bundle
        .draws
        .iter()
        .chain(std::iter::once(&bundle.active_draw))
    {
        if let Some(evidence) = probe_restore_scene_draw(bundle_dir, bundle_file, draw)? {
            return Ok(Some(evidence));
        }
    }
    Ok(None)
}

fn restore_probe_bundle_files(session_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let manifest_path = session_dir.join("map_capture_session.json");
    if manifest_path.is_file() {
        let manifest: MapCaptureSession = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        for moment in manifest.moments {
            push_unique(&mut out, &mut seen, moment.bundle_file);
        }
    }
    push_unique(&mut out, &mut seen, RESTORE_PASS_BUNDLE_FILE.to_string());
    push_unique(&mut out, &mut seen, "map_frame_bundle.json".to_string());
    if session_dir.is_dir() {
        for entry in fs::read_dir(session_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with("_bundle.json") || name == "map_frame_bundle.json" {
                push_unique(&mut out, &mut seen, name);
            }
        }
    }
    Ok(out)
}

fn push_unique(out: &mut Vec<String>, seen: &mut BTreeSet<String>, value: String) {
    if seen.insert(value.clone()) {
        out.push(value);
    }
}

fn probe_restore_scene_draw(
    bundle_dir: &Path,
    bundle_file: &str,
    draw: &MapFrameDraw,
) -> Result<Option<RestoreSceneEvidence>> {
    let mut reasons = Vec::new();
    if restore_resource_slots_match(draw) {
        reasons.push(
            "PS SRV slots match restorescene MainScene/RestoreBloom/ColorCube/AverageLuminanceTexture 0-3"
                .to_string(),
        );
    }
    if draw
        .shader
        .source_evidence
        .iter()
        .any(|source| source.eq_ignore_ascii_case("gfx/FX/restorescene.shader"))
    {
        reasons.push("pixel shader source evidence is gfx/FX/restorescene.shader".to_string());
    }
    let uniforms = restore_posteffect_uniforms(bundle_dir, draw)?;
    if let Some(uniforms) = uniforms {
        reasons.push("PS constant buffer slot 1 decodes as posteffect_base layout".to_string());
        if reasons.is_empty() {
            return Ok(None);
        }
        Ok(Some(RestoreSceneEvidence {
            bundle_file: bundle_file.to_string(),
            draw_index: draw.draw_index,
            draw_call: draw.draw_call.clone(),
            match_reasons: reasons,
            uniforms: Some(uniforms),
            average_luminance: restore_average_luminance(bundle_dir, draw)?,
            color_cube_payload: restore_texture_payload(draw, RESTORE_COLOR_CUBE),
        }))
    } else if reasons.is_empty() {
        Ok(None)
    } else {
        Ok(Some(RestoreSceneEvidence {
            bundle_file: bundle_file.to_string(),
            draw_index: draw.draw_index,
            draw_call: draw.draw_call.clone(),
            match_reasons: reasons,
            uniforms: None,
            average_luminance: restore_average_luminance(bundle_dir, draw)?,
            color_cube_payload: restore_texture_payload(draw, RESTORE_COLOR_CUBE),
        }))
    }
}

fn restore_resource_slots_match(draw: &MapFrameDraw) -> bool {
    let expected = BTreeMap::from([
        (RESTORE_MAIN_SCENE, 0),
        (RESTORE_BLOOM, 1),
        (RESTORE_COLOR_CUBE, 2),
        (RESTORE_AVERAGE_LUMINANCE, 3),
    ]);
    expected.iter().all(|(name, slot)| {
        draw.shader_resources.iter().any(|resource| {
            resource.stage == "PS"
                && resource.slot == *slot
                && resource
                    .declared_name
                    .as_deref()
                    .map(|actual| actual.eq_ignore_ascii_case(name))
                    .unwrap_or(false)
        })
    })
}

fn restore_posteffect_uniforms(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<Option<RestoreSceneUniforms>> {
    let Some(binding) = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "PS" && binding.slot == 1 && binding.byte_len >= 68)
    else {
        return Ok(None);
    };
    let Some(file) = binding.file.as_ref() else {
        return Ok(None);
    };
    let bytes = fs::read(bundle_dir.join(file))?;
    if bytes.len() != binding.byte_len || bytes.len() < 68 {
        return Ok(None);
    }
    let uniforms = decode_restore_posteffect_uniforms(&bytes)?;
    Ok(restore_uniforms_plausible(&uniforms).then_some(uniforms))
}

fn restore_uniforms_plausible(uniforms: &RestoreSceneUniforms) -> bool {
    let all_values = uniforms
        .hsv
        .into_iter()
        .chain(uniforms.color_balance)
        .chain([uniforms.emissive_bloom_strength])
        .chain(uniforms.inv_down_sample_size)
        .chain(uniforms.inv_window_size)
        .chain(uniforms.bloom_to_screen_scale)
        .chain([
            uniforms.bright_threshold,
            uniforms.middle_grey,
            uniforms.lum_white2,
        ]);
    if all_values.into_iter().any(|value| !value.is_finite()) {
        return false;
    }
    (-6.0..=6.0).contains(&uniforms.hsv[0])
        && (0.0..=4.0).contains(&uniforms.hsv[1])
        && (0.0..=4.0).contains(&uniforms.hsv[2])
        && (0.0..=1.0).contains(&uniforms.hsv[3])
        && uniforms
            .color_balance
            .iter()
            .all(|value| (0.0..=4.0).contains(value) && *value > 0.0)
        && uniforms.middle_grey > 0.0
        && uniforms.middle_grey <= 10.0
        && uniforms.lum_white2 > 0.0
        && uniforms.lum_white2 <= 1000.0
}

fn restore_average_luminance(bundle_dir: &Path, draw: &MapFrameDraw) -> Result<Option<f32>> {
    let Some(binding) = draw.shader_resources.iter().find(|resource| {
        resource.stage == "PS"
            && resource.slot == 3
            && resource
                .declared_name
                .as_deref()
                .map(|name| name.eq_ignore_ascii_case(RESTORE_AVERAGE_LUMINANCE))
                .unwrap_or(false)
    }) else {
        return Ok(None);
    };
    let Some(file) = binding.file.as_ref() else {
        return Ok(None);
    };
    let bytes = fs::read(bundle_dir.join(file))?;
    if bytes.len() < 4 || binding.byte_len < 4 {
        return Ok(None);
    }
    let offset = binding
        .subresources
        .first()
        .map(|subresource| subresource.file_offset)
        .unwrap_or(0);
    if offset
        .checked_add(4)
        .map(|end| end <= bytes.len())
        .unwrap_or(false)
    {
        let value = read_f32_le(&bytes, offset)?;
        if value.is_finite() && value > 0.0 {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn restore_texture_payload(
    draw: &MapFrameDraw,
    declared_name: &str,
) -> Option<RestoreSceneTexturePayloadSummary> {
    draw.shader_resources
        .iter()
        .find(|resource| {
            resource.stage == "PS"
                && resource
                    .declared_name
                    .as_deref()
                    .map(|actual| actual.eq_ignore_ascii_case(declared_name))
                    .unwrap_or(false)
        })
        .map(|resource| RestoreSceneTexturePayloadSummary {
            slot: resource.slot,
            declared_name: declared_name.to_string(),
            file: resource.file.clone(),
            byte_len: resource.byte_len,
            fnv1a64: resource.fnv1a64.clone(),
            dxgi_format: resource.dxgi_format,
            width: resource.width,
            height: resource.height,
            mip_count: resource.mip_count,
            payload_format: resource.payload_format.clone(),
        })
}

fn read_f32_le(bytes: &[u8], offset: usize) -> Result<f32> {
    let end = offset
        .checked_add(4)
        .ok_or("restore evidence f32 offset overflow")?;
    let chunk = bytes
        .get(offset..end)
        .ok_or("restore evidence f32 offset is outside the buffer")?;
    Ok(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

fn load_restore_posteffect_cb(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<RestoreSceneUniforms> {
    let binding = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "PS" && binding.slot == 1)
        .ok_or("restore-pass bundle is missing PS slot 1 PostEffectBase constant buffer")?;
    // Live 2026-07-06 (restore_gate_1): the shipped compiled restorescene PS
    // reflects its cbuffer as "dx11_cb1"; the harness canonical name is
    // "PostEffectBase". Slot 1 + 80 bytes + the sampler-signature-identified
    // shader pin the identity; the reflected NAME is metadata, so both
    // observed spellings are accepted and anything else fails closed.
    if binding
        .declared_name
        .as_deref()
        .map(|name| {
            !name.eq_ignore_ascii_case("PostEffectBase") && !name.eq_ignore_ascii_case("dx11_cb1")
        })
        .unwrap_or(true)
    {
        return Err(format!(
            "restore-pass PS slot 1 constant buffer is {:?}, expected PostEffectBase or dx11_cb1",
            binding.declared_name
        )
        .into());
    }
    if binding.byte_len != RESTORE_POSTEFFECT_CB_BYTES {
        return Err(format!(
            "restore-pass PostEffectBase constant buffer has {} bytes, expected {}",
            binding.byte_len, RESTORE_POSTEFFECT_CB_BYTES
        )
        .into());
    }
    let file = binding
        .file
        .as_ref()
        .ok_or("restore-pass PostEffectBase constant buffer has no payload file")?;
    let path = bundle_dir.join(file);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "restore-pass PostEffectBase constant buffer {} is not readable: {}",
            path.display(),
            error
        )
    })?;
    if bytes.len() != RESTORE_POSTEFFECT_CB_BYTES {
        return Err(format!(
            "restore-pass PostEffectBase constant buffer {} has {} bytes, expected {}",
            path.display(),
            bytes.len(),
            RESTORE_POSTEFFECT_CB_BYTES
        )
        .into());
    }
    verify_optional_hash(
        &path,
        binding.fnv1a64.as_ref(),
        "restore-pass PostEffectBase constant buffer",
    )?;
    let uniforms = decode_restore_posteffect_uniforms(&bytes)?;
    if !restore_uniforms_finite(&uniforms) {
        return Err(
            "restore-pass PostEffectBase constant buffer contains a non-finite float".into(),
        );
    }
    Ok(uniforms)
}

fn decode_restore_posteffect_uniforms(bytes: &[u8]) -> Result<RestoreSceneUniforms> {
    if bytes.len() < 68 {
        return Err(format!(
            "restore-pass PostEffectBase constant buffer has {} bytes, expected at least 68",
            bytes.len()
        )
        .into());
    }
    Ok(RestoreSceneUniforms {
        hsv: [
            read_f32_le(bytes, 0)?,
            read_f32_le(bytes, 4)?,
            read_f32_le(bytes, 8)?,
            read_f32_le(bytes, 12)?,
        ],
        color_balance: [
            read_f32_le(bytes, 16)?,
            read_f32_le(bytes, 20)?,
            read_f32_le(bytes, 24)?,
        ],
        emissive_bloom_strength: read_f32_le(bytes, 28)?,
        inv_down_sample_size: [read_f32_le(bytes, 32)?, read_f32_le(bytes, 36)?],
        inv_window_size: [read_f32_le(bytes, 40)?, read_f32_le(bytes, 44)?],
        bloom_to_screen_scale: [read_f32_le(bytes, 48)?, read_f32_le(bytes, 52)?],
        bright_threshold: read_f32_le(bytes, 56)?,
        middle_grey: read_f32_le(bytes, 60)?,
        lum_white2: read_f32_le(bytes, 64)?,
    })
}

fn restore_uniforms_finite(uniforms: &RestoreSceneUniforms) -> bool {
    uniforms
        .hsv
        .into_iter()
        .chain(uniforms.color_balance)
        .chain([uniforms.emissive_bloom_strength])
        .chain(uniforms.inv_down_sample_size)
        .chain(uniforms.inv_window_size)
        .chain(uniforms.bloom_to_screen_scale)
        .chain([
            uniforms.bright_threshold,
            uniforms.middle_grey,
            uniforms.lum_white2,
        ])
        .all(f32::is_finite)
}

fn load_restore_color_cube(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<RestoreSceneColorCube> {
    let resource = require_restore_resource(draw, RESTORE_COLOR_CUBE, 2)?;
    // Live 2026-07-06 (restore_gate_1): the engine's composed ColorCube is
    // DXGI 88 (B8G8R8X8_UNORM); harness stand-ins are 28 (R8G8B8A8_UNORM).
    validate_restore_texture_metadata(
        resource,
        RESTORE_COLOR_CUBE,
        &[DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM],
        RESTORE_COLOR_CUBE_WIDTH,
        RESTORE_COLOR_CUBE_HEIGHT,
    )?;
    native_d3d11_validate_texture_payload_shape(resource).map_err(|error| {
        format!(
            "restore-pass ColorCube D3D11 payload metadata is invalid: {}",
            error
        )
    })?;
    let row_bytes = RESTORE_COLOR_CUBE_WIDTH
        .checked_mul(4)
        .ok_or("restore-pass ColorCube row byte width overflow")?;
    let subresource = single_restore_subresource(resource, RESTORE_COLOR_CUBE)?;
    if subresource.file_offset != 0
        || subresource.byte_len != RESTORE_COLOR_CUBE_BYTES
        || subresource.row_pitch != row_bytes
        || subresource.depth_pitch != RESTORE_COLOR_CUBE_BYTES as u32
    {
        return Err(format!(
            "restore-pass ColorCube subresource is offset {} byte_len {} row_pitch {} depth_pitch {}, expected offset 0 byte_len {} row_pitch {} depth_pitch {} for 1024x32 RGBA8",
            subresource.file_offset,
            subresource.byte_len,
            subresource.row_pitch,
            subresource.depth_pitch,
            RESTORE_COLOR_CUBE_BYTES,
            row_bytes,
            RESTORE_COLOR_CUBE_BYTES
        )
        .into());
    }
    let (file, raw) = read_restore_texture_file(
        bundle_dir,
        resource,
        RESTORE_COLOR_CUBE,
        RESTORE_COLOR_CUBE_BYTES,
        "1024x32 RGBA8",
    )?;
    let rgba8 = if resource.dxgi_format == DXGI_FORMAT_B8G8R8X8_UNORM {
        bgrx8_to_rgba8(&raw)
    } else {
        raw
    };
    Ok(RestoreSceneColorCube {
        slot: resource.slot,
        declared_name: RESTORE_COLOR_CUBE.to_string(),
        file,
        width: resource.width,
        height: resource.height,
        dxgi_format: resource.dxgi_format,
        fnv1a64: resource.fnv1a64.clone(),
        rgba8,
    })
}

/// B8G8R8X8_UNORM texels to canonical RGBA8: swap B/R, force opaque alpha
/// (the X channel carries undefined data by contract).
fn bgrx8_to_rgba8(raw: &[u8]) -> Vec<u8> {
    let mut out = raw.to_vec();
    for texel in out.chunks_exact_mut(4) {
        texel.swap(0, 2);
        texel[3] = 0xFF;
    }
    out
}

fn load_restore_average_luminance(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
) -> Result<RestoreSceneAverageLuminance> {
    let resource = require_restore_resource(draw, RESTORE_AVERAGE_LUMINANCE, 3)?;
    // Live 2026-07-06 (restore_gate_1): the engine's average-luminance
    // texture is DXGI 54 (R16_FLOAT); harness stand-ins are 41 (R32_FLOAT).
    validate_restore_texture_metadata(
        resource,
        RESTORE_AVERAGE_LUMINANCE,
        &[DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R16_FLOAT],
        RESTORE_AVERAGE_LUMINANCE_WIDTH,
        RESTORE_AVERAGE_LUMINANCE_HEIGHT,
    )?;
    let value_bytes: usize = if resource.dxgi_format == DXGI_FORMAT_R16_FLOAT {
        2
    } else {
        4
    };
    let subresource = single_restore_subresource(resource, RESTORE_AVERAGE_LUMINANCE)?;
    if (subresource.row_pitch as usize) < value_bytes || subresource.byte_len < value_bytes {
        return Err(format!(
            "restore-pass AverageLuminanceTexture subresource has row_pitch {} byte_len {}, expected at least {} bytes for its 1x1 float texel",
            subresource.row_pitch, subresource.byte_len, value_bytes
        )
        .into());
    }
    let file = resource
        .file
        .as_ref()
        .ok_or("restore-pass AverageLuminanceTexture has no payload file")?;
    let path = bundle_dir.join(file);
    let raw = fs::read(&path).map_err(|error| {
        format!(
            "restore-pass AverageLuminanceTexture payload {} is not readable: {}",
            path.display(),
            error
        )
    })?;
    if raw.len() != resource.byte_len {
        return Err(format!(
            "restore-pass AverageLuminanceTexture payload {} has {} bytes, metadata expects {} bytes",
            path.display(),
            raw.len(),
            resource.byte_len
        )
        .into());
    }
    verify_optional_hash(
        &path,
        resource.fnv1a64.as_ref(),
        "restore-pass AverageLuminanceTexture payload",
    )?;
    let value_offset = subresource.file_offset;
    if value_offset
        .checked_add(value_bytes)
        .map(|end| end <= raw.len())
        .unwrap_or(false)
    {
        let value = if resource.dxgi_format == DXGI_FORMAT_R16_FLOAT {
            f16_bits_to_f32(u16::from_le_bytes([
                raw[value_offset],
                raw[value_offset + 1],
            ]))
        } else {
            read_f32_le(&raw, value_offset)?
        };
        if !value.is_finite() {
            return Err(
                "restore-pass AverageLuminanceTexture first float texel value is non-finite".into(),
            );
        }
        Ok(RestoreSceneAverageLuminance {
            slot: resource.slot,
            declared_name: RESTORE_AVERAGE_LUMINANCE.to_string(),
            file: file.clone(),
            width: resource.width,
            height: resource.height,
            dxgi_format: resource.dxgi_format,
            value,
            raw,
        })
    } else {
        Err(format!(
            "restore-pass AverageLuminanceTexture payload {} has no readable float texel at subresource offset {}",
            path.display(),
            value_offset
        )
        .into())
    }
}

/// IEEE 754 half-precision bits to f32 (R16_FLOAT texel decode).
fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x3FF);
    let magnitude = match (exponent, mantissa) {
        (0, 0) => 0,
        (0, m) => {
            // Subnormal half: renormalize into the f32 exponent range.
            let shift = m.leading_zeros() - 21;
            let m = (m << (shift + 1)) & 0x3FF;
            ((113 - shift) << 23) | (m << 13)
        }
        (0x1F, 0) => 0xFF << 23,
        (0x1F, m) => (0xFF << 23) | (m << 13),
        (e, m) => ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(sign | magnitude)
}

fn require_restore_resource<'a>(
    draw: &'a MapFrameDraw,
    declared_name: &str,
    slot: u32,
) -> Result<&'a MapFrameTextureBinding> {
    draw.shader_resources
        .iter()
        .find(|resource| {
            resource.stage == "PS"
                && resource.slot == slot
                && resource
                    .declared_name
                    .as_deref()
                    .map(|actual| actual.eq_ignore_ascii_case(declared_name))
                    .unwrap_or(false)
        })
        .ok_or_else(|| {
            format!("restore-pass bundle is missing PS slot {slot} shader resource {declared_name}")
                .into()
        })
}

fn validate_restore_texture_metadata(
    resource: &MapFrameTextureBinding,
    role: &str,
    accepted_formats: &[u32],
    expected_width: u32,
    expected_height: u32,
) -> Result<()> {
    if resource.is_null {
        return Err(format!("restore-pass {role} is null, expected captured texture").into());
    }
    if !accepted_formats.contains(&resource.dxgi_format) {
        return Err(format!(
            "restore-pass {role} DXGI format is {}, expected one of {:?}",
            resource.dxgi_format, accepted_formats
        )
        .into());
    }
    if resource.width != expected_width || resource.height != expected_height {
        return Err(format!(
            "restore-pass {role} dimensions are {}x{}, expected {}x{}",
            resource.width, resource.height, expected_width, expected_height
        )
        .into());
    }
    if resource.mip_count != 1 {
        return Err(format!(
            "restore-pass {role} mip_count is {}, expected 1",
            resource.mip_count
        )
        .into());
    }
    if resource.array_size != 1 {
        return Err(format!(
            "restore-pass {role} array_size is {}, expected 1",
            resource.array_size
        )
        .into());
    }
    if resource.sample_count != 1 {
        return Err(format!(
            "restore-pass {role} sample_count is {}, expected 1",
            resource.sample_count
        )
        .into());
    }
    if resource.payload_format.as_deref() != Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT) {
        return Err(format!(
            "restore-pass {role} payload_format is {:?}, expected {}",
            resource.payload_format, NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT
        )
        .into());
    }
    Ok(())
}

fn single_restore_subresource<'a>(
    resource: &'a MapFrameTextureBinding,
    role: &str,
) -> Result<&'a MapFrameTextureSubresource> {
    if resource.subresources.len() != 1 {
        return Err(format!(
            "restore-pass {role} has {} subresource record(s), expected 1",
            resource.subresources.len()
        )
        .into());
    }
    let subresource = &resource.subresources[0];
    if subresource.mip_slice != 0
        || subresource.array_slice != 0
        || subresource.width != resource.width
        || subresource.height != resource.height
    {
        return Err(format!(
            "restore-pass {role} subresource is mip {} array {} {}x{}, expected mip 0 array 0 {}x{}",
            subresource.mip_slice,
            subresource.array_slice,
            subresource.width,
            subresource.height,
            resource.width,
            resource.height
        )
        .into());
    }
    Ok(subresource)
}

fn read_restore_texture_file(
    bundle_dir: &Path,
    resource: &MapFrameTextureBinding,
    role: &str,
    expected_bytes: usize,
    expected_shape: &str,
) -> Result<(String, Vec<u8>)> {
    if resource.byte_len != expected_bytes {
        return Err(format!(
            "restore-pass {role} metadata byte_len is {}, expected {expected_bytes} for {expected_shape}",
            resource.byte_len
        )
        .into());
    }
    let file = resource
        .file
        .as_ref()
        .ok_or_else(|| format!("restore-pass {role} has no payload file"))?;
    let path = bundle_dir.join(file);
    let raw = fs::read(&path).map_err(|error| {
        format!(
            "restore-pass {role} payload {} is not readable: {}",
            path.display(),
            error
        )
    })?;
    if raw.len() != expected_bytes {
        return Err(format!(
            "restore-pass {role} payload {} has {} bytes, expected {expected_bytes} for {expected_shape}",
            path.display(),
            raw.len()
        )
        .into());
    }
    verify_optional_hash(
        &path,
        resource.fnv1a64.as_ref(),
        &format!("restore-pass {role} payload"),
    )?;
    Ok((file.clone(), raw))
}

fn verify_optional_hash(path: &Path, expected: Option<&String>, role: &str) -> Result<()> {
    if let Some(expected) = expected {
        let actual = fnv1a64_file(path)?;
        if &actual != expected {
            return Err(format!(
                "{role} {} hash changed from {} to {}",
                path.display(),
                expected,
                actual
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn restore_pass_bundle_loader_decodes_fixture() {
        // Fixture is the LIVE restore_gate_1 bundle (2026-07-06, vanilla
        // 1936): zero blockers, ColorCube B8G8R8X8, luminance R16_FLOAT,
        // the exact evidence class the grading path consumes in production.
        let bundle = load_restore_scene_bundle_session_dir(&fixture_dir())
            .expect("load restore pass fixture")
            .expect("restore pass bundle exists");
        assert_eq!(bundle.bundle_file, RESTORE_PASS_BUNDLE_FILE);
        assert_eq!(bundle.draw_index, 0);
        assert_eq!(bundle.draw_call, "Draw");
        assert_eq!(bundle.color_cube.width, 1024);
        assert_eq!(bundle.color_cube.height, 32);
        assert_eq!(bundle.color_cube.dxgi_format, DXGI_FORMAT_B8G8R8X8_UNORM);
        assert_eq!(bundle.color_cube.rgba8.len(), 131_072);
        // Swizzle applied: every texel opaque after BGRX -> RGBA.
        assert!(bundle
            .color_cube
            .rgba8
            .chunks_exact(4)
            .all(|t| t[3] == 0xFF));
        assert_eq!(bundle.average_luminance.width, 1);
        assert_eq!(bundle.average_luminance.height, 1);
        assert_eq!(bundle.average_luminance.dxgi_format, DXGI_FORMAT_R16_FLOAT);
        assert!((bundle.average_luminance.value - 0.5).abs() < 0.001);
        assert_eq!(
            bundle.posteffect_cb.hsv,
            [-0.0, 1.0, 1.0, 1.0],
            "live HSV is neutral shift with w=1"
        );
        assert_eq!(bundle.posteffect_cb.color_balance, [1.0, 1.0, 1.0]);
        assert!((bundle.posteffect_cb.emissive_bloom_strength - 1.0).abs() < 0.000001);
        assert!((bundle.posteffect_cb.middle_grey - 0.55).abs() < 0.000001);
        assert!((bundle.posteffect_cb.lum_white2 - 1.0).abs() < 0.000001);
    }

    #[test]
    fn restore_pass_bundle_loader_rejects_truncated_color_cube_payload() {
        let root = copy_fixture_to_temp("truncated_color_cube");
        let color_cube = std::fs::read_dir(&root)
            .expect("read fixture root")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.contains("_ps_srv_ColorCube_"))
                    .unwrap_or(false)
            })
            .expect("fixture has a ColorCube payload file");
        let mut bytes = std::fs::read(&color_cube).expect("read ColorCube fixture");
        bytes.truncate(bytes.len() - 1);
        std::fs::write(&color_cube, bytes).expect("truncate ColorCube fixture");

        let err = load_restore_scene_bundle_session_dir(&root)
            .expect_err("truncated ColorCube must fail closed");
        let message = err.to_string();
        assert!(message.contains("ColorCube payload"), "{message}");
        assert!(message.contains("expected 131072"), "{message}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_pass_bundle_loader_rejects_missing_ps_slot1_cb() {
        let root = copy_fixture_to_temp("missing_ps_slot1_cb");
        let bundle_path = root.join(RESTORE_PASS_BUNDLE_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&bundle_path).expect("read bundle"))
                .expect("parse bundle");
        let constant_buffers = json["draws"][0]["constant_buffers"]
            .as_array_mut()
            .expect("constant_buffers array");
        constant_buffers.retain(|binding| {
            !(binding["stage"].as_str() == Some("PS") && binding["slot"].as_u64() == Some(1))
        });
        std::fs::write(
            &bundle_path,
            serde_json::to_vec_pretty(&json).expect("serialize mutated bundle"),
        )
        .expect("write mutated bundle");

        let err = load_restore_scene_bundle_session_dir(&root)
            .expect_err("missing PS slot 1 CB must fail closed");
        let message = err.to_string();
        assert!(message.contains("missing PS slot 1"), "{message}");
        assert!(message.contains("PostEffectBase"), "{message}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn posteffect_uniform_plausibility_rejects_zero_pdxmap_slot1_shape() {
        let zero = RestoreSceneUniforms {
            hsv: [0.0, 0.0, 0.0, 0.0],
            color_balance: [0.0, 0.0, 0.0],
            emissive_bloom_strength: 0.0,
            inv_down_sample_size: [0.0, 0.0],
            inv_window_size: [0.0, 0.0],
            bloom_to_screen_scale: [0.0, 0.0],
            bright_threshold: 0.0,
            middle_grey: 0.0,
            lum_white2: 3584.0,
        };
        assert!(!restore_uniforms_plausible(&zero));
    }

    #[test]
    fn posteffect_uniform_plausibility_accepts_vanilla_identity_grade_shape() {
        let uniforms = RestoreSceneUniforms {
            hsv: [0.0, 1.0, 1.0, 0.0],
            color_balance: [1.0, 1.0, 1.0],
            emissive_bloom_strength: 0.0,
            inv_down_sample_size: [0.0005, 0.0005],
            inv_window_size: [0.001, 0.001],
            bloom_to_screen_scale: [1.0, 1.0],
            bright_threshold: 1.0,
            middle_grey: 0.18,
            lum_white2: 1.0,
        };
        assert!(restore_uniforms_plausible(&uniforms));
    }

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/restore_pass_bundle")
    }

    fn copy_fixture_to_temp(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hoi4_restore_bundle_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp restore fixture");
        for entry in std::fs::read_dir(fixture_dir()).expect("read restore fixture dir") {
            let entry = entry.expect("fixture entry");
            std::fs::copy(entry.path(), root.join(entry.file_name())).expect("copy fixture file");
        }
        root
    }

    #[test]
    fn restore_pass_bundle_with_blockers_fails_closed() {
        let root = copy_fixture_to_temp("blockers");
        let bundle_path = root.join(RESTORE_PASS_BUNDLE_FILE);
        let mut bundle: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&bundle_path).expect("read fixture bundle"),
        )
        .expect("parse fixture bundle");
        bundle["blockers"] =
            serde_json::json!(["map-frame restore_pass is still active; waiting for close"]);
        std::fs::write(
            &bundle_path,
            serde_json::to_string(&bundle).expect("serialize bundle"),
        )
        .expect("write fixture bundle");

        let err = load_restore_scene_bundle_session_dir(&root)
            .expect_err("bundle with blockers must fail closed");
        let message = err.to_string();
        assert!(message.contains("capture blocker"), "{message}");
        assert!(message.contains("still active"), "{message}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bgrx8_texels_swizzle_to_rgba8_with_opaque_alpha() {
        // One blue-ish BGRX texel (B=10 G=20 R=30 X=undefined 7).
        let raw = [10u8, 20, 30, 7, 200, 100, 50, 0];
        let rgba = bgrx8_to_rgba8(&raw);
        assert_eq!(rgba, vec![30, 20, 10, 0xFF, 50, 100, 200, 0xFF]);
    }

    #[test]
    fn f16_decode_matches_known_half_values() {
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0xBC00), -1.0);
        assert_eq!(f16_bits_to_f32(0x4000), 2.0);
        assert_eq!(f16_bits_to_f32(0x3800), 0.5);
        // Smallest positive subnormal half = 2^-24.
        assert_eq!(f16_bits_to_f32(0x0001), 2.0_f32.powi(-24));
        assert!(f16_bits_to_f32(0x7C00).is_infinite());
        assert!(f16_bits_to_f32(0x7C01).is_nan());
    }

    #[test]
    fn restore_texture_metadata_rejects_format_outside_accepted_list() {
        let mut bundle: MapFrameRuntimeBundle = serde_json::from_str(
            &std::fs::read_to_string(fixture_dir().join(RESTORE_PASS_BUNDLE_FILE))
                .expect("read fixture bundle"),
        )
        .expect("parse fixture bundle");
        let resource = bundle.draws[0]
            .shader_resources
            .iter_mut()
            .find(|resource| resource.declared_name.as_deref() == Some(RESTORE_COLOR_CUBE))
            .expect("fixture ColorCube");
        // B8G8R8A8 (87) is close to the live 88 but NOT accepted — alpha
        // semantics differ and it has never been observed; must fail closed.
        resource.dxgi_format = DXGI_FORMAT_B8G8R8A8_UNORM;
        let err = validate_restore_texture_metadata(
            resource,
            RESTORE_COLOR_CUBE,
            &[DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM],
            RESTORE_COLOR_CUBE_WIDTH,
            RESTORE_COLOR_CUBE_HEIGHT,
        )
        .expect_err("format 87 must be rejected");
        let message = err.to_string();
        assert!(message.contains("87"), "{message}");
        assert!(message.contains("[28, 88]"), "{message}");
    }
}
