use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn load_map_frame_terrain_shader_constants(
    frame: &MapFrameRuntimeBundle,
    bundle: &Path,
) -> Result<MapFrameTerrainShaderConstants> {
    let terrain_srv = map_frame_terrain_srv(&frame.active_draw)?;
    if let Some(constants) = &frame.shader_cache_constants {
        validate_map_frame_shader_cache_constants_evidence(frame)?;
        let constants = MapFrameTerrainShaderConstants {
            map_size_x: constants.map_size_x,
            map_size_y: constants.map_size_y,
            map_pow2_x: constants.map_pow2_x,
            map_pow2_y: constants.map_pow2_y,
            fow_pow2_x: constants.fow_pow2_x,
            fow_pow2_y: constants.fow_pow2_y,
        };
        validate_map_frame_shader_constants(constants)?;
        return Ok(constants);
    }
    let manifest_path = bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| {
        format!(
            "production terrain renderer blocked: map-frame vertex shader replay needs shader-cache constants in map_frame_bundle.json or legacy --bundle shader-cache terrain runtime evidence; {} is not readable: {}",
            manifest_path.display(),
            error
        )
    })?)?;
    let runtime_path = bundle.join(&manifest.files.terrain_runtime);
    let runtime: TerrainRuntimeInputs =
        serde_json::from_slice(&fs::read(&runtime_path).map_err(|error| {
            format!(
                "production terrain renderer blocked: map-frame vertex shader replay needs MAP_POW2_X/Y shader-cache evidence; {} is not readable: {}",
                runtime_path.display(),
                error
            )
        })?)?;
    if runtime.schema != TERRAIN_RUNTIME_SCHEMA {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain runtime schema {} for map-frame vertex shader constants",
            runtime.schema
        )
        .into());
    }
    if runtime.terrain_id_map.width != terrain_srv.width as usize
        || runtime.terrain_id_map.height != terrain_srv.height as usize
    {
        return Err(format!(
            "production terrain renderer blocked: map-frame TerrainIDMap is {}x{}, but shader-cache runtime evidence is {}x{}",
            terrain_srv.width,
            terrain_srv.height,
            runtime.terrain_id_map.width,
            runtime.terrain_id_map.height
        )
        .into());
    }
    if runtime.map_pow2.source.provenance.trim().is_empty()
        || runtime.map_pow2.source.extraction.trim().is_empty()
    {
        return Err(
            "production terrain renderer blocked: MAP_POW2_X/Y must carry shader-cache provenance for map-frame vertex shader replay"
                .into(),
        );
    }
    let constants = MapFrameTerrainShaderConstants {
        map_size_x: terrain_srv.width as f32,
        map_size_y: terrain_srv.height as f32,
        map_pow2_x: runtime.map_pow2.x,
        map_pow2_y: runtime.map_pow2.y,
        fow_pow2_x: runtime.fow_pow2.x,
        fow_pow2_y: runtime.fow_pow2.y,
    };
    validate_map_frame_shader_constants(constants)?;
    Ok(constants)
}

pub(crate) fn validate_map_frame_shader_constants(
    constants: MapFrameTerrainShaderConstants,
) -> Result<()> {
    if !constants.map_size_x.is_finite()
        || !constants.map_size_y.is_finite()
        || !constants.map_pow2_x.is_finite()
        || !constants.map_pow2_y.is_finite()
        || !constants.fow_pow2_x.is_finite()
        || !constants.fow_pow2_y.is_finite()
        || constants.map_size_x <= 0.0
        || constants.map_size_y <= 0.0
        || constants.map_pow2_x <= 0.0
        || constants.map_pow2_y <= 0.0
        || constants.fow_pow2_x <= 0.0
        || constants.fow_pow2_y <= 0.0
    {
        return Err(
            "production terrain renderer blocked: map-frame shader constants must be finite positive values"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn map_frame_terrain_srv(draw: &MapFrameDraw) -> Result<&MapFrameTextureBinding> {
    let terrain_slot = draw
        .shader
        .reflection_resources
        .get(TERRAIN_ID_RESOURCE_SYMBOL)
        .copied()
        .ok_or("production terrain renderer blocked: active shader reflection does not declare TerrainIDMap")?;
    draw.shader_resources
        .iter()
        .find(|resource| resource.stage == "PS" && resource.slot == terrain_slot)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: active shader declares TerrainIDMap at PS slot {terrain_slot}, but the map-frame bundle has no matching SRV"
            )
            .into()
        })
}

#[cfg(test)]
pub(crate) fn replay_map_frame_terrain_draw(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    constants: MapFrameTerrainShaderConstants,
) -> Result<MapFrameTerrainReplaySummary> {
    let (outputs, unique_vertex_count) =
        decode_map_frame_terrain_draw_outputs(draw, bundle_dir, constants)?;
    let mut summary: Option<MapFrameTerrainReplaySummary> = None;
    for output in outputs {
        update_map_frame_replay_summary(
            &mut summary,
            draw.draw_index,
            usize::try_from(draw.draw_count)?,
            unique_vertex_count,
            output,
        );
    }
    let mut summary = summary.ok_or(
        "production terrain renderer blocked: map-frame terrain replay emitted no vertices",
    )?;
    summary.unique_vertex_count = unique_vertex_count;
    Ok(summary)
}

pub(crate) fn decode_map_frame_terrain_draw_outputs(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    constants: MapFrameTerrainShaderConstants,
) -> Result<(Vec<TerrainVertexShaderOutput>, usize)> {
    validate_map_frame_shader_constants(constants)?;
    if draw.draw_call != "DrawIndexed" {
        return Err(format!(
            "production terrain renderer blocked: map-frame terrain replay currently supports the captured DrawIndexed path, got {}",
            draw.draw_call
        )
        .into());
    }
    if draw.topology != Some(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) {
        return Err(format!(
            "production terrain renderer blocked: map-frame terrain replay requires triangle-list topology {}, got {:?}",
            D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, draw.topology
        )
        .into());
    }
    if draw.draw_count % 3 != 0 {
        return Err(format!(
            "production terrain renderer blocked: indexed triangle-list draw count {} is not divisible by 3",
            draw.draw_count
        )
        .into());
    }
    let position_element =
        required_map_frame_input_element(draw, "POSITION", 0, DXGI_FORMAT_R16G16B16A16_SINT)?;
    let height_element =
        required_map_frame_input_element(draw, "TEXCOORD", 0, DXGI_FORMAT_R16G16_SINT)?;
    let position_binding = required_map_frame_vertex_buffer(draw, position_element.input_slot)?;
    let height_binding = required_map_frame_vertex_buffer(draw, height_element.input_slot)?;
    let position_bytes =
        read_map_frame_buffer_bytes(bundle_dir, position_binding, "vertex buffer")?;
    let height_bytes = read_map_frame_buffer_bytes(bundle_dir, height_binding, "vertex buffer")?;
    let index_buffer = draw
        .index_buffer
        .as_ref()
        .ok_or("production terrain renderer blocked: DrawIndexed map-frame terrain replay has no index buffer")?;
    let index_bytes = read_map_frame_index_bytes(bundle_dir, index_buffer)?;
    let cb0 = decode_map_frame_terrain_scene_cb0(draw, bundle_dir)?;
    let cb1 = decode_map_frame_terrain_draw_cb1(draw, bundle_dir)?;

    let mut unique_indices = BTreeSet::new();
    let mut outputs = Vec::with_capacity(usize::try_from(draw.draw_count)?);
    for i in 0..usize::try_from(draw.draw_count)? {
        let index_location = usize::try_from(draw.draw_start)?
            .checked_add(i)
            .ok_or("production terrain renderer blocked: map-frame index location overflow")?;
        let index = read_map_frame_index(&index_bytes, index_buffer, index_location)?;
        let vertex_index_i64 = i64::from(index) + i64::from(draw.base_vertex_location);
        if vertex_index_i64 < 0 {
            return Err(format!(
                "production terrain renderer blocked: map-frame base vertex {} makes index {} negative",
                draw.base_vertex_location, index
            )
            .into());
        }
        let vertex_index = usize::try_from(vertex_index_i64)?;
        unique_indices.insert(u32::try_from(vertex_index)?);
        let input = decode_map_frame_terrain_vertex_input(
            vertex_index,
            position_element,
            position_binding,
            &position_bytes,
            height_element,
            height_binding,
            &height_bytes,
        )?;
        let output = emulate_pdxmap_terrain_vertex_shader(input, cb0, cb1, constants)?;
        outputs.push(output);
    }
    if outputs.is_empty() {
        return Err(
            "production terrain renderer blocked: map-frame terrain replay emitted no vertices"
                .into(),
        );
    }
    Ok((outputs, unique_indices.len()))
}

pub(crate) fn decode_map_frame_terrain_scene_cb0(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<TerrainSceneCb0Decoded> {
    decode_map_frame_terrain_scene_cb0_for_stage(draw, bundle_dir, "VS")
}

pub(crate) fn decode_map_frame_ps_terrain_scene_cb0(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<TerrainSceneCb0Decoded> {
    decode_map_frame_terrain_scene_cb0_for_stage(draw, bundle_dir, "PS")
}

pub(crate) fn decode_map_frame_terrain_scene_cb0_for_stage(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    stage: &str,
) -> Result<TerrainSceneCb0Decoded> {
    for buffer in draw
        .constant_buffers
        .iter()
        .filter(|buffer| buffer.stage == stage && buffer.slot == TERRAIN_SCENE_CB0_BIND_POINT)
    {
        let bytes = read_map_frame_buffer_bytes(bundle_dir, buffer, "constant buffer")?;
        if let Some(layout) =
            terrain_scene_cb0_camera_layout_for_binding(bundle_dir, draw, buffer, bytes.len())?
        {
            return decode_terrain_scene_cb0(&bytes, &layout);
        }
    }
    Err(format!(
        "production terrain renderer blocked: map-frame draw {} has no {TERRAIN_SCENE_CB0_SYMBOL}-compatible {stage} constant-buffer capture at slot {TERRAIN_SCENE_CB0_BIND_POINT}",
        draw.draw_index
    )
    .into())
}

pub(crate) fn required_map_frame_input_element<'a>(
    draw: &'a MapFrameDraw,
    semantic_name: &str,
    semantic_index: u32,
    expected_format: u32,
) -> Result<&'a RuntimeInputElementMeta> {
    let element = draw
        .input_layout
        .elements
        .iter()
        .find(|element| {
            element.semantic_name.eq_ignore_ascii_case(semantic_name)
                && element.semantic_index == semantic_index
        })
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: map-frame input layout missing {semantic_name}{semantic_index}"
            )
        })?;
    if element.format != expected_format {
        return Err(format!(
            "production terrain renderer blocked: map-frame input layout {semantic_name}{semantic_index} format {} does not match captured pdxmap terrain replay format {}",
            element.format, expected_format
        )
        .into());
    }
    if element.input_slot_class != D3D11_INPUT_PER_VERTEX_DATA {
        return Err(format!(
            "production terrain renderer blocked: map-frame input layout {semantic_name}{semantic_index} input slot class {} is not D3D11_INPUT_PER_VERTEX_DATA",
            element.input_slot_class
        )
        .into());
    }
    if element.instance_data_step_rate != 0 {
        return Err(format!(
            "production terrain renderer blocked: map-frame input layout {semantic_name}{semantic_index} has nonzero instance step rate {}",
            element.instance_data_step_rate
        )
        .into());
    }
    Ok(element)
}

pub(crate) fn required_map_frame_vertex_buffer(
    draw: &MapFrameDraw,
    slot: u32,
) -> Result<&MapFrameBufferBinding> {
    draw.vertex_buffers.get(&slot.to_string()).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: map-frame input slot {slot} has no captured vertex buffer"
        )
        .into()
    })
}

pub(crate) fn read_map_frame_buffer_bytes(
    bundle_dir: &Path,
    binding: &MapFrameBufferBinding,
    role: &str,
) -> Result<Vec<u8>> {
    let file = binding.file.as_ref().ok_or_else(|| {
        format!(
            "production terrain renderer blocked: map-frame {role} slot {} has no file",
            binding.slot
        )
    })?;
    let path = bundle_dir.join(file);
    let bytes = fs::read(&path)?;
    if bytes.len() != binding.byte_len {
        return Err(format!(
            "production terrain renderer blocked: map-frame {role} {} has {} bytes, metadata expects {}",
            path.display(),
            bytes.len(),
            binding.byte_len
        )
        .into());
    }
    Ok(bytes)
}

pub(crate) fn read_map_frame_index_bytes(
    bundle_dir: &Path,
    index_buffer: &MapFrameIndexBufferBinding,
) -> Result<Vec<u8>> {
    let file = index_buffer
        .file
        .as_ref()
        .ok_or("production terrain renderer blocked: map-frame index buffer has no file")?;
    let path = bundle_dir.join(file);
    let bytes = fs::read(&path)?;
    if bytes.len() != index_buffer.byte_len {
        return Err(format!(
            "production terrain renderer blocked: map-frame index buffer {} has {} bytes, metadata expects {}",
            path.display(),
            bytes.len(),
            index_buffer.byte_len
        )
        .into());
    }
    Ok(bytes)
}

pub(crate) fn read_map_frame_index(
    bytes: &[u8],
    index_buffer: &MapFrameIndexBufferBinding,
    index_location: usize,
) -> Result<u32> {
    let (byte_width, reader): (usize, fn(&[u8], usize) -> Result<u32>) =
        match index_buffer.dxgi_format {
            DXGI_FORMAT_R16_UINT => (2, read_u16_as_u32_at),
            DXGI_FORMAT_R32_UINT => (4, read_u32_at),
            other => {
                return Err(format!(
                    "production terrain renderer blocked: unsupported map-frame index buffer DXGI format {other}"
                )
                .into())
            }
        };
    let start =
        usize::try_from(index_buffer.offset)?
            .checked_add(index_location.checked_mul(byte_width).ok_or(
                "production terrain renderer blocked: map-frame index byte offset overflow",
            )?)
            .ok_or("production terrain renderer blocked: map-frame index byte offset overflow")?;
    reader(bytes, start)
}

pub(crate) fn decode_map_frame_terrain_vertex_input(
    vertex_index: usize,
    position_element: &RuntimeInputElementMeta,
    position_binding: &MapFrameBufferBinding,
    position_bytes: &[u8],
    height_element: &RuntimeInputElementMeta,
    height_binding: &MapFrameBufferBinding,
    height_bytes: &[u8],
) -> Result<TerrainVertexShaderInput> {
    let position_offset =
        map_frame_vertex_element_byte_offset(vertex_index, position_element, position_binding)?;
    let height_offset =
        map_frame_vertex_element_byte_offset(vertex_index, height_element, height_binding)?;
    Ok(TerrainVertexShaderInput {
        position: [
            read_i16_at(position_bytes, position_offset)?,
            read_i16_at(position_bytes, position_offset + 2)?,
            read_i16_at(position_bytes, position_offset + 4)?,
            read_i16_at(position_bytes, position_offset + 6)?,
        ],
        height: [
            read_i16_at(height_bytes, height_offset)?,
            read_i16_at(height_bytes, height_offset + 2)?,
        ],
    })
}

pub(crate) fn map_frame_vertex_element_byte_offset(
    vertex_index: usize,
    element: &RuntimeInputElementMeta,
    binding: &MapFrameBufferBinding,
) -> Result<usize> {
    let stride = usize::try_from(binding.stride.ok_or_else(|| {
        format!(
            "production terrain renderer blocked: vertex buffer slot {} has no stride",
            binding.slot
        )
    })?)?;
    let base_offset = usize::try_from(binding.offset.unwrap_or(0))?;
    let element_offset = usize::try_from(element.aligned_byte_offset)?;
    base_offset
        .checked_add(
            vertex_index.checked_mul(stride).ok_or(
                "production terrain renderer blocked: map-frame vertex byte offset overflow",
            )?,
        )
        .and_then(|offset| offset.checked_add(element_offset))
        .ok_or_else(|| {
            "production terrain renderer blocked: map-frame vertex byte offset overflow".into()
        })
}

pub(crate) fn decode_map_frame_terrain_draw_cb1(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<TerrainDrawCb1Decoded> {
    let buffer = draw
        .constant_buffers
        .iter()
        .find(|buffer| buffer.stage == "VS" && buffer.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: map-frame draw {} has no VS constant-buffer slot {} required by {}",
                draw.draw_index, TERRAIN_DRAW_CB1_BIND_POINT, TERRAIN_DRAW_CB1_LAYOUT_SOURCE
            )
        })?;
    let bytes = read_map_frame_buffer_bytes(bundle_dir, buffer, "constant buffer")?;
    let source = draw
        .shader
        .source_evidence
        .iter()
        .find(|source| source.as_str() == "gfx/FX/pdxmap.fxh")
        .ok_or("production terrain renderer blocked: map-frame shader identity must cite gfx/FX/pdxmap.fxh for terrain draw CB1 layout")?;
    let source_path = bundle_dir.join("vanilla").join(source);
    let layout = terrain_draw_cb1_layout(&read_latin1(&source_path)?)?;
    decode_terrain_draw_cb1(&bytes, &layout)
}

pub(crate) fn terrain_draw_cb1_layout(shader_source: &str) -> Result<ShaderConstantBufferLayout> {
    let layout = parse_constant_buffer_layout(shader_source, 1, 32)?;
    let required_offsets = BTreeMap::from([
        ("ShadowMapTextureMatrix", 0usize),
        ("QuadOffset_Scale_IsDetail", 64usize),
        ("vSnap", 80usize),
        (
            "vBorderLookup_HeightScale_UseMultisample_SeasonLerp",
            96usize,
        ),
    ]);
    for (field, expected_offset) in required_offsets {
        let actual = layout
            .fields
            .iter()
            .find(|candidate| candidate.name == field)
            .ok_or_else(|| {
                format!(
                    "production terrain renderer blocked: {TERRAIN_DRAW_CB1_LAYOUT_SOURCE} missing field {field}"
                )
            })?;
        if actual.offset != expected_offset {
            return Err(format!(
                "production terrain renderer blocked: {TERRAIN_DRAW_CB1_LAYOUT_SOURCE} field {field} moved from byte {expected_offset} to {}",
                actual.offset
            )
            .into());
        }
    }
    if layout.byte_len != 112 {
        return Err(format!(
            "production terrain renderer blocked: {TERRAIN_DRAW_CB1_LAYOUT_SOURCE} byte size changed from 112 to {}",
            layout.byte_len
        )
        .into());
    }
    Ok(layout)
}

pub(crate) fn decode_terrain_draw_cb1(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
) -> Result<TerrainDrawCb1Decoded> {
    validate_constant_buffer_byte_len(bytes.len(), layout.byte_len)?;
    let decoded = TerrainDrawCb1Decoded {
        shadow_map_texture_matrix: read_float4x4_field(bytes, layout, "ShadowMapTextureMatrix")?,
        quad_offset_scale_is_detail: read_float4_field(bytes, layout, "QuadOffset_Scale_IsDetail")?,
        v_snap: read_float4_field(bytes, layout, "vSnap")?,
        v_border_lookup_height_scale_use_multisample_season_lerp: read_float4_field(
            bytes,
            layout,
            "vBorderLookup_HeightScale_UseMultisample_SeasonLerp",
        )?,
    };
    let finite = decoded
        .shadow_map_texture_matrix
        .iter()
        .flatten()
        .chain(decoded.quad_offset_scale_is_detail.iter())
        .chain(decoded.v_snap.iter())
        .chain(
            decoded
                .v_border_lookup_height_scale_use_multisample_season_lerp
                .iter(),
        )
        .all(|value| value.is_finite());
    if !finite {
        return Err(
            "production terrain renderer blocked: terrain draw CB1 decoded non-finite values"
                .into(),
        );
    }
    Ok(decoded)
}

pub(crate) fn decode_map_frame_point_light_cb2(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<TerrainPointLightCb2Decoded> {
    let buffer = draw
        .constant_buffers
        .iter()
        .find(|buffer| buffer.stage == "PS" && buffer.slot == TERRAIN_POINT_LIGHT_CB2_BIND_POINT)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: map-frame draw {} has no PS constant-buffer slot {} required by {}",
                draw.draw_index, TERRAIN_POINT_LIGHT_CB2_BIND_POINT, TERRAIN_POINT_LIGHT_CB2_LAYOUT_SOURCE
            )
        })?;
    let bytes = read_map_frame_buffer_bytes(bundle_dir, buffer, "constant buffer")?;
    let source_path = bundle_dir
        .join("vanilla")
        .join("gfx/FX/tiled_pointlights.fxh");
    let layout = terrain_point_light_cb2_layout(&read_latin1(&source_path)?)?;
    decode_terrain_point_light_cb2(&bytes, &layout)
}

