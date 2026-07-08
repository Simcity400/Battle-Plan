use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const BUILDING_MESH_VERTEX_STRIDE_BYTES: u32 = 48;
pub const BUILDINGS_ENTITY_ASSET_SOURCE: &str = "gfx/entities/buildings.asset";
pub const BUILDINGS_GFX_SOURCE: &str = "gfx/entities/buildings.gfx";
const BUILDINGS_EXTRA_GFX_SOURCES: [&str; 1] = ["gfx/models/buildings/supply_station.gfx"];
pub const BUILDING_SHADER_SOURCE: &str = "gfx/FX/pdxmesh.shader";
pub const BUILDING_SHADER_EFFECT: &str = "PdxMeshStandard";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingEntityBinding {
    pub name: String,
    pub pdxmesh: String,
    pub scale: f32,
    pub source: String,
    pub block_line: usize,
    pub name_line: usize,
    pub pdxmesh_line: usize,
    pub scale_line: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingPdxmeshBinding {
    pub name: String,
    pub mesh: String,
    pub scale: f32,
    pub source: String,
    pub block_line: usize,
    pub name_line: usize,
    pub file_line: usize,
    pub scale_line: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingBindingCoverageRow {
    pub building: String,
    pub show_on_map: u32,
    pub show_on_map_meshes: Option<u32>,
    pub candidate_entities: Vec<String>,
    pub resolved_entities: Vec<String>,
    pub missing_entities: Vec<String>,
    pub missing_meshes: Vec<String>,
}

impl BuildingBindingCoverageRow {
    pub fn fully_resolved(&self) -> bool {
        !self.resolved_entities.is_empty()
            && self.missing_entities.is_empty()
            && self.missing_meshes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildingShaderCBufferMapping {
    pub slot: u32,
    pub dx9_base: u32,
    pub declaration: String,
    pub filled_bytes: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildingShaderPreparation {
    pub shader_source: String,
    pub effect_name: String,
    pub vertex_hlsl_bytes: usize,
    pub pixel_hlsl_bytes: usize,
    pub vertex_bytecode_bytes: Option<usize>,
    pub pixel_bytecode_bytes: Option<usize>,
    pub status: String,
    pub cbuffer_mapping: Vec<BuildingShaderCBufferMapping>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingMeshVertexBuffer {
    pub source_mesh: String,
    pub entity_name: String,
    pub pdxmesh_name: String,
    pub total_scale: f32,
    pub material_shader: String,
    pub material_diffuse: Option<String>,
    pub material_normal_map: Option<String>,
    pub material_specular: Option<String>,
    pub vertex_stride_bytes: u32,
    pub vertex_count: usize,
    pub index_count: usize,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingPreviewInstance {
    pub state_id: u32,
    pub building: String,
    pub slot_index: u32,
    pub entity_name: String,
    pub model_index: usize,
    pub pos: [f32; 3],
    pub rotation: f32,
    pub scale: f32,
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingFileDerivedLayerStats {
    pub schema: String,
    pub preview_all: bool,
    pub show_on_map_type_count: usize,
    pub resolved_type_count: usize,
    pub missing_type_count: usize,
    pub model_count: usize,
    pub instance_count: usize,
    pub coverage: Vec<BuildingBindingCoverageRow>,
    pub shader: BuildingShaderPreparation,
    pub level_policy: String,
    pub gaps: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BuildingFileDerivedLayer {
    pub models: Vec<BuildingMeshVertexBuffer>,
    pub instances: Vec<BuildingPreviewInstance>,
    pub stats: BuildingFileDerivedLayerStats,
}

#[derive(Clone, Debug)]
pub struct ResolvedBuildingEntity {
    pub building: ShowOnMapBuilding,
    pub entity: BuildingEntityBinding,
    pub pdxmesh: BuildingPdxmeshBinding,
}

pub fn building_file_derived_layer_from_resolver(
    resolver: &GameFileResolver,
    preview_all: bool,
) -> Result<BuildingFileDerivedLayer> {
    let placements = map_building_placements_from_resolver(resolver)?;
    let show_on_map = show_on_map_buildings_from_resolver(resolver)?;
    let entities = building_entity_bindings_from_resolver(resolver)?;
    let meshes = building_pdxmesh_bindings_from_resolver(resolver)?;
    let (resolved, coverage) =
        resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes);
    let (models, model_indices, coverage, model_gaps) =
        load_building_models(resolver, &resolved, coverage)?;
    let instances = building_preview_instances(
        &placements,
        &show_on_map,
        &resolved,
        &model_indices,
        preview_all,
    );
    let shader = prepare_building_shader_from_resolver(resolver);
    let resolved_type_count = coverage.iter().filter(|row| row.fully_resolved()).count();
    let missing_type_count = coverage.len().saturating_sub(resolved_type_count);
    let mut gaps = model_gaps;
    if !preview_all {
        gaps.push(
            "--buildings-from-files prepared static placements and meshes but emitted zero instances; REPLAY_DATA_REQUIREMENTS item 10 must supply recorded state building levels before slot visibility is real".to_string(),
        );
    }
    for row in coverage.iter().filter(|row| !row.fully_resolved()) {
        gaps.push(format!(
            "building {} fail-closed: missing entities {:?}, missing meshes {:?}",
            row.building, row.missing_entities, row.missing_meshes
        ));
    }
    let stats = BuildingFileDerivedLayerStats {
        schema: "battle-plan-building-file-derived-layer-v1".to_string(),
        preview_all,
        show_on_map_type_count: show_on_map.len(),
        resolved_type_count,
        missing_type_count,
        model_count: models.len(),
        instance_count: instances.len(),
        coverage,
        shader,
        level_policy: if preview_all {
            "visual gate override: render slot 0..show_on_map-1 per state at level>=1 assumption; real playback must gate slots on recorded building levels per REPLAY_DATA_REQUIREMENTS item 10".to_string()
        } else {
            "default preview policy: rows in map/buildings.txt are slots; without recorded building levels no slots are rendered, even under --buildings-from-files".to_string()
        },
        gaps,
    };
    Ok(BuildingFileDerivedLayer {
        models,
        instances,
        stats,
    })
}

pub fn building_entity_bindings_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BuildingEntityBinding>> {
    let path = resolver.resolve_required(BUILDINGS_ENTITY_ASSET_SOURCE, "building entity asset")?;
    let text = read_latin1(&path)?;
    parse_building_entity_bindings(&text, BUILDINGS_ENTITY_ASSET_SOURCE)
}

pub fn building_pdxmesh_bindings_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BuildingPdxmeshBinding>> {
    let path = resolver.resolve_required(BUILDINGS_GFX_SOURCE, "building pdxmesh binding")?;
    let text = read_latin1(&path)?;
    let mut out = parse_building_pdxmesh_bindings(&text, BUILDINGS_GFX_SOURCE)?;
    for source in BUILDINGS_EXTRA_GFX_SOURCES {
        let Some(path) = resolver.resolve(source)? else {
            continue;
        };
        let text = read_latin1(&path)?;
        out.extend(parse_building_pdxmesh_bindings(&text, source)?);
    }
    Ok(out)
}

pub fn parse_building_entity_bindings(
    text: &str,
    source: &str,
) -> Result<Vec<BuildingEntityBinding>> {
    let mut explicit = BTreeMap::new();
    let mut clones = Vec::new();
    for block in parse_assignment_blocks(text, source, "entity")? {
        match parse_building_entity_binding_block(&block, source)? {
            ParsedBuildingEntityBlock::Binding(binding) => {
                explicit.insert(binding.name.clone(), binding);
            }
            ParsedBuildingEntityBlock::Clone(clone) => clones.push(clone),
            ParsedBuildingEntityBlock::Ignored => {}
        }
    }
    let mut unresolved = clones;
    loop {
        let mut changed = false;
        let mut still_unresolved = Vec::new();
        for clone in unresolved {
            if let Some(base) = explicit.get(&clone.clone).cloned() {
                explicit.insert(clone.name.clone(), clone.to_binding(&base));
                changed = true;
            } else {
                still_unresolved.push(clone);
            }
        }
        if still_unresolved.is_empty() {
            break;
        }
        if !changed {
            break;
        }
        unresolved = still_unresolved;
    }
    Ok(explicit.into_values().collect())
}

pub fn parse_building_pdxmesh_bindings(
    text: &str,
    source: &str,
) -> Result<Vec<BuildingPdxmeshBinding>> {
    parse_assignment_blocks(text, source, "pdxmesh")?
        .iter()
        .map(|block| parse_building_pdxmesh_binding_block(block, source))
        .collect()
}

pub fn resolve_show_on_map_building_bindings(
    show_on_map: &[ShowOnMapBuilding],
    entity_bindings: &[BuildingEntityBinding],
    pdxmesh_bindings: &[BuildingPdxmeshBinding],
) -> (
    BTreeMap<String, Vec<ResolvedBuildingEntity>>,
    Vec<BuildingBindingCoverageRow>,
) {
    let entities_by_name = entity_bindings
        .iter()
        .map(|binding| (binding.name.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let meshes_by_name = pdxmesh_bindings
        .iter()
        .map(|binding| (binding.name.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut resolved = BTreeMap::new();
    let mut coverage = Vec::new();
    for building in show_on_map {
        let candidates = candidate_entity_names(building);
        let mut resolved_entities = Vec::new();
        let mut missing_entities = Vec::new();
        let mut missing_meshes = Vec::new();
        let mut building_resolved = Vec::new();
        for candidate in &candidates {
            let Some(entity) = entities_by_name.get(candidate) else {
                missing_entities.push(candidate.clone());
                continue;
            };
            let Some(pdxmesh) = meshes_by_name.get(&entity.pdxmesh) else {
                missing_meshes.push(format!("{} -> {}", entity.name, entity.pdxmesh));
                continue;
            };
            resolved_entities.push(candidate.clone());
            building_resolved.push(ResolvedBuildingEntity {
                building: building.clone(),
                entity: entity.clone(),
                pdxmesh: pdxmesh.clone(),
            });
        }
        if !building_resolved.is_empty() {
            resolved.insert(building.name.clone(), building_resolved);
        }
        coverage.push(BuildingBindingCoverageRow {
            building: building.name.clone(),
            show_on_map: building.show_on_map,
            show_on_map_meshes: building.show_on_map_meshes,
            candidate_entities: candidates,
            resolved_entities,
            missing_entities,
            missing_meshes,
        });
    }
    (resolved, coverage)
}

pub fn building_preview_instances(
    placements: &[MapBuildingPlacement],
    show_on_map: &[ShowOnMapBuilding],
    resolved: &BTreeMap<String, Vec<ResolvedBuildingEntity>>,
    model_indices: &BTreeMap<String, usize>,
    preview_all: bool,
) -> Vec<BuildingPreviewInstance> {
    if !preview_all {
        return Vec::new();
    }
    let show_by_name = show_on_map
        .iter()
        .map(|row| (row.name.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let mut slots_by_state_building: BTreeMap<(u32, &str), u32> = BTreeMap::new();
    let mut out = Vec::new();
    for placement in placements {
        let Some(show) = show_by_name.get(placement.building.as_str()) else {
            continue;
        };
        let key = (placement.state_id, placement.building.as_str());
        let slot = slots_by_state_building.entry(key).or_insert(0);
        let slot_index = *slot;
        *slot += 1;
        if slot_index >= show.show_on_map {
            continue;
        }
        let Some(variants) = resolved.get(&placement.building) else {
            continue;
        };
        if variants.is_empty() {
            continue;
        }
        let variant_index = (slot_index as usize).min(variants.len().saturating_sub(1));
        let variant = &variants[variant_index];
        let Some(model_index) = model_indices.get(&variant.entity.name).copied() else {
            continue;
        };
        out.push(BuildingPreviewInstance {
            state_id: placement.state_id,
            building: placement.building.clone(),
            slot_index,
            entity_name: variant.entity.name.clone(),
            model_index,
            pos: placement.pos,
            rotation: placement.rotation,
            scale: variant.entity.scale * variant.pdxmesh.scale,
            color: building_preview_color(&placement.building),
        });
    }
    out
}

pub fn building_mesh_vertex_buffer_from_model(
    model: &PdxModel,
    entity: &BuildingEntityBinding,
    pdxmesh: &BuildingPdxmeshBinding,
) -> Result<BuildingMeshVertexBuffer> {
    let mesh = model
        .shapes
        .iter()
        .flat_map(|shape| shape.meshes.iter())
        .find(|mesh| !mesh.material.is_collision() && mesh.skin.is_none() && !mesh.verts.is_empty())
        .ok_or_else(|| {
            format!(
                "building file layer blocked for {}: {} has no non-collision unskinned mesh with vertices",
                entity.name, pdxmesh.mesh
            )
        })?;
    validate_building_mesh_arrays(mesh, &pdxmesh.mesh)?;
    let scale = entity.scale * pdxmesh.scale;
    let mut vertices =
        Vec::with_capacity(mesh.verts.len() * BUILDING_MESH_VERTEX_STRIDE_BYTES as usize);
    for index in 0..mesh.verts.len() {
        let position = mesh.verts[index];
        push_f32s(
            &mut vertices,
            &[
                position[0] * scale,
                position[1] * scale,
                position[2] * scale,
            ],
        );
        push_f32s(
            &mut vertices,
            &mesh
                .normals
                .as_ref()
                .map(|normals| normals[index])
                .unwrap_or([0.0, 1.0, 0.0]),
        );
        push_f32s(
            &mut vertices,
            &mesh
                .tangents
                .as_ref()
                .map(|tangents| tangents[index])
                .unwrap_or([1.0, 0.0, 0.0, 1.0]),
        );
        push_f32s(
            &mut vertices,
            &mesh
                .uv0
                .as_ref()
                .map(|uvs| uvs[index])
                .unwrap_or([0.0, 0.0]),
        );
    }
    Ok(BuildingMeshVertexBuffer {
        source_mesh: pdxmesh.mesh.clone(),
        entity_name: entity.name.clone(),
        pdxmesh_name: pdxmesh.name.clone(),
        total_scale: scale,
        material_shader: mesh.material.shader.clone(),
        material_diffuse: mesh.material.diffuse.clone(),
        material_normal_map: mesh.material.normal_map.clone(),
        material_specular: mesh.material.specular.clone(),
        vertex_stride_bytes: BUILDING_MESH_VERTEX_STRIDE_BYTES,
        vertex_count: mesh.verts.len(),
        index_count: mesh.indices.len(),
        vertices,
        indices: mesh.indices.clone(),
    })
}

fn load_building_models(
    resolver: &GameFileResolver,
    resolved: &BTreeMap<String, Vec<ResolvedBuildingEntity>>,
    mut coverage: Vec<BuildingBindingCoverageRow>,
) -> Result<(
    Vec<BuildingMeshVertexBuffer>,
    BTreeMap<String, usize>,
    Vec<BuildingBindingCoverageRow>,
    Vec<String>,
)> {
    let mut seen_entity = BTreeSet::new();
    let mut models = Vec::new();
    let mut model_indices = BTreeMap::new();
    let mut model_gaps = Vec::new();
    for variants in resolved.values() {
        for variant in variants {
            if !seen_entity.insert(variant.entity.name.clone()) {
                continue;
            }
            let Some(_) = resolver.resolve(&variant.pdxmesh.mesh)? else {
                if let Some(row) = coverage
                    .iter_mut()
                    .find(|row| row.building == variant.building.name)
                {
                    row.missing_meshes.push(format!(
                        "{} -> {} (missing mesh file)",
                        variant.entity.name, variant.pdxmesh.mesh
                    ));
                    row.resolved_entities
                        .retain(|name| name != &variant.entity.name);
                }
                model_gaps.push(format!(
                    "building {} fail-closed: {} -> {} missing mesh file",
                    variant.building.name, variant.entity.name, variant.pdxmesh.mesh
                ));
                continue;
            };
            let maybe_model =
                pdx_model_from_resolver(resolver, &variant.pdxmesh.mesh).and_then(|model| {
                    building_mesh_vertex_buffer_from_model(
                        &model,
                        &variant.entity,
                        &variant.pdxmesh,
                    )
                });
            match maybe_model {
                Ok(model) => {
                    model_indices.insert(variant.entity.name.clone(), models.len());
                    models.push(model);
                }
                Err(err) => {
                    model_gaps.push(format!(
                        "building {} render model skipped fail-closed: {} -> {} ({err})",
                        variant.building.name, variant.entity.name, variant.pdxmesh.mesh
                    ));
                }
            }
        }
    }
    Ok((models, model_indices, coverage, model_gaps))
}

fn prepare_building_shader_from_resolver(resolver: &GameFileResolver) -> BuildingShaderPreparation {
    let cbuffers = building_shader_cbuffer_mapping(resolver).unwrap_or_else(|err| {
        vec![BuildingShaderCBufferMapping {
            slot: 0,
            dx9_base: 0,
            declaration: "unavailable".to_string(),
            filled_bytes: "unavailable".to_string(),
            status: format!("shader cbuffer parse failed: {err}"),
        }]
    });
    let vertex_hlsl = build_stage_hlsl(
        resolver,
        BUILDING_SHADER_SOURCE,
        BUILDING_SHADER_EFFECT,
        ShaderStage::Vertex,
    );
    let pixel_hlsl = build_stage_hlsl(
        resolver,
        BUILDING_SHADER_SOURCE,
        BUILDING_SHADER_EFFECT,
        ShaderStage::Pixel,
    );
    let (vertex_hlsl_bytes, pixel_hlsl_bytes, mut status) = match (&vertex_hlsl, &pixel_hlsl) {
        (Ok(vs), Ok(ps)) => (
            vs.len(),
            ps.len(),
            "PdxMeshStandard VS/PS HLSL reconstructed".to_string(),
        ),
        (Err(err), _) => (
            0,
            pixel_hlsl.as_ref().map(|hlsl| hlsl.len()).unwrap_or(0),
            format!("PdxMeshStandard vertex HLSL unavailable: {err}"),
        ),
        (_, Err(err)) => (
            vertex_hlsl.as_ref().map(|hlsl| hlsl.len()).unwrap_or(0),
            0,
            format!("PdxMeshStandard pixel HLSL unavailable: {err}"),
        ),
    };

    let mut vertex_bytecode_bytes = None;
    let mut pixel_bytecode_bytes = None;
    #[cfg(windows)]
    {
        if let Ok(hlsl) = vertex_hlsl.as_ref() {
            match d3d_compile_stage(hlsl, ShaderStage::Vertex, 0) {
                Ok(bytes) => vertex_bytecode_bytes = Some(bytes.len()),
                Err(err) => status.push_str(&format!("; VS compile failed: {err}")),
            }
        }
        if let Ok(hlsl) = pixel_hlsl.as_ref() {
            match d3d_compile_stage(hlsl, ShaderStage::Pixel, 0) {
                Ok(bytes) => pixel_bytecode_bytes = Some(bytes.len()),
                Err(err) => status.push_str(&format!("; PS compile failed: {err}")),
            }
        }
    }

    BuildingShaderPreparation {
        shader_source: BUILDING_SHADER_SOURCE.to_string(),
        effect_name: BUILDING_SHADER_EFFECT.to_string(),
        vertex_hlsl_bytes,
        pixel_hlsl_bytes,
        vertex_bytecode_bytes,
        pixel_bytecode_bytes,
        status,
        cbuffer_mapping: cbuffers,
    }
}

fn building_shader_cbuffer_mapping(
    resolver: &GameFileResolver,
) -> Result<Vec<BuildingShaderCBufferMapping>> {
    let file = effect_file_from_resolver(resolver, BUILDING_SHADER_SOURCE)?;
    let mut out = vec![BuildingShaderCBufferMapping {
        slot: 0,
        dx9_base: 0,
        declaration: "canonical scene cb0 from gfx/FX/constants.fxh; PdxMeshStandard VS reads ViewProjectionMatrix through included shader code".to_string(),
        filled_bytes: "captured/product map scene CB0 bytes, not guessed by the building layer".to_string(),
        status: "canonical".to_string(),
    }];
    for (_, element) in &file.elements {
        if let EffectElement::ConstantBuffer {
            slot,
            dx9_base,
            body,
        } = element
        {
            if *slot != 1 && *slot != 2 {
                continue;
            }
            let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let (filled, status) = match *slot {
                1 if normalized.contains("WorldMatrix")
                    && normalized.contains("AtlasCoordinate")
                    && normalized.contains("vUVAnimSpeed") =>
                {
                    (
                        "WorldMatrix = placement translation/rotation/scale in renderer-specific preview shader; AtlasCoordinate = (0,0,1,1) GUESS; vUVAnimSpeed = 0 GUESS".to_string(),
                        "slot 1 building draw constants; guessed atlas/UV animation documented".to_string(),
                    )
                }
                2 if normalized.contains("matBones") => (
                    "unfilled; preview skips skinned meshes so bone matrix CB is not used".to_string(),
                    "skinned-only; fail-closed skipped for building preview".to_string(),
                ),
                1 => (
                    "unfilled; duplicate slot 1 train alias is not used by building entities".to_string(),
                    "duplicate slot ignored for PdxMeshTrain alias".to_string(),
                ),
                _ => (
                    "unfilled".to_string(),
                    "not used by this preview layer".to_string(),
                ),
            };
            out.push(BuildingShaderCBufferMapping {
                slot: *slot,
                dx9_base: *dx9_base,
                declaration: normalized,
                filled_bytes: filled,
                status,
            });
        }
    }
    Ok(out)
}

fn parse_assignment_blocks(
    text: &str,
    source: &str,
    key: &str,
) -> Result<Vec<Vec<(usize, String)>>> {
    let mut blocks = Vec::new();
    let mut block: Vec<(usize, String)> = Vec::new();
    let mut depth = 0i32;
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_no = line_index + 1;
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .to_string();
        if depth == 0 {
            if find_assignment(&line, key, 0).is_none() {
                continue;
            }
            block.clear();
        }
        if depth > 0 || find_assignment(&line, key, 0).is_some() {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            block.push((line_no, line));
            if depth <= 0 {
                blocks.push(block.clone());
                block.clear();
                depth = 0;
            }
        }
    }
    if depth != 0 {
        return Err(format!("{source}: unterminated {key} block").into());
    }
    if blocks.is_empty() {
        return Err(format!("{source}: no {key} blocks found").into());
    }
    Ok(blocks)
}

enum ParsedBuildingEntityBlock {
    Binding(BuildingEntityBinding),
    Clone(ParsedBuildingEntityClone),
    Ignored,
}

struct ParsedBuildingEntityClone {
    name: String,
    clone: String,
    scale: Option<f32>,
    source: String,
    block_line: usize,
    name_line: usize,
    scale_line: Option<usize>,
}

impl ParsedBuildingEntityClone {
    fn to_binding(&self, base: &BuildingEntityBinding) -> BuildingEntityBinding {
        BuildingEntityBinding {
            name: self.name.clone(),
            pdxmesh: base.pdxmesh.clone(),
            scale: self.scale.unwrap_or(base.scale),
            source: self.source.clone(),
            block_line: self.block_line,
            name_line: self.name_line,
            pdxmesh_line: base.pdxmesh_line,
            scale_line: self.scale_line.or(base.scale_line),
        }
    }
}

fn parse_building_entity_binding_block(
    block: &[(usize, String)],
    source: &str,
) -> Result<ParsedBuildingEntityBlock> {
    let block_line = block.first().map(|(line, _)| *line).unwrap_or(1);
    let mut name = None;
    let mut name_line = None;
    let mut clone = None;
    let mut pdxmesh = None;
    let mut pdxmesh_line = None;
    let mut scale = None;
    let mut scale_line = None;
    for (line_no, line) in block {
        if name.is_none() {
            if let Some(value) = parse_quoted_value(line, "name") {
                name = Some(value);
                name_line = Some(*line_no);
            }
        }
        if clone.is_none() {
            if let Some(value) = parse_quoted_value(line, "clone") {
                clone = Some(value);
            }
        }
        if pdxmesh.is_none() {
            if let Some(value) = parse_quoted_value(line, "pdxmesh") {
                pdxmesh = Some(value);
                pdxmesh_line = Some(*line_no);
            }
        }
        if scale.is_none() {
            if let Some(value) = parse_token_value(line, "scale") {
                scale = Some(value.parse::<f32>().map_err(|err| {
                    format!("{source}:{line_no}: entity scale is not a float: {err}")
                })?);
                scale_line = Some(*line_no);
            }
        }
    }
    let name = name.ok_or_else(|| format!("{source}:{block_line}: entity block has no name"))?;
    let Some(pdxmesh) = pdxmesh else {
        let Some(clone) = clone else {
            return Ok(ParsedBuildingEntityBlock::Ignored);
        };
        return Ok(ParsedBuildingEntityBlock::Clone(
            ParsedBuildingEntityClone {
                name,
                clone,
                scale,
                source: source.to_string(),
                block_line,
                name_line: name_line.unwrap_or(block_line),
                scale_line,
            },
        ));
    };
    Ok(ParsedBuildingEntityBlock::Binding(BuildingEntityBinding {
        name,
        pdxmesh,
        scale: scale.unwrap_or(1.0),
        source: source.to_string(),
        block_line,
        name_line: name_line.unwrap_or(block_line),
        pdxmesh_line: pdxmesh_line.unwrap_or(block_line),
        scale_line,
    }))
}

fn parse_building_pdxmesh_binding_block(
    block: &[(usize, String)],
    source: &str,
) -> Result<BuildingPdxmeshBinding> {
    let block_line = block.first().map(|(line, _)| *line).unwrap_or(1);
    let mut name = None;
    let mut name_line = None;
    let mut mesh = None;
    let mut file_line = None;
    let mut scale = None;
    let mut scale_line = None;
    for (line_no, line) in block {
        if name.is_none() {
            if let Some(value) = parse_quoted_value(line, "name") {
                name = Some(value);
                name_line = Some(*line_no);
            }
        }
        if mesh.is_none() {
            if let Some(value) = parse_quoted_value(line, "file") {
                mesh = Some(value.replace('\\', "/"));
                file_line = Some(*line_no);
            }
        }
        if scale.is_none() {
            if let Some(value) = parse_token_value(line, "scale") {
                scale = Some(value.parse::<f32>().map_err(|err| {
                    format!("{source}:{line_no}: pdxmesh scale is not a float: {err}")
                })?);
                scale_line = Some(*line_no);
            }
        }
    }
    let name = name.ok_or_else(|| format!("{source}:{block_line}: pdxmesh block has no name"))?;
    let mesh = mesh.ok_or_else(|| format!("{source}:{block_line}: pdxmesh block has no file"))?;
    Ok(BuildingPdxmeshBinding {
        name,
        mesh,
        scale: scale.unwrap_or(1.0),
        source: source.to_string(),
        block_line,
        name_line: name_line.unwrap_or(block_line),
        file_line: file_line.unwrap_or(block_line),
        scale_line,
    })
}

fn candidate_entity_names(building: &ShowOnMapBuilding) -> Vec<String> {
    match building.show_on_map_meshes {
        Some(count) if count > 0 => (1..=count)
            .map(|index| format!("building_{}_{}", building.name, index))
            .collect(),
        _ => vec![format!("building_{}", building.name)],
    }
}

fn validate_building_mesh_arrays(mesh: &PdxMeshData, source: &str) -> Result<()> {
    let vertex_count = mesh.verts.len();
    if let Some(normals) = &mesh.normals {
        if normals.len() != vertex_count {
            return Err(format!(
                "{source}: mesh normal count {} does not match vertex count {vertex_count}",
                normals.len()
            )
            .into());
        }
    }
    if let Some(tangents) = &mesh.tangents {
        if tangents.len() != vertex_count {
            return Err(format!(
                "{source}: mesh tangent count {} does not match vertex count {vertex_count}",
                tangents.len()
            )
            .into());
        }
    }
    if let Some(uv0) = &mesh.uv0 {
        if uv0.len() != vertex_count {
            return Err(format!(
                "{source}: mesh uv0 count {} does not match vertex count {vertex_count}",
                uv0.len()
            )
            .into());
        }
    }
    for (index, vertex_index) in mesh.indices.iter().copied().enumerate() {
        if usize::try_from(vertex_index)
            .ok()
            .filter(|v| *v < vertex_count)
            .is_none()
        {
            return Err(format!(
                "{source}: mesh index {index} references vertex {vertex_index}, but vertex count is {vertex_count}"
            )
            .into());
        }
    }
    Ok(())
}

fn building_preview_color(building: &str) -> [f32; 4] {
    match building {
        "arms_factory" | "industrial_complex" => [0.72, 0.64, 0.52, 1.0],
        "dockyard" | "naval_base" => [0.44, 0.58, 0.72, 1.0],
        "air_base" | "anti_air_building" => [0.60, 0.64, 0.70, 1.0],
        "bunker" | "coastal_bunker" => [0.48, 0.52, 0.46, 1.0],
        "synthetic_refinery" | "fuel_silo" => [0.68, 0.60, 0.44, 1.0],
        _ => [0.64, 0.62, 0.56, 1.0],
    }
}

fn push_f32s<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    #[test]
    fn placement_slots_map_to_instances_only_when_preview_all() {
        let placements = (0..8)
            .map(|slot| MapBuildingPlacement {
                state_id: 10,
                building: "arms_factory".to_string(),
                pos: [100.0 + slot as f32, 2.0, 200.0],
                rotation: slot as f32,
                adjacent_sea_province: 0,
            })
            .collect::<Vec<_>>();
        let show_on_map = vec![ShowOnMapBuilding {
            name: "arms_factory".to_string(),
            show_on_map: 3,
            show_on_map_meshes: None,
        }];
        let entity = BuildingEntityBinding {
            name: "building_arms_factory".to_string(),
            pdxmesh: "building_arms_factory".to_string(),
            scale: 1.0,
            source: BUILDINGS_ENTITY_ASSET_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            pdxmesh_line: 2,
            scale_line: None,
        };
        let pdxmesh = BuildingPdxmeshBinding {
            name: "building_arms_factory".to_string(),
            mesh: "gfx/models/buildings/factory.mesh".to_string(),
            scale: 1.0,
            source: BUILDINGS_GFX_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            file_line: 2,
            scale_line: None,
        };
        let mut resolved = BTreeMap::new();
        resolved.insert(
            "arms_factory".to_string(),
            vec![ResolvedBuildingEntity {
                building: show_on_map[0].clone(),
                entity,
                pdxmesh,
            }],
        );
        let mut model_indices = BTreeMap::new();
        model_indices.insert("building_arms_factory".to_string(), 0);

        assert!(building_preview_instances(
            &placements,
            &show_on_map,
            &resolved,
            &model_indices,
            false,
        )
        .is_empty());

        let instances =
            building_preview_instances(&placements, &show_on_map, &resolved, &model_indices, true);
        assert_eq!(instances.len(), 3);
        assert_eq!(
            instances
                .iter()
                .map(|instance| instance.slot_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(instances[2].pos[0], 102.0);
    }

    #[test]
    fn binding_resolution_reports_missing_mesh_fail_closed() {
        let show_on_map = vec![ShowOnMapBuilding {
            name: "arms_factory".to_string(),
            show_on_map: 6,
            show_on_map_meshes: None,
        }];
        let entities = vec![BuildingEntityBinding {
            name: "building_arms_factory".to_string(),
            pdxmesh: "building_arms_factory".to_string(),
            scale: 1.0,
            source: BUILDINGS_ENTITY_ASSET_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            pdxmesh_line: 2,
            scale_line: None,
        }];
        let meshes = vec![BuildingPdxmeshBinding {
            name: "building_other".to_string(),
            mesh: "gfx/models/buildings/missing.mesh".to_string(),
            scale: 1.0,
            source: BUILDINGS_GFX_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            file_line: 2,
            scale_line: None,
        }];
        let (resolved, coverage) =
            resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes);
        assert!(resolved.is_empty());
        assert_eq!(
            coverage[0].missing_meshes,
            vec!["building_arms_factory -> building_arms_factory".to_string()]
        );
        assert!(!coverage[0].fully_resolved());
    }

    #[test]
    fn parsers_resolve_entity_to_pdxmesh() {
        let entities = parse_building_entity_bindings(
            r#"
            entity = {
                name = "building_air_base"
                pdxmesh = "building_air_base"
                scale = 1.25
            }
            "#,
            BUILDINGS_ENTITY_ASSET_SOURCE,
        )
        .expect("entity parse");
        let meshes = parse_building_pdxmesh_bindings(
            r#"
            objectTypes = {
                pdxmesh = {
                    name = "building_air_base"
                    file = "gfx\models\buildings\hangar.mesh"
                    scale = 0.5
                }
            }
            "#,
            BUILDINGS_GFX_SOURCE,
        )
        .expect("pdxmesh parse");
        let show_on_map = vec![ShowOnMapBuilding {
            name: "air_base".to_string(),
            show_on_map: 1,
            show_on_map_meshes: None,
        }];
        let (resolved, coverage) =
            resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes);
        assert!(coverage[0].fully_resolved());
        assert_eq!(
            resolved["air_base"][0].pdxmesh.mesh,
            "gfx/models/buildings/hangar.mesh"
        );
        assert_eq!(resolved["air_base"][0].entity.scale, 1.25);
        assert_eq!(resolved["air_base"][0].pdxmesh.scale, 0.5);
    }

    #[test]
    fn parser_skips_entities_without_pdxmesh() {
        let entities = parse_building_entity_bindings(
            r#"
            entity = {
                name = "non_mesh_locator"
            }
            entity = {
                name = "building_air_base"
                pdxmesh = "building_air_base"
            }
            "#,
            BUILDINGS_ENTITY_ASSET_SOURCE,
        )
        .expect("entity parse");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "building_air_base");
        assert_eq!(entities[0].pdxmesh, "building_air_base");
    }

    #[test]
    fn parser_resolves_cloned_entity_pdxmesh() {
        let entities = parse_building_entity_bindings(
            r#"
            entity = {
                name = "building_generic_dam"
                pdxmesh = "generic_dam_mesh"
                scale = 2.0
            }
            entity = {
                clone = "building_generic_dam"
                name = "building_dam"
            }
            "#,
            BUILDINGS_ENTITY_ASSET_SOURCE,
        )
        .expect("entity parse");
        let cloned = entities
            .iter()
            .find(|entity| entity.name == "building_dam")
            .expect("cloned dam");
        assert_eq!(cloned.pdxmesh, "generic_dam_mesh");
        assert_eq!(cloned.scale, 2.0);
    }

    #[test]
    fn layer_skips_missing_mesh_files_without_crashing() {
        let root = crate::test_support::temp_test_dir("building_missing_mesh");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            "map/buildings.txt",
            "1;arms_factory;10.0;1.0;20.0;0.0;0\n",
        );
        write_text(
            &vanilla,
            "common/buildings/00_buildings.txt",
            r#"buildings = { arms_factory = { show_on_map = 6 } }"#,
        );
        write_text(
            &vanilla,
            BUILDINGS_ENTITY_ASSET_SOURCE,
            r#"entity = { name = "building_arms_factory" pdxmesh = "building_arms_factory" }"#,
        );
        write_text(
            &vanilla,
            BUILDINGS_GFX_SOURCE,
            r#"objectTypes = { pdxmesh = { name = "building_arms_factory" file = "gfx/models/buildings/missing.mesh" } }"#,
        );
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let layer = building_file_derived_layer_from_resolver(&resolver, true).expect("layer");
        assert_eq!(layer.stats.model_count, 0);
        assert_eq!(layer.stats.instance_count, 0);
        assert_eq!(layer.stats.missing_type_count, 1);
        assert!(
            layer.stats.coverage[0]
                .missing_meshes
                .iter()
                .any(|mesh| mesh.contains("missing.mesh")),
            "{:?}",
            layer.stats.coverage[0].missing_meshes
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gate_vanilla_building_binding_coverage() {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla building binding gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let show_on_map =
            show_on_map_buildings_from_resolver(&resolver).expect("show_on_map buildings");
        let entities =
            building_entity_bindings_from_resolver(&resolver).expect("building entities");
        let meshes = building_pdxmesh_bindings_from_resolver(&resolver).expect("building meshes");
        let (_, coverage) = resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes);
        let missing = coverage
            .iter()
            .filter(|row| !row.fully_resolved())
            .map(|row| {
                format!(
                    "{}: missing entities {:?}, missing meshes {:?}",
                    row.building, row.missing_entities, row.missing_meshes
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(coverage.len(), 29);
        assert_eq!(
            missing,
            vec![
                "fuel_silo: missing entities [\"building_fuel_silo\"], missing meshes []"
                    .to_string()
            ],
            "unexpected show_on_map building binding gaps"
        );
    }

    fn write_text(root: &std::path::Path, rel: &str, text: &str) {
        let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, text).expect("write text");
    }
}
