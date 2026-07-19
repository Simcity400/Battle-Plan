use crate::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Single-pass extraction of one sampler block: strip comments and walk the
/// PixelShader -> Samplers -> sampler nesting exactly once. Every sampler
/// parse (contract, Index, Type) reads from the block this returns instead of
/// re-parsing the full shader source.
fn locate_sampler_block(
    shader_source: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<String> {
    let text = strip_comments(shader_source);
    let (pixel_shader, _) = extract_block_after(&text, "PixelShader", 0).ok_or_else(|| {
        format!("production terrain renderer blocked: {shader_label} missing PixelShader block")
    })?;
    let (samplers, _) = extract_block_after(&pixel_shader, "Samplers", 0).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: {shader_label} missing PixelShader Samplers block"
        )
    })?;
    let (sampler, _) = extract_block_after(&samplers, sampler_name, 0).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: {shader_label} missing sampler {sampler_name}"
        )
    })?;
    Ok(sampler)
}

fn sampler_contract_from_block(
    sampler_block: &str,
    sampler_name: &str,
) -> Result<TerrainRuntimeSampler> {
    Ok(TerrainRuntimeSampler {
        mag_filter: required_sampler_value(sampler_block, sampler_name, "MagFilter")?,
        min_filter: required_sampler_value(sampler_block, sampler_name, "MinFilter")?,
        mip_filter: required_sampler_value(sampler_block, sampler_name, "MipFilter")?,
        address_u: required_sampler_value(sampler_block, sampler_name, "AddressU")?,
        address_v: required_sampler_value(sampler_block, sampler_name, "AddressV")?,
    })
}

fn sampler_index_from_block(
    sampler_block: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<u32> {
    parse_u32_value(sampler_block, "Index").ok_or_else(|| {
        format!("production terrain renderer blocked: {shader_label} sampler {sampler_name} missing Index")
            .into()
    })
}

fn sampler_resource_type_from_block(
    sampler_block: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<ShaderSamplerResourceType> {
    match parse_quoted_value(sampler_block, "Type").as_deref() {
        None => Ok(ShaderSamplerResourceType::Texture2D),
        Some("Cube") => Ok(ShaderSamplerResourceType::Cube),
        Some("Shadow") => Ok(ShaderSamplerResourceType::Shadow),
        Some(other) => Err(format!(
            "production terrain renderer blocked: {shader_label} sampler {sampler_name} declares unsupported Type {other:?}"
        )
        .into()),
    }
}

pub(crate) fn parse_shader_sampler_contract(
    shader_source: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<TerrainRuntimeSampler> {
    let sampler = locate_sampler_block(shader_source, shader_label, sampler_name)?;
    sampler_contract_from_block(&sampler, sampler_name)
}

pub(crate) fn parse_shader_sampler_index(
    shader_source: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<u32> {
    let sampler = locate_sampler_block(shader_source, shader_label, sampler_name)?;
    sampler_index_from_block(&sampler, shader_label, sampler_name)
}

pub(crate) fn parse_shader_sampler_resource_type(
    shader_source: &str,
    shader_label: &str,
    sampler_name: &str,
) -> Result<ShaderSamplerResourceType> {
    let sampler = locate_sampler_block(shader_source, shader_label, sampler_name)?;
    sampler_resource_type_from_block(&sampler, shader_label, sampler_name)
}

pub(crate) fn parse_pdxmap_sampler_contract(
    pdxmap_shader_source: &str,
    sampler_name: &str,
) -> Result<TerrainRuntimeSampler> {
    parse_shader_sampler_contract(pdxmap_shader_source, "pdxmap.shader", sampler_name)
}

pub(crate) fn shader_sampler_mode_from_contract(
    shader_label: &str,
    sampler_name: &str,
    sampler: &TerrainRuntimeSampler,
) -> Result<ShaderSamplerMode> {
    let mag = sampler.mag_filter.as_str();
    let min = sampler.min_filter.as_str();
    let mip = sampler.mip_filter.as_str();
    let address_u = sampler.address_u.as_str();
    let address_v = sampler.address_v.as_str();
    match (mag, min, mip, address_u, address_v) {
        ("Linear", "Linear", "Linear", "Wrap", "Wrap") => Ok(ShaderSamplerMode::LinearWrap),
        ("Linear", "Linear", "Linear", "Clamp", "Clamp") => Ok(ShaderSamplerMode::LinearClamp),
        ("Point", "Point", "Point", "Clamp", "Clamp") => Ok(ShaderSamplerMode::PointClamp),
        _ => Err(format!(
            "production terrain renderer blocked: {shader_label} sampler {sampler_name} declares unsupported sampler mode MagFilter={} MinFilter={} MipFilter={} AddressU={} AddressV={}",
            sampler.mag_filter, sampler.min_filter, sampler.mip_filter, sampler.address_u, sampler.address_v
        )
        .into()),
    }
}

pub(crate) fn validate_shader_sampler_expectation_against_source(
    shader_source: &str,
    shader_label: &str,
    expectation: ShaderSamplerExpectation,
) -> Result<()> {
    // One locate per sampler: the contract, Index, and Type reads below all
    // parse the same block, in the same order (contract fields, then the mode
    // check derived from them, then Index, then Type) as the previous
    // triple-parse implementation, so blocker messages are unchanged.
    let sampler_block = locate_sampler_block(shader_source, shader_label, expectation.name)?;
    let sampler = sampler_contract_from_block(&sampler_block, expectation.name)?;
    let mode = shader_sampler_mode_from_contract(shader_label, expectation.name, &sampler)?;
    let index = sampler_index_from_block(&sampler_block, shader_label, expectation.name)?;
    if index != expectation.index {
        return Err(format!(
            "production terrain renderer blocked: {shader_label} sampler {} declares Index {}, but renderer contract expects {}",
            expectation.name, index, expectation.index
        )
        .into());
    }
    let resource_type =
        sampler_resource_type_from_block(&sampler_block, shader_label, expectation.name)?;
    if resource_type != expectation.resource_type {
        return Err(format!(
            "production terrain renderer blocked: {shader_label} sampler {} declares Type {}, but renderer contract expects {}",
            expectation.name,
            resource_type.label(),
            expectation.resource_type.label()
        )
        .into());
    }
    if mode != expectation.mode {
        return Err(format!(
            "production terrain renderer blocked: {shader_label} sampler {} is {}, but renderer contract expects {}",
            expectation.name,
            mode.label(),
            expectation.mode.label()
        )
        .into());
    }
    Ok(())
}

/// The golden (field name, byte offset) contract for TerrainSceneCB0 — the
/// single fail-closed source of truth shared by `terrain_scene_cb0_layout`
/// (which asserts every offset against the parsed shader source) and
/// `terrain_scene_cb0_required_offsets` (which re-reads them from a validated
/// layout).
const TERRAIN_SCENE_CB0_FIELDS: &[(&str, usize)] = &[
    ("vGBCamDistOverride_GBOutlineCutoff", 128),
    ("vVirtualSunPos", 144),
    ("vVirtualMoonPos", 160),
    ("vSecondVirtualSunPos", 176),
    ("vSecondVirtualMoonPos", 192),
    ("vCamPos", 208),
    ("HdrRange", 220),
    ("vGlobalTime", 236),
    ("vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed", 240),
    ("DayNight_Hour_SunDir", 256),
    ("AmbientPosX", 272),
    ("ShadowFadeFactor", 284),
    ("AmbientNegX", 288),
    ("FOWFadeFactor", 300),
    ("AmbientPosY", 304),
    ("AmbientNegY", 320),
    ("NegFogMultiplier", 332),
    ("AmbientPosZ", 336),
    ("AmbientNegZ", 352),
    ("SunDiffuseIntensity", 368),
    ("MoonDiffuseIntensity", 384),
    ("GB_TextureHeight", 400),
    ("SunSpecularIntensity", 404),
];

pub(crate) fn terrain_scene_cb0_layout(
    standard_functions_source: &str,
) -> Result<ShaderConstantBufferLayout> {
    let layout = parse_constant_buffer_layout(standard_functions_source, 0, 0)?;
    for (field, expected_offset) in TERRAIN_SCENE_CB0_FIELDS.iter().copied() {
        let actual = constant_buffer_field(&layout, field)?;
        if actual.offset != expected_offset {
            return Err(format!(
                "production terrain renderer blocked: {TERRAIN_SCENE_CB0_LAYOUT_SOURCE} field {field} moved from byte {expected_offset} to {}",
                actual.offset
            )
            .into());
        }
    }
    if layout.byte_len != 416 {
        return Err(format!(
            "production terrain renderer blocked: {TERRAIN_SCENE_CB0_LAYOUT_SOURCE} byte size changed from 416 to {}",
            layout.byte_len
        )
        .into());
    }
    Ok(layout)
}

pub(crate) fn terrain_scene_cb0_required_offsets(
    layout: &ShaderConstantBufferLayout,
) -> Result<BTreeMap<String, usize>> {
    TERRAIN_SCENE_CB0_FIELDS
        .iter()
        .map(|(name, _)| {
            Ok((
                name.to_string(),
                constant_buffer_field(layout, name)?.offset,
            ))
        })
        .collect()
}

pub(crate) fn parse_constant_buffer_layout(
    shader_source: &str,
    slot: u32,
    bind_point: u32,
) -> Result<ShaderConstantBufferLayout> {
    let block = extract_constant_buffer_block(shader_source, slot, bind_point).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: missing ConstantBuffer({slot},{bind_point}) declaration"
        )
    })?;
    let mut fields = Vec::new();
    let mut offset = 0usize;
    for raw in block.lines() {
        let line = raw
            .split_once("//")
            .map(|(head, _)| head)
            .unwrap_or(raw)
            .trim();
        if line.is_empty() || !line.ends_with(';') {
            continue;
        }
        let declaration = line.trim_end_matches(';').trim();
        let mut parts = declaration.split_whitespace();
        let Some(type_name) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if type_name == "float4x4" {
            offset = align_16(offset);
            fields.push(ShaderConstantBufferField {
                type_name: type_name.to_string(),
                name: name.to_string(),
                offset,
                byte_len: 64,
            });
            offset += 64;
            continue;
        }
        let Some(component_count) = hlsl_float_component_count(type_name) else {
            return Err(format!(
                "production terrain renderer blocked: unsupported ConstantBuffer field type {type_name} for {name}"
            )
            .into());
        };
        let register_component_offset = (offset % 16) / 4;
        if register_component_offset + component_count > 4 {
            offset = align_16(offset);
        }
        let byte_len = component_count * 4;
        fields.push(ShaderConstantBufferField {
            type_name: type_name.to_string(),
            name: name.to_string(),
            offset,
            byte_len,
        });
        offset += byte_len;
    }
    Ok(ShaderConstantBufferLayout {
        byte_len: align_16(offset),
        fields,
    })
}

pub(crate) fn extract_constant_buffer_block(
    shader_source: &str,
    slot: u32,
    bind_point: u32,
) -> Option<String> {
    let text = strip_comments(shader_source);
    let mut start = 0usize;
    while let Some(relative) = text[start..].find("ConstantBuffer") {
        let idx = start + relative;
        let after_keyword = idx + "ConstantBuffer".len();
        let before_ok = idx == 0 || !is_word_byte(text.as_bytes()[idx - 1]);
        let after_ok = after_keyword >= text.len() || !is_word_byte(text.as_bytes()[after_keyword]);
        if !before_ok || !after_ok {
            start = after_keyword;
            continue;
        }
        let after = &text[after_keyword..];
        let open_relative = after.find('(')?;
        if !after[..open_relative].trim().is_empty() {
            start = after_keyword;
            continue;
        }
        let open = after_keyword + open_relative;
        let close = text[open + 1..].find(')')? + open + 1;
        let args = parse_u32s(&text[open + 1..close]);
        if args.get(0) == Some(&slot) && args.get(1) == Some(&bind_point) {
            let brace = text[close + 1..].find('{')? + close + 1;
            return extract_braced_block_at(&text, brace).map(|(block, _)| block);
        }
        start = close + 1;
    }
    None
}

pub(crate) fn extract_braced_block_at(text: &str, brace: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(brace) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let inner_start = brace + 1;
    let mut i = brace;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[inner_start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(crate) fn hlsl_float_component_count(type_name: &str) -> Option<usize> {
    match type_name {
        "float" => Some(1),
        "float2" => Some(2),
        "float3" => Some(3),
        "float4" => Some(4),
        _ => None,
    }
}

pub(crate) fn align_16(value: usize) -> usize {
    (value + 15) & !15
}

pub(crate) fn constant_buffer_field<'a>(
    layout: &'a ShaderConstantBufferLayout,
    name: &str,
) -> Result<&'a ShaderConstantBufferField> {
    layout.fields.iter().find(|field| field.name == name).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: {TERRAIN_SCENE_CB0_LAYOUT_SOURCE} missing field {name}"
        )
        .into()
    })
}

