use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
#[cfg(windows)]
use winapi::um::d3dcommon::{D3D11_SRV_DIMENSION_TEXTURE2D, D3D11_SRV_DIMENSION_TEXTURECUBE};

pub(crate) fn validate_production_terrain_renderer(bundle: &Path) -> Result<TerrainRendererInputs> {
    validate_production_terrain_renderer_impl(bundle, true)
}

pub(crate) fn validate_production_terrain_renderer_for_map_frame(
    bundle: &Path,
) -> Result<TerrainRendererInputs> {
    validate_production_terrain_renderer_impl(bundle, false)
}

pub(crate) fn validate_production_terrain_renderer_impl(
    bundle: &Path,
    require_legacy_runtime_draw_inputs: bool,
) -> Result<TerrainRendererInputs> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(bundle.join("manifest.json"))?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let contract: MapMaterialContract = serde_json::from_slice(&fs::read(
        bundle.join(&manifest.files.map_material_contract),
    )?)?;
    let (terrain_id_map, terrain_runtime, runtime) = if require_legacy_runtime_draw_inputs {
        let terrain_id_map = bundle.join(&manifest.files.terrain_id_map);
        if !terrain_id_map.is_file() {
            return Err(format!(
                "production terrain renderer blocked: missing {}. the game pdxmap.shader samples TerrainIDMap before atlas blending; no fallback from terrain.bmp is allowed.",
                manifest.files.terrain_id_map
            )
            .into());
        }
        let terrain_runtime = required_bundle_file(
            bundle,
            "terrain runtime input contract",
            &manifest.files.terrain_runtime,
        )?;
        let runtime: TerrainRuntimeInputs = serde_json::from_slice(&fs::read(&terrain_runtime)?)?;
        validate_terrain_runtime_inputs(&runtime, &manifest, bundle, &terrain_id_map)?;
        (Some(terrain_id_map), Some(terrain_runtime), Some(runtime))
    } else {
        (None, None, None)
    };
    if !contract
        .generated_runtime_inputs
        .contains_key("terrain_id_map")
    {
        return Err("production terrain renderer blocked: material contract does not classify TerrainIDMap provenance".into());
    }

    let inputs = TerrainRendererInputs {
        terrain_runtime,
        terrain_id_map,
        terrain_diffuse_atlas: required_shader_source_path(
            bundle,
            &manifest,
            "map/terrain/atlas0.dds",
        )?,
        terrain_normal_atlas: required_shader_source_path(
            bundle,
            &manifest,
            "map/terrain/atlas_normal0.dds",
        )?,
        terrain_diffuse_atlas_mips: required_bundle_file(
            bundle,
            "terrain diffuse atlas mip chain",
            &manifest.files.terrain_diffuse_atlas_mips,
        )?,
        terrain_normal_atlas_mips: required_bundle_file(
            bundle,
            "terrain normal atlas mip chain",
            &manifest.files.terrain_normal_atlas_mips,
        )?,
        terrain_color_tint: required_shader_source_path(
            bundle,
            &manifest,
            "map/terrain/colormap_rgb_cityemissivemask_a.dds",
        )?,
        height_normal: required_bundle_file(
            bundle,
            "height_normal",
            &manifest.files.height_normal,
        )?,
        constants_source: required_map_source_path(bundle, &manifest, "gfx/FX/constants.fxh")?,
        pdxmap_shader_source: required_map_source_path(bundle, &manifest, "gfx/FX/pdxmap.shader")?,
        pdxmap_include_source: required_map_source_path(bundle, &manifest, "gfx/FX/pdxmap.fxh")?,
        standard_functions_source: required_map_source_path(
            bundle,
            &manifest,
            "gfx/FX/standardfuncsgfx.fxh",
        )?,
        shadow_include_source: required_map_source_path(bundle, &manifest, "gfx/FX/shadow.fxh")?,
        fow_include_source: required_map_source_path(bundle, &manifest, "gfx/FX/fow.fxh")?,
        runtime,
    };
    validate_pdxmap_sampler_contract(&inputs.pdxmap_shader_source)?;
    validate_standard_function_shader_constant(
        &inputs.standard_functions_source,
        "NIGHT_OPACITY",
        NIGHT_OPACITY,
    )?;
    validate_standard_function_shader_constant(
        &inputs.standard_functions_source,
        "NIGHT_DARKNESS",
        NIGHT_DARKNESS,
    )?;
    validate_occupation_mask_shader_source(&inputs.standard_functions_source)?;
    validate_shader_source_vector3_constant(
        &inputs.fow_include_source,
        "gfx/FX/fow.fxh",
        "BRIGHT_FOW_COLOR",
        BRIGHT_FOW_COLOR,
    )?;
    validate_shader_source_vector3_constant(
        &inputs.fow_include_source,
        "gfx/FX/fow.fxh",
        "DARK_FOW_COLOR",
        DARK_FOW_COLOR,
    )?;
    if let Some(runtime) = &inputs.runtime {
        validate_runtime_texture_sampler_contract(runtime, &inputs.pdxmap_shader_source)?;
    }
    validate_material_mip_outputs(&contract, bundle)?;
    validate_shader_constant(&contract, "TERRAIN_TILE_FREQ", TERRAIN_TILE_FREQ)?;
    validate_shader_constant(&contract, "TEXELS_PER_TILE", TEXELS_PER_TILE)?;
    validate_shader_constant(&contract, "MAP_NUM_TILES", MAP_NUM_TILES as f32)?;
    validate_shader_constant(
        &contract,
        "ATLAS_TEXEL_POW2_EXPONENT",
        ATLAS_TEXEL_POW2_EXPONENT,
    )?;
    validate_shader_constant(
        &contract,
        "COLORMAP_OVERLAY_STRENGTH",
        COLORMAP_OVERLAY_STRENGTH,
    )?;
    validate_shader_constant(
        &contract,
        "COLORMAP_MUD_OVERLAY_STRENGTH",
        COLORMAP_MUD_OVERLAY_STRENGTH,
    )?;
    validate_shader_constant(&contract, "MUD_TILING", MUD_TILING)?;
    validate_shader_constant(&contract, "MUD_NORMAL_CUTOFF", MUD_NORMAL_CUTOFF)?;
    validate_shader_constant(&contract, "MUD_STRENGHTEN", MUD_STRENGHTEN)?;
    validate_shader_constant(&contract, "MUD_CAM_MIN", MUD_CAM_MIN)?;
    validate_shader_constant(&contract, "MUD_CAM_MAX", MUD_CAM_MAX)?;
    validate_shader_constant(&contract, "SNOW_OPACITY_MIN", SNOW_OPACITY_MIN)?;
    validate_shader_constant(&contract, "SNOW_OPACITY_MAX", SNOW_OPACITY_MAX)?;
    validate_shader_constant(&contract, "SNOW_CAM_MIN", SNOW_CAM_MIN)?;
    validate_shader_constant(&contract, "SNOW_CAM_MAX", SNOW_CAM_MAX)?;
    validate_shader_constant(&contract, "SNOW_START_HEIGHT", SNOW_START_HEIGHT)?;
    validate_shader_constant(
        &contract,
        "SNOW_RIDGE_START_HEIGHT",
        SNOW_RIDGE_START_HEIGHT,
    )?;
    validate_shader_constant(&contract, "SNOW_NORMAL_START", SNOW_NORMAL_START)?;
    validate_shader_constant(&contract, "SNOW_CLIFFS", SNOW_CLIFFS)?;
    validate_shader_constant(&contract, "SNOW_SPEC_GLOSS_MULT", SNOW_SPEC_GLOSS_MULT)?;
    validate_shader_constant(&contract, "SNOW_TILING", SNOW_TILING)?;
    validate_shader_constant(&contract, "SNOW_NOISE_TILING", SNOW_NOISE_TILING)?;
    validate_shader_constant(&contract, "SNOW_FROST_MIN_EFFECT", SNOW_FROST_MIN_EFFECT)?;
    validate_shader_constant(&contract, "CITY_LIGHTS_TILING", CITY_LIGHTS_TILING)?;
    validate_shader_constant(&contract, "CITY_LIGHTS_INTENSITY", CITY_LIGHTS_INTENSITY)?;
    validate_shader_constant(&contract, "FOG_BEGIN", FOG_BEGIN)?;
    validate_shader_constant(&contract, "FOG_END", FOG_END)?;
    validate_shader_constant(&contract, "FOG_MAX", FOG_MAX)?;
    validate_shader_constant(&contract, "GB_CAM_MIN", GB_CAM_MIN)?;
    validate_shader_constant(&contract, "GB_CAM_MAX", GB_CAM_MAX)?;
    validate_shader_constant(
        &contract,
        "GB_CAM_MAX_FILLING_CLAMP",
        GB_CAM_MAX_FILLING_CLAMP,
    )?;
    validate_shader_constant(&contract, "GB_THRESHOLD", GB_THRESHOLD)?;
    validate_shader_constant(&contract, "GB_THRESHOLD2", GB_THRESHOLD2)?;
    validate_shader_constant(&contract, "GB_OPACITY_NEAR", GB_OPACITY_NEAR)?;
    validate_shader_constant(&contract, "GB_OPACITY_FAR", GB_OPACITY_FAR)?;
    validate_shader_constant(&contract, "GB_STRENGTH_CH1", GB_STRENGTH_CH1)?;
    validate_shader_constant(&contract, "GB_STRENGTH_CH2", GB_STRENGTH_CH2)?;
    validate_shader_constant(
        &contract,
        "GB_FIRST_LAYER_PRIORITY",
        GB_FIRST_LAYER_PRIORITY,
    )?;
    validate_shader_constant(&contract, "SEC_MAP_TILE", SEC_MAP_TILE)?;
    validate_shader_constant(
        &contract,
        "BORDER_NIGHT_DESATURATION_MAX",
        BORDER_NIGHT_DESATURATION_MAX,
    )?;
    validate_shader_constant(
        &contract,
        "BORDER_FOW_REMOVAL_FACTOR",
        BORDER_FOW_REMOVAL_FACTOR,
    )?;
    validate_shader_constant(
        &contract,
        "BORDER_LIGHT_REMOVAL_FACTOR",
        BORDER_LIGHT_REMOVAL_FACTOR,
    )?;
    validate_shader_vector3_constant(&contract, "SNOW_COLOR", SNOW_COLOR)?;
    validate_shader_vector3_constant(&contract, "FOG_COLOR", FOG_COLOR)?;
    if require_legacy_runtime_draw_inputs {
        let runtime = inputs
            .runtime
            .as_ref()
            .ok_or("production terrain renderer blocked: missing legacy terrain runtime inputs")?;
        validate_required_runtime_renderer_inputs(runtime)?;
    }
    Ok(inputs)
}

pub(crate) fn validate_map_frame_runtime_bundle(path: &Path) -> Result<MapFrameRuntimeBundle> {
    validate_map_frame_runtime_bundle_for(path, TERRAIN_MAP_FRAME_SPEC)
}

#[cfg(test)]
pub(crate) fn validate_map_frame_runtime_bundle_with_resolver(
    path: &Path,
    resolver: &GameFileResolver,
) -> Result<MapFrameRuntimeBundle> {
    validate_map_frame_runtime_bundle_for_with_resolver(
        path,
        TERRAIN_MAP_FRAME_SPEC,
        Some(resolver),
    )
}

pub fn validate_water_map_frame_bundle(path: &Path) -> Result<()> {
    validate_map_frame_runtime_bundle_for(path, WATER_MAP_FRAME_SPEC).map(|_| ())
}

pub fn validate_rivers_map_frame_bundle(path: &Path) -> Result<()> {
    validate_map_frame_runtime_bundle_for(path, RIVERS_MAP_FRAME_SPEC).map(|_| ())
}

pub fn validate_borders_map_frame_bundle(path: &Path) -> Result<()> {
    validate_map_frame_runtime_bundle_for(path, BORDERS_MAP_FRAME_SPEC).map(|_| ())
}

pub fn validate_trees_map_frame_bundle(path: &Path) -> Result<()> {
    validate_map_frame_runtime_bundle_for(path, TREES_MAP_FRAME_SPEC).map(|_| ())
}

pub(crate) fn validate_map_frame_runtime_bundle_for(
    path: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<MapFrameRuntimeBundle> {
    validate_map_frame_runtime_bundle_for_with_resolver(path, spec, None)
}

pub(crate) fn validate_map_frame_runtime_bundle_for_with_resolver(
    path: &Path,
    spec: MapFrameBundleValidationSpec,
    resolver: Option<&GameFileResolver>,
) -> Result<MapFrameRuntimeBundle> {
    let manifest_path = map_frame_bundle_manifest_path_for(path, spec);
    let bundle_dir = map_frame_bundle_dir_for(path, spec)?;
    if !manifest_path.exists() {
        return Err(
            map_capture_session_blocker_error_for(&bundle_dir, spec).unwrap_or_else(|| {
                format!(
                    "{} blocked: map-frame bundle {} is missing",
                    spec.renderer_label,
                    manifest_path.display()
                )
                .into()
            }),
        );
    }
    let mut frame: MapFrameRuntimeBundle = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if frame.schema != MAP_FRAME_BUNDLE_SCHEMA {
        return Err(format!(
            "{} blocked: unsupported map-frame bundle schema {}",
            spec.renderer_label, frame.schema
        )
        .into());
    }
    populate_descriptor_only_runtime_shader_resources(&mut frame, &bundle_dir, resolver)?;
    // TerrainIDMap is a runtime engine product. Resolver files can validate
    // source identity but cannot replace an absent authenticated payload: the
    // former file synthesis matched only a capture-specific subset and could
    // silently manufacture a product SRV. Missing bytes remain an extraction
    // blocker even when a VFS resolver is available.
    let extraction_blockers = effective_map_frame_extraction_blockers(&frame, spec);
    if !extraction_blockers.is_empty() {
        return Err(format!(
            "{} blocked: map-frame extraction reported blocker(s): {}",
            spec.renderer_label,
            extraction_blockers.join("; ")
        )
        .into());
    }
    validate_map_capture_session_manifest_for(&frame, &bundle_dir, spec)?;
    if frame.draws.len() < spec.min_draws {
        return Err(format!(
            "{} blocked: map-frame bundle has {} draw record(s), expected at least {} for {}",
            spec.renderer_label,
            frame.draws.len(),
            spec.min_draws,
            spec.capture_moment
        )
        .into());
    }
    if !frame.draws.iter().any(|draw| {
        draw.draw_index == frame.active_draw.draw_index
            && draw.draw_call == frame.active_draw.draw_call
            && draw.draw_count == frame.active_draw.draw_count
            && draw.draw_start == frame.active_draw.draw_start
    }) {
        return Err(format!(
            "{} blocked: active_draw is not present in map-frame draws sequence",
            spec.renderer_label
        )
        .into());
    }
    if spec.terrain_strict {
        validate_map_frame_shader_cache_constants_evidence(&frame)?;
    }
    validate_map_frame_vanilla_sources(&frame, &bundle_dir, spec, resolver)?;
    for draw in &frame.draws {
        validate_map_frame_draw(draw, &frame, &bundle_dir, spec)?;
    }
    validate_map_frame_resources(&frame, &bundle_dir, spec)?;
    Ok(frame)
}

fn substitute_synthesized_terrain_id_map(
    frame: &mut MapFrameRuntimeBundle,
    bundle_dir: &Path,
    resolver: &GameFileResolver,
) -> Result<()> {
    let started = Instant::now();
    let synthesized = synthesize_terrain_id_map(resolver)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut saw_captured_payload = false;
    let mut substituted = 0usize;

    substitute_synthesized_terrain_id_map_for_draw(
        &mut frame.active_draw,
        bundle_dir,
        &frame.runtime_files,
        &synthesized,
        &mut saw_captured_payload,
        &mut substituted,
    )?;
    for draw in &mut frame.draws {
        substitute_synthesized_terrain_id_map_for_draw(
            draw,
            bundle_dir,
            &frame.runtime_files,
            &synthesized,
            &mut saw_captured_payload,
            &mut substituted,
        )?;
    }
    if substituted == 0 {
        return Err(
            "production terrain renderer blocked: resolver synthesized TerrainIDMap, but the active shader reflection does not expose a TerrainIDMap SRV slot"
                .into(),
        );
    }

    let provenance = format!(
        "synthesized TerrainIDMap from resolver {}",
        resolver.selection_source()
    );
    frame.runtime_files.insert(
        SYNTHESIZED_TERRAIN_ID_MAP_FILE.to_string(),
        MapFrameFileEvidence {
            file: SYNTHESIZED_TERRAIN_ID_MAP_FILE.to_string(),
            byte_len: synthesized.bytes.len(),
            fnv1a64: synthesized.descriptor.fnv1a64.clone(),
            provenance: provenance.clone(),
        },
    );
    frame.synthetic_payloads.insert(
        SYNTHESIZED_TERRAIN_ID_MAP_FILE.to_string(),
        MapFrameSyntheticPayload {
            bytes: synthesized.bytes.clone(),
            fnv1a64: synthesized.descriptor.fnv1a64.clone(),
            provenance,
        },
    );

    if saw_captured_payload {
        eprintln!(
            "info: synthesized TerrainIDMap {}x{} fnv1a64 {} matched captured payload; using synthesized bytes for {} binding(s), elapsed_ms {:.2}",
            synthesized.width,
            synthesized.height,
            synthesized.descriptor.fnv1a64,
            substituted,
            elapsed_ms
        );
    } else {
        eprintln!(
            "info: synthesized TerrainIDMap {}x{} fnv1a64 {} supplied missing captured payload for {} binding(s), elapsed_ms {:.2}",
            synthesized.width,
            synthesized.height,
            synthesized.descriptor.fnv1a64,
            substituted,
            elapsed_ms
        );
    }
    Ok(())
}

fn substitute_synthesized_terrain_id_map_for_draw(
    draw: &mut MapFrameDraw,
    bundle_dir: &Path,
    runtime_files: &BTreeMap<String, MapFrameFileEvidence>,
    synthesized: &SynthesizedTerrainIdMap,
    saw_captured_payload: &mut bool,
    substituted: &mut usize,
) -> Result<()> {
    let Some(slot) = draw
        .shader
        .reflection_resources
        .get(TERRAIN_ID_RESOURCE_SYMBOL)
        .copied()
    else {
        return Ok(());
    };
    let resource_index = draw
        .shader_resources
        .iter()
        .position(|resource| resource.stage == "PS" && resource.slot == slot);
    match resource_index {
        Some(index) => {
            let resource = draw.shader_resources[index].clone();
            validate_terrain_id_map_descriptor_compatible(&resource, synthesized)?;
            if terrain_id_map_resource_has_captured_payload(&resource) {
                *saw_captured_payload = true;
                verify_captured_terrain_id_map_matches_synthesized(
                    bundle_dir,
                    runtime_files,
                    synthesized,
                )?;
            }
            draw.shader_resources[index] =
                synthesized_terrain_id_map_resource(slot, Some(resource), synthesized)?;
        }
        None => {
            draw.shader_resources
                .push(synthesized_terrain_id_map_resource(
                    slot,
                    None,
                    synthesized,
                )?);
        }
    }
    *substituted += 1;
    Ok(())
}

fn terrain_id_map_resource_has_captured_payload(resource: &MapFrameTextureBinding) -> bool {
    resource
        .file
        .as_deref()
        .map(|file| !is_synthetic_map_frame_file(file))
        .unwrap_or(false)
        && resource.byte_len > 0
        && resource.fnv1a64.is_some()
}

pub(crate) fn is_synthetic_map_frame_file(file: &str) -> bool {
    file == SYNTHESIZED_TERRAIN_ID_MAP_FILE || file.starts_with("resolver_static_texture_")
}

fn verify_captured_terrain_id_map_matches_synthesized(
    bundle_dir: &Path,
    runtime_files: &BTreeMap<String, MapFrameFileEvidence>,
    synthesized: &SynthesizedTerrainIdMap,
) -> Result<()> {
    let evidence = runtime_files.get(CANONICAL_TERRAIN_ID_MAP_FILE).ok_or(
        "production terrain renderer blocked: captured TerrainIDMap canonical payload evidence is missing",
    )?;
    let path = bundle_dir.join(&evidence.file);
    let captured = fs::read(&path).map_err(|error| {
        format!(
            "production terrain renderer blocked: captured TerrainIDMap {} is not readable: {}",
            path.display(),
            error
        )
    })?;
    if captured.len() != evidence.byte_len {
        return Err(format!(
            "production terrain renderer blocked: captured TerrainIDMap {} has {} bytes, canonical evidence expects {}",
            path.display(),
            captured.len(),
            evidence.byte_len
        )
        .into());
    }
    if captured.len() != synthesized.bytes.len() {
        return Err(format!(
            "production terrain renderer blocked: captured TerrainIDMap {} has {} bytes, synthesized resolver TerrainIDMap has {} bytes",
            path.display(),
            captured.len(),
            synthesized.bytes.len()
        )
        .into());
    }
    let captured_hash = terrain_id_map_payload_fnv1a64(&captured);
    if captured_hash != evidence.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: captured TerrainIDMap {} hash changed from {} to {}",
            path.display(),
            evidence.fnv1a64,
            captured_hash
        )
        .into());
    }
    if captured_hash != synthesized.descriptor.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: captured TerrainIDMap hash {} does not match resolver-synthesized hash {}; likely cause: captured bundle modset differs from resolver modset, so replay refuses to mix modded capture evidence with current resolved installed content",
            captured_hash,
            synthesized.descriptor.fnv1a64
        )
        .into());
    }
    Ok(())
}

fn validate_terrain_id_map_descriptor_compatible(
    resource: &MapFrameTextureBinding,
    synthesized: &SynthesizedTerrainIdMap,
) -> Result<()> {
    if resource.is_null {
        return Err(
            "production terrain renderer blocked: TerrainIDMap SRV is explicitly null".into(),
        );
    }
    if resource.stage != "PS" {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap SRV stage is {}, expected PS",
            resource.stage
        )
        .into());
    }
    if resource.declared_name.as_deref() != Some(TERRAIN_ID_RESOURCE_SYMBOL) {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap SRV slot {} is declared as {:?}",
            resource.slot, resource.declared_name
        )
        .into());
    }
    if resource.width != synthesized.width as u32 || resource.height != synthesized.height as u32 {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor is {}x{}, synthesized resolver TerrainIDMap is {}x{}",
            resource.width, resource.height, synthesized.width, synthesized.height
        )
        .into());
    }
    if resource.byte_len != 0 && resource.byte_len != synthesized.bytes.len() {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor byte_len {} does not match synthesized byte_len {}",
            resource.byte_len,
            synthesized.bytes.len()
        )
        .into());
    }
    if resource.dxgi_format != TERRAIN_ID_MAP_DXGI_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor DXGI format {} does not match expected {}",
            resource.dxgi_format, TERRAIN_ID_MAP_DXGI_FORMAT
        )
        .into());
    }
    if resource.mip_count != TERRAIN_ID_MAP_MIP_COUNT {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor mip count {} does not match expected {}",
            resource.mip_count, TERRAIN_ID_MAP_MIP_COUNT
        )
        .into());
    }
    if resource.array_size != 0 && resource.array_size != 1 {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor array size {} does not match Texture2D",
            resource.array_size
        )
        .into());
    }
    if resource.sample_count != 0 && resource.sample_count != 1 {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor sample count {} is not a single-sample texture",
            resource.sample_count
        )
        .into());
    }
    if resource.bind_flags != 0 && resource.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap descriptor bind_flags {} do not include D3D11_BIND_SHADER_RESOURCE",
            resource.bind_flags
        )
        .into());
    }
    if let Some(view) = resource.view_desc.as_ref() {
        if view.view_dimension != D3D11_SRV_DIMENSION_TEXTURE2D
            || view.most_detailed_mip != 0
            || view.mip_levels != 1
            || view.first_array_slice != 0
            || view.array_size > 1
            || view.num_cubes != 0
        {
            return Err(format!(
                "production terrain renderer blocked: TerrainIDMap SRV view descriptor is not a single Texture2D mip: {:?}",
                view
            )
            .into());
        }
    }
    if !resource.subresources.is_empty() {
        if resource.subresources.len() != 1 {
            return Err(format!(
                "production terrain renderer blocked: TerrainIDMap has {} subresources, expected one tightly packed level",
                resource.subresources.len()
            )
            .into());
        }
        let subresource = &resource.subresources[0];
        if subresource.file_offset != 0
            || subresource.byte_len != synthesized.bytes.len()
            || subresource.row_pitch != synthesized.descriptor.row_pitch as u32
            || subresource.depth_pitch != synthesized.bytes.len() as u32
            || subresource.width != synthesized.width as u32
            || subresource.height != synthesized.height as u32
        {
            return Err(format!(
                "production terrain renderer blocked: TerrainIDMap subresource layout is not tightly packed row_pitch {} depth_pitch {} byte_len {}; synthesized row_pitch {} byte_len {}",
                subresource.row_pitch,
                subresource.depth_pitch,
                subresource.byte_len,
                synthesized.descriptor.row_pitch,
                synthesized.bytes.len()
            )
            .into());
        }
    }
    Ok(())
}

fn synthesized_terrain_id_map_resource(
    slot: u32,
    existing: Option<MapFrameTextureBinding>,
    synthesized: &SynthesizedTerrainIdMap,
) -> Result<MapFrameTextureBinding> {
    let view_desc = existing
        .as_ref()
        .and_then(|resource| resource.view_desc.clone())
        .or_else(|| {
            Some(MapFrameShaderResourceViewDesc {
                format: TERRAIN_ID_MAP_DXGI_FORMAT,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 1,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            })
        });
    Ok(MapFrameTextureBinding {
        stage: "PS".to_string(),
        slot,
        declared_name: Some(TERRAIN_ID_RESOURCE_SYMBOL.to_string()),
        is_null: false,
        view_desc,
        file: Some(SYNTHESIZED_TERRAIN_ID_MAP_FILE.to_string()),
        byte_len: synthesized.bytes.len(),
        fnv1a64: Some(synthesized.descriptor.fnv1a64.clone()),
        dxgi_format: TERRAIN_ID_MAP_DXGI_FORMAT,
        width: synthesized.width as u32,
        height: synthesized.height as u32,
        mip_count: TERRAIN_ID_MAP_MIP_COUNT,
        array_size: 1,
        sample_count: 1,
        sample_quality: 0,
        payload_format: None,
        subresources: Vec::new(),
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        usage: D3D11_USAGE_DEFAULT,
        cpu_access_flags: 0,
        misc_flags: 0,
    })
}

