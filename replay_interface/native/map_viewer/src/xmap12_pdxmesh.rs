
use crate::{
    extract_pdx_model, parse_pdx_asset, GameFileResolver, L03DrawArguments, L03MaterialFamily,
    L03PdxMeshGroupInput, L03PdxMeshObjectInput, L03PdxMeshSubmeshInput, L03SemanticInputRow,
    L03TypedOwner, MapObjectDrawArguments, MapObjectMaterialFamily, MapObjectMeshGroupContent,
    MapObjectRenderContent, MapObjectSubmeshContent, PreparedMapObjectDraw, Result,
    ViewerMapCameraState, L03_PARTITION_MASK_ALL, L03_PARTITION_PRIMARY, L03_PARTITION_WRAP_EAST,
    L03_PARTITION_WRAP_WEST,
};
use replay_core::{UsedContentKey, ValidatedContentAdmission};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const XMAP12_PDXMESH_MATERIALIZATION_SCHEMA: &str =
    "hoi4.replay.xmap12.pdxmesh-materialization.a729.v1";
pub const XMAP12_VERTEX_STRIDE_BYTES: u32 = 48;
pub const XMAP13_SKINNED_VERTEX_STRIDE_BYTES: u32 = 76;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap12PdxMeshMaterialFamily {
    Standard,
    FixedMapFurnitureBorder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12TextureSource {
    pub slot: String,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12MeshSettingOverride {
    pub shape_index: u32,
    pub texture_diffuse: Option<String>,
    pub texture_normal: Option<String>,
    pub texture_specular: Option<String>,
    pub shader: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12PdxMeshRequest {
    pub visible_order: u32,
    pub stable_typed_id: String,
    pub owner: L03TypedOwner,
    pub entity_token: String,
    pub pdxmesh_token: String,
    pub entity_source: UsedContentKey,
    pub pdxmesh_declaration_source: UsedContentKey,
    pub mesh_source: UsedContentKey,
    pub shader_source: UsedContentKey,
    pub ordered_texture_sources: Vec<Xmap12TextureSource>,
    pub mesh_settings: Vec<Xmap12MeshSettingOverride>,
    pub world_matrix_f32_bits: [u32; 16],
    pub entity_scale_f32_bits: u32,
    pub pdxmesh_scale_f32_bits: u32,
    pub material_family: Xmap12PdxMeshMaterialFamily,
    #[serde(default)]
    pub selected_lod_override: Option<u32>,
    #[serde(default)]
    pub selected_shape_indices: Option<Vec<u32>>,
    #[serde(default)]
    pub skin_palette_3200_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12MaterializedSubmesh {
    pub group_index: u32,
    pub object_group_ordinal: u32,
    pub object_submesh_ordinal: u32,
    pub shape_name: String,
    pub material_shader: String,
    pub diffuse: Option<String>,
    pub normal_map: Option<String>,
    pub specular: Option<String>,
    pub vertex_stride_bytes: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub vertex_bytes: Vec<u8>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12PdxMeshMaterialization {
    pub schema: String,
    pub stable_typed_id: String,
    pub owner: L03TypedOwner,
    pub entity_token: String,
    pub pdxmesh_token: String,
    pub mesh_source: UsedContentKey,
    pub shader_source: UsedContentKey,
    pub ordered_texture_sources: Vec<Xmap12TextureSource>,
    pub selected_lod: u32,
    pub partition_mask: u8,
    pub world_matrix_f32_bits: [u32; 16],
    pub world_bounding_sphere_f32_bits: [u32; 4],
    pub material_family: Xmap12PdxMeshMaterialFamily,
    pub skin_palette_3200_bytes: Option<Vec<u8>>,
    pub submeshes: Vec<Xmap12MaterializedSubmesh>,
    pub object_draw: PreparedMapObjectDraw,
    pub deterministic_sha256: String,
}

impl Xmap12PdxMeshMaterialization {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_PDXMESH_MATERIALIZATION_SCHEMA
            || self.stable_typed_id.trim().is_empty()
            || self.entity_token.trim().is_empty()
            || self.pdxmesh_token.trim().is_empty()
            || self.partition_mask == 0
            || self.partition_mask & !L03_PARTITION_MASK_ALL != 0
            || self.submeshes.is_empty()
        {
            return Err("invalid XMAP12 PdxMesh materialization identity/content".into());
        }
        self.mesh_source
            .validate()
            .map_err(|error| format!("invalid XMAP12 mesh identity: {error}"))?;
        self.shader_source
            .validate()
            .map_err(|error| format!("invalid XMAP12 shader identity: {error}"))?;
        validate_affine(self.world_matrix_f32_bits)?;
        if self
            .skin_palette_3200_bytes
            .as_ref()
            .is_some_and(|palette| palette.len() != 3_200)
        {
            return Err("XMAP13 skin palette is not exactly 3,200 bytes".into());
        }
        let sphere = self.world_bounding_sphere_f32_bits.map(f32::from_bits);
        if sphere.iter().any(|value| !value.is_finite()) || sphere[3] < 0.0 {
            return Err("XMAP12 world bounding sphere is invalid".into());
        }
        for submesh in &self.submeshes {
            if !matches!(
                submesh.vertex_stride_bytes,
                XMAP12_VERTEX_STRIDE_BYTES | XMAP13_SKINNED_VERTEX_STRIDE_BYTES
            ) || (submesh.vertex_stride_bytes == XMAP13_SKINNED_VERTEX_STRIDE_BYTES)
                != self.skin_palette_3200_bytes.is_some()
                || submesh.vertex_count == 0
                || submesh.index_count == 0
                || submesh.vertex_bytes.len()
                    != submesh.vertex_count as usize * submesh.vertex_stride_bytes as usize
                || submesh.indices.len() != submesh.index_count as usize
                || submesh
                    .indices
                    .iter()
                    .any(|index| *index >= submesh.vertex_count)
            {
                return Err("XMAP12 materialized submesh geometry is invalid".into());
            }
            let shared = self
                .object_draw
                .render
                .groups
                .get(submesh.object_group_ordinal as usize)
                .and_then(|group| group.submeshes.get(submesh.object_submesh_ordinal as usize))
                .ok_or("XMAP12 materialized geometry has no shared object submesh")?;
            if shared.selected_material != submesh.material_shader
                || shared.draw
                    != (MapObjectDrawArguments::DrawIndexed {
                        index_count: submesh.index_count,
                        start_index: 0,
                        base_vertex: 0,
                    })
            {
                return Err(
                    "XMAP12 materialized geometry drifts from shared object submesh".into(),
                );
            }
        }
        let shared_submesh_count = self
            .object_draw
            .render
            .groups
            .iter()
            .map(|group| group.submeshes.len())
            .sum::<usize>();
        if shared_submesh_count != self.submeshes.len() {
            return Err("XMAP12 shared/object geometry submesh count differs".into());
        }
        self.object_draw
            .validate(
                self.object_draw.visible_order,
                &self.mesh_source.effective_vfs_manifest_sha256,
            )
            .map_err(|error| format!("XMAP12 shared map-object contract: {error}"))?;
        if self.object_draw.stable_typed_id != self.stable_typed_id
            || self.object_draw.partition_mask != self.partition_mask
            || self.object_draw.render.world_matrix_f32_bits != self.world_matrix_f32_bits
            || self.object_draw.render.selected_mesh
                != self.mesh_source.normalized_game_relative_path
            || self.object_draw.render.skinned != self.skin_palette_3200_bytes.is_some()
        {
            return Err("XMAP12 materialization drifts from shared map-object draw".into());
        }
        validate_sha256(&self.deterministic_sha256)?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 materialization digest mismatch".into());
        }
        Ok(())
    }

    pub fn l03_caster_row(&self) -> L03SemanticInputRow {
        L03SemanticInputRow::PdxmeshObject(L03PdxMeshObjectInput {
            stable_typed_upstream_id: self.stable_typed_id.clone(),
            owner: self.owner,
            world_matrix_f32_bits: self.world_matrix_f32_bits,
            selected_mesh: self.mesh_source.normalized_game_relative_path.clone(),
            groups: self
                .object_draw
                .render
                .groups
                .iter()
                .map(|group| L03PdxMeshGroupInput {
                    submeshes: group
                        .submeshes
                        .iter()
                        .map(|submesh| L03PdxMeshSubmeshInput {
                            selected_submesh_material: submesh.selected_material.clone(),
                            material_family: match submesh.material_family {
                                MapObjectMaterialFamily::Standard => L03MaterialFamily::Standard,
                                MapObjectMaterialFamily::FixedMapFurnitureBorder => {
                                    L03MaterialFamily::FixedMapFurnitureBorder
                                }
                                MapObjectMaterialFamily::Tree => L03MaterialFamily::Standard,
                            },
                            draw: match submesh.draw {
                                MapObjectDrawArguments::Draw {
                                    vertex_count,
                                    start_vertex,
                                } => L03DrawArguments::Draw {
                                    vertex_count,
                                    start_vertex,
                                },
                                MapObjectDrawArguments::DrawIndexed {
                                    index_count,
                                    start_index,
                                    base_vertex,
                                } => L03DrawArguments::DrawIndexed {
                                    index_count,
                                    start_index,
                                    base_vertex,
                                },
                                MapObjectDrawArguments::DrawIndexedInstanced {
                                    index_count_per_instance,
                                    instance_count,
                                    start_index,
                                    base_vertex,
                                    start_instance,
                                } => L03DrawArguments::DrawIndexedInstanced {
                                    index_count_per_instance,
                                    instance_count,
                                    start_index,
                                    base_vertex,
                                    start_instance,
                                },
                            },
                        })
                        .collect(),
                })
                .collect(),
            skinned: self.skin_palette_3200_bytes.is_some(),
            partition_mask: self.partition_mask,
        })
    }
}

pub fn materialize_xmap12_pdxmesh(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    camera: &ViewerMapCameraState,
    request: &Xmap12PdxMeshRequest,
) -> Result<Option<Xmap12PdxMeshMaterialization>> {
    camera.validate()?;
    validate_request(request)?;
    for content in [
        &request.entity_source,
        &request.pdxmesh_declaration_source,
        &request.mesh_source,
        &request.shader_source,
    ] {
        admitted_bytes(resolver, admission, content)?;
    }
    for texture in &request.ordered_texture_sources {
        if texture.slot.trim().is_empty() {
            return Err("XMAP12 texture binding has a blank slot".into());
        }
        admitted_bytes(resolver, admission, &texture.content)?;
    }

    let mesh_bytes = admitted_bytes(resolver, admission, &request.mesh_source)?;
    let tree = parse_pdx_asset(&mesh_bytes)?;
    let model = extract_pdx_model(&tree)?;
    let effect_file = crate::effect_file_from_resolver(
        resolver,
        &request.shader_source.normalized_game_relative_path,
    )?;
    let local_sphere = model_bounding_sphere(&model)?;
    let total_scale = f32::from_bits(request.entity_scale_f32_bits)
        * f32::from_bits(request.pdxmesh_scale_f32_bits);
    if !total_scale.is_finite() || total_scale <= 0.0 {
        return Err("XMAP12 entity/PdxMesh total scale is invalid".into());
    }
    let world_matrix = scaled_world_matrix(request.world_matrix_f32_bits, total_scale)?;
    let world_sphere = transform_sphere(local_sphere, world_matrix)?;
    let product_camera = camera.production_camera()?;
    let partition_mask =
        sphere_partition_mask(camera, product_camera.view_projection_matrix, world_sphere)?;
    if partition_mask == 0 {
        return Ok(None);
    }
    let selected_lod = request.selected_lod_override.unwrap_or(select_pdxmesh_lod(
        &model.lod_distances,
        squared_distance(
            world_sphere[..3].try_into().expect("three lanes"),
            camera.current_eye(),
        ),
    )?);

    let mut submeshes = Vec::new();
    let mut shared_groups = Vec::new();
    for (group_index, shape) in model.shapes.iter().enumerate() {
        if shape.lod.unwrap_or(0).max(0) as u32 != selected_lod {
            continue;
        }
        if request
            .selected_shape_indices
            .as_ref()
            .is_some_and(|indices| !indices.contains(&(group_index as u32)))
        {
            continue;
        }
        let mut shared_submeshes = Vec::new();
        let object_group_ordinal = u32::try_from(shared_groups.len())?;
        let mesh_setting = mesh_setting_for_shape(&request.mesh_settings, group_index);
        for mesh in &shape.meshes {
            if mesh.material.is_collision() || mesh.verts.is_empty() || mesh.indices.is_empty() {
                continue;
            }
            if mesh.skin.is_some() != request.skin_palette_3200_bytes.is_some() {
                return Err("PdxMesh skin arrays and XMAP13 palette presence disagree".into());
            }
            let effective = effective_material(mesh, mesh_setting);
            validate_material_effects(&effect_file, &effective, mesh)?;
            let materialized = materialize_submesh(
                u32::try_from(group_index)?,
                object_group_ordinal,
                u32::try_from(shared_submeshes.len())?,
                &shape.name,
                mesh,
                &effective,
            )?;
            let texture_sources =
                texture_sources_for_material(&effective, &request.ordered_texture_sources)?;
            shared_submeshes.push(MapObjectSubmeshContent {
                selected_material: materialized.material_shader.clone(),
                material_source: request.mesh_source.clone(),
                material_family: match request.material_family {
                    Xmap12PdxMeshMaterialFamily::Standard => MapObjectMaterialFamily::Standard,
                    Xmap12PdxMeshMaterialFamily::FixedMapFurnitureBorder => {
                        MapObjectMaterialFamily::FixedMapFurnitureBorder
                    }
                },
                ordered_texture_sources: texture_sources,
                draw: MapObjectDrawArguments::DrawIndexed {
                    index_count: materialized.index_count,
                    start_index: 0,
                    base_vertex: 0,
                },
            });
            submeshes.push(materialized);
        }
        if !shared_submeshes.is_empty() {
            shared_groups.push(MapObjectMeshGroupContent {
                submeshes: shared_submeshes,
            });
        }
    }
    if submeshes.is_empty() {
        return Err(format!(
            "XMAP12 selected LOD {selected_lod} has no drawable submesh in {}",
            request.mesh_source.normalized_game_relative_path
        )
        .into());
    }
    let object_draw = PreparedMapObjectDraw {
        visible_order: request.visible_order,
        stable_typed_id: request.stable_typed_id.clone(),
        partition_mask,
        render: MapObjectRenderContent {
            entity_name: request.entity_token.clone(),
            entity_source: request.entity_source.clone(),
            selected_mesh: request.mesh_source.normalized_game_relative_path.clone(),
            mesh_source: request.mesh_source.clone(),
            groups: shared_groups,
            world_matrix_f32_bits: world_matrix.map(f32::to_bits),
            skinned: request.skin_palette_3200_bytes.is_some(),
        },
    };
    let mut result = Xmap12PdxMeshMaterialization {
        schema: XMAP12_PDXMESH_MATERIALIZATION_SCHEMA.into(),
        stable_typed_id: request.stable_typed_id.clone(),
        owner: request.owner,
        entity_token: request.entity_token.clone(),
        pdxmesh_token: request.pdxmesh_token.clone(),
        mesh_source: request.mesh_source.clone(),
        shader_source: request.shader_source.clone(),
        ordered_texture_sources: request.ordered_texture_sources.clone(),
        selected_lod,
        partition_mask,
        world_matrix_f32_bits: world_matrix.map(f32::to_bits),
        world_bounding_sphere_f32_bits: world_sphere.map(f32::to_bits),
        material_family: request.material_family,
        skin_palette_3200_bytes: request.skin_palette_3200_bytes.clone(),
        submeshes,
        object_draw,
        deterministic_sha256: String::new(),
    };
    result.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&result)?);
    result.validate()?;
    Ok(Some(result))
}

pub fn select_pdxmesh_lod(lod_distances: &[f32], distance_squared: f32) -> Result<u32> {
    if !distance_squared.is_finite()
        || distance_squared < 0.0
        || lod_distances
            .iter()
            .any(|distance| !distance.is_finite() || *distance < 0.0)
    {
        return Err("XMAP12 PdxMesh LOD input is invalid".into());
    }
    let mut selected = u32::try_from(lod_distances.len())?;
    for (index, distance) in lod_distances.iter().enumerate().rev() {
        if distance * distance <= distance_squared {
            selected = u32::try_from(index)?;
            break;
        }
    }
    Ok(selected)
}

fn validate_request(request: &Xmap12PdxMeshRequest) -> Result<()> {
    if request.stable_typed_id.trim().is_empty()
        || request.entity_token.trim().is_empty()
        || request.pdxmesh_token.trim().is_empty()
    {
        return Err("XMAP12 PdxMesh request identity is blank".into());
    }
    validate_affine(request.world_matrix_f32_bits)?;
    if request
        .skin_palette_3200_bytes
        .as_ref()
        .is_some_and(|palette| palette.len() != 3_200)
    {
        return Err("XMAP13 PdxMesh request palette is not exactly 3,200 bytes".into());
    }
    if request
        .selected_shape_indices
        .as_ref()
        .is_some_and(|indices| {
            indices.is_empty()
                || indices
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != indices.len()
        })
    {
        return Err("XMAP13 PdxMesh request shape selection is empty or duplicated".into());
    }
    for bits in [
        request.entity_scale_f32_bits,
        request.pdxmesh_scale_f32_bits,
    ] {
        let value = f32::from_bits(bits);
        if !value.is_finite() || value <= 0.0 {
            return Err("XMAP12 PdxMesh authored scale is invalid".into());
        }
    }
    let mut mesh_setting_indices = std::collections::BTreeSet::new();
    if request.mesh_settings.iter().any(|row| {
        !mesh_setting_indices.insert(row.shape_index)
            || row
                .shader
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
    }) {
        return Err("XMAP12 PdxMesh meshsettings are duplicate or invalid".into());
    }
    Ok(())
}

fn mesh_setting_for_shape(
    settings: &[Xmap12MeshSettingOverride],
    raw_shape_index: usize,
) -> Option<&Xmap12MeshSettingOverride> {
    settings
        .iter()
        .find(|row| row.shape_index as usize == raw_shape_index)
}

fn validate_material_effects(
    file: &crate::EffectFile,
    material: &EffectiveXmap12Material,
    mesh: &crate::PdxMeshData,
) -> Result<()> {
    let main = file
        .effects
        .iter()
        .find(|effect| effect.name == material.shader)
        .ok_or_else(|| {
            format!(
                "XMAP12 effective pdxmesh.shader has no selected effect {}",
                material.shader
            )
        })?;
    if main.name.contains("Skinned") != mesh.skin.is_some() {
        return Err(format!(
            "selected effect {} and PdxMesh skin arrays disagree",
            material.shader
        )
        .into());
    }
    if main.defines.iter().any(|define| define == "PDX_MESH_UV1") {
        return Err(format!(
            "XMAP12 selected effect {} requires UV1, which is not representable by the exact 48-byte static layout",
            material.shader
        )
        .into());
    }
    if mesh.normals.is_none() || mesh.tangents.is_none() || mesh.uv0.is_none() {
        return Err(format!(
            "XMAP12 selected effect {} requires complete position/normal/tangent/UV0 geometry",
            material.shader
        )
        .into());
    }
    let shadow_name = format!("{}Shadow", material.shader);
    if file.effects.iter().all(|effect| effect.name != shadow_name) {
        return Err(format!(
            "XMAP12 selected effect {} has no exact shadow effect {}",
            material.shader, shadow_name
        )
        .into());
    }
    Ok(())
}

fn admitted_bytes(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    content: &UsedContentKey,
) -> Result<Vec<u8>> {
    content
        .validate()
        .map_err(|error| format!("invalid XMAP12 content identity: {error}"))?;
    let admitted = admission
        .require_used_content(&content.normalized_game_relative_path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    if admitted != content {
        return Err(format!(
            "XMAP12 content identity differs from admission for {}",
            content.normalized_game_relative_path
        )
        .into());
    }
    resolver.read_admitted_used_content(content)
}

fn materialize_submesh(
    group_index: u32,
    object_group_ordinal: u32,
    object_submesh_ordinal: u32,
    shape_name: &str,
    mesh: &crate::PdxMeshData,
    material: &EffectiveXmap12Material,
) -> Result<Xmap12MaterializedSubmesh> {
    validate_mesh_arrays(mesh)?;
    let stride = if mesh.skin.is_some() {
        XMAP13_SKINNED_VERTEX_STRIDE_BYTES
    } else {
        XMAP12_VERTEX_STRIDE_BYTES
    };
    let mut vertex_bytes = Vec::with_capacity(mesh.verts.len() * stride as usize);
    for index in 0..mesh.verts.len() {
        push_f32s(&mut vertex_bytes, &mesh.verts[index]);
        push_f32s(
            &mut vertex_bytes,
            &mesh.normals.as_ref().expect("validated normals")[index],
        );
        push_f32s(
            &mut vertex_bytes,
            &mesh.tangents.as_ref().expect("validated tangents")[index],
        );
        push_f32s(
            &mut vertex_bytes,
            &mesh.uv0.as_ref().expect("validated UV0")[index],
        );
        if let Some(skin) = mesh.skin.as_ref() {
            for joint in &skin.joint_indices[index * 4..index * 4 + 4] {
                let joint =
                    u32::try_from(*joint).map_err(|_| "XMAP13 PdxMesh bone index is negative")?;
                if joint >= 50 {
                    return Err(
                        "XMAP13 PdxMesh bone index exceeds the 50-matrix shader palette".into(),
                    );
                }
                vertex_bytes.extend_from_slice(&joint.to_le_bytes());
            }
            let weights: [f32; 3] = skin.weights[index * 4..index * 4 + 3]
                .try_into()
                .expect("three skin weights");
            push_f32s(&mut vertex_bytes, &weights);
        }
    }
    Ok(Xmap12MaterializedSubmesh {
        group_index,
        object_group_ordinal,
        object_submesh_ordinal,
        shape_name: shape_name.into(),
        material_shader: material.shader.clone(),
        diffuse: material.diffuse.clone(),
        normal_map: material.normal_map.clone(),
        specular: material.specular.clone(),
        vertex_stride_bytes: stride,
        vertex_count: u32::try_from(mesh.verts.len())?,
        index_count: u32::try_from(mesh.indices.len())?,
        vertex_bytes,
        indices: mesh.indices.clone(),
    })
}

#[derive(Clone, Debug)]
struct EffectiveXmap12Material {
    shader: String,
    diffuse: Option<String>,
    normal_map: Option<String>,
    specular: Option<String>,
}

fn effective_material(
    mesh: &crate::PdxMeshData,
    override_row: Option<&Xmap12MeshSettingOverride>,
) -> EffectiveXmap12Material {
    EffectiveXmap12Material {
        shader: override_row
            .and_then(|row| row.shader.clone())
            .unwrap_or_else(|| mesh.material.shader.clone()),
        diffuse: override_row
            .and_then(|row| row.texture_diffuse.clone())
            .or_else(|| mesh.material.diffuse.clone()),
        normal_map: override_row
            .and_then(|row| row.texture_normal.clone())
            .or_else(|| mesh.material.normal_map.clone()),
        specular: override_row
            .and_then(|row| row.texture_specular.clone())
            .or_else(|| mesh.material.specular.clone()),
    }
}

fn texture_sources_for_material(
    material: &EffectiveXmap12Material,
    supplied: &[Xmap12TextureSource],
) -> Result<Vec<UsedContentKey>> {
    let mut out = Vec::new();
    for (slot, authored) in [
        ("diffuse", material.diffuse.as_deref()),
        ("normal", material.normal_map.as_deref()),
        ("specular", material.specular.as_deref()),
    ] {
        let Some(authored) = authored else {
            continue;
        };
        let authored = authored.replace('\\', "/").to_ascii_lowercase();
        let authored_base = authored.rsplit('/').next().unwrap_or(&authored);
        let matches = supplied
            .iter()
            .filter(|row| {
                if !row.slot.eq_ignore_ascii_case(slot) {
                    return false;
                }
                let path = row
                    .content
                    .normalized_game_relative_path
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                path == authored || path.rsplit('/').next() == Some(authored_base)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!(
                "XMAP12 material texture {slot}={authored:?} resolved to {} admitted rows",
                matches.len()
            )
            .into());
        }
        out.push(matches[0].content.clone());
    }
    Ok(out)
}

fn validate_mesh_arrays(mesh: &crate::PdxMeshData) -> Result<()> {
    let count = mesh.verts.len();
    for (name, actual) in [
        ("normal", mesh.normals.as_ref().map(Vec::len)),
        ("tangent", mesh.tangents.as_ref().map(Vec::len)),
        ("uv0", mesh.uv0.as_ref().map(Vec::len)),
    ] {
        if actual.is_some_and(|actual| actual != count) {
            return Err(format!("XMAP12 mesh {name} count differs from vertex count").into());
        }
    }
    if mesh.indices.iter().any(|index| *index as usize >= count) {
        return Err("XMAP12 mesh index is outside the vertex buffer".into());
    }
    Ok(())
}

fn push_f32s<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn model_bounding_sphere(model: &crate::PdxModel) -> Result<[f32; 4]> {
    let meshes = model
        .shapes
        .iter()
        .flat_map(|shape| shape.meshes.iter())
        .filter(|mesh| !mesh.material.is_collision() && !mesh.verts.is_empty())
        .collect::<Vec<_>>();
    if meshes.is_empty() {
        return Err("XMAP12 PdxMesh has no drawable mesh for bounds".into());
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for mesh in &meshes {
        for lane in 0..3 {
            min[lane] = min[lane].min(mesh.aabb_min[lane]);
            max[lane] = max[lane].max(mesh.aabb_max[lane]);
        }
    }
    if min.iter().chain(max.iter()).any(|value| !value.is_finite()) {
        return Err("XMAP12 PdxMesh AABB is non-finite".into());
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let mut radius = 0.0f32;
    for mesh in meshes {
        let sphere = mesh.bounding_sphere.unwrap_or_else(|| {
            let local_center = [
                (mesh.aabb_min[0] + mesh.aabb_max[0]) * 0.5,
                (mesh.aabb_min[1] + mesh.aabb_max[1]) * 0.5,
                (mesh.aabb_min[2] + mesh.aabb_max[2]) * 0.5,
            ];
            let local_radius = squared_distance(local_center, mesh.aabb_max).sqrt();
            [
                local_center[0],
                local_center[1],
                local_center[2],
                local_radius,
            ]
        });
        radius = radius
            .max(squared_distance([sphere[0], sphere[1], sphere[2]], center).sqrt() + sphere[3]);
    }
    Ok([center[0], center[1], center[2], radius])
}

fn scaled_world_matrix(bits: [u32; 16], scale: f32) -> Result<[f32; 16]> {
    validate_affine(bits)?;
    let mut matrix = bits.map(f32::from_bits);
    for row in 0..3 {
        for column in 0..3 {
            matrix[row * 4 + column] *= scale;
        }
    }
    Ok(matrix)
}

fn transform_sphere(local: [f32; 4], matrix: [f32; 16]) -> Result<[f32; 4]> {
    let center = [
        local[0] * matrix[0] + local[1] * matrix[4] + local[2] * matrix[8] + matrix[12],
        local[0] * matrix[1] + local[1] * matrix[5] + local[2] * matrix[9] + matrix[13],
        local[0] * matrix[2] + local[1] * matrix[6] + local[2] * matrix[10] + matrix[14],
    ];
    let max_scale = [
        matrix[0] * matrix[0] + matrix[1] * matrix[1] + matrix[2] * matrix[2],
        matrix[4] * matrix[4] + matrix[5] * matrix[5] + matrix[6] * matrix[6],
        matrix[8] * matrix[8] + matrix[9] * matrix[9] + matrix[10] * matrix[10],
    ]
    .into_iter()
    .fold(0.0f32, f32::max)
    .sqrt();
    let result = [center[0], center[1], center[2], local[3] * max_scale];
    if result.iter().any(|value| !value.is_finite()) || result[3] < 0.0 {
        return Err("XMAP12 transformed sphere is invalid".into());
    }
    Ok(result)
}

fn sphere_partition_mask(
    camera: &ViewerMapCameraState,
    main_matrix: [[f32; 4]; 4],
    sphere: [f32; 4],
) -> Result<u8> {
    let mut mask = 0u8;
    if sphere_visible(main_matrix, sphere)? {
        mask |= L03_PARTITION_PRIMARY;
    }
    let map_width = camera.map_width as f32;
    let projection = crate::ProductionMapCameraProjection::from_engine_constants(
        camera.viewport_width as f32 / camera.viewport_height as f32,
    )?;
    for (shift, bit) in [
        (-map_width, L03_PARTITION_WRAP_WEST),
        (map_width, L03_PARTITION_WRAP_EAST),
    ] {
        let mut eye = camera.current_eye();
        let mut look_at = camera.current_look_at();
        eye[0] += shift;
        look_at[0] += shift;
        let shifted = projection.camera_from_eye_look_at(eye, look_at)?;
        if sphere_visible(shifted.view_projection_matrix, sphere)? {
            mask |= bit;
        }
    }
    Ok(mask)
}

fn sphere_visible(matrix: [[f32; 4]; 4], sphere: [f32; 4]) -> Result<bool> {
    let raw = [
        [
            matrix[0][0] + matrix[0][3],
            matrix[1][0] + matrix[1][3],
            matrix[2][0] + matrix[2][3],
            matrix[3][0] + matrix[3][3],
        ],
        [
            matrix[0][3] - matrix[0][0],
            matrix[1][3] - matrix[1][0],
            matrix[2][3] - matrix[2][0],
            matrix[3][3] - matrix[3][0],
        ],
        [
            matrix[0][3] - matrix[0][1],
            matrix[1][3] - matrix[1][1],
            matrix[2][3] - matrix[2][1],
            matrix[3][3] - matrix[3][1],
        ],
        [
            matrix[0][1] + matrix[0][3],
            matrix[1][1] + matrix[1][3],
            matrix[2][1] + matrix[2][3],
            matrix[3][1] + matrix[3][3],
        ],
    ];
    for mut plane in raw {
        let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
        if !length.is_finite() || length <= 0.0 {
            return Err("XMAP12 camera produced an invalid side plane".into());
        }
        for lane in &mut plane {
            *lane = -*lane / length;
        }
        if sphere[3] < sphere[0] * plane[0] + sphere[1] * plane[1] + sphere[2] * plane[2] + plane[3]
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2])
}

fn validate_affine(bits: [u32; 16]) -> Result<()> {
    if bits.iter().any(|bits| !f32::from_bits(*bits).is_finite())
        || bits[3] != 0.0f32.to_bits()
        || bits[7] != 0.0f32.to_bits()
        || bits[11] != 0.0f32.to_bits()
        || bits[15] != 1.0f32.to_bits()
    {
        return Err("XMAP12 world matrix is not finite affine".into());
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("XMAP12 digest is not canonical lowercase SHA-256".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_selection_scans_squared_thresholds_from_last_to_first() {
        assert_eq!(select_pdxmesh_lod(&[10.0, 20.0], 0.0).unwrap(), 2);
        assert_eq!(select_pdxmesh_lod(&[10.0, 20.0], 100.0).unwrap(), 0);
        assert_eq!(select_pdxmesh_lod(&[10.0, 20.0], 399.0).unwrap(), 0);
        assert_eq!(select_pdxmesh_lod(&[10.0, 20.0], 400.0).unwrap(), 1);
    }

    #[test]
    fn authored_scale_changes_basis_not_translation() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 1.0,
        ]
        .map(f32::to_bits);
        let scaled = scaled_world_matrix(identity, 3.0).unwrap();
        assert_eq!([scaled[12], scaled[13], scaled[14]], [4.0, 5.0, 6.0]);
        assert_eq!([scaled[0], scaled[5], scaled[10]], [3.0, 3.0, 3.0]);
    }

    #[test]
    fn aggregate_sphere_covers_each_component_sphere() {
        let model = crate::PdxModel {
            lod_distances: Vec::new(),
            shapes: vec![crate::PdxShape {
                name: "shape".into(),
                lod: None,
                meshes: vec![
                    crate::PdxMeshData {
                        verts: vec![[0.0, 0.0, 0.0]],
                        normals: None,
                        tangents: None,
                        uv0: None,
                        uv1: None,
                        indices: vec![0, 0, 0],
                        aabb_min: [0.0, 0.0, 0.0],
                        aabb_max: [2.0, 2.0, 2.0],
                        bounding_sphere: Some([1.0, 1.0, 1.0, 1.0]),
                        material: crate::PdxMaterial {
                            shader: "PdxMeshStandard".into(),
                            diffuse: None,
                            normal_map: None,
                            specular: None,
                        },
                        skin: None,
                    },
                    crate::PdxMeshData {
                        verts: vec![[8.0, 0.0, 0.0]],
                        normals: None,
                        tangents: None,
                        uv0: None,
                        uv1: None,
                        indices: vec![0, 0, 0],
                        aabb_min: [8.0, 0.0, 0.0],
                        aabb_max: [10.0, 2.0, 2.0],
                        bounding_sphere: Some([9.0, 1.0, 1.0, 1.0]),
                        material: crate::PdxMaterial {
                            shader: "PdxMeshStandard".into(),
                            diffuse: None,
                            normal_map: None,
                            specular: None,
                        },
                        skin: None,
                    },
                ],
                skeleton: Vec::new(),
            }],
            locators: Vec::new(),
        };
        assert_eq!(model_bounding_sphere(&model).unwrap(), [5.0, 1.0, 1.0, 5.0]);
    }

    #[test]
    fn meshsettings_index_selects_raw_shape_not_inner_submesh() {
        let settings = vec![
            Xmap12MeshSettingOverride {
                shape_index: 0,
                texture_diffuse: Some("shape_zero.dds".into()),
                texture_normal: None,
                texture_specular: None,
                shader: None,
            },
            Xmap12MeshSettingOverride {
                shape_index: 2,
                texture_diffuse: Some("shape_two.dds".into()),
                texture_normal: None,
                texture_specular: None,
                shader: None,
            },
        ];
        assert_eq!(
            mesh_setting_for_shape(&settings, 0).and_then(|row| row.texture_diffuse.as_deref()),
            Some("shape_zero.dds")
        );
        assert!(mesh_setting_for_shape(&settings, 1).is_none());
        assert_eq!(
            mesh_setting_for_shape(&settings, 2).and_then(|row| row.texture_diffuse.as_deref()),
            Some("shape_two.dds")
        );
    }

    #[test]
    fn skinned_materialization_emits_exact_shader_vertex_layout() {
        let mesh = crate::PdxMeshData {
            verts: vec![[1.0, 2.0, 3.0]],
            normals: Some(vec![[0.0, 1.0, 0.0]]),
            tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]]),
            uv0: Some(vec![[0.25, 0.75]]),
            uv1: None,
            indices: vec![0, 0, 0],
            aabb_min: [1.0, 2.0, 3.0],
            aabb_max: [1.0, 2.0, 3.0],
            bounding_sphere: Some([1.0, 2.0, 3.0, 0.0]),
            material: crate::PdxMaterial {
                shader: "PdxMeshStandardSkinned".into(),
                diffuse: None,
                normal_map: None,
                specular: None,
            },
            skin: Some(crate::PdxSkin {
                influences_per_vertex: 4,
                joint_indices: vec![1, 2, 3, 4],
                weights: vec![0.4, 0.3, 0.2, 0.1],
            }),
        };
        let material = EffectiveXmap12Material {
            shader: "PdxMeshStandardSkinned".into(),
            diffuse: None,
            normal_map: None,
            specular: None,
        };
        let draw = materialize_submesh(0, 0, 0, "body", &mesh, &material)
            .expect("exact skinned materialization");
        assert_eq!(draw.vertex_stride_bytes, XMAP13_SKINNED_VERTEX_STRIDE_BYTES);
        assert_eq!(draw.vertex_bytes.len(), 76);
        assert_eq!(
            &draw.vertex_bytes[48..64],
            &[1u32, 2, 3, 4]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &draw.vertex_bytes[64..76],
            &[0.4f32, 0.3, 0.2]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn skinned_materialization_rejects_palette_overflow_bone_index() {
        let mesh = crate::PdxMeshData {
            verts: vec![[0.0, 0.0, 0.0]],
            normals: Some(vec![[0.0, 1.0, 0.0]]),
            tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]]),
            uv0: Some(vec![[0.0, 0.0]]),
            uv1: None,
            indices: vec![0, 0, 0],
            aabb_min: [0.0; 3],
            aabb_max: [0.0; 3],
            bounding_sphere: Some([0.0; 4]),
            material: crate::PdxMaterial {
                shader: "PdxMeshStandardSkinned".into(),
                diffuse: None,
                normal_map: None,
                specular: None,
            },
            skin: Some(crate::PdxSkin {
                influences_per_vertex: 1,
                joint_indices: vec![50, -1, -1, -1],
                weights: vec![1.0, 0.0, 0.0, 0.0],
            }),
        };
        let material = EffectiveXmap12Material {
            shader: "PdxMeshStandardSkinned".into(),
            diffuse: None,
            normal_map: None,
            specular: None,
        };
        assert!(materialize_submesh(0, 0, 0, "body", &mesh, &material).is_err());
    }
}