pub(crate) fn validate_constant_buffer_byte_len(
    byte_len: usize,
    layout_byte_len: usize,
) -> Result<()> {
    if byte_len == 0 {
        return Err(
            "production terrain renderer blocked: runtime constant buffer byte_len must be nonzero"
                .into(),
        );
    }
    if byte_len % 16 != 0 {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer byte_len {byte_len} is not 16-byte aligned"
        )
        .into());
    }
    if byte_len < layout_byte_len {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer byte_len {byte_len} is smaller than shader layout byte_len {layout_byte_len}"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn decode_terrain_scene_cb0(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
) -> Result<TerrainSceneCb0Decoded> {
    validate_constant_buffer_byte_len(bytes.len(), layout.byte_len)?;
    let view_projection_matrix = read_float4x4_field(bytes, layout, "ViewProjectionMatrix")?;
    let v_gb_cam_dist_override_gb_outline_cutoff =
        read_float4_field(bytes, layout, "vGBCamDistOverride_GBOutlineCutoff")?;
    let v_virtual_sun_pos = read_vec3_field(bytes, layout, "vVirtualSunPos")?;
    let v_virtual_moon_pos = read_vec3_field(bytes, layout, "vVirtualMoonPos")?;
    let v_second_virtual_sun_pos = read_vec3_field(bytes, layout, "vSecondVirtualSunPos")?;
    let v_second_virtual_moon_pos = read_vec3_field(bytes, layout, "vSecondVirtualMoonPos")?;
    let v_cam_pos = read_vec3_field(bytes, layout, "vCamPos")?;
    let hdr_range = read_f32_field(bytes, layout, "HdrRange")?;
    let v_global_time = read_f32_field(bytes, layout, "vGlobalTime")?;
    let v_fow_opacity_fow_time_snow_mud_fade_max_game_speed = read_float4_field(
        bytes,
        layout,
        "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed",
    )?;
    let day_night_hour_sun_dir = read_float4_field(bytes, layout, "DayNight_Hour_SunDir")?;
    let decoded = TerrainSceneCb0Decoded {
        view_projection_matrix,
        v_gb_cam_dist_override_gb_outline_cutoff,
        v_virtual_sun_pos,
        v_virtual_moon_pos,
        v_second_virtual_sun_pos,
        v_second_virtual_moon_pos,
        v_cam_pos,
        hdr_range,
        v_global_time,
        v_fow_opacity_fow_time_snow_mud_fade_max_game_speed,
        day_night_hour_sun_dir,
        ambient_pos_x: read_vec3_field(bytes, layout, "AmbientPosX")?,
        shadow_fade_factor: read_f32_field(bytes, layout, "ShadowFadeFactor")?,
        ambient_neg_x: read_vec3_field(bytes, layout, "AmbientNegX")?,
        fow_fade_factor: read_f32_field(bytes, layout, "FOWFadeFactor")?,
        ambient_pos_y: read_vec3_field(bytes, layout, "AmbientPosY")?,
        ambient_neg_y: read_vec3_field(bytes, layout, "AmbientNegY")?,
        neg_fog_multiplier: read_f32_field(bytes, layout, "NegFogMultiplier")?,
        ambient_pos_z: read_vec3_field(bytes, layout, "AmbientPosZ")?,
        ambient_neg_z: read_vec3_field(bytes, layout, "AmbientNegZ")?,
        sun_diffuse_intensity: read_float4_field(bytes, layout, "SunDiffuseIntensity")?,
        moon_diffuse_intensity: read_float4_field(bytes, layout, "MoonDiffuseIntensity")?,
        gb_texture_height: read_f32_field(bytes, layout, "GB_TextureHeight")?,
        sun_specular_intensity: read_f32_field(bytes, layout, "SunSpecularIntensity")?,
    };
    validate_finite_terrain_scene_cb0(decoded)?;
    Ok(decoded)
}

pub(crate) fn validate_finite_terrain_scene_cb0(decoded: TerrainSceneCb0Decoded) -> Result<()> {
    if decoded
        .view_projection_matrix
        .iter()
        .flatten()
        .copied()
        .chain([
            decoded.v_virtual_sun_pos.x,
            decoded.v_virtual_sun_pos.y,
            decoded.v_virtual_sun_pos.z,
            decoded.v_virtual_moon_pos.x,
            decoded.v_virtual_moon_pos.y,
            decoded.v_virtual_moon_pos.z,
            decoded.v_second_virtual_sun_pos.x,
            decoded.v_second_virtual_sun_pos.y,
            decoded.v_second_virtual_sun_pos.z,
            decoded.v_second_virtual_moon_pos.x,
            decoded.v_second_virtual_moon_pos.y,
            decoded.v_second_virtual_moon_pos.z,
            decoded.v_cam_pos.x,
            decoded.v_cam_pos.y,
            decoded.v_cam_pos.z,
            decoded.hdr_range,
            decoded.v_global_time,
            decoded.v_gb_cam_dist_override_gb_outline_cutoff[0],
            decoded.v_gb_cam_dist_override_gb_outline_cutoff[1],
            decoded.v_gb_cam_dist_override_gb_outline_cutoff[2],
            decoded.v_gb_cam_dist_override_gb_outline_cutoff[3],
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[0],
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[1],
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[2],
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3],
            decoded.day_night_hour_sun_dir[0],
            decoded.day_night_hour_sun_dir[1],
            decoded.day_night_hour_sun_dir[2],
            decoded.day_night_hour_sun_dir[3],
            decoded.ambient_pos_x.x,
            decoded.ambient_pos_x.y,
            decoded.ambient_pos_x.z,
            decoded.shadow_fade_factor,
            decoded.ambient_neg_x.x,
            decoded.ambient_neg_x.y,
            decoded.ambient_neg_x.z,
            decoded.fow_fade_factor,
            decoded.ambient_pos_y.x,
            decoded.ambient_pos_y.y,
            decoded.ambient_pos_y.z,
            decoded.ambient_neg_y.x,
            decoded.ambient_neg_y.y,
            decoded.ambient_neg_y.z,
            decoded.neg_fog_multiplier,
            decoded.ambient_pos_z.x,
            decoded.ambient_pos_z.y,
            decoded.ambient_pos_z.z,
            decoded.ambient_neg_z.x,
            decoded.ambient_neg_z.y,
            decoded.ambient_neg_z.z,
            decoded.sun_diffuse_intensity[0],
            decoded.sun_diffuse_intensity[1],
            decoded.sun_diffuse_intensity[2],
            decoded.sun_diffuse_intensity[3],
            decoded.moon_diffuse_intensity[0],
            decoded.moon_diffuse_intensity[1],
            decoded.moon_diffuse_intensity[2],
            decoded.moon_diffuse_intensity[3],
            decoded.gb_texture_height,
            decoded.sun_specular_intensity,
        ])
        .any(|value| !value.is_finite())
    {
        return Err(
            "production terrain renderer blocked: TerrainSceneCB0 decoded non-finite values".into(),
        );
    }
    Ok(())
}

pub(crate) fn read_vec3_field(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
) -> Result<Vec3> {
    let field = constant_buffer_field(layout, name)?;
    if !matches!(field.type_name.as_str(), "float3" | "float4") {
        return Err(format!(
            "production terrain renderer blocked: {name} expected float3/float4 xyz field, got {}",
            field.type_name
        )
        .into());
    }
    Ok(Vec3 {
        x: read_f32_at(bytes, field.offset)?,
        y: read_f32_at(bytes, field.offset + 4)?,
        z: read_f32_at(bytes, field.offset + 8)?,
    })
}

pub(crate) fn read_float4_field(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
) -> Result<[f32; 4]> {
    let field = constant_buffer_field(layout, name)?;
    if field.type_name != "float4" {
        return Err(format!(
            "production terrain renderer blocked: {name} expected float4, got {}",
            field.type_name
        )
        .into());
    }
    Ok([
        read_f32_at(bytes, field.offset)?,
        read_f32_at(bytes, field.offset + 4)?,
        read_f32_at(bytes, field.offset + 8)?,
        read_f32_at(bytes, field.offset + 12)?,
    ])
}