pub(crate) fn terrain_point_light_cb2_layout(
    shader_source: &str,
) -> Result<ShaderConstantBufferLayout> {
    let layout = parse_constant_buffer_layout(shader_source, 2, 62)?;
    let field = layout
        .fields
        .iter()
        .find(|candidate| candidate.name == "GridStart_InvCellSize")
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: {TERRAIN_POINT_LIGHT_CB2_LAYOUT_SOURCE} missing field GridStart_InvCellSize"
            )
        })?;
    if field.offset != 0 {
        return Err(format!(
            "production terrain renderer blocked: {TERRAIN_POINT_LIGHT_CB2_LAYOUT_SOURCE} field GridStart_InvCellSize moved from byte 0 to {}",
            field.offset
        )
        .into());
    }
    if layout.byte_len != 16 {
        return Err(format!(
            "production terrain renderer blocked: {TERRAIN_POINT_LIGHT_CB2_LAYOUT_SOURCE} byte size changed from 16 to {}",
            layout.byte_len
        )
        .into());
    }
    Ok(layout)
}

pub(crate) fn decode_terrain_point_light_cb2(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
) -> Result<TerrainPointLightCb2Decoded> {
    validate_constant_buffer_byte_len(bytes.len(), layout.byte_len)?;
    let decoded = TerrainPointLightCb2Decoded {
        grid_start_inv_cell_size: read_float4_field(bytes, layout, "GridStart_InvCellSize")?,
    };
    if decoded
        .grid_start_inv_cell_size
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(
            "production terrain renderer blocked: tiled point-light CB2 decoded non-finite values"
                .into(),
        );
    }
    Ok(decoded)
}

pub(crate) fn emulate_pdxmap_terrain_vertex_shader(
    input: TerrainVertexShaderInput,
    cb0: TerrainSceneCb0Decoded,
    cb1: TerrainDrawCb1Decoded,
    constants: MapFrameTerrainShaderConstants,
) -> Result<TerrainVertexShaderOutput> {
    let scale = cb1.quad_offset_scale_is_detail[2];
    let mut pos = Vec2 {
        x: f32::from(input.position[0]) * scale + cb1.quad_offset_scale_is_detail[0],
        y: f32::from(input.position[1]) * scale + cb1.quad_offset_scale_is_detail[1],
    };
    let z = input.position[2];
    if !(0..=4).contains(&z) {
        return Err(format!(
            "production terrain renderer blocked: pdxmap terrain position.z {} is outside shader-documented [0,4] snap range",
            z
        )
        .into());
    }
    let v_use_alt_height = if z == 0 {
        0.0
    } else {
        z.clamp(0, 1) as f32 * cb1.v_snap[usize::try_from(z - 1)?]
    };
    pos.x += v_use_alt_height * (1.0 - f32::from(input.position[3])) * scale;
    pos.y += v_use_alt_height * f32::from(input.position[3]) * scale;
    let uv = Vec2 {
        x: (pos.x + 0.5) / constants.map_size_x,
        y: (pos.y + 0.5) / constants.map_size_y,
    };
    let uv2 = Vec2 {
        x: uv.x * constants.map_pow2_x,
        y: ((pos.y + 0.5 - constants.map_size_y) / -constants.map_size_y) * constants.map_pow2_y,
    };
    let v_height = (f32::from(input.height[0]) * (1.0 - v_use_alt_height)
        + f32::from(input.height[1]) * v_use_alt_height)
        * 0.01;
    let prepos = Vec3 {
        x: pos.x,
        y: v_height,
        z: pos.y,
    };
    let prepos4 = Vec4 {
        x: prepos.x,
        y: prepos.y,
        z: prepos.z,
        w: 1.0,
    };
    let position = mul_float4x4_float4(cb0.view_projection_matrix, prepos4);
    let v_shadow_proj = mul_float4x4_float4(cb1.shadow_map_texture_matrix, prepos4);
    let v_screen_coord = Vec4 {
        x: position.x * 0.5 + position.w * 0.5,
        y: position.w * 0.5 - position.y * 0.5,
        z: position.w,
        w: position.w,
    };
    let output = TerrainVertexShaderOutput {
        input,
        position,
        uv,
        uv2,
        prepos,
        v_shadow_proj,
        v_screen_coord,
    };
    if [
        output.position.x,
        output.position.y,
        output.position.z,
        output.position.w,
        output.uv.x,
        output.uv.y,
        output.uv2.x,
        output.uv2.y,
        output.prepos.x,
        output.prepos.y,
        output.prepos.z,
        output.v_shadow_proj.x,
        output.v_shadow_proj.y,
        output.v_shadow_proj.z,
        output.v_shadow_proj.w,
        output.v_screen_coord.x,
        output.v_screen_coord.y,
        output.v_screen_coord.z,
        output.v_screen_coord.w,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return Err(
            "production terrain renderer blocked: pdxmap terrain vertex replay produced non-finite output"
                .into(),
        );
    }
    Ok(output)
}

pub(crate) fn mul_float4x4_float4(matrix: [[f32; 4]; 4], value: Vec4) -> Vec4 {
    Vec4 {
        x: matrix[0][0] * value.x
            + matrix[0][1] * value.y
            + matrix[0][2] * value.z
            + matrix[0][3] * value.w,
        y: matrix[1][0] * value.x
            + matrix[1][1] * value.y
            + matrix[1][2] * value.z
            + matrix[1][3] * value.w,
        z: matrix[2][0] * value.x
            + matrix[2][1] * value.y
            + matrix[2][2] * value.z
            + matrix[2][3] * value.w,
        w: matrix[3][0] * value.x
            + matrix[3][1] * value.y
            + matrix[3][2] * value.z
            + matrix[3][3] * value.w,
    }
}

#[cfg(test)]
pub(crate) fn update_map_frame_replay_summary(
    summary: &mut Option<MapFrameTerrainReplaySummary>,
    draw_index: u32,
    index_count: usize,
    unique_vertex_count: usize,
    output: TerrainVertexShaderOutput,
) {
    let triangle_count = index_count / 3;
    match summary {
        Some(summary) => {
            summary.unique_vertex_count = unique_vertex_count;
            summary.uv_min.x = summary.uv_min.x.min(output.uv.x);
            summary.uv_min.y = summary.uv_min.y.min(output.uv.y);
            summary.uv_max.x = summary.uv_max.x.max(output.uv.x);
            summary.uv_max.y = summary.uv_max.y.max(output.uv.y);
            summary.uv2_min.x = summary.uv2_min.x.min(output.uv2.x);
            summary.uv2_min.y = summary.uv2_min.y.min(output.uv2.y);
            summary.uv2_max.x = summary.uv2_max.x.max(output.uv2.x);
            summary.uv2_max.y = summary.uv2_max.y.max(output.uv2.y);
            summary.prepos_min.x = summary.prepos_min.x.min(output.prepos.x);
            summary.prepos_min.y = summary.prepos_min.y.min(output.prepos.y);
            summary.prepos_min.z = summary.prepos_min.z.min(output.prepos.z);
            summary.prepos_max.x = summary.prepos_max.x.max(output.prepos.x);
            summary.prepos_max.y = summary.prepos_max.y.max(output.prepos.y);
            summary.prepos_max.z = summary.prepos_max.z.max(output.prepos.z);
        }
        None => {
            *summary = Some(MapFrameTerrainReplaySummary {
                draw_index,
                index_count,
                triangle_count,
                unique_vertex_count,
                uv_min: output.uv,
                uv_max: output.uv,
                uv2_min: output.uv2,
                uv2_max: output.uv2,
                prepos_min: output.prepos,
                prepos_max: output.prepos,
            });
        }
    }
}

pub(crate) fn read_float4x4_field(
    bytes: &[u8],
    layout: &ShaderConstantBufferLayout,
    name: &str,
) -> Result<[[f32; 4]; 4]> {
    let field = constant_buffer_field(layout, name)?;
    if field.type_name != "float4x4" {
        return Err(format!(
            "production terrain renderer blocked: {name} expected float4x4, got {}",
            field.type_name
        )
        .into());
    }
    Ok([
        [
            read_f32_at(bytes, field.offset)?,
            read_f32_at(bytes, field.offset + 4)?,
            read_f32_at(bytes, field.offset + 8)?,
            read_f32_at(bytes, field.offset + 12)?,
        ],
        [
            read_f32_at(bytes, field.offset + 16)?,
            read_f32_at(bytes, field.offset + 20)?,
            read_f32_at(bytes, field.offset + 24)?,
            read_f32_at(bytes, field.offset + 28)?,
        ],
        [
            read_f32_at(bytes, field.offset + 32)?,
            read_f32_at(bytes, field.offset + 36)?,
            read_f32_at(bytes, field.offset + 40)?,
            read_f32_at(bytes, field.offset + 44)?,
        ],
        [
            read_f32_at(bytes, field.offset + 48)?,
            read_f32_at(bytes, field.offset + 52)?,
            read_f32_at(bytes, field.offset + 56)?,
            read_f32_at(bytes, field.offset + 60)?,
        ],
    ])
}

pub(crate) fn read_u16_as_u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(2)
        .ok_or("production terrain renderer blocked: u16 offset overflow")?;
    let raw = bytes.get(offset..end).ok_or_else(|| {
        format!("production terrain renderer blocked: missing u16 bytes {offset}..{end}")
    })?;
    Ok(u32::from(u16::from_le_bytes([raw[0], raw[1]])))
}

pub(crate) fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or("production terrain renderer blocked: u32 offset overflow")?;
    let raw = bytes.get(offset..end).ok_or_else(|| {
        format!("production terrain renderer blocked: missing u32 bytes {offset}..{end}")
    })?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

pub(crate) fn read_i16_at(bytes: &[u8], offset: usize) -> Result<i16> {
    let end = offset
        .checked_add(2)
        .ok_or("production terrain renderer blocked: i16 offset overflow")?;
    let raw = bytes.get(offset..end).ok_or_else(|| {
        format!("production terrain renderer blocked: missing i16 bytes {offset}..{end}")
    })?;
    Ok(i16::from_le_bytes([raw[0], raw[1]]))
}

pub(crate) fn render_production_terrain(
    bundle: &Path,
    output: &Path,
) -> Result<ProductionTerrainRenderSummary> {
    let inputs = validate_production_terrain_renderer(bundle)?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(bundle.join("manifest.json"))?)?;
    let contract: MapMaterialContract = serde_json::from_slice(&fs::read(
        bundle.join(&manifest.files.map_material_contract),
    )?)?;
    let runtime = inputs
        .runtime
        .as_ref()
        .ok_or("production terrain renderer blocked: missing legacy terrain runtime inputs")?;
    let terrain_id_map_path = inputs
        .terrain_id_map
        .as_ref()
        .ok_or("production terrain renderer blocked: missing legacy TerrainIDMap input")?;
    let terrain_id_map = read_rgba_raw(
        terrain_id_map_path,
        runtime.terrain_id_map.width,
        runtime.terrain_id_map.height,
    )?;
    let terrain_atlas_meta = material_source_meta(&contract, "map/terrain/atlas0.dds")?;
    let terrain_atlas = read_rgba_mip_chain_raw(
        &inputs.terrain_diffuse_atlas_mips,
        terrain_atlas_meta.dimensions.width,
        terrain_atlas_meta.dimensions.height,
        terrain_atlas_meta.mip_count,
    )?;
    let terrain_normal_atlas_meta =
        material_source_meta(&contract, "map/terrain/atlas_normal0.dds")?;
    let terrain_normal_atlas = read_rgba_mip_chain_raw(
        &inputs.terrain_normal_atlas_mips,
        terrain_normal_atlas_meta.dimensions.width,
        terrain_normal_atlas_meta.dimensions.height,
        terrain_normal_atlas_meta.mip_count,
    )?;
    let height_normal_dimensions = bundle_source_dimensions(&manifest, "map/world_normal.bmp")?;
    let height_normal = RgbxImage {
        width: height_normal_dimensions.width,
        height: height_normal_dimensions.height,
        pixels: read_u32_raw(
            &inputs.height_normal,
            height_normal_dimensions.width * height_normal_dimensions.height,
        )?,
    };
    let terrain_color_tint_dimensions =
        material_dimensions(&contract, "map/terrain/colormap_rgb_cityemissivemask_a.dds")?;
    let terrain_color_tint = RgbaMipChain {
        levels: vec![read_rgba_raw(
            &inputs.terrain_color_tint,
            terrain_color_tint_dimensions.width,
            terrain_color_tint_dimensions.height,
        )?],
        color_space: RgbaColorSpace::Linear,
    };

    let pixels = render_terrain_idmap_diffuse(
        manifest.dimensions.width,
        manifest.dimensions.height,
        &terrain_id_map,
        &terrain_atlas,
        &terrain_normal_atlas,
        &height_normal,
        &terrain_color_tint,
        runtime.map_pow2.x,
        runtime.map_pow2.y,
    )?;
    write_u32_raw(output, &pixels)?;

    Ok(ProductionTerrainRenderSummary {
        validated_input_count: inputs.validated_input_count(),
        terrain_id_map_width: runtime.terrain_id_map.width,
        terrain_id_map_height: runtime.terrain_id_map.height,
        map_pow2_x: runtime.map_pow2.x,
        map_pow2_y: runtime.map_pow2.y,
        fow_pow2_x: runtime.fow_pow2.x,
        fow_pow2_y: runtime.fow_pow2.y,
    })
}

pub(crate) fn render_map_frame_terrain(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    bundle: &Path,
    output: &Path,
) -> Result<MapFrameTerrainRenderSummary> {
    let inputs = validate_production_terrain_renderer_for_map_frame(bundle)?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(bundle.join("manifest.json"))?)?;
    let contract: MapMaterialContract = serde_json::from_slice(&fs::read(
        bundle.join(&manifest.files.map_material_contract),
    )?)?;
    let constants = load_map_frame_terrain_shader_constants(frame, bundle)?;
    validate_map_frame_base_terrain_shipped_resources(frame, bundle, &manifest, &contract)?;
    validate_map_frame_post_diffuse_inputs(frame, bundle, bundle_dir)?;
    let terrain_srv = map_frame_terrain_srv(&frame.active_draw)?;
    let terrain_file = terrain_srv.file.as_ref().ok_or(
        "production terrain renderer blocked: map-frame TerrainIDMap has no captured file",
    )?;
    let map_w = usize::try_from(terrain_srv.width)?;
    let map_h = usize::try_from(terrain_srv.height)?;
    let terrain_id_map = read_rgba_raw(&bundle_dir.join(terrain_file), map_w, map_h)?;
    let terrain_atlas_meta = material_source_meta(&contract, "map/terrain/atlas0.dds")?;
    let terrain_atlas = read_rgba_mip_chain_raw_with_color_space(
        &inputs.terrain_diffuse_atlas_mips,
        terrain_atlas_meta.dimensions.width,
        terrain_atlas_meta.dimensions.height,
        terrain_atlas_meta.mip_count,
        map_frame_shipped_material_color_space(&frame.active_draw, TERRAIN_DIFFUSE_SYMBOL)?,
    )?;
    let terrain_normal_atlas_meta =
        material_source_meta(&contract, "map/terrain/atlas_normal0.dds")?;
    let terrain_normal_atlas = read_rgba_mip_chain_raw_with_color_space(
        &inputs.terrain_normal_atlas_mips,
        terrain_normal_atlas_meta.dimensions.width,
        terrain_normal_atlas_meta.dimensions.height,
        terrain_normal_atlas_meta.mip_count,
        map_frame_shipped_material_color_space(&frame.active_draw, TERRAIN_NORMAL_SYMBOL)?,
    )?;
    let height_normal_dimensions = bundle_source_dimensions(&manifest, "map/world_normal.bmp")?;
    let height_normal = RgbxImage {
        width: height_normal_dimensions.width,
        height: height_normal_dimensions.height,
        pixels: read_u32_raw(
            &inputs.height_normal,
            height_normal_dimensions.width * height_normal_dimensions.height,
        )?,
    };
    let terrain_color_tint_dimensions =
        material_dimensions(&contract, "map/terrain/colormap_rgb_cityemissivemask_a.dds")?;
    let terrain_color_tint = RgbaMipChain {
        levels: vec![read_rgba_raw(
            &inputs.terrain_color_tint,
            terrain_color_tint_dimensions.width,
            terrain_color_tint_dimensions.height,
        )?],
        color_space: map_frame_shipped_material_color_space(
            &frame.active_draw,
            TERRAIN_COLOR_TINT_SYMBOL,
        )?,
    };
    let lod =
        terrain_mipmap_lod_for_pixel_step(map_w, map_h, constants.map_pow2_x, constants.map_pow2_y);
    let mut out = vec![0u32; map_w * map_h];
    let mut covered = vec![false; map_w * map_h];
    let mut index_count = 0usize;
    let mut triangle_count = 0usize;
    for draw in &frame.draws {
        let post_diffuse =
            load_map_frame_post_diffuse_resources(frame, draw, bundle_dir, bundle, constants)?;
        let resources = TerrainPixelResources {
            map_w,
            map_h,
            terrain_id_map: &terrain_id_map,
            terrain_atlas: &terrain_atlas,
            terrain_normal_atlas: &terrain_normal_atlas,
            height_normal: &height_normal,
            terrain_color_tint: &terrain_color_tint,
            post_diffuse: Some(&post_diffuse),
            lod,
        };
        let (outputs, _) = decode_map_frame_terrain_draw_outputs(draw, bundle_dir, constants)?;
        index_count += outputs.len();
        let mut chunks = outputs.chunks_exact(3);
        for triangle in &mut chunks {
            let tri = [triangle[0], triangle[1], triangle[2]];
            rasterize_map_frame_terrain_triangle(&mut out, &mut covered, &resources, tri)?;
            triangle_count += 1;
        }
        if !chunks.remainder().is_empty() {
            return Err(format!(
                "production terrain renderer blocked: map-frame draw {} emitted non-triangle vertex remainder {}",
                draw.draw_index,
                chunks.remainder().len()
            )
            .into());
        }
    }
    let covered_pixel_count = covered.iter().filter(|covered| **covered).count();
    if covered_pixel_count == 0 {
        return Err(
            "production terrain renderer blocked: captured terrain triangles covered no pixels"
                .into(),
        );
    }
    write_u32_raw(output, &out)?;
    Ok(MapFrameTerrainRenderSummary {
        draw_count: frame.draws.len(),
        index_count,
        triangle_count,
        covered_pixel_count,
        output_width: map_w,
        output_height: map_h,
        snow_mud_data_loaded: true,
    })
}

pub(crate) fn load_map_frame_post_diffuse_resources(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    bundle: &Path,
    constants: MapFrameTerrainShaderConstants,
) -> Result<TerrainPostDiffuseResources> {
    let (manifest, contract) = load_post_diffuse_bundle_context(bundle)?;
    let snow_mud_data =
        load_map_frame_resource_rgba(frame, draw, bundle_dir, SNOW_MUD_DATA_SYMBOL)?;
    let snow_texture = load_shipped_material_mip_chain(
        bundle,
        &manifest,
        &contract,
        draw,
        SNOW_TEXTURE_SYMBOL,
        "map/terrain/snow_normal_rgb_diffuse_a.dds",
    )?;
    let city_lights_snow_noise = load_shipped_material_mip_chain(
        bundle,
        &manifest,
        &contract,
        draw,
        CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL,
        "map/terrain/citylights_rgb_snowmask_a_0.dds",
    )?;
    let mud_diffuse_gloss = load_shipped_material_mip_chain(
        bundle,
        &manifest,
        &contract,
        draw,
        MUD_DIFFUSE_GLOSS_SYMBOL,
        "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
    )?;
    let mud_normal_spec = load_shipped_material_mip_chain(
        bundle,
        &manifest,
        &contract,
        draw,
        MUD_NORMAL_SPEC_SYMBOL,
        "map/terrain/mud_normal_rgb_spec_a_0.dds",
    )?;
    let province_secondary_color_map = load_map_frame_resource_rgba_chain(
        frame,
        draw,
        bundle_dir,
        PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
    )?;
    let gradient_border_channel1 = load_map_frame_resource_rgba_chain(
        frame,
        draw,
        bundle_dir,
        GRADIENT_BORDER_CHANNEL1_SYMBOL,
    )?;
    let gradient_border_channel2 = load_map_frame_resource_rgba_chain(
        frame,
        draw,
        bundle_dir,
        GRADIENT_BORDER_CHANNEL2_SYMBOL,
    )?;
    let light_index_map =
        load_map_frame_resource_rgba(frame, draw, bundle_dir, LIGHT_INDEX_MAP_SYMBOL)?;
    validate_exact_map_frame_resource_dimensions(
        LIGHT_INDEX_MAP_SYMBOL,
        light_index_map.width,
        light_index_map.height,
        LIGHT_INDEX_TEXTURE_SIZE,
        LIGHT_INDEX_TEXTURE_SIZE,
    )?;
    let light_data_map =
        load_map_frame_resource_rgba32f(frame, draw, bundle_dir, LIGHT_DATA_MAP_SYMBOL)?;
    validate_exact_map_frame_resource_dimensions(
        LIGHT_DATA_MAP_SYMBOL,
        light_data_map.width,
        light_data_map.height,
        LIGHT_DATA_TEXTURE_SIZE,
        1,
    )?;
    Ok(TerrainPostDiffuseResources {
        snow_mud_data,
        snow_texture,
        city_lights_snow_noise,
        mud_diffuse_gloss,
        mud_normal_spec,
        province_secondary_color_map,
        gradient_border_channel1,
        gradient_border_channel2,
        light_index_map,
        light_data_map,
        shadow_map: map_frame_shadow_map_state(draw)?,
        scene_cb0: decode_map_frame_ps_terrain_scene_cb0(draw, bundle_dir)?,
        point_light_cb2: decode_map_frame_point_light_cb2(draw, bundle_dir)?,
        constants,
    })
}

