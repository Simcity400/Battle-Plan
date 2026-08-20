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

impl BuildingEntityBinding {
    pub fn semantically_eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.pdxmesh == other.pdxmesh
            && self.scale.to_bits() == other.scale.to_bits()
    }
}

impl BuildingPdxmeshBinding {
    pub fn semantically_eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.mesh == other.mesh
            && self.scale.to_bits() == other.scale.to_bits()
            && self.mesh_settings == other.mesh_settings
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshSettingTexture {
    pub index: Option<u32>,
    pub texture_diffuse: Option<String>,
    pub texture_normal: Option<String>,
    pub texture_specular: Option<String>,
    pub shader: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingPdxmeshBinding {
    pub name: String,
    pub mesh: String,
    pub scale: f32,
    pub mesh_settings: Vec<MeshSettingTexture>,
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
pub struct BuildingMaterialTextureRef {
    pub entity_name: String,
    pub pdxmesh_name: String,
    pub source_mesh: String,
    pub slot: String,
    pub texture_name: String,
    pub resolved_rel: String,
    pub resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingMaterialTextureReport {
    pub total: usize,
    pub resolved: usize,
    pub refs: Vec<BuildingMaterialTextureRef>,
}

impl BuildingMaterialTextureReport {
    pub fn unresolved(&self) -> impl Iterator<Item = &BuildingMaterialTextureRef> {
        self.refs.iter().filter(|row| !row.resolved)
    }
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
    pub material_texture_count: usize,
    pub material_texture_resolved_count: usize,
    pub material_texture_unresolved: Vec<String>,
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
        resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes)?;
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
    let textures = resolve_building_material_textures(resolver, &models, &meshes)?;
    let resolved_type_count = coverage.iter().filter(|row| row.fully_resolved()).count();
    let missing_type_count = coverage.len().saturating_sub(resolved_type_count);
    let mut gaps = model_gaps;
    let material_texture_unresolved = textures
        .unresolved()
        .map(|row| {
            format!(
                "{} {} texture unresolved: {} (looked up {})",
                row.entity_name, row.slot, row.texture_name, row.resolved_rel
            )
        })
        .collect::<Vec<_>>();
    for message in &material_texture_unresolved {
        gaps.push(format!("building material {message}"));
    }
    if !preview_all {
        gaps.push(
            "--buildings-from-files prepared static placements and meshes but emitted zero instances; recorded state building levels are required before slot visibility is real".to_string(),
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
        material_texture_count: textures.total,
        material_texture_resolved_count: textures.resolved,
        material_texture_unresolved,
        level_policy: if preview_all {
            "visual gate override: render slot 0..show_on_map-1 per state at level>=1 assumption; real playback must gate slots on recorded building levels".to_string()
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
    refuse_on_duplicate_pdxmesh_names(&out)?;
    Ok(out)
}

fn refuse_on_duplicate_pdxmesh_names(bindings: &[BuildingPdxmeshBinding]) -> Result<()> {
    let mut seen: BTreeMap<&str, &BuildingPdxmeshBinding> = BTreeMap::new();
    let mut gaps = Vec::new();
    for binding in bindings {
        if let Some(previous) = seen.get(binding.name.as_str()) {
            if !previous.semantically_eq(binding) {
                gaps.push(format!(
                    "pdxmesh {:?} from {}:{} conflicts with {}:{}",
                    binding.name,
                    binding.source,
                    binding.block_line,
                    previous.source,
                    previous.block_line
                ));
            }
            continue;
        }
        seen.insert(binding.name.as_str(), binding);
    }
    refuse_on_binding_gaps("building pdxmesh bindings", "pdxmesh", &gaps)
}

pub fn parse_building_entity_bindings(
    text: &str,
    source: &str,
) -> Result<Vec<BuildingEntityBinding>> {
    let mut explicit: BTreeMap<String, BuildingEntityBinding> = BTreeMap::new();
    let mut clones = Vec::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut declared: BTreeMap<String, DeclaredEntity> = BTreeMap::new();
    let mut mesh_less: BTreeSet<String> = BTreeSet::new();
    let mut note_declaration = |gaps: &mut Vec<String>,
                                declared: &mut BTreeMap<String, DeclaredEntity>,
                                name: &str,
                                entry: DeclaredEntity|
     -> bool {
        match declared.get(name) {
            Some(previous) if previous.semantically_eq(&entry) => false,
            Some(previous) if previous.kind.label() == entry.kind.label() => {
                gaps.push(format!(
                    "entity {name:?} on line {} conflicts with {} declared on line {}",
                    entry.line,
                    entry.kind.label(),
                    previous.line
                ));
                false
            }
            Some(previous) => {
                gaps.push(format!(
                    "entity {name:?} on line {} is declared as {} but line {} already declared it as {}",
                    entry.line,
                    entry.kind.label(),
                    previous.line,
                    previous.kind.label()
                ));
                false
            }
            None => {
                declared.insert(name.to_string(), entry);
                true
            }
        }
    };
    for block in parse_assignment_blocks(text, source, "entity")? {
        match parse_building_entity_binding_block(&block, source)? {
            ParsedBuildingEntityBlock::Binding(binding) => {
                if !note_declaration(
                    &mut gaps,
                    &mut declared,
                    &binding.name,
                    DeclaredEntity::binding(&binding),
                ) {
                    continue;
                }
                explicit.insert(binding.name.clone(), binding);
            }
            ParsedBuildingEntityBlock::Clone(clone) => {
                if !note_declaration(
                    &mut gaps,
                    &mut declared,
                    &clone.name,
                    DeclaredEntity::clone_of(&clone),
                ) {
                    continue;
                }
                clones.push(clone);
            }
            ParsedBuildingEntityBlock::Ignored { name, block_line } => {
                if !note_declaration(
                    &mut gaps,
                    &mut declared,
                    &name,
                    DeclaredEntity::mesh_less(block_line),
                ) {
                    continue;
                }
                mesh_less.insert(name);
            }
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
            } else if mesh_less.contains(&clone.clone) {
                mesh_less.insert(clone.name.clone());
                changed = true;
            } else {
                still_unresolved.push(clone);
            }
        }
        if still_unresolved.is_empty() {
            break;
        }
        if !changed {
            for clone in &still_unresolved {
                gaps.push(format!(
                    "entity {:?} clones {:?}, which is not defined by any entity block",
                    clone.name, clone.clone
                ));
            }
            break;
        }
        unresolved = still_unresolved;
    }
    refuse_on_binding_gaps(source, "entity", &gaps)?;
    Ok(explicit.into_values().collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DeclaredEntityKind {
    Binding { pdxmesh: String, scale: u32 },
    Clone { base: String, scale: Option<u32> },
    MeshLess,
}

impl DeclaredEntityKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Binding { .. } => "a pdxmesh binding",
            Self::Clone { .. } => "a clone",
            Self::MeshLess => "a mesh-less entity",
        }
    }
}

#[derive(Clone, Debug)]
struct DeclaredEntity {
    kind: DeclaredEntityKind,
    line: usize,
}

impl DeclaredEntity {
    fn binding(binding: &BuildingEntityBinding) -> Self {
        Self {
            kind: DeclaredEntityKind::Binding {
                pdxmesh: binding.pdxmesh.clone(),
                scale: binding.scale.to_bits(),
            },
            line: binding.block_line,
        }
    }

    fn clone_of(clone: &ParsedBuildingEntityClone) -> Self {
        Self {
            kind: DeclaredEntityKind::Clone {
                base: clone.clone.clone(),
                scale: clone.scale.map(f32::to_bits),
            },
            line: clone.block_line,
        }
    }

    fn mesh_less(line: usize) -> Self {
        Self {
            kind: DeclaredEntityKind::MeshLess,
            line,
        }
    }

    fn semantically_eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

pub const MAX_REPORTED_BINDING_GAPS: usize = 20;

fn refuse_on_binding_gaps(source: &str, kind: &str, gaps: &[String]) -> Result<()> {
    if gaps.is_empty() {
        return Ok(());
    }
    let shown = gaps.len().min(MAX_REPORTED_BINDING_GAPS);
    let elided = gaps.len() - shown;
    let suffix = if elided == 0 {
        String::new()
    } else {
        format!("; and {elided} more")
    };
    Err(format!(
        "{source} refused: {} unusable {kind} binding(s) — {}{suffix}",
        gaps.len(),
        gaps[..shown].join("; ")
    )
    .into())
}

pub fn parse_building_pdxmesh_bindings(
    text: &str,
    source: &str,
) -> Result<Vec<BuildingPdxmeshBinding>> {
    let bindings = parse_assignment_blocks(text, source, "pdxmesh")?
        .iter()
        .map(|block| parse_building_pdxmesh_binding_block(block, source))
        .collect::<Result<Vec<_>>>()?;
    refuse_on_duplicate_pdxmesh_names(&bindings)?;
    Ok(bindings)
}

pub fn resolve_show_on_map_building_bindings(
    show_on_map: &[ShowOnMapBuilding],
    entity_bindings: &[BuildingEntityBinding],
    pdxmesh_bindings: &[BuildingPdxmeshBinding],
) -> Result<(
    BTreeMap<String, Vec<ResolvedBuildingEntity>>,
    Vec<BuildingBindingCoverageRow>,
)> {
    let mut entities_by_name: BTreeMap<String, BuildingEntityBinding> = BTreeMap::new();
    let mut gaps = Vec::new();
    for binding in entity_bindings {
        match entities_by_name.get(&binding.name) {
            Some(previous) if !previous.semantically_eq(binding) => gaps.push(format!(
                "entity {:?} from {}:{} conflicts with {}:{}",
                binding.name,
                binding.source,
                binding.block_line,
                previous.source,
                previous.block_line
            )),
            Some(_) => {}
            None => {
                entities_by_name.insert(binding.name.clone(), binding.clone());
            }
        }
    }
    let mut meshes_by_name: BTreeMap<String, BuildingPdxmeshBinding> = BTreeMap::new();
    for binding in pdxmesh_bindings {
        match meshes_by_name.get(&binding.name) {
            Some(previous) if !previous.semantically_eq(binding) => gaps.push(format!(
                "pdxmesh {:?} from {}:{} conflicts with {}:{}",
                binding.name,
                binding.source,
                binding.block_line,
                previous.source,
                previous.block_line
            )),
            Some(_) => {}
            None => {
                meshes_by_name.insert(binding.name.clone(), binding.clone());
            }
        }
    }
    refuse_on_binding_gaps("show_on_map building bindings", "binding", &gaps)?;
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
    Ok((resolved, coverage))
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

pub fn resolve_building_material_textures(
    resolver: &GameFileResolver,
    models: &[BuildingMeshVertexBuffer],
    pdxmesh_bindings: &[BuildingPdxmeshBinding],
) -> Result<BuildingMaterialTextureReport> {
    if models.is_empty() {
        return Ok(BuildingMaterialTextureReport {
            total: 0,
            resolved: 0,
            refs: Vec::new(),
        });
    }
    let settings_by_mesh = pdxmesh_bindings
        .iter()
        .map(|binding| (binding.name.as_str(), binding))
        .collect::<BTreeMap<_, _>>();
    // Filename -> resolver-relative path index across gfx/models: the engine's
    // texture manager is filename-keyed, so the same basename resolves wherever
    // it lives under the models root.
    let mut by_basename: BTreeMap<String, String> = BTreeMap::new();
    for (rel, _) in resolver.resolve_dir_files_recursive(PDX_MODELS_ROOT, Some("dds"))? {
        if let Some(base) = rel.rsplit('/').next() {
            by_basename
                .entry(base.to_ascii_lowercase())
                .or_insert_with(|| rel.clone());
        }
    }

    let mut refs = Vec::new();
    let mut resolved = 0usize;
    for model in models {
        let mesh_dir = model
            .source_mesh
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let overrides = settings_by_mesh
            .get(model.pdxmesh_name.as_str())
            .map(|binding| binding.mesh_settings.as_slice())
            .unwrap_or(&[]);
        let mut slots: Vec<(&'static str, String)> = Vec::new();
        if overrides.is_empty() {
            if let Some(texture) = &model.material_diffuse {
                slots.push(("diffuse", texture.clone()));
            }
            if let Some(texture) = &model.material_normal_map {
                slots.push(("normal", texture.clone()));
            }
            if let Some(texture) = &model.material_specular {
                slots.push(("specular", texture.clone()));
            }
        } else {
            for setting in overrides {
                if let Some(texture) = &setting.texture_diffuse {
                    slots.push(("diffuse", texture.clone()));
                }
                if let Some(texture) = &setting.texture_normal {
                    slots.push(("normal", texture.clone()));
                }
                if let Some(texture) = &setting.texture_specular {
                    slots.push(("specular", texture.clone()));
                }
            }
        }
        for (slot, texture) in slots {
            let normalized = texture.replace('\\', "/");
            let base = normalized
                .rsplit('/')
                .next()
                .unwrap_or(&normalized)
                .to_ascii_lowercase();
            let mut candidates = Vec::new();
            if !mesh_dir.is_empty() && !normalized.starts_with("gfx/") {
                candidates.push(format!("{mesh_dir}/{normalized}"));
            }
            candidates.push(normalized.clone());
            let mut hit = None;
            for candidate in &candidates {
                if resolver.resolve(candidate)?.is_some() {
                    hit = Some(candidate.clone());
                    break;
                }
            }
            if hit.is_none() {
                if let Some(rel) = by_basename.get(&base) {
                    hit = Some(rel.clone());
                }
            }
            let is_resolved = hit.is_some();
            if is_resolved {
                resolved += 1;
            }
            refs.push(BuildingMaterialTextureRef {
                entity_name: model.entity_name.clone(),
                pdxmesh_name: model.pdxmesh_name.clone(),
                source_mesh: model.source_mesh.clone(),
                slot: slot.to_string(),
                texture_name: texture.clone(),
                resolved_rel: hit
                    .unwrap_or_else(|| candidates.first().cloned().unwrap_or(normalized)),
                resolved: is_resolved,
            });
        }
    }
    Ok(BuildingMaterialTextureReport {
        total: refs.len(),
        resolved,
        refs,
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
                        "WorldMatrix = placement translation/rotation/scale in renderer-specific preview shader; AtlasCoordinate + vUVAnimSpeed = DEAD BYTES for every non-skinned building effect: the shipped shader reads them only under #ifdef ATLAS (gfx/FX/pdxmesh.shader:453) / #ifdef UV_ANIM (gfx/FX/pdxmesh.shader:448), and ATLAS is defined only by PdxMeshAdvancedSkinned (pdxmesh.shader:795-801) + PdxMeshAdvancedSnowSkinned (:824-829), UV_ANIM only by PdxMeshAdvancedAnimSkinned (:859-864); PdxMeshStandard (:747-752) and the other non-skinned effects define neither, so the compiled bytecode never reads these slots (D3D_SVF_USED gate: gate_building_cb1_atlas_uvanim_dead_for_standard). Bytes are zero-filled; any value is output-identical.".to_string(),
                        "slot 1 building draw constants; atlas/UV-anim disposition file-derived (compiled-out for non-skinned effects), no guessed values".to_string(),
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
    Ignored {
        name: String,
        block_line: usize,
    },
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
            return Ok(ParsedBuildingEntityBlock::Ignored { name, block_line });
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
        mesh_settings: parse_mesh_settings(block),
        source: source.to_string(),
        block_line,
        name_line: name_line.unwrap_or(block_line),
        file_line: file_line.unwrap_or(block_line),
        scale_line,
    })
}

fn parse_mesh_settings(block: &[(usize, String)]) -> Vec<MeshSettingTexture> {
    let mut out = Vec::new();
    let mut current: Option<MeshSettingTexture> = None;
    let mut depth = 0i32;
    for (_, line) in block {
        if current.is_none() {
            if find_assignment(line, "meshsettings", 0).is_some() {
                current = Some(MeshSettingTexture::default());
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    out.push(current.take().expect("just set"));
                }
            }
            continue;
        }
        if let Some(ms) = current.as_mut() {
            if ms.index.is_none() {
                if let Some(value) = parse_token_value(line, "index") {
                    ms.index = value.parse::<u32>().ok();
                }
            }
            if ms.texture_diffuse.is_none() {
                if let Some(value) = parse_quoted_value(line, "texture_diffuse") {
                    ms.texture_diffuse = Some(value.replace('\\', "/"));
                }
            }
            if ms.texture_normal.is_none() {
                if let Some(value) = parse_quoted_value(line, "texture_normal") {
                    ms.texture_normal = Some(value.replace('\\', "/"));
                }
            }
            if ms.texture_specular.is_none() {
                if let Some(value) = parse_quoted_value(line, "texture_specular") {
                    ms.texture_specular = Some(value.replace('\\', "/"));
                }
            }
            if ms.shader.is_none() {
                if let Some(value) = parse_quoted_value(line, "shader") {
                    ms.shader = Some(value);
                }
            }
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 {
            out.push(current.take().expect("open meshsettings"));
        }
    }
    if let Some(ms) = current.take() {
        out.push(ms);
    }
    out
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
            mesh_settings: Vec::new(),
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
            mesh_settings: Vec::new(),
            source: BUILDINGS_GFX_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            file_line: 2,
            scale_line: None,
        }];
        let (resolved, coverage) =
            resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes)
                .expect("binding resolution");
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
            resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes)
                .expect("binding resolution");
        assert!(coverage[0].fully_resolved());
        assert_eq!(
            resolved["air_base"][0].pdxmesh.mesh,
            "gfx/models/buildings/hangar.mesh"
        );
        assert_eq!(resolved["air_base"][0].entity.scale, 1.25);
        assert_eq!(resolved["air_base"][0].pdxmesh.scale, 0.5);
    }


    #[test]
    fn entity_clone_with_unresolvable_base_is_a_typed_gap_not_a_silent_drop() {
        let text = r#"
            objectTypes = {
                entity = { name = "building_a" pdxmesh = "mesh_a" }
                entity = { name = "building_orphan" clone = "building_absent" }
            }
            "#;
        let error = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect_err("unresolvable clone must refuse");
        let message = error.to_string();
        assert!(message.contains("building_orphan"), "{message}");
        assert!(message.contains("building_absent"), "{message}");
        assert!(message.contains("is not defined"), "{message}");
    }

    #[test]
    fn clone_of_a_mesh_less_entity_is_admitted_not_refused() {
        let text = r#"
            objectTypes = {
                entity = { name = "particle_smoke" }
                entity = { name = "particle_smoke_variant" clone = "particle_smoke" }
                entity = { name = "particle_smoke_variant2" clone = "particle_smoke_variant" }
                entity = { name = "building_a" pdxmesh = "mesh_a" }
            }
            "#;
        let bindings = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect("a clone of a mesh-less entity is valid content");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].name, "building_a");
    }