pub(crate) fn read_f32_field(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
) -> Result<f32> {
    let field = constant_buffer_field(layout, name)?;
    if field.type_name != "float" {
        return Err(format!(
            "production terrain renderer blocked: {name} expected float, got {}",
            field.type_name
        )
        .into());
    }
    read_f32_at(bytes, field.offset)
}

pub(crate) fn read_f32_at(bytes: &[u8], offset: usize) -> Result<f32> {
    let end = offset
        .checked_add(4)
        .ok_or("constant buffer offset overflow")?;
    let raw = bytes.get(offset..end).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: constant buffer missing bytes {offset}..{end}"
        )
    })?;
    Ok(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

pub(crate) fn is_terrain_scene_cb0_camera_binding(binding: &MapFrameBufferBinding) -> bool {
    binding.declared_name.as_deref() == Some(TERRAIN_SCENE_CB0_SYMBOL)
        && binding.slot == TERRAIN_SCENE_CB0_BIND_POINT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerrainSceneCb0CameraUpdateMethod {
    DefaultUpdateSubresource,
    DynamicMapWriteDiscard,
}

fn reflected_constant_buffer_name_for_slot<'a>(
    shader: &'a MapFrameShader,
    slot: u32,
) -> Option<&'a str> {
    shader
        .constant_buffers
        .iter()
        .find(|(_, reflected_slot)| **reflected_slot == slot)
        .map(|(name, _)| name.as_str())
}

fn is_generated_slot_zero_cb0_alias(name: &str) -> bool {
    name == "dx11_cb0"
}

pub(crate) fn terrain_scene_cb0_camera_layout_for_binding(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
    binding: &MapFrameBufferBinding,
    byte_len: usize,
) -> Result<Option<ShaderConstantBufferLayout>> {
    if binding.slot != TERRAIN_SCENE_CB0_BIND_POINT {
        return Ok(None);
    }
    let shader = match binding.stage.as_str() {
        "VS" => draw.vertex_shader.as_ref().ok_or(
            "production map renderer blocked: app-owned terrain camera needs captured VS identity",
        )?,
        "PS" => &draw.shader,
        stage => {
            return Err(format!(
                "production map renderer blocked: app-owned terrain camera cannot patch {TERRAIN_SCENE_CB0_SYMBOL} for unsupported constant-buffer stage {stage}"
            )
            .into())
        }
    };
    let reflected_name = reflected_constant_buffer_name_for_slot(shader, binding.slot);
    let declared_name = binding.declared_name.as_deref();
    let direct_name_match = is_terrain_scene_cb0_camera_binding(binding)
        || reflected_name == Some(TERRAIN_SCENE_CB0_SYMBOL);
    let generated_slot_zero_alias =
        declared_name
            .zip(reflected_name)
            .is_some_and(|(declared, reflected)| {
                declared == reflected && is_generated_slot_zero_cb0_alias(reflected)
            });
    let has_standardfuncs_layout_evidence = shader
        .source_evidence
        .iter()
        .any(|source| source == "gfx/FX/standardfuncsgfx.fxh");
    if !direct_name_match && !(generated_slot_zero_alias && has_standardfuncs_layout_evidence) {
        return Ok(None);
    }
    if !shader
        .source_evidence
        .iter()
        .any(|source| source == "gfx/FX/standardfuncsgfx.fxh")
    {
        return Err(
            "production map renderer blocked: app-owned terrain camera requires TerrainSceneCB0 layout evidence from gfx/FX/standardfuncsgfx.fxh"
                .into(),
        );
    }
    let source_path = bundle_dir
        .join("vanilla")
        .join("gfx/FX/standardfuncsgfx.fxh");
    let layout = terrain_scene_cb0_layout(&read_latin1(&source_path)?)?;
    validate_constant_buffer_byte_len(byte_len, layout.byte_len)?;
    Ok(Some(layout))
}

pub(crate) fn camera_constant_buffer_update_method(
    binding: &MapFrameBufferBinding,
    byte_len: usize,
    draw_index: u32,
) -> Result<TerrainSceneCb0CameraUpdateMethod> {
    if binding.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
        return Err(format!(
            "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} missing D3D11_BIND_CONSTANT_BUFFER",
            binding.stage, binding.slot
        )
        .into());
    }
    if byte_len == 0 || byte_len % 16 != 0 {
        return Err(format!(
            "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} byte length {byte_len} is not a non-zero 16-byte aligned constant buffer",
            binding.stage, binding.slot
        )
        .into());
    }
    match binding.usage {
        D3D11_USAGE_DEFAULT => {
            if binding.cpu_access_flags != 0 {
                return Err(format!(
                    "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} uses captured D3D11_USAGE 0 (DEFAULT) but has CPU access flags {}, cannot update app camera with UpdateSubresource",
                    binding.stage, binding.slot, binding.cpu_access_flags
                )
                .into());
            }
            Ok(TerrainSceneCb0CameraUpdateMethod::DefaultUpdateSubresource)
        }
        D3D11_USAGE_DYNAMIC_CAPTURED => {
            if binding.cpu_access_flags != D3D11_CPU_ACCESS_WRITE_CAPTURED {
                return Err(format!(
                    "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} uses captured D3D11_USAGE 2 (DYNAMIC) but has CPU access flags {}, expected D3D11_CPU_ACCESS_WRITE for Map(WRITE_DISCARD)",
                    binding.stage, binding.slot, binding.cpu_access_flags
                )
                .into());
            }
            Ok(TerrainSceneCb0CameraUpdateMethod::DynamicMapWriteDiscard)
        }
        D3D11_USAGE_IMMUTABLE_CAPTURED => Err(format!(
            "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} uses captured D3D11_USAGE 1 (IMMUTABLE); app camera update supports only DEFAULT (0) via UpdateSubresource or DYNAMIC (2) via Map(WRITE_DISCARD)",
            binding.stage, binding.slot
        )
        .into()),
        D3D11_USAGE_STAGING_CAPTURED => Err(format!(
            "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} uses captured D3D11_USAGE 3 (STAGING); app camera update supports only DEFAULT (0) via UpdateSubresource or DYNAMIC (2) via Map(WRITE_DISCARD)",
            binding.stage, binding.slot
        )
        .into()),
        usage => Err(format!(
            "production native D3D11 map viewer blocked: draw {draw_index} {TERRAIN_SCENE_CB0_SYMBOL} {} constant buffer slot {} uses captured D3D11_USAGE {usage}; app camera update supports only DEFAULT (0) via UpdateSubresource or DYNAMIC (2) via Map(WRITE_DISCARD)",
            binding.stage, binding.slot
        )
        .into()),
    }
}

pub(crate) fn validate_camera_constant_buffer_update_binding(
    binding: &MapFrameBufferBinding,
    byte_len: usize,
    draw_index: u32,
) -> Result<()> {
    camera_constant_buffer_update_method(binding, byte_len, draw_index).map(|_| ())
}

#[cfg(test)]
pub(crate) fn terrain_scene_cb0_bytes_with_app_camera(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
    binding: &MapFrameBufferBinding,
    bytes: Vec<u8>,
    camera: ProductionMapCamera,
) -> Result<Vec<u8>> {
    let Some(layout) =
        terrain_scene_cb0_camera_layout_for_binding(bundle_dir, draw, binding, bytes.len())?
    else {
        return Ok(bytes);
    };
    terrain_scene_cb0_bytes_with_app_camera_layout(&layout, bytes, camera)
}

pub(crate) fn terrain_scene_cb0_bytes_with_app_camera_layout(
    layout: &ShaderConstantBufferLayout,
    bytes: Vec<u8>,
    camera: ProductionMapCamera,
) -> Result<Vec<u8>> {
    terrain_scene_cb0_bytes_with_app_scene_layout(layout, bytes, camera, None, None, None)
}

/// COL3: the POLITICAL-mapmode scene CB0 state (session_POLCOLOR_1 +
/// BORDERZOOM sweeps, 3768 CB0 samples at heights 50..3000 — see
/// `documents/FIDELITY_LEDGER.md`). Two field groups:
///
/// * `vGBCamDistOverride_GBOutlineCutoff = (1.0, 0.0, cutoff_strategic,
///   cutoff_country)` — HEIGHT-INVARIANT in every political-mapmode sample.
///   `.x = 1` forces the strategic-region (sea) gradient layer to the "near"
///   camera response, `.y = 0` leaves the country layer on the natural
///   `cam_distance` ramp, and `.zw` are the shipped defines
///   `GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS` / `_COUNTRY`
///   (`common/defines/00_graphics.lua:880-885`, vanilla 0.98 / 0.973).
/// * `ShadowFadeFactor = clamp((DRAW_SHADOWS_CUTOFF - camera_height) /
///   DRAW_SHADOWS_FADE_LENGTH, 0, 1)` (`00_graphics.lua:860-861`, vanilla
///   400 / 50) — max error 0.000000 against all 1433 BORDERZOOM_1 sweep
///   samples.
///
/// The values are resolver-read by the caller (mod-aware); this struct only
/// carries them into the CB0 write. `None` at the compose call keeps the
/// captured bytes verbatim (all byte-exact reference/validation paths).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoliticalSceneCb0 {
    pub outline_cutoff_strategic: f32,
    pub outline_cutoff_country: f32,
    pub draw_shadows_cutoff: f32,
    pub draw_shadows_fade_length: f32,
    /// SEASON1 (validation-only for now): when `Some`, overwrite the scene
    /// `SnowMudFade` (`vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z`) —
    /// the snow_fade0->snow_fade1 crossfade the engine drives with its
    /// weather clock. `None` keeps the captured byte (the July capture's 0).
    pub snow_mud_fade: Option<f32>,
    /// Whether a REAL shadow map is bound for this render (Codex-review
    /// provenance gate). The product capture (live_PAN2, taken at h=800
    /// where shadows are faded out) recorded a NULL ShadowMap SRV; sampling
    /// null returns 0, so writing the engine's nonzero fade factor would turn
    /// `shadow.fxh:50` (`vShadowValue += 1.0 - ShadowFadeFactor`) into a
    /// UNIFORM darkening of every pixel instead of the engine's mostly-lit
    /// response. Until a shadow pass is replayed/synthesized, `false`
    /// collapses the written factor to 0 ("shadows off"), which is exact for
    /// every unshadowed pixel and only misses the sparse true shadow casters
    /// below `DRAW_SHADOWS_CUTOFF`.
    pub shadow_map_bound: bool,
}