fn populate_descriptor_only_runtime_shader_resources(
    frame: &mut MapFrameRuntimeBundle,
    bundle_dir: &Path,
    resolver: Option<&GameFileResolver>,
) -> Result<()> {
    let fallbacks = terrain_chunk_library_shader_resource_fallbacks(bundle_dir)?;
    populate_descriptor_only_runtime_shader_resources_for_draw(
        &mut frame.active_draw,
        &mut frame.runtime_files,
        &mut frame.synthetic_payloads,
        &fallbacks,
        resolver,
    )?;
    for draw in &mut frame.draws {
        populate_descriptor_only_runtime_shader_resources_for_draw(
            draw,
            &mut frame.runtime_files,
            &mut frame.synthetic_payloads,
            &fallbacks,
            resolver,
        )?;
    }
    Ok(())
}

fn populate_descriptor_only_runtime_shader_resources_for_draw(
    draw: &mut MapFrameDraw,
    runtime_files: &mut BTreeMap<String, MapFrameFileEvidence>,
    synthetic_payloads: &mut BTreeMap<String, MapFrameSyntheticPayload>,
    fallbacks: &BTreeMap<String, Vec<(MapFrameTextureBinding, MapFrameFileEvidence)>>,
    resolver: Option<&GameFileResolver>,
) -> Result<()> {
    for resource in &mut draw.shader_resources {
        let Some(name) = resource.declared_name.as_deref() else {
            continue;
        };
        if !is_descriptor_only_shader_resource(resource) {
            continue;
        }
        let compatible_fallbacks = fallbacks
            .get(name)
            .into_iter()
            .flatten()
            .filter(|(fallback, _)| texture_descriptors_match(resource, fallback))
            .collect::<Vec<_>>();
        if compatible_fallbacks.len() > 1 {
            return Err(format!(
                "production terrain renderer blocked: descriptor-only shader resource {name} at PS slot {} has {} descriptor-compatible terrain chunk payloads; exact source is ambiguous",
                resource.slot,
                compatible_fallbacks.len()
            )
            .into());
        }
        if let Some((fallback, evidence)) = compatible_fallbacks.first().copied() {
            resource.file = fallback.file.clone();
            resource.byte_len = fallback.byte_len;
            resource.fnv1a64 = fallback.fnv1a64.clone();
            resource.payload_format = fallback.payload_format.clone();
            resource.subresources = fallback.subresources.clone();
            if let Some(file) = resource.file.as_ref() {
                runtime_files
                    .entry(file.clone())
                    .or_insert_with(|| evidence.clone());
            }
            continue;
        }
        if let Some(resolver) = resolver {
            if let Some(resolved) = resolve_descriptor_only_static_texture(resolver, resource)? {
                resource.file = Some(resolved.file.clone());
                resource.byte_len = resolved.bytes.len();
                resource.fnv1a64 = Some(resolved.fnv1a64.clone());
                resource.payload_format = Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string());
                resource.subresources = resolved.subresources.clone();
                runtime_files
                    .entry(resolved.file.clone())
                    .or_insert_with(|| MapFrameFileEvidence {
                        file: resolved.file.clone(),
                        byte_len: resolved.bytes.len(),
                        fnv1a64: resolved.fnv1a64.clone(),
                        provenance: resolved.provenance.clone(),
                    });
                synthetic_payloads
                    .entry(resolved.file.clone())
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: resolved.bytes,
                        fnv1a64: resolved.fnv1a64,
                        provenance: resolved.provenance,
                    });
                continue;
            }
        }
        if let Some(file) = canonical_runtime_shader_resource_file(name) {
            if let Some(evidence) = runtime_files.get(file) {
                resource.file = Some(evidence.file.clone());
                resource.byte_len = evidence.byte_len;
                resource.fnv1a64 = Some(evidence.fnv1a64.clone());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct ResolvedStaticTexturePayload {
    file: String,
    bytes: Vec<u8>,
    fnv1a64: String,
    subresources: Vec<MapFrameTextureSubresource>,
    provenance: String,
}

fn static_texture_source_candidates(name: &str) -> Option<&'static [&'static str]> {
    match name {
        TERRAIN_DIFFUSE_SYMBOL => Some(&["map/terrain/atlas0.dds"]),
        "HeightNormal" => Some(&["map/world_normal.bmp"]),
        TERRAIN_COLOR_TINT_SYMBOL => Some(&["map/terrain/colormap_rgb_cityemissivemask_a.dds"]),
        SNOW_TEXTURE_SYMBOL => Some(&["map/terrain/snow_normal_rgb_diffuse_a.dds"]),
        TERRAIN_NORMAL_SYMBOL => Some(&["map/terrain/atlas_normal0.dds"]),
        CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL => Some(&[
            "map/terrain/citylights_rgb_snowmask_a_0.dds",
            "map/terrain/citylights_rgb_snowmask_a_1.dds",
            "map/terrain/citylights_rgb_snowmask_a_2.dds",
        ]),
        MUD_NORMAL_SPEC_SYMBOL => Some(&[
            "map/terrain/mud_normal_rgb_spec_a_0.dds",
            "map/terrain/mud_normal_rgb_spec_a_1.dds",
            "map/terrain/mud_normal_rgb_spec_a_2.dds",
        ]),
        MUD_DIFFUSE_GLOSS_SYMBOL => Some(&[
            "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
            "map/terrain/mud_diffuse_rgb_gloss_a_1.dds",
            "map/terrain/mud_diffuse_rgb_gloss_a_2.dds",
        ]),
        _ => None,
    }
}

fn resolve_descriptor_only_static_texture(
    resolver: &GameFileResolver,
    resource: &MapFrameTextureBinding,
) -> Result<Option<ResolvedStaticTexturePayload>> {
    let Some(name) = resource.declared_name.as_deref() else {
        return Ok(None);
    };
    let Some(candidates) = static_texture_source_candidates(name) else {
        return Ok(None);
    };
    let mut matches = Vec::new();
    let mut rejected = Vec::new();
    for source in candidates {
        let Some(path) = resolver.resolve(source)? else {
            rejected.push(format!("{source}: not resolved"));
            continue;
        };
        match static_texture_payload_from_source(name, source, &path, resource) {
            Ok(payload) => matches.push(payload),
            Err(error) => rejected.push(format!("{source}: {error}")),
        }
    }
    match matches.len() {
        1 => Ok(matches.pop()),
        0 => Err(format!(
            "production terrain renderer blocked: descriptor-only shader resource {name} at PS slot {} has no exact resolver source matching captured descriptor {}x{} DXGI {} mip_count {}: {}",
            resource.slot,
            resource.width,
            resource.height,
            resource.dxgi_format,
            resource.mip_count,
            rejected.join("; ")
        )
        .into()),
        count => Err(format!(
            "production terrain renderer blocked: descriptor-only shader resource {name} at PS slot {} has {count} exact resolver sources matching the captured descriptor; source admission is ambiguous",
            resource.slot
        )
        .into()),
    }
}

fn static_texture_payload_from_source(
    name: &str,
    source: &str,
    path: &Path,
    resource: &MapFrameTextureBinding,
) -> Result<ResolvedStaticTexturePayload> {
    validate_static_texture_capture_descriptor(name, resource)?;
    let (bytes, subresources) = if source.ends_with(".bmp") {
        static_bmp_texture_payload(path, resource)?
    } else {
        static_dds_texture_payload(path, resource)?
    };
    let fnv1a64 = fnv1a64_bytes(&bytes);
    Ok(ResolvedStaticTexturePayload {
        file: format!(
            "resolver_static_texture_{}.d3d11",
            sanitize_source_name(source)
        ),
        bytes,
        fnv1a64,
        subresources,
        provenance: format!(
            "exact descriptor/source/mip/format join for {name} from resolver {} source {source}",
            path.display()
        ),
    })
}

fn validate_static_texture_capture_descriptor(
    name: &str,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.stage != "PS"
        || resource.is_null
        || resource.width == 0
        || resource.height == 0
        || resource.mip_count == 0
        || resource.array_size != 1
        || resource.sample_count != 1
        || resource.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0
    {
        return Err(format!(
            "captured {name} descriptor is not a non-null, single-sample Texture2D shader resource"
        )
        .into());
    }
    let view = resource
        .view_desc
        .as_ref()
        .ok_or_else(|| format!("captured {name} descriptor has no SRV view descriptor"))?;
    if view.format != resource.dxgi_format
        || view.view_dimension != D3D11_SRV_DIMENSION_TEXTURE2D
        || view.most_detailed_mip != 0
        || view.mip_levels != resource.mip_count
        || view.first_array_slice != 0
        || view.array_size > 1
        || view.num_cubes != 0
    {
        return Err(format!(
            "captured {name} SRV view does not expose the complete single Texture2D mip chain"
        )
        .into());
    }
    Ok(())
}

fn static_dds_texture_payload(
    path: &Path,
    resource: &MapFrameTextureBinding,
) -> Result<(Vec<u8>, Vec<MapFrameTextureSubresource>)> {
    let data = fs::read(path)?;
    if data.len() < 128 || data.get(0..4) != Some(b"DDS ") {
        return Err(format!("{} is not a complete legacy DDS", path.display()).into());
    }
    let height = u32::from_le_bytes(data[12..16].try_into()?);
    let width = u32::from_le_bytes(data[16..20].try_into()?);
    let mip_count = u32::from_le_bytes(data[28..32].try_into()?).max(1);
    if width != resource.width || height != resource.height || mip_count != resource.mip_count {
        return Err(format!(
            "DDS descriptor is {width}x{height} mip_count {mip_count}, captured descriptor is {}x{} mip_count {}",
            resource.width, resource.height, resource.mip_count
        )
        .into());
    }
    let fourcc = &data[84..88];
    let rgb_bits = u32::from_le_bytes(data[88..92].try_into()?) as usize;
    let rmask = u32::from_le_bytes(data[92..96].try_into()?);
    let gmask = u32::from_le_bytes(data[96..100].try_into()?);
    let bmask = u32::from_le_bytes(data[100..104].try_into()?);
    let amask = u32::from_le_bytes(data[104..108].try_into()?);
    let format_matches = match resource.dxgi_format {
        DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT_BC1_UNORM_SRGB => fourcc == b"DXT1",
        DXGI_FORMAT_BC3_UNORM | DXGI_FORMAT_BC3_UNORM_SRGB => fourcc == b"DXT5",
        DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => {
            fourcc.iter().all(|byte| *byte == 0)
                && rgb_bits == 32
                && rmask == 0x00ff0000
                && gmask == 0x0000ff00
                && bmask == 0x000000ff
                && amask == 0xff000000
        }
        DXGI_FORMAT_B8G8R8X8_UNORM => {
            fourcc.iter().all(|byte| *byte == 0)
                && rgb_bits == 32
                && rmask == 0x00ff0000
                && gmask == 0x0000ff00
                && bmask == 0x000000ff
                && amask == 0
        }
        _ => false,
    };
    if !format_matches {
        return Err(format!(
            "DDS encoding {:?}/RGB{} masks {:08x}/{:08x}/{:08x}/{:08x} does not admit captured DXGI {}",
            String::from_utf8_lossy(fourcc),
            rgb_bits,
            rmask,
            gmask,
            bmask,
            amask,
            resource.dxgi_format
        )
        .into());
    }
    const CAPTURED_STAGING_ROW_ALIGNMENT: u32 = 128;
    let mut bytes = Vec::new();
    let mut file_offset = 0usize;
    let mut source_offset = 128usize;
    let mut subresources = Vec::with_capacity(mip_count as usize);
    for mip_slice in 0..mip_count {
        let mip_width = (width >> mip_slice).max(1);
        let mip_height = (height >> mip_slice).max(1);
        let source_byte_len =
            dds_mip_byte_len(mip_width as usize, mip_height as usize, fourcc, rgb_bits)?;
        let source_row_pitch = native_d3d11_minimum_row_pitch(resource.dxgi_format, mip_width)?;
        let row_pitch = source_row_pitch
            .checked_add(CAPTURED_STAGING_ROW_ALIGNMENT - 1)
            .ok_or("DDS captured row pitch overflow")?
            / CAPTURED_STAGING_ROW_ALIGNMENT
            * CAPTURED_STAGING_ROW_ALIGNMENT;
        let row_count = native_d3d11_format_row_count(resource.dxgi_format, mip_height)?;
        let depth_pitch = row_pitch
            .checked_mul(row_count)
            .ok_or("DDS subresource depth pitch overflow")?;
        if source_byte_len != (source_row_pitch as usize) * (row_count as usize) {
            return Err(format!(
                "DDS mip {mip_slice} source byte_len {source_byte_len} does not equal tight D3D11 row layout"
            )
            .into());
        }
        let source_end = source_offset
            .checked_add(source_byte_len)
            .ok_or("DDS source range overflow")?;
        if source_end > data.len() {
            return Err(format!("{} is truncated at mip {mip_slice}", path.display()).into());
        }
        for row in 0..row_count as usize {
            let row_start = source_offset + row * source_row_pitch as usize;
            let row_end = row_start + source_row_pitch as usize;
            bytes.extend_from_slice(&data[row_start..row_end]);
            bytes.resize(bytes.len() + (row_pitch - source_row_pitch) as usize, 0);
        }
        source_offset = source_end;
        subresources.push(MapFrameTextureSubresource {
            mip_slice,
            array_slice: 0,
            file_offset,
            byte_len: depth_pitch as usize,
            row_pitch,
            depth_pitch,
            width: mip_width,
            height: mip_height,
        });
        file_offset += depth_pitch as usize;
    }
    if source_offset != data.len() {
        return Err(format!(
            "{} contains {} trailing byte(s) outside the declared mip chain",
            path.display(),
            data.len() - source_offset
        )
        .into());
    }
    Ok((bytes, subresources))
}

fn static_bmp_texture_payload(
    path: &Path,
    resource: &MapFrameTextureBinding,
) -> Result<(Vec<u8>, Vec<MapFrameTextureSubresource>)> {
    if resource.dxgi_format != DXGI_FORMAT_B8G8R8X8_UNORM || resource.mip_count != 1 {
        return Err(format!(
            "BMP source only admits captured DXGI {} with one mip, got DXGI {} mip_count {}",
            DXGI_FORMAT_B8G8R8X8_UNORM, resource.dxgi_format, resource.mip_count
        )
        .into());
    }
    let image = load_bmp_rgbx(path)?;
    if image.width as u32 != resource.width || image.height as u32 != resource.height {
        return Err(format!(
            "BMP descriptor is {}x{}, captured descriptor is {}x{}",
            image.width, image.height, resource.width, resource.height
        )
        .into());
    }
    let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in image.pixels {
        bytes.extend_from_slice(&pixel.to_le_bytes());
    }
    let row_pitch = resource
        .width
        .checked_mul(4)
        .ok_or("BMP BGRX row pitch overflow")?;
    let depth_pitch = row_pitch
        .checked_mul(resource.height)
        .ok_or("BMP BGRX depth pitch overflow")?;
    Ok((
        bytes,
        vec![MapFrameTextureSubresource {
            mip_slice: 0,
            array_slice: 0,
            file_offset: 0,
            byte_len: depth_pitch as usize,
            row_pitch,
            depth_pitch,
            width: resource.width,
            height: resource.height,
        }],
    ))
}

fn texture_descriptors_match(
    captured: &MapFrameTextureBinding,
    fallback: &MapFrameTextureBinding,
) -> bool {
    captured.stage == fallback.stage
        && captured.slot == fallback.slot
        && captured.declared_name == fallback.declared_name
        && captured.is_null == fallback.is_null
        && captured.view_desc == fallback.view_desc
        && captured.dxgi_format == fallback.dxgi_format
        && captured.width == fallback.width
        && captured.height == fallback.height
        && captured.mip_count == fallback.mip_count
        && captured.array_size == fallback.array_size
        && captured.sample_count == fallback.sample_count
        && captured.sample_quality == fallback.sample_quality
        && captured.bind_flags == fallback.bind_flags
        && captured.usage == fallback.usage
        && captured.cpu_access_flags == fallback.cpu_access_flags
        && captured.misc_flags == fallback.misc_flags
}

fn is_descriptor_only_shader_resource(resource: &MapFrameTextureBinding) -> bool {
    !resource.is_null
        && resource.stage == "PS"
        && resource.file.is_none()
        && resource.fnv1a64.is_none()
        && resource.byte_len == 0
}

fn canonical_runtime_shader_resource_file(name: &str) -> Option<&'static str> {
    match name {
        SNOW_MUD_DATA_SYMBOL => Some("snow_mud_data.rgba"),
        LIGHT_INDEX_MAP_SYMBOL => Some("light_index_map.rgba"),
        LIGHT_DATA_MAP_SYMBOL => Some("light_data_map.r32g32b32a32_float"),
        PROVINCE_SECONDARY_COLOR_MAP_SYMBOL => Some("province_secondary_color_map.rgba"),
        GRADIENT_BORDER_CHANNEL1_SYMBOL => Some("gradient_border_channel1.rgba"),
        GRADIENT_BORDER_CHANNEL2_SYMBOL => Some("gradient_border_channel2.rgba"),
        GRADIENT_BORDER_CHANNEL3_SYMBOL => Some("gradient_border_channel3.rgba"),
        SHADOW_MAP_SYMBOL => Some("shadow_map.rgba"),
        _ => None,
    }
}

fn terrain_chunk_library_shader_resource_fallbacks(
    bundle_dir: &Path,
) -> Result<BTreeMap<String, Vec<(MapFrameTextureBinding, MapFrameFileEvidence)>>> {
    let path = bundle_dir.join(TERRAIN_CHUNK_LIBRARY_FILE);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let library: TerrainChunkLibraryManifest = serde_json::from_slice(&fs::read(&path)?)?;
    let mut fallbacks: BTreeMap<String, Vec<(MapFrameTextureBinding, MapFrameFileEvidence)>> =
        BTreeMap::new();
    for chunk in &library.chunks {
        for resource in &chunk.draw.shader_resources {
            let Some(name) = resource.declared_name.as_ref() else {
                continue;
            };
            if resource.file.is_none() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
                continue;
            }
            let Some(file) = resource.file.as_ref() else {
                continue;
            };
            let evidence =
                library
                    .runtime_files
                    .get(file)
                    .cloned()
                    .unwrap_or_else(|| MapFrameFileEvidence {
                        file: file.clone(),
                        byte_len: resource.byte_len,
                        fnv1a64: resource.fnv1a64.clone().unwrap_or_default(),
                        provenance: format!(
                            "terrain_chunk_library_shader_resource_fallback_{name}"
                        ),
                    });
            let entries = fallbacks.entry(name.clone()).or_insert_with(Vec::new);
            if !entries.iter().any(|(existing, existing_evidence)| {
                texture_descriptors_match(existing, resource)
                    && existing.file == resource.file
                    && existing.byte_len == resource.byte_len
                    && existing.fnv1a64 == resource.fnv1a64
                    && existing_evidence.fnv1a64 == evidence.fnv1a64
            }) {
                entries.push((resource.clone(), evidence));
            }
        }
    }
    Ok(fallbacks)
}

pub(crate) fn effective_map_frame_extraction_blockers(
    frame: &MapFrameRuntimeBundle,
    spec: MapFrameBundleValidationSpec,
) -> Vec<String> {
    effective_extraction_blockers(&frame.blockers, spec)
}

pub(crate) fn effective_extraction_blockers(
    blockers: &[String],
    spec: MapFrameBundleValidationSpec,
) -> Vec<String> {
    blockers
        .iter()
        .filter(|blocker| {
            !(spec.ignore_dependent_pre_draw_output_blockers
                && is_dependent_pre_draw_output_blocker(blocker))
        })
        .cloned()
        .collect()
}

pub(crate) fn is_dependent_pre_draw_output_blocker(blocker: &str) -> bool {
    let lower = blocker.to_ascii_lowercase();
    let mentions_pre_draw_output = lower.contains("pre-draw om render target")
        || lower.contains("pre-draw om depth-stencil")
        || lower.contains("pre-draw output-merger render target")
        || lower.contains("pre-draw output-merger depth-stencil");
    mentions_pre_draw_output
        && (lower.contains("payload")
            || lower.contains("resolved readback")
            || lower.contains("readback"))
}

pub(crate) fn validate_map_frame_shader_cache_constants_evidence(
    frame: &MapFrameRuntimeBundle,
) -> Result<()> {
    let Some(constants) = &frame.shader_cache_constants else {
        return Ok(());
    };
    let terrain_slot = frame
        .active_draw
        .shader
        .reflection_resources
        .get(TERRAIN_ID_RESOURCE_SYMBOL)
        .copied()
        .ok_or("production terrain renderer blocked: active shader reflection does not declare TerrainIDMap for shader-cache constants")?;
    if constants.terrain_id_srv_slot != terrain_slot {
        return Err(format!(
            "production terrain renderer blocked: map-frame shader-cache constants name TerrainIDMap t{}, but active shader reflection declares t{}",
            constants.terrain_id_srv_slot, terrain_slot
        )
        .into());
    }
    let terrain_srv = map_frame_terrain_srv(&frame.active_draw)?;
    if constants.map_size_x.round() as u32 != terrain_srv.width
        || constants.map_size_y.round() as u32 != terrain_srv.height
    {
        return Err(format!(
            "production terrain renderer blocked: map-frame shader-cache MAP_SIZE {}x{} does not match captured TerrainIDMap {}x{}",
            constants.map_size_x, constants.map_size_y, terrain_srv.width, terrain_srv.height
        )
        .into());
    }
    if constants.cache_files.is_empty()
        || constants.provenance.trim().is_empty()
        || constants.extraction.trim().is_empty()
    {
        return Err(
            "production terrain renderer blocked: map-frame shader-cache constants must include cache file provenance and extraction"
                .into(),
        );
    }
    validate_map_frame_shader_constants(MapFrameTerrainShaderConstants {
        map_size_x: constants.map_size_x,
        map_size_y: constants.map_size_y,
        map_pow2_x: constants.map_pow2_x,
        map_pow2_y: constants.map_pow2_y,
        fow_pow2_x: constants.fow_pow2_x,
        fow_pow2_y: constants.fow_pow2_y,
    })
}