pub(crate) fn load_map_frame_resource_rgba(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    name: &str,
) -> Result<RgbaImage> {
    let resource = map_frame_shader_resource(draw, name)?;
    validate_map_frame_runtime_rgba_resource(name, resource)?;
    validate_map_frame_runtime_file_link(frame, resource, name)?;
    let file = resource.file.as_ref().ok_or_else(|| {
        format!("production terrain renderer blocked: runtime resource {name} has no file")
    })?;
    read_rgba_raw(
        &bundle_dir.join(file),
        usize::try_from(resource.width)?,
        usize::try_from(resource.height)?,
    )
}

pub(crate) fn load_map_frame_resource_rgba_chain(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    name: &str,
) -> Result<RgbaMipChain> {
    let resource = map_frame_shader_resource(draw, name)?;
    validate_map_frame_runtime_rgba_resource(name, resource)?;
    validate_map_frame_runtime_file_link(frame, resource, name)?;
    let color_space = runtime_rgba_color_space(name, resource)?;
    let file = resource.file.as_ref().ok_or_else(|| {
        format!("production terrain renderer blocked: runtime resource {name} has no file")
    })?;
    Ok(RgbaMipChain {
        levels: vec![read_rgba_raw(
            &bundle_dir.join(file),
            usize::try_from(resource.width)?,
            usize::try_from(resource.height)?,
        )?],
        color_space,
    })
}

pub(crate) fn load_map_frame_resource_rgba32f(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    name: &str,
) -> Result<Rgba32fImage> {
    let resource = map_frame_shader_resource(draw, name)?;
    validate_map_frame_runtime_r32g32b32a32_float_resource(name, resource)?;
    validate_map_frame_runtime_file_link(frame, resource, name)?;
    let file = resource.file.as_ref().ok_or_else(|| {
        format!("production terrain renderer blocked: runtime resource {name} has no file")
    })?;
    read_rgba32f_raw(
        &bundle_dir.join(file),
        usize::try_from(resource.width)?,
        usize::try_from(resource.height)?,
    )
}

pub(crate) fn load_shipped_material_mip_chain(
    bundle: &Path,
    manifest: &Manifest,
    contract: &MapMaterialContract,
    draw: &MapFrameDraw,
    symbol: &str,
    source: &str,
) -> Result<RgbaMipChain> {
    let meta = material_source_meta(contract, source)?;
    let resource = map_frame_shader_resource(draw, symbol)?;
    let color_space = shipped_material_color_space(symbol, resource)?;
    if let Some(mip_output) = meta.mip_output.as_deref() {
        return read_rgba_mip_chain_raw_with_color_space(
            &required_bundle_file(bundle, source, mip_output)?,
            meta.dimensions.width,
            meta.dimensions.height,
            meta.mip_count,
            color_space,
        );
    }
    let base = required_shader_source_path(bundle, manifest, source)?;
    Ok(RgbaMipChain {
        levels: vec![read_rgba_raw(
            &base,
            meta.dimensions.width,
            meta.dimensions.height,
        )?],
        color_space,
    })
}

pub(crate) fn map_frame_shipped_material_color_space(
    draw: &MapFrameDraw,
    symbol: &str,
) -> Result<RgbaColorSpace> {
    shipped_material_color_space(symbol, map_frame_shader_resource(draw, symbol)?)
}

pub(crate) fn shipped_material_color_space(
    symbol: &str,
    resource: &MapFrameTextureBinding,
) -> Result<RgbaColorSpace> {
    let (expected_format, color_space) = match symbol {
        TERRAIN_DIFFUSE_SYMBOL | CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL | MUD_DIFFUSE_GLOSS_SYMBOL => {
            (DXGI_FORMAT_BC3_UNORM_SRGB, RgbaColorSpace::Srgb)
        }
        TERRAIN_NORMAL_SYMBOL | SNOW_TEXTURE_SYMBOL | MUD_NORMAL_SPEC_SYMBOL => {
            (DXGI_FORMAT_BC3_UNORM, RgbaColorSpace::Linear)
        }
        TERRAIN_COLOR_TINT_SYMBOL => (DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, RgbaColorSpace::Srgb),
        _ => {
            return Err(format!(
                "production terrain renderer blocked: shipped material {symbol} has no strict live SRV format expectation"
            )
            .into())
        }
    };
    if resource.dxgi_format != expected_format {
        return Err(format!(
            "production terrain renderer blocked: shipped material {symbol} expected live SRV DXGI format {expected_format}, got {}",
            resource.dxgi_format
        )
        .into());
    }
    Ok(color_space)
}

pub(crate) fn runtime_rgba_color_space(
    name: &str,
    resource: &MapFrameTextureBinding,
) -> Result<RgbaColorSpace> {
    match resource.dxgi_format {
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM => Ok(RgbaColorSpace::Linear),
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Ok(RgbaColorSpace::Srgb),
        other => Err(format!(
            "production terrain renderer blocked: runtime resource {name} has unsupported RGBA color-space DXGI format {other}"
        )
        .into()),
    }
}

pub(crate) fn validate_exact_map_frame_resource_dimensions(
    name: &str,
    width: usize,
    height: usize,
    expected_width: usize,
    expected_height: usize,
) -> Result<()> {
    if width != expected_width || height != expected_height {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} is {width}x{height}, expected {expected_width}x{expected_height}"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn map_frame_shadow_map_state(draw: &MapFrameDraw) -> Result<TerrainShadowMapState> {
    let resource = map_frame_shader_resource(draw, SHADOW_MAP_SYMBOL)?;
    if !resource.is_null {
        return Err(format!(
            "production terrain renderer blocked: non-null {SHADOW_MAP_SYMBOL} capture is present at PS slot {}, but shadow texture sampling is not implemented yet",
            resource.slot
        )
        .into());
    }
    validate_map_frame_null_shadow_resource(draw, resource)?;
    Ok(TerrainShadowMapState::NullSrv)
}

pub(crate) fn material_dimensions(
    contract: &MapMaterialContract,
    source: &str,
) -> Result<Dimensions> {
    Ok(material_source_meta(contract, source)?.dimensions.clone())
}

pub(crate) fn material_source_meta<'a>(
    contract: &'a MapMaterialContract,
    source: &str,
) -> Result<&'a MaterialSourceMeta> {
    contract.shipped_sources.get(source).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: missing material source metadata for {source}"
        )
        .into()
    })
}

pub(crate) fn bundle_source_dimensions(manifest: &Manifest, source: &str) -> Result<Dimensions> {
    manifest
        .source
        .visual_input_dimensions
        .get(source)
        .cloned()
        .ok_or_else(|| {
            format!("production terrain renderer blocked: missing source dimensions for {source}")
                .into()
        })
}

pub(crate) fn render_terrain_idmap_diffuse(
    map_w: usize,
    map_h: usize,
    terrain_id_map: &RgbaImage,
    terrain_atlas: &RgbaMipChain,
    terrain_normal_atlas: &RgbaMipChain,
    height_normal: &RgbxImage,
    terrain_color_tint: &RgbaMipChain,
    map_pow2_x: f32,
    map_pow2_y: f32,
) -> Result<Vec<u32>> {
    if map_w == 0 || map_h == 0 {
        return Err("production terrain renderer blocked: map dimensions must be nonzero".into());
    }
    if !map_pow2_x.is_finite() || !map_pow2_y.is_finite() || map_pow2_x <= 0.0 || map_pow2_y <= 0.0
    {
        return Err(
            "production terrain renderer blocked: MAP_POW2_X/Y must be finite positive values"
                .into(),
        );
    }
    let resources = TerrainPixelResources {
        map_w,
        map_h,
        terrain_id_map,
        terrain_atlas,
        terrain_normal_atlas,
        height_normal,
        terrain_color_tint,
        post_diffuse: None,
        lod: terrain_mipmap_lod_for_pixel_step(map_w, map_h, map_pow2_x, map_pow2_y),
    };
    let mut out = vec![0u32; map_w * map_h];
    for y in 0..map_h {
        for x in 0..map_w {
            let uv_x = (x as f32 + 0.5) / map_w as f32;
            let uv_y = (y as f32 + 0.5) / map_h as f32;
            let uv2_x = uv_x * map_pow2_x;
            let uv2_y = (1.0 - uv_y) * map_pow2_y;
            out[y * map_w + x] = shade_terrain_pixel_with_input(
                &resources,
                TerrainPixelShaderInput {
                    uv: Vec2 { x: uv_x, y: uv_y },
                    uv2: Vec2 { x: uv2_x, y: uv2_y },
                    prepos: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    v_shadow_proj: Vec4 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 0.0,
                    },
                    v_screen_coord: Vec4 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 0.0,
                    },
                },
                TerrainPixelShaderDerivatives {
                    prepos_dx: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    prepos_dy: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
            )?;
        }
    }
    Ok(out)
}

pub(crate) struct TerrainPixelResources<'a> {
    pub(crate) map_w: usize,
    pub(crate) map_h: usize,
    pub(crate) terrain_id_map: &'a RgbaImage,
    pub(crate) terrain_atlas: &'a RgbaMipChain,
    pub(crate) terrain_normal_atlas: &'a RgbaMipChain,
    pub(crate) height_normal: &'a RgbxImage,
    pub(crate) terrain_color_tint: &'a RgbaMipChain,
    pub(crate) post_diffuse: Option<&'a TerrainPostDiffuseResources>,
    pub(crate) lod: f32,
}

pub(crate) fn shade_terrain_pixel_with_input(
    resources: &TerrainPixelResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
) -> Result<u32> {
    let mip_texels = 2.0f32.powf(ATLAS_TEXEL_POW2_EXPONENT - resources.lod);
    let id = sample_terrain_id_point_clamp(
        resources.terrain_id_map,
        input.uv.x - 0.5 / resources.map_w as f32,
        input.uv.y - 0.5 / resources.map_h as f32,
    );
    let blend = calculate_map_tex_index_bytes(id);
    let mut tile_repeat_x = input.uv2.x * TERRAIN_TILE_FREQ;
    let tile_repeat_y = input.uv2.y * TERRAIN_TILE_FREQ;
    tile_repeat_x *= resources.map_w as f32 / resources.map_h as f32;
    let frac = terrain_blend_frac(input.uv.x, input.uv.y, resources.map_w, resources.map_h);
    let material = sample_terrain_material(
        resources.terrain_atlas,
        resources.terrain_normal_atlas,
        resources.height_normal,
        &blend,
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        resources.lod,
        frac,
        input.uv2.x,
        input.uv2.y,
    );
    let terrain_color_sample = sample_uv_wrap_linear_lod_linear_rgba(
        resources.terrain_color_tint,
        input.uv2.x,
        input.uv2.y,
        0.0,
    );
    let terrain_color_vec = terrain_color_sample.rgb;
    let mut surface = TerrainSurfaceSample {
        diffuse: get_overlay_vec3(
            material.diffuse,
            terrain_color_vec,
            COLORMAP_OVERLAY_STRENGTH,
        ),
        normal: material.normal,
        glossiness: material.glossiness,
        specular: material.specular,
        snow_alpha: 1.0 - material.specular,
    };
    if let Some(post_diffuse) = resources.post_diffuse {
        let post_diffuse_sample = sample_post_diffuse_runtime(
            post_diffuse,
            input,
            derivatives,
            surface,
            terrain_color_vec,
        )?;
        let gradient_sample =
            apply_terrain_gradient_borders(post_diffuse, input, post_diffuse_sample.surface)?;
        let secondary_mask_sample = apply_terrain_secondary_mask(
            post_diffuse,
            input,
            gradient_sample.bloom_alpha,
            gradient_sample.surface,
        )?;
        surface = secondary_mask_sample.surface;
        let lit = shade_lit_terrain_pre_border(
            post_diffuse,
            input,
            surface,
            post_diffuse_sample.shadow_term,
        )?;
        let border_lit =
            apply_border_light_removal(lit, surface.diffuse, secondary_mask_sample.bloom_alpha);
        let out = apply_terrain_final_shader_tail(
            post_diffuse,
            input,
            derivatives,
            border_lit,
            secondary_mask_sample.bloom_alpha,
            terrain_color_sample.alpha,
        )?;
        if !vec3_is_finite(out) {
            return Err(
                "production terrain renderer blocked: pdxmap terrain final fog/day-night tail produced non-finite color"
                    .into(),
            );
        }
        return Ok(vec3_to_rgb_u32(out));
    }
    let _lighting_channels_ready_for_next_shader_stage =
        (surface.normal, surface.glossiness, surface.specular);
    Ok(vec3_to_rgb_u32(surface.diffuse))
}

pub(crate) fn sample_post_diffuse_runtime(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
    base_surface: TerrainSurfaceSample,
    terrain_color: Vec3,
) -> Result<TerrainPostDiffuseRuntimeSample> {
    validate_finite_terrain_pixel_shader_input(input)?;
    validate_finite_terrain_pixel_derivatives(derivatives)?;
    let mud_snow = sample_mud_snow_color(post_diffuse, input.prepos);
    let surface = apply_snow_and_mud(
        post_diffuse,
        input,
        derivatives,
        base_surface,
        terrain_color,
        mud_snow,
    );
    let point_light_indices = sample_point_light_indices(
        &post_diffuse.light_index_map,
        post_diffuse.point_light_cb2,
        input.prepos,
    );
    let point_light_count = point_light_indices
        .iter()
        .position(|index| *index >= 255)
        .unwrap_or(point_light_indices.len());
    for index in point_light_indices.iter().take(point_light_count) {
        let _light_data = sample_point_light_data_pair(&post_diffuse.light_data_map, *index);
    }
    let shadow_term = match post_diffuse.shadow_map {
        TerrainShadowMapState::NullSrv => get_shadow_scaled(
            SHADOW_WEIGHT_TERRAIN,
            0.0,
            post_diffuse.scene_cb0.shadow_fade_factor,
        ),
    };
    if !mud_snow.iter().all(|value| value.is_finite())
        || !shadow_term.is_finite()
        || !terrain_surface_sample_is_finite(surface)
    {
        return Err(
            "production terrain renderer blocked: post-diffuse terrain runtime sampling produced non-finite values"
                .into(),
        );
    }
    Ok(TerrainPostDiffuseRuntimeSample {
        mud_snow,
        point_light_indices,
        point_light_count,
        shadow_term,
        surface,
    })
}

pub(crate) fn apply_terrain_gradient_borders(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    mut surface: TerrainSurfaceSample,
) -> Result<TerrainSecondaryMaskSample> {
    if post_diffuse.scene_cb0.gb_texture_height <= 0.0
        || !post_diffuse.scene_cb0.gb_texture_height.is_finite()
        || post_diffuse.constants.map_size_x <= 0.0
        || post_diffuse.constants.map_size_y <= 0.0
        || !post_diffuse.constants.map_size_x.is_finite()
        || !post_diffuse.constants.map_size_y.is_finite()
    {
        return Err(
            "production terrain renderer blocked: GB_TextureHeight and MAP_SIZE_X/Y must be finite and positive for active gradient-border shader"
                .into(),
        );
    }
    let mut uv = input.uv2;
    let gb_cam_dist = gradient_border_camera_distance(post_diffuse.scene_cb0.v_cam_pos.y);
    let overrides = post_diffuse
        .scene_cb0
        .v_gb_cam_dist_override_gb_outline_cutoff;
    let gb_cam_dist_ch1 =
        saturate(gb_cam_dist * hlsl_int_to_float(1.0 - overrides[0]) + overrides[0]);
    let gb_cam_dist_ch2 =
        saturate(gb_cam_dist * hlsl_int_to_float(1.0 - overrides[1]) + overrides[1]);
    let half_pix = 0.5 / post_diffuse.scene_cb0.gb_texture_height;
    uv.y *= 0.5 - half_pix;
    let uv2 = Vec2 {
        x: uv.x,
        y: uv.y + 0.5,
    };
    let opacity = GB_OPACITY_NEAR + (1.0 - gb_cam_dist) * (GB_OPACITY_FAR - GB_OPACITY_NEAR);
    let (grad_mix, alpha1) = gradient_border_process_channel(
        post_diffuse,
        surface.diffuse,
        gb_cam_dist_ch1,
        uv,
        1.0,
        overrides[2],
        GB_STRENGTH_CH1,
    );
    let transp_a = sample_gradient_border_channel2(post_diffuse, uv).rgb.y;
    surface.diffuse = lerp_vec3(surface.diffuse, grad_mix, opacity * transp_a);

    let (grad_mix, alpha2) = gradient_border_process_channel(
        post_diffuse,
        surface.diffuse,
        gb_cam_dist_ch2,
        uv2,
        1.0,
        overrides[3],
        (1.0 - alpha1 * GB_STRENGTH_CH1 * GB_FIRST_LAYER_PRIORITY) * GB_STRENGTH_CH2,
    );
    let transp_b = sample_gradient_border_channel2(post_diffuse, uv2).rgb.y;
    surface.diffuse = lerp_vec3(surface.diffuse, grad_mix, opacity * transp_b);
    let bloom_alpha = 1.0 - alpha1.max(alpha2);
    if !bloom_alpha.is_finite() || !terrain_surface_sample_is_finite(surface) {
        return Err(
            "production terrain renderer blocked: gradient border shader path produced non-finite terrain color"
                .into(),
        );
    }
    Ok(TerrainSecondaryMaskSample {
        surface,
        bloom_alpha,
    })
}

pub(crate) fn gradient_border_process_channel(
    post_diffuse: &TerrainPostDiffuseResources,
    initial_color: Vec3,
    cam_dist: f32,
    uv: Vec2,
    outline_mult: f32,
    outline_cutoff: f32,
    mut strength: f32,
) -> (Vec3, f32) {
    let fx_alpha = sample_gradient_border_channel2(post_diffuse, uv);
    let pulse = ((post_diffuse.scene_cb0.v_global_time * 3.5).sin() + 1.0) * 0.5;
    strength *= lerp_f32(lerp_f32(0.45, 1.0, 1.0 - fx_alpha.rgb.z), 1.0, pulse);
    let full_width = 5.25 / 255.0;
    let gradient_width = 0.5 / 255.0;
    let gb_dist = gradient_border_multisample_alpha(post_diffuse, uv);
    let alpha = gb_dist.alpha;
    let mut color_opacity = levels_f32(alpha, 0.0, outline_cutoff);
    let mut outline = 1.0 - levels_f32(alpha, outline_cutoff, 1.0);
    let old_outline = outline;
    outline *= color_opacity.floor();
    outline *= outline_mult;
    color_opacity = gradient_border_distance_to_alpha(color_opacity, cam_dist);
    color_opacity *= old_outline.floor();
    let mut thick = smoothstep(
        0.0,
        1.0,
        levels_f32(
            alpha,
            outline_cutoff - full_width,
            outline_cutoff - full_width + gradient_width,
        ),
    );
    thick *= old_outline.floor();
    let max_gradient = color_opacity.max(outline);
    let mut channel = lerp_vec3(
        initial_color,
        gb_dist.rgb,
        max_gradient.max(thick) * strength,
    );
    channel = lerp_vec3(channel, scale_vec3(channel, 0.5), thick);
    (channel, max_gradient.max(thick) * fx_alpha.rgb.y)
}