impl PoliticalSceneCb0 {
    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("outline_cutoff_strategic", self.outline_cutoff_strategic),
            ("outline_cutoff_country", self.outline_cutoff_country),
        ] {
            if !value.is_finite() || value <= 0.0 || value > 1.0 {
                return Err(format!(
                    "production map renderer blocked: political GB {name} must be finite in (0,1], got {value}"
                )
                .into());
            }
        }
        if !self.draw_shadows_cutoff.is_finite() || self.draw_shadows_cutoff <= 0.0 {
            return Err(format!(
                "production map renderer blocked: DRAW_SHADOWS_CUTOFF must be finite positive, got {}",
                self.draw_shadows_cutoff
            )
            .into());
        }
        if !self.draw_shadows_fade_length.is_finite() || self.draw_shadows_fade_length <= 0.0 {
            return Err(format!(
                "production map renderer blocked: DRAW_SHADOWS_FADE_LENGTH must be finite positive, got {}",
                self.draw_shadows_fade_length
            )
            .into());
        }
        Ok(())
    }

    /// The engine's shadow fade at `camera_height` (see struct docs) — gated
    /// to 0 while no real shadow map is bound (`shadow_map_bound` docs).
    pub fn shadow_fade_factor(&self, camera_height: f32) -> f32 {
        if !self.shadow_map_bound {
            return 0.0;
        }
        ((self.draw_shadows_cutoff - camera_height) / self.draw_shadows_fade_length).clamp(0.0, 1.0)
    }
}

/// Write the political-mapmode `vGBCamDistOverride_GBOutlineCutoff` float4
/// and the height-law `ShadowFadeFactor` (see [`PoliticalSceneCb0`]).
pub(crate) fn write_political_scene_fields(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    political: PoliticalSceneCb0,
    camera_height: f32,
) -> Result<()> {
    political.validate()?;
    if !camera_height.is_finite() {
        return Err(
            "production map renderer blocked: political scene camera height is non-finite".into(),
        );
    }
    let field = constant_buffer_field(layout, "vGBCamDistOverride_GBOutlineCutoff")?;
    if field.type_name != "float4" {
        return Err(format!(
            "production map renderer blocked: vGBCamDistOverride_GBOutlineCutoff expected float4, got {}",
            field.type_name
        )
        .into());
    }
    write_f32_at(bytes, field.offset, 1.0)?;
    write_f32_at(bytes, field.offset + 4, 0.0)?;
    write_f32_at(bytes, field.offset + 8, political.outline_cutoff_strategic)?;
    write_f32_at(bytes, field.offset + 12, political.outline_cutoff_country)?;
    let shadow = constant_buffer_field(layout, "ShadowFadeFactor")?;
    write_f32_at(
        bytes,
        shadow.offset,
        political.shadow_fade_factor(camera_height),
    )?;
    if let Some(fade) = political.snow_mud_fade {
        if !(0.0..=1.0).contains(&fade) {
            return Err(format!(
                "production map renderer blocked: SnowMudFade override must be in [0,1], got {fade}"
            )
            .into());
        }
        let field = constant_buffer_field(layout, "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed")?;
        write_f32_at(bytes, field.offset + 8, fade)?;
    }
    Ok(())
}

/// `day_night_hour = None` leaves the captured virtual light rig untouched
/// (all reference / byte-exact captured-frame paths keep the captured CB0
/// bytes verbatim). `Some(hour_fraction)` drives the four rig `.x` longitudes
/// from the PROVEN day-cycle law (`light_law::virtual_rig_longitudes_x`)
/// evaluated at `hour24 = hour_fraction * 24` — the SAME hour that feeds
/// `DayNight_Hour_SunDir.x`, so the rig and the terminator share one clock.
/// The rig no longer follows the pan camera (that FIT is removed; see
/// `write_virtual_light_rig_fields` and `LIGHT_LAW.md`).
pub(crate) fn terrain_scene_cb0_bytes_with_app_scene_layout(
    layout: &ShaderConstantBufferLayout,
    mut bytes: Vec<u8>,
    camera: ProductionMapCamera,
    day_night_hour: Option<f32>,
    map_clock_time: Option<f32>,
    political: Option<PoliticalSceneCb0>,
) -> Result<Vec<u8>> {
    camera.validate()?;
    let vp_field = constant_buffer_field(layout, "ViewProjectionMatrix")?;
    let vp_captured = bytes
        .get(vp_field.offset..vp_field.offset + 64)
        .map(<[u8]>::to_vec)
        .ok_or("production map renderer blocked: scene CB0 too short for ViewProjectionMatrix")?;
    write_float4x4_field(
        &mut bytes,
        layout,
        "ViewProjectionMatrix",
        camera.view_projection_matrix,
    )?;
    // InvViewProjMatrix: the captured bytes hold the game's own inversion,
    // which differs from ours in the low mantissa bits. The captured terrain
    // pixel shader consumes it (world-position reconstruction feeding the
    // land-only snow/mud sampling), so replaying an unchanged camera must
    // keep the captured inverse bit-exact; only an actually moved camera
    // needs a recomputed inverse.
    if bytes[vp_field.offset..vp_field.offset + 64] != vp_captured[..] {
        write_float4x4_field(
            &mut bytes,
            layout,
            "InvViewProjMatrix",
            invert_float4x4(camera.view_projection_matrix)?,
        )?;
    }
    write_vec3_field(&mut bytes, layout, "vCamPos", camera.position)?;
    if let Some(hour) = day_night_hour {
        // One clock: the rig longitudes and the day/night terminator both
        // read this same `hour` fraction. The rig `.x` law consumes hour24.
        write_virtual_light_rig_fields(&mut bytes, layout, hour * 24.0)?;
        write_day_night_hour_field(&mut bytes, layout, hour)?;
        // NIGHT1: the sun-direction `.yzw` is HOUR-INVARIANT (mined from the
        // hour-swept sweep; see `light_law::render_sun_dir_yzw`). We do NOT
        // rotate it with the hour — we echo the captured `.yzw` (the correct
        // sun direction for the recording's date). This is a no-op on the bytes
        // (echo of the captured vector) so it CANNOT disturb `--hour 0.5`
        // byte-identity; it exists to make the "leave `.yzw` alone" law explicit
        // and to fail loudly if the CB0 layout ever changes shape.
        write_sun_dir_yzw_hour_invariant_field(&mut bytes, layout, hour * 24.0)?;
        write_day_night_factor_gate_field(&mut bytes, layout, 1.0)?;
    }
    if let Some(seconds) = map_clock_time {
        write_global_time_field(&mut bytes, layout, seconds)?;
    }
    // COL3: political-mapmode GB override/cutoffs + the shadow fade height
    // law. `None` (every byte-exact reference/validation path) keeps the
    // captured bytes verbatim.
    if let Some(political) = political {
        write_political_scene_fields(&mut bytes, layout, political, camera.position[1])?;
    }
    decode_terrain_scene_cb0(&bytes, layout)?;
    Ok(bytes)
}

/// Drive the four virtual sun/moon `.x` longitudes from the PROVEN day-cycle
/// law (`light_law::virtual_rig_longitudes_x`) at the render `hour24`. This
/// REPLACES the old camera-follow FIT (the sun no longer tracks the pan
/// camera). Grounding:
/// - `common/defines/00_graphics.lua:1043-1050` fixes the rig's latitudes and
///   heights (SUN_LATITUDE 848, SECOND_*_LATITUDE 100 "to put a fake sun/moon
///   on the other side of the globe to hide the seam", SUN_HEIGHT 600,
///   SUN_HEIGHT_WATER 5000); no `.x` is defined in any file.
/// - The `.x` longitude is engine-runtime, CLOSED as a law in `LIGHT_LAW.md`
///   from session_FINAL_1's unpaused CB0 samples (residual < 0.01 map units):
///   `sun.x = MAP_SIZE_X * (1 - hour24/24)` (mod MAP_SIZE_X), `moon.x` is
///   antipodal, and the two seam-hiding "second" bodies are the SWAPPED pair.
///   The old renderer translated all four `.x` by `camera.x - reference_x`,
///   which its own comment admitted the engine did NOT do (every captured
///   frame carried rig `.x = 2816` regardless of camera). The mined corpus's
///   fixed `2816` is exactly this law at the noon hour those paused saves sat
///   on: `virtual_sun_longitude(12, 5632) = 2816` (byte-exact in f32).
/// - Only `.x` is written; the `.y/.z/.w` height/latitude lanes stay at the
///   captured/file-derived constants. This function is ONLY called when a
///   render hour is set (`day_night_hour = Some`); reference, byte-exact and
///   validation paths (`day_night_hour = None`) keep the captured rig verbatim.
pub(crate) fn write_virtual_light_rig_fields(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    hour24: f32,
) -> Result<()> {
    if !hour24.is_finite() {
        return Err(
            "production map renderer blocked: virtual light rig hour is non-finite"
                .to_string()
                .into(),
        );
    }
    let longitudes =
        crate::light_law::virtual_rig_longitudes_x(hour24, crate::light_law::defines::MAP_SIZE_X);
    for (field, longitude_x) in [
        "vVirtualSunPos",
        "vVirtualMoonPos",
        "vSecondVirtualSunPos",
        "vSecondVirtualMoonPos",
    ]
    .into_iter()
    .zip(longitudes)
    {
        let offset = constant_buffer_field(layout, field)?.offset;
        write_f32_at(bytes, offset, longitude_x)?;
    }
    Ok(())
}

/// NIGHT1: write the render sun-direction `DayNight_Hour_SunDir.yzw`, which is
/// **hour-invariant** — it is echoed from the captured `.yzw` verbatim (see
/// `light_law::render_sun_dir_yzw` for the mined law and its evidence). The
/// hour rotation lives entirely in `.x` (the terminator longitude phase written
/// by `write_day_night_hour_field`), so the sun direction for the recording's
/// date must NOT be rotated with the render hour.
///
/// This is deliberately a byte no-op (it reads the captured `.yzw` and writes
/// the same bits back through the law function), so it can never perturb the
/// `--hour 0.5` byte-identity gate. It is here so the "leave `.yzw` alone"
/// decision is an explicit, validated step of the write path and so the CB0
/// layout is re-checked (a float4 of the right name) whenever a hour is set.
pub(crate) fn write_sun_dir_yzw_hour_invariant_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    hour24: f32,
) -> Result<()> {
    if !hour24.is_finite() {
        return Err(
            "production map renderer blocked: sun-direction render hour is non-finite"
                .to_string()
                .into(),
        );
    }
    let field = constant_buffer_field(layout, "DayNight_Hour_SunDir")?;
    if field.type_name != "float4" {
        return Err(format!(
            "production map renderer blocked: DayNight_Hour_SunDir expected float4, got {}",
            field.type_name
        )
        .into());
    }
    // .yzw are the 2nd/3rd/4th f32 of the float4 (offsets +4/+8/+12).
    let captured = [
        read_f32_at(bytes, field.offset + 4)?,
        read_f32_at(bytes, field.offset + 8)?,
        read_f32_at(bytes, field.offset + 12)?,
    ];
    let render = crate::light_law::render_sun_dir_yzw(hour24, captured);
    write_f32_at(bytes, field.offset + 4, render[0])?;
    write_f32_at(bytes, field.offset + 8, render[1])?;
    write_f32_at(bytes, field.offset + 12, render[2])?;
    Ok(())
}

