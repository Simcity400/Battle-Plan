
use crate::Result;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;

pub const L03_PARTITION_PRIMARY: u8 = 1;
pub const L03_PARTITION_WRAP_WEST: u8 = 2;
pub const L03_PARTITION_WRAP_EAST: u8 = 4;
pub const L03_PARTITION_MASK_ALL: u8 =
    L03_PARTITION_PRIMARY | L03_PARTITION_WRAP_WEST | L03_PARTITION_WRAP_EAST;
pub const L03_LIGHT_DEPTH_WIDTH: u32 = 2048;
pub const L03_LIGHT_DEPTH_HEIGHT: u32 = 2048;
pub const L03_LIGHT_DEPTH_DXGI_FORMAT: u32 = 41;
pub const L03_LIGHT_DEPTH_MIP_LEVELS: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L03SelectedLight {
    Sun,
    Moon,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03LightDepthTarget {
    pub stable_resource_id: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub mip_levels: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03SelectedLightFrame {
    pub selected_light: L03SelectedLight,
    pub light_view_projection_f32_bits: [u32; 16],
    pub shadow_map_texture_matrix_f32_bits: [u32; 16],
    pub light_depth_target: L03LightDepthTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L03TypedOwner {
    GenericPdxmesh,
    M01Unit,
    O04City,
    O05StaticFixed,
    M04RailwayBridge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L03MaterialFamily {
    Standard,
    FixedMapFurnitureBorder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L03ShadowPipeline {
    StandardUnskinned,
    StandardSkinned,
    FixedMapFurnitureBorder,
    TreeShadow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub enum L03DrawArguments {
    Draw {
        vertex_count: u32,
        start_vertex: u32,
    },
    DrawIndexed {
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index: u32,
        base_vertex: i32,
        start_instance: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03PdxMeshSubmeshInput {
    pub selected_submesh_material: String,
    pub material_family: L03MaterialFamily,
    pub draw: L03DrawArguments,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03PdxMeshGroupInput {
    pub submeshes: Vec<L03PdxMeshSubmeshInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03PdxMeshObjectInput {
    pub stable_typed_upstream_id: String,
    pub owner: L03TypedOwner,
    pub world_matrix_f32_bits: [u32; 16],
    pub selected_mesh: String,
    pub groups: Vec<L03PdxMeshGroupInput>,
    pub skinned: bool,
    pub partition_mask: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "producer_family", rename_all = "snake_case")]
pub enum L03SemanticInputRow {
    PdxmeshObject(L03PdxMeshObjectInput),
    Particle { stable_typed_upstream_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03TreeBatchInput {
    pub source_row_ordinal: u32,
    pub selected_mesh_ordinal: u32,
    pub sort_byte: u8,
    pub sort_value_f32_bits: u32,
    pub partition_mask: u8,
    pub selected_lod_index: u32,
    pub selected_mesh: String,
    pub selected_material: String,
    pub draw: L03DrawArguments,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03ProjectionRequest {
    pub shadow_pass_active: Option<bool>,
    pub selected_light_frame: Option<L03SelectedLightFrame>,
    pub enabled_partition_mask: Option<u8>,
    pub semantic_rows: Option<Vec<L03SemanticInputRow>>,
    pub tree_batches: Option<Vec<L03TreeBatchInput>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum L03LightDepthSubmission {
    MapArrow { draw: crate::MapArrowLightDraw },
    Pdxmesh {
        partition_mask: u8,
        stable_typed_upstream_id: String,
        owner: L03TypedOwner,
        world_matrix_f32_bits: [u32; 16],
        selected_mesh: String,
        selected_submesh_material: String,
        group_ordinal: u32,
        submesh_ordinal: u32,
        pipeline: L03ShadowPipeline,
        draw: L03DrawArguments,
    },
    Tree {
        partition_mask: u8,
        source_row_ordinal: u32,
        selected_mesh_ordinal: u32,
        selected_lod_index: u32,
        selected_mesh: String,
        selected_material: String,
        pipeline: L03ShadowPipeline,
        draw: L03DrawArguments,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L03LightDepthProjection {
    pub selected_light: Option<L03SelectedLight>,
    pub light_depth_target: Option<L03LightDepthTarget>,
    pub inactive_excluded: bool,
    pub excluded_particle_rows: usize,
    pub submissions: Vec<L03LightDepthSubmission>,
}

fn fail(message: impl Into<String>) -> Box<dyn std::error::Error> {
    message.into().into()
}

fn validate_f32_bits(bits: u32, label: &str) -> Result<f32> {
    let value = f32::from_bits(bits);
    if !value.is_finite() {
        return Err(fail(format!(
            "L03 projection blocked: {label} is not finite"
        )));
    }
    Ok(value)
}

fn validate_matrix(bits: &[u32; 16], label: &str) -> Result<()> {
    for (lane, raw) in bits.iter().copied().enumerate() {
        validate_f32_bits(raw, &format!("{label}[{lane}]"))?;
    }
    Ok(())
}

fn validate_draw(draw: L03DrawArguments, label: &str) -> Result<()> {
    let empty = match draw {
        L03DrawArguments::Draw { vertex_count, .. } => vertex_count == 0,
        L03DrawArguments::DrawIndexed { index_count, .. } => index_count == 0,
        L03DrawArguments::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            ..
        } => index_count_per_instance == 0 || instance_count == 0,
    };
    if empty {
        return Err(fail(format!(
            "L03 projection blocked: {label} has an empty exact draw"
        )));
    }
    Ok(())
}

fn validate_partition_mask(mask: u8, label: &str) -> Result<()> {
    if mask == 0 || mask & !L03_PARTITION_MASK_ALL != 0 {
        return Err(fail(format!(
            "L03 projection blocked: {label} has invalid partition mask {mask:#04x}"
        )));
    }
    Ok(())
}

fn validate_selected_light(frame: &L03SelectedLightFrame) -> Result<()> {
    if frame
        .light_depth_target
        .stable_resource_id
        .trim()
        .is_empty()
    {
        return Err(fail(
            "L03 projection blocked: selected light-depth resource identity is missing",
        ));
    }
    let target = &frame.light_depth_target;
    if target.width != L03_LIGHT_DEPTH_WIDTH
        || target.height != L03_LIGHT_DEPTH_HEIGHT
        || target.dxgi_format != L03_LIGHT_DEPTH_DXGI_FORMAT
        || target.mip_levels != L03_LIGHT_DEPTH_MIP_LEVELS
    {
        return Err(fail(format!(
            "L03 projection blocked: selected light-depth target must be {}x{} format {} mip {}",
            L03_LIGHT_DEPTH_WIDTH,
            L03_LIGHT_DEPTH_HEIGHT,
            L03_LIGHT_DEPTH_DXGI_FORMAT,
            L03_LIGHT_DEPTH_MIP_LEVELS
        )));
    }
    validate_matrix(
        &frame.light_view_projection_f32_bits,
        "light_view_projection_f32_bits",
    )?;
    validate_matrix(
        &frame.shadow_map_texture_matrix_f32_bits,
        "shadow_map_texture_matrix_f32_bits",
    )?;
    let mut light_view_projection = [[0.0f32; 4]; 4];
    for row in 0..4 {
        for column in 0..4 {
            light_view_projection[row][column] =
                f32::from_bits(frame.light_view_projection_f32_bits[row * 4 + column]);
        }
    }
    let composed = crate::shadow_law::compose_shadow_map_texture_matrix(light_view_projection)?;
    let composed_bits: [u32; 16] =
        std::array::from_fn(|lane| composed[lane / 4][lane % 4].to_bits());
    if composed_bits != frame.shadow_map_texture_matrix_f32_bits {
        return Err(fail(
            "L03 projection blocked: selected-light texture matrix is not the exact L02 light-view-projection composition",
        ));
    }
    Ok(())
}

fn pipeline_for(submesh: &L03PdxMeshSubmeshInput, skinned: bool) -> Result<L03ShadowPipeline> {
    match (submesh.material_family, skinned) {
        (L03MaterialFamily::Standard, false) => Ok(L03ShadowPipeline::StandardUnskinned),
        (L03MaterialFamily::Standard, true) => Ok(L03ShadowPipeline::StandardSkinned),
        (L03MaterialFamily::FixedMapFurnitureBorder, false) => {
            Ok(L03ShadowPipeline::FixedMapFurnitureBorder)
        }
        (L03MaterialFamily::FixedMapFurnitureBorder, true) => Err(fail(
            "L03 projection blocked: fixed map-furniture border shadow cannot be skinned",
        )),
    }
}

fn validate_object(object: &L03PdxMeshObjectInput) -> Result<()> {
    if object.stable_typed_upstream_id.trim().is_empty() {
        return Err(fail(
            "L03 projection blocked: stable typed upstream ID is missing",
        ));
    }
    if object.selected_mesh.trim().is_empty() {
        return Err(fail(format!(
            "L03 projection blocked: {} has no selected mesh",
            object.stable_typed_upstream_id
        )));
    }
    validate_matrix(
        &object.world_matrix_f32_bits,
        &format!("{} world_matrix_f32_bits", object.stable_typed_upstream_id),
    )?;
    validate_partition_mask(object.partition_mask, &object.stable_typed_upstream_id)?;
    if object.groups.is_empty() {
        return Err(fail(format!(
            "L03 projection blocked: {} has no stored PdxMesh groups",
            object.stable_typed_upstream_id
        )));
    }
    for (group_ordinal, group) in object.groups.iter().enumerate() {
        if group.submeshes.is_empty() {
            return Err(fail(format!(
                "L03 projection blocked: {} group {group_ordinal} has no stored submeshes",
                object.stable_typed_upstream_id
            )));
        }
        for (submesh_ordinal, submesh) in group.submeshes.iter().enumerate() {
            if submesh.selected_submesh_material.trim().is_empty() {
                return Err(fail(format!(
                    "L03 projection blocked: {} group {group_ordinal} submesh {submesh_ordinal} has no selected material",
                    object.stable_typed_upstream_id
                )));
            }
            validate_draw(
                submesh.draw,
                &format!(
                    "{} group {group_ordinal} submesh {submesh_ordinal}",
                    object.stable_typed_upstream_id
                ),
            )?;
            pipeline_for(submesh, object.skinned)?;
        }
    }
    Ok(())
}

fn validate_tree_batch(batch: &L03TreeBatchInput) -> Result<f32> {
    validate_partition_mask(
        batch.partition_mask,
        &format!("tree source row {}", batch.source_row_ordinal),
    )?;
    if batch.selected_mesh.trim().is_empty() || batch.selected_material.trim().is_empty() {
        return Err(fail(format!(
            "L03 projection blocked: tree source row {} lacks exact selected LOD mesh/material",
            batch.source_row_ordinal
        )));
    }
    validate_draw(
        batch.draw,
        &format!("tree source row {}", batch.source_row_ordinal),
    )?;
    validate_f32_bits(
        batch.sort_value_f32_bits,
        &format!(
            "tree source row {} comparator value",
            batch.source_row_ordinal
        ),
    )
}

pub fn project_l03_light_depth(request: &L03ProjectionRequest) -> Result<L03LightDepthProjection> {
    let shadow_pass_active = request
        .shadow_pass_active
        .ok_or_else(|| fail("L03 projection blocked: exact shadow-pass active input is missing"))?;
    if !shadow_pass_active {
        return Ok(L03LightDepthProjection {
            selected_light: None,
            light_depth_target: None,
            inactive_excluded: true,
            excluded_particle_rows: 0,
            submissions: Vec::new(),
        });
    }

    let selected = request
        .selected_light_frame
        .as_ref()
        .ok_or_else(|| fail("L03 projection blocked: exact L02 selected-light frame is missing"))?;
    validate_selected_light(selected)?;

    let enabled = request
        .enabled_partition_mask
        .ok_or_else(|| fail("L03 projection blocked: exact enabled partition mask is missing"))?;
    validate_partition_mask(enabled, "enabled partition set")?;
    if enabled & L03_PARTITION_PRIMARY == 0 {
        return Err(fail(
            "L03 projection blocked: primary partition mask 1 is absent",
        ));
    }

    let rows = request
        .semantic_rows
        .as_ref()
        .ok_or_else(|| fail("L03 projection blocked: ordered typed semantic rows are missing"))?;
    let tree_batches = request
        .tree_batches
        .as_ref()
        .ok_or_else(|| fail("L03 projection blocked: exact tree producer rows are missing"))?;

    let mut typed_ids = BTreeSet::new();
    let mut excluded_particle_rows = 0usize;
    for row in rows {
        match row {
            L03SemanticInputRow::PdxmeshObject(object) => {
                validate_object(object)?;
                if !typed_ids.insert(object.stable_typed_upstream_id.as_str()) {
                    return Err(fail(format!(
                        "L03 projection blocked: duplicate stable typed upstream ID {}",
                        object.stable_typed_upstream_id
                    )));
                }
            }
            L03SemanticInputRow::Particle {
                stable_typed_upstream_id,
            } => {
                if stable_typed_upstream_id.trim().is_empty() {
                    return Err(fail(
                        "L03 projection blocked: particle producer row has no stable typed ID",
                    ));
                }
                excluded_particle_rows += 1;
            }
        }
    }

    let mut validated_trees = Vec::with_capacity(tree_batches.len());
    for batch in tree_batches {
        let value = validate_tree_batch(batch)?;
        validated_trees.push((batch, value));
    }

    let mut submissions = Vec::new();
    for partition in [
        L03_PARTITION_PRIMARY,
        L03_PARTITION_WRAP_WEST,
        L03_PARTITION_WRAP_EAST,
    ] {
        if enabled & partition == 0 {
            continue;
        }

        for row in rows {
            let L03SemanticInputRow::PdxmeshObject(object) = row else {
                continue;
            };
            if object.partition_mask & partition == 0 {
                continue;
            }
            for (group_ordinal, group) in object.groups.iter().enumerate() {
                for (submesh_ordinal, submesh) in group.submeshes.iter().enumerate() {
                    submissions.push(L03LightDepthSubmission::Pdxmesh {
                        partition_mask: partition,
                        stable_typed_upstream_id: object.stable_typed_upstream_id.clone(),
                        owner: object.owner,
                        world_matrix_f32_bits: object.world_matrix_f32_bits,
                        selected_mesh: object.selected_mesh.clone(),
                        selected_submesh_material: submesh.selected_submesh_material.clone(),
                        group_ordinal: group_ordinal as u32,
                        submesh_ordinal: submesh_ordinal as u32,
                        pipeline: pipeline_for(submesh, object.skinned)?,
                        draw: submesh.draw,
                    });
                }
            }
        }
    }
    for partition in [
        L03_PARTITION_PRIMARY,
        L03_PARTITION_WRAP_WEST,
        L03_PARTITION_WRAP_EAST,
    ] {
        if enabled & partition == 0 {
            continue;
        }
        let mut partition_trees: Vec<_> = validated_trees
            .iter()
            .copied()
            .filter(|(batch, _)| batch.partition_mask & partition != 0)
            .collect();
        partition_trees.sort_by(|(left, left_value), (right, right_value)| {
            left.sort_byte.cmp(&right.sort_byte).then_with(|| {
                left_value
                    .partial_cmp(right_value)
                    .unwrap_or(Ordering::Equal)
            })
        });
        let mut source_mesh_keys = BTreeSet::new();
        for (batch, _) in partition_trees {
            if !source_mesh_keys.insert((batch.source_row_ordinal, batch.selected_mesh_ordinal)) {
                return Err(fail(format!(
                    "L03 projection blocked: duplicate tree source row {} mesh {} in partition {partition}",
                    batch.source_row_ordinal, batch.selected_mesh_ordinal
                )));
            }
            submissions.push(L03LightDepthSubmission::Tree {
                partition_mask: partition,
                source_row_ordinal: batch.source_row_ordinal,
                selected_mesh_ordinal: batch.selected_mesh_ordinal,
                selected_lod_index: batch.selected_lod_index,
                selected_mesh: batch.selected_mesh.clone(),
                selected_material: batch.selected_material.clone(),
                pipeline: L03ShadowPipeline::TreeShadow,
                draw: batch.draw,
            });
        }
    }

    let first_tree = submissions
        .iter()
        .position(|row| matches!(row, L03LightDepthSubmission::Tree { .. }));
    if let Some(first_tree) = first_tree {
        if submissions[first_tree..]
            .iter()
            .any(|row| matches!(row, L03LightDepthSubmission::Pdxmesh { .. }))
        {
            return Err(fail(
                "L03 projection blocked: exact retained law forbids PdxMesh submission after the first tree batch",
            ));
        }
    }

    Ok(L03LightDepthProjection {
        selected_light: Some(selected.selected_light),
        light_depth_target: Some(selected.light_depth_target.clone()),
        inactive_excluded: false,
        excluded_particle_rows,
        submissions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(value: f32) -> u32 {
        value.to_bits()
    }

    fn identity_bits() -> [u32; 16] {
        let mut out = [bits(0.0); 16];
        for lane in [0usize, 5, 10, 15] {
            out[lane] = bits(1.0);
        }
        out
    }

    fn selected_light() -> L03SelectedLightFrame {
        let light_view_projection = identity_bits();
        let mut matrix = [[0.0f32; 4]; 4];
        for row in 0..4 {
            for column in 0..4 {
                matrix[row][column] = f32::from_bits(light_view_projection[row * 4 + column]);
            }
        }
        let texture = crate::shadow_law::compose_shadow_map_texture_matrix(matrix).unwrap();
        L03SelectedLightFrame {
            selected_light: L03SelectedLight::Sun,
            light_view_projection_f32_bits: light_view_projection,
            shadow_map_texture_matrix_f32_bits: std::array::from_fn(|lane| {
                texture[lane / 4][lane % 4].to_bits()
            }),
            light_depth_target: L03LightDepthTarget {
                stable_resource_id: "frame/light_depth".into(),
                width: 2048,
                height: 2048,
                dxgi_format: 41,
                mip_levels: 1,
            },
        }
    }

    fn draw() -> L03DrawArguments {
        L03DrawArguments::DrawIndexed {
            index_count: 6,
            start_index: 0,
            base_vertex: 0,
        }
    }

    fn object(id: &str, owner: L03TypedOwner, mask: u8) -> L03SemanticInputRow {
        L03SemanticInputRow::PdxmeshObject(L03PdxMeshObjectInput {
            stable_typed_upstream_id: id.into(),
            owner,
            world_matrix_f32_bits: identity_bits(),
            selected_mesh: format!("{id}.mesh"),
            groups: vec![L03PdxMeshGroupInput {
                submeshes: vec![L03PdxMeshSubmeshInput {
                    selected_submesh_material: "standard".into(),
                    material_family: L03MaterialFamily::Standard,
                    draw: draw(),
                }],
            }],
            skinned: false,
            partition_mask: mask,
        })
    }

    fn request(rows: Vec<L03SemanticInputRow>) -> L03ProjectionRequest {
        L03ProjectionRequest {
            shadow_pass_active: Some(true),
            selected_light_frame: Some(selected_light()),
            enabled_partition_mask: Some(1),
            semantic_rows: Some(rows),
            tree_batches: Some(Vec::new()),
        }
    }

    #[test]
    fn retained_629_object_owner_partition_projects_three_equal_passes() {
        let mut rows = Vec::new();
        for i in 0..413 {
            rows.push(object(&format!("M01:{i}"), L03TypedOwner::M01Unit, 7));
        }
        for i in 0..149 {
            rows.push(object(&format!("O04:{i}"), L03TypedOwner::O04City, 7));
        }
        for i in 0..50 {
            rows.push(object(
                &format!("O05:{i}"),
                L03TypedOwner::O05StaticFixed,
                7,
            ));
        }
        for i in 0..17 {
            rows.push(object(
                &format!("M04:{i}"),
                L03TypedOwner::M04RailwayBridge,
                7,
            ));
        }
        let mut req = request(rows);
        req.enabled_partition_mask = Some(7);
        let projected = project_l03_light_depth(&req).expect("exact 629-object projection");
        assert_eq!(projected.submissions.len(), 1887);
        let ids: Vec<_> = projected
            .submissions
            .iter()
            .map(|row| match row {
                L03LightDepthSubmission::Pdxmesh {
                    stable_typed_upstream_id,
                    ..
                } => stable_typed_upstream_id.as_str(),
                _ => panic!("no trees in this oracle"),
            })
            .collect();
        assert_eq!(&ids[0..629], &ids[629..1258]);
        assert_eq!(&ids[0..629], &ids[1258..1887]);
    }

    #[test]
    fn semantic_group_and_submesh_order_is_not_sorted() {
        let mut row = match object("O05:fixed", L03TypedOwner::O05StaticFixed, 1) {
            L03SemanticInputRow::PdxmeshObject(row) => row,
            _ => unreachable!(),
        };
        row.groups = vec![
            L03PdxMeshGroupInput {
                submeshes: vec![
                    L03PdxMeshSubmeshInput {
                        selected_submesh_material: "z-last-name".into(),
                        material_family: L03MaterialFamily::Standard,
                        draw: draw(),
                    },
                    L03PdxMeshSubmeshInput {
                        selected_submesh_material: "a-first-name".into(),
                        material_family: L03MaterialFamily::Standard,
                        draw: draw(),
                    },
                ],
            },
            L03PdxMeshGroupInput {
                submeshes: vec![L03PdxMeshSubmeshInput {
                    selected_submesh_material: "middle".into(),
                    material_family: L03MaterialFamily::Standard,
                    draw: draw(),
                }],
            },
        ];
        let projected =
            project_l03_light_depth(&request(vec![L03SemanticInputRow::PdxmeshObject(row)]))
                .unwrap();
        let materials: Vec<_> = projected
            .submissions
            .iter()
            .map(|row| match row {
                L03LightDepthSubmission::Pdxmesh {
                    selected_submesh_material,
                    ..
                } => selected_submesh_material.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(materials, ["z-last-name", "a-first-name", "middle"]);
    }

    #[test]
    fn pipeline_substitution_and_particle_exclusion_are_typed() {
        let mut skinned = match object("M01:1", L03TypedOwner::M01Unit, 1) {
            L03SemanticInputRow::PdxmeshObject(row) => row,
            _ => unreachable!(),
        };
        skinned.skinned = true;
        let mut fixed = match object("O05:fixed", L03TypedOwner::O05StaticFixed, 1) {
            L03SemanticInputRow::PdxmeshObject(row) => row,
            _ => unreachable!(),
        };
        fixed.groups[0].submeshes[0].material_family = L03MaterialFamily::FixedMapFurnitureBorder;
        let projected = project_l03_light_depth(&request(vec![
            L03SemanticInputRow::PdxmeshObject(skinned),
            L03SemanticInputRow::Particle {
                stable_typed_upstream_id: "particle:1".into(),
            },
            L03SemanticInputRow::PdxmeshObject(fixed),
        ]))
        .unwrap();
        assert_eq!(projected.excluded_particle_rows, 1);
        let pipelines: Vec<_> = projected
            .submissions
            .iter()
            .map(|row| match row {
                L03LightDepthSubmission::Pdxmesh { pipeline, .. } => *pipeline,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            pipelines,
            [
                L03ShadowPipeline::StandardSkinned,
                L03ShadowPipeline::FixedMapFurnitureBorder
            ]
        );
    }

    #[test]
    fn tree_batches_sort_by_byte_then_f32_and_remain_terminal() {
        let tree = |ordinal, byte, value| L03TreeBatchInput {
            source_row_ordinal: ordinal,
            selected_mesh_ordinal: 0,
            sort_byte: byte,
            sort_value_f32_bits: bits(value),
            partition_mask: 1,
            selected_lod_index: 2,
            selected_mesh: format!("tree-{ordinal}.mesh"),
            selected_material: "treeshadow".into(),
            draw: draw(),
        };
        let mut req = request(vec![object("O04:1", L03TypedOwner::O04City, 1)]);
        req.tree_batches = Some(vec![tree(1, 2, 0.1), tree(2, 1, 9.0), tree(3, 1, 2.0)]);
        let projected = project_l03_light_depth(&req).unwrap();
        let ordinals: Vec<_> = projected
            .submissions
            .iter()
            .filter_map(|row| match row {
                L03LightDepthSubmission::Tree {
                    source_row_ordinal, ..
                } => Some(*source_row_ordinal),
                _ => None,
            })
            .collect();
        assert_eq!(ordinals, [3, 2, 1]);
        assert!(matches!(
            projected.submissions.last(),
            Some(L03LightDepthSubmission::Tree { .. })
        ));
    }

    #[test]
    fn one_tree_source_row_may_project_each_exact_selected_lod_mesh() {
        let tree = |selected_mesh_ordinal| L03TreeBatchInput {
            source_row_ordinal: 17,
            selected_mesh_ordinal,
            sort_byte: 1,
            sort_value_f32_bits: bits(2.0),
            partition_mask: 1,
            selected_lod_index: 0,
            selected_mesh: "multi-submesh-lod0".into(),
            selected_material: format!("tree-{selected_mesh_ordinal}"),
            draw: draw(),
        };
        let mut req = request(Vec::new());
        req.tree_batches = Some(vec![tree(0), tree(1)]);
        let projected = project_l03_light_depth(&req).unwrap();
        let keys = projected
            .submissions
            .iter()
            .map(|row| match row {
                L03LightDepthSubmission::Tree {
                    source_row_ordinal,
                    selected_mesh_ordinal,
                    ..
                } => (*source_row_ordinal, *selected_mesh_ordinal),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, [(17, 0), (17, 1)]);

        req.tree_batches = Some(vec![tree(0), tree(0)]);
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("duplicate tree source row 17 mesh 0"));
    }

    #[test]
    fn inactive_pass_emits_nothing_without_other_inputs() {
        let projected = project_l03_light_depth(&L03ProjectionRequest {
            shadow_pass_active: Some(false),
            selected_light_frame: None,
            enabled_partition_mask: None,
            semantic_rows: None,
            tree_batches: None,
        })
        .unwrap();
        assert!(projected.inactive_excluded);
        assert!(projected.submissions.is_empty());
    }

    #[test]
    fn active_frame_fails_closed_on_every_missing_exact_boundary() {
        let mut req = request(vec![]);
        req.selected_light_frame = None;
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("selected-light frame is missing"));

        let mut req = request(vec![]);
        req.semantic_rows = None;
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("semantic rows are missing"));

        let mut req = request(vec![]);
        req.tree_batches = None;
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("tree producer rows are missing"));
    }

    #[test]
    fn selected_light_target_and_matrix_fail_closed() {
        let mut req = request(vec![]);
        req.selected_light_frame
            .as_mut()
            .unwrap()
            .light_depth_target
            .dxgi_format = 40;
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("format 41"));

        let mut req = request(vec![]);
        req.selected_light_frame
            .as_mut()
            .unwrap()
            .light_view_projection_f32_bits[7] = f32::NAN.to_bits();
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("not finite"));

        let mut req = request(vec![]);
        req.selected_light_frame
            .as_mut()
            .unwrap()
            .shadow_map_texture_matrix_f32_bits[0] = bits(9.0);
        assert!(project_l03_light_depth(&req)
            .unwrap_err()
            .to_string()
            .contains("exact L02 light-view-projection composition"));
    }

    #[test]
    fn wrap_partition_pdxmesh_remains_before_all_terminal_trees() {
        let mut req = request(vec![object("M01:1", L03TypedOwner::M01Unit, 2)]);
        req.enabled_partition_mask = Some(3);
        req.tree_batches = Some(vec![L03TreeBatchInput {
            source_row_ordinal: 9,
            selected_mesh_ordinal: 0,
            sort_byte: 0,
            sort_value_f32_bits: bits(1.0),
            partition_mask: 1,
            selected_lod_index: 0,
            selected_mesh: "tree.mesh".into(),
            selected_material: "treeshadow".into(),
            draw: draw(),
        }]);
        let projected = project_l03_light_depth(&req).unwrap();
        assert_eq!(projected.submissions.len(), 2);
        assert!(matches!(
            projected.submissions[0],
            L03LightDepthSubmission::Pdxmesh {
                partition_mask: 2,
                ..
            }
        ));
        assert!(matches!(
            projected.submissions[1],
            L03LightDepthSubmission::Tree {
                partition_mask: 1,
                ..
            }
        ));
    }
}