pub(crate) fn map_capture_session_blocker_error_for(
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Option<Box<dyn Error>> {
    let session_path = bundle_dir.join(MAP_CAPTURE_SESSION_FILE);
    let session: MapCaptureSession = serde_json::from_slice(&fs::read(&session_path).ok()?).ok()?;
    if session.schema != MAP_CAPTURE_SESSION_SCHEMA {
        return None;
    }
    let moment = session
        .moments
        .iter()
        .find(|moment| moment.name == spec.capture_moment)?;
    if moment.status != "blocked" {
        return None;
    }
    let blockers = if moment.blockers.is_empty() {
        "no blocker details recorded".to_string()
    } else {
        moment.blockers.join("; ")
    };
    Some(
        format!(
            "{} blocked: map capture session {} is blocked before {} exists: {blockers}",
            spec.renderer_label, spec.capture_moment, spec.bundle_file
        )
        .into(),
    )
}

pub(crate) fn validate_map_capture_session_manifest_for(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    if frame.capture_moment != spec.capture_moment {
        return Err(format!(
            "{} blocked: map-frame bundle capture moment {} is not {}",
            spec.renderer_label, frame.capture_moment, spec.capture_moment
        )
        .into());
    }
    if frame.session_manifest != MAP_CAPTURE_SESSION_FILE {
        return Err(format!(
            "{} blocked: map-frame bundle must cite session manifest {MAP_CAPTURE_SESSION_FILE}",
            spec.renderer_label
        )
        .into());
    }
    let session_path = bundle_dir.join(&frame.session_manifest);
    let session_bytes = fs::read(&session_path).map_err(|error| {
        format!(
            "{} blocked: map-frame bundle session manifest {} is missing or unreadable: {error}",
            spec.renderer_label, frame.session_manifest
        )
    })?;
    let session: MapCaptureSession = serde_json::from_slice(&session_bytes)?;
    if session.schema != MAP_CAPTURE_SESSION_SCHEMA {
        return Err(format!(
            "{} blocked: unsupported map capture session schema {}",
            spec.renderer_label, session.schema
        )
        .into());
    }
    if session.required_first_moment != TERRAIN_CAPTURE_MOMENT {
        return Err(format!(
            "{} blocked: map capture session first moment {} is not {}",
            spec.renderer_label, session.required_first_moment, TERRAIN_CAPTURE_MOMENT
        )
        .into());
    }
    let moment = session
        .moments
        .iter()
        .find(|moment| moment.name == spec.capture_moment)
        .ok_or_else(|| {
            format!(
                "{} blocked: map capture session is missing {} moment",
                spec.renderer_label, spec.capture_moment
            )
        })?;
    let moment_blockers = effective_extraction_blockers(&moment.blockers, spec);
    let only_ignored_dependent_pre_draw_blockers = spec.ignore_dependent_pre_draw_output_blockers
        && moment.status == "blocked"
        && !moment.blockers.is_empty()
        && moment_blockers.is_empty();
    if moment.status != "captured_pending_renderer_validation"
        && !only_ignored_dependent_pre_draw_blockers
    {
        return Err(format!(
            "{} blocked: map capture session {} moment status is {}",
            spec.renderer_label, spec.capture_moment, moment.status
        )
        .into());
    }
    if moment.bundle_file != spec.bundle_file {
        return Err(format!(
            "{} blocked: map capture session {} moment points to {}, expected {}",
            spec.renderer_label, spec.capture_moment, moment.bundle_file, spec.bundle_file
        )
        .into());
    }
    for required in MAP_CAPTURE_REQUIRED_PAYLOAD {
        if !moment
            .required_payload
            .iter()
            .any(|payload| payload == required)
        {
            return Err(format!(
                "{} blocked: map capture session {} moment is missing required payload {required}",
                spec.renderer_label, spec.capture_moment
            )
            .into());
        }
    }
    if !moment_blockers.is_empty() {
        return Err(format!(
            "{} blocked: map capture session {} moment reported blocker(s): {}",
            spec.renderer_label,
            spec.capture_moment,
            moment_blockers.join("; ")
        )
        .into());
    }
    if spec.capture_moment != TERRAIN_CAPTURE_MOMENT {
        let terrain_moment = session
            .moments
            .iter()
            .find(|moment| moment.name == TERRAIN_CAPTURE_MOMENT)
            .ok_or_else(|| {
                format!(
                    "{} blocked: map capture session for {} is missing required prior terrain moment",
                    spec.renderer_label, spec.capture_moment
                )
            })?;
        if terrain_moment.status != "captured_pending_renderer_validation" {
            return Err(format!(
                "{} blocked: map capture session for {} requires terrain moment status captured_pending_renderer_validation, got {}",
                spec.renderer_label, spec.capture_moment, terrain_moment.status
            )
            .into());
        }
        return Ok(());
    }
    for deferred in &session.deferred_moments {
        if deferred.required_after != TERRAIN_CAPTURE_MOMENT {
            return Err(format!(
                "{} blocked: deferred capture moment {} is not gated behind {}",
                spec.renderer_label, deferred.name, TERRAIN_CAPTURE_MOMENT
            )
            .into());
        }
        if deferred.status != "deferred_until_terrain_active_pass_validated" {
            return Err(format!(
                "{} blocked: deferred capture moment {} has unsupported status {}",
                spec.renderer_label, deferred.name, deferred.status
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn map_frame_bundle_manifest_path(path: &Path) -> PathBuf {
    map_frame_bundle_manifest_path_for(path, TERRAIN_MAP_FRAME_SPEC)
}

pub(crate) fn map_frame_bundle_manifest_path_for(
    path: &Path,
    spec: MapFrameBundleValidationSpec,
) -> PathBuf {
    if path.is_dir() {
        path.join(spec.bundle_file)
    } else {
        path.to_path_buf()
    }
}

pub(crate) fn map_frame_bundle_dir(path: &Path) -> Result<PathBuf> {
    map_frame_bundle_dir_for(path, TERRAIN_MAP_FRAME_SPEC)
}

pub(crate) fn map_frame_bundle_dir_for(
    path: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<PathBuf> {
    let manifest_path = map_frame_bundle_manifest_path_for(path, spec);
    Ok(manifest_path
        .parent()
        .ok_or_else(|| {
            format!(
                "{} blocked: {} has no parent directory",
                spec.renderer_label,
                manifest_path.display()
            )
        })?
        .to_path_buf())
}

pub(crate) fn validate_map_frame_draw(
    draw: &MapFrameDraw,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    if !matches!(
        draw.draw_call.as_str(),
        "Draw" | "DrawIndexed" | "DrawInstanced" | "DrawIndexedInstanced"
    ) {
        return Err(format!(
            "{} blocked: unsupported map-frame draw call {}",
            spec.renderer_label, draw.draw_call
        )
        .into());
    }
    if draw.draw_count == 0 {
        return Err(format!(
            "{} blocked: map-frame draw {} count must be nonzero",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    if draw.topology.is_none() {
        return Err(format!(
            "{} blocked: map-frame draw {} has no captured IA primitive topology",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    validate_map_frame_draw_args(draw, spec.renderer_label)?;
    if spec.terrain_strict {
        validate_map_frame_shader_identity(&draw.shader, frame, bundle_dir)?;
    } else {
        validate_generic_map_frame_shader_identity(&draw.shader, frame, bundle_dir, spec)?;
    }
    let vertex_shader = draw.vertex_shader.as_ref().ok_or_else(|| {
        format!(
            "{} blocked: map-frame draw {} has no active vertex shader bytecode/reflection evidence",
            spec.renderer_label, draw.draw_index
        )
    })?;
    if spec.terrain_strict {
        validate_map_frame_vertex_shader_identity(vertex_shader, frame, bundle_dir)?;
    } else {
        validate_generic_map_frame_vertex_shader_identity(vertex_shader, frame, bundle_dir, spec)?;
    }
    validate_map_frame_input_layout(
        &draw.input_layout,
        draw,
        spec.renderer_label,
        spec.terrain_strict,
    )?;
    Ok(())
}

pub(crate) fn validate_map_frame_draw_args(
    draw: &MapFrameDraw,
    renderer_label: &str,
) -> Result<()> {
    match draw.draw_call.as_str() {
        "Draw" | "DrawIndexed" => {
            if draw.per_instance_count.is_some()
                || draw.instance_count.is_some()
                || draw.start_instance_location.is_some()
            {
                return Err(format!(
                    "{renderer_label} blocked: non-instanced map-frame draw {} must not carry instanced draw args",
                    draw.draw_index
                )
                .into());
            }
        }
        "DrawInstanced" | "DrawIndexedInstanced" => {
            let Some(per_instance_count) = draw.per_instance_count else {
                return Err(format!(
                    "{renderer_label} blocked: map-frame draw {} {} is missing per-instance count",
                    draw.draw_index, draw.draw_call
                )
                .into());
            };
            let Some(instance_count) = draw.instance_count else {
                return Err(format!(
                    "{renderer_label} blocked: map-frame draw {} {} is missing instance count",
                    draw.draw_index, draw.draw_call
                )
                .into());
            };
            if draw.start_instance_location.is_none() {
                return Err(format!(
                    "{renderer_label} blocked: map-frame draw {} {} is missing start instance location",
                    draw.draw_index, draw.draw_call
                )
                .into());
            }
            if per_instance_count == 0 || instance_count == 0 {
                return Err(format!(
                    "{renderer_label} blocked: map-frame draw {} {} has zero per-instance or instance count",
                    draw.draw_index, draw.draw_call
                )
                .into());
            }
            if per_instance_count.saturating_mul(instance_count) != draw.draw_count {
                return Err(format!(
                    "{renderer_label} blocked: map-frame draw {} {} count {} does not match per-instance count {} * instance count {}",
                    draw.draw_index,
                    draw.draw_call,
                    draw.draw_count,
                    per_instance_count,
                    instance_count
                )
                .into());
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_map_frame_shader_identity(
    shader: &MapFrameShader,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
) -> Result<()> {
    if shader.stage != "PS" {
        return Err(format!(
            "production terrain renderer blocked: active pdxmap shader stage {} is not PS",
            shader.stage
        )
        .into());
    }
    if shader.pointer.trim().is_empty() || shader.byte_len == 0 || shader.fnv1a64.trim().is_empty()
    {
        return Err(
            "production terrain renderer blocked: active pdxmap shader identity must include pointer, byte length, and hash"
                .into(),
        );
    }
    if shader.bytecode_file.trim().is_empty() {
        return Err(
            "production terrain renderer blocked: active pdxmap shader identity has no bytecode file"
                .into(),
        );
    }
    let evidence = frame
        .runtime_files
        .get(&shader.bytecode_file)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: active pdxmap shader bytecode {} is missing from runtime file evidence",
                shader.bytecode_file
            )
        })?;
    if evidence.byte_len != shader.byte_len || evidence.fnv1a64 != shader.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: active pdxmap shader bytecode evidence {} does not match shader identity",
            shader.bytecode_file
        )
        .into());
    }
    validate_map_frame_file_evidence(bundle_dir, evidence)?;
    if !shader
        .reflection_resources
        .contains_key(TERRAIN_ID_RESOURCE_SYMBOL)
    {
        return Err(format!(
            "production terrain renderer blocked: active shader reflection does not declare {TERRAIN_ID_RESOURCE_SYMBOL}"
        )
        .into());
    }
    if !shader
        .source_evidence
        .iter()
        .any(|source| source == "gfx/FX/pdxmap.shader")
    {
        return Err(
            "production terrain renderer blocked: map-frame shader identity must cite gfx/FX/pdxmap.shader"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_generic_map_frame_shader_identity(
    shader: &MapFrameShader,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    if shader.stage != "PS" {
        return Err(format!(
            "{} blocked: active {} shader stage {} is not PS",
            spec.renderer_label, spec.capture_moment, shader.stage
        )
        .into());
    }
    validate_map_frame_shader_bytecode_evidence(
        shader,
        frame,
        bundle_dir,
        "pixel shader",
        spec.renderer_label,
        spec.capture_moment,
    )?;
    if shader.reflection_resources.is_empty() {
        return Err(format!(
            "{} blocked: active {} pixel shader reflection captured no texture resources",
            spec.renderer_label, spec.capture_moment
        )
        .into());
    }
    for resource in spec.required_reflected_resources {
        if !shader.reflection_resources.contains_key(*resource) {
            return Err(format!(
                "{} blocked: active {} pixel shader reflection does not declare required the game resource {}",
                spec.renderer_label, spec.capture_moment, resource
            )
            .into());
        }
    }
    if !shader
        .source_evidence
        .iter()
        .any(|source| source == spec.primary_shader_source)
    {
        return Err(format!(
            "{} blocked: active {} pixel shader identity must cite {}",
            spec.renderer_label, spec.capture_moment, spec.primary_shader_source
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_vertex_shader_identity(
    shader: &MapFrameShader,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
) -> Result<()> {
    if shader.stage != "VS" {
        return Err(format!(
            "production terrain renderer blocked: active terrain vertex shader stage {} is not VS",
            shader.stage
        )
        .into());
    }
    validate_map_frame_shader_bytecode_evidence(
        shader,
        frame,
        bundle_dir,
        "vertex shader",
        "production terrain renderer",
        "pdxmap",
    )?;
    let has_position = shader.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("POSITION")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_SINT32
            && parameter.mask == 0x0f
    });
    let has_height = shader.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("TEXCOORD")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_SINT32
            && parameter.mask & 0x03 == 0x03
    });
    if !has_position || !has_height {
        return Err(
            "production terrain renderer blocked: active terrain vertex shader reflection must include POSITION0 int4 and TEXCOORD0 height inputs"
                .into(),
        );
    }
    if shader
        .constant_buffers
        .get(TERRAIN_DRAW_CB1_LAYOUT_SOURCE)
        .copied()
        != Some(TERRAIN_DRAW_CB1_BIND_POINT)
    {
        return Err(format!(
            "production terrain renderer blocked: active terrain vertex shader reflection must bind {} at VS constant-buffer slot {}",
            TERRAIN_DRAW_CB1_LAYOUT_SOURCE, TERRAIN_DRAW_CB1_BIND_POINT
        )
        .into());
    }
    if !shader
        .source_evidence
        .iter()
        .any(|source| source == "gfx/FX/pdxmap.shader")
        || !shader
            .source_evidence
            .iter()
            .any(|source| source == "gfx/FX/pdxmap.fxh")
    {
        return Err(
            "production terrain renderer blocked: active terrain vertex shader identity must cite gfx/FX/pdxmap.shader and gfx/FX/pdxmap.fxh"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_generic_map_frame_vertex_shader_identity(
    shader: &MapFrameShader,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    if shader.stage != "VS" {
        return Err(format!(
            "{} blocked: active {} vertex shader stage {} is not VS",
            spec.renderer_label, spec.capture_moment, shader.stage
        )
        .into());
    }
    validate_map_frame_shader_bytecode_evidence(
        shader,
        frame,
        bundle_dir,
        "vertex shader",
        spec.renderer_label,
        spec.capture_moment,
    )?;
    if shader.input_parameters.is_empty() {
        return Err(format!(
            "{} blocked: active {} vertex shader reflection captured no input parameters",
            spec.renderer_label, spec.capture_moment
        )
        .into());
    }
    if !shader
        .source_evidence
        .iter()
        .any(|source| source == spec.primary_shader_source)
    {
        return Err(format!(
            "{} blocked: active {} vertex shader identity must cite {}",
            spec.renderer_label, spec.capture_moment, spec.primary_shader_source
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_shader_bytecode_evidence(
    shader: &MapFrameShader,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    role: &str,
    renderer_label: &str,
    shader_label: &str,
) -> Result<()> {
    if shader.pointer.trim().is_empty() || shader.byte_len == 0 || shader.fnv1a64.trim().is_empty()
    {
        return Err(format!(
            "{renderer_label} blocked: active {shader_label} {role} identity must include pointer, byte length, and hash"
        )
        .into());
    }
    if shader.bytecode_file.trim().is_empty() {
        return Err(format!(
            "{renderer_label} blocked: active {shader_label} {role} identity has no bytecode file"
        )
        .into());
    }
    let evidence = frame
        .runtime_files
        .get(&shader.bytecode_file)
        .ok_or_else(|| {
            format!(
                "{renderer_label} blocked: active {shader_label} {role} bytecode {} is missing from runtime file evidence",
                shader.bytecode_file
            )
        })?;
    if evidence.byte_len != shader.byte_len || evidence.fnv1a64 != shader.fnv1a64 {
        return Err(format!(
            "{renderer_label} blocked: active {shader_label} {role} bytecode evidence {} does not match shader identity",
            shader.bytecode_file
        )
        .into());
    }
    validate_map_frame_file_evidence(bundle_dir, evidence)
}

pub(crate) fn validate_map_frame_vanilla_sources(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
    resolver: Option<&GameFileResolver>,
) -> Result<()> {
    for &source in spec.required_vanilla_sources {
        let evidence = frame.vanilla_sources.get(source).ok_or_else(|| {
            format!(
                "{} blocked: map-frame bundle is missing vanilla source evidence for {source}",
                spec.renderer_label
            )
        })?;
        validate_map_frame_file_evidence(bundle_dir, evidence)?;
        if let Some(resolver) = resolver {
            let resolved = resolver.resolve_required(
                source,
                &format!("effective-VFS source required by {}", spec.renderer_label),
            )?;
            let actual_len = usize::try_from(fs::metadata(&resolved)?.len())?;
            let actual_hash = fnv1a64_file(&resolved)?;
            if actual_len != evidence.byte_len || actual_hash != evidence.fnv1a64 {
                return Err(format!(
                    "{} blocked: effective-VFS source {source} at {} does not match the replay-captured content identity (bytes {} vs {}, fnv1a64 {} vs {})",
                    spec.renderer_label,
                    resolved.display(),
                    actual_len,
                    evidence.byte_len,
                    actual_hash,
                    evidence.fnv1a64,
                )
                .into());
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_map_frame_base_terrain_shipped_resources(
    frame: &MapFrameRuntimeBundle,
    bundle: &Path,
    manifest: &Manifest,
    contract: &MapMaterialContract,
) -> Result<()> {
    let mut blockers = Vec::new();
    for draw in &frame.draws {
        for (name, source) in TERRAIN_BASE_SHIPPED_SHADER_RESOURCES {
            validate_map_frame_post_diffuse_shipped_resource(
                draw,
                bundle,
                manifest,
                contract,
                name,
                source,
                &mut blockers,
            );
        }
    }
    if blockers.is_empty() {
        return Ok(());
    }
    Err(format!(
        "production terrain renderer blocked: base pdxmap terrain shader inputs are incomplete: {}",
        blockers.join("; ")
    )
    .into())
}

pub(crate) fn validate_map_frame_post_diffuse_inputs(
    frame: &MapFrameRuntimeBundle,
    bundle: &Path,
    map_frame_bundle_dir: &Path,
) -> Result<()> {
    let blockers = map_frame_post_diffuse_input_blockers(frame, bundle, map_frame_bundle_dir);
    if blockers.is_empty() {
        return Ok(());
    }
    Err(format!(
        "production terrain renderer blocked: post-diffuse pdxmap terrain shader inputs are incomplete: {}",
        blockers.join("; ")
    )
    .into())
}

pub(crate) fn map_frame_post_diffuse_input_blockers(
    frame: &MapFrameRuntimeBundle,
    bundle: &Path,
    map_frame_bundle_dir: &Path,
) -> Vec<String> {
    let mut blockers = Vec::new();
    for source in MAP_FRAME_REQUIRED_VANILLA_SOURCES {
        if !frame.vanilla_sources.contains_key(source) {
            blockers.push(format!("missing vanilla source evidence {source}"));
        }
    }

    let bundle_context = match load_post_diffuse_bundle_context(bundle) {
        Ok(context) => Some(context),
        Err(error) => {
            blockers.push(error.to_string());
            None
        }
    };

    for draw in &frame.draws {
        if let Some((manifest, contract)) = bundle_context.as_ref() {
            for (name, source) in TERRAIN_POST_DIFFUSE_SHIPPED_SHADER_RESOURCES {
                validate_map_frame_post_diffuse_shipped_resource(
                    draw,
                    bundle,
                    manifest,
                    contract,
                    name,
                    source,
                    &mut blockers,
                );
            }
        }
        for name in TERRAIN_POST_DIFFUSE_RUNTIME_SHADER_RESOURCES {
            validate_map_frame_post_diffuse_runtime_resource(
                frame,
                draw,
                bundle_context.as_ref().map(|(_, contract)| contract),
                name,
                &mut blockers,
            );
        }
        validate_map_frame_post_diffuse_vertex_outputs(
            frame,
            draw,
            bundle,
            map_frame_bundle_dir,
            &mut blockers,
        );
        for blocker in map_frame_post_diffuse_state_blockers(draw) {
            blockers.push(format!("draw {}: {blocker}", draw.draw_index));
        }
    }
    blockers
}

pub(crate) fn validate_map_frame_post_diffuse_vertex_outputs(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    bundle: &Path,
    map_frame_bundle_dir: &Path,
    blockers: &mut Vec<String>,
) {
    let constants = match load_map_frame_terrain_shader_constants(frame, bundle) {
        Ok(constants) => constants,
        Err(error) => {
            blockers.push(error.to_string());
            return;
        }
    };
    match decode_map_frame_terrain_draw_outputs(draw, map_frame_bundle_dir, constants) {
        Ok((outputs, _)) => {
            if outputs.iter().any(|output| {
                [
                    output.v_screen_coord.x,
                    output.v_screen_coord.y,
                    output.v_screen_coord.z,
                    output.v_screen_coord.w,
                    output.v_shadow_proj.x,
                    output.v_shadow_proj.y,
                    output.v_shadow_proj.z,
                    output.v_shadow_proj.w,
                ]
                .iter()
                .any(|value| !value.is_finite())
            }) {
                blockers.push(
                    "VS_OUTPUT_TERRAIN.vScreenCoord/vShadowProj replay produced non-finite values"
                        .to_string(),
                );
            }
        }
        Err(error) => blockers.push(format!(
            "draw {} VS_OUTPUT_TERRAIN.vScreenCoord/vShadowProj replay blocked: {error}",
            draw.draw_index
        )),
    }
}

pub(crate) fn map_frame_post_diffuse_state_blockers(draw: &MapFrameDraw) -> Vec<String> {
    let mut blockers = Vec::new();
    let Some(state) = draw.draw_state.as_ref() else {
        return vec![
            "map-frame draw state has no viewport, scissor, rasterizer, depth-stencil, or blend state capture".to_string(),
        ];
    };
    if state.viewports.is_empty() {
        blockers.push("map-frame draw state has no captured viewport".to_string());
    }
    for (index, viewport) in state.viewports.iter().enumerate() {
        if !viewport.top_left_x.is_finite()
            || !viewport.top_left_y.is_finite()
            || !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || !viewport.min_depth.is_finite()
            || !viewport.max_depth.is_finite()
            || viewport.width <= 0.0
            || viewport.height <= 0.0
            || viewport.min_depth > viewport.max_depth
        {
            blockers.push(format!(
                "map-frame draw state viewport {index} is not a valid D3D11 viewport"
            ));
        }
    }
    if !state.rasterizer.default_state && state.rasterizer.desc.is_none() {
        blockers.push("map-frame draw state has no rasterizer descriptor".to_string());
    }
    if !state.blend.default_state && state.blend.desc.is_none() {
        blockers.push("map-frame draw state has no blend descriptor".to_string());
    }
    if !state.depth_stencil.default_state && state.depth_stencil.desc.is_none() {
        blockers.push("map-frame draw state has no depth-stencil descriptor".to_string());
    }
    validate_map_frame_output_merger_state(state, &mut blockers);
    blockers
}

pub(crate) fn validate_map_frame_output_merger_state(
    state: &MapFrameDrawState,
    blockers: &mut Vec<String>,
) {
    let depth_enabled = state
        .depth_stencil
        .desc
        .as_ref()
        .map(|desc| desc.depth_enable)
        .unwrap_or(false);
    if !depth_enabled {
        return;
    }
    let Some(output_merger) = state.output_merger.as_ref() else {
        blockers.push(
            "depth test is enabled, but map-frame bundle has no output-merger target/depth resource evidence"
                .to_string(),
        );
        return;
    };
    if output_merger.render_targets.is_empty() {
        blockers.push(
            "depth test is enabled, but output-merger capture has no render target descriptor"
                .to_string(),
        );
    }
    for target in &output_merger.render_targets {
        validate_output_merger_texture(target, D3D11_BIND_RENDER_TARGET, "render target", blockers);
        validate_render_target_clear(target, blockers);
    }
    let Some(depth_stencil) = output_merger.depth_stencil.as_ref() else {
        blockers.push(
            "depth test is enabled, but output-merger capture has no depth-stencil descriptor"
                .to_string(),
        );
        return;
    };
    validate_output_merger_texture(
        depth_stencil,
        D3D11_BIND_DEPTH_STENCIL,
        "depth-stencil",
        blockers,
    );
    validate_depth_stencil_clear(
        depth_stencil,
        state
            .depth_stencil
            .desc
            .as_ref()
            .map(|desc| desc.stencil_enable)
            .unwrap_or(false),
        blockers,
    );
}

pub(crate) fn validate_output_merger_texture(
    texture: &MapFrameOutputMergerTexture,
    required_bind: u32,
    role: &str,
    blockers: &mut Vec<String>,
) {
    if texture.view_pointer.trim().is_empty() || texture.resource_pointer.trim().is_empty() {
        blockers.push(format!(
            "output-merger {role} descriptor is missing view/resource identity"
        ));
    }
    if texture.desc.width == 0 || texture.desc.height == 0 {
        blockers.push(format!(
            "output-merger {role} descriptor has zero dimensions"
        ));
    }
    if texture.desc.sample_count == 0 {
        blockers.push(format!(
            "output-merger {role} descriptor has zero sample count"
        ));
    }
    if texture.desc.bind_flags & required_bind == 0 {
        blockers.push(format!(
            "output-merger {role} descriptor bind flags {} do not include required D3D11 bind flag {required_bind}",
            texture.desc.bind_flags
        ));
    }
    if texture.view_desc.is_none() {
        blockers.push(format!(
            "output-merger {role} descriptor is missing captured D3D11 view descriptor"
        ));
    }
}

pub(crate) fn validate_render_target_clear(
    texture: &MapFrameOutputMergerTexture,
    blockers: &mut Vec<String>,
) {
    let Some(clear) = texture.last_render_target_clear.as_ref() else {
        blockers.push(
            "depth test is enabled, but output-merger render target has no captured ClearRenderTargetView provenance"
                .to_string(),
        );
        return;
    };
    if clear.sequence == 0 {
        blockers.push(
            "output-merger render target clear provenance has an invalid zero sequence".to_string(),
        );
    }
    if clear.color.iter().any(|channel| !channel.is_finite()) {
        blockers.push(
            "output-merger render target clear provenance contains a non-finite color".to_string(),
        );
    }
}

pub(crate) fn validate_depth_stencil_clear(
    texture: &MapFrameOutputMergerTexture,
    stencil_enabled: bool,
    blockers: &mut Vec<String>,
) {
    let Some(clear) = texture.last_depth_stencil_clear.as_ref() else {
        blockers.push(
            "depth test is enabled, but output-merger depth-stencil has no captured ClearDepthStencilView provenance"
                .to_string(),
        );
        return;
    };
    if clear.sequence == 0 {
        blockers.push(
            "output-merger depth-stencil clear provenance has an invalid zero sequence".to_string(),
        );
    }
    if clear.clear_flags & D3D11_CLEAR_DEPTH == 0 {
        blockers.push(
            "output-merger depth-stencil clear provenance did not clear D3D11 depth".to_string(),
        );
    }
    if stencil_enabled && clear.clear_flags & D3D11_CLEAR_STENCIL == 0 {
        blockers.push(
            "output-merger depth-stencil clear provenance did not clear D3D11 stencil while stencil test is enabled"
                .to_string(),
        );
    }
    if !clear.depth.is_finite() || !(0.0..=1.0).contains(&clear.depth) {
        blockers.push(
            "output-merger depth-stencil clear provenance has a depth outside D3D11 0..1"
                .to_string(),
        );
    }
}

pub(crate) fn load_post_diffuse_bundle_context(
    bundle: &Path,
) -> Result<(Manifest, MapMaterialContract)> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(bundle.join("manifest.json"))?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let contract: MapMaterialContract = serde_json::from_slice(&fs::read(
        bundle.join(&manifest.files.map_material_contract),
    )?)?;
    validate_material_mip_outputs(&contract, bundle)?;
    Ok((manifest, contract))
}

pub(crate) fn validate_map_frame_post_diffuse_shipped_resource(
    draw: &MapFrameDraw,
    bundle: &Path,
    manifest: &Manifest,
    contract: &MapMaterialContract,
    name: &str,
    source: &str,
    blockers: &mut Vec<String>,
) {
    let Some(resource) = map_frame_post_diffuse_bound_resource(draw, name, blockers) else {
        return;
    };
    let Ok(meta) = material_source_meta(contract, source) else {
        blockers.push(format!(
            "shader resource {name} is a shipped material sampler, but material contract has no source metadata for {source}"
        ));
        return;
    };
    if !meta
        .shader_samplers
        .iter()
        .any(|sampler| sampler.as_str() == name)
    {
        blockers.push(format!(
            "shader resource {name} is not listed as a sampler consumer of shipped material {source}"
        ));
    }
    if resource.width != meta.dimensions.width as u32
        || resource.height != meta.dimensions.height as u32
    {
        blockers.push(format!(
            "shader resource {name} SRV is {}x{}, but shipped material {source} is {}x{}",
            resource.width, resource.height, meta.dimensions.width, meta.dimensions.height
        ));
    }
    if resource.mip_count != meta.mip_count {
        blockers.push(format!(
            "shader resource {name} SRV has {} mip(s), but shipped material {source} has {}",
            resource.mip_count, meta.mip_count
        ));
    }
    if let Err(error) = shipped_material_color_space(name, resource) {
        blockers.push(error.to_string());
    }
    if let Err(error) = required_shader_source_path(bundle, manifest, source) {
        blockers.push(error.to_string());
    }
    if meta.mip_count > 1 && meta.mip_output.is_none() {
        blockers.push(format!(
            "shader resource {name} uses mipped shipped material {source}, but the material contract has no exact mip-chain output"
        ));
    }
    let sampler = draw
        .samplers
        .iter()
        .find(|sampler| sampler.stage == "PS" && sampler.slot == resource.slot);
    match sampler {
        Some(sampler) => {
            if let Err(error) = validate_map_frame_linear_wrap_sampler(name, sampler) {
                blockers.push(error.to_string());
            }
        }
        None => blockers.push(format!(
            "shader resource {name} at PS slot {} has no sampler-state proof",
            resource.slot
        )),
    }
}

pub(crate) fn validate_map_frame_post_diffuse_runtime_resource(
    frame: &MapFrameRuntimeBundle,
    draw: &MapFrameDraw,
    contract: Option<&MapMaterialContract>,
    name: &str,
    blockers: &mut Vec<String>,
) {
    let Some(resource) = map_frame_post_diffuse_bound_resource(draw, name, blockers) else {
        return;
    };
    if resource.is_null {
        if name == SHADOW_MAP_SYMBOL {
            if let Err(error) = validate_map_frame_null_shadow_resource(draw, resource) {
                blockers.push(error.to_string());
            }
        } else {
            blockers.push(format!(
                "shader resource {name} at PS slot {} is explicitly null in captured D3D11 state",
                resource.slot
            ));
        }
        return;
    }
    if resource.file.is_none() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
        let provenance = contract
            .and_then(|contract| runtime_input_reason_for_symbol(contract, name))
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        blockers.push(format!(
            "shader resource {name} at PS slot {} is generated/runtime and has no captured bytes/hash{provenance}",
            resource.slot
        ));
        return;
    }
    if let Err(error) = validate_map_frame_runtime_resource(name, resource) {
        blockers.push(error.to_string());
        return;
    }
    let sampler = draw
        .samplers
        .iter()
        .find(|sampler| sampler.stage == "PS" && sampler.slot == resource.slot);
    match sampler {
        Some(sampler) => {
            let sampler_validation = match name {
                LIGHT_INDEX_MAP_SYMBOL | LIGHT_DATA_MAP_SYMBOL => {
                    validate_map_frame_point_clamp_sampler(name, sampler)
                }
                PROVINCE_SECONDARY_COLOR_MAP_SYMBOL | SNOW_MUD_DATA_SYMBOL => {
                    validate_map_frame_linear_wrap_sampler(name, sampler)
                }
                GRADIENT_BORDER_CHANNEL1_SYMBOL | GRADIENT_BORDER_CHANNEL2_SYMBOL => {
                    validate_map_frame_linear_clamp_sampler(name, sampler)
                }
                _ => Ok(()),
            };
            if let Err(error) = sampler_validation {
                blockers.push(error.to_string());
            }
        }
        None => blockers.push(format!(
            "shader resource {name} at PS slot {} has no sampler-state proof",
            resource.slot
        )),
    }
    if let Err(error) = validate_map_frame_runtime_file_link(frame, resource, name) {
        blockers.push(error.to_string());
    }
}

pub(crate) fn validate_map_frame_null_shadow_resource(
    draw: &MapFrameDraw,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.declared_name.as_deref() != Some(SHADOW_MAP_SYMBOL) {
        return Err(format!(
            "production terrain renderer blocked: null shadow SRV slot {} is declared as {:?}, expected {SHADOW_MAP_SYMBOL}",
            resource.slot, resource.declared_name
        )
        .into());
    }
    if resource.file.is_some()
        || resource.fnv1a64.is_some()
        || resource.byte_len != 0
        || resource.dxgi_format != 0
        || resource.width != 0
        || resource.height != 0
        || resource.mip_count != 0
        || resource.bind_flags != 0
        || resource.usage != 0
    {
        return Err(format!(
            "production terrain renderer blocked: null {SHADOW_MAP_SYMBOL} SRV at PS slot {} must not carry fake texture metadata or bytes",
            resource.slot
        )
        .into());
    }
    let sampler = draw
        .samplers
        .iter()
        .find(|sampler| sampler.stage == "PS" && sampler.slot == resource.slot)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: null {SHADOW_MAP_SYMBOL} PS slot {} has no sampler-state proof",
                resource.slot
            )
        })?;
    validate_map_frame_linear_wrap_sampler(SHADOW_MAP_SYMBOL, sampler)
}

pub(crate) fn map_frame_post_diffuse_bound_resource<'a>(
    draw: &'a MapFrameDraw,
    name: &str,
    blockers: &mut Vec<String>,
) -> Option<&'a MapFrameTextureBinding> {
    let Some(slot) = draw.shader.reflection_resources.get(name) else {
        blockers.push(format!(
            "draw {} shader reflection/capture does not identify required pdxmap resource {name}",
            draw.draw_index
        ));
        return None;
    };
    let resource = draw
        .shader_resources
        .iter()
        .find(|resource| resource.stage == "PS" && resource.slot == *slot);
    let Some(resource) = resource else {
        blockers.push(format!(
            "shader resource {name} is declared at PS slot {slot}, but no SRV binding was captured"
        ));
        return None;
    };
    if resource.declared_name.as_deref() != Some(name) {
        blockers.push(format!(
            "shader resource {name} is expected at PS slot {slot}, but the bound SRV declared name is {:?}",
            resource.declared_name
        ));
    }
    Some(resource)
}

pub(crate) fn runtime_input_reason_for_symbol<'a>(
    contract: &'a MapMaterialContract,
    symbol: &str,
) -> Option<&'a str> {
    contract
        .generated_runtime_inputs
        .values()
        .find(|input| {
            input
                .shader_symbols
                .iter()
                .any(|candidate| candidate == symbol)
        })
        .map(|input| input.reason.as_str())
}

pub(crate) fn validate_map_frame_input_layout(
    input_layout: &MapFrameInputLayout,
    draw: &MapFrameDraw,
    renderer_label: &str,
    require_elements_within_stride: bool,
) -> Result<()> {
    if input_layout.pointer.trim().is_empty() || input_layout.elements.is_empty() {
        return Err(format!(
            "{renderer_label} blocked: map-frame input layout must include pointer and elements"
        )
        .into());
    }
    for element in &input_layout.elements {
        let key = element.input_slot.to_string();
        let vertex_buffer = draw.vertex_buffers.get(&key).ok_or_else(|| {
            format!(
                "{renderer_label} blocked: map-frame input layout {}{} uses IA slot {}, but the bundle has no vertex buffer for that slot",
                element.semantic_name, element.semantic_index, element.input_slot
            )
        })?;
        if vertex_buffer.stage != "IA" {
            return Err(format!(
                "{renderer_label} blocked: vertex buffer slot {} stage {} is not IA",
                element.input_slot, vertex_buffer.stage
            )
            .into());
        }
        let stride = vertex_buffer.stride.ok_or_else(|| {
            format!(
                "{renderer_label} blocked: vertex buffer slot {} is missing stride",
                element.input_slot
            )
        })?;
        if stride == 0 {
            return Err(format!(
                "{renderer_label} blocked: vertex buffer slot {} has zero stride",
                element.input_slot
            )
            .into());
        }
        let format_byte_width = d3d11_input_format_byte_width(element.format).ok_or_else(|| {
            format!(
                "{renderer_label} blocked: input layout {}{} has unsupported DXGI format {}",
                element.semantic_name, element.semantic_index, element.format
            )
        })?;
        if element.aligned_byte_offset == D3D11_APPEND_ALIGNED_ELEMENT {
            return Err(format!(
                "{renderer_label} blocked: input layout {}{} uses D3D11_APPEND_ALIGNED_ELEMENT",
                element.semantic_name, element.semantic_index
            )
            .into());
        }
        if require_elements_within_stride
            && element
                .aligned_byte_offset
                .saturating_add(format_byte_width)
                > stride
        {
            return Err(format!(
                "{renderer_label} blocked: input layout {}{} offset {} plus {} bytes exceeds slot {} stride {}",
                element.semantic_name,
                element.semantic_index,
                element.aligned_byte_offset,
                format_byte_width,
                element.input_slot,
                stride
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn validate_map_frame_resources(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    for evidence in frame.runtime_files.values() {
        validate_map_frame_file_evidence(bundle_dir, evidence)?;
    }
    for draw in &frame.draws {
        if spec.terrain_strict {
            validate_map_frame_draw_resources(draw, frame, bundle_dir)?;
        } else {
            validate_generic_map_frame_draw_resources(draw, frame, bundle_dir, spec)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_generic_map_frame_draw_resources(
    draw: &MapFrameDraw,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    spec: MapFrameBundleValidationSpec,
) -> Result<()> {
    if draw.vertex_buffers.is_empty() {
        return Err(format!(
            "{} blocked: map-frame draw {} has no IA vertex-buffer captures",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    for vertex_buffer in draw.vertex_buffers.values() {
        validate_map_frame_buffer_file(bundle_dir, vertex_buffer, "vertex buffer")?;
    }
    if draw.constant_buffers.is_empty() {
        return Err(format!(
            "{} blocked: map-frame draw {} has no VS/PS constant-buffer captures",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    for constant_buffer in &draw.constant_buffers {
        validate_map_frame_constant_buffer(bundle_dir, constant_buffer)?;
    }
    validate_map_frame_reflected_constant_buffer_bindings(&draw.shader, draw, spec.renderer_label)?;
    if let Some(vertex_shader) = draw.vertex_shader.as_ref() {
        validate_map_frame_reflected_constant_buffer_bindings(
            vertex_shader,
            draw,
            spec.renderer_label,
        )?;
    }
    if spec.capture_moment == WATER_CAPTURE_MOMENT {
        validate_map_frame_app_camera_constant_buffer_coverage(
            draw,
            bundle_dir,
            spec.renderer_label,
        )?;
    }
    if draw.draw_call.contains("Indexed") {
        let index_buffer = draw.index_buffer.as_ref().ok_or_else(|| {
            format!(
                "{} blocked: indexed map-frame draw {} has no index buffer binding",
                spec.renderer_label, draw.draw_index
            )
        })?;
        validate_map_frame_index_buffer(bundle_dir, index_buffer)?;
    }
    validate_map_frame_bound_resource_names(draw, spec.renderer_label)?;
    validate_map_frame_reflected_resource_bindings(draw, spec.renderer_label)?;
    validate_map_frame_reflected_sampler_bindings(draw, spec.renderer_label)?;
    if spec.capture_moment == WATER_CAPTURE_MOMENT {
        validate_pdxwater_sampler_contract(draw, frame, bundle_dir, spec.renderer_label)?;
    }
    for resource in &draw.shader_resources {
        validate_map_frame_texture_file(bundle_dir, resource)?;
    }
    if draw.samplers.is_empty() {
        return Err(format!(
            "{} blocked: map-frame draw {} has no PS sampler-state captures",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    for resource in draw
        .shader_resources
        .iter()
        .filter(|resource| !resource.is_null)
    {
        if resource.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
            return Err(format!(
                "{} blocked: map-frame draw {} shader resource {:?} slot {} missing D3D11_BIND_SHADER_RESOURCE",
                spec.renderer_label, draw.draw_index, resource.declared_name, resource.slot
            )
            .into());
        }
        if resource.view_desc.is_none() {
            return Err(format!(
                "{} blocked: map-frame draw {} shader resource {:?} slot {} is missing captured SRV descriptor",
                spec.renderer_label, draw.draw_index, resource.declared_name, resource.slot
            )
            .into());
        }
        if spec.capture_moment == WATER_CAPTURE_MOMENT
            && resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
        {
            validate_app_owned_water_refraction_binding(
                draw.draw_index,
                resource,
                spec.renderer_label,
            )?;
            continue;
        }
        if resource.file.is_none() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
            return Err(format!(
                "{} blocked: map-frame draw {} shader resource {:?} slot {} must include captured bytes and hash",
                spec.renderer_label, draw.draw_index, resource.declared_name, resource.slot
            )
            .into());
        }
    }
    let mut state_blockers = map_frame_post_diffuse_state_blockers(draw);
    if !spec.terrain_strict {
        state_blockers.retain(|blocker| !is_clear_provenance_blocker(blocker));
    }
    for blocker in state_blockers {
        return Err(format!(
            "{} blocked: map-frame draw {}: {blocker}",
            spec.renderer_label, draw.draw_index
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_app_owned_water_refraction_binding(
    draw_index: u32,
    resource: &MapFrameTextureBinding,
    renderer_label: &str,
) -> Result<()> {
    if resource.is_null {
        return Err(format!(
            "{renderer_label} blocked: map-frame draw {draw_index} {WATER_REFRACTION_RESOURCE_SYMBOL} cannot be null because the production graph binds the app-owned terrain color target there"
        )
        .into());
    }
    if resource.width == 0
        || resource.height == 0
        || resource.dxgi_format == 0
        || resource.mip_count == 0
        || resource.array_size == 0
        || resource.sample_count == 0
    {
        return Err(format!(
            "{renderer_label} blocked: map-frame draw {draw_index} {WATER_REFRACTION_RESOURCE_SYMBOL} needs captured D3D11 texture descriptor metadata even when raw bytes are supplied by the app-owned terrain color target"
        )
        .into());
    }
    let Some(view) = resource.view_desc.as_ref() else {
        return Err(format!(
            "{renderer_label} blocked: map-frame draw {draw_index} {WATER_REFRACTION_RESOURCE_SYMBOL} needs captured SRV descriptor evidence"
        )
        .into());
    };
    if view.view_dimension != D3D11_SRV_DIMENSION_TEXTURE2D {
        return Err(format!(
            "{renderer_label} blocked: map-frame draw {draw_index} {WATER_REFRACTION_RESOURCE_SYMBOL} must be a Texture2D SRV for the app-owned terrain color target"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn is_clear_provenance_blocker(blocker: &str) -> bool {
    blocker.contains("no captured ClearRenderTargetView provenance")
        || blocker.contains("no captured ClearDepthStencilView provenance")
}

pub(crate) fn validate_map_frame_draw_resources(
    draw: &MapFrameDraw,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
) -> Result<()> {
    for vertex_buffer in draw.vertex_buffers.values() {
        validate_map_frame_buffer_file(bundle_dir, vertex_buffer, "vertex buffer")?;
    }
    if draw.constant_buffers.is_empty() {
        return Err(format!(
            "production terrain renderer blocked: map-frame draw {} has no VS/PS constant-buffer captures",
            draw.draw_index
        )
        .into());
    }
    for constant_buffer in &draw.constant_buffers {
        validate_map_frame_constant_buffer(bundle_dir, constant_buffer)?;
    }
    let mut has_ps_terrain_scene_cb0 = false;
    for buffer in draw
        .constant_buffers
        .iter()
        .filter(|buffer| buffer.stage == "PS" && buffer.slot == TERRAIN_SCENE_CB0_BIND_POINT)
    {
        if terrain_scene_cb0_camera_layout_for_binding(bundle_dir, draw, buffer, buffer.byte_len)?
            .is_some()
        {
            has_ps_terrain_scene_cb0 = true;
            break;
        }
    }
    if !has_ps_terrain_scene_cb0 {
        return Err(format!(
            "production terrain renderer blocked: map-frame draw {} has no {TERRAIN_SCENE_CB0_SYMBOL}-compatible PS constant-buffer capture at slot {TERRAIN_SCENE_CB0_BIND_POINT}",
            draw.draw_index
        )
        .into());
    }
    if draw.draw_call.contains("Indexed") {
        let index_buffer = draw.index_buffer.as_ref().ok_or(
            "production terrain renderer blocked: indexed map-frame draw has no index buffer binding",
        )?;
        validate_map_frame_index_buffer(bundle_dir, index_buffer)?;
    }
    let terrain_slot = draw.shader.reflection_resources[TERRAIN_ID_RESOURCE_SYMBOL];
    let terrain_srv = map_frame_shader_resource(draw, TERRAIN_ID_RESOURCE_SYMBOL)?;
    validate_map_frame_runtime_rgba_resource(TERRAIN_ID_RESOURCE_SYMBOL, terrain_srv)?;
    validate_map_frame_runtime_file_link(frame, terrain_srv, TERRAIN_ID_RESOURCE_SYMBOL)?;
    validate_map_frame_texture_file(bundle_dir, terrain_srv)?;
    let terrain_sampler = draw
        .samplers
        .iter()
        .find(|sampler| sampler.stage == "PS" && sampler.slot == terrain_slot)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: active TerrainIDMap PS slot {terrain_slot} has no sampler-state proof"
            )
        })?;
    validate_map_frame_terrain_sampler(terrain_sampler)?;
    if draw
        .shader
        .reflection_resources
        .contains_key(SNOW_MUD_DATA_SYMBOL)
    {
        let snow_mud_slot = draw.shader.reflection_resources[SNOW_MUD_DATA_SYMBOL];
        let resource = map_frame_shader_resource(draw, SNOW_MUD_DATA_SYMBOL)?;
        validate_map_frame_runtime_rgba_resource(SNOW_MUD_DATA_SYMBOL, resource)?;
        validate_map_frame_runtime_file_link(frame, resource, SNOW_MUD_DATA_SYMBOL)?;
        let sampler = draw
            .samplers
            .iter()
            .find(|sampler| sampler.stage == "PS" && sampler.slot == snow_mud_slot)
            .ok_or_else(|| {
                format!(
                    "production terrain renderer blocked: active {SNOW_MUD_DATA_SYMBOL} PS slot {snow_mud_slot} has no sampler-state proof"
                )
            })?;
        validate_map_frame_linear_wrap_sampler(SNOW_MUD_DATA_SYMBOL, sampler)?;
    }
    validate_map_frame_bound_resource_names(draw, "production terrain renderer")?;
    for resource in &draw.shader_resources {
        validate_map_frame_texture_file(bundle_dir, resource)?;
    }
    Ok(())
}

pub(crate) fn validate_map_frame_bound_resource_names(
    draw: &MapFrameDraw,
    renderer_label: &str,
) -> Result<()> {
    for resource in &draw.shader_resources {
        let Some(name) = resource.declared_name.as_ref() else {
            continue;
        };
        let expected_slot = draw.shader.reflection_resources.get(name).ok_or_else(|| {
            format!(
                "{renderer_label} blocked: PS slot {} is declared as {name}, but active shader reflection has no matching resource name",
                resource.slot
            )
        })?;
        if *expected_slot != resource.slot {
            return Err(format!(
                "{renderer_label} blocked: PS slot {} is declared as {name}, but active shader reflection expects slot {expected_slot}",
                resource.slot
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn validate_map_frame_reflected_constant_buffer_bindings(
    shader: &MapFrameShader,
    draw: &MapFrameDraw,
    renderer_label: &str,
) -> Result<()> {
    if let Some(blocker) = map_frame_reflected_constant_buffer_binding_blockers(shader, draw)
        .into_iter()
        .next()
    {
        return Err(format!("{renderer_label} blocked: {blocker}").into());
    }
    Ok(())
}

pub(crate) fn map_frame_reflected_constant_buffer_binding_blockers(
    shader: &MapFrameShader,
    draw: &MapFrameDraw,
) -> Vec<String> {
    let mut blockers = Vec::new();
    for (name, slot) in &shader.constant_buffers {
        let Some(buffer) = draw
            .constant_buffers
            .iter()
            .find(|buffer| buffer.stage == shader.stage && buffer.slot == *slot)
        else {
            blockers.push(format!(
                "map-frame draw {} active {} shader reflects constant buffer {name} at slot {slot}, but the bundle has no captured {} constant-buffer binding for that slot",
                draw.draw_index, shader.stage, shader.stage
            ));
            continue;
        };
        if buffer.declared_name.as_deref() != Some(name.as_str()) {
            blockers.push(format!(
                "map-frame draw {} {} constant-buffer slot {slot} is captured as {:?}, expected reflected constant buffer {name}",
                draw.draw_index, shader.stage, buffer.declared_name
            ));
        }
    }
    blockers
}

pub(crate) fn validate_map_frame_app_camera_constant_buffer_coverage(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
    renderer_label: &str,
) -> Result<()> {
    let mut found = false;
    for binding in &draw.constant_buffers {
        if terrain_scene_cb0_camera_layout_for_binding(bundle_dir, draw, binding, binding.byte_len)?
            .is_some()
        {
            validate_camera_constant_buffer_update_binding(
                binding,
                binding.byte_len,
                draw.draw_index,
            )?;
            found = true;
        }
    }
    if !found {
        return Err(format!(
            "{renderer_label} blocked: map-frame draw {} has no camera-updatable {TERRAIN_SCENE_CB0_SYMBOL} constant buffer for the app-owned map camera",
            draw.draw_index
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_reflected_resource_bindings(
    draw: &MapFrameDraw,
    renderer_label: &str,
) -> Result<()> {
    if let Some(blocker) = map_frame_reflected_resource_binding_blockers(draw)
        .into_iter()
        .next()
    {
        return Err(format!("{renderer_label} blocked: {blocker}").into());
    }
    Ok(())
}

pub(crate) fn map_frame_reflected_resource_binding_blockers(draw: &MapFrameDraw) -> Vec<String> {
    let mut blockers = Vec::new();
    for (name, slot) in &draw.shader.reflection_resources {
        let Some(resource) = draw
            .shader_resources
            .iter()
            .find(|resource| resource.stage == "PS" && resource.slot == *slot)
        else {
            blockers.push(format!(
                "map-frame draw {} active pixel shader reflects {name} at PS SRV slot {slot}, but the bundle has no captured SRV binding for that slot",
                draw.draw_index
            ));
            continue;
        };
        if resource.declared_name.as_deref() != Some(name.as_str()) {
            blockers.push(format!(
                "map-frame draw {} PS SRV slot {slot} is captured as {:?}, expected reflected resource {name}",
                draw.draw_index, resource.declared_name
            ));
        }
    }
    blockers
}

pub(crate) fn validate_map_frame_reflected_sampler_bindings(
    draw: &MapFrameDraw,
    renderer_label: &str,
) -> Result<()> {
    if let Some(blocker) = map_frame_reflected_sampler_binding_blockers(draw)
        .into_iter()
        .next()
    {
        return Err(format!("{renderer_label} blocked: {blocker}").into());
    }
    Ok(())
}

pub(crate) fn map_frame_reflected_sampler_binding_blockers(draw: &MapFrameDraw) -> Vec<String> {
    let mut blockers = Vec::new();
    for (name, slot) in &draw.shader.reflection_resources {
        let Some(sampler) = draw
            .samplers
            .iter()
            .find(|sampler| sampler.stage == "PS" && sampler.slot == *slot)
        else {
            blockers.push(format!(
                "map-frame draw {} active pixel shader reflects {name} at PS SRV slot {slot}, but the bundle has no captured PS sampler-state binding for that slot",
                draw.draw_index
            ));
            continue;
        };
        if sampler.max_lod < sampler.min_lod {
            blockers.push(format!(
                "map-frame draw {} PS sampler-state slot {slot} for reflected resource {name} has MaxLOD {} below MinLOD {}",
                draw.draw_index, sampler.max_lod, sampler.min_lod
            ));
        }
    }
    blockers
}

pub(crate) fn validate_pdxwater_sampler_contract(
    draw: &MapFrameDraw,
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    renderer_label: &str,
) -> Result<()> {
    let shader_source = map_frame_water_shader_source(frame, bundle_dir, renderer_label)?;
    for expectation in PDXWATER_SAMPLER_EXPECTATIONS {
        validate_shader_sampler_expectation_against_source(
            &shader_source,
            "pdxwater.shader",
            expectation,
        )?;
    }
    let blockers = pdxwater_sampler_binding_blockers(draw);
    if let Some(blocker) = blockers.into_iter().next() {
        return Err(format!("{renderer_label} blocked: {blocker}").into());
    }
    Ok(())
}

pub(crate) fn map_frame_water_shader_source(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
    renderer_label: &str,
) -> Result<String> {
    let evidence = frame.vanilla_sources.get(PDXWATER_SHADER_SOURCE).ok_or_else(|| {
        format!(
            "{renderer_label} blocked: map-frame bundle is missing vanilla source evidence for {PDXWATER_SHADER_SOURCE}"
        )
    })?;
    fs::read_to_string(bundle_dir.join(&evidence.file)).map_err(|error| {
        format!(
            "{renderer_label} blocked: vanilla source evidence {PDXWATER_SHADER_SOURCE} is unreadable: {error}"
        )
        .into()
    })
}

pub(crate) fn pdxwater_sampler_binding_blockers(draw: &MapFrameDraw) -> Vec<String> {
    let mut blockers = Vec::new();
    for expectation in PDXWATER_SAMPLER_EXPECTATIONS {
        let Some(slot) = draw
            .shader
            .reflection_resources
            .get(expectation.name)
            .copied()
        else {
            continue;
        };
        if slot != expectation.index {
            blockers.push(format!(
                "map-frame draw {} active pixel shader reflection declares {} at PS SRV slot {slot}, but pdxwater.shader declares Index {}",
                draw.draw_index, expectation.name, expectation.index
            ));
            continue;
        }
        let Some(sampler) = draw
            .samplers
            .iter()
            .find(|sampler| sampler.stage == "PS" && sampler.slot == slot)
        else {
            blockers.push(format!(
                "map-frame draw {} active pixel shader reflects {} at PS SRV slot {slot}, but the bundle has no captured PS sampler-state binding for the pdxwater.shader contract",
                draw.draw_index, expectation.name
            ));
            continue;
        };
        if let Some(blocker) =
            map_frame_sampler_mode_blocker(expectation.name, sampler, expectation.mode)
        {
            blockers.push(format!(
                "map-frame draw {} {blocker} from pdxwater.shader",
                draw.draw_index
            ));
        }
        if let Some(blocker) = pdxwater_resource_type_blocker(draw, expectation) {
            blockers.push(format!(
                "map-frame draw {} {blocker} from pdxwater.shader",
                draw.draw_index
            ));
        }
    }
    blockers
}

pub(crate) fn pdxwater_resource_type_blocker(
    draw: &MapFrameDraw,
    expectation: ShaderSamplerExpectation,
) -> Option<String> {
    if expectation.resource_type != ShaderSamplerResourceType::Cube {
        return None;
    }
    let resource = draw
        .shader_resources
        .iter()
        .find(|resource| resource.stage == "PS" && resource.slot == expectation.index)?;
    if resource.is_null {
        return None;
    }
    let Some(view) = resource.view_desc.as_ref() else {
        return None;
    };
    if view.view_dimension != D3D11_SRV_DIMENSION_TEXTURECUBE {
        return Some(format!(
            "{} declares Type Cube, but captured SRV slot {} view_dimension is {}",
            expectation.name, expectation.index, view.view_dimension
        ));
    }
    if resource.misc_flags & D3D11_RESOURCE_MISC_TEXTURECUBE == 0 {
        return Some(format!(
            "{} declares Type Cube, but captured texture MiscFlags do not include D3D11_RESOURCE_MISC_TEXTURECUBE",
            expectation.name
        ));
    }
    None
}

#[cfg(test)]
pub(crate) fn pdxwater_sampler_expectation(name: &str) -> Option<ShaderSamplerExpectation> {
    PDXWATER_SAMPLER_EXPECTATIONS
        .iter()
        .copied()
        .find(|expectation| expectation.name == name)
}

pub(crate) fn map_frame_sampler_mode_blocker(
    name: &str,
    sampler: &MapFrameSamplerBinding,
    mode: ShaderSamplerMode,
) -> Option<String> {
    if sampler.filter != mode.filter()
        || sampler.address_u != mode.address_u()
        || sampler.address_v != mode.address_v()
    {
        return Some(format!(
            "{name} sampler must be {}, got filter {} address_u {} address_v {}",
            mode.label(),
            sampler.filter,
            sampler.address_u,
            sampler.address_v
        ));
    }
    if sampler.max_lod < sampler.min_lod {
        return Some(format!(
            "{name} sampler MaxLOD {} is below MinLOD {}",
            sampler.max_lod, sampler.min_lod
        ));
    }
    None
}

pub(crate) fn map_frame_shader_resource<'a>(
    draw: &'a MapFrameDraw,
    name: &str,
) -> Result<&'a MapFrameTextureBinding> {
    let slot = draw
        .shader
        .reflection_resources
        .get(name)
        .copied()
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: active shader reflection does not declare {name}"
            )
        })?;
    let resource = draw
        .shader_resources
        .iter()
        .find(|resource| resource.stage == "PS" && resource.slot == slot)
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: active shader declares {name} at PS slot {slot}, but the map-frame bundle has no matching SRV"
            )
        })?;
    if resource.declared_name.as_deref() != Some(name) {
        return Err(format!(
            "production terrain renderer blocked: PS slot {slot} is declared as {:?}, expected {name}",
            resource.declared_name
        )
        .into());
    }
    Ok(resource)
}

pub(crate) fn validate_map_frame_runtime_rgba_resource(
    name: &str,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.width == 0 || resource.height == 0 || resource.mip_count == 0 {
        return Err(format!(
            "production terrain renderer blocked: {name} map-frame SRV metadata is incomplete"
        )
        .into());
    }
    if resource.file.is_none() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
        return Err(format!(
            "production terrain renderer blocked: shader-declared runtime resource {name} at PS slot {} must include captured bytes and hash",
            resource.slot
        )
        .into());
    }
    if resource.mip_count != 1 {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} must be captured as a single resolved RGBA level, got {} mips",
            resource.mip_count
        )
        .into());
    }
    if !matches!(
        resource.dxgi_format,
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
    ) {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} has unsupported source DXGI format {}",
            resource.dxgi_format
        )
        .into());
    }
    let expected_byte_len = checked_rgba_byte_len(
        usize::try_from(resource.width)?,
        usize::try_from(resource.height)?,
    )?;
    if resource.byte_len != expected_byte_len {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} byte_len {} does not match {}x{} RGBA byte_len {}",
            resource.byte_len, resource.width, resource.height, expected_byte_len
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_runtime_resource(
    name: &str,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if name == LIGHT_DATA_MAP_SYMBOL {
        validate_map_frame_runtime_r32g32b32a32_float_resource(name, resource)
    } else {
        validate_map_frame_runtime_rgba_resource(name, resource)
    }
}

pub(crate) fn validate_map_frame_runtime_r32g32b32a32_float_resource(
    name: &str,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.width == 0 || resource.height == 0 || resource.mip_count == 0 {
        return Err(format!(
            "production terrain renderer blocked: {name} map-frame SRV metadata is incomplete"
        )
        .into());
    }
    if resource.file.is_none() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
        return Err(format!(
            "production terrain renderer blocked: shader-declared runtime resource {name} at PS slot {} must include captured bytes and hash",
            resource.slot
        )
        .into());
    }
    if resource.mip_count != 1 {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} must be captured as a single resolved R32G32B32A32_FLOAT level, got {} mips",
            resource.mip_count
        )
        .into());
    }
    if resource.dxgi_format != DXGI_FORMAT_R32G32B32A32_FLOAT {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} has unsupported source DXGI format {}",
            resource.dxgi_format
        )
        .into());
    }
    let expected_byte_len = checked_rgba32f_byte_len(
        usize::try_from(resource.width)?,
        usize::try_from(resource.height)?,
    )?;
    if resource.byte_len != expected_byte_len {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} byte_len {} does not match {}x{} R32G32B32A32_FLOAT byte_len {}",
            resource.byte_len, resource.width, resource.height, expected_byte_len
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_runtime_file_link(
    frame: &MapFrameRuntimeBundle,
    resource: &MapFrameTextureBinding,
    name: &str,
) -> Result<()> {
    let file = resource.file.as_ref().ok_or_else(|| {
        format!("production terrain renderer blocked: runtime resource {name} has no file")
    })?;
    let evidence = frame.runtime_files.get(file).ok_or_else(|| {
        format!(
            "production terrain renderer blocked: runtime resource {name} file {file} is missing from bundle runtime file evidence"
        )
    })?;
    if evidence.byte_len != resource.byte_len
        || resource.fnv1a64.as_ref() != Some(&evidence.fnv1a64)
    {
        return Err(format!(
            "production terrain renderer blocked: runtime resource {name} file evidence does not match draw binding metadata"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_terrain_sampler(sampler: &MapFrameSamplerBinding) -> Result<()> {
    validate_map_frame_point_clamp_sampler(TERRAIN_ID_RESOURCE_SYMBOL, sampler)?;
    if sampler.max_lod < sampler.min_lod {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap sampler MaxLOD {} is below MinLOD {}",
            sampler.max_lod, sampler.min_lod
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_point_clamp_sampler(
    name: &str,
    sampler: &MapFrameSamplerBinding,
) -> Result<()> {
    if sampler.filter != D3D11_FILTER_MIN_MAG_MIP_POINT
        || sampler.address_u != D3D11_TEXTURE_ADDRESS_CLAMP
        || sampler.address_v != D3D11_TEXTURE_ADDRESS_CLAMP
    {
        return Err(format!(
            "production terrain renderer blocked: {name} sampler must be point/clamp, got filter {} address_u {} address_v {}",
            sampler.filter, sampler.address_u, sampler.address_v
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_linear_wrap_sampler(
    name: &str,
    sampler: &MapFrameSamplerBinding,
) -> Result<()> {
    if sampler.filter != D3D11_FILTER_MIN_MAG_MIP_LINEAR
        || sampler.address_u != D3D11_TEXTURE_ADDRESS_WRAP
        || sampler.address_v != D3D11_TEXTURE_ADDRESS_WRAP
    {
        return Err(format!(
            "production terrain renderer blocked: {name} sampler must be linear/wrap, got filter {} address_u {} address_v {}",
            sampler.filter, sampler.address_u, sampler.address_v
        )
        .into());
    }
    if sampler.max_lod < sampler.min_lod {
        return Err(format!(
            "production terrain renderer blocked: {name} sampler MaxLOD {} is below MinLOD {}",
            sampler.max_lod, sampler.min_lod
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_linear_clamp_sampler(
    name: &str,
    sampler: &MapFrameSamplerBinding,
) -> Result<()> {
    if sampler.filter != D3D11_FILTER_MIN_MAG_MIP_LINEAR
        || sampler.address_u != D3D11_TEXTURE_ADDRESS_CLAMP
        || sampler.address_v != D3D11_TEXTURE_ADDRESS_CLAMP
    {
        return Err(format!(
            "production terrain renderer blocked: {name} sampler must be linear/clamp, got filter {} address_u {} address_v {}",
            sampler.filter, sampler.address_u, sampler.address_v
        )
        .into());
    }
    if sampler.max_lod < sampler.min_lod {
        return Err(format!(
            "production terrain renderer blocked: {name} sampler MaxLOD {} is below MinLOD {}",
            sampler.max_lod, sampler.min_lod
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_texture_file(
    bundle_dir: &Path,
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if let Some(file) = &resource.file {
        if is_synthetic_map_frame_file(file) {
            return Ok(());
        }
        let path = bundle_dir.join(file);
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "production terrain renderer blocked: map-frame texture {} is not readable: {}",
                path.display(),
                error
            )
        })?;
        let actual_len = usize::try_from(metadata.len())
            .map_err(|_| format!("{} is too large to address", path.display()))?;
        if actual_len != resource.byte_len {
            return Err(format!(
                "production terrain renderer blocked: map-frame texture {} has {} bytes, metadata expects {}",
                path.display(),
                actual_len,
                resource.byte_len
            )
            .into());
        }
        let expected_hash = resource.fnv1a64.as_ref().ok_or_else(|| {
            format!(
                "production terrain renderer blocked: map-frame texture {} has no hash",
                path.display()
            )
        })?;
        let actual_hash = fnv1a64_file(&path)?;
        if &actual_hash != expected_hash {
            return Err(format!(
                "production terrain renderer blocked: map-frame texture {} hash changed from {} to {}",
                path.display(),
                expected_hash,
                actual_hash
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn validate_map_frame_file_evidence(
    bundle_dir: &Path,
    evidence: &MapFrameFileEvidence,
) -> Result<()> {
    if is_synthetic_map_frame_file(&evidence.file) {
        if evidence.byte_len == 0 || evidence.fnv1a64.trim().is_empty() {
            return Err(format!(
                "production terrain renderer blocked: synthetic map-frame file {} has incomplete evidence",
                evidence.file
            )
            .into());
        }
        return Ok(());
    }
    let path = bundle_dir.join(&evidence.file);
    let actual_len = usize::try_from(fs::metadata(&path)?.len())
        .map_err(|_| format!("{} is too large to address", path.display()))?;
    if actual_len != evidence.byte_len {
        return Err(format!(
            "production terrain renderer blocked: map-frame file {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            evidence.byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != evidence.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: map-frame file {} hash changed from {} to {}",
            path.display(),
            evidence.fnv1a64,
            actual_hash
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_buffer_file(
    bundle_dir: &Path,
    buffer: &MapFrameBufferBinding,
    role: &str,
) -> Result<()> {
    let file = buffer.file.as_ref().ok_or_else(|| {
        format!(
            "production terrain renderer blocked: map-frame {role} slot {} has no raw capture file",
            buffer.slot
        )
    })?;
    let path = bundle_dir.join(file);
    let metadata = fs::metadata(&path).map_err(|error| {
        format!(
            "production terrain renderer blocked: map-frame {role} {} is not readable: {}",
            path.display(),
            error
        )
    })?;
    let actual_len = usize::try_from(metadata.len())
        .map_err(|_| format!("{} is too large to address", path.display()))?;
    if actual_len != buffer.byte_len {
        return Err(format!(
            "production terrain renderer blocked: map-frame {role} {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            buffer.byte_len
        )
        .into());
    }
    let expected_hash = buffer.fnv1a64.as_ref().ok_or_else(|| {
        format!(
            "production terrain renderer blocked: map-frame {role} slot {} has no hash",
            buffer.slot
        )
    })?;
    let actual_hash = fnv1a64_file(&path)?;
    if &actual_hash != expected_hash {
        return Err(format!(
            "production terrain renderer blocked: map-frame {role} {} hash changed from {} to {}",
            path.display(),
            expected_hash,
            actual_hash
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_map_frame_constant_buffer(
    bundle_dir: &Path,
    buffer: &MapFrameBufferBinding,
) -> Result<()> {
    if !matches!(buffer.stage.as_str(), "VS" | "PS") {
        return Err(format!(
            "production terrain renderer blocked: map-frame constant buffer stage {} is not VS or PS",
            buffer.stage
        )
        .into());
    }
    if buffer.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
        return Err(format!(
            "production terrain renderer blocked: map-frame {} constant buffer slot {} missing D3D11_BIND_CONSTANT_BUFFER",
            buffer.stage, buffer.slot
        )
        .into());
    }
    validate_map_frame_buffer_file(bundle_dir, buffer, "constant buffer")
}

pub(crate) fn validate_map_frame_index_buffer(
    bundle_dir: &Path,
    index_buffer: &MapFrameIndexBufferBinding,
) -> Result<()> {
    if index_buffer.dxgi_format == 0 || index_buffer.byte_len == 0 {
        return Err(
            "production terrain renderer blocked: map-frame index buffer metadata is incomplete"
                .into(),
        );
    }
    let file = index_buffer.file.as_ref().ok_or(
        "production terrain renderer blocked: map-frame index buffer has no raw capture file",
    )?;
    let path = bundle_dir.join(file);
    let actual_len = usize::try_from(fs::metadata(&path)?.len())
        .map_err(|_| format!("{} is too large to address", path.display()))?;
    if actual_len != index_buffer.byte_len {
        return Err(format!(
            "production terrain renderer blocked: map-frame index buffer {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            index_buffer.byte_len
        )
        .into());
    }
    let expected_hash = index_buffer
        .fnv1a64
        .as_ref()
        .ok_or("production terrain renderer blocked: map-frame index buffer has no hash")?;
    let actual_hash = fnv1a64_file(&path)?;
    if &actual_hash != expected_hash {
        return Err(format!(
            "production terrain renderer blocked: map-frame index buffer {} hash changed from {} to {}",
            path.display(),
            expected_hash,
            actual_hash
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_terrain_runtime_inputs(
    runtime: &TerrainRuntimeInputs,
    manifest: &Manifest,
    bundle: &Path,
    terrain_id_map: &Path,
) -> Result<()> {
    if runtime.schema != TERRAIN_RUNTIME_SCHEMA {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain runtime schema: {}",
            runtime.schema
        )
        .into());
    }
    if runtime.terrain_id_map.file != manifest.files.terrain_id_map {
        return Err(format!(
            "production terrain renderer blocked: terrain runtime file {} does not match manifest {}",
            runtime.terrain_id_map.file, manifest.files.terrain_id_map
        )
        .into());
    }
    if runtime.terrain_id_map.width == 0 || runtime.terrain_id_map.height == 0 {
        return Err(
            "production terrain renderer blocked: TerrainIDMap dimensions must be nonzero".into(),
        );
    }
    if runtime.terrain_id_map.format != TERRAIN_ID_MAP_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: unsupported TerrainIDMap format {}",
            runtime.terrain_id_map.format
        )
        .into());
    }
    let expected_len =
        checked_rgba_byte_len(runtime.terrain_id_map.width, runtime.terrain_id_map.height)?;
    if runtime.terrain_id_map.byte_len != expected_len {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap metadata byte_len {} does not match {}x{} RGBA byte_len {}",
            runtime.terrain_id_map.byte_len,
            runtime.terrain_id_map.width,
            runtime.terrain_id_map.height,
            expected_len
        )
        .into());
    }
    let actual_len = fs::metadata(terrain_id_map)?.len();
    if actual_len != runtime.terrain_id_map.byte_len as u64 {
        return Err(format!(
            "production terrain renderer blocked: {} has {} bytes, terrain runtime metadata expects {}",
            terrain_id_map.display(),
            actual_len,
            runtime.terrain_id_map.byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(terrain_id_map)?;
    if actual_hash != runtime.terrain_id_map.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: TerrainIDMap hash changed from {} to {}",
            runtime.terrain_id_map.fnv1a64, actual_hash
        )
        .into());
    }
    if runtime.terrain_id_map.channel_order != terrain_id_map_channel_order() {
        return Err(
            "production terrain renderer blocked: TerrainIDMap channel contract does not match pdxmap.shader evidence"
                .into(),
        );
    }
    if runtime.terrain_id_map.sampler != terrain_id_map_sampler() {
        return Err(
            "production terrain renderer blocked: TerrainIDMap sampler contract does not match pdxmap.shader"
                .into(),
        );
    }
    for (symbol, texture) in &runtime.runtime_textures {
        validate_runtime_texture_input(symbol, texture, manifest, bundle)?;
    }
    for (symbol, constant_buffer) in &runtime.runtime_constant_buffers {
        validate_runtime_constant_buffer_input(symbol, constant_buffer, manifest, bundle)?;
    }
    for (symbol, vertex_buffer) in &runtime.runtime_vertex_buffers {
        validate_runtime_vertex_buffer_input(symbol, vertex_buffer, bundle)?;
    }
    for (symbol, input_layout) in &runtime.runtime_input_layouts {
        validate_runtime_input_layout_input(symbol, input_layout)?;
    }
    if !runtime.map_pow2.x.is_finite()
        || !runtime.map_pow2.y.is_finite()
        || runtime.map_pow2.x <= 0.0
        || runtime.map_pow2.y <= 0.0
    {
        return Err("production terrain renderer blocked: MAP_POW2_X/Y must be finite positive runtime values".into());
    }
    if !runtime.fow_pow2.x.is_finite()
        || !runtime.fow_pow2.y.is_finite()
        || runtime.fow_pow2.x <= 0.0
        || runtime.fow_pow2.y <= 0.0
    {
        return Err("production terrain renderer blocked: FOW_POW2_X/Y must be finite positive runtime values".into());
    }
    if runtime.terrain_id_map.source.provenance.trim().is_empty()
        || runtime.map_pow2.source.provenance.trim().is_empty()
        || runtime.fow_pow2.source.provenance.trim().is_empty()
    {
        return Err(
            "production terrain renderer blocked: runtime terrain inputs must carry provenance labels"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_required_runtime_renderer_inputs(
    runtime: &TerrainRuntimeInputs,
) -> Result<()> {
    if !runtime
        .runtime_vertex_buffers
        .contains_key(TERRAIN_VERTEX_BUFFER_SYMBOL)
    {
        return Err(format!(
            "production terrain renderer blocked: missing runtime {TERRAIN_VERTEX_BUFFER_SYMBOL}; terrain height and vertex-derived shader inputs must come from the game's active pdxmap draw"
        )
        .into());
    }
    if !runtime
        .runtime_input_layouts
        .contains_key(TERRAIN_INPUT_LAYOUT_SYMBOL)
    {
        return Err(format!(
            "production terrain renderer blocked: missing runtime {TERRAIN_INPUT_LAYOUT_SYMBOL}; D3D11 input layout descriptors are required to decode the captured terrain vertex buffer"
        )
        .into());
    }
    let input_layout = runtime
        .runtime_input_layouts
        .get(TERRAIN_INPUT_LAYOUT_SYMBOL)
        .expect("contains_key checked above");
    let nonzero_slots: BTreeSet<u32> = input_layout
        .elements
        .iter()
        .map(|element| element.input_slot)
        .filter(|slot| *slot != 0)
        .collect();
    if !nonzero_slots.is_empty() {
        return Err(format!(
            "production terrain renderer blocked: {TERRAIN_INPUT_LAYOUT_SYMBOL} uses input slot(s) {:?}, but runtime {TERRAIN_VERTEX_BUFFER_SYMBOL} metadata currently proves only slot 0; capture/import per-slot terrain vertex buffers before decoding the game terrain vertices",
            nonzero_slots
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_terrain_vertex_buffer_capture_file(
    capture: &TerrainVertexBufferCaptureFile,
) -> Result<()> {
    if capture.schema != TERRAIN_VERTEX_BUFFER_CAPTURE_SCHEMA {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain vertex buffer capture schema {}",
            capture.schema
        )
        .into());
    }
    if capture.symbol != TERRAIN_VERTEX_BUFFER_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain vertex buffer capture symbol {}",
            capture.symbol
        )
        .into());
    }
    validate_terrain_vertex_buffer_metadata(
        capture.source_byte_width,
        capture.captured_byte_len,
        capture.stride,
        capture.offset,
        capture.terrain_id_srv_slot,
        &capture.minimum_stride_source,
        &capture.draw_call,
    )
}

pub(crate) fn validate_terrain_input_layout_capture_file(
    capture: &TerrainInputLayoutCaptureFile,
) -> Result<()> {
    if capture.schema != TERRAIN_INPUT_LAYOUT_CAPTURE_SCHEMA {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain input layout capture schema {}",
            capture.schema
        )
        .into());
    }
    if capture.symbol != TERRAIN_INPUT_LAYOUT_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain input layout capture symbol {}",
            capture.symbol
        )
        .into());
    }
    validate_terrain_input_layout_metadata(
        capture.element_count,
        &capture.elements,
        capture.vertex_stride,
        capture.vertex_offset,
        capture.terrain_id_srv_slot,
        &capture.shader_input_source,
        &capture.draw_call,
    )
}

pub(crate) fn validate_runtime_input_layout_input(
    symbol: &str,
    input_layout: &RuntimeInputLayoutRuntimeMeta,
) -> Result<()> {
    if symbol != TERRAIN_INPUT_LAYOUT_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported runtime input layout symbol {symbol}"
        )
        .into());
    }
    if input_layout.format != RUNTIME_INPUT_LAYOUT_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: runtime input layout {symbol} has unsupported format {}",
            input_layout.format
        )
        .into());
    }
    validate_terrain_input_layout_metadata(
        input_layout.element_count,
        &input_layout.elements,
        input_layout.vertex_stride,
        input_layout.vertex_offset,
        input_layout.terrain_id_srv_slot,
        &input_layout.shader_input_source,
        &input_layout.draw_call,
    )?;
    if input_layout.source.provenance.trim().is_empty()
        || input_layout.source.extraction.trim().is_empty()
    {
        return Err(format!(
            "production terrain renderer blocked: runtime input layout {symbol} must carry provenance and extraction labels"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_terrain_input_layout_metadata(
    element_count: usize,
    elements: &[RuntimeInputElementMeta],
    vertex_stride: u32,
    vertex_offset: u32,
    terrain_id_srv_slot: u32,
    shader_input_source: &str,
    draw_call: &str,
) -> Result<()> {
    if element_count == 0 || elements.is_empty() {
        return Err(
            "production terrain renderer blocked: terrain input layout elements must be nonzero"
                .into(),
        );
    }
    if element_count != elements.len() {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout element_count {element_count} does not match {} elements",
            elements.len()
        )
        .into());
    }
    if vertex_stride == 0 {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout vertex stride {vertex_stride} is invalid"
        )
        .into());
    }
    if vertex_offset % 4 != 0 {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout vertex offset {vertex_offset} is not 4-byte aligned"
        )
        .into());
    }
    if terrain_id_srv_slot == 0 {
        return Err(
            "production terrain renderer blocked: terrain input layout capture must name the active TerrainIDMap SRV slot"
                .into(),
        );
    }
    if shader_input_source != TERRAIN_INPUT_LAYOUT_SHADER_SOURCE {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout source {shader_input_source} does not match {TERRAIN_INPUT_LAYOUT_SHADER_SOURCE}"
        )
        .into());
    }
    if !matches!(
        draw_call,
        "Draw" | "DrawIndexed" | "DrawInstanced" | "DrawIndexedInstanced"
    ) {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain input layout draw call {draw_call}"
        )
        .into());
    }
    validate_terrain_input_element(
        elements,
        InputElementContract {
            semantic_name: "POSITION",
            semantic_index: 0,
            input_slot: 0,
            component_count: 4,
            allow_signed_normalized: false,
        },
        vertex_stride,
    )?;
    validate_terrain_input_element(
        elements,
        InputElementContract {
            semantic_name: "TEXCOORD",
            semantic_index: 0,
            input_slot: 1,
            component_count: 2,
            allow_signed_normalized: false,
        },
        vertex_stride,
    )
}

pub(crate) struct InputElementContract {
    pub(crate) semantic_name: &'static str,
    pub(crate) semantic_index: u32,
    pub(crate) input_slot: u32,
    pub(crate) component_count: u32,
    pub(crate) allow_signed_normalized: bool,
}

pub(crate) fn validate_terrain_input_element(
    elements: &[RuntimeInputElementMeta],
    contract: InputElementContract,
    vertex_stride: u32,
) -> Result<()> {
    let element = elements
        .iter()
        .find(|element| {
            element
                .semantic_name
                .eq_ignore_ascii_case(contract.semantic_name)
                && element.semantic_index == contract.semantic_index
        })
        .ok_or_else(|| {
            format!(
                "production terrain renderer blocked: terrain input layout missing {}{}",
                contract.semantic_name, contract.semantic_index
            )
        })?;
    let Some(format) = terrain_numeric_input_format(element.format) else {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} format {} is not {}",
            contract.semantic_name,
            contract.semantic_index,
            element.format,
            contract.expected_format_label()
        )
        .into());
    };
    if format.component_count != contract.component_count {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} format {} is not {}",
            contract.semantic_name,
            contract.semantic_index,
            element.format,
            contract.expected_format_label()
        )
        .into());
    }
    if format.class == NumericInputFormatClass::SignedNormalized
        && !contract.allow_signed_normalized
    {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} format {} is not {}",
            contract.semantic_name,
            contract.semantic_index,
            element.format,
            contract.expected_format_label()
        )
        .into());
    }
    if element.input_slot != contract.input_slot {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} input slot {} does not match expected slot {}",
            contract.semantic_name, contract.semantic_index, element.input_slot, contract.input_slot
        )
        .into());
    }
    if element.aligned_byte_offset == D3D11_APPEND_ALIGNED_ELEMENT {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} uses D3D11_APPEND_ALIGNED_ELEMENT; append offset resolution is not implemented",
            contract.semantic_name, contract.semantic_index
        )
        .into());
    }
    if element.input_slot_class != D3D11_INPUT_PER_VERTEX_DATA {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} input slot class {} is not D3D11_INPUT_PER_VERTEX_DATA",
            contract.semantic_name, contract.semantic_index, element.input_slot_class
        )
        .into());
    }
    if element.instance_data_step_rate != 0 {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} has nonzero instance step rate {}",
            contract.semantic_name, contract.semantic_index, element.instance_data_step_rate
        )
        .into());
    }
    if element
        .aligned_byte_offset
        .saturating_add(format.byte_width)
        > vertex_stride
    {
        return Err(format!(
            "production terrain renderer blocked: terrain input layout {}{} offset {} plus {} bytes exceeds stride {}",
            contract.semantic_name,
            contract.semantic_index,
            element.aligned_byte_offset,
            format.byte_width,
            vertex_stride
        )
        .into());
    }
    Ok(())
}

impl InputElementContract {
    pub(crate) fn expected_format_label(&self) -> &'static str {
        match self.component_count {
            4 => "a signed integer DXGI format with 4 components for HLSL int4",
            2 => "a signed integer DXGI format with 2 components for HLSL int2",
            _ => "a signed integer DXGI format matching the HLSL input component count",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NumericInputFormat {
    pub(crate) component_count: u32,
    pub(crate) byte_width: u32,
    pub(crate) class: NumericInputFormatClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericInputFormatClass {
    SignedInteger,
    SignedNormalized,
}

pub(crate) fn terrain_numeric_input_format(format: u32) -> Option<NumericInputFormat> {
    match format {
        DXGI_FORMAT_R32G32B32A32_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 16,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R16G16B16A16_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 8,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R8G8B8A8_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 4,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R32G32_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 8,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R16G16_SNORM => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 4,
            class: NumericInputFormatClass::SignedNormalized,
        }),
        DXGI_FORMAT_R16G16_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 4,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R8G8_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 2,
            class: NumericInputFormatClass::SignedInteger,
        }),
        _ => None,
    }
}

pub(crate) fn d3d11_input_format_byte_width(format: u32) -> Option<u32> {
    terrain_numeric_input_format(format)
        .map(|format| format.byte_width)
        .or_else(|| match format {
            DXGI_FORMAT_R32G32B32A32_FLOAT => Some(16),
            DXGI_FORMAT_R32G32B32_FLOAT => Some(12),
            DXGI_FORMAT_R32G32_FLOAT => Some(8),
            DXGI_FORMAT_R16G16B16A16_FLOAT => Some(8),
            _ => None,
        })
}

pub(crate) fn validate_runtime_vertex_buffer_input(
    symbol: &str,
    vertex_buffer: &RuntimeVertexBufferRuntimeMeta,
    bundle: &Path,
) -> Result<()> {
    if symbol != TERRAIN_VERTEX_BUFFER_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported runtime vertex buffer symbol {symbol}"
        )
        .into());
    }
    if vertex_buffer.format != RUNTIME_VERTEX_BUFFER_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: runtime vertex buffer {symbol} has unsupported format {}",
            vertex_buffer.format
        )
        .into());
    }
    validate_terrain_vertex_buffer_metadata(
        vertex_buffer.source_byte_width,
        vertex_buffer.captured_byte_len,
        vertex_buffer.stride,
        vertex_buffer.offset,
        vertex_buffer.terrain_id_srv_slot,
        &vertex_buffer.minimum_stride_source,
        &vertex_buffer.draw_call,
    )?;
    let path = bundle.join(&vertex_buffer.file);
    let actual_len = fs::metadata(&path)?.len();
    if actual_len != vertex_buffer.captured_byte_len as u64 {
        return Err(format!(
            "production terrain renderer blocked: runtime vertex buffer {symbol} file {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            vertex_buffer.captured_byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != vertex_buffer.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: runtime vertex buffer {symbol} hash changed from {} to {}",
            vertex_buffer.fnv1a64, actual_hash
        )
        .into());
    }
    if vertex_buffer.source.provenance.trim().is_empty()
        || vertex_buffer.source.extraction.trim().is_empty()
    {
        return Err(format!(
            "production terrain renderer blocked: runtime vertex buffer {symbol} must carry provenance and extraction labels"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_terrain_vertex_buffer_metadata(
    source_byte_width: usize,
    captured_byte_len: usize,
    stride: u32,
    offset: u32,
    terrain_id_srv_slot: u32,
    minimum_stride_source: &str,
    draw_call: &str,
) -> Result<()> {
    if source_byte_width == 0 || captured_byte_len == 0 {
        return Err(
            "production terrain renderer blocked: terrain vertex buffer byte counts must be nonzero"
                .into(),
        );
    }
    if captured_byte_len > source_byte_width {
        return Err(format!(
            "production terrain renderer blocked: terrain vertex buffer captured {} bytes from {} byte source",
            captured_byte_len, source_byte_width
        )
        .into());
    }
    if stride == 0 {
        return Err(format!(
            "production terrain renderer blocked: terrain vertex buffer stride {stride} is invalid"
        )
        .into());
    }
    if stride % 4 != 0 {
        return Err(format!(
            "production terrain renderer blocked: terrain vertex buffer stride {stride} is not 4-byte aligned"
        )
        .into());
    }
    if offset as usize >= source_byte_width
        || (offset as usize).saturating_add(stride as usize) > source_byte_width
    {
        return Err(format!(
            "production terrain renderer blocked: terrain vertex buffer offset {offset} and stride {stride} exceed source byte width {source_byte_width}"
        )
        .into());
    }
    if terrain_id_srv_slot == 0 {
        return Err(
            "production terrain renderer blocked: terrain vertex buffer capture must name the active TerrainIDMap SRV slot"
                .into(),
        );
    }
    if !minimum_stride_source.contains("VS_INPUT_TERRAIN_NOTEXTURE")
        || !minimum_stride_source.contains("int4 position + int2 height")
    {
        return Err(
            "production terrain renderer blocked: terrain vertex buffer minimum stride source must cite pdxmap.shader VS_INPUT_TERRAIN_NOTEXTURE"
                .into(),
        );
    }
    if !matches!(
        draw_call,
        "Draw" | "DrawIndexed" | "DrawInstanced" | "DrawIndexedInstanced"
    ) {
        return Err(format!(
            "production terrain renderer blocked: unsupported terrain vertex buffer draw call {draw_call}"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_material_mip_outputs(
    contract: &MapMaterialContract,
    bundle: &Path,
) -> Result<()> {
    for (source, meta) in &contract.shipped_sources {
        match meta.preservation.as_str() {
            "decoded_rgba_exact_2d_with_mip_chain" => {
                if meta.mip_count <= 1 {
                    return Err(format!(
                        "production terrain renderer blocked: {source} declares mip-chain preservation but DDS mip_count is {}",
                        meta.mip_count
                    )
                    .into());
                }
                let output = meta.mip_output.as_deref().ok_or_else(|| {
                    format!(
                        "production terrain renderer blocked: {source} has {} DDS mips but no exact mip-chain output",
                        meta.mip_count
                    )
                })?;
                let path = required_bundle_file(bundle, source, output)?;
                read_rgba_mip_chain_raw(
                    &path,
                    meta.dimensions.width,
                    meta.dimensions.height,
                    meta.mip_count,
                )?;
            }
            "decoded_rgba_exact_2d" => {
                if meta.mip_output.is_some() {
                    return Err(format!(
                        "production terrain renderer blocked: {source} declares base-only preservation but has mip_output"
                    )
                    .into());
                }
                if meta.mip_count > 1 {
                    return Err(format!(
                        "production terrain renderer blocked: {source} has {} DDS mips but contract preserved only mip 0",
                        meta.mip_count
                    )
                    .into());
                }
            }
            "raw_dds_preserved" => {
                if meta.mip_output.is_some() {
                    return Err(format!(
                        "production terrain renderer blocked: {source} is raw DDS-preserved but also declares decoded mip_output"
                    )
                    .into());
                }
            }
            other => {
                return Err(format!(
                    "production terrain renderer blocked: {source} has unsupported material preservation mode {other}"
                )
                .into());
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_runtime_constant_buffer_input(
    symbol: &str,
    constant_buffer: &RuntimeConstantBufferRuntimeMeta,
    manifest: &Manifest,
    bundle: &Path,
) -> Result<()> {
    let expected_file = runtime_constant_buffer_file_for_symbol(manifest, symbol)?;
    if constant_buffer.file != expected_file {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} file {} does not match manifest {}",
            constant_buffer.file, expected_file
        )
        .into());
    }
    if constant_buffer.format != RUNTIME_CONSTANT_BUFFER_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} has unsupported format {}",
            constant_buffer.format
        )
        .into());
    }
    let expected_bind_point = runtime_constant_buffer_bind_point_for_symbol(symbol)?;
    if constant_buffer.bind_point != expected_bind_point {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} bind point {} does not match shader slot {}",
            constant_buffer.bind_point, expected_bind_point
        )
        .into());
    }
    let standard_functions_source =
        required_map_source_path(bundle, manifest, "gfx/FX/standardfuncsgfx.fxh")?;
    let standard_functions_text = read_latin1(&standard_functions_source)?;
    let layout = terrain_scene_cb0_layout(&standard_functions_text)?;
    if constant_buffer.layout_source != TERRAIN_SCENE_CB0_LAYOUT_SOURCE {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} layout source {} does not match {}",
            constant_buffer.layout_source, TERRAIN_SCENE_CB0_LAYOUT_SOURCE
        )
        .into());
    }
    if constant_buffer.layout_byte_len != layout.byte_len {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} layout byte_len {} does not match shader layout {}",
            constant_buffer.layout_byte_len, layout.byte_len
        )
        .into());
    }
    let expected_offsets = terrain_scene_cb0_required_offsets(&layout)?;
    if constant_buffer.required_offsets != expected_offsets {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} required offsets do not match {TERRAIN_SCENE_CB0_LAYOUT_SOURCE}"
        )
        .into());
    }
    validate_constant_buffer_byte_len(constant_buffer.byte_len, layout.byte_len)?;
    let path = bundle.join(&constant_buffer.file);
    let actual_len = fs::metadata(&path)?.len();
    if actual_len != constant_buffer.byte_len as u64 {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} file {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            constant_buffer.byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != constant_buffer.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} hash changed from {} to {}",
            constant_buffer.fnv1a64, actual_hash
        )
        .into());
    }
    if constant_buffer.source.provenance.trim().is_empty() {
        return Err(format!(
            "production terrain renderer blocked: runtime constant buffer {symbol} must carry a provenance label"
        )
        .into());
    }
    let bytes = fs::read(path)?;
    let _decoded = decode_terrain_scene_cb0(&bytes, &layout)?;
    Ok(())
}