pub(crate) fn write_day_night_hour_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    hour: f32,
) -> Result<()> {
    if !hour.is_finite() || !(0.0..1.0).contains(&hour) {
        return Err(format!(
            "production map renderer blocked: DayNight_Hour_SunDir.x hour must be finite in [0,1), got {hour}"
        )
        .into());
    }
    let field = constant_buffer_field(layout, "DayNight_Hour_SunDir")?;
    if field.type_name != "float4" {
        return Err(format!(
            "production map renderer blocked: DayNight_Hour_SunDir expected float4, got {}",
            field.type_name
        )
        .into());
    }
    write_f32_at(bytes, field.offset, hour)
}

pub(crate) fn write_day_night_factor_gate_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    value: f32,
) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "production map renderer blocked: day/night factor gate must be finite in [0,1], got {value}"
        )
        .into());
    }
    let field = constant_buffer_field(layout, "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed")?;
    if field.type_name != "float4" {
        return Err(format!(
            "production map renderer blocked: vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed expected float4, got {}",
            field.type_name
        )
        .into());
    }
    write_f32_at(bytes, field.offset + 12, value)
}

pub(crate) fn write_global_time_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    seconds: f32,
) -> Result<()> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!(
            "production map renderer blocked: vGlobalTime must be finite non-negative seconds, got {seconds}"
        )
        .into());
    }
    let field = constant_buffer_field(layout, "vGlobalTime")?;
    if field.type_name != "float" {
        return Err(format!(
            "production map renderer blocked: vGlobalTime expected float, got {}",
            field.type_name
        )
        .into());
    }
    write_f32_at(bytes, field.offset, seconds)
}