    #[test]
    fn a_name_declared_by_two_different_entity_kinds_is_refused() {
        let binding_then_mesh_less = r#"
            objectTypes = {
                entity = { name = "building_a" pdxmesh = "mesh_a" }
                entity = { name = "building_a" }
            }
            "#;
        let error =
            parse_building_entity_bindings(binding_then_mesh_less, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect_err("a mesh-less redeclaration must be reported");
        assert!(error.to_string().contains("building_a"), "{error}");
        assert!(error.to_string().contains("mesh-less entity"), "{error}");

        let mesh_less_then_binding = r#"
            objectTypes = {
                entity = { name = "building_a" }
                entity = { name = "building_a" pdxmesh = "mesh_a" }
            }
            "#;
        let error =
            parse_building_entity_bindings(mesh_less_then_binding, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect_err("the reverse order must be reported too");
        assert!(error.to_string().contains("building_a"), "{error}");

        let clone_over_mesh_less = r#"
            objectTypes = {
                entity = { name = "base" pdxmesh = "mesh_a" }
                entity = { name = "building_a" }
                entity = { name = "building_a" clone = "base" }
            }
            "#;
        let error =
            parse_building_entity_bindings(clone_over_mesh_less, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect_err("a clone colliding with a mesh-less name must be reported");
        assert!(error.to_string().contains("building_a"), "{error}");

        let clone_over_binding = r#"
            objectTypes = {
                entity = { name = "base" pdxmesh = "mesh_a" }
                entity = { name = "building_a" pdxmesh = "mesh_b" }
                entity = { name = "building_a" clone = "base" }
            }
            "#;
        let error =
            parse_building_entity_bindings(clone_over_binding, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect_err("a clone colliding with a binding name must be reported");
        assert!(error.to_string().contains("building_a"), "{error}");
    }

    #[test]
    fn clone_of_a_wholly_absent_base_is_still_refused() {
        let text = r#"
            objectTypes = {
                entity = { name = "particle_smoke" }
                entity = { name = "orphan" clone = "never_declared" }
            }
            "#;
        let error = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect_err("an absent base is still a gap");
        assert!(error.to_string().contains("never_declared"), "{error}");
    }

    #[test]
    fn the_public_binding_fold_refuses_conflicting_duplicates() {
        let entity = |pdxmesh: &str, line: usize| BuildingEntityBinding {
            name: "building_a".to_string(),
            pdxmesh: pdxmesh.to_string(),
            scale: 1.0,
            source: BUILDINGS_ENTITY_ASSET_SOURCE.to_string(),
            block_line: line,
            name_line: line,
            pdxmesh_line: line,
            scale_line: None,
        };
        let mesh = BuildingPdxmeshBinding {
            name: "mesh_one".to_string(),
            mesh: "gfx/models/buildings/one.mesh".to_string(),
            scale: 1.0,
            mesh_settings: Vec::new(),
            source: BUILDINGS_GFX_SOURCE.to_string(),
            block_line: 1,
            name_line: 1,
            file_line: 2,
            scale_line: None,
        };
        let error = resolve_show_on_map_building_bindings(
            &[],
            &[entity("mesh_one", 1), entity("mesh_two", 9)],
            &[mesh.clone()],
        )
        .expect_err("conflicting entity duplicates must refuse");
        assert!(error.to_string().contains("building_a"), "{error}");

        resolve_show_on_map_building_bindings(
            &[],
            &[entity("mesh_one", 1), entity("mesh_one", 9)],
            &[mesh.clone(), mesh],
        )
        .expect("identical redefinitions are lossless");
    }

    #[test]
    fn entity_clone_chain_still_resolves() {
        let text = r#"
            objectTypes = {
                entity = { name = "building_base" pdxmesh = "mesh_base" scale = 2.0 }
                entity = { name = "building_mid" clone = "building_base" }
                entity = { name = "building_leaf" clone = "building_mid" }
            }
            "#;
        let bindings = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect("resolvable clone chain");
        assert_eq!(bindings.len(), 3);
        let leaf = bindings
            .iter()
            .find(|binding| binding.name == "building_leaf")
            .expect("leaf binding");
        assert_eq!(leaf.pdxmesh, "mesh_base");
        assert_eq!(leaf.scale, 2.0);
    }

    #[test]
    fn conflicting_duplicate_entity_name_is_refused() {
        let text = r#"
            objectTypes = {
                entity = { name = "building_a" pdxmesh = "mesh_one" }
                entity = { name = "building_a" pdxmesh = "mesh_two" }
            }
            "#;
        let error = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect_err("conflicting duplicate must refuse");
        assert!(error.to_string().contains("building_a"), "{error}");
        assert!(error.to_string().contains("conflicts with"), "{error}");
    }

    #[test]
    fn identical_repeated_clone_and_mesh_less_declarations_are_admitted() {
        let identical_clones = r#"
            objectTypes = {
                entity = { name = "base" pdxmesh = "mesh_a" }
                entity = { name = "derived" clone = "base" }
                entity = { name = "derived" clone = "base" }
            }
            "#;
        let bindings =
            parse_building_entity_bindings(identical_clones, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect("an identical repeated clone is lossless");
        assert_eq!(bindings.len(), 2);

        let identical_mesh_less = r#"
            objectTypes = {
                entity = { name = "particle_smoke" }
                entity = { name = "particle_smoke" }
            }
            "#;
        let bindings =
            parse_building_entity_bindings(identical_mesh_less, BUILDINGS_ENTITY_ASSET_SOURCE)
                .expect("an identical repeated mesh-less entity is lossless");
        assert!(bindings.is_empty());

        let differing_clones = r#"
            objectTypes = {
                entity = { name = "base_one" pdxmesh = "mesh_a" }
                entity = { name = "base_two" pdxmesh = "mesh_b" }
                entity = { name = "derived" clone = "base_one" }
                entity = { name = "derived" clone = "base_two" }
            }
            "#;
        let error = parse_building_entity_bindings(differing_clones, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect_err("a differing repeated clone must refuse");
        assert!(error.to_string().contains("derived"), "{error}");
    }

    #[test]
    fn identical_duplicate_entity_definition_is_admitted() {
        let text = r#"
            objectTypes = {
                entity = { name = "building_a" pdxmesh = "mesh_one" }
                entity = { name = "building_a" pdxmesh = "mesh_one" }
            }
            "#;
        let bindings = parse_building_entity_bindings(text, BUILDINGS_ENTITY_ASSET_SOURCE)
            .expect("identical redefinition is lossless");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].pdxmesh, "mesh_one");
    }

    #[test]
    fn conflicting_duplicate_pdxmesh_name_is_refused() {
        let text = r#"
            objectTypes = {
                pdxmesh = { name = "mesh_a" file = "gfx\models\buildings\one.mesh" }
                pdxmesh = { name = "mesh_a" file = "gfx\models\buildings\two.mesh" }
            }
            "#;
        let error = parse_building_pdxmesh_bindings(text, BUILDINGS_GFX_SOURCE)
            .expect_err("conflicting pdxmesh must refuse");
        let message = error.to_string();
        assert!(message.contains("mesh_a"), "{message}");
        assert!(message.contains("conflicts with"), "{message}");
    }

    #[test]
    fn identical_duplicate_pdxmesh_definition_is_admitted() {
        let text = r#"
            objectTypes = {
                pdxmesh = { name = "mesh_a" file = "gfx\models\buildings\one.mesh" }
                pdxmesh = { name = "mesh_a" file = "gfx\models\buildings\one.mesh" }
            }
            "#;
        let bindings = parse_building_pdxmesh_bindings(text, BUILDINGS_GFX_SOURCE)
            .expect("identical redefinition is lossless");
        assert_eq!(bindings.len(), 2);
        assert!(bindings[0].semantically_eq(&bindings[1]));
    }

    #[test]
    fn conflicting_pdxmesh_across_sources_is_refused() {
        let root = crate::test_support::temp_test_dir("building_pdxmesh_cross_source");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            BUILDINGS_GFX_SOURCE,
            r#"objectTypes = { pdxmesh = { name = "mesh_a" file = "gfx/models/buildings/one.mesh" } }"#,
        );
        write_text(
            &vanilla,
            "gfx/models/buildings/supply_station.gfx",
            r#"objectTypes = { pdxmesh = { name = "mesh_a" file = "gfx/models/buildings/two.mesh" } }"#,
        );
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let error = building_pdxmesh_bindings_from_resolver(&resolver)
            .expect_err("cross-source conflict must refuse");
        let message = error.to_string();
        assert!(message.contains("mesh_a"), "{message}");
        assert!(
            message.contains("gfx/models/buildings/supply_station.gfx"),
            "{message}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pdxmesh_parser_captures_meshsettings_texture_overrides() {
        let meshes = parse_building_pdxmesh_bindings(
            r#"
            objectTypes = {
                pdxmesh = {
                    name = "building_nuclear_facility"
                    file = "gfx\models\buildings\facility_nuclear.mesh"
                    scale = 0.6
                    meshsettings = {
                        name = "facility_nuclearShape"
                        index = 0
                        texture_diffuse = "facility_diffuse.dds"
                        texture_normal = "facility_normal.dds"
                        texture_specular = "facility_specular.dds"
                        shader = "PdxMeshAdvancedSnow"
                    }
                    meshsettings = {
                        name = "facility_nuclearShape"
                        index = 1
                        texture_diffuse = "nuclearreactor_diffuse.dds"
                        texture_normal = "nuclearreactor_normal.dds"
                        texture_specular = "nuclearreactor_specular.dds"
                        shader = "PdxMeshAdvanced"
                    }
                }
                pdxmesh = {
                    name = "building_rocket_site"
                    file = "gfx\models\buildings\rocket_site.mesh"
                    scale = 0.1
                }
            }
            "#,
            BUILDINGS_GFX_SOURCE,
        )
        .expect("pdxmesh parse");
        let facility = meshes
            .iter()
            .find(|binding| binding.name == "building_nuclear_facility")
            .expect("facility binding");
        assert_eq!(facility.mesh_settings.len(), 2);
        assert_eq!(facility.mesh_settings[0].index, Some(0));
        assert_eq!(
            facility.mesh_settings[0].texture_diffuse.as_deref(),
            Some("facility_diffuse.dds")
        );
        assert_eq!(
            facility.mesh_settings[1].texture_diffuse.as_deref(),
            Some("nuclearreactor_diffuse.dds")
        );
        assert_eq!(
            facility.mesh_settings[1].shader.as_deref(),
            Some("PdxMeshAdvanced")
        );
        let rocket = meshes
            .iter()
            .find(|binding| binding.name == "building_rocket_site")
            .expect("rocket binding");
        assert!(
            rocket.mesh_settings.is_empty(),
            "rocket_site has no meshsettings; baked material applies"
        );
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
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_building_binding_coverage() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let show_on_map =
            show_on_map_buildings_from_resolver(&resolver).expect("show_on_map buildings");
        let entities =
            building_entity_bindings_from_resolver(&resolver).expect("building entities");
        let meshes = building_pdxmesh_bindings_from_resolver(&resolver).expect("building meshes");
        let (_, coverage) = resolve_show_on_map_building_bindings(&show_on_map, &entities, &meshes)
            .expect("binding resolution");
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

    #[test]
    fn material_textures_resolve_relative_to_mesh_directory() {
        let models = vec![BuildingMeshVertexBuffer {
            source_mesh: "gfx/models/buildings/bunker.mesh".to_string(),
            entity_name: "building_bunker".to_string(),
            pdxmesh_name: "building_bunker".to_string(),
            total_scale: 1.0,
            material_shader: "PdxMeshSnow".to_string(),
            material_diffuse: Some("bunkercomplex_diffuse.dds".to_string()),
            material_normal_map: Some("bunkercomplex_normal.dds".to_string()),
            material_specular: None,
            vertex_stride_bytes: BUILDING_MESH_VERTEX_STRIDE_BYTES,
            vertex_count: 0,
            index_count: 0,
            vertices: Vec::new(),
            indices: Vec::new(),
        }];
        let root = crate::test_support::temp_test_dir("building_material_textures_meshdir");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            "gfx/models/buildings/bunkercomplex_diffuse.dds",
            "x",
        );
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let report = resolve_building_material_textures(&resolver, &models, &[]).expect("textures");
        assert_eq!(report.total, 2);
        assert_eq!(report.resolved, 1);
        let diffuse = report
            .refs
            .iter()
            .find(|row| row.slot == "diffuse")
            .expect("diffuse row");
        assert!(diffuse.resolved);
        assert_eq!(
            diffuse.resolved_rel,
            "gfx/models/buildings/bunkercomplex_diffuse.dds"
        );
        let unresolved = report.unresolved().collect::<Vec<_>>();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].slot, "normal");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_building_material_textures_resolve() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let layer = building_file_derived_layer_from_resolver(&resolver, false).expect("layer");
        let pdxmeshes =
            building_pdxmesh_bindings_from_resolver(&resolver).expect("pdxmesh bindings");
        let report = resolve_building_material_textures(&resolver, &layer.models, &pdxmeshes)
            .expect("texture report");
        println!(
            "gate_building_material_textures: {} models, {} texture refs, {} resolved, {} \
             unresolved",
            layer.models.len(),
            report.total,
            report.resolved,
            report.total - report.resolved
        );
        for row in report.unresolved() {
            println!(
                "  UNRESOLVED {} {} {} (looked up {})",
                row.entity_name, row.slot, row.texture_name, row.resolved_rel
            );
        }
        assert!(
            !layer.models.is_empty(),
            "expected loaded building meshes for the resolved bindings"
        );
        assert!(
            report.total >= layer.models.len(),
            "every loaded mesh should reference at least one material texture"
        );
        assert_eq!(
            report.resolved,
            report.total,
            "unresolved building material textures: {:?}",
            report
                .unresolved()
                .map(|row| format!("{} {} {}", row.entity_name, row.slot, row.texture_name))
                .collect::<Vec<_>>()
        );
        assert!(
            layer.stats.material_texture_unresolved.is_empty(),
            "layer stats should carry no unresolved-texture gaps: {:?}",
            layer.stats.material_texture_unresolved
        );
        assert_eq!(layer.stats.material_texture_count, report.total);
        assert_eq!(layer.stats.material_texture_resolved_count, report.resolved);
    }

    fn write_text(root: &std::path::Path, rel: &str, text: &str) {
        let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, text).expect("write text");
    }

    #[cfg(windows)]
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_building_cb1_atlas_uvanim_dead_for_standard() {
        use crate::shader_compile::{
            build_stage_hlsl, d3d_compile_stage, parse_dxbc_reflection, ShaderStage,
        };
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");

        let var_used = |effect: &str, stage: ShaderStage, var: &str| -> Option<bool> {
            let hlsl = build_stage_hlsl(&resolver, BUILDING_SHADER_SOURCE, effect, stage)
                .unwrap_or_else(|err| panic!("{effect} HLSL: {err}"));
            let blob = d3d_compile_stage(&hlsl, stage, 0)
                .unwrap_or_else(|err| panic!("{effect} compile: {err}"));
            let reflection = parse_dxbc_reflection(&blob).expect("reflection");
            reflection
                .cbuffers
                .iter()
                .flat_map(|cb| &cb.vars)
                .find(|v| v.name == var)
                .map(|v| v.used)
        };

        for stage in [ShaderStage::Vertex, ShaderStage::Pixel] {
            for var in ["AtlasCoordinate", "vUVAnimSpeed"] {
                let used = var_used("PdxMeshStandard", stage, var);
                assert_ne!(
                    used,
                    Some(true),
                    "PdxMeshStandard {stage:?} {var}: compiled bytecode reads the constant; \
                     the dead-bytes disposition is wrong"
                );
            }
        }
        assert_eq!(
            var_used(
                "PdxMeshAdvancedSkinned",
                ShaderStage::Pixel,
                "AtlasCoordinate"
            ),
            Some(true),
            "PdxMeshAdvancedSkinned PS must read AtlasCoordinate (ATLAS define)"
        );
        assert_eq!(
            var_used(
                "PdxMeshAdvancedAnimSkinned",
                ShaderStage::Pixel,
                "vUVAnimSpeed"
            ),
            Some(true),
            "PdxMeshAdvancedAnimSkinned PS must read vUVAnimSpeed (UV_ANIM define)"
        );
    }
}