pub(crate) fn gradient_border_multisample_alpha(
    post_diffuse: &TerrainPostDiffuseResources,
    uv: Vec2,
) -> SampledLinearRgba {
    let offset_x = -0.5 / post_diffuse.constants.map_size_x;
    let offset_y = -0.5 / post_diffuse.constants.map_size_y;
    let offsets = [
        Vec2 { x: 0.0, y: 0.0 },
        Vec2 {
            x: -offset_x,
            y: 0.0,
        },
        Vec2 {
            x: 0.0,
            y: -offset_y,
        },
        Vec2 {
            x: offset_x,
            y: 0.0,
        },
        Vec2 {
            x: 0.0,
            y: offset_y,
        },
        Vec2 {
            x: -offset_x,
            y: -offset_y,
        },
        Vec2 {
            x: offset_x,
            y: -offset_y,
        },
        Vec2 {
            x: offset_x,
            y: offset_y,
        },
        Vec2 {
            x: -offset_x,
            y: offset_y,
        },
    ];
    let mut out = SampledLinearRgba {
        rgb: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        alpha: 0.0,
    };
    for offset in offsets {
        let sample = sample_gradient_border_channel1(
            post_diffuse,
            Vec2 {
                x: uv.x + offset.x,
                y: uv.y + offset.y,
            },
        );
        out.rgb = add_vec3(out.rgb, sample.rgb);
        out.alpha += sample.alpha;
    }
    SampledLinearRgba {
        rgb: scale_vec3(out.rgb, 1.0 / 9.0),
        alpha: out.alpha / 9.0,
    }
}

pub(crate) fn sample_gradient_border_channel1(
    post_diffuse: &TerrainPostDiffuseResources,
    uv: Vec2,
) -> SampledLinearRgba {
    sample_uv_clamp_linear_lod_linear_rgba(&post_diffuse.gradient_border_channel1, uv.x, uv.y, 0.0)
}

pub(crate) fn sample_gradient_border_channel2(
    post_diffuse: &TerrainPostDiffuseResources,
    uv: Vec2,
) -> SampledLinearRgba {
    sample_uv_clamp_linear_lod_linear_rgba(&post_diffuse.gradient_border_channel2, uv.x, uv.y, 0.0)
}

pub(crate) fn gradient_border_camera_distance(cam_y: f32) -> f32 {
    1.0 - cam_distance(cam_y, GB_CAM_MIN, GB_CAM_MAX).clamp(0.0, GB_CAM_MAX_FILLING_CLAMP)
}

pub(crate) fn gradient_border_distance_to_alpha(distance: f32, cam_dist: f32) -> f32 {
    let distance = (1.0 - distance) * ((cam_dist - GB_THRESHOLD) / GB_THRESHOLD2);
    1.0 - saturate(distance)
}

pub(crate) fn levels_f32(value: f32, min: f32, max: f32) -> f32 {
    saturate((value - min) / (max - min))
}

pub(crate) fn hlsl_int_to_float(value: f32) -> f32 {
    (value as i32) as f32
}

pub(crate) fn apply_terrain_secondary_mask(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    mut bloom_alpha: f32,
    mut surface: TerrainSurfaceSample,
) -> Result<TerrainSecondaryMaskSample> {
    let color_mask = sample_uv_wrap_linear_lod_linear_rgba(
        &post_diffuse.province_secondary_color_map,
        input.uv2.x,
        input.uv2.y,
        0.0,
    );
    let occupation_mask =
        calculate_occupation_mask(input.uv2, post_diffuse.scene_cb0.v_cam_pos.y) * color_mask.alpha;
    bloom_alpha *= 1.0 - occupation_mask;
    surface.diffuse = lerp_vec3(surface.diffuse, color_mask.rgb, occupation_mask);
    if !bloom_alpha.is_finite() || !terrain_surface_sample_is_finite(surface) {
        return Err(
            "production terrain renderer blocked: secondary color mask produced non-finite terrain color"
                .into(),
        );
    }
    Ok(TerrainSecondaryMaskSample {
        surface,
        bloom_alpha,
    })
}

pub(crate) fn calculate_occupation_mask(uv: Vec2, cam_y: f32) -> f32 {
    // Port of gfx/FX/standardfuncsgfx.fxh:965-976 CalculateOccupationMask.
    let t: f32 = 3.14159 / 8.0;
    let mut stripe_val = (uv.x * t.cos() * SEC_MAP_TILE + uv.y * t.sin() * SEC_MAP_TILE).cos();
    let cam_dist = cam_distance(cam_y, SEC_CAM_MIN, SEC_CAM_MAX);
    stripe_val += cam_dist * 1.5;
    smoothstep(0.0, 1.0, stripe_val * 1.7) * lerp_f32(1.0, 0.3, cam_dist)
}

pub(crate) fn apply_border_light_removal(lit: Vec3, diffuse: Vec3, bloom_alpha: f32) -> Vec3 {
    lerp_vec3(
        lit,
        diffuse,
        BORDER_LIGHT_REMOVAL_FACTOR * (1.0 - bloom_alpha),
    )
}

pub(crate) fn apply_terrain_final_shader_tail(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
    mut color: Vec3,
    bloom_alpha: f32,
    city_lights_mask: f32,
) -> Result<Vec3> {
    let globe_normal =
        calc_globe_normal(post_diffuse.scene_cb0, post_diffuse.constants, input.prepos);
    let night_factor = day_night_factor(
        post_diffuse.scene_cb0,
        globe_normal,
        FEATHER_MIN,
        FEATHER_MAX,
    );
    let city_lights = sample_world_xz_mip_rgba(
        &post_diffuse.city_lights_snow_noise,
        input.prepos,
        derivatives,
        CITY_LIGHTS_TILING,
    );
    color = add_vec3(
        color,
        scale_vec3(
            city_lights.rgb,
            CITY_LIGHTS_INTENSITY * city_lights_mask * night_factor,
        ),
    );
    let fow_color = apply_shadow_map_fow(post_diffuse, color);
    color = lerp_vec3(
        fow_color,
        color,
        BORDER_FOW_REMOVAL_FACTOR * (1.0 - bloom_alpha),
    );
    color = apply_distance_fog(post_diffuse.scene_cb0, color, input.prepos);
    color = day_night_with_blend(
        post_diffuse.scene_cb0,
        color,
        globe_normal,
        lerp_f32(BORDER_NIGHT_DESATURATION_MAX, 1.0, bloom_alpha),
    );
    if !vec3_is_finite(color) {
        return Err(
            "production terrain renderer blocked: pdxmap terrain city/FOW/fog/day-night tail produced non-finite color"
                .into(),
        );
    }
    Ok(color)
}

pub(crate) fn apply_shadow_map_fow(
    post_diffuse: &TerrainPostDiffuseResources,
    color: Vec3,
) -> Vec3 {
    match post_diffuse.shadow_map {
        TerrainShadowMapState::NullSrv => apply_fow(color, 0.0, 0.0),
    }
}

pub(crate) fn apply_fow(color: Vec3, fog_color_factor: f32, fog_alpha_factor: f32) -> Vec3 {
    let fog_color = lerp_vec3(DARK_FOW_COLOR, BRIGHT_FOW_COLOR, fog_color_factor);
    lerp_vec3(color, fog_color, fog_alpha_factor)
}

pub(crate) fn apply_distance_fog(scene: TerrainSceneCb0Decoded, color: Vec3, pos: Vec3) -> Vec3 {
    lerp_vec3(color, FOG_COLOR, calculate_distance_fog_factor(scene, pos))
}

pub(crate) fn calculate_distance_fog_factor(scene: TerrainSceneCb0Decoded, pos: Vec3) -> f32 {
    let diff = sub_vec3(scene.v_cam_pos, pos);
    let fog_factor = 1.0 - normalize_vec3(diff).y.abs();
    let sq_distance = dot_vec3(diff, diff);
    let begin = FOG_BEGIN * FOG_BEGIN;
    let end = FOG_END * FOG_END;
    let min_factor = ((sq_distance - begin) / (end - begin)).min(FOG_MAX);
    saturate(min_factor) * fog_factor
}

pub(crate) fn day_night_with_blend(
    scene: TerrainSceneCb0Decoded,
    day_color: Vec3,
    globe_normal: Vec3,
    blend: f32,
) -> Vec3 {
    lerp_vec3(
        day_color,
        nightify_color(day_color, blend),
        day_night_factor(scene, globe_normal, FEATHER_MIN, FEATHER_MAX) * NIGHT_OPACITY,
    )
}

pub(crate) fn nightify_color(day_color: Vec3, blend: f32) -> Vec3 {
    let desaturation = 0.8 * blend * blend * blend;
    let grey = dot_vec3(
        day_color,
        Vec3 {
            x: 0.4,
            y: 0.3,
            z: 0.05,
        },
    );
    let grey_vec = Vec3 {
        x: grey,
        y: grey,
        z: grey,
    };
    let cool_grey = mul_vec3(
        grey_vec,
        Vec3 {
            x: 0.2,
            y: 0.7,
            z: 1.2,
        },
    );
    let night_color = saturate_vec3(lerp_vec3(grey_vec, cool_grey, 0.25));
    scale_vec3(
        lerp_vec3(day_color, night_color, desaturation),
        NIGHT_DARKNESS,
    )
}

pub(crate) fn shade_lit_terrain_pre_border(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    surface: TerrainSurfaceSample,
    shadow_term: f32,
) -> Result<Vec3> {
    let properties = terrain_lighting_properties(post_diffuse.scene_cb0, input.prepos, surface);
    let light_dir =
        calculate_sun_direction(post_diffuse.scene_cb0, post_diffuse.constants, input.prepos);
    let (mut diffuse_light, mut specular_light) = calculate_sun_light(
        post_diffuse.scene_cb0,
        post_diffuse.constants,
        properties,
        shadow_term,
        light_dir,
    );
    calculate_point_lights(
        post_diffuse,
        properties,
        &mut diffuse_light,
        &mut specular_light,
    );
    let out = compose_light_snow(
        post_diffuse.scene_cb0,
        post_diffuse.constants,
        properties,
        diffuse_light,
        specular_light,
        surface.snow_alpha,
    );
    if !vec3_is_finite(out) {
        return Err(
            "production terrain renderer blocked: pdxmap terrain lighting produced non-finite color"
                .into(),
        );
    }
    Ok(out)
}

pub(crate) fn terrain_lighting_properties(
    scene: TerrainSceneCb0Decoded,
    world_pos: Vec3,
    surface: TerrainSurfaceSample,
) -> TerrainLightingProperties {
    let spec_remapped = surface.specular * surface.specular * 0.4;
    TerrainLightingProperties {
        world_space_pos: world_pos,
        to_camera_dir: normalize_vec3(sub_vec3(scene.v_cam_pos, world_pos)),
        normal: surface.normal,
        diffuse: surface.diffuse,
        specular_color: Vec3 {
            x: spec_remapped,
            y: spec_remapped,
            z: spec_remapped,
        },
        glossiness: surface.glossiness,
        non_linear_glossiness: get_non_linear_glossiness(surface.glossiness),
    }
}

pub(crate) fn calculate_sun_light(
    scene: TerrainSceneCb0Decoded,
    constants: MapFrameTerrainShaderConstants,
    properties: TerrainLightingProperties,
    shadow_term: f32,
    light_source_direction: Vec3,
) -> (Vec3, Vec3) {
    let globe = calc_globe_normal(scene, constants, properties.world_space_pos);
    let day_factor = 1.0 - day_night_factor(scene, globe, FEATHER_MIN, FEATHER_MAX);
    let night_factor = day_night_factor(scene, globe, MOON_FEATHER_MIN, MOON_FEATHER_MAX);
    let shadow_term = shadow_term * saturate(day_factor + night_factor);
    let sun_intensity = add_vec3(
        scale_vec3(
            Vec3 {
                x: scene.sun_diffuse_intensity[0],
                y: scene.sun_diffuse_intensity[1],
                z: scene.sun_diffuse_intensity[2],
            },
            scene.sun_diffuse_intensity[3] * shadow_term * day_factor,
        ),
        scale_vec3(
            Vec3 {
                x: scene.moon_diffuse_intensity[0],
                y: scene.moon_diffuse_intensity[1],
                z: scene.moon_diffuse_intensity[2],
            },
            scene.moon_diffuse_intensity[3] * shadow_term * night_factor,
        ),
    );
    let (diffuse, specular) = improved_blinn_phong(
        sun_intensity,
        scale_vec3(light_source_direction, -1.0),
        properties,
    );
    (diffuse, scale_vec3(specular, scene.sun_specular_intensity))
}

pub(crate) fn calculate_point_lights(
    post_diffuse: &TerrainPostDiffuseResources,
    properties: TerrainLightingProperties,
    diffuse_light: &mut Vec3,
    specular_light: &mut Vec3,
) {
    let indices = sample_point_light_indices(
        &post_diffuse.light_index_map,
        post_diffuse.point_light_cb2,
        properties.world_space_pos,
    );
    for index in indices {
        if index >= 255 {
            break;
        }
        let (position_radius, color_falloff) =
            sample_point_light_data_pair(&post_diffuse.light_data_map, index);
        calculate_point_light(
            TerrainPointLight {
                position: Vec3 {
                    x: position_radius[0],
                    y: position_radius[1],
                    z: position_radius[2],
                },
                radius: position_radius[3],
                color: Vec3 {
                    x: color_falloff[0],
                    y: color_falloff[1],
                    z: color_falloff[2],
                },
                falloff: color_falloff[3],
            },
            properties,
            diffuse_light,
            specular_light,
        );
    }
}

pub(crate) fn calculate_point_light(
    light: TerrainPointLight,
    properties: TerrainLightingProperties,
    diffuse_light: &mut Vec3,
    specular_light: &mut Vec3,
) {
    let pos_to_light = sub_vec3(light.position, properties.world_space_pos);
    let light_distance = length_vec3(pos_to_light);
    let light_intensity = saturate((light.radius - light_distance) / light.falloff);
    if light_intensity > 0.0 && light_distance > 0.0 {
        let to_light_dir = scale_vec3(pos_to_light, 1.0 / light_distance);
        let (diffuse, specular) = improved_blinn_phong(
            scale_vec3(light.color, light_intensity),
            to_light_dir,
            properties,
        );
        *diffuse_light = add_vec3(*diffuse_light, diffuse);
        *specular_light = add_vec3(*specular_light, specular);
    }
}

pub(crate) fn improved_blinn_phong(
    light_color: Vec3,
    to_light_dir: Vec3,
    properties: TerrainLightingProperties,
) -> (Vec3, Vec3) {
    let h = normalize_vec3(add_vec3(properties.to_camera_dir, to_light_dir));
    let ndot_l = saturate(dot_vec3(properties.normal, to_light_dir));
    let ndot_h = saturate(dot_vec3(properties.normal, h));
    let normalization = (properties.non_linear_glossiness + 2.0) / 8.0;
    let fresnel = fresnel_schlick(properties.specular_color, to_light_dir, h);
    let spec_color = scale_vec3(
        fresnel,
        normalization * ndot_h.powf(properties.non_linear_glossiness),
    );
    (
        scale_vec3(light_color, ndot_l),
        mul_vec3(
            mul_vec3(spec_color, light_color),
            Vec3 {
                x: ndot_l,
                y: ndot_l,
                z: ndot_l,
            },
        ),
    )
}

pub(crate) fn compose_light_snow(
    scene: TerrainSceneCb0Decoded,
    constants: MapFrameTerrainShaderConstants,
    properties: TerrainLightingProperties,
    diffuse_light: Vec3,
    specular_light: Vec3,
    snow_factor: f32,
) -> Vec3 {
    let day_night = day_night_factor(
        scene,
        calc_globe_normal(scene, constants, properties.world_space_pos),
        FEATHER_MIN,
        FEATHER_MAX,
    );
    let ambient = ambient_light(properties.normal, day_night, scene.neg_fog_multiplier);
    let snow_ambient = calc_snow_ambient(snow_factor);
    add_vec3(
        scale_vec3(
            mul_vec3(
                add_vec3(add_vec3(snow_ambient, ambient), diffuse_light),
                properties.diffuse,
            ),
            scene.hdr_range,
        ),
        specular_light,
    )
}

pub(crate) fn calc_snow_ambient(snow_factor: f32) -> Vec3 {
    scale_vec3(
        Vec3 {
            x: 0.2,
            y: 0.7,
            z: 1.0,
        },
        0.07 * smoothstep(0.0, 0.1, snow_factor),
    )
}

pub(crate) fn ambient_light(world_normal: Vec3, day_factor: f32, neg_fog_multiplier: f32) -> Vec3 {
    let adjusted = lerp_vec3(
        world_normal,
        normalize_vec3(sub_vec3(
            world_normal,
            scale_vec3(
                Vec3 {
                    x: 0.0,
                    y: 0.9,
                    z: 0.0,
                },
                smoothstep(
                    -0.6,
                    0.5,
                    dot_vec3(
                        world_normal,
                        Vec3 {
                            x: 0.0,
                            y: -1.0,
                            z: 0.0,
                        },
                    ),
                ),
            ),
        )),
        neg_fog_multiplier,
    );
    let squared = mul_vec3(adjusted, adjusted);
    let day = [
        DAY_AMBIENT_MAP_POS_X,
        DAY_AMBIENT_MAP_NEG_X,
        DAY_AMBIENT_MAP_POS_Y,
        DAY_AMBIENT_MAP_NEG_Y,
        DAY_AMBIENT_MAP_POS_Z,
        DAY_AMBIENT_MAP_NEG_Z,
    ];
    let night = [
        NIGHT_AMBIENT_MAP_POS_X,
        NIGHT_AMBIENT_MAP_NEG_X,
        NIGHT_AMBIENT_MAP_POS_Y,
        NIGHT_AMBIENT_MAP_NEG_Y,
        NIGHT_AMBIENT_MAP_POS_Z,
        NIGHT_AMBIENT_MAP_NEG_Z,
    ];
    add_vec3(
        add_vec3(
            scale_vec3(
                lerp_vec3(
                    day[(adjusted.x < 0.0) as usize],
                    saturate_vec3(scale_vec3(
                        night[(adjusted.x < 0.0) as usize],
                        NIGHT_AMBIENT_BOOST,
                    )),
                    day_factor,
                ),
                squared.x,
            ),
            scale_vec3(
                lerp_vec3(
                    day[(adjusted.y < 0.0) as usize + 2],
                    saturate_vec3(scale_vec3(
                        night[(adjusted.y < 0.0) as usize + 2],
                        NIGHT_AMBIENT_BOOST,
                    )),
                    day_factor,
                ),
                squared.y,
            ),
        ),
        scale_vec3(
            lerp_vec3(
                day[(adjusted.z < 0.0) as usize + 4],
                saturate_vec3(scale_vec3(
                    night[(adjusted.z < 0.0) as usize + 4],
                    NIGHT_AMBIENT_BOOST,
                )),
                day_factor,
            ),
            squared.z,
        ),
    )
}

pub(crate) fn calculate_sun_direction(
    scene: TerrainSceneCb0Decoded,
    constants: MapFrameTerrainShaderConstants,
    world_pos: Vec3,
) -> Vec3 {
    let selected = day_night_factor(
        scene,
        calc_globe_normal(scene, constants, world_pos),
        0.0,
        0.0001,
    );
    let mut source_pos = lerp_vec3(scene.v_virtual_sun_pos, scene.v_virtual_moon_pos, selected);
    let mut second_source_pos = lerp_vec3(
        scene.v_second_virtual_sun_pos,
        scene.v_second_virtual_moon_pos,
        selected,
    );
    let half_map = constants.map_size_x * 0.5;
    if world_pos.x - source_pos.x > half_map {
        source_pos.x += constants.map_size_x;
    } else if world_pos.x - source_pos.x < -half_map {
        source_pos.x -= constants.map_size_x;
    }
    if world_pos.x - second_source_pos.x > half_map {
        second_source_pos.x += constants.map_size_x;
    } else if world_pos.x - second_source_pos.x < -half_map {
        second_source_pos.x -= constants.map_size_x;
    }
    let lerp_factor = smoothstep(0.5, 1.0, (world_pos.x - source_pos.x).abs() / half_map);
    source_pos = lerp_vec3(source_pos, second_source_pos, lerp_factor);
    normalize_vec3(sub_vec3(world_pos, source_pos))
}

pub(crate) fn calc_globe_normal(
    scene: TerrainSceneCb0Decoded,
    constants: MapFrameTerrainShaderConstants,
    world_pos: Vec3,
) -> Vec3 {
    let x = ((world_pos.x - GMT_OFFSET) / constants.map_size_x + scene.day_night_hour_sun_dir[0])
        .rem_euclid(1.0);
    let y = SOUTH_POLE_OFFSET
        + (NORTH_POLE_OFFSET - SOUTH_POLE_OFFSET) * (world_pos.z / constants.map_size_y);
    let y = -(y * 3.1415).cos();
    let xz_len = 1.0 - y.abs();
    normalize_vec3(Vec3 {
        x: (x * 6.2831).sin() * xz_len,
        y,
        z: (x * 6.2831).cos() * xz_len,
    })
}