pub(crate) fn write_float4x4_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
    value: [[f32; 4]; 4],
) -> Result<()> {
    let field = constant_buffer_field(layout, name)?;
    if field.type_name != "float4x4" {
        return Err(format!(
            "production map renderer blocked: {name} expected float4x4, got {}",
            field.type_name
        )
        .into());
    }
    for (row_index, row) in value.iter().enumerate() {
        for (column_index, component) in row.iter().enumerate() {
            write_f32_at(
                bytes,
                field.offset + row_index * 16 + column_index * 4,
                *component,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn write_vec3_field(
    bytes: &mut [u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
    value: [f32; 3],
) -> Result<()> {
    let field = constant_buffer_field(layout, name)?;
    if !matches!(field.type_name.as_str(), "float3" | "float4") {
        return Err(format!(
            "production map renderer blocked: {name} expected float3/float4 xyz field, got {}",
            field.type_name
        )
        .into());
    }
    write_f32_at(bytes, field.offset, value[0])?;
    write_f32_at(bytes, field.offset + 4, value[1])?;
    write_f32_at(bytes, field.offset + 8, value[2])
}

pub(crate) fn write_f32_at(bytes: &mut [u8], offset: usize, value: f32) -> Result<()> {
    if !value.is_finite() {
        return Err(
            "production map renderer blocked: constant-buffer camera value is non-finite".into(),
        );
    }
    let end = offset
        .checked_add(4)
        .ok_or("constant buffer offset overflow")?;
    let target = bytes.get_mut(offset..end).ok_or_else(|| {
        format!("production map renderer blocked: constant buffer missing bytes {offset}..{end}")
    })?;
    target.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub(crate) fn required_sampler_value(
    sampler_block: &str,
    sampler_name: &str,
    key: &str,
) -> Result<String> {
    parse_quoted_value(sampler_block, key).ok_or_else(|| {
        format!("production terrain renderer blocked: pdxmap.shader sampler {sampler_name} missing {key}")
            .into()
    })
}

pub(crate) fn validate_pdxmap_sampler_contract(pdxmap_shader_source: &Path) -> Result<()> {
    let text = read_latin1(pdxmap_shader_source)?;
    for (sampler_name, expected) in [
        (TERRAIN_DIFFUSE_SYMBOL, terrain_linear_wrap_sampler()),
        ("HeightNormal", terrain_linear_wrap_sampler()),
        (TERRAIN_COLOR_TINT_SYMBOL, terrain_linear_wrap_sampler()),
        ("SnowTexture", terrain_linear_wrap_sampler()),
        (TERRAIN_NORMAL_SYMBOL, terrain_linear_wrap_sampler()),
        ("TerrainIDMap", terrain_id_map_sampler()),
        ("SnowMudData", terrain_linear_wrap_sampler()),
        ("CityLightsAndSnowNoise", terrain_linear_wrap_sampler()),
        ("MudNormalSpec", terrain_linear_wrap_sampler()),
        ("MudDiffuseGloss", terrain_linear_wrap_sampler()),
    ] {
        let actual = parse_pdxmap_sampler_contract(&text, sampler_name)?;
        if actual != expected {
            return Err(format!(
                "production terrain renderer blocked: pdxmap.shader sampler {sampler_name} contract changed from {expected:?} to {actual:?}"
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn find_shader_cache_map_constants(
    shader_cache: &Path,
    required_terrain_id_slot: Option<u32>,
) -> Result<ShaderCacheMapConstants> {
    let shader_cache = require_dir(shader_cache, "shader cache directory")?;
    let mut candidates = Vec::new();
    collect_files_with_extension(&shader_cache, "scache", &mut candidates)?;
    candidates.sort();

    for path in candidates {
        let text = read_latin1(&path)?;
        let Some(terrain_id_srv_slot) = terrain_id_slot_from_shader_text(&text) else {
            continue;
        };
        if required_terrain_id_slot.is_some_and(|slot| slot != terrain_id_srv_slot) {
            continue;
        }
        let Some(map_size_x) = parse_shader_define_float(&text, "MAP_SIZE_X") else {
            continue;
        };
        let Some(map_size_y) = parse_shader_define_float(&text, "MAP_SIZE_Y") else {
            continue;
        };
        let Some(map_pow2_x) = parse_shader_define_float(&text, "MAP_POW2_X") else {
            continue;
        };
        let Some(map_pow2_y) = parse_shader_define_float(&text, "MAP_POW2_Y") else {
            continue;
        };
        let Some(fow_pow2_x) = parse_shader_define_float(&text, "FOW_POW2_X") else {
            continue;
        };
        let Some(fow_pow2_y) = parse_shader_define_float(&text, "FOW_POW2_Y") else {
            continue;
        };
        if !map_size_x.is_finite()
            || !map_size_y.is_finite()
            || !map_pow2_x.is_finite()
            || !map_pow2_y.is_finite()
            || !fow_pow2_x.is_finite()
            || !fow_pow2_y.is_finite()
            || map_size_x <= 0.0
            || map_size_y <= 0.0
            || map_pow2_x <= 0.0
            || map_pow2_y <= 0.0
            || fow_pow2_x <= 0.0
            || fow_pow2_y <= 0.0
        {
            continue;
        }
        return Ok(ShaderCacheMapConstants {
            cache_file: shader_cache_relative_name(&shader_cache, &path),
            map_size_x,
            map_size_y,
            map_pow2_x,
            map_pow2_y,
            fow_pow2_x,
            fow_pow2_y,
            terrain_id_srv_slot,
        });
    }

    Err(format!(
        "no pdxmap terrain shader cache with TerrainIDMap_Texture register({}), MAP_POW2, and FOW_POW2 defines found under {}",
        required_terrain_id_slot.map_or("tN".to_string(), |slot| format!("t{slot}")),
        shader_cache.display()
    )
    .into())
}

pub(crate) fn terrain_id_slot_from_shader_text(text: &str) -> Option<u32> {
    text.lines()
        .filter(|line| line.contains("TerrainIDMap_Texture"))
        .find_map(parse_register_t_slot)
}

pub(crate) fn parse_register_t_slot(line: &str) -> Option<u32> {
    let register = line.find("register")?;
    let after_register = &line[register..];
    let open = after_register.find('(')?;
    let mut chars = after_register[open + 1..].trim_start().chars();
    if chars.next()? != 't' {
        return None;
    }
    let digits: String = chars.take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

pub(crate) fn collect_files_with_extension(
    root: &Path,
    extension: &str,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, out)?;
            continue;
        }
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            out.push(path);
        }
    }
    Ok(())
}

pub(crate) fn parse_shader_define_float(text: &str, key: &str) -> Option<f32> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next()? != "#define" {
            continue;
        }
        if parts.next()? != key {
            continue;
        }
        let token = parts.next()?.trim_end_matches('f');
        if let Ok(value) = token.parse::<f32>() {
            return Some(value);
        }
    }
    None
}

pub(crate) fn shader_cache_relative_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|value| value.replace('\\', "/"))
        .or_else(|| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| "unknown_shader_cache_file".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;

    #[test]
    fn terrain_scene_cb0_app_camera_overrides_only_camera_fields() {
        let root = temp_test_dir("terrain_scene_cb0_app_camera_patch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let binding = frame
            .active_draw
            .constant_buffers
            .iter()
            .find(|binding| {
                binding.stage == "VS"
                    && binding.slot == TERRAIN_SCENE_CB0_BIND_POINT
                    && binding.declared_name.as_deref() == Some(TERRAIN_SCENE_CB0_SYMBOL)
            })
            .expect("VS TerrainSceneCB0 binding");
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("TerrainSceneCB0 layout");
        let mut bytes = valid_scene_cb0_bytes();
        write_f32_le(&mut bytes, 220, 7.0);
        write_f32_le(&mut bytes, 236, 42.0);
        let original = bytes.clone();
        let camera_matrix = [
            [2.0, 0.0, 0.0, 4.0],
            [0.0, 4.0, 0.0, 8.0],
            [0.0, 0.0, 5.0, 10.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let camera = ProductionMapCamera::new(camera_matrix, [44.0, 55.0, 66.0])
            .expect("finite production camera");

        let patched = terrain_scene_cb0_bytes_with_app_camera(
            &root,
            &frame.active_draw,
            binding,
            bytes,
            camera,
        )
        .expect("patch app-owned camera into TerrainSceneCB0");
        let decoded =
            decode_terrain_scene_cb0(&patched, &layout).expect("decode patched TerrainSceneCB0");

        assert_eq!(decoded.view_projection_matrix, camera_matrix);
        assert_eq!(
            read_float4x4_field(&patched, &layout, "InvViewProjMatrix")
                .expect("patched inverse matrix"),
            [
                [0.5, 0.0, 0.0, -2.0],
                [0.0, 0.25, 0.0, -2.0],
                [0.0, 0.0, 0.2, -2.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
        assert_close(decoded.v_cam_pos.x, 44.0);
        assert_close(decoded.v_cam_pos.y, 55.0);
        assert_close(decoded.v_cam_pos.z, 66.0);
        assert_close(decoded.hdr_range, 7.0);
        assert_close(decoded.v_global_time, 42.0);
        // Without a light-rig reference (the reference/byte-exact paths),
        // everything outside the camera fields stays captured verbatim,
        // including the virtual light rig at bytes 144..208.
        assert_eq!(&patched[128..208], &original[128..208]);
        assert_eq!(&patched[220..], &original[220..]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn light_rig_x_follows_the_day_cycle_law_not_the_camera() {
        // The four rig .x lanes are written from the day-cycle law at the
        // render hour and DO NOT depend on the camera x (camera-follow removed).
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("TerrainSceneCB0 layout");
        let read = |bytes: &[u8], offset: usize| {
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        // Two very different camera x values; identical hour -> identical rig.
        let camera_matrix = [
            [2.0, 0.0, 0.0, 4.0],
            [0.0, 4.0, 0.0, 8.0],
            [0.0, 0.0, 5.0, 10.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let make = |camera_x: f32, hour: Option<f32>| {
            let mut base = valid_scene_cb0_bytes();
            write_identity_float4x4(&mut base, 64);
            // Captured rig fields to be overwritten on the .x lane only.
            write_f32_le(&mut base, 144, 2816.0); // vVirtualSunPos.x
            write_f32_le(&mut base, 148, 600.0); //  .y SUN_HEIGHT
            write_f32_le(&mut base, 152, 848.0); //  .z SUN_LATITUDE
            write_f32_le(&mut base, 156, 5000.0); // .w SUN_HEIGHT_WATER
            write_f32_le(&mut base, 160, 0.0); //   vVirtualMoonPos.x
            write_f32_le(&mut base, 176, 0.0); //   vSecondVirtualSunPos.x
            write_f32_le(&mut base, 192, 2816.0); // vSecondVirtualMoonPos.x
            let camera = ProductionMapCamera::new(camera_matrix, [camera_x, 620.22, 66.0])
                .expect("finite production camera");
            terrain_scene_cb0_bytes_with_app_scene_layout(&layout, base, camera, hour, None, None)
                .expect("patch scene")
        };

        // Noon (fraction 0.5 -> hour24 12): law reproduces the captured rig
        // BYTE-IDENTICALLY, at any camera.
        for cam_x in [1400.0f32, 4200.0] {
            let patched = make(cam_x, Some(0.5));
            assert_eq!(read(&patched, 144).to_bits(), 2816.0f32.to_bits());
            assert_eq!(read(&patched, 160).to_bits(), 0.0f32.to_bits());
            assert_eq!(read(&patched, 176).to_bits(), 0.0f32.to_bits());
            assert_eq!(read(&patched, 192).to_bits(), 2816.0f32.to_bits());
            // .y/.z/.w untouched.
            assert_close(read(&patched, 148), 600.0);
            assert_close(read(&patched, 152), 848.0);
            assert_close(read(&patched, 156), 5000.0);
        }

        // Two cameras, one non-noon hour -> identical rig (no camera-follow).
        let a = make(1400.0, Some(0.25));
        let b = make(4200.0, Some(0.25));
        assert_eq!(a[144..160], b[144..160]);
        assert_eq!(a[176..208], b[176..208]);
        // 06:00 sun.x = 4224, moon.x = 1408 per the law.
        assert_close(read(&a, 144), 4224.0);
        assert_close(read(&a, 160), 1408.0);
        assert_close(read(&a, 176), 1408.0); // second sun rides moon longitude
        assert_close(read(&a, 192), 4224.0); // second moon rides sun longitude

        // hour = None keeps the captured rig verbatim (byte-exact path).
        let untouched = make(4200.0, None);
        assert_eq!(read(&untouched, 144).to_bits(), 2816.0f32.to_bits());
        assert_eq!(read(&untouched, 160).to_bits(), 0.0f32.to_bits());
        assert_eq!(read(&untouched, 176).to_bits(), 0.0f32.to_bits());
        assert_eq!(read(&untouched, 192).to_bits(), 2816.0f32.to_bits());
    }

    #[test]
    fn light_rig_writer_decodes_to_law_values_across_hours() {
        // The new unit test the spec asks for: for several hours the CB0 bytes
        // written by the rig writer decode to exactly the law values for all
        // four x fields, and y/z/w lanes are untouched.
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("TerrainSceneCB0 layout");
        let read = |bytes: &[u8], offset: usize| {
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        const MAPX: f32 = crate::light_law::defines::MAP_SIZE_X;
        // (offset, captured .y, .z, .w) for the four rig float4s.
        let rig = [
            (144usize, 600.0f32, 848.0f32, 5000.0f32), // vVirtualSunPos
            (160, 600.0, 1448.0, 550.0),               // vVirtualMoonPos
            (176, 600.0, 100.0, 5000.0),               // vSecondVirtualSunPos
            (192, 600.0, 100.0, 550.0),                // vSecondVirtualMoonPos
        ];
        for hour_fraction in [6.0f32 / 24.0, 12.0 / 24.0, 18.0 / 24.0, 23.5 / 24.0] {
            let mut base = valid_scene_cb0_bytes();
            write_identity_float4x4(&mut base, 64);
            for (off, y, z, w) in rig {
                write_f32_le(&mut base, off, 12345.0); // sentinel .x to be overwritten
                write_f32_le(&mut base, off + 4, y);
                write_f32_le(&mut base, off + 8, z);
                write_f32_le(&mut base, off + 12, w);
            }
            let camera = ProductionMapCamera::new(
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [999.0, 620.0, 66.0],
            )
            .expect("finite production camera");
            let patched = terrain_scene_cb0_bytes_with_app_scene_layout(
                &layout,
                base,
                camera,
                Some(hour_fraction),
                None,
                None,
            )
            .expect("patch scene");

            let hour24 = hour_fraction * 24.0;
            let expected = crate::light_law::virtual_rig_longitudes_x(hour24, MAPX);
            for (i, (off, y, z, w)) in rig.into_iter().enumerate() {
                assert_eq!(
                    read(&patched, off).to_bits(),
                    expected[i].to_bits(),
                    "rig .x at offset {off}, hour24 {hour24}"
                );
                // y/z/w lanes are byte-identical to the captured values.
                assert_eq!(read(&patched, off + 4).to_bits(), y.to_bits());
                assert_eq!(read(&patched, off + 8).to_bits(), z.to_bits());
                assert_eq!(read(&patched, off + 12).to_bits(), w.to_bits());
            }
        }
    }

    #[test]
    fn terrain_scene_cb0_camera_binding_requires_named_slot_zero() {
        let mut binding = MapFrameBufferBinding {
            stage: "VS".to_string(),
            slot: TERRAIN_SCENE_CB0_BIND_POINT,
            declared_name: Some(TERRAIN_SCENE_CB0_SYMBOL.to_string()),
            file: Some("cb0.bin".to_string()),
            byte_len: 416,
            fnv1a64: Some("hash".to_string()),
            stride: None,
            offset: None,
            dxgi_format: None,
            bind_flags: D3D11_BIND_CONSTANT_BUFFER,
            usage: D3D11_USAGE_DEFAULT,
            cpu_access_flags: 0,
        };

        assert!(is_terrain_scene_cb0_camera_binding(&binding));

        binding.slot = 1;
        assert!(!is_terrain_scene_cb0_camera_binding(&binding));

        binding.slot = TERRAIN_SCENE_CB0_BIND_POINT;
        binding.declared_name = Some("TerrainDrawConstants".to_string());
        assert!(!is_terrain_scene_cb0_camera_binding(&binding));

        binding.declared_name = None;
        assert!(!is_terrain_scene_cb0_camera_binding(&binding));
    }

    #[test]
    fn camera_constant_buffer_update_binding_dispatches_default_and_dynamic() {
        let valid = MapFrameBufferBinding {
            stage: "PS".to_string(),
            slot: TERRAIN_SCENE_CB0_BIND_POINT,
            declared_name: Some(TERRAIN_SCENE_CB0_SYMBOL.to_string()),
            file: Some("cb0.bin".to_string()),
            byte_len: 416,
            fnv1a64: Some("hash".to_string()),
            stride: None,
            offset: None,
            dxgi_format: None,
            bind_flags: D3D11_BIND_CONSTANT_BUFFER,
            usage: D3D11_USAGE_DEFAULT,
            cpu_access_flags: 0,
        };

        assert_eq!(
            camera_constant_buffer_update_method(&valid, 416, 7)
                .expect("default constant buffer is updateable"),
            TerrainSceneCb0CameraUpdateMethod::DefaultUpdateSubresource
        );

        let mut dynamic = valid.clone();
        dynamic.usage = D3D11_USAGE_DYNAMIC_CAPTURED;
        dynamic.cpu_access_flags = D3D11_CPU_ACCESS_WRITE_CAPTURED;
        assert_eq!(
            camera_constant_buffer_update_method(&dynamic, 416, 7)
                .expect("dynamic constant buffer is map-updateable"),
            TerrainSceneCb0CameraUpdateMethod::DynamicMapWriteDiscard
        );
        validate_camera_constant_buffer_update_binding(&dynamic, 416, 7)
            .expect("dynamic constant buffer validates through wrapper");

        let mut missing_bind = valid.clone();
        missing_bind.bind_flags = 0;
        let err = validate_camera_constant_buffer_update_binding(&missing_bind, 416, 7)
            .expect_err("missing constant-buffer bind flag must fail closed");
        assert!(err.to_string().contains("D3D11_BIND_CONSTANT_BUFFER"));

        let mut immutable = valid.clone();
        immutable.usage = D3D11_USAGE_IMMUTABLE_CAPTURED;
        let err = validate_camera_constant_buffer_update_binding(&immutable, 416, 7)
            .expect_err("immutable D3D usage must fail closed");
        assert!(err.to_string().contains("IMMUTABLE"));

        let mut staging = valid.clone();
        staging.usage = D3D11_USAGE_STAGING_CAPTURED;
        let err = validate_camera_constant_buffer_update_binding(&staging, 416, 7)
            .expect_err("staging D3D usage must fail closed");
        assert!(err.to_string().contains("STAGING"));

        let mut cpu_access = valid.clone();
        cpu_access.cpu_access_flags = 1;
        let err = validate_camera_constant_buffer_update_binding(&cpu_access, 416, 7)
            .expect_err("CPU access flags must fail closed");
        assert!(err.to_string().contains("CPU access flags"));

        let mut dynamic_without_cpu_write = dynamic.clone();
        dynamic_without_cpu_write.cpu_access_flags = 0;
        let err =
            validate_camera_constant_buffer_update_binding(&dynamic_without_cpu_write, 416, 7)
                .expect_err("dynamic constant buffer without CPU write must fail closed");
        assert!(err.to_string().contains("D3D11_CPU_ACCESS_WRITE"));

        let err = validate_camera_constant_buffer_update_binding(&valid, 410, 7)
            .expect_err("unaligned constant-buffer length must fail closed");
        assert!(err.to_string().contains("16-byte aligned"));

        let err = validate_camera_constant_buffer_update_binding(&valid, 0, 7)
            .expect_err("zero constant-buffer length must fail closed");
        assert!(err.to_string().contains("non-zero"));
    }

    #[test]
    fn terrain_scene_cb0_app_camera_layout_round_trips_captured_camera_bytes() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("TerrainSceneCB0 layout");
        let mut base = valid_scene_cb0_bytes();
        write_identity_float4x4(&mut base, 64);
        let decoded = decode_terrain_scene_cb0(&base, &layout).expect("decode captured CB0");
        let camera =
            production_map_camera_from_scene_cb0(decoded).expect("derive camera from captured CB0");

        let patched = terrain_scene_cb0_bytes_with_app_camera_layout(&layout, base.clone(), camera)
            .expect("patch captured camera back into CB0");

        assert_eq!(patched, base);
    }

    #[test]
    fn terrain_scene_cb0_app_camera_layout_repatches_from_captured_base() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("TerrainSceneCB0 layout");
        let base = valid_scene_cb0_bytes();
        let camera_a = ProductionMapCamera::new(
            [
                [2.0, 0.0, 0.0, 4.0],
                [0.0, 4.0, 0.0, 8.0],
                [0.0, 0.0, 5.0, 10.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [44.0, 55.0, 66.0],
        )
        .expect("finite camera A");
        let camera_b = ProductionMapCamera::new(
            [
                [3.0, 0.0, 0.0, 6.0],
                [0.0, 6.0, 0.0, 12.0],
                [0.0, 0.0, 7.0, 14.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [77.0, 88.0, 99.0],
        )
        .expect("finite camera B");

        let _patched_a =
            terrain_scene_cb0_bytes_with_app_camera_layout(&layout, base.clone(), camera_a)
                .expect("patch camera A");
        let patched_b =
            terrain_scene_cb0_bytes_with_app_camera_layout(&layout, base.clone(), camera_b)
                .expect("patch camera B from original base");
        let decoded_b = decode_terrain_scene_cb0(&patched_b, &layout).expect("decode camera B");

        assert_eq!(
            decoded_b.view_projection_matrix,
            camera_b.view_projection_matrix
        );
        assert_close(decoded_b.v_cam_pos.x, camera_b.position[0]);
        assert_close(decoded_b.v_cam_pos.y, camera_b.position[1]);
        assert_close(decoded_b.v_cam_pos.z, camera_b.position[2]);
        assert_eq!(&patched_b[220..], &base[220..]);
    }

    #[test]
    fn terrain_scene_cb0_layout_matches_standard_functions_constant_buffer() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        assert_eq!(layout.byte_len, 416);
        let cam = constant_buffer_field(&layout, "vCamPos").expect("vCamPos field");
        assert_eq!(cam.type_name, "float3");
        assert_eq!(cam.offset, 208);
        let hdr = constant_buffer_field(&layout, "HdrRange").expect("HdrRange field");
        assert_eq!(hdr.type_name, "float");
        assert_eq!(hdr.offset, 220);
        let snow_mud =
            constant_buffer_field(&layout, "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed")
                .expect("snow mud fade field");
        assert_eq!(snow_mud.type_name, "float4");
        assert_eq!(snow_mud.offset, 240);
    }

    #[test]
    fn terrain_scene_cb0_required_offsets_match_golden_field_table() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let offsets =
            terrain_scene_cb0_required_offsets(&layout).expect("required TerrainSceneCB0 offsets");
        assert_eq!(offsets.len(), TERRAIN_SCENE_CB0_FIELDS.len());
        for (name, golden_offset) in TERRAIN_SCENE_CB0_FIELDS.iter().copied() {
            assert_eq!(
                offsets.get(name).copied(),
                Some(golden_offset),
                "TerrainSceneCB0 field {name} offset drifted from golden table"
            );
        }
    }

    #[test]
    fn terrain_scene_cb0_decoder_reads_shader_offsets() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let mut bytes = vec![0u8; layout.byte_len];
        write_identity_float4x4(&mut bytes, 0);
        write_f32_le(&mut bytes, 208, 2695.059);
        write_f32_le(&mut bytes, 212, 310.11);
        write_f32_le(&mut bytes, 216, 1569.962);
        write_f32_le(&mut bytes, 220, 1.0);
        write_f32_le(&mut bytes, 128, 0.1);
        write_f32_le(&mut bytes, 132, 0.2);
        write_f32_le(&mut bytes, 136, 0.973);
        write_f32_le(&mut bytes, 140, 0.98);
        write_f32_le(&mut bytes, 236, 42.0);
        write_f32_le(&mut bytes, 240, 0.25);
        write_f32_le(&mut bytes, 244, 457.3188);
        write_f32_le(&mut bytes, 248, 1.0);
        write_f32_le(&mut bytes, 252, 5.0);
        write_f32_le(&mut bytes, 256, 0.75);
        write_f32_le(&mut bytes, 260, 12.0);
        write_f32_le(&mut bytes, 264, -0.2);
        write_f32_le(&mut bytes, 268, 0.9);
        write_f32_le(&mut bytes, 400, 2048.0);

        let decoded = decode_terrain_scene_cb0(&bytes, &layout).expect("decode TerrainSceneCB0");
        assert_close(decoded.view_projection_matrix[0][0], 1.0);
        assert_close(decoded.view_projection_matrix[3][3], 1.0);
        assert_close(decoded.v_cam_pos.x, 2695.059);
        assert_close(decoded.v_cam_pos.y, 310.11);
        assert_close(decoded.v_cam_pos.z, 1569.962);
        assert_close(decoded.hdr_range, 1.0);
        assert_close(decoded.v_gb_cam_dist_override_gb_outline_cutoff[0], 0.1);
        assert_close(decoded.v_gb_cam_dist_override_gb_outline_cutoff[3], 0.98);
        assert_close(decoded.v_global_time, 42.0);
        assert_close(
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[2],
            1.0,
        );
        assert_close(decoded.day_night_hour_sun_dir[1], 12.0);
        assert_close(decoded.gb_texture_height, 2048.0);
    }

    #[test]
    fn terrain_scene_cb0_day_night_hour_patch_updates_only_hour_component() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let mut bytes = valid_scene_cb0_bytes();
        write_f32_le(&mut bytes, 256, 0.041_666_668);
        write_f32_le(&mut bytes, 260, 0.0);
        write_f32_le(&mut bytes, 264, -0.078_338_5);
        write_f32_le(&mut bytes, 268, 0.996_926_84);

        write_day_night_hour_field(&mut bytes, &layout, 0.75)
            .expect("patch normalized replay hour");
        let decoded = decode_terrain_scene_cb0(&bytes, &layout).expect("decode patched CB0");

        assert_close(decoded.day_night_hour_sun_dir[0], 0.75);
        assert_close(decoded.day_night_hour_sun_dir[1], 0.0);
        assert_close(decoded.day_night_hour_sun_dir[2], -0.078_338_5);
        assert_close(decoded.day_night_hour_sun_dir[3], 0.996_926_84);
        let err = write_day_night_hour_field(&mut bytes, &layout, 1.0)
            .expect_err("hour outside normalized range must fail");
        assert!(err.to_string().contains("DayNight_Hour_SunDir.x"));
    }

    #[test]
    fn terrain_scene_cb0_global_time_patch_updates_only_c14_w() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let mut bytes = valid_scene_cb0_bytes();
        for (index, byte) in bytes[224..256].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(7).wrapping_add(3);
        }
        let original = bytes.clone();

        write_global_time_field(&mut bytes, &layout, 10.5).expect("patch vGlobalTime");

        assert_eq!(&bytes[224..236], &original[224..236]);
        assert_close(
            read_f32_at(&bytes, 236).expect("read patched vGlobalTime"),
            10.5,
        );
        assert_eq!(&bytes[240..256], &original[240..256]);
    }

    #[test]
    fn terrain_scene_cb0_scene_patch_enables_day_night_factor_gate() {
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let mut bytes = valid_scene_cb0_bytes();
        write_f32_le(&mut bytes, 240, 0.0);
        write_f32_le(&mut bytes, 244, 23.291_967);
        write_f32_le(&mut bytes, 248, 0.0);
        write_f32_le(&mut bytes, 252, 0.0);
        write_f32_le(&mut bytes, 256, 0.041_666_668);
        let camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 0.0, 0.0],
        )
        .expect("finite production camera");

        let patched = terrain_scene_cb0_bytes_with_app_scene_layout(
            &layout,
            bytes,
            camera,
            Some(0.75),
            None,
            None,
        )
        .expect("patch replay hour and day/night gate");
        let decoded = decode_terrain_scene_cb0(&patched, &layout).expect("decode patched CB0");

        assert_close(decoded.day_night_hour_sun_dir[0], 0.75);
        assert_close(
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[0],
            0.0,
        );
        assert_close(
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[1],
            23.291_967,
        );
        assert_close(
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[2],
            0.0,
        );
        assert_close(
            decoded.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3],
            1.0,
        );
    }

    #[test]
    fn day_night_hour_patch_leaves_sun_dir_yzw_bit_identical_at_every_hour() {
        // NIGHT1: the sun-direction `.yzw` is hour-invariant. Patching the
        // render hour through the FULL scene-patch path must leave the captured
        // `.yzw` bytes bit-for-bit unchanged at EVERY hour (the terminator moves
        // via `.x` only). This is the same-bundle A/B that guards `--hour 0.5`
        // byte-identity for the `.yzw` lanes.
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        // Captured `.yzw` from a real swept sample (session_CAPFIX2_FULL_2):
        // .y = 0, .z = 0.0773575..., .w = 0.9970026...
        let cap_y = 0.0f32;
        let cap_z = f32::from_bits(0x3d9e_6d9c); // 0.077357...
        let cap_w = f32::from_bits(0x3f7f_3b9f); // 0.997003...
        let camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 0.0, 0.0],
        )
        .expect("finite production camera");
        for hour in [0.0f32, 0.25, 0.5, 0.75, 23.5 / 24.0] {
            let mut base = valid_scene_cb0_bytes();
            write_identity_float4x4(&mut base, 64);
            // Seed the captured `.yzw` (offsets 260/264/268) and an arbitrary `.x`.
            write_f32_le(&mut base, 256, 0.6666667);
            write_f32_le(&mut base, 260, cap_y);
            write_f32_le(&mut base, 264, cap_z);
            write_f32_le(&mut base, 268, cap_w);
            let patched = terrain_scene_cb0_bytes_with_app_scene_layout(
                &layout,
                base,
                camera.clone(),
                Some(hour),
                None,
                None,
            )
            .expect("patch replay hour");
            // `.x` moved to the render hour...
            assert_eq!(
                read_f32_at(&patched, 256).unwrap().to_bits(),
                hour.to_bits(),
                "hour {hour}: .x must be the render hour"
            );
            // ...but `.yzw` is bit-identical to the captured vector.
            assert_eq!(
                read_f32_at(&patched, 260).unwrap().to_bits(),
                cap_y.to_bits(),
                "hour {hour}: .y (dir.x) moved"
            );
            assert_eq!(
                read_f32_at(&patched, 264).unwrap().to_bits(),
                cap_z.to_bits(),
                "hour {hour}: .z (dir.y) moved"
            );
            assert_eq!(
                read_f32_at(&patched, 268).unwrap().to_bits(),
                cap_w.to_bits(),
                "hour {hour}: .w (dir.z) moved"
            );
        }
    }

    #[test]
    fn sun_dir_yzw_writer_is_a_full_buffer_byte_noop_at_hour_0_5() {
        // The load-bearing byte-identity A/B: the NIGHT1 `.yzw` writer must be a
        // strict no-op on the produced CB0 at the --hour 0.5 default. We build
        // the same scene CB0 the render produces (rig .x + hour .x + FoW gate),
        // then run `write_sun_dir_yzw_hour_invariant_field` and assert the ENTIRE
        // 416-byte buffer is byte-identical before/after. This is what keeps the
        // locked v3 baseline byte-exact when my `.yzw` step is added to the path.
        let layout = terrain_scene_cb0_layout(&minimal_standard_functions_source())
            .expect("parse TerrainSceneCB0 layout");
        let mut base = valid_scene_cb0_bytes();
        write_identity_float4x4(&mut base, 64);
        // Seed a realistic captured `.yzw` (noon baseline live_PAN2 values).
        write_f32_le(&mut base, 256, 0.5); // .x hour fraction (12:00)
        write_f32_le(&mut base, 260, 0.0); // .y (dir.x)
        write_f32_le(&mut base, 264, -0.078_338_5); // .z (dir.y)
        write_f32_le(&mut base, 268, 0.996_926_84); // .w (dir.z)

        // A = the buffer BEFORE the .yzw writer (what the old path produced at
        // this point). B = after running it. hour24 = 0.5 * 24 = 12.0.
        let before = base.clone();
        write_sun_dir_yzw_hour_invariant_field(&mut base, &layout, 0.5 * 24.0)
            .expect("write hour-invariant .yzw");
        assert_eq!(
            before, base,
            "NIGHT1 .yzw writer perturbed the CB0 bytes at --hour 0.5 (byte-identity broken)"
        );
    }

    #[test]
    fn pdxmap_sampler_contract_parses_shader_source() {
        assert_eq!(
            parse_pdxmap_sampler_contract(&minimal_pdxmap_sampler_source(), "TerrainDiffuse")
                .expect("parse TerrainDiffuse sampler"),
            terrain_linear_wrap_sampler()
        );
        assert_eq!(
            parse_pdxmap_sampler_contract(&minimal_pdxmap_sampler_source(), "TerrainIDMap")
                .expect("parse TerrainIDMap sampler"),
            terrain_id_map_sampler()
        );
        assert_eq!(
            parse_pdxmap_sampler_contract(&minimal_pdxmap_sampler_source(), "MudDiffuseGloss")
                .expect("parse MudDiffuseGloss sampler"),
            terrain_linear_wrap_sampler()
        );
    }

    #[test]
    fn pdxwater_sampler_contract_parses_shader_source() {
        let text = minimal_pdxwater_sampler_source();
        for expectation in PDXWATER_SAMPLER_EXPECTATIONS {
            validate_shader_sampler_expectation_against_source(
                &text,
                "pdxwater.shader",
                expectation,
            )
            .expect("water sampler source matches renderer contract");
        }
        assert_eq!(
            shader_sampler_mode_from_contract(
                "pdxwater.shader",
                WATER_REFRACTION_RESOURCE_SYMBOL,
                &parse_shader_sampler_contract(
                    &text,
                    "pdxwater.shader",
                    WATER_REFRACTION_RESOURCE_SYMBOL,
                )
                .expect("parse WaterRefraction sampler"),
            )
            .expect("WaterRefraction mode"),
            ShaderSamplerMode::LinearClamp
        );
        assert_eq!(
            parse_shader_sampler_index(&text, "pdxwater.shader", WATER_REFRACTION_RESOURCE_SYMBOL)
                .expect("parse WaterRefraction index"),
            5
        );
        assert_eq!(
            shader_sampler_mode_from_contract(
                "pdxwater.shader",
                LIGHT_INDEX_MAP_SYMBOL,
                &parse_shader_sampler_contract(&text, "pdxwater.shader", LIGHT_INDEX_MAP_SYMBOL)
                    .expect("parse LightIndexMap sampler"),
            )
            .expect("LightIndexMap mode"),
            ShaderSamplerMode::PointClamp
        );
        assert_eq!(
            parse_shader_sampler_index(&text, "pdxwater.shader", LIGHT_INDEX_MAP_SYMBOL)
                .expect("parse LightIndexMap index"),
            10
        );
        assert_eq!(
            parse_shader_sampler_resource_type(&text, "pdxwater.shader", "ReflectionCubeMap")
                .expect("parse ReflectionCubeMap type"),
            ShaderSamplerResourceType::Cube
        );
        assert_eq!(
            parse_shader_sampler_resource_type(&text, "pdxwater.shader", SHADOW_MAP_SYMBOL)
                .expect("parse ShadowMap type"),
            ShaderSamplerResourceType::Shadow
        );
    }

    #[test]
    fn shader_cache_map_constants_require_terrain_id_map_signature() {
        let root = temp_test_dir("shader_cache_signature");
        fs::create_dir_all(&root).expect("create temp shader cache dir");
        fs::write(
            root.join("wrong.scache"),
            b"#define MAP_SIZE_X 5632.000\n#define MAP_SIZE_Y 2048.000\n#define MAP_POW2_X 1.000\n#define MAP_POW2_Y 1.000\n#define FOW_POW2_X 1.000\n#define FOW_POW2_Y 1.000\n",
        )
        .expect("write non-map shader cache");
        let err = find_shader_cache_map_constants(&root, None)
            .expect_err("shader cache without TerrainIDMap signature must not match");
        assert!(err
            .to_string()
            .contains("TerrainIDMap_Texture register(tN)"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn shader_cache_map_constants_parse_pdxmap_cache() {
        let root = temp_test_dir("shader_cache_map_constants");
        fs::create_dir_all(root.join("dx11_win32").join("ps_5_0"))
            .expect("create temp shader cache dir");
        fs::write(
            root.join("dx11_win32").join("ps_5_0").join("map.scache"),
            b"#define MAP_SIZE_X 5632.000\n#define MAP_SIZE_Y 2048.000\n#define MAP_POW2_X 1.000\n#define MAP_POW2_Y 1.000\n#define FOW_POW2_X 0.500\n#define FOW_POW2_Y 0.250\nTexture2D TerrainIDMap_Texture : register(t3);\n",
        )
        .expect("write map shader cache");

        let constants =
            find_shader_cache_map_constants(&root, None).expect("parse map shader cache");
        assert_eq!(constants.cache_file, "dx11_win32/ps_5_0/map.scache");
        assert_eq!(constants.map_size_x, 5632.0);
        assert_eq!(constants.map_size_y, 2048.0);
        assert_eq!(constants.map_pow2_x, 1.0);
        assert_eq!(constants.map_pow2_y, 1.0);
        assert_eq!(constants.fow_pow2_x, 0.5);
        assert_eq!(constants.fow_pow2_y, 0.25);
        assert_eq!(constants.terrain_id_srv_slot, 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn shader_cache_map_constants_can_filter_active_terrain_id_slot() {
        let root = temp_test_dir("shader_cache_active_slot");
        fs::create_dir_all(root.join("dx11_win32").join("ps_5_0"))
            .expect("create temp shader cache dir");
        fs::write(
            root.join("dx11_win32").join("ps_5_0").join("map_t3.scache"),
            b"#define MAP_SIZE_X 5632.000\n#define MAP_SIZE_Y 2048.000\n#define MAP_POW2_X 1.000\n#define MAP_POW2_Y 1.000\n#define FOW_POW2_X 0.500\n#define FOW_POW2_Y 0.250\nTexture2D TerrainIDMap_Texture : register(t3);\n",
        )
        .expect("write t3 map shader cache");
        fs::write(
            root.join("dx11_win32").join("ps_5_0").join("map_t5.scache"),
            b"#define MAP_SIZE_X 5632.000\n#define MAP_SIZE_Y 2048.000\n#define MAP_POW2_X 1.000\n#define MAP_POW2_Y 1.000\n#define FOW_POW2_X 1.000\n#define FOW_POW2_Y 1.000\nTexture2D TerrainIDMap_Texture : register(t5);\n",
        )
        .expect("write t5 map shader cache");

        let constants =
            find_shader_cache_map_constants(&root, Some(5)).expect("parse active slot cache");
        assert_eq!(constants.cache_file, "dx11_win32/ps_5_0/map_t5.scache");
        assert_eq!(constants.terrain_id_srv_slot, 5);
        assert_eq!(constants.fow_pow2_x, 1.0);
        assert_eq!(constants.fow_pow2_y, 1.0);
        let _ = fs::remove_dir_all(&root);
    }
}