pub(crate) fn validate_runtime_texture_input(
    symbol: &str,
    texture: &RuntimeTextureRuntimeMeta,
    manifest: &Manifest,
    bundle: &Path,
) -> Result<()> {
    let expected_file = runtime_texture_file_for_symbol(manifest, symbol)?;
    if texture.file != expected_file {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} file {} does not match manifest {}",
            texture.file, expected_file
        )
        .into());
    }
    if texture.width == 0 || texture.height == 0 {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} dimensions must be nonzero"
        )
        .into());
    }
    if texture.format != RUNTIME_RGBA8_FORMAT {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} has unsupported format {}",
            texture.format
        )
        .into());
    }
    let expected_len = checked_rgba_byte_len(texture.width, texture.height)?;
    if texture.byte_len != expected_len {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} metadata byte_len {} does not match {}x{} RGBA byte_len {}",
            texture.byte_len, texture.width, texture.height, expected_len
        )
        .into());
    }
    let path = bundle.join(&texture.file);
    let actual_len = fs::metadata(&path)?.len();
    if actual_len != texture.byte_len as u64 {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} file {} has {} bytes, metadata expects {}",
            path.display(),
            actual_len,
            texture.byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != texture.fnv1a64 {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} hash changed from {} to {}",
            texture.fnv1a64, actual_hash
        )
        .into());
    }
    if texture.source.provenance.trim().is_empty() {
        return Err(format!(
            "production terrain renderer blocked: runtime texture {symbol} must carry a provenance label"
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_runtime_texture_sampler_contract(
    runtime: &TerrainRuntimeInputs,
    pdxmap_shader_source: &Path,
) -> Result<()> {
    let text = read_latin1(pdxmap_shader_source)?;
    for (symbol, texture) in &runtime.runtime_textures {
        let expected = parse_pdxmap_sampler_contract(&text, symbol)?;
        if texture.sampler != expected {
            return Err(format!(
                "production terrain renderer blocked: runtime texture {symbol} sampler changed from {:?} to {:?}",
                texture.sampler, expected
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    use winapi::um::d3dcommon::{D3D11_SRV_DIMENSION_TEXTURE2D, D3D11_SRV_DIMENSION_TEXTURECUBE};

    fn synthetic_terrain_resolver(root: &Path, palette: u8) -> GameFileResolver {
        let game = root.join("game");
        write_synthetic_terrain_id_map_game_files(&game, palette);
        for source in MAP_FRAME_REQUIRED_VANILLA_SOURCES {
            let path = source
                .split('/')
                .fold(game.clone(), |path, component| path.join(component));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create vanilla source fixture directory");
            }
            fs::write(&path, vanilla_source_fixture_bytes(source))
                .expect("write vanilla source fixture");
        }
        GameFileResolver::vanilla(game).expect("synthetic terrain resolver")
    }

    fn static_texture_test_descriptor(
        name: &str,
        slot: u32,
        width: u32,
        height: u32,
        mip_count: u32,
        dxgi_format: u32,
    ) -> MapFrameTextureBinding {
        MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot,
            declared_name: Some(name.to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: dxgi_format,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: mip_count,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format,
            width,
            height,
            mip_count,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: D3D11_USAGE_DEFAULT,
            cpu_access_flags: 0,
            misc_flags: 0,
        }
    }

    fn write_test_dxt5_dds(path: &Path, width: u32, height: u32, mip_count: u32) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create DDS parent");
        }
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(b"DDS ");
        data[4..8].copy_from_slice(&124u32.to_le_bytes());
        data[12..16].copy_from_slice(&height.to_le_bytes());
        data[16..20].copy_from_slice(&width.to_le_bytes());
        data[28..32].copy_from_slice(&mip_count.to_le_bytes());
        data[76..80].copy_from_slice(&32u32.to_le_bytes());
        data[80..84].copy_from_slice(&4u32.to_le_bytes());
        data[84..88].copy_from_slice(b"DXT5");
        for mip in 0..mip_count {
            let mip_width = (width >> mip).max(1);
            let mip_height = (height >> mip).max(1);
            let byte_len = mip_width.div_ceil(4) * mip_height.div_ceil(4) * 16;
            data.extend((0..byte_len).map(|index| (index as u8).wrapping_add(mip as u8)));
        }
        fs::write(path, data).expect("write DXT5 DDS");
    }

    fn write_test_bmp24(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create BMP parent");
        }
        let mut data = vec![0u8; 54];
        data[0..2].copy_from_slice(b"BM");
        data[2..6].copy_from_slice(&62u32.to_le_bytes());
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes());
        data[18..22].copy_from_slice(&2i32.to_le_bytes());
        data[22..26].copy_from_slice(&1i32.to_le_bytes());
        data[26..28].copy_from_slice(&1u16.to_le_bytes());
        data[28..30].copy_from_slice(&24u16.to_le_bytes());
        data.extend_from_slice(&[3, 2, 1, 6, 5, 4, 0, 0]);
        fs::write(path, data).expect("write 24-bit BMP");
    }

    #[test]
    fn descriptor_only_dds_join_reconstructs_captured_row_pitch_law() {
        let root = temp_test_dir("descriptor_only_dds_join");
        let path = root.join("atlas0.dds");
        write_test_dxt5_dds(&path, 8, 8, 3);
        let resource = static_texture_test_descriptor(
            TERRAIN_DIFFUSE_SYMBOL,
            0,
            8,
            8,
            3,
            DXGI_FORMAT_BC3_UNORM_SRGB,
        );

        let payload = static_texture_payload_from_source(
            TERRAIN_DIFFUSE_SYMBOL,
            "map/terrain/atlas0.dds",
            &path,
            &resource,
        )
        .expect("exact DDS descriptor join");

        assert_eq!(payload.subresources.len(), 3);
        assert!(payload
            .subresources
            .iter()
            .all(|subresource| subresource.row_pitch == 128));
        assert_eq!(payload.bytes.len(), 512);
        assert_eq!(&payload.bytes[0..32], &(0u8..32).collect::<Vec<_>>());
        assert!(payload.bytes[32..128].iter().all(|byte| *byte == 0));
        assert_eq!(&payload.bytes[128..160], &(32u8..64).collect::<Vec<_>>());
        assert!(payload.bytes[160..256].iter().all(|byte| *byte == 0));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn descriptor_only_bmp_join_is_top_down_bgrx() {
        let root = temp_test_dir("descriptor_only_bmp_join");
        let path = root.join("world_normal.bmp");
        write_test_bmp24(&path);
        let resource =
            static_texture_test_descriptor("HeightNormal", 1, 2, 1, 1, DXGI_FORMAT_B8G8R8X8_UNORM);

        let payload = static_texture_payload_from_source(
            "HeightNormal",
            "map/world_normal.bmp",
            &path,
            &resource,
        )
        .expect("exact BMP descriptor join");

        assert_eq!(payload.bytes, vec![3, 2, 1, 0, 6, 5, 4, 0]);
        assert_eq!(payload.subresources[0].row_pitch, 8);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn descriptor_only_resolver_join_rejects_ambiguous_exact_sources() {
        let root = temp_test_dir("descriptor_only_ambiguous_join");
        let game = root.join("game");
        for index in 0..=2 {
            write_test_dxt5_dds(
                &game.join(format!("map/terrain/citylights_rgb_snowmask_a_{index}.dds")),
                4,
                4,
                1,
            );
        }
        let resolver = GameFileResolver::vanilla(&game).expect("fixture resolver");
        let resource = static_texture_test_descriptor(
            CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL,
            8,
            4,
            4,
            1,
            DXGI_FORMAT_BC3_UNORM_SRGB,
        );

        let error = resolve_descriptor_only_static_texture(&resolver, &resource)
            .expect_err("ambiguous exact sources must fail closed")
            .to_string();
        assert!(error.contains("3 exact resolver sources"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn descriptor_only_resolver_join_rejects_descriptor_mismatch() {
        let root = temp_test_dir("descriptor_only_mismatch_join");
        let game = root.join("game");
        write_test_dxt5_dds(&game.join("map/terrain/atlas0.dds"), 8, 8, 2);
        let resolver = GameFileResolver::vanilla(&game).expect("fixture resolver");
        let resource = static_texture_test_descriptor(
            TERRAIN_DIFFUSE_SYMBOL,
            0,
            16,
            8,
            2,
            DXGI_FORMAT_BC3_UNORM_SRGB,
        );

        let error = resolve_descriptor_only_static_texture(&resolver, &resource)
            .expect_err("descriptor mismatch must fail closed")
            .to_string();
        assert!(error.contains("no exact resolver source"), "{error}");
        assert!(error.contains("DDS descriptor is 8x8"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    fn write_full_terrain_id_map_frame_bundle(
        root: &Path,
        captured_hash: Option<String>,
        captured_len: usize,
    ) -> PathBuf {
        fs::create_dir_all(root).expect("create frame bundle dir");
        write_valid_map_frame_bundle(root, |mut frame| {
            frame
                .shader_cache_constants
                .as_mut()
                .expect("shader cache constants")
                .map_size_x = TERRAIN_ID_MAP_WIDTH as f32;
            frame
                .shader_cache_constants
                .as_mut()
                .expect("shader cache constants")
                .map_size_y = TERRAIN_ID_MAP_HEIGHT as f32;
            update_draw_terrain_id_resource(
                &mut frame.active_draw,
                captured_hash.clone(),
                captured_len,
            );
            for draw in &mut frame.draws {
                update_draw_terrain_id_resource(draw, captured_hash.clone(), captured_len);
            }
            match captured_hash {
                Some(hash) => {
                    frame.runtime_files.insert(
                        CANONICAL_TERRAIN_ID_MAP_FILE.to_string(),
                        MapFrameFileEvidence {
                            file: CANONICAL_TERRAIN_ID_MAP_FILE.to_string(),
                            byte_len: captured_len,
                            fnv1a64: hash,
                            provenance: "unit_test_full_terrain_id_map".to_string(),
                        },
                    );
                }
                None => {
                    frame.runtime_files.remove(CANONICAL_TERRAIN_ID_MAP_FILE);
                }
            }
            frame
        })
    }

    fn update_draw_terrain_id_resource(
        draw: &mut MapFrameDraw,
        captured_hash: Option<String>,
        captured_len: usize,
    ) {
        let terrain = draw
            .shader_resources
            .iter_mut()
            .find(|resource| {
                resource.stage == "PS"
                    && resource.declared_name.as_deref() == Some(TERRAIN_ID_RESOURCE_SYMBOL)
            })
            .expect("TerrainIDMap resource");
        match captured_hash {
            Some(hash) => update_draw_terrain_id_resource_file(
                terrain,
                CANONICAL_TERRAIN_ID_MAP_FILE,
                &hash,
                captured_len,
            ),
            None => {
                configure_terrain_id_resource_descriptor(terrain);
                terrain.file = None;
                terrain.byte_len = 0;
                terrain.fnv1a64 = None;
                terrain.payload_format = None;
                terrain.subresources = Vec::new();
            }
        }
    }

    fn configure_terrain_id_resource_descriptor(terrain: &mut MapFrameTextureBinding) {
        terrain.view_desc = Some(MapFrameShaderResourceViewDesc {
            format: TERRAIN_ID_MAP_DXGI_FORMAT,
            view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            most_detailed_mip: 0,
            mip_levels: 1,
            first_array_slice: 0,
            array_size: 1,
            num_cubes: 0,
        });
        terrain.dxgi_format = TERRAIN_ID_MAP_DXGI_FORMAT;
        terrain.width = TERRAIN_ID_MAP_WIDTH as u32;
        terrain.height = TERRAIN_ID_MAP_HEIGHT as u32;
        terrain.mip_count = TERRAIN_ID_MAP_MIP_COUNT;
        terrain.array_size = 1;
        terrain.sample_count = 1;
        terrain.sample_quality = 0;
        terrain.bind_flags = D3D11_BIND_SHADER_RESOURCE;
        terrain.usage = D3D11_USAGE_DEFAULT;
        terrain.cpu_access_flags = 0;
        terrain.misc_flags = 0;
    }

    fn update_draw_terrain_id_resource_file(
        terrain: &mut MapFrameTextureBinding,
        file: &str,
        hash: &str,
        captured_len: usize,
    ) {
        configure_terrain_id_resource_descriptor(terrain);
        terrain.file = Some(file.to_string());
        terrain.byte_len = captured_len;
        terrain.fnv1a64 = Some(hash.to_string());
        terrain.payload_format = Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string());
        terrain.subresources = vec![MapFrameTextureSubresource {
            mip_slice: 0,
            array_slice: 0,
            file_offset: 0,
            byte_len: captured_len,
            row_pitch: (TERRAIN_ID_MAP_WIDTH * 4) as u32,
            depth_pitch: captured_len as u32,
            width: TERRAIN_ID_MAP_WIDTH as u32,
            height: TERRAIN_ID_MAP_HEIGHT as u32,
        }];
    }

    fn assert_captured_terrain_id_map_bound(
        frame: &MapFrameRuntimeBundle,
        expected_file: &str,
        expected_hash: &str,
    ) {
        let terrain = map_frame_terrain_srv(&frame.active_draw).expect("TerrainIDMap SRV");
        assert_eq!(terrain.file.as_deref(), Some(expected_file));
        assert_eq!(terrain.fnv1a64.as_deref(), Some(expected_hash));
        assert_eq!(
            terrain.byte_len,
            TERRAIN_ID_MAP_WIDTH * TERRAIN_ID_MAP_HEIGHT * 4
        );
        assert!(!frame
            .synthetic_payloads
            .contains_key(SYNTHESIZED_TERRAIN_ID_MAP_FILE));
    }

    #[test]
    fn map_frame_bundle_validation_accepts_multislot_terrain_draw() {
        let root = temp_test_dir("map_frame_bundle_valid");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);

        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        assert_eq!(frame.active_draw.draw_call, "DrawIndexed");
        assert_eq!(
            frame.active_draw.shader.reflection_resources["TerrainIDMap"],
            5
        );
        assert!(frame.active_draw.vertex_buffers.contains_key("0"));
        assert!(frame.active_draw.vertex_buffers.contains_key("1"));
        assert_eq!(frame.active_draw.input_layout.elements[1].input_slot, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_accepts_matching_effective_vfs_shader_sources() {
        let root = temp_test_dir("map_frame_effective_vfs_sources_match");
        let resolver = synthetic_terrain_resolver(&root, 0);
        let bundle_dir = root.join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create map-frame bundle directory");
        let path = write_valid_map_frame_bundle(&bundle_dir, |frame| frame);
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid captured bundle");

        validate_map_frame_vanilla_sources(
            &frame,
            &bundle_dir,
            TERRAIN_MAP_FRAME_SPEC,
            Some(&resolver),
        )
        .expect("matching effective-VFS source identities must validate");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_rejects_changed_effective_vfs_shader_source() {
        let root = temp_test_dir("map_frame_effective_vfs_source_changed");
        let resolver = synthetic_terrain_resolver(&root, 0);
        fs::write(
            root.join("game/gfx/FX/pdxmap.shader"),
            b"changed after replay capture",
        )
        .expect("change selected effective shader source");
        let bundle_dir = root.join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create map-frame bundle directory");
        let path = write_valid_map_frame_bundle(&bundle_dir, |frame| frame);
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid captured bundle");

        let error = validate_map_frame_vanilla_sources(
            &frame,
            &bundle_dir,
            TERRAIN_MAP_FRAME_SPEC,
            Some(&resolver),
        )
        .expect_err("changed effective-VFS source must fail closed")
        .to_string();
        assert!(
            error.contains("effective-VFS source gfx/FX/pdxmap.shader"),
            "{error}"
        );
        assert!(
            error.contains("does not match the replay-captured content identity"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_uses_mod_override_only_when_content_identity_matches() {
        let root = temp_test_dir("map_frame_effective_vfs_mod_override");
        let vanilla_resolver = synthetic_terrain_resolver(&root, 0);
        drop(vanilla_resolver);
        let game = root.join("game");
        let mod_root = root.join("mod");
        let override_path = mod_root.join("gfx/FX/pdxmap.shader");
        fs::create_dir_all(override_path.parent().expect("override parent"))
            .expect("create mod override directory");
        fs::write(
            &override_path,
            vanilla_source_fixture_bytes("gfx/FX/pdxmap.shader"),
        )
        .expect("write byte-identical mod override");
        let resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root.clone()])
            .expect("modded resolver");
        let bundle_dir = root.join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create map-frame bundle directory");
        let path = write_valid_map_frame_bundle(&bundle_dir, |frame| frame);
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid captured bundle");

        validate_map_frame_vanilla_sources(
            &frame,
            &bundle_dir,
            TERRAIN_MAP_FRAME_SPEC,
            Some(&resolver),
        )
        .expect("byte-identical selected mod override must validate");

        fs::write(&override_path, b"different selected mod shader")
            .expect("change mod override content");
        let changed_resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root])
            .expect("changed modded resolver");
        let error = validate_map_frame_vanilla_sources(
            &frame,
            &bundle_dir,
            TERRAIN_MAP_FRAME_SPEC,
            Some(&changed_resolver),
        )
        .expect_err("changed selected mod override must fail closed")
        .to_string();
        assert!(
            error.contains("does not match the replay-captured content identity"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_rejects_replace_path_that_removes_required_shader() {
        let root = temp_test_dir("map_frame_effective_vfs_replace_path");
        let vanilla_resolver = synthetic_terrain_resolver(&root, 0);
        drop(vanilla_resolver);
        let game = root.join("game");
        let mod_root = root.join("replace_mod");
        fs::create_dir_all(&mod_root).expect("create replace-path mod");
        fs::write(
            mod_root.join("descriptor.mod"),
            b"name=\"Replace FX\"\nreplace_path=\"gfx/FX\"\n",
        )
        .expect("write replace-path descriptor");
        let resolver =
            GameFileResolver::with_mod_paths(&game, vec![mod_root]).expect("replace-path resolver");
        let bundle_dir = root.join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create map-frame bundle directory");
        let path = write_valid_map_frame_bundle(&bundle_dir, |frame| frame);
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid captured bundle");

        let error = validate_map_frame_vanilla_sources(
            &frame,
            &bundle_dir,
            TERRAIN_MAP_FRAME_SPEC,
            Some(&resolver),
        )
        .expect_err("replace_path removing a required source must fail closed")
        .to_string();
        assert!(error.contains("gfx/FX/pdxmap.shader"), "{error}");
        assert!(error.contains("missing effective-VFS source"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_keeps_authenticated_captured_terrain_id_map() {
        let root = temp_test_dir("terrain_id_map_synth_matches_capture");
        let resolver = synthetic_terrain_resolver(&root, 0);
        let synthesized = synthesize_terrain_id_map(&resolver).expect("synthesize TerrainIDMap");
        let bundle_dir = root.join("bundle");
        let path = write_full_terrain_id_map_frame_bundle(
            &bundle_dir,
            Some(synthesized.descriptor.fnv1a64.clone()),
            synthesized.bytes.len(),
        );
        fs::write(bundle_dir.join("terrain_id_map.rgba"), &synthesized.bytes)
            .expect("write captured TerrainIDMap");

        let frame = validate_map_frame_runtime_bundle_with_resolver(&path, &resolver)
            .expect("authenticated captured payload validates");

        assert_captured_terrain_id_map_bound(
            &frame,
            CANONICAL_TERRAIN_ID_MAP_FILE,
            &synthesized.descriptor.fnv1a64,
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_keeps_authenticated_srv_payload_without_canonical_substitution() {
        let root = temp_test_dir("terrain_id_map_synth_canonical_payload");
        let resolver = synthetic_terrain_resolver(&root, 0);
        let synthesized = synthesize_terrain_id_map(&resolver).expect("synthesize TerrainIDMap");
        let mut srv_payload = synthesized.bytes.clone();
        srv_payload[0] ^= 0xff;
        let srv_hash = terrain_id_map_payload_fnv1a64(&srv_payload);
        let srv_file = "terrain_id_map_srv_copy.rgba";
        let bundle_dir = root.join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create bundle dir");
        let path = write_valid_map_frame_bundle(&bundle_dir, |mut frame| {
            frame
                .shader_cache_constants
                .as_mut()
                .expect("shader cache constants")
                .map_size_x = TERRAIN_ID_MAP_WIDTH as f32;
            frame
                .shader_cache_constants
                .as_mut()
                .expect("shader cache constants")
                .map_size_y = TERRAIN_ID_MAP_HEIGHT as f32;
            frame.runtime_files.insert(
                CANONICAL_TERRAIN_ID_MAP_FILE.to_string(),
                MapFrameFileEvidence {
                    file: CANONICAL_TERRAIN_ID_MAP_FILE.to_string(),
                    byte_len: synthesized.bytes.len(),
                    fnv1a64: synthesized.descriptor.fnv1a64.clone(),
                    provenance: "unit_test_canonical_terrain_id_map".to_string(),
                },
            );
            frame.runtime_files.insert(
                srv_file.to_string(),
                MapFrameFileEvidence {
                    file: srv_file.to_string(),
                    byte_len: srv_payload.len(),
                    fnv1a64: srv_hash.clone(),
                    provenance: "unit_test_authenticated_srv_terrain_id_map".to_string(),
                },
            );
            update_draw_terrain_id_resource_file(
                frame
                    .active_draw
                    .shader_resources
                    .iter_mut()
                    .find(|resource| {
                        resource.stage == "PS"
                            && resource.declared_name.as_deref() == Some(TERRAIN_ID_RESOURCE_SYMBOL)
                    })
                    .expect("active TerrainIDMap resource"),
                srv_file,
                &srv_hash,
                srv_payload.len(),
            );
            for draw in &mut frame.draws {
                update_draw_terrain_id_resource_file(
                    draw.shader_resources
                        .iter_mut()
                        .find(|resource| {
                            resource.stage == "PS"
                                && resource.declared_name.as_deref()
                                    == Some(TERRAIN_ID_RESOURCE_SYMBOL)
                        })
                        .expect("draw TerrainIDMap resource"),
                    srv_file,
                    &srv_hash,
                    srv_payload.len(),
                );
            }
            frame
        });
        fs::write(
            bundle_dir.join(CANONICAL_TERRAIN_ID_MAP_FILE),
            &synthesized.bytes,
        )
        .expect("write canonical TerrainIDMap");
        fs::write(bundle_dir.join(srv_file), &srv_payload).expect("write divergent SRV copy");

        let frame = validate_map_frame_runtime_bundle_with_resolver(&path, &resolver)
            .expect("authenticated SRV payload validates without canonical substitution");

        assert_captured_terrain_id_map_bound(&frame, srv_file, &srv_hash);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_rejects_withheld_terrain_id_map_even_with_resolver() {
        let root = temp_test_dir("terrain_id_map_synth_without_payload");
        let resolver = synthetic_terrain_resolver(&root, 0);
        let bundle_dir = root.join("bundle");
        let path = write_full_terrain_id_map_frame_bundle(&bundle_dir, None, 0);

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("no resolver path must still require captured TerrainIDMap payload");
        assert!(err.to_string().contains("TerrainIDMap"));

        let resolver_error = validate_map_frame_runtime_bundle_with_resolver(&path, &resolver)
            .expect_err("resolver must not substitute an unauthenticated TerrainIDMap");
        assert!(resolver_error.to_string().contains("TerrainIDMap"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_accepts_self_consistent_captured_terrain_id_map_without_resynthesis() {
        let root = temp_test_dir("terrain_id_map_synth_mismatch");
        let resolver = synthetic_terrain_resolver(&root, 0);
        let synthesized = synthesize_terrain_id_map(&resolver).expect("synthesize TerrainIDMap");
        let mut captured = synthesized.bytes.clone();
        captured[0] ^= 0xff;
        let captured_hash = terrain_id_map_payload_fnv1a64(&captured);
        let bundle_dir = root.join("bundle");
        let path = write_full_terrain_id_map_frame_bundle(
            &bundle_dir,
            Some(captured_hash.clone()),
            captured.len(),
        );
        fs::write(bundle_dir.join("terrain_id_map.rgba"), &captured)
            .expect("write mismatched captured TerrainIDMap");

        let frame = validate_map_frame_runtime_bundle_with_resolver(&path, &resolver)
            .expect("self-consistent captured payload is authoritative");
        assert_captured_terrain_id_map_bound(&frame, CANONICAL_TERRAIN_ID_MAP_FILE, &captured_hash);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_shader_constants_load_from_self_contained_bundle() {
        let root = temp_test_dir("map_frame_self_contained_shader_constants");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);

        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let constants =
            load_map_frame_terrain_shader_constants(&frame, &root.join("missing_map_bundle"))
                .expect("shader constants come from map-frame bundle");
        assert_eq!(
            constants,
            MapFrameTerrainShaderConstants {
                map_size_x: 2.0,
                map_size_y: 2.0,
                map_pow2_x: 1.0,
                map_pow2_y: 1.0,
                fow_pow2_x: 1.0,
                fow_pow2_y: 1.0,
            }
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_shader_cache_slot_mismatch() {
        let root = temp_test_dir("map_frame_shader_cache_slot_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .shader_cache_constants
                .as_mut()
                .expect("shader constants")
                .terrain_id_srv_slot = 3;
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("shader-cache slot mismatch must fail closed");
        assert!(err
            .to_string()
            .contains("active shader reflection declares t5"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_input_slot_buffer() {
        let root = temp_test_dir("map_frame_bundle_missing_slot");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.vertex_buffers.remove("1");
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("slot 1 input without vertex buffer must fail closed");
        assert!(err.to_string().contains("slot 1"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_terrain_sampler() {
        let root = temp_test_dir("map_frame_bundle_missing_sampler");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.samplers.clear();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("TerrainIDMap sampler proof must fail closed");
        assert!(err.to_string().contains("sampler-state proof"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_declared_snow_mud_without_captured_bytes() {
        let root = temp_test_dir("map_frame_bundle_missing_snow_mud_bytes");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .shader
                .reflection_resources
                .insert(SNOW_MUD_DATA_SYMBOL.to_string(), 7);
            frame
                .active_draw
                .shader_resources
                .push(MapFrameTextureBinding {
                    stage: "PS".to_string(),
                    slot: 7,
                    declared_name: Some(SNOW_MUD_DATA_SYMBOL.to_string()),
                    is_null: false,
                    view_desc: None,
                    file: None,
                    byte_len: 0,
                    fnv1a64: None,
                    dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    width: 2,
                    height: 2,
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
                });
            frame.active_draw.samplers.push(MapFrameSamplerBinding {
                stage: "PS".to_string(),
                slot: 7,
                filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                address_u: D3D11_TEXTURE_ADDRESS_WRAP,
                address_v: D3D11_TEXTURE_ADDRESS_WRAP,
                address_w: D3D11_TEXTURE_ADDRESS_WRAP,
                mip_lod_bias: 0.0,
                max_anisotropy: 1,
                comparison_func: 0,
                border_color: [0.0; 4],
                min_lod: 0.0,
                max_lod: f32::MAX,
            });
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("declared SnowMudData without bytes must fail closed");
        assert!(err.to_string().contains(SNOW_MUD_DATA_SYMBOL));
        assert!(err.to_string().contains("captured bytes and hash"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_wrong_snow_mud_sampler() {
        let root = temp_test_dir("map_frame_bundle_wrong_snow_mud_sampler");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            add_snow_mud_resource(&root, &mut frame);
            frame
                .active_draw
                .samplers
                .iter_mut()
                .find(|sampler| sampler.slot == 7)
                .expect("SnowMudData sampler")
                .address_u = D3D11_TEXTURE_ADDRESS_CLAMP;
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("wrong SnowMudData sampler must fail closed");
        assert!(err.to_string().contains(SNOW_MUD_DATA_SYMBOL));
        assert!(err.to_string().contains("linear/wrap"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_runtime_rgba_resource_accepts_bgra_srgb_source_format() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 6,
            declared_name: Some(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL.to_string()),
            is_null: false,
            view_desc: None,
            file: Some("province_secondary_color_map.rgba".to_string()),
            byte_len: 16,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            width: 2,
            height: 2,
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
        validate_map_frame_runtime_rgba_resource(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL, &resource)
            .expect("BGRA SRGB runtime source can be captured as canonical RGBA bytes");
    }

    #[test]
    fn map_frame_runtime_resource_accepts_light_data_float_format() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 11,
            declared_name: Some(LIGHT_DATA_MAP_SYMBOL.to_string()),
            is_null: false,
            view_desc: None,
            file: Some("light_data_map.r32g32b32a32_float".to_string()),
            byte_len: 128 * 16,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            width: 128,
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
        validate_map_frame_runtime_resource(LIGHT_DATA_MAP_SYMBOL, &resource)
            .expect("LightDataMap is captured as raw R32G32B32A32_FLOAT bytes");

        let wrong_format = MapFrameTextureBinding {
            dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..resource.clone()
        };
        let err = validate_map_frame_runtime_resource(LIGHT_DATA_MAP_SYMBOL, &wrong_format)
            .expect_err("LightDataMap must not be accepted as RGBA8");
        assert!(err.to_string().contains("unsupported source DXGI format"));
    }

    #[test]
    fn map_frame_post_diffuse_shadow_map_accepts_reflected_null_srv() {
        let root = temp_test_dir("map_frame_post_diffuse_null_shadow_map");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            add_null_shadow_map_resource(&mut frame);
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");

        let mut blockers = Vec::new();
        validate_map_frame_post_diffuse_runtime_resource(
            &frame,
            &frame.active_draw,
            None,
            SHADOW_MAP_SYMBOL,
            &mut blockers,
        );
        assert!(blockers.is_empty(), "{blockers:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_post_diffuse_gate_reports_unproven_shader_inputs() {
        let root = temp_test_dir("map_frame_post_diffuse_gate");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let map_bundle = root.join("map_bundle");
        write_post_diffuse_material_bundle(&map_bundle);
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            add_snow_mud_resource(&root, &mut frame);
            add_post_diffuse_shipped_resources(&mut frame);
            add_uncaptured_post_diffuse_runtime_resources(&mut frame);
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path)
            .expect("base map-frame validation should accept SnowMudData");

        let err = validate_map_frame_post_diffuse_inputs(&frame, &map_bundle, &root)
            .expect_err("post-diffuse shader path must fail closed until all inputs are proven");
        let message = err.to_string();
        assert!(!message.contains(SNOW_TEXTURE_SYMBOL));
        assert!(!message.contains(CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL));
        assert!(!message.contains(MUD_DIFFUSE_GLOSS_SYMBOL));
        assert!(!message.contains(MUD_NORMAL_SPEC_SYMBOL));
        assert!(message.contains(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL));
        assert!(message.contains(GRADIENT_BORDER_CHANNEL1_SYMBOL));
        assert!(message.contains(LIGHT_INDEX_MAP_SYMBOL));
        assert!(message.contains(SHADOW_MAP_SYMBOL));
        assert!(!message.contains("vScreenCoord"));
        assert!(!message.contains("vShadowProj"));
        assert!(message.contains("viewport"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_post_diffuse_gate_checks_each_draw_binding() {
        let root = temp_test_dir("map_frame_post_diffuse_each_draw");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let map_bundle = root.join("map_bundle");
        write_post_diffuse_material_bundle(&map_bundle);
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            add_snow_mud_resource(&root, &mut frame);
            add_post_diffuse_shipped_resources(&mut frame);
            add_uncaptured_post_diffuse_runtime_resources(&mut frame);
            frame
        });
        let mut frame = validate_map_frame_runtime_bundle(&path)
            .expect("base map-frame validation should accept draw sequence");
        frame.draws[0]
            .shader
            .reflection_resources
            .remove(SNOW_TEXTURE_SYMBOL);

        let err = validate_map_frame_post_diffuse_inputs(&frame, &map_bundle, &root)
            .expect_err("post-diffuse gate must inspect every captured draw");
        let message = err.to_string();
        assert!(message.contains("draw 0"), "{message}");
        assert!(message.contains(SNOW_TEXTURE_SYMBOL), "{message}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_post_diffuse_gate_requires_replayed_vertex_interpolants() {
        let root = temp_test_dir("map_frame_post_diffuse_missing_vs_cb0");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let map_bundle = root.join("map_bundle");
        write_post_diffuse_material_bundle(&map_bundle);
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            add_snow_mud_resource(&root, &mut frame);
            add_post_diffuse_shipped_resources(&mut frame);
            add_uncaptured_post_diffuse_runtime_resources(&mut frame);
            frame
                .active_draw
                .constant_buffers
                .retain(|buffer| !(buffer.stage == "VS" && buffer.slot == 0));
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path)
            .expect("base map-frame validation should accept PS TerrainSceneCB0");

        let err = validate_map_frame_post_diffuse_inputs(&frame, &map_bundle, &root)
            .expect_err("post-diffuse shader path must fail closed without VS interpolant proof");
        let message = err.to_string();
        assert!(
            message.contains("vScreenCoord/vShadowProj replay blocked"),
            "{message}"
        );
        assert!(message.contains(TERRAIN_SCENE_CB0_SYMBOL));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_post_diffuse_state_gate_accepts_captured_d3d11_state() {
        let root = temp_test_dir("map_frame_post_diffuse_state_gate");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        let mut frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let missing = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(missing.iter().any(|blocker| blocker.contains("viewport")));

        frame.active_draw.draw_state = Some(valid_map_frame_draw_state());
        let blockers = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(blockers.is_empty(), "{blockers:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_post_diffuse_state_gate_requires_om_depth_evidence_when_depth_enabled() {
        let root = temp_test_dir("map_frame_post_diffuse_om_depth_gate");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        let mut frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let mut state = valid_map_frame_draw_state();
        state.depth_stencil = MapFrameDepthStencilState {
            default_state: false,
            stencil_ref: 0,
            desc: Some(valid_depth_enabled_desc()),
        };
        frame.active_draw.draw_state = Some(state);

        let blockers = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("output-merger")),
            "{blockers:?}"
        );

        frame
            .active_draw
            .draw_state
            .as_mut()
            .expect("draw state")
            .output_merger = Some(valid_output_merger_state());
        let blockers = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(blockers.is_empty(), "{blockers:?}");

        let output_merger = frame
            .active_draw
            .draw_state
            .as_mut()
            .expect("draw state")
            .output_merger
            .as_mut()
            .expect("output merger");
        output_merger
            .depth_stencil
            .as_mut()
            .expect("depth stencil")
            .last_depth_stencil_clear = None;
        let blockers = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("ClearDepthStencilView")),
            "{blockers:?}"
        );

        let output_merger = frame
            .active_draw
            .draw_state
            .as_mut()
            .expect("draw state")
            .output_merger
            .as_mut()
            .expect("output merger");
        output_merger.depth_stencil = valid_output_merger_state().depth_stencil;
        output_merger.render_targets[0].last_render_target_clear = None;
        let blockers = map_frame_post_diffuse_state_blockers(&frame.active_draw);
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("ClearRenderTargetView")),
            "{blockers:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_reports_extractor_blockers() {
        let root = temp_test_dir("map_frame_bundle_blockers");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .blockers
                .push("bound constant buffer raw bytes missing".to_string());
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("extractor blockers must fail closed");
        assert!(err
            .to_string()
            .contains("bound constant buffer raw bytes missing"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_constant_buffer_file() {
        let root = temp_test_dir("map_frame_bundle_missing_cb_file");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.constant_buffers[0].file =
                Some("missing_constant_buffer.bin".to_string());
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("missing constant-buffer raw bytes must fail closed");
        assert!(err.to_string().contains("missing_constant_buffer.bin"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_single_draw_bundle() {
        let root = temp_test_dir("map_frame_bundle_single_draw");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.draws.truncate(1);
            frame.active_draw = frame.draws[0].clone();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("single draw must not satisfy production terrain pass evidence");
        assert!(err.to_string().contains("draw record"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_session_manifest() {
        let root = temp_test_dir("map_frame_bundle_missing_session");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        fs::remove_file(root.join(MAP_CAPTURE_SESSION_FILE)).expect("remove session manifest");

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("standalone terrain bundle without session manifest must fail closed");
        assert!(err.to_string().contains(MAP_CAPTURE_SESSION_FILE));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_reports_blocked_session_without_bundle() {
        let root = temp_test_dir("map_frame_blocked_session_without_bundle");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        fs::remove_file(&path).expect("remove map-frame bundle");
        let session_path = root.join(MAP_CAPTURE_SESSION_FILE);
        let mut session: MapCaptureSession =
            serde_json::from_slice(&fs::read(&session_path).expect("read session manifest"))
                .expect("parse session manifest");
        session.moments[0].status = "blocked".to_string();
        session.moments[0].blockers =
            vec!["no active pdxmap pixel shader for map-frame capture".to_string()];
        write_json(&session_path, &session).expect("rewrite blocked session manifest");

        let err = validate_map_frame_runtime_bundle(&root)
            .expect_err("blocked session without map-frame bundle must fail closed");
        let message = err.to_string();
        assert!(message.contains(TERRAIN_CAPTURE_MOMENT));
        assert!(message.contains("no active pdxmap pixel shader"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_wrong_capture_moment() {
        let root = temp_test_dir("map_frame_bundle_wrong_capture_moment");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.capture_moment = "water_pass".to_string();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("non-terrain capture moment must fail closed");
        assert!(err.to_string().contains(TERRAIN_CAPTURE_MOMENT));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn water_map_frame_validation_accepts_explicit_water_bundle() {
        let root = temp_test_dir("water_map_frame_explicit");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |frame| frame);

        let frame = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect("explicit water bundle");
        assert_eq!(frame.capture_moment, WATER_CAPTURE_MOMENT);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_accepts_app_owned_refraction_without_captured_bytes() {
        let root = temp_test_dir("water_refraction_app_owned_without_payload");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            strip_water_refraction_payloads(&mut frame);
            frame
        });
        let terrain_path = root.join(MAP_FRAME_BUNDLE_FILE);
        rewrite_terrain_bundle_with_valid_output_merger_state(&terrain_path);
        let terrain_bundle = NativeD3d11TerrainBundle::load(&terrain_path)
            .expect("valid native D3D11 terrain bundle");
        let water_bundle = NativeD3d11TerrainBundle::load_water(&path)
            .expect("water bundle can rely on app-owned WaterRefraction bytes");
        let camera = terrain_bundle
            .production_camera_from_capture()
            .expect("derive production camera");
        let graph = ProductionMapPassGraph::from_terrain_capture(&terrain_bundle, camera)
            .expect("terrain graph");

        graph
            .with_water_capture(&water_bundle)
            .expect("water refraction comes from app-owned terrain color target");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_app_owned_refraction_without_srv_descriptor() {
        let root = temp_test_dir("water_refraction_missing_srv_desc");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            strip_water_refraction_payloads(&mut frame);
            for draw in &mut frame.draws {
                if let Some(resource) = water_refraction_resource_mut(draw) {
                    resource.view_desc = None;
                }
            }
            if let Some(resource) = water_refraction_resource_mut(&mut frame.active_draw) {
                resource.view_desc = None;
            }
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("WaterRefraction still needs captured SRV descriptor evidence");

        let message = err.to_string();
        assert!(message.contains(WATER_REFRACTION_RESOURCE_SYMBOL));
        assert!(message.contains("SRV descriptor"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_water_refraction_wrap_sampler() {
        let root = temp_test_dir("water_refraction_wrap_sampler");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            mutate_water_sampler(&mut frame, WATER_REFRACTION_RESOURCE_SYMBOL, |sampler| {
                sampler.address_u = D3D11_TEXTURE_ADDRESS_WRAP;
                sampler.address_v = D3D11_TEXTURE_ADDRESS_WRAP;
            });
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("WaterRefraction sampler must follow pdxwater.shader");
        let message = err.to_string();
        assert!(message.contains(WATER_REFRACTION_RESOURCE_SYMBOL));
        assert!(message.contains("linear/clamp"));

        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(&path).expect("read water frame"))
                .expect("parse water frame");
        let blockers = native_d3d11_replay_payload_blockers(&frame, false);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.contains(WATER_REFRACTION_RESOURCE_SYMBOL)
                && blocker.contains("linear/clamp")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_reflection_slot_drift_from_pdxwater_source() {
        let root = temp_test_dir("water_reflection_slot_drift");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            move_water_binding_to_slot(&mut frame, WATER_REFRACTION_RESOURCE_SYMBOL, 6);
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("WaterRefraction reflection slot must match pdxwater.shader Index");
        let message = err.to_string();
        assert!(message.contains(WATER_REFRACTION_RESOURCE_SYMBOL));
        assert!(message.contains("pdxwater.shader declares Index 5"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_reflection_cube_map_2d_view() {
        let root = temp_test_dir("water_reflection_cube_2d_view");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_reflected_water_resource(
                &mut frame,
                "ReflectionCubeMap",
                D3D11_SRV_DIMENSION_TEXTURE2D,
                0,
            );
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("ReflectionCubeMap must be a cube view per pdxwater.shader");
        let message = err.to_string();
        assert!(message.contains("ReflectionCubeMap"));
        assert!(message.contains("Type Cube"));
        assert!(message.contains("view_dimension"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_reflection_cube_map_missing_misc_flag() {
        let root = temp_test_dir("water_reflection_cube_missing_misc");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            add_reflected_water_resource(
                &mut frame,
                "ReflectionCubeMap",
                D3D11_SRV_DIMENSION_TEXTURECUBE,
                0,
            );
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("ReflectionCubeMap cube view must carry D3D11 cube misc flag");
        let message = err.to_string();
        assert!(message.contains("ReflectionCubeMap"));
        assert!(message.contains("D3D11_RESOURCE_MISC_TEXTURECUBE"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_height_texture_point_sampler() {
        let root = temp_test_dir("water_height_texture_point_sampler");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            mutate_water_sampler(&mut frame, "HeightTexture", |sampler| {
                sampler.filter = D3D11_FILTER_MIN_MAG_MIP_POINT;
                sampler.address_u = D3D11_TEXTURE_ADDRESS_CLAMP;
                sampler.address_v = D3D11_TEXTURE_ADDRESS_CLAMP;
            });
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("HeightTexture sampler must follow pdxwater.shader");
        let message = err.to_string();
        assert!(message.contains("HeightTexture"));
        assert!(message.contains("linear/wrap"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_ignores_dependent_pre_draw_payload_blockers() {
        let root = temp_test_dir("water_map_frame_pre_draw_payload_blockers");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.blockers = vec![
                "OM render target slot 0: pre-draw OM render target slot 0 is 4x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback".to_string(),
                "OM depth-stencil view: pre-draw OM depth-stencil target is 4x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback".to_string(),
                "draw 2 pre-draw output-merger render target slot Some(0) has no payload file".to_string(),
                "draw 2 pre-draw output-merger depth-stencil has no payload file".to_string(),
            ];
            frame
        });

        validate_water_map_frame_bundle(&path)
            .expect("production water consumes the app-owned terrain output/depth");
        NativeD3d11TerrainBundle::load_water(&path)
            .expect("production water bundle can load without isolated pre-draw payloads");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_unrelated_extraction_blockers() {
        let root = temp_test_dir("water_map_frame_unrelated_blocker");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame
                .blockers
                .push("PS shader resource slot 7 could not be captured".to_string());
            frame
        });

        let err = validate_water_map_frame_bundle(&path)
            .expect_err("unrelated water extraction blockers must remain fatal");
        assert!(err.to_string().contains("shader resource slot 7"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_layer_frame_validation_accepts_grounded_river_border_and_tree_bundles() {
        let river_root = temp_test_dir("rivers_map_frame_valid");
        fs::create_dir_all(&river_root).expect("create river dir");
        let river_path = write_valid_map_layer_frame_bundle(
            &river_root,
            RIVERS_MAP_FRAME_SPEC,
            &RIVER_FRAME_SHADER_SOURCES,
            &RIVER_FRAME_REQUIRED_SHADER_RESOURCES,
            "river_pixel_shader_1111111111111111.bin",
            "map_vertex_shader_1111111111111111.bin",
            |frame| frame,
        );
        let river_frame = validate_map_frame_runtime_bundle_for(&river_path, RIVERS_MAP_FRAME_SPEC)
            .expect("valid rivers bundle");
        assert_eq!(river_frame.capture_moment, RIVERS_CAPTURE_MOMENT);

        let border_root = temp_test_dir("borders_map_frame_valid");
        fs::create_dir_all(&border_root).expect("create border dir");
        let border_path = write_valid_map_layer_frame_bundle(
            &border_root,
            BORDERS_MAP_FRAME_SPEC,
            &BORDER_FRAME_SHADER_SOURCES,
            &BORDER_FRAME_REQUIRED_SHADER_RESOURCES,
            "border_pixel_shader_2222222222222222.bin",
            "map_vertex_shader_2222222222222222.bin",
            |frame| frame,
        );
        let border_frame =
            validate_map_frame_runtime_bundle_for(&border_path, BORDERS_MAP_FRAME_SPEC)
                .expect("valid borders bundle");
        assert_eq!(border_frame.capture_moment, BORDERS_CAPTURE_MOMENT);

        let tree_root = temp_test_dir("trees_map_frame_valid");
        fs::create_dir_all(&tree_root).expect("create tree dir");
        let tree_path = write_valid_map_layer_frame_bundle(
            &tree_root,
            TREES_MAP_FRAME_SPEC,
            &TREE_FRAME_SHADER_SOURCES,
            &TREE_FRAME_REQUIRED_SHADER_RESOURCES,
            "tree_pixel_shader_3333333333333333.bin",
            "map_vertex_shader_3333333333333333.bin",
            |frame| frame,
        );
        let tree_frame = validate_map_frame_runtime_bundle_for(&tree_path, TREES_MAP_FRAME_SPEC)
            .expect("valid trees bundle");
        assert_eq!(tree_frame.capture_moment, TREES_CAPTURE_MOMENT);

        let _ = fs::remove_dir_all(river_root);
        let _ = fs::remove_dir_all(border_root);
        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn map_layer_frame_validation_rejects_missing_required_shader_resource() {
        let root = temp_test_dir("rivers_map_frame_missing_river_data");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_layer_frame_bundle(
            &root,
            RIVERS_MAP_FRAME_SPEC,
            &RIVER_FRAME_SHADER_SOURCES,
            &RIVER_FRAME_REQUIRED_SHADER_RESOURCES,
            "river_pixel_shader_4444444444444444.bin",
            "map_vertex_shader_4444444444444444.bin",
            |mut frame| {
                for draw in &mut frame.draws {
                    draw.shader.reflection_resources.remove("RiverData");
                }
                frame
                    .active_draw
                    .shader
                    .reflection_resources
                    .remove("RiverData");
                frame
            },
        );

        let err = validate_map_frame_runtime_bundle_for(&path, RIVERS_MAP_FRAME_SPEC)
            .expect_err("missing RiverData must fail closed");

        assert!(err.to_string().contains("RiverData"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_map_frame_validation_rejects_pre_draw_payload_extraction_blockers() {
        let root = temp_test_dir("terrain_map_frame_pre_draw_blocker");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.blockers.push(
                "OM render target slot 0: pre-draw OM render target slot 0 is 4x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback".to_string(),
            );
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("terrain extraction blockers must remain fatal");
        assert!(err.to_string().contains("pre-draw OM render target"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_uses_water_bundle_for_directory_input() {
        let root = temp_test_dir("water_map_frame_directory");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        write_valid_water_map_frame_bundle(&root, |frame| frame);

        let frame = validate_map_frame_runtime_bundle_for(&root, WATER_MAP_FRAME_SPEC)
            .expect("directory resolves water bundle");
        assert_eq!(frame.capture_moment, WATER_CAPTURE_MOMENT);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_map_frame_validation_keeps_terrain_default_when_water_bundle_exists() {
        let root = temp_test_dir("terrain_default_with_water_bundle");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        write_valid_water_map_frame_bundle(&root, |frame| frame);

        let frame =
            validate_map_frame_runtime_bundle(&root).expect("directory resolves terrain bundle");
        assert_eq!(frame.capture_moment, TERRAIN_CAPTURE_MOMENT);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_map_frame_validation_rejects_explicit_water_bundle() {
        let root = temp_test_dir("terrain_rejects_water_bundle");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |frame| frame);

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("terrain validator must reject explicit water bundle");
        assert!(err.to_string().contains(TERRAIN_CAPTURE_MOMENT));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_pdxwater_source() {
        let root = temp_test_dir("water_missing_pdxwater_source");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.vanilla_sources.remove("gfx/FX/pdxwater.shader");
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require pdxwater source evidence");
        assert!(err.to_string().contains("gfx/FX/pdxwater.shader"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_uses_water_label_for_missing_shader_bytecode() {
        let root = temp_test_dir("water_missing_shader_bytecode_label");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            let bytecode = frame.active_draw.shader.bytecode_file.clone();
            frame.runtime_files.remove(&bytecode);
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must label water bytecode blockers");
        let message = err.to_string();
        assert!(message.contains("production water renderer blocked"));
        assert!(message.contains(WATER_CAPTURE_MOMENT));
        assert!(message.contains("pixel shader bytecode"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_reflected_srv_binding() {
        let root = temp_test_dir("water_missing_reflected_srv_binding");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.shader_resources.retain(|resource| {
                resource.declared_name.as_deref() != Some(WATER_REFRACTION_RESOURCE_SYMBOL)
            });
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require every reflected PS SRV binding");
        let message = err.to_string();
        assert!(message.contains("WaterRefraction"));
        assert!(message.contains("no captured SRV binding"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_reflected_srv_name_mismatch() {
        let root = temp_test_dir("water_reflected_srv_name_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .shader
                .reflection_resources
                .insert(WATER_REFRACTION_RESOURCE_SYMBOL.to_string(), 0);
            frame.active_draw.shader_resources.retain(|resource| {
                resource.declared_name.as_deref() != Some(WATER_REFRACTION_RESOURCE_SYMBOL)
            });
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must match reflected PS SRV names");
        let message = err.to_string();
        assert!(message.contains("WaterRefraction"));
        assert!(message.contains("HeightTexture"));
        assert!(message.contains("expected reflected resource"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_reflected_constant_buffer() {
        let root = temp_test_dir("water_missing_reflected_constant_buffer");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .shader
                .constant_buffers
                .insert("WaterSceneConstants".to_string(), 9);
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require every reflected constant buffer");
        let message = err.to_string();
        assert!(message.contains("WaterSceneConstants"));
        assert!(message.contains("no captured PS constant-buffer binding"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_reflected_constant_buffer_name_mismatch() {
        let root = temp_test_dir("water_reflected_constant_buffer_name_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .constant_buffers
                .iter_mut()
                .find(|buffer| buffer.stage == "PS" && buffer.slot == 0)
                .expect("PS constant buffer slot 0")
                .declared_name = Some("WrongWaterConstants".to_string());
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must match reflected constant-buffer names");
        let message = err.to_string();
        assert!(message.contains(TERRAIN_SCENE_CB0_SYMBOL));
        assert!(message.contains("WrongWaterConstants"));
        assert!(message.contains("expected reflected constant buffer"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_non_updateable_camera_constant_buffer() {
        let root = temp_test_dir("water_non_updateable_camera_constant_buffer");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            for buffer in frame
                .active_draw
                .constant_buffers
                .iter_mut()
                .filter(|buffer| is_terrain_scene_cb0_camera_binding(buffer))
            {
                buffer.usage = 1;
            }
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require updateable camera constant buffers");
        let message = err.to_string();
        assert!(message.contains(TERRAIN_SCENE_CB0_SYMBOL));
        assert!(message.contains("D3D11_USAGE"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_accepts_dynamic_dx11_cb0_camera_constant_buffer() {
        fn use_dynamic_dx11_cb0_camera_alias(draw: &mut MapFrameDraw) {
            draw.shader.constant_buffers = BTreeMap::from([("dx11_cb0".to_string(), 0)]);
            if let Some(vertex_shader) = draw.vertex_shader.as_mut() {
                vertex_shader.constant_buffers = BTreeMap::from([("dx11_cb0".to_string(), 0)]);
            }
            for buffer in draw.constant_buffers.iter_mut().filter(|buffer| {
                buffer.slot == TERRAIN_SCENE_CB0_BIND_POINT
                    && matches!(buffer.stage.as_str(), "VS" | "PS")
            }) {
                buffer.declared_name = Some("dx11_cb0".to_string());
                buffer.usage = D3D11_USAGE_DYNAMIC_CAPTURED;
                buffer.cpu_access_flags = D3D11_CPU_ACCESS_WRITE_CAPTURED;
            }
        }

        let root = temp_test_dir("water_dynamic_dx11_cb0_camera_constant_buffer");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                use_dynamic_dx11_cb0_camera_alias(draw);
            }
            use_dynamic_dx11_cb0_camera_alias(&mut frame.active_draw);
            frame
        });

        validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect("water validator must accept dynamic dx11_cb0 camera constant buffers");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_reflected_sampler_state() {
        let root = temp_test_dir("water_missing_reflected_sampler_state");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.samplers.clear();
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require sampler state for reflected SRVs");
        let message = err.to_string();
        assert!(message.contains("sampler-state"));
        assert!(message.contains("HeightTexture"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_draw_state() {
        let root = temp_test_dir("water_missing_draw_state");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.draw_state = None;
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require captured D3D11 draw state");
        let message = err.to_string();
        assert!(message.contains("draw state"));
        assert!(message.contains("viewport"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn water_map_frame_validation_rejects_missing_topology() {
        let root = temp_test_dir("water_missing_topology");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.topology = None;
            frame
        });

        let err = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect_err("water validator must require captured IA topology");
        let message = err.to_string();
        assert!(message.contains("primitive topology"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_bundle_validation_accepts_exact_instanced_draw_args() {
        let root = temp_test_dir("map_frame_bundle_instanced_args");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                draw.draw_call = "DrawIndexedInstanced".to_string();
                draw.draw_count = 6;
                draw.per_instance_count = Some(3);
                draw.instance_count = Some(2);
                draw.start_instance_location = Some(4);
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });

        validate_map_frame_runtime_bundle(&path).expect("exact instanced args should validate");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_instanced_draw_without_exact_args() {
        let root = temp_test_dir("map_frame_bundle_missing_instanced_args");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                draw.draw_call = "DrawInstanced".to_string();
                draw.draw_count = 6;
                draw.per_instance_count = Some(3);
                draw.instance_count = Some(2);
                draw.start_instance_location = None;
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("instanced draw without start instance location must fail closed");
        assert!(err.to_string().contains("start instance location"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_instanced_draw_count_mismatch() {
        let root = temp_test_dir("map_frame_bundle_instanced_count_mismatch");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                draw.draw_call = "DrawIndexedInstanced".to_string();
                draw.draw_count = 7;
                draw.per_instance_count = Some(3);
                draw.instance_count = Some(2);
                draw.start_instance_location = Some(0);
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("instanced draw count mismatch must fail closed");
        assert!(err
            .to_string()
            .contains("does not match per-instance count"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_non_instanced_draw_with_instanced_args() {
        let root = temp_test_dir("map_frame_bundle_non_instanced_extra_args");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.draws[0].instance_count = Some(1);
            frame.active_draw = frame.draws[0].clone();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("non-instanced draw with instanced args must fail closed");
        assert!(err
            .to_string()
            .contains("must not carry instanced draw args"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_session_missing_required_payload() {
        let root = temp_test_dir("map_frame_bundle_session_missing_payload");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |frame| frame);
        let session_path = root.join(MAP_CAPTURE_SESSION_FILE);
        let mut session: MapCaptureSession =
            serde_json::from_slice(&fs::read(&session_path).expect("read session manifest"))
                .expect("parse session manifest");
        session.moments[0]
            .required_payload
            .retain(|payload| payload != "sampler_states");
        write_json(&session_path, &session).expect("rewrite session manifest");

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("session without required sampler payload must fail closed");
        assert!(err.to_string().contains("sampler_states"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_shader_bytecode_evidence() {
        let root = temp_test_dir("map_frame_bundle_missing_shader_bytecode");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .runtime_files
                .remove("pdxmap_pixel_shader_0123456789abcdef.bin");
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("shader bytecode evidence must fail closed");
        assert!(err.to_string().contains("shader bytecode"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_vertex_shader_identity() {
        let root = temp_test_dir("map_frame_bundle_missing_vertex_shader");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.vertex_shader = None;
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("terrain draw without VS bytecode/reflection must fail closed");
        assert!(err.to_string().contains("active vertex shader"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_vertex_shader_bytecode_evidence() {
        let root = temp_test_dir("map_frame_bundle_missing_vertex_shader_bytecode");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .runtime_files
                .remove("pdxmap_vertex_shader_fedcba9876543210.bin");
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader bytecode evidence must fail closed");
        assert!(err.to_string().contains("vertex shader bytecode"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_vertex_shader_without_position_height_inputs() {
        let root = temp_test_dir("map_frame_bundle_bad_vertex_shader_inputs");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .vertex_shader
                .as_mut()
                .expect("vertex shader")
                .input_parameters
                .retain(|parameter| !parameter.semantic_name.eq_ignore_ascii_case("TEXCOORD"));
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader reflection without height input must fail closed");
        assert!(err
            .to_string()
            .contains("POSITION0 int4 and TEXCOORD0 height inputs"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_vertex_shader_with_wrong_component_type() {
        let root = temp_test_dir("map_frame_bundle_bad_vertex_shader_component");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .vertex_shader
                .as_mut()
                .expect("vertex shader")
                .input_parameters[0]
                .component_type = 3;
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader reflection with non-integer input must fail closed");
        assert!(err
            .to_string()
            .contains("POSITION0 int4 and TEXCOORD0 height inputs"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_vertex_shader_without_cb1_binding() {
        let root = temp_test_dir("map_frame_bundle_bad_vertex_shader_cb");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .vertex_shader
                .as_mut()
                .expect("vertex shader")
                .constant_buffers
                .clear();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader reflection without CB1 must fail closed");
        assert!(err.to_string().contains("VS constant-buffer slot 1"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_vertex_shader_with_wrong_cb1_source() {
        let root = temp_test_dir("map_frame_bundle_wrong_vertex_shader_cb_source");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            let vertex_shader = frame
                .active_draw
                .vertex_shader
                .as_mut()
                .expect("vertex shader");
            vertex_shader.constant_buffers.clear();
            vertex_shader
                .constant_buffers
                .insert("SomeOtherConstantBuffer".to_string(), 1);
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader reflection with wrong CB1 source must fail closed");
        assert!(err
            .to_string()
            .contains("gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_vertex_shader_missing_source_citation() {
        let root = temp_test_dir("map_frame_bundle_missing_vertex_shader_source");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .vertex_shader
                .as_mut()
                .expect("vertex shader")
                .source_evidence
                .retain(|source| source != "gfx/FX/pdxmap.fxh");
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vertex shader without pdxmap.fxh source citation must fail closed");
        assert!(err
            .to_string()
            .contains("pdxmap.shader and gfx/FX/pdxmap.fxh"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_missing_vanilla_shader_source() {
        let root = temp_test_dir("map_frame_bundle_missing_vanilla_source");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.vanilla_sources.remove("gfx/FX/constants.fxh");
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("vanilla shader source evidence must fail closed");
        assert!(err.to_string().contains("constants.fxh"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_empty_constant_buffers() {
        let root = temp_test_dir("map_frame_bundle_empty_constant_buffers");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.constant_buffers.clear();
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("empty constant-buffer set must fail closed");
        assert!(err.to_string().contains("constant-buffer captures"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_frame_bundle_validation_rejects_wrong_terrain_sampler_state() {
        let root = temp_test_dir("map_frame_bundle_wrong_terrain_sampler");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.samplers[0].filter = 21;
            frame.active_draw.samplers[0].address_u = 1;
            frame
        });

        let err = validate_map_frame_runtime_bundle(&path)
            .expect_err("TerrainIDMap sampler assumptions must fail closed");
        assert!(err.to_string().contains("point/clamp"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_terrain_renderer_refuses_missing_terrain_id_map() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_renderer_gate_{}_{}",
            std::process::id(),
            "missing_terrain_id"
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp bundle dir");

        let manifest = Manifest {
            schema: "battle-plan-rust-map-bundle-v1".to_string(),
            dimensions: Dimensions {
                width: 4,
                height: 4,
            },
            province_id_encoding: "little-endian u32 per pixel".to_string(),
            files: BundleFiles {
                province_ids: "province_ids.r32".to_string(),
                terrain_id_map: "terrain_id_map.rgba".to_string(),
                terrain_runtime: "terrain_runtime.json".to_string(),
                snow_mud_data: "snow_mud_data.rgba".to_string(),
                terrain_scene_cb0: "terrain_scene_cb0.bin".to_string(),
                province_secondary_color_map: "province_secondary_color_map.rgba".to_string(),
                gradient_border_channel1: "gradient_border_channel1.rgba".to_string(),
                gradient_border_channel2: "gradient_border_channel2.rgba".to_string(),
                gradient_border_channel3: "gradient_border_channel3.rgba".to_string(),
                light_index_map: "light_index_map.rgba".to_string(),
                light_data_map: "light_data_map.rgba".to_string(),
                shadow_map: "shadow_map.rgba".to_string(),
                terrain_indices: "terrain_indices.r8".to_string(),
                terrain_texture_ids: "terrain_texture_ids.r8".to_string(),
                heightmap: "heightmap.r8".to_string(),
                rivers: "rivers.r8".to_string(),
                height_normal: "height_normal.rgbx".to_string(),
                cities: "cities.r8".to_string(),
                trees: "trees.r8".to_string(),
                tree_tint: "tree_tint.rgbx".to_string(),
                tree_season: "tree_season.rgbx".to_string(),
                terrain_diffuse_atlas: "terrain_diffuse_atlas0.rgbx".to_string(),
                terrain_normal_atlas: "terrain_normal_atlas0.rgbx".to_string(),
                terrain_diffuse_atlas_mips: "terrain_diffuse_atlas0_mips.rgba".to_string(),
                terrain_normal_atlas_mips: "terrain_normal_atlas0_mips.rgba".to_string(),
                shader_sources: BTreeMap::new(),
                map_sources: BTreeMap::new(),
                map_object_meta: "map_object_meta.json".to_string(),
                map_material_contract: "map_material_contract.json".to_string(),
                province_to_state: "province_to_state.json".to_string(),
                state_meta: "state_meta.json".to_string(),
                state_owner_1936: "state_owner_1936.json".to_string(),
                country_colors: "country_colors.json".to_string(),
                province_definitions: "province_definitions.json".to_string(),
                province_to_strategic_region: "province_to_strategic_region.json".to_string(),
                strategic_region_to_provinces: "strategic_region_to_provinces.json".to_string(),
                strategic_region_meta: "strategic_region_meta.json".to_string(),
            },
            counts: BundleCounts {
                definition_rows: 0,
                distinct_ids_in_provinces_bmp: 0,
                states: 0,
                state_owners_1936: 0,
                province_to_state: 0,
                country_colors: 0,
                strategic_regions: 0,
                province_to_strategic_region: 0,
                unknown_pixels: 0,
            },
            source: BundleSource {
                game_dir_name: "the game".to_string(),
                default_map: "map/default.map".to_string(),
                definition_csv_fnv1a64: String::new(),
                provinces_bmp_fnv1a64: String::new(),
                visual_input_fnv1a64: BTreeMap::new(),
                visual_input_dimensions: BTreeMap::new(),
            },
            modset: None,
            viewer_note: String::new(),
        };
        write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
        let contract = MapMaterialContract {
            implemented_visible_level: "production_terrain_renderer_gate".to_string(),
            shader_constants: BTreeMap::new(),
            shader_vector3_constants: BTreeMap::new(),
            shipped_sources: BTreeMap::new(),
            generated_runtime_inputs: generated_runtime_material_inputs(),
        };
        write_json(&root.join("map_material_contract.json"), &contract).expect("write contract");

        let err = validate_production_terrain_renderer(&root)
            .expect_err("missing TerrainIDMap must block renderer");
        assert!(err.to_string().contains("TerrainIDMap"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_terrain_renderer_refuses_orphan_terrain_id_map() {
        let root = temp_test_dir("orphan_terrain_id_map");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = minimal_test_manifest();
        write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
        write_json(
            &root.join("map_material_contract.json"),
            &minimal_test_contract(),
        )
        .expect("write contract");
        fs::write(root.join(&manifest.files.terrain_id_map), [0u8; 16])
            .expect("write orphan terrain id map");

        let err = validate_production_terrain_renderer(&root)
            .expect_err("orphan TerrainIDMap without runtime metadata must block renderer");
        assert!(err.to_string().contains("terrain runtime input contract"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_terrain_renderer_blocks_missing_multislot_vertex_buffers() {
        let root = temp_test_dir("blocked_multislot_terrain_runtime_contract");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let mut manifest = minimal_test_manifest();
        manifest.files.shader_sources = BTreeMap::from([
            (
                "map/terrain/atlas0.dds".to_string(),
                "shader_map_terrain_atlas0.rgba".to_string(),
            ),
            (
                "map/terrain/atlas_normal0.dds".to_string(),
                "shader_map_terrain_atlas_normal0.rgba".to_string(),
            ),
            (
                "map/terrain/colormap_rgb_cityemissivemask_a.dds".to_string(),
                "shader_map_terrain_colormap_rgb_cityemissivemask_a.rgba".to_string(),
            ),
        ]);
        manifest.files.map_sources = BTreeMap::from([
            (
                "gfx/FX/constants.fxh".to_string(),
                "source_gfx_fx_constants_fxh".to_string(),
            ),
            (
                "gfx/FX/pdxmap.shader".to_string(),
                "source_gfx_fx_pdxmap_shader".to_string(),
            ),
            (
                "gfx/FX/pdxmap.fxh".to_string(),
                "source_gfx_fx_pdxmap_fxh".to_string(),
            ),
            (
                "gfx/FX/standardfuncsgfx.fxh".to_string(),
                "source_gfx_fx_standardfuncsgfx_fxh".to_string(),
            ),
            (
                "gfx/FX/shadow.fxh".to_string(),
                "source_gfx_fx_shadow_fxh".to_string(),
            ),
            (
                "gfx/FX/fow.fxh".to_string(),
                "source_gfx_fx_fow_fxh".to_string(),
            ),
        ]);
        write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
        write_json(
            &root.join("map_material_contract.json"),
            &minimal_test_contract(),
        )
        .expect("write contract");

        for output in manifest.files.shader_sources.values() {
            fs::write(root.join(output), [7u8]).expect("write shader source file");
        }
        for (source, output) in &manifest.files.map_sources {
            let bytes = minimal_map_source_bytes(source);
            fs::write(root.join(output), bytes).expect("write map source file");
        }
        fs::write(root.join(&manifest.files.height_normal), [3u8]).expect("write height normal");
        fs::write(root.join(&manifest.files.terrain_diffuse_atlas_mips), [4u8])
            .expect("write terrain diffuse atlas mip chain");
        fs::write(root.join(&manifest.files.terrain_normal_atlas_mips), [5u8])
            .expect("write terrain normal atlas mip chain");
        fs::write(root.join(&manifest.files.terrain_id_map), [1u8; 16])
            .expect("write terrain id map");
        let mut runtime =
            minimal_test_runtime(&manifest, &root.join(&manifest.files.terrain_id_map));
        add_valid_vertex_and_input_layout_runtime(&root, &mut runtime);
        write_json(&root.join(&manifest.files.terrain_runtime), &runtime)
            .expect("write runtime contract");

        let err = validate_production_terrain_renderer(&root)
            .expect_err("slot 1 terrain input without slot 1 vertex buffer capture must block");
        assert!(err.to_string().contains("input slot(s)"));
        assert!(err.to_string().contains("slot 0"));

        let map_frame_inputs = validate_production_terrain_renderer_for_map_frame(&root).expect(
            "map-frame renderer validates captured draw bundles instead of legacy slot 0 metadata",
        );
        assert!(map_frame_inputs.runtime.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn material_mip_output_validation_requires_exact_chain() {
        let root = temp_test_dir("material_mip_validation");
        fs::create_dir_all(&root).expect("create temp material mip dir");
        let source = "map/terrain/snow_normal_rgb_diffuse_a.dds";
        let mip_output = shader_rgba_mip_output_name(source);
        let chain = vec![
            RgbaImage {
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            },
            RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![17, 18, 19, 20],
            },
        ];
        write_rgba_mip_chain_raw(&root.join(&mip_output), &chain).expect("write material mips");
        let mut contract = minimal_test_contract();
        contract.shipped_sources.insert(
            source.to_string(),
            test_material_source_meta(
                2,
                "decoded_rgba_exact_2d_with_mip_chain",
                Some(mip_output.clone()),
            ),
        );

        validate_material_mip_outputs(&contract, &root).expect("exact material mip chain");

        fs::write(root.join(&mip_output), [1u8, 2, 3, 4]).expect("truncate mip chain");
        let err = validate_material_mip_outputs(&contract, &root)
            .expect_err("truncated material mip chain must fail");
        assert!(err.to_string().contains("expected 20"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn material_mip_output_validation_rejects_flattened_mipped_material() {
        let mut contract = minimal_test_contract();
        contract.shipped_sources.insert(
            "map/terrain/mud_diffuse_rgb_gloss_a_0.dds".to_string(),
            test_material_source_meta(2, "decoded_rgba_exact_2d", None),
        );
        let err = validate_material_mip_outputs(&contract, Path::new("."))
            .expect_err("mipped material without chain must fail");
        assert!(err.to_string().contains("preserved only mip 0"));
    }
}