pub(crate) fn day_night_factor(
    scene: TerrainSceneCb0Decoded,
    globe_normal: Vec3,
    min: f32,
    max: f32,
) -> f32 {
    let sun_dir = Vec3 {
        x: scene.day_night_hour_sun_dir[1],
        y: scene.day_night_hour_sun_dir[2],
        z: scene.day_night_hour_sun_dir[3],
    };
    saturate((dot_vec3(globe_normal, sun_dir) - min) / (max - min))
        * scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3]
}

pub(crate) fn fresnel_schlick(specular_color: Vec3, e: Vec3, h: Vec3) -> Vec3 {
    add_vec3(
        specular_color,
        scale_vec3(
            sub_vec3(
                Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                specular_color,
            ),
            (1.0 - saturate(dot_vec3(e, h))).powf(5.0),
        ),
    )
}

pub(crate) fn get_non_linear_glossiness(glossiness: f32) -> f32 {
    (11.0 * glossiness).exp2()
}

pub(crate) fn validate_finite_terrain_pixel_shader_input(
    input: TerrainPixelShaderInput,
) -> Result<()> {
    if [
        input.uv.x,
        input.uv.y,
        input.uv2.x,
        input.uv2.y,
        input.prepos.x,
        input.prepos.y,
        input.prepos.z,
        input.v_shadow_proj.x,
        input.v_shadow_proj.y,
        input.v_shadow_proj.z,
        input.v_shadow_proj.w,
        input.v_screen_coord.x,
        input.v_screen_coord.y,
        input.v_screen_coord.z,
        input.v_screen_coord.w,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return Err(
            "production terrain renderer blocked: post-diffuse terrain pixel input contains non-finite VS output"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_finite_terrain_pixel_derivatives(
    derivatives: TerrainPixelShaderDerivatives,
) -> Result<()> {
    if [
        derivatives.prepos_dx.x,
        derivatives.prepos_dx.y,
        derivatives.prepos_dx.z,
        derivatives.prepos_dy.x,
        derivatives.prepos_dy.y,
        derivatives.prepos_dy.z,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return Err(
            "production terrain renderer blocked: post-diffuse terrain pixel derivatives contain non-finite values"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn sample_mud_snow_color(
    post_diffuse: &TerrainPostDiffuseResources,
    world_pos: Vec3,
) -> [f32; 4] {
    sample_uv_wrap_linear_rgba_vec4(
        &post_diffuse.snow_mud_data,
        ((world_pos.x + 0.5) / post_diffuse.constants.map_size_x)
            * post_diffuse.constants.fow_pow2_x
            * post_diffuse.constants.fow_pow2_y,
        ((world_pos.z + 0.5) / post_diffuse.constants.map_size_y)
            * post_diffuse.constants.fow_pow2_y,
    )
}

pub(crate) fn apply_snow_and_mud(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
    base_surface: TerrainSurfaceSample,
    terrain_color: Vec3,
    mud_snow: [f32; 4],
) -> TerrainSurfaceSample {
    let snowed = apply_snow(post_diffuse, input, derivatives, base_surface, mud_snow);
    apply_mud(
        post_diffuse,
        input,
        derivatives,
        snowed,
        terrain_color,
        mud_snow,
    )
}

pub(crate) fn apply_snow(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
    mut surface: TerrainSurfaceSample,
    mud_snow: [f32; 4],
) -> TerrainSurfaceSample {
    let snow_fade = saturate(input.prepos.y - SNOW_START_HEIGHT);
    let normal_fade = saturate(saturate(surface.normal.y - SNOW_NORMAL_START) * SNOW_CLIFFS);
    let snow_texture = sample_world_xz_mip_rgba(
        &post_diffuse.snow_texture,
        input.prepos,
        derivatives,
        SNOW_TILING,
    );
    let noise = sample_world_xz_mip_rgba(
        &post_diffuse.city_lights_snow_noise,
        input.prepos,
        derivatives,
        SNOW_NOISE_TILING,
    )
    .alpha;
    let is_snow = get_snow(mud_snow, post_diffuse.scene_cb0);

    let mut transp = noise;
    transp += saturate(input.prepos.y - SNOW_RIDGE_START_HEIGHT)
        * (saturate((surface.normal.y - 0.9) * 1000.0) * is_snow);
    transp = saturate(transp);

    let snow = saturate(saturate(transp - (1.0 - is_snow)) * 5.0);
    let frost = saturate(saturate(transp + 0.5) - (1.0 - is_snow));
    let mut opacity = cam_distance(
        post_diffuse.scene_cb0.v_cam_pos.y,
        SNOW_CAM_MIN,
        SNOW_CAM_MAX,
    );
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);
    let snow_alpha = saturate(
        saturate(snow + frost) * snow_fade * normal_fade * saturate(is_snow * 2.25) * opacity,
    );
    let min_snow = smoothstep(0.0, 1.0, is_snow);
    surface.diffuse = lerp_vec3(
        surface.diffuse,
        scale_vec3(SNOW_COLOR, snow_texture.alpha),
        surface.snow_alpha * saturate(snow_alpha + (SNOW_FROST_MIN_EFFECT * min_snow)),
    );

    let snow_normal = normalize_vec3(normal_from_vec3_rbg(snow_texture.rgb));
    let snow_normal = normalize_vec3(rotate_vector_by_vector(snow_normal, surface.normal));
    surface.normal = normalize_vec3(lerp_vec3(surface.normal, snow_normal, snow_alpha));
    surface.snow_alpha = snow_alpha;
    surface.glossiness += snow_texture.alpha * snow_alpha * SNOW_SPEC_GLOSS_MULT;
    surface
}

pub(crate) fn apply_mud(
    post_diffuse: &TerrainPostDiffuseResources,
    input: TerrainPixelShaderInput,
    derivatives: TerrainPixelShaderDerivatives,
    mut surface: TerrainSurfaceSample,
    terrain_color: Vec3,
    mud_snow: [f32; 4],
) -> TerrainSurfaceSample {
    let opacity = cam_distance(post_diffuse.scene_cb0.v_cam_pos.y, MUD_CAM_MIN, MUD_CAM_MAX);
    let snow_noise = sample_world_xz_mip_rgba(
        &post_diffuse.city_lights_snow_noise,
        input.prepos,
        derivatives,
        MUD_NOISE_TILING,
    )
    .alpha;
    let noise = lerp_f32(1.0, (0.5 + snow_noise) * 0.5, opacity);
    let fade = post_diffuse
        .scene_cb0
        .v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[2];
    let mut mud_current = lerp_f32(mud_snow[0], mud_snow[3], fade);
    mud_current *= 1.0
        - saturate(
            saturate(surface.normal.y - MUD_NORMAL_CUTOFF) * ((1.0 - MUD_NORMAL_CUTOFF) * 1000.0),
        );
    mud_current = saturate(mud_current * MUD_STRENGHTEN * noise);

    let mud_diffuse_gloss = sample_world_xz_mip_rgba(
        &post_diffuse.mud_diffuse_gloss,
        input.prepos,
        derivatives,
        MUD_TILING,
    );
    let mud_normal_spec = sample_world_xz_mip_rgba(
        &post_diffuse.mud_normal_spec,
        input.prepos,
        derivatives,
        MUD_TILING,
    );

    let mud_normal = normalize_vec3(normal_from_vec3_rbg(mud_normal_spec.rgb));
    let mud_normal = normalize_vec3(rotate_vector_by_vector(mud_normal, surface.normal));
    surface.normal = normalize_vec3(lerp_vec3(surface.normal, mud_normal, mud_current));
    surface.glossiness = lerp_f32(surface.glossiness, mud_diffuse_gloss.alpha, mud_current);
    surface.specular = lerp_f32(surface.specular, mud_normal_spec.alpha, mud_current);

    let mud_mix = get_overlay_vec3(
        mud_diffuse_gloss.rgb,
        terrain_color,
        COLORMAP_MUD_OVERLAY_STRENGTH,
    );
    surface.diffuse = lerp_vec3(surface.diffuse, mud_mix, mud_current);
    surface
}

pub(crate) fn sample_world_xz_mip_rgba(
    chain: &RgbaMipChain,
    world_pos: Vec3,
    derivatives: TerrainPixelShaderDerivatives,
    tiling: f32,
) -> SampledLinearRgba {
    sample_uv_wrap_linear_lod_linear_rgba(
        chain,
        world_pos.x * tiling,
        world_pos.z * tiling,
        texture_lod_for_world_xz(chain, derivatives, tiling),
    )
}

pub(crate) fn texture_lod_for_world_xz(
    chain: &RgbaMipChain,
    derivatives: TerrainPixelShaderDerivatives,
    tiling: f32,
) -> f32 {
    let Some(base) = chain.levels.first() else {
        return 0.0;
    };
    let du_dx = derivatives.prepos_dx.x * tiling;
    let dv_dx = derivatives.prepos_dx.z * tiling;
    let du_dy = derivatives.prepos_dy.x * tiling;
    let dv_dy = derivatives.prepos_dy.z * tiling;
    let width = base.width as f32;
    let height = base.height as f32;
    let rho_sq_x = (du_dx * width).powi(2) + (dv_dx * height).powi(2);
    let rho_sq_y = (du_dy * width).powi(2) + (dv_dy * height).powi(2);
    let rho_sq = rho_sq_x.max(rho_sq_y);
    if !rho_sq.is_finite() || rho_sq <= 1.0 {
        return 0.0;
    }
    0.5 * rho_sq.log2()
}

pub(crate) fn get_snow(mud_snow: [f32; 4], scene: TerrainSceneCb0Decoded) -> f32 {
    lerp_f32(
        mud_snow[2],
        mud_snow[1],
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[2],
    )
}

pub(crate) fn sample_point_light_indices(
    light_index_map: &RgbaImage,
    cb2: TerrainPointLightCb2Decoded,
    world_pos: Vec3,
) -> [u8; 4] {
    let light_index_u = ((world_pos.x - cb2.grid_start_inv_cell_size[0])
        * cb2.grid_start_inv_cell_size[2])
        / LIGHT_INDEX_TEXTURE_SIZE as f32;
    let light_index_v = ((world_pos.z - cb2.grid_start_inv_cell_size[1])
        * cb2.grid_start_inv_cell_size[3])
        / LIGHT_INDEX_TEXTURE_SIZE as f32;
    sample_rgba_point_clamp(light_index_map, light_index_u, light_index_v)
}

pub(crate) fn sample_point_light_data_pair(
    light_data_map: &Rgba32fImage,
    light_index: u8,
) -> ([f32; 4], [f32; 4]) {
    let base = usize::from(light_index) * 2;
    let second = base + 1;
    (
        sample_rgba32f_point_clamp(
            light_data_map,
            (base as f32 + 0.5) / LIGHT_DATA_TEXTURE_SIZE as f32,
            0.0,
        ),
        sample_rgba32f_point_clamp(
            light_data_map,
            (second as f32 + 0.5) / LIGHT_DATA_TEXTURE_SIZE as f32,
            0.0,
        ),
    )
}

pub(crate) fn get_shadow_scaled(f_scaler: f32, shadow_sample: f32, shadow_fade_factor: f32) -> f32 {
    let f_scaler = saturate(f_scaler);
    let shadow_value = saturate(shadow_sample + (1.0 - shadow_fade_factor));
    (1.0 - f_scaler) + f_scaler * shadow_value
}

pub(crate) fn sample_rgba_point_clamp(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let x = (u * image.width as f32)
        .floor()
        .clamp(0.0, image.width.saturating_sub(1) as f32) as usize;
    let y = (v * image.height as f32)
        .floor()
        .clamp(0.0, image.height.saturating_sub(1) as f32) as usize;
    let offset = (y * image.width + x) * 4;
    [
        image.pixels[offset],
        image.pixels[offset + 1],
        image.pixels[offset + 2],
        image.pixels[offset + 3],
    ]
}

pub(crate) fn sample_rgba32f_point_clamp(image: &Rgba32fImage, u: f32, v: f32) -> [f32; 4] {
    let x = (u * image.width as f32)
        .floor()
        .clamp(0.0, image.width.saturating_sub(1) as f32) as usize;
    let y = (v * image.height as f32)
        .floor()
        .clamp(0.0, image.height.saturating_sub(1) as f32) as usize;
    image.pixels[y * image.width + x]
}

pub(crate) fn sample_uv_wrap_linear_rgba_vec4(image: &RgbaImage, u: f32, v: f32) -> [f32; 4] {
    let sampled = sample_uv_wrap_linear_rgba(image, u, v);
    let (r, g, b) = rgb_tuple(sampled.rgb);
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        sampled.alpha,
    ]
}

pub(crate) fn saturate(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

pub(crate) fn rasterize_map_frame_terrain_triangle(
    out: &mut [u32],
    covered: &mut [bool],
    resources: &TerrainPixelResources,
    triangle: [TerrainVertexShaderOutput; 3],
) -> Result<()> {
    let p = [
        terrain_vertex_pixel_position(triangle[0], resources.map_w, resources.map_h),
        terrain_vertex_pixel_position(triangle[1], resources.map_w, resources.map_h),
        terrain_vertex_pixel_position(triangle[2], resources.map_w, resources.map_h),
    ];
    let area = edge_function(p[0], p[1], p[2]);
    if !area.is_finite() || area.abs() <= f32::EPSILON {
        return Ok(());
    }
    let derivatives = terrain_triangle_prepos_derivatives(p, triangle, area);
    let min_x = p
        .iter()
        .map(|p| p.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_x = p
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(resources.map_w.saturating_sub(1) as f32) as usize;
    let min_y = p
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_y = p
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(resources.map_h.saturating_sub(1) as f32) as usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let pixel = Vec2 {
                x: x as f32,
                y: y as f32,
            };
            let w0 = edge_function(p[1], p[2], pixel) / area;
            let w1 = edge_function(p[2], p[0], pixel) / area;
            let w2 = edge_function(p[0], p[1], pixel) / area;
            if w0 < -0.0001 || w1 < -0.0001 || w2 < -0.0001 {
                continue;
            }
            let uv_x = w0 * triangle[0].uv.x + w1 * triangle[1].uv.x + w2 * triangle[2].uv.x;
            let uv_y = w0 * triangle[0].uv.y + w1 * triangle[1].uv.y + w2 * triangle[2].uv.y;
            let uv2_x = w0 * triangle[0].uv2.x + w1 * triangle[1].uv2.x + w2 * triangle[2].uv2.x;
            let uv2_y = w0 * triangle[0].uv2.y + w1 * triangle[1].uv2.y + w2 * triangle[2].uv2.y;
            let prepos = Vec3 {
                x: w0 * triangle[0].prepos.x
                    + w1 * triangle[1].prepos.x
                    + w2 * triangle[2].prepos.x,
                y: w0 * triangle[0].prepos.y
                    + w1 * triangle[1].prepos.y
                    + w2 * triangle[2].prepos.y,
                z: w0 * triangle[0].prepos.z
                    + w1 * triangle[1].prepos.z
                    + w2 * triangle[2].prepos.z,
            };
            let v_shadow_proj = Vec4 {
                x: w0 * triangle[0].v_shadow_proj.x
                    + w1 * triangle[1].v_shadow_proj.x
                    + w2 * triangle[2].v_shadow_proj.x,
                y: w0 * triangle[0].v_shadow_proj.y
                    + w1 * triangle[1].v_shadow_proj.y
                    + w2 * triangle[2].v_shadow_proj.y,
                z: w0 * triangle[0].v_shadow_proj.z
                    + w1 * triangle[1].v_shadow_proj.z
                    + w2 * triangle[2].v_shadow_proj.z,
                w: w0 * triangle[0].v_shadow_proj.w
                    + w1 * triangle[1].v_shadow_proj.w
                    + w2 * triangle[2].v_shadow_proj.w,
            };
            let v_screen_coord = Vec4 {
                x: w0 * triangle[0].v_screen_coord.x
                    + w1 * triangle[1].v_screen_coord.x
                    + w2 * triangle[2].v_screen_coord.x,
                y: w0 * triangle[0].v_screen_coord.y
                    + w1 * triangle[1].v_screen_coord.y
                    + w2 * triangle[2].v_screen_coord.y,
                z: w0 * triangle[0].v_screen_coord.z
                    + w1 * triangle[1].v_screen_coord.z
                    + w2 * triangle[2].v_screen_coord.z,
                w: w0 * triangle[0].v_screen_coord.w
                    + w1 * triangle[1].v_screen_coord.w
                    + w2 * triangle[2].v_screen_coord.w,
            };
            let color = shade_terrain_pixel_with_input(
                resources,
                TerrainPixelShaderInput {
                    uv: Vec2 { x: uv_x, y: uv_y },
                    uv2: Vec2 { x: uv2_x, y: uv2_y },
                    prepos,
                    v_shadow_proj,
                    v_screen_coord,
                },
                derivatives,
            )?;
            let offset = y * resources.map_w + x;
            out[offset] = color;
            covered[offset] = true;
        }
    }
    Ok(())
}

pub(crate) fn terrain_triangle_prepos_derivatives(
    p: [Vec2; 3],
    triangle: [TerrainVertexShaderOutput; 3],
    area: f32,
) -> TerrainPixelShaderDerivatives {
    let dw0_dx = (p[2].y - p[1].y) / area;
    let dw0_dy = -(p[2].x - p[1].x) / area;
    let dw1_dx = (p[0].y - p[2].y) / area;
    let dw1_dy = -(p[0].x - p[2].x) / area;
    let dw2_dx = (p[1].y - p[0].y) / area;
    let dw2_dy = -(p[1].x - p[0].x) / area;
    TerrainPixelShaderDerivatives {
        prepos_dx: barycentric_derivative_vec3(
            triangle[0].prepos,
            triangle[1].prepos,
            triangle[2].prepos,
            dw0_dx,
            dw1_dx,
            dw2_dx,
        ),
        prepos_dy: barycentric_derivative_vec3(
            triangle[0].prepos,
            triangle[1].prepos,
            triangle[2].prepos,
            dw0_dy,
            dw1_dy,
            dw2_dy,
        ),
    }
}

pub(crate) fn barycentric_derivative_vec3(
    v0: Vec3,
    v1: Vec3,
    v2: Vec3,
    dw0: f32,
    dw1: f32,
    dw2: f32,
) -> Vec3 {
    Vec3 {
        x: v0.x * dw0 + v1.x * dw1 + v2.x * dw2,
        y: v0.y * dw0 + v1.y * dw1 + v2.y * dw2,
        z: v0.z * dw0 + v1.z * dw1 + v2.z * dw2,
    }
}

pub(crate) fn terrain_vertex_pixel_position(
    vertex: TerrainVertexShaderOutput,
    map_w: usize,
    map_h: usize,
) -> Vec2 {
    Vec2 {
        x: vertex.uv.x * map_w as f32 - 0.5,
        y: vertex.uv.y * map_h as f32 - 0.5,
    }
}

pub(crate) fn edge_function(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)
}

pub(crate) fn terrain_mipmap_lod_for_pixel_step(
    map_w: usize,
    map_h: usize,
    map_pow2_x: f32,
    map_pow2_y: f32,
) -> f32 {
    (terrain_mipmap_level_for_pixel_step(map_w, map_h, map_pow2_x, map_pow2_y) - 0.5)
        .clamp(0.0, 6.0)
}

pub(crate) fn terrain_mipmap_level_for_pixel_step(
    map_w: usize,
    map_h: usize,
    map_pow2_x: f32,
    map_pow2_y: f32,
) -> f32 {
    let dx_x = (TERRAIN_TILE_FREQ * map_pow2_x * (map_w as f32 / map_h as f32) / map_w as f32)
        * TEXELS_PER_TILE;
    let dy_y = (TERRAIN_TILE_FREQ * map_pow2_y / map_h as f32) * TEXELS_PER_TILE;
    let d = (dx_x * dx_x).max(dy_y * dy_y);
    0.5 * d.log2()
}

pub(crate) fn terrain_blend_frac(uv_x: f32, uv_y: f32, map_w: usize, map_h: usize) -> (f32, f32) {
    (
        (uv_x * map_w as f32 - 0.5).fract(),
        (uv_y * map_h as f32 - 0.5).fract(),
    )
}

pub(crate) fn sample_terrain_material(
    terrain_atlas: &RgbaMipChain,
    terrain_normal_atlas: &RgbaMipChain,
    height_normal: &RgbxImage,
    blend: &TerrainIndexBlend,
    tile_repeat_x: f32,
    tile_repeat_y: f32,
    mip_texels: f32,
    lod: f32,
    frac: (f32, f32),
    uv2_x: f32,
    uv2_y: f32,
) -> TerrainMaterialSample {
    let diffuse = sample_terrain_id_blend(
        terrain_atlas,
        blend,
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
        frac,
    );
    let terrain_normal_raw = sample_terrain_id_blend(
        terrain_normal_atlas,
        blend,
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
        frac,
    );
    TerrainMaterialSample {
        diffuse: diffuse.rgb,
        normal: compose_terrain_normal(
            sample_height_normal(height_normal, uv2_x, uv2_y),
            normal_from_vec3_rbg(terrain_normal_raw.rgb),
        ),
        glossiness: diffuse.alpha,
        specular: terrain_normal_raw.alpha,
    }
}

pub(crate) fn sample_terrain_id_point_clamp(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let x = (u * image.width as f32)
        .floor()
        .clamp(0.0, (image.width.saturating_sub(1)) as f32) as usize;
    let y = (v * image.height as f32)
        .floor()
        .clamp(0.0, (image.height.saturating_sub(1)) as f32) as usize;
    let offset = (y * image.width + x) * 4;
    [
        image.pixels[offset],
        image.pixels[offset + 1],
        image.pixels[offset + 2],
        image.pixels[offset + 3],
    ]
}

pub(crate) fn calculate_map_tex_index_bytes(ids: [u8; 4]) -> TerrainIndexBlend {
    let mut values = [
        f32::from(ids[0]),
        f32::from(ids[1]),
        f32::from(ids[2]),
        f32::from(ids[3]),
    ];
    let all_same_factor = (values[2] - 98.0).clamp(0.0, 1.0);
    values[2] -= all_same_factor * 100.0;
    let mut index_u = [0.0f32; 4];
    let mut index_v = [0.0f32; 4];
    for idx in 0..4 {
        index_v[idx] = ((values[idx] + 0.5) / MAP_NUM_TILES as f32).trunc();
        index_u[idx] = (values[idx] - index_v[idx] * MAP_NUM_TILES as f32 + 0.5).trunc();
    }
    TerrainIndexBlend {
        index_u,
        index_v,
        all_same: all_same_factor >= 1.0,
    }
}

pub(crate) fn sample_terrain_id_blend(
    atlas: &RgbaMipChain,
    blend: &TerrainIndexBlend,
    tile_repeat_x: f32,
    tile_repeat_y: f32,
    mip_texels: f32,
    lod: f32,
    frac: (f32, f32),
) -> SampledLinearRgba {
    let diffuse = sample_terrain_lod(
        atlas,
        blend.index_u[3],
        blend.index_v[3],
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
    );
    if blend.all_same {
        return diffuse;
    }
    let color_rd = sample_terrain_lod(
        atlas,
        blend.index_u[0],
        blend.index_v[0],
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
    );
    let color_lu = sample_terrain_lod(
        atlas,
        blend.index_u[1],
        blend.index_v[1],
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
    );
    let color_ru = sample_terrain_lod(
        atlas,
        blend.index_u[2],
        blend.index_v[2],
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
        lod,
    );
    blend_terrain_linear_samples(diffuse, color_rd, color_lu, color_ru, frac)
}

#[cfg(test)]
pub(crate) fn blend_terrain_colors(
    diffuse: u32,
    color_rd: u32,
    color_lu: u32,
    color_ru: u32,
    frac: (f32, f32),
) -> u32 {
    let top = lerp_rgb(color_ru, color_lu, frac.0);
    let bottom = lerp_rgb(color_rd, diffuse, frac.0);
    lerp_rgb(top, bottom, frac.1)
}

#[cfg(test)]
pub(crate) fn blend_terrain_samples(
    diffuse: SampledRgba,
    color_rd: SampledRgba,
    color_lu: SampledRgba,
    color_ru: SampledRgba,
    frac: (f32, f32),
) -> SampledRgba {
    let top = mix_sampled_rgba(color_ru, color_lu, frac.0);
    let bottom = mix_sampled_rgba(color_rd, diffuse, frac.0);
    mix_sampled_rgba(top, bottom, frac.1)
}

pub(crate) fn blend_terrain_linear_samples(
    diffuse: SampledLinearRgba,
    color_rd: SampledLinearRgba,
    color_lu: SampledLinearRgba,
    color_ru: SampledLinearRgba,
    frac: (f32, f32),
) -> SampledLinearRgba {
    let top = mix_sampled_linear_rgba(color_ru, color_lu, frac.0);
    let bottom = mix_sampled_linear_rgba(color_rd, diffuse, frac.0);
    mix_sampled_linear_rgba(top, bottom, frac.1)
}

pub(crate) fn sample_height_normal(image: &RgbxImage, u: f32, v: f32) -> Vec3 {
    normalize_vec3(normal_from_rbg(sample_uv_wrap_linear(image, u, v)))
}

pub(crate) fn normal_from_rbg(color: u32) -> Vec3 {
    let (r, g, b) = rgb_tuple(color);
    Vec3 {
        x: f32::from(r) / 255.0 - 0.5,
        y: f32::from(b) / 255.0 - 0.5,
        z: f32::from(g) / 255.0 - 0.5,
    }
}

pub(crate) fn normal_from_vec3_rbg(color: Vec3) -> Vec3 {
    Vec3 {
        x: color.x - 0.5,
        y: color.z - 0.5,
        z: color.y - 0.5,
    }
}

pub(crate) fn compose_terrain_normal(height_normal: Vec3, terrain_normal_raw: Vec3) -> Vec3 {
    let terrain_normal = normalize_vec3(terrain_normal_raw);
    normalize_vec3(rotate_vector_by_vector(height_normal, terrain_normal))
}

pub(crate) fn rotate_vector_by_vector(v1: Vec3, v2: Vec3) -> Vec3 {
    let zaxis = v1;
    let xaxis = normalize_vec3(cross_vec3(
        zaxis,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    ));
    let yaxis = normalize_vec3(cross_vec3(xaxis, zaxis));
    add_vec3(
        add_vec3(scale_vec3(xaxis, v2.x), scale_vec3(zaxis, v2.y)),
        scale_vec3(yaxis, v2.z),
    )
}

pub(crate) fn normalize_vec3(v: Vec3) -> Vec3 {
    let len_sq = v.x * v.x + v.y * v.y + v.z * v.z;
    if len_sq <= f32::EPSILON || !len_sq.is_finite() {
        return v;
    }
    scale_vec3(v, 1.0 / len_sq.sqrt())
}

pub(crate) fn cross_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(crate) fn dot_vec3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(crate) fn length_vec3(v: Vec3) -> f32 {
    dot_vec3(v, v).sqrt()
}

pub(crate) fn add_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(crate) fn sub_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(crate) fn mul_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x * b.x,
        y: a.y * b.y,
        z: a.z * b.z,
    }
}

pub(crate) fn scale_vec3(v: Vec3, scale: f32) -> Vec3 {
    Vec3 {
        x: v.x * scale,
        y: v.y * scale,
        z: v.z * scale,
    }
}

pub(crate) fn saturate_vec3(v: Vec3) -> Vec3 {
    Vec3 {
        x: saturate(v.x),
        y: saturate(v.y),
        z: saturate(v.z),
    }
}

pub(crate) fn vec3_is_finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

pub(crate) fn lerp_vec3(left: Vec3, right: Vec3, t: f32) -> Vec3 {
    Vec3 {
        x: lerp_f32(left.x, right.x, t),
        y: lerp_f32(left.y, right.y, t),
        z: lerp_f32(left.z, right.z, t),
    }
}

pub(crate) fn lerp_f32(left: f32, right: f32, t: f32) -> f32 {
    left + (right - left) * t
}

pub(crate) fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = saturate((value - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn cam_distance(cam_y: f32, min: f32, max: f32) -> f32 {
    (cam_y.clamp(min, max) - min) / (max - min)
}

pub(crate) fn terrain_surface_sample_is_finite(surface: TerrainSurfaceSample) -> bool {
    [
        surface.diffuse.x,
        surface.diffuse.y,
        surface.diffuse.z,
        surface.normal.x,
        surface.normal.y,
        surface.normal.z,
        surface.glossiness,
        surface.specular,
        surface.snow_alpha,
    ]
    .iter()
    .all(|value| value.is_finite())
}

#[cfg(test)]
pub(crate) fn lerp_rgb(left: u32, right: u32, t: f32) -> u32 {
    let (lr, lg, lb) = rgb_tuple(left);
    let (rr, rg, rb) = rgb_tuple(right);
    rgb_u32([lerp_u8(lr, rr, t), lerp_u8(lg, rg, t), lerp_u8(lb, rb, t)])
}

#[cfg(test)]
pub(crate) fn lerp_u8(left: u8, right: u8, t: f32) -> u8 {
    (f32::from(left) + (f32::from(right) - f32::from(left)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub(crate) fn required_shader_source_path(
    bundle: &Path,
    manifest: &Manifest,
    source: &str,
) -> Result<PathBuf> {
    let output = manifest.files.shader_sources.get(source).ok_or_else(|| {
        format!("production terrain renderer blocked: missing shader source {source}")
    })?;
    required_bundle_file(bundle, source, output)
}

pub(crate) fn required_map_source_path(
    bundle: &Path,
    manifest: &Manifest,
    source: &str,
) -> Result<PathBuf> {
    let output = manifest.files.map_sources.get(source).ok_or_else(|| {
        format!("production terrain renderer blocked: missing map source {source}")
    })?;
    required_bundle_file(bundle, source, output)
}

pub(crate) fn required_bundle_file(bundle: &Path, label: &str, output: &str) -> Result<PathBuf> {
    let path = bundle.join(output);
    if !path.is_file() {
        return Err(format!(
            "production terrain renderer blocked: bundle file for {label} is missing: {output}"
        )
        .into());
    }
    Ok(path)
}

pub(crate) fn validate_shader_constant(
    contract: &MapMaterialContract,
    key: &str,
    expected: f32,
) -> Result<()> {
    let actual = contract
        .shader_constants
        .get(key)
        .ok_or_else(|| {
            format!("production terrain renderer blocked: missing shader constant {key}")
        })?
        .value;
    if (actual - expected).abs() > 0.0001 {
        return Err(format!(
            "production terrain renderer blocked: shader constant {key} changed from compiled value {expected} to source value {actual}"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_standard_function_shader_constant(
    standard_functions_source: &Path,
    key: &str,
    expected: f32,
) -> Result<()> {
    let text = read_latin1(standard_functions_source)?;
    let actual = parse_shader_float_constant(&strip_comments(&text), key).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: gfx/FX/standardfuncsgfx.fxh missing shader constant {key}"
        )
    })?;
    if (actual - expected).abs() > 0.0001 {
        return Err(format!(
            "production terrain renderer blocked: gfx/FX/standardfuncsgfx.fxh shader constant {key} changed from compiled value {expected} to source value {actual}"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_occupation_mask_shader_source(
    standard_functions_source: &Path,
) -> Result<()> {
    let text = strip_comments(&read_latin1(standard_functions_source)?);
    let source = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let expected = [
        "floatCalculateOccupationMask(infloat2uv)",
        "floatt=3.14159/8.0;",
        "floatw=SEC_MAP_TILE;",
        "floatstripeVal=cos((uv.x*cos(t)*w)+(uv.y*sin(t)*w));",
        "floatcamDist=cam_distance(300.0,1200.0);",
        "stripeVal+=camDist*1.5;",
        "stripeVal=smoothstep(0.0,1.0,stripeVal*1.7)*lerp(1.0,0.3,camDist);",
        "returnstripeVal;",
    ];
    for needle in expected {
        if !source.contains(needle) {
            return Err(format!(
                "production terrain renderer blocked: gfx/FX/standardfuncsgfx.fxh CalculateOccupationMask no longer matches the ported occupation-stripe law near lines 965-976; missing `{needle}`"
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn validate_shader_source_vector3_constant(
    source_path: &Path,
    source_label: &str,
    key: &str,
    expected: Vec3,
) -> Result<()> {
    let text = read_latin1(source_path)?;
    let actual = parse_shader_float3_constant(&strip_comments(&text), key).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: {source_label} missing shader vector constant {key}"
        )
    })?;
    let expected = [expected.x, expected.y, expected.z];
    for (index, (actual_value, expected_value)) in actual.iter().zip(expected.iter()).enumerate() {
        if (actual_value - expected_value).abs() > 0.0001 {
            return Err(format!(
                "production terrain renderer blocked: {source_label} shader vector constant {key}[{index}] changed from compiled value {expected_value} to source value {actual_value}"
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn validate_shader_vector3_constant(
    contract: &MapMaterialContract,
    key: &str,
    expected: Vec3,
) -> Result<()> {
    let actual = contract.shader_vector3_constants.get(key).ok_or_else(|| {
        format!("production terrain renderer blocked: missing shader vector constant {key}")
    })?;
    let expected = [expected.x, expected.y, expected.z];
    for (index, (actual_value, expected_value)) in
        actual.value.iter().zip(expected.iter()).enumerate()
    {
        if (actual_value - expected_value).abs() > 0.0001 {
            return Err(format!(
                "production terrain renderer blocked: shader vector constant {key}[{index}] changed from compiled value {expected_value} to source value {actual_value}"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;

    #[test]
    fn map_frame_terrain_draw_replay_decodes_multistream_vertex_shader_outputs() {
        let root = temp_test_dir("map_frame_draw_replay_valid");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);

        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let summary = replay_map_frame_terrain_draw(
            &frame.active_draw,
            path.parent().expect("bundle dir"),
            test_map_frame_shader_constants(),
        )
        .expect("replay active terrain draw");

        assert_eq!(summary.draw_index, 1);
        assert_eq!(summary.index_count, 3);
        assert_eq!(summary.triangle_count, 1);
        assert_eq!(summary.unique_vertex_count, 3);
        assert_close(summary.prepos_min.x, 10.0);
        assert_close(summary.prepos_max.x, 12.0);
        assert_close(summary.prepos_min.y, 10.0);
        assert_close(summary.prepos_max.y, 22.0);
        assert_close(summary.prepos_min.z, 20.0);
        assert_close(summary.prepos_max.z, 24.0);
        assert_close(summary.uv_min.x, 10.5 / 64.0);
        assert_close(summary.uv_max.y, 24.5 / 64.0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_terrain_draw_replay_requires_vs_cb1() {
        let root = temp_test_dir("map_frame_draw_replay_missing_cb1");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .constant_buffers
                .retain(|buffer| !(buffer.stage == "VS" && buffer.slot == 1));
            frame
        });

        let frame = validate_map_frame_runtime_bundle(&path)
            .expect("manifest validation does not decode vertex shader CB1");
        let err = replay_map_frame_terrain_draw(
            &frame.active_draw,
            path.parent().expect("bundle dir"),
            test_map_frame_shader_constants(),
        )
        .expect_err("missing pdxmap.fxh CB1 must block replay");
        assert!(err.to_string().contains("VS constant-buffer slot 1"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_terrain_draw_replay_rejects_unsupported_index_format() {
        let root = temp_test_dir("map_frame_draw_replay_bad_index_format");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .index_buffer
                .as_mut()
                .expect("index buffer")
                .dxgi_format = 999;
            frame
        });

        let frame = validate_map_frame_runtime_bundle(&path)
            .expect("manifest validation only proves index buffer bytes");
        let err = replay_map_frame_terrain_draw(
            &frame.active_draw,
            path.parent().expect("bundle dir"),
            test_map_frame_shader_constants(),
        )
        .expect_err("unsupported index format must block replay");
        assert!(err
            .to_string()
            .contains("unsupported map-frame index buffer DXGI format"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_terrain_draw_replay_rejects_height_stream_offset_overrun() {
        let root = temp_test_dir("map_frame_draw_replay_height_offset_overrun");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .vertex_buffers
                .get_mut("1")
                .expect("slot 1 vertex buffer")
                .offset = Some(999);
            frame
        });

        let frame = validate_map_frame_runtime_bundle(&path)
            .expect("manifest validation does not walk indexed vertex addresses");
        let err = replay_map_frame_terrain_draw(
            &frame.active_draw,
            path.parent().expect("bundle dir"),
            test_map_frame_shader_constants(),
        )
        .expect_err("out-of-range captured IA offset must block replay");
        assert!(err.to_string().contains("missing i16 bytes"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_diffuse_runtime_sampling_uses_shader_point_light_sentinel() {
        let scene_cb0 = decode_terrain_scene_cb0(
            &valid_scene_cb0_bytes(),
            &terrain_scene_cb0_layout(&minimal_standard_functions_source()).expect("CB0 layout"),
        )
        .expect("decode CB0");
        let mut light_index_pixels =
            vec![255u8; LIGHT_INDEX_TEXTURE_SIZE * LIGHT_INDEX_TEXTURE_SIZE * 4];
        let light_index_offset = (2 * LIGHT_INDEX_TEXTURE_SIZE + 1) * 4;
        light_index_pixels[light_index_offset..light_index_offset + 4]
            .copy_from_slice(&[3, 255, 9, 9]);
        let mut light_data_pixels = vec![[0.0f32; 4]; LIGHT_DATA_TEXTURE_SIZE];
        light_data_pixels[6] = [1.0, 2.0, 3.0, 4.0];
        light_data_pixels[7] = [5.0, 6.0, 7.0, 8.0];
        let post_diffuse = TerrainPostDiffuseResources {
            snow_mud_data: RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![64, 128, 192, 255],
            },
            snow_texture: single_pixel_rgba_chain([128, 128, 255, 0]),
            city_lights_snow_noise: single_pixel_rgba_chain([0, 0, 0, 0]),
            mud_diffuse_gloss: single_pixel_rgba_chain([0, 0, 0, 0]),
            mud_normal_spec: single_pixel_rgba_chain([128, 128, 255, 0]),
            province_secondary_color_map: single_pixel_rgba_chain([0, 0, 0, 0]),
            gradient_border_channel1: single_pixel_rgba_chain([0, 0, 0, 0]),
            gradient_border_channel2: single_pixel_rgba_chain([0, 0, 0, 0]),
            light_index_map: RgbaImage {
                width: LIGHT_INDEX_TEXTURE_SIZE,
                height: LIGHT_INDEX_TEXTURE_SIZE,
                pixels: light_index_pixels,
            },
            light_data_map: Rgba32fImage {
                width: LIGHT_DATA_TEXTURE_SIZE,
                height: 1,
                pixels: light_data_pixels,
            },
            shadow_map: TerrainShadowMapState::NullSrv,
            scene_cb0: TerrainSceneCb0Decoded {
                shadow_fade_factor: 0.25,
                ..scene_cb0
            },
            point_light_cb2: TerrainPointLightCb2Decoded {
                grid_start_inv_cell_size: [0.0, 0.0, 1.0, 1.0],
            },
            constants: test_map_frame_shader_constants(),
        };

        let sample = sample_post_diffuse_runtime(
            &post_diffuse,
            TerrainPixelShaderInput {
                uv: Vec2 { x: 0.5, y: 0.5 },
                uv2: Vec2 { x: 0.5, y: 0.5 },
                prepos: Vec3 {
                    x: 1.2,
                    y: 0.0,
                    z: 2.3,
                },
                v_shadow_proj: Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                v_screen_coord: Vec4 {
                    x: 0.5,
                    y: 0.5,
                    z: 1.0,
                    w: 1.0,
                },
            },
            TerrainPixelShaderDerivatives {
                prepos_dx: Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                prepos_dy: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            },
            TerrainSurfaceSample {
                diffuse: Vec3 {
                    x: 0.2,
                    y: 0.2,
                    z: 0.2,
                },
                normal: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                glossiness: 0.0,
                specular: 0.0,
                snow_alpha: 0.0,
            },
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .expect("post-diffuse runtime sample");

        assert_close(sample.mud_snow[0], 64.0 / 255.0);
        assert_close(sample.mud_snow[1], 128.0 / 255.0);
        assert_close(sample.mud_snow[2], 192.0 / 255.0);
        assert_close(sample.mud_snow[3], 1.0);
        assert_eq!(sample.point_light_indices, [3, 255, 9, 9]);
        assert_eq!(sample.point_light_count, 1);
        assert_close(sample.shadow_term, 0.825);
    }

    #[test]
    fn mud_snow_sampling_multiplies_entire_uv_by_fow_pow2_y() {
        let scene_cb0 = test_scene_cb0_with_camera_and_fade(50.0, 1.0);
        let mut post_diffuse = test_post_diffuse_resources(scene_cb0);
        post_diffuse.snow_mud_data = RgbaImage {
            width: 4,
            height: 1,
            pixels: vec![
                0, 0, 0, 255, //
                64, 64, 64, 255, //
                128, 128, 128, 255, //
                192, 192, 192, 255,
            ],
        };
        post_diffuse.constants = MapFrameTerrainShaderConstants {
            map_size_x: 4.0,
            map_size_y: 4.0,
            map_pow2_x: 1.0,
            map_pow2_y: 1.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 2.0,
        };

        let sample = sample_mud_snow_color(
            &post_diffuse,
            Vec3 {
                x: 0.25,
                y: 0.0,
                z: 0.5,
            },
        );

        assert_close(sample[0], 64.0 / 255.0);
        assert_close(sample[1], 64.0 / 255.0);
        assert_close(sample[2], 64.0 / 255.0);
        assert_close(sample[3], 1.0);
    }

    #[test]
    fn apply_snow_matches_shader_alpha_color_and_gloss_formula() {
        let scene_cb0 = test_scene_cb0_with_camera_and_fade(50.0, 1.0);
        let post_diffuse = test_post_diffuse_resources(scene_cb0);
        let surface = apply_snow(
            &post_diffuse,
            test_pixel_input_with_prepos(Vec3 {
                x: 0.0,
                y: 4.0,
                z: 0.0,
            }),
            test_pixel_derivatives(),
            TerrainSurfaceSample {
                diffuse: Vec3 {
                    x: 0.2,
                    y: 0.2,
                    z: 0.2,
                },
                normal: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                glossiness: 0.0,
                specular: 0.0,
                snow_alpha: 1.0,
            },
            [0.0, 1.0, 0.0, 0.0],
        );

        assert_vec3_close(surface.diffuse, SNOW_COLOR);
        assert_close(surface.snow_alpha, 0.95);
        assert_close(surface.glossiness, 0.95 * SNOW_SPEC_GLOSS_MULT);
        assert!(terrain_surface_sample_is_finite(surface));
    }

    #[test]
    fn apply_mud_matches_shader_current_overlay_gloss_and_spec_formula() {
        let scene_cb0 = test_scene_cb0_with_camera_and_fade(50.0, 1.0);
        let mut post_diffuse = test_post_diffuse_resources(scene_cb0);
        post_diffuse.mud_diffuse_gloss = single_pixel_rgba_chain([255, 0, 0, 128]);
        post_diffuse.mud_normal_spec = single_pixel_rgba_chain([128, 128, 255, 64]);
        let terrain_color = Vec3 {
            x: 0.5,
            y: 0.5,
            z: 0.5,
        };
        let surface = apply_mud(
            &post_diffuse,
            test_pixel_input_with_prepos(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            test_pixel_derivatives(),
            TerrainSurfaceSample {
                diffuse: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                normal: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                glossiness: 0.0,
                specular: 0.0,
                snow_alpha: 0.0,
            },
            terrain_color,
            [0.0, 0.0, 0.0, 1.0],
        );

        assert_vec3_close(
            surface.diffuse,
            get_overlay_vec3(
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                terrain_color,
                COLORMAP_MUD_OVERLAY_STRENGTH,
            ),
        );
        assert_close(surface.glossiness, 128.0 / 255.0);
        assert_close(surface.specular, 64.0 / 255.0);
        assert!(terrain_surface_sample_is_finite(surface));
    }

    #[test]
    fn post_diffuse_texture_lod_uses_world_position_derivatives() {
        let chain = RgbaMipChain {
            levels: vec![RgbaImage {
                width: 1024,
                height: 1024,
                pixels: vec![0; 1024 * 1024 * 4],
            }],
            color_space: RgbaColorSpace::Linear,
        };
        let lod = texture_lod_for_world_xz(
            &chain,
            TerrainPixelShaderDerivatives {
                prepos_dx: Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                prepos_dy: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            },
            SNOW_TILING,
        );
        assert_close(lod, (1024.0_f32 * SNOW_TILING).log2());
    }

    #[test]
    fn runtime_rgba_color_space_comes_from_captured_dxgi_format() {
        let base = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 6,
            declared_name: Some(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL.to_string()),
            is_null: false,
            view_desc: None,
            file: Some("province_secondary_color_map.rgba".to_string()),
            byte_len: 4,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            width: 1,
            height: 1,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: 8,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        assert_eq!(
            runtime_rgba_color_space(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL, &base)
                .expect("captured BGRA sRGB runtime texture"),
            RgbaColorSpace::Srgb
        );
        assert_eq!(
            runtime_rgba_color_space(
                PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
                &MapFrameTextureBinding {
                    dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    ..base.clone()
                },
            )
            .expect("captured BGRA linear runtime texture"),
            RgbaColorSpace::Linear
        );
        assert_eq!(
            runtime_rgba_color_space(
                PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
                &MapFrameTextureBinding {
                    dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ..base.clone()
                },
            )
            .expect("captured RGBA linear runtime texture"),
            RgbaColorSpace::Linear
        );
        let err = runtime_rgba_color_space(
            PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
            &MapFrameTextureBinding {
                dxgi_format: DXGI_FORMAT_BC3_UNORM_SRGB,
                ..base
            },
        )
        .expect_err("compressed runtime capture bytes are not the renderer input format");
        assert!(err.to_string().contains("unsupported RGBA color-space"));
    }

    #[test]
    fn shipped_material_color_space_comes_from_live_srv_dxgi_format() {
        let base = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 8,
            declared_name: Some(CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL.to_string()),
            is_null: false,
            view_desc: None,
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format: DXGI_FORMAT_BC3_UNORM_SRGB,
            width: 512,
            height: 512,
            mip_count: 10,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: 8,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        assert_eq!(
            shipped_material_color_space(CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL, &base)
                .expect("BC3 sRGB material"),
            RgbaColorSpace::Srgb
        );
        assert_eq!(
            shipped_material_color_space(
                SNOW_TEXTURE_SYMBOL,
                &MapFrameTextureBinding {
                    dxgi_format: DXGI_FORMAT_BC3_UNORM,
                    ..base.clone()
                },
            )
            .expect("BC3 linear material"),
            RgbaColorSpace::Linear
        );
        assert_eq!(
            shipped_material_color_space(
                TERRAIN_COLOR_TINT_SYMBOL,
                &MapFrameTextureBinding {
                    dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                    ..base.clone()
                },
            )
            .expect("BGRA sRGB colormap"),
            RgbaColorSpace::Srgb
        );
        let err = shipped_material_color_space(
            CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL,
            &MapFrameTextureBinding {
                dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ..base
            },
        )
        .expect_err("unsupported shipped material format must fail closed");
        assert!(err.to_string().contains("expected live SRV DXGI format"));
    }

    #[test]
    fn active_terrain_gradient_border_branch_samples_channels_and_bloom_alpha() {
        let scene_cb0 = test_scene_cb0_with_camera_and_fade(GB_CAM_MAX, 1.0);
        let mut post_diffuse = test_post_diffuse_resources(scene_cb0);
        post_diffuse.gradient_border_channel1 = single_pixel_rgba_chain([255, 0, 0, 0]);
        post_diffuse.gradient_border_channel2 = single_pixel_rgba_chain([0, 255, 0, 255]);
        let base_surface = TerrainSurfaceSample {
            diffuse: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            glossiness: 0.0,
            specular: 0.0,
            snow_alpha: 0.0,
        };

        let sample = apply_terrain_gradient_borders(
            &post_diffuse,
            TerrainPixelShaderInput {
                uv2: Vec2 { x: 0.5, y: 0.5 },
                ..test_pixel_input_with_prepos(Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                })
            },
            base_surface,
        )
        .expect("active gradient border shader path");

        let gb_cam_dist = gradient_border_camera_distance(GB_CAM_MAX);
        let alpha1 = gradient_border_distance_to_alpha(0.0, gb_cam_dist);
        let opacity = GB_OPACITY_NEAR + (1.0 - gb_cam_dist) * (GB_OPACITY_FAR - GB_OPACITY_NEAR);
        assert_close(sample.bloom_alpha, 1.0 - alpha1);
        assert_vec3_close(
            sample.surface.diffuse,
            Vec3 {
                x: alpha1 * opacity,
                y: 0.0,
                z: 0.0,
            },
        );
    }

    #[test]
    fn secondary_color_mask_matches_shader_occupation_mask_and_bloom_alpha() {
        let scene_cb0 = test_scene_cb0_with_camera_and_fade(SEC_CAM_MIN, 1.0);
        let mut post_diffuse = test_post_diffuse_resources(scene_cb0);
        post_diffuse.province_secondary_color_map = RgbaMipChain {
            levels: vec![RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![128, 64, 255, 128],
            }],
            color_space: RgbaColorSpace::Srgb,
        };
        let base_surface = TerrainSurfaceSample {
            diffuse: Vec3 {
                x: 0.2,
                y: 0.4,
                z: 0.6,
            },
            normal: Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            glossiness: 0.25,
            specular: 0.5,
            snow_alpha: 0.75,
        };

        let sample = apply_terrain_secondary_mask(
            &post_diffuse,
            TerrainPixelShaderInput {
                uv2: Vec2 { x: 0.0, y: 0.0 },
                ..test_pixel_input_with_prepos(Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                })
            },
            1.0,
            base_surface,
        )
        .expect("secondary color mask");

        let occupation_mask =
            calculate_occupation_mask(Vec2 { x: 0.0, y: 0.0 }, SEC_CAM_MIN) * (128.0 / 255.0);
        assert_close(sample.bloom_alpha, 1.0 - occupation_mask);
        assert_vec3_close(
            sample.surface.diffuse,
            lerp_vec3(
                base_surface.diffuse,
                Vec3 {
                    x: srgb_u8_to_linear_f32(128),
                    y: srgb_u8_to_linear_f32(64),
                    z: 1.0,
                },
                occupation_mask,
            ),
        );
        assert_close(sample.surface.glossiness, base_surface.glossiness);
        assert_close(sample.surface.specular, base_surface.specular);
        assert_close(sample.surface.snow_alpha, base_surface.snow_alpha);
    }

    #[test]
    fn border_light_removal_matches_pdxmap_lerp() {
        let lit = Vec3 {
            x: 0.8,
            y: 0.6,
            z: 0.4,
        };
        let diffuse = Vec3 {
            x: 0.2,
            y: 0.4,
            z: 0.6,
        };

        assert_vec3_close(
            apply_border_light_removal(lit, diffuse, 0.25),
            lerp_vec3(lit, diffuse, BORDER_LIGHT_REMOVAL_FACTOR * 0.75),
        );
    }

    #[test]
    fn post_diffuse_light_data_uses_point_clamp_for_high_non_sentinel_index() {
        let mut pixels = vec![[0.0; 4]; LIGHT_DATA_TEXTURE_SIZE];
        pixels[LIGHT_DATA_TEXTURE_SIZE - 1] = [9.0, 8.0, 7.0, 6.0];
        let light_data_map = Rgba32fImage {
            width: LIGHT_DATA_TEXTURE_SIZE,
            height: 1,
            pixels,
        };
        let (first, second) = sample_point_light_data_pair(&light_data_map, 64);
        assert_eq!(first, [9.0, 8.0, 7.0, 6.0]);
        assert_eq!(second, [9.0, 8.0, 7.0, 6.0]);
    }

    #[test]
    fn map_frame_light_map_dimension_validation_fails_closed() {
        let err = validate_exact_map_frame_resource_dimensions(
            LIGHT_INDEX_MAP_SYMBOL,
            32,
            LIGHT_INDEX_TEXTURE_SIZE,
            LIGHT_INDEX_TEXTURE_SIZE,
            LIGHT_INDEX_TEXTURE_SIZE,
        )
        .expect_err("LightIndexMap must stay 64x64 per tiled_pointlights.fxh");
        assert!(err.to_string().contains(LIGHT_INDEX_MAP_SYMBOL));
        assert!(err.to_string().contains("expected 64x64"));
    }

    #[test]
    fn shader_vector_constant_validation_rejects_source_drift() {
        let mut contract = minimal_test_contract();
        contract
            .shader_vector3_constants
            .get_mut("SNOW_COLOR")
            .expect("SNOW_COLOR vector")
            .value[1] = 0.5;
        let err = validate_shader_vector3_constant(&contract, "SNOW_COLOR", SNOW_COLOR)
            .expect_err("changed SNOW_COLOR must fail closed");
        assert!(err.to_string().contains("SNOW_COLOR[1]"));
    }

    #[test]
    fn terrain_id_map_decode_matches_shader_formula() {
        let blend = calculate_map_tex_index_bytes([0, 1, 2, 3]);
        assert!(!blend.all_same);
        assert_eq!(blend.index_u, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(blend.index_v, [0.0, 0.0, 0.0, 0.0]);

        let sentinel = calculate_map_tex_index_bytes([7, 8, 105, 9]);
        assert!(sentinel.all_same);
        assert_eq!(sentinel.index_u[0], 3.0);
        assert_eq!(sentinel.index_v[0], 1.0);
        assert_eq!(sentinel.index_u[2], 1.0);
        assert_eq!(sentinel.index_v[2], 1.0);
        assert_eq!(sentinel.index_u[3], 1.0);
        assert_eq!(sentinel.index_v[3], 2.0);
    }

    #[test]
    fn terrain_blend_color_order_matches_pdxmap_shader() {
        let diffuse = rgb_u32([0, 0, 255]);
        let color_rd = rgb_u32([255, 0, 0]);
        let color_lu = rgb_u32([0, 255, 0]);
        let color_ru = rgb_u32([255, 255, 0]);

        assert_eq!(
            blend_terrain_colors(diffuse, color_rd, color_lu, color_ru, (0.0, 0.0)),
            color_ru
        );
        assert_eq!(
            blend_terrain_colors(diffuse, color_rd, color_lu, color_ru, (1.0, 0.0)),
            color_lu
        );
        assert_eq!(
            blend_terrain_colors(diffuse, color_rd, color_lu, color_ru, (0.0, 1.0)),
            color_rd
        );
        assert_eq!(
            blend_terrain_colors(diffuse, color_rd, color_lu, color_ru, (1.0, 1.0)),
            diffuse
        );
    }

    #[test]
    fn terrain_blend_sample_order_preserves_material_alpha() {
        let diffuse = SampledRgba {
            rgb: rgb_u32([0, 0, 255]),
            alpha: 0.4,
        };
        let color_rd = SampledRgba {
            rgb: rgb_u32([255, 0, 0]),
            alpha: 0.1,
        };
        let color_lu = SampledRgba {
            rgb: rgb_u32([0, 255, 0]),
            alpha: 0.2,
        };
        let color_ru = SampledRgba {
            rgb: rgb_u32([255, 255, 0]),
            alpha: 0.3,
        };

        assert_close(
            blend_terrain_samples(diffuse, color_rd, color_lu, color_ru, (0.0, 0.0)).alpha,
            color_ru.alpha,
        );
        assert_close(
            blend_terrain_samples(diffuse, color_rd, color_lu, color_ru, (1.0, 0.0)).alpha,
            color_lu.alpha,
        );
        assert_close(
            blend_terrain_samples(diffuse, color_rd, color_lu, color_ru, (0.0, 1.0)).alpha,
            color_rd.alpha,
        );
        assert_close(
            blend_terrain_samples(diffuse, color_rd, color_lu, color_ru, (1.0, 1.0)).alpha,
            diffuse.alpha,
        );
    }

    #[test]
    fn terrain_material_sample_decodes_diffuse_gloss_normal_and_specular_channels() {
        let diffuse_atlas = single_pixel_rgba_chain([10, 20, 30, 64]);
        let normal_atlas = single_pixel_rgba_chain([255, 128, 128, 200]);
        let height_normal = RgbxImage {
            width: 1,
            height: 1,
            pixels: vec![rgb_u32([128, 128, 255])],
        };
        let blend = TerrainIndexBlend {
            index_u: [0.0; 4],
            index_v: [0.0; 4],
            all_same: true,
        };

        let material = sample_terrain_material(
            &diffuse_atlas,
            &normal_atlas,
            &height_normal,
            &blend,
            0.0,
            0.0,
            2048.0,
            0.0,
            (0.0, 0.0),
            0.5,
            0.5,
        );

        assert_vec3_close(
            material.diffuse,
            Vec3 {
                x: 10.0 / 255.0,
                y: 20.0 / 255.0,
                z: 30.0 / 255.0,
            },
        );
        assert_close(material.glossiness, 64.0 / 255.0);
        assert_close(material.specular, 200.0 / 255.0);
        assert_unit_vec3(material.normal);
    }

    #[test]
    fn terrain_material_sample_decodes_srgb_diffuse_atlas_before_blending() {
        let mut diffuse_atlas = single_pixel_rgba_chain([128, 64, 255, 64]);
        diffuse_atlas.color_space = RgbaColorSpace::Srgb;
        let normal_atlas = single_pixel_rgba_chain([128, 128, 255, 200]);
        let height_normal = RgbxImage {
            width: 1,
            height: 1,
            pixels: vec![rgb_u32([128, 128, 255])],
        };
        let blend = TerrainIndexBlend {
            index_u: [0.0; 4],
            index_v: [0.0; 4],
            all_same: true,
        };

        let material = sample_terrain_material(
            &diffuse_atlas,
            &normal_atlas,
            &height_normal,
            &blend,
            0.0,
            0.0,
            2048.0,
            0.0,
            (0.0, 0.0),
            0.5,
            0.5,
        );

        assert_vec3_close(
            material.diffuse,
            Vec3 {
                x: srgb_u8_to_linear_f32(128),
                y: srgb_u8_to_linear_f32(64),
                z: 1.0,
            },
        );
        assert_close(material.glossiness, 64.0 / 255.0);
        assert_close(material.specular, 200.0 / 255.0);
    }

    #[test]
    fn terrain_lighting_glossiness_matches_shader_exp2() {
        assert_close(get_non_linear_glossiness(0.0), 1.0);
        assert_close(get_non_linear_glossiness(1.0), 2048.0);
        assert_close(get_non_linear_glossiness(0.5), (11.0_f32 * 0.5).exp2());
    }

    #[test]
    fn terrain_ambient_light_matches_map_axis_constants() {
        assert_vec3_close(
            ambient_light(
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                0.0,
                0.0,
            ),
            DAY_AMBIENT_MAP_POS_X,
        );
        assert_vec3_close(
            ambient_light(
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                1.0,
                0.0,
            ),
            saturate_vec3(scale_vec3(NIGHT_AMBIENT_MAP_POS_X, NIGHT_AMBIENT_BOOST)),
        );
    }

    #[test]
    fn terrain_point_lights_use_index_map_and_data_pairs() {
        let scene = test_scene_cb0_with_camera_and_fade(10.0, 1.0);
        let mut post_diffuse = test_post_diffuse_resources(scene);
        post_diffuse.light_index_map.pixels =
            vec![255; LIGHT_INDEX_TEXTURE_SIZE * LIGHT_INDEX_TEXTURE_SIZE * 4];
        post_diffuse.light_index_map.pixels[0..4].copy_from_slice(&[3, 255, 255, 255]);
        post_diffuse.light_data_map.pixels[6] = [0.0, 0.0, 10.0, 20.0];
        post_diffuse.light_data_map.pixels[7] = [0.25, 0.5, 1.0, 10.0];
        let properties = TerrainLightingProperties {
            world_space_pos: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            to_camera_dir: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            normal: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            diffuse: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            specular_color: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            glossiness: 0.0,
            non_linear_glossiness: 1.0,
        };
        let mut diffuse = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let mut specular = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };

        calculate_point_lights(&post_diffuse, properties, &mut diffuse, &mut specular);

        assert_vec3_close(
            diffuse,
            Vec3 {
                x: 0.25,
                y: 0.5,
                z: 1.0,
            },
        );
        assert_vec3_close(
            specular,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        );
    }

    #[test]
    fn compose_light_snow_matches_shader_ambient_diffuse_specular_formula() {
        let mut scene = test_scene_cb0_with_camera_and_fade(10.0, 1.0);
        scene.hdr_range = 2.0;
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 0.0;
        let properties = TerrainLightingProperties {
            world_space_pos: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            to_camera_dir: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            normal: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            diffuse: Vec3 {
                x: 0.5,
                y: 0.5,
                z: 0.5,
            },
            specular_color: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            glossiness: 0.0,
            non_linear_glossiness: 1.0,
        };
        let diffuse_light = Vec3 {
            x: 0.1,
            y: 0.2,
            z: 0.3,
        };
        let specular_light = Vec3 {
            x: 0.01,
            y: 0.02,
            z: 0.03,
        };

        let lit = compose_light_snow(
            scene,
            test_map_frame_shader_constants(),
            properties,
            diffuse_light,
            specular_light,
            0.0,
        );

        assert_vec3_close(
            lit,
            add_vec3(
                scale_vec3(
                    mul_vec3(
                        add_vec3(DAY_AMBIENT_MAP_POS_X, diffuse_light),
                        properties.diffuse,
                    ),
                    2.0,
                ),
                specular_light,
            ),
        );
    }

    #[test]
    fn terrain_id_map_sampling_is_point_clamped() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        assert_eq!(
            sample_terrain_id_point_clamp(&image, 0.0, 0.0),
            [1, 2, 3, 4]
        );
        assert_eq!(
            sample_terrain_id_point_clamp(&image, 0.75, 0.25),
            [5, 6, 7, 8]
        );
        assert_eq!(
            sample_terrain_id_point_clamp(&image, -10.0, 10.0),
            [9, 10, 11, 12]
        );
    }

    #[test]
    fn map_frame_triangle_rasterizer_shades_covered_pixels_from_terrain_resources() {
        let terrain_id_map = RgbaImage {
            width: 2,
            height: 2,
            pixels: vec![0; 2 * 2 * 4],
        };
        let terrain_atlas = single_pixel_rgba_chain([80, 120, 160, 255]);
        let terrain_normal_atlas = single_pixel_rgba_chain([128, 128, 255, 128]);
        let height_normal = RgbxImage {
            width: 1,
            height: 1,
            pixels: vec![rgb_u32([128, 128, 255])],
        };
        let terrain_color_tint = single_pixel_rgba_chain([192, 192, 192, 255]);
        let resources = TerrainPixelResources {
            map_w: 2,
            map_h: 2,
            terrain_id_map: &terrain_id_map,
            terrain_atlas: &terrain_atlas,
            terrain_normal_atlas: &terrain_normal_atlas,
            height_normal: &height_normal,
            terrain_color_tint: &terrain_color_tint,
            post_diffuse: None,
            lod: 0.0,
        };
        let input = TerrainVertexShaderInput {
            position: [0, 0, 0, 0],
            height: [0, 0],
        };
        let triangle = [
            TerrainVertexShaderOutput {
                input,
                position: Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                uv: Vec2 { x: 0.0, y: 0.0 },
                uv2: Vec2 { x: 0.0, y: 1.0 },
                prepos: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                v_shadow_proj: Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                v_screen_coord: Vec4 {
                    x: 0.5,
                    y: 0.5,
                    z: 1.0,
                    w: 1.0,
                },
            },
            TerrainVertexShaderOutput {
                input,
                position: Vec4 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                uv: Vec2 { x: 1.0, y: 0.0 },
                uv2: Vec2 { x: 1.0, y: 1.0 },
                prepos: Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                v_shadow_proj: Vec4 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                v_screen_coord: Vec4 {
                    x: 1.0,
                    y: 0.5,
                    z: 1.0,
                    w: 1.0,
                },
            },
            TerrainVertexShaderOutput {
                input,
                position: Vec4 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                    w: 1.0,
                },
                uv: Vec2 { x: 0.0, y: 1.0 },
                uv2: Vec2 { x: 0.0, y: 0.0 },
                prepos: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                v_shadow_proj: Vec4 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                    w: 1.0,
                },
                v_screen_coord: Vec4 {
                    x: 0.5,
                    y: 0.0,
                    z: 1.0,
                    w: 1.0,
                },
            },
        ];
        let mut out = vec![0; 4];
        let mut covered = vec![false; 4];

        rasterize_map_frame_terrain_triangle(&mut out, &mut covered, &resources, triangle)
            .expect("triangle rasterization");

        assert!(covered.iter().any(|covered| *covered));
        assert!(out
            .iter()
            .zip(covered.iter())
            .any(|(pixel, covered)| *covered && *pixel != 0));
    }

    #[test]
    fn terrain_mipmap_level_matches_shader_derivative_formula_for_full_map() {
        assert_close(
            terrain_mipmap_level_for_pixel_step(5632, 2048, 1.0, 1.0),
            5.0,
        );
        assert_close(terrain_mipmap_lod_for_pixel_step(5632, 2048, 1.0, 1.0), 4.5);
    }

    #[test]
    fn terrain_normal_rbg_swizzle_matches_pdxmap_shader() {
        let normal = normal_from_rbg(rgb_u32([255, 128, 0]));
        assert_close(normal.x, 0.5);
        assert_close(normal.y, -0.5);
        assert_close(normal.z, 128.0 / 255.0 - 0.5);
    }

    #[test]
    fn terrain_normal_composition_matches_shader_basis() {
        let height_normal = Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        assert_vec3_close(
            compose_terrain_normal(
                height_normal,
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ),
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        );
        assert_vec3_close(
            compose_terrain_normal(
                height_normal,
                Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ),
            height_normal,
        );
    }

    #[test]
    fn terrain_final_tail_applies_city_lights_and_daynight_after_null_fow() {
        let prepos = Vec3 {
            x: 1.0,
            y: 0.0,
            z: 1.0,
        };
        let mut scene = test_scene_cb0_with_camera_and_fade(prepos.y, 0.0);
        scene.v_cam_pos = prepos;
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 1.0;
        scene.day_night_hour_sun_dir[0] = 0.0;
        let constants = test_map_frame_shader_constants();
        let globe = calc_globe_normal(scene, constants, prepos);
        scene.day_night_hour_sun_dir[1] = globe.x;
        scene.day_night_hour_sun_dir[2] = globe.y;
        scene.day_night_hour_sun_dir[3] = globe.z;

        let mut post_diffuse = test_post_diffuse_resources(scene);
        post_diffuse.city_lights_snow_noise = single_pixel_rgba_chain([255, 0, 0, 255]);
        let input = test_pixel_input_with_prepos(prepos);
        let base = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let out = apply_terrain_final_shader_tail(
            &post_diffuse,
            input,
            test_pixel_derivatives(),
            base,
            1.0,
            0.5,
        )
        .expect("terrain shader tail");

        let city_added = Vec3 {
            x: CITY_LIGHTS_INTENSITY * 0.5,
            y: 0.0,
            z: 0.0,
        };
        let expected = day_night_with_blend(scene, city_added, globe, 1.0);
        assert_vec3_close(out, expected);
        assert!(
            out.x < city_added.x,
            "day/night blend must run after city lights"
        );
        assert!(
            out.y > 0.0,
            "night color blend must affect non-red channels"
        );
    }

    // ---- NIGHT2 (2026-07-09): gates for the ported night path (FIDELITY
    // LEDGER 2.13). The mechanism is the shipped-HLSL port above
    // (`calc_globe_normal` fxh:347, `day_night_factor` fxh:358/361,
    // `nightify_color` fxh:370, `day_night_with_blend` fxh:393). Product
    // validation stays capture-gated (no FoW.w != 0 frame exists); these are
    // the synthetic unit gates + the captured-day-state byte-noop proof.

    #[test]
    fn gate_fow_gate_zero_makes_ported_night_path_a_byte_noop() {
        // Every captured scene CB0 carries
        // vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.w == 0, and
        // DayNightFactor multiplies by that gate (standardfuncsgfx.fxh:361).
        // So on the captured day state the ported night path must change
        // NOTHING: factor == +0.0 exactly and day_night_with_blend returns the
        // input color bit-for-bit (lerp_f32(a, b, 0) = a + (b-a)*0 = a).
        let mut scene = test_scene_cb0_with_camera_and_fade(800.0, 0.0);
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 0.0;
        let constants = test_map_frame_shader_constants();
        let colors = [
            Vec3 {
                x: 0.6,
                y: 0.4,
                z: 0.2,
            },
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            Vec3 {
                x: 0.031_25,
                y: 12.5,
                z: 0.007_812_5,
            },
        ];
        let positions = [
            Vec3 {
                x: 0.5,
                y: 1.0,
                z: 0.5,
            },
            Vec3 {
                x: 31.0,
                y: 2.0,
                z: 63.0,
            },
            Vec3 {
                x: 63.5,
                y: 0.0,
                z: 0.5,
            },
        ];
        for hour_phase in [0.0f32, 1.0 / 24.0, 0.25, 16.0 / 24.0, 0.99] {
            scene.day_night_hour_sun_dir[0] = hour_phase;
            for pos in positions {
                let globe = calc_globe_normal(scene, constants, pos);
                let factor = day_night_factor(scene, globe, FEATHER_MIN, FEATHER_MAX);
                assert_eq!(
                    factor.to_bits(),
                    0.0f32.to_bits(),
                    "FoW.w == 0 must gate the day/night factor to +0.0 exactly \
                     (hour phase {hour_phase}, pos {pos:?})"
                );
                for color in colors {
                    for blend in [0.0f32, 0.2, 1.0] {
                        let out = day_night_with_blend(scene, color, globe, blend);
                        for (lane, (o, c)) in [(out.x, color.x), (out.y, color.y), (out.z, color.z)]
                            .into_iter()
                            .enumerate()
                        {
                            assert_eq!(
                                o.to_bits(),
                                c.to_bits(),
                                "night path must be a byte-noop at FoW.w == 0 \
                                 (lane {lane}, hour {hour_phase}, blend {blend})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn gate_ambient_light_day_state_is_pure_day_rig() {
        // AmbientLight (fxh:456-470) at day (day_factor == 0) must return the
        // pure DAY rig contribution bit-exactly: lerp(day, boosted-night, 0)
        // == day. With neg_fog_multiplier == 0 the normal adjustment (fxh:459)
        // is also an exact identity lerp, so the whole result is the
        // hand-computable sum of squared components times the day faces.
        let day = [
            DAY_AMBIENT_MAP_POS_X,
            DAY_AMBIENT_MAP_NEG_X,
            DAY_AMBIENT_MAP_POS_Y,
            DAY_AMBIENT_MAP_NEG_Y,
            DAY_AMBIENT_MAP_POS_Z,
            DAY_AMBIENT_MAP_NEG_Z,
        ];
        let normals = [
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            Vec3 {
                x: 0.36,
                y: 0.48,
                z: -0.8,
            },
        ];
        for normal in normals {
            let out = ambient_light(normal, 0.0, 0.0);
            let sq = mul_vec3(normal, normal);
            let pick = |neg: bool, base: usize| day[base + neg as usize];
            let expected = add_vec3(
                add_vec3(
                    scale_vec3(pick(normal.x < 0.0, 0), sq.x),
                    scale_vec3(pick(normal.y < 0.0, 2), sq.y),
                ),
                scale_vec3(pick(normal.z < 0.0, 4), sq.z),
            );
            for (lane, (o, e)) in [
                (out.x, expected.x),
                (out.y, expected.y),
                (out.z, expected.z),
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(
                    o.to_bits(),
                    e.to_bits(),
                    "day-state ambient must be the pure day rig (normal {normal:?}, lane {lane})"
                );
            }
        }
        // And at full night (day_factor == 1) the boosted night faces drive it:
        // normal (0,-1,0) selects NightAmbientMapNegY = (0, 0, 0.1) * 3.0.
        let night_out = ambient_light(
            Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            1.0,
            0.0,
        );
        assert!((night_out.z - 0.1 * NIGHT_AMBIENT_BOOST).abs() < 1e-6);
        assert!(night_out.x.abs() < 1e-6 && night_out.y.abs() < 1e-6);
    }

    #[test]
    fn gate_day_night_factor_feather_ramp_endpoints_and_gate_scale() {
        // DayNightFactor = saturate((dot - vMin)/(vMax - vMin)) * FoW.w
        // (fxh:358-361) with FEATHER_MIN/MAX = -0.01/+0.01 (fxh:324-325).
        let mut scene = test_scene_cb0_with_camera_and_fade(800.0, 0.0);
        scene.day_night_hour_sun_dir = [0.0, 0.0, 0.0, 1.0]; // sun dir = +z
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 1.0;
        let globe = |d: f32| Vec3 {
            x: 0.0,
            y: 0.0,
            z: d,
        };
        let f = |scene, d| day_night_factor(scene, globe(d), FEATHER_MIN, FEATHER_MAX);
        assert_eq!(f(scene, FEATHER_MIN), 0.0, "ramp floor at dot == vMin");
        assert_eq!(f(scene, FEATHER_MAX), 1.0, "ramp ceiling at dot == vMax");
        assert!((f(scene, 0.0) - 0.5).abs() < 1e-6, "ramp midpoint");
        assert_eq!(f(scene, -1.0), 0.0, "fully day side saturates to 0");
        assert_eq!(f(scene, 1.0), 1.0, "fully night side saturates to 1");
        // The gate scales the whole factor (fxh:361).
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 0.5;
        assert_eq!(f(scene, 1.0), 0.5, "FoW.w scales the factor");
        // The moon window constants are a same-width feather (fxh:326-327).
        assert_eq!(MOON_FEATHER_MIN, FEATHER_MIN);
        assert_eq!(MOON_FEATHER_MAX, FEATHER_MAX);
    }

    #[test]
    fn gate_nightify_and_blend_match_hand_computed_shader_reference() {
        // Hand-computed from the shipped HLSL (fxh:370-394) for
        // day = (0.6, 0.4, 0.2), blend = 1: Grey = 0.37 (fxh:375), night =
        // saturate(lerp(0.37, 0.37*(0.2,0.7,1.2), 0.25)) = (0.296, 0.34225,
        // 0.3885) (fxh:376), desaturation = 0.8 (fxh:373), color * 0.5
        // (NIGHT_DARKNESS, fxh:380) = (0.1784, 0.1769, 0.1754).
        let day = Vec3 {
            x: 0.6,
            y: 0.4,
            z: 0.2,
        };
        let n = nightify_color(day, 1.0);
        assert!((n.x - 0.1784).abs() < 2e-6, "nightify r {}", n.x);
        assert!((n.y - 0.1769).abs() < 2e-6, "nightify g {}", n.y);
        assert!((n.z - 0.1754).abs() < 2e-6, "nightify b {}", n.z);
        // blend = 0: no desaturation, but NIGHT_DARKNESS still halves (the
        // night look is never a plain passthrough: fxh:380 multiplies last).
        let n0 = nightify_color(day, 0.0);
        assert_eq!(n0.x.to_bits(), (day.x * NIGHT_DARKNESS).to_bits());
        assert_eq!(n0.y.to_bits(), (day.y * NIGHT_DARKNESS).to_bits());
        assert_eq!(n0.z.to_bits(), (day.z * NIGHT_DARKNESS).to_bits());
        // Full-gate full-night blend caps at NIGHT_OPACITY = 0.85 (fxh:393):
        // out = lerp(day, nightified, 1.0 * 0.85); hand-computed
        // = (0.24164, 0.210365, 0.17909).
        let mut scene = test_scene_cb0_with_camera_and_fade(800.0, 0.0);
        scene.day_night_hour_sun_dir = [0.0, 0.0, 0.0, 1.0];
        scene.v_fow_opacity_fow_time_snow_mud_fade_max_game_speed[3] = 1.0;
        let night_globe = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        let out = day_night_with_blend(scene, day, night_globe, 1.0);
        assert!((out.x - 0.24164).abs() < 2e-6, "blend r {}", out.x);
        assert!((out.y - 0.210365).abs() < 2e-6, "blend g {}", out.y);
        assert!((out.z - 0.17909).abs() < 2e-6, "blend b {}", out.z);
    }

    #[test]
    fn gate_calc_globe_normal_geometry() {
        // CalcGlobeNormal (fxh:347-356) on the real 5632x2048 map constants.
        let constants = MapFrameTerrainShaderConstants {
            map_size_x: 5632.0,
            map_size_y: 2048.0,
            map_pow2_x: 1.0,
            map_pow2_y: 1.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
        };
        let mut scene = test_scene_cb0_with_camera_and_fade(800.0, 0.0);
        scene.day_night_hour_sun_dir[0] = 0.0;
        let at = |scene, x: f32, z: f32| calc_globe_normal(scene, constants, Vec3 { x, y: 0.0, z });
        // Greenwich (world x == GMT_OFFSET == 2793) at the map's mid-latitude,
        // phase 0: longitude angle 0 -> normal (0, y, +z). Hand-computed:
        // y_param = 0.17 + 0.76*0.5 = 0.55, y = -cos(0.55*3.1415) = 0.15607,
        // normalized -> (0, 0.18227, 0.98325).
        let greenwich = at(scene, GMT_OFFSET, 1024.0);
        assert!(greenwich.x.abs() < 1e-6, "greenwich x {}", greenwich.x);
        assert!(
            (greenwich.y - 0.182268).abs() < 1e-5,
            "greenwich y {}",
            greenwich.y
        );
        assert!(
            (greenwich.z - 0.983249).abs() < 1e-5,
            "greenwich z {}",
            greenwich.z
        );
        // Unit length everywhere sampled.
        for (x, z) in [(0.0, 0.0), (2816.0, 1024.0), (5000.0, 2000.0)] {
            let g = at(scene, x, z);
            assert!((length_vec3(g) - 1.0).abs() < 1e-5, "unit at ({x},{z})");
        }
        // Half a map east = the antipodal longitude: x and z negate.
        let antipode = at(scene, GMT_OFFSET + 2816.0, 1024.0);
        assert!((antipode.z + greenwich.z).abs() < 1e-5, "antipodal z");
        // The shader's truncated turn constant (6.2831, fxh:354) makes
        // sin(half-turn) ~ 4.6e-5 rather than 0 -- port faithfully reproduces it.
        assert!(
            antipode.x.abs() < 1e-4,
            "antipodal x {} (sin(6.2831/2) scale)",
            antipode.x
        );
        // The hour phase rotates the terminator: phase 0.5 == +half-map shift.
        scene.day_night_hour_sun_dir[0] = 0.5;
        let phased = at(scene, GMT_OFFSET, 1024.0);
        assert!(
            (phased.z - antipode.z).abs() < 1e-5,
            "phase == longitude shift"
        );
        // South pole offset: the missing-globe clamp keeps y strictly inside
        // (-1, 1) at the map edges (SOUTH/NORTH_POLE_OFFSET, fxh:330-331).
        scene.day_night_hour_sun_dir[0] = 0.0;
        let south = at(scene, GMT_OFFSET, 0.0);
        let north = at(scene, GMT_OFFSET, 2047.0);
        assert!(south.y < 0.0 && south.y > -1.0, "south y {}", south.y);
        assert!(north.y > 0.0 && north.y < 1.0, "north y {}", north.y);
    }

    #[test]
    fn shader_source_vector3_constant_gate_fails_closed_on_fow_color_drift() {
        let root = temp_test_dir("shader_source_vector3_constant_gate");
        fs::create_dir_all(&root).expect("create temp dir");
        let source = root.join("fow.fxh");
        fs::write(
            &source,
            b"static const float3 BRIGHT_FOW_COLOR = float3(0.98, 0.93, 0.93);",
        )
        .expect("write fow source");
        validate_shader_source_vector3_constant(
            &source,
            "gfx/FX/fow.fxh",
            "BRIGHT_FOW_COLOR",
            BRIGHT_FOW_COLOR,
        )
        .expect("matching FOW color");

        fs::write(
            &source,
            b"static const float3 BRIGHT_FOW_COLOR = float3(0.99, 0.93, 0.93);",
        )
        .expect("write changed fow source");
        let err = validate_shader_source_vector3_constant(
            &source,
            "gfx/FX/fow.fxh",
            "BRIGHT_FOW_COLOR",
            BRIGHT_FOW_COLOR,
        )
        .expect_err("changed FOW color must fail closed");
        assert!(err.to_string().contains("BRIGHT_FOW_COLOR[0]"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn occupation_mask_shader_source_gate_rejects_formula_drift() {
        let root = temp_test_dir("occupation_mask_shader_source_gate");
        fs::create_dir_all(&root).expect("create temp dir");
        let source = root.join("standardfuncsgfx.fxh");
        fs::write(
            &source,
            b"
float CalculateOccupationMask( in float2 uv )
{
    float t = 3.14159 / 8.0;
    float w = SEC_MAP_TILE;
    float stripeVal = cos( ( uv.x * cos( t ) * w ) + ( uv.y * sin( t ) * w ) );
    float camDist = cam_distance( 300.0, 1200.0 );
    stripeVal += camDist * 1.5;
    stripeVal = smoothstep(0.0, 1.0, stripeVal*1.7) * lerp(1.0, 0.3, camDist);
    return stripeVal;
}
",
        )
        .expect("write standard functions source");
        validate_occupation_mask_shader_source(&source).expect("matching occupation law");

        fs::write(
            &source,
            b"
float CalculateOccupationMask( in float2 uv )
{
    float t = 3.14159 / 8.0;
    float w = SEC_MAP_TILE;
    float stripeVal = cos( ( uv.x * cos( t ) * w ) + ( uv.y * sin( t ) * w ) );
    float camDist = cam_distance( 300.0, 1000.0 );
    stripeVal += camDist * 1.5;
    stripeVal = smoothstep(0.0, 1.0, stripeVal*1.7) * lerp(1.0, 0.3, camDist);
    return stripeVal;
}
",
        )
        .expect("write changed standard functions source");
        let err = validate_occupation_mask_shader_source(&source)
            .expect_err("changed occupation camera fade must fail closed");
        assert!(err.to_string().contains("CalculateOccupationMask"));
        let _ = fs::remove_dir_all(&root);
    }
}
