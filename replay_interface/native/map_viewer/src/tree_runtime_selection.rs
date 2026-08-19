
use crate::*;
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    AssignedStage, AssignedStageEdge, AssignedStageIdentity, MapObjectDrawArguments,
    MapObjectMaterialFamily, MapObjectMeshGroupContent, MapObjectRenderContent,
    MapObjectSubmeshContent, PreparedMapObjectDraw,
};
use replay_core::ValidatedContentAdmission;
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const XMAP11_TREE_FRAME_SCHEMA: &str = "hoi4.xmap11.tree-frame.a729.v1";
pub const XMAP11_TREE_TARGET_BOUND_DRAW_SCHEMA: &str = "hoi4.xmap11.tree-target-bound-draw.v1";
pub const XMAP11_TREE_MASK_TEXTURE_SCHEMA: &str = "hoi4.xmap11.tree-mask-texture.a729.v1";
pub const XMAP11_TREE_STAGE_NAME: &str = "trees";
pub const XMAP11_TREE_OPERATION_NAME: &str = "trees.prepare-and-draw";
pub const XMAP11_TREE_MAIN_EFFECT: &str = "tree";
pub const XMAP11_TREE_LOW_GFX_EFFECT: &str = "tree_low_gfx";
pub const XMAP11_TREE_UNLIT_EFFECT: &str = "treeunlit";
pub const XMAP11_TREE_SHADOW_EFFECT: &str = "treeshadow";
pub const XMAP11_TREE_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const XMAP11_TREE_CULL_MAX_Y_BITS: u32 = 25.5f32.to_bits();
pub const XMAP11_TREE_DISTANCE_Y_BITS: u32 = 9.5f32.to_bits();
pub const XMAP11_TREE_MASK_DXGI_FORMAT: u32 = 65;

pub fn xmap11_assigned_stage_rows() -> [AssignedStageIdentity; 1] {
    [AssignedStageIdentity::new(
        XMAP11_TREE_STAGE_NAME,
        XMAP11_TREE_OPERATION_NAME,
    )]
}

pub fn xmap11_assigned_stage_edges() -> [AssignedStageEdge; 0] {
    []
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeFrustumPlane {
    pub x_f32_bits: u32,
    pub y_f32_bits: u32,
    pub z_f32_bits: u32,
    pub w_f32_bits: u32,
}

impl TreeFrustumPlane {
    fn values(self) -> Result<[f32; 4]> {
        let values = [
            f32::from_bits(self.x_f32_bits),
            f32::from_bits(self.y_f32_bits),
            f32::from_bits(self.z_f32_bits),
            f32::from_bits(self.w_f32_bits),
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err("XMAP11 tree frustum plane contains a non-finite lane".into());
        }
        Ok(values)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeViewerPartition {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub side_planes: [TreeFrustumPlane; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeViewerState {
    pub camera_x_f32_bits: u32,
    pub camera_y_f32_bits: u32,
    pub camera_z_f32_bits: u32,
    pub ordered_partitions: Vec<TreeViewerPartition>,
}

impl TreeViewerState {
    pub fn validate(&self) -> Result<()> {
        self.camera().map(|_| ())
    }

    fn camera(&self) -> Result<[f32; 3]> {
        let camera = [
            f32::from_bits(self.camera_x_f32_bits),
            f32::from_bits(self.camera_y_f32_bits),
            f32::from_bits(self.camera_z_f32_bits),
        ];
        if camera.iter().any(|value| !value.is_finite()) {
            return Err("XMAP11 tree viewer camera contains a non-finite lane".into());
        }
        if self.ordered_partitions.is_empty() {
            return Err("XMAP11 tree viewer has no render partitions".into());
        }
        let mut seen = 0u8;
        for partition in &self.ordered_partitions {
            let mask = partition.render_view_mask;
            if !matches!(mask, 1 | 2 | 4) || seen & mask != 0 {
                return Err("XMAP11 tree viewer partitions are duplicated or invalid".into());
            }
            seen |= mask;
            if !f32::from_bits(partition.map_copy_offset_f32_bits).is_finite() {
                return Err("XMAP11 tree map-copy offset is non-finite".into());
            }
            for plane in partition.side_planes {
                plane.values()?;
            }
        }
        Ok(camera)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeFrameInput {
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub viewer_state_sha256: String,
    pub map_width: u32,
    pub map_height: u32,
    pub low_end_gfx: Option<bool>,
    pub viewer: TreeViewerState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeFadeWindow {
    pub near_f32_bits: u32,
    pub far_f32_bits: u32,
}

impl TreeFadeWindow {
    pub fn value(self, camera_y: f32) -> Result<f32> {
        let near = f32::from_bits(self.near_f32_bits);
        let far = f32::from_bits(self.far_f32_bits);
        if !near.is_finite()
            || !far.is_finite()
            || !camera_y.is_finite()
            || near < 0.0
            || far <= near
        {
            return Err("XMAP11 tree fade window or camera height is invalid".into());
        }
        if camera_y > far {
            return Ok(0.0);
        }
        Ok(1.0 - (camera_y.max(near) - near) / (far - near))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeCompiledProgram {
    pub effect_name: String,
    pub vertex_entry: String,
    pub pixel_entry: String,
    pub ordered_defines: Vec<String>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub vertex_bytecode: Vec<u8>,
    pub pixel_bytecode: Vec<u8>,
}

impl TreeCompiledProgram {
    fn validate(&self) -> Result<()> {
        if self.effect_name.is_empty()
            || self.vertex_entry.is_empty()
            || self.pixel_entry.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
        {
            return Err("XMAP11 compiled tree program is incomplete".into());
        }
        for (value, label) in [
            (
                &self.vertex_translation_unit_sha256,
                "tree vertex translation unit",
            ),
            (
                &self.pixel_translation_unit_sha256,
                "tree pixel translation unit",
            ),
            (&self.vertex_bytecode_sha256, "tree vertex bytecode"),
            (&self.pixel_bytecode_sha256, "tree pixel bytecode"),
        ] {
            validate_sha256(value, label)?;
        }
        if self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
        {
            return Err("XMAP11 compiled tree bytecode identity drifted".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeShaderPrograms {
    pub shader_source: UsedContentKey,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub pixel_samplers: Vec<TreeSamplerStateDescriptor>,
    pub main_pipeline: TreePipelineStateDescriptor,
    pub main: TreeCompiledProgram,
    pub unlit: TreeCompiledProgram,
    pub shadow: TreeCompiledProgram,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeSamplerStateDescriptor {
    pub name: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreePipelineStateDescriptor {
    pub blend_enable: bool,
    pub alpha_test: bool,
    pub write_mask: String,
    pub source_blend: String,
    pub dest_blend: String,
    pub depth_enable: bool,
    pub depth_write: bool,
    pub depth_func: String,
    pub fill_mode: String,
    pub cull_mode: String,
    pub front_counter_clockwise: bool,
    pub depth_clip_enable: bool,
}

impl TreePipelineStateDescriptor {
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.blend_enable
            || self.alpha_test
            || self.write_mask != "RED|GREEN|BLUE"
            || self.source_blend != "SRC_ALPHA"
            || self.dest_blend != "INV_SRC_ALPHA"
            || !self.depth_enable
            || !self.depth_write
            || self.depth_func != "LESS_EQUAL"
            || self.fill_mode != "SOLID"
            || self.cull_mode != "BACK"
            || self.front_counter_clockwise
            || !self.depth_clip_enable
        {
            return Err(
                "XMAP11 effective tree pipeline differs from the retained exact-a729 state and has no proven translation"
                    .into(),
            );
        }
        Ok(())
    }
}

pub(crate) fn validate_tree_sampler_descriptors(
    samplers: &[TreeSamplerStateDescriptor],
) -> Result<()> {
    const REQUIRED: [(&str, u32); 15] = [
        ("DiffuseMap", 0),
        ("NormalMap", 1),
        ("TintMap", 2),
        ("SeasonMap", 3),
        ("ColorMap", 4),
        ("ColorMapSecond", 5),
        ("SnowMudData", 6),
        ("ShadowMap", 8),
        ("EnvironmentMap", 9),
        ("LightIndexMap", 10),
        ("LightDataMap", 11),
        ("GradientBorderChannel1", 12),
        ("GradientBorderChannel2", 13),
        ("TreeMaskTexture", 14),
        ("ProvinceSecondaryColorMap", 15),
    ];
    if samplers.len() != REQUIRED.len() {
        return Err("XMAP11 effective tree sampler census is incomplete".into());
    }
    for (sampler, (expected_name, expected_register)) in samplers.iter().zip(REQUIRED) {
        if sampler.name != expected_name
            || sampler.shader_register != expected_register
            || !matches!(
                sampler.resource_type.as_str(),
                "Texture2D" | "Shadow" | "Cube"
            )
            || !matches!(sampler.mag_filter.as_str(), "Linear" | "Point")
            || !matches!(sampler.min_filter.as_str(), "Linear" | "Point")
            || !matches!(sampler.mip_filter.as_str(), "Linear" | "Point")
            || !matches!(sampler.address_u.as_str(), "Wrap" | "Clamp")
            || !matches!(sampler.address_v.as_str(), "Wrap" | "Clamp")
        {
            return Err(format!(
                "XMAP11 effective tree sampler {expected_name} has an unsupported descriptor"
            )
            .into());
        }
        let expected_type = match expected_name {
            "ShadowMap" => "Shadow",
            "EnvironmentMap" => "Cube",
            _ => "Texture2D",
        };
        if sampler.resource_type != expected_type {
            return Err(format!(
                "XMAP11 effective tree sampler {expected_name} has resource type {}, expected {expected_type}",
                sampler.resource_type
            )
            .into());
        }
    }
    Ok(())
}

impl TreeShaderPrograms {
    fn validate(&self) -> Result<()> {
        self.shader_source
            .validate()
            .map_err(|error| format!("invalid XMAP11 tree shader source: {error}"))?;
        if self.ordered_shader_content.is_empty()
            || self
                .ordered_shader_content
                .last()
                .is_none_or(|content| content != &self.shader_source)
        {
            return Err(
                "XMAP11 tree shader translation-unit content is empty or root is not last".into(),
            );
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_shader_content {
            content
                .validate()
                .map_err(|error| format!("invalid XMAP11 shader include authority: {error}"))?;
            if content.effective_vfs_manifest_sha256
                != self.shader_source.effective_vfs_manifest_sha256
                || !paths.insert(content.normalized_game_relative_path.clone())
            {
                return Err("XMAP11 shader include authority drifts or repeats a path".into());
            }
        }
        self.main.validate()?;
        self.unlit.validate()?;
        self.shadow.validate()?;
        if !matches!(
            self.main.effect_name.as_str(),
            XMAP11_TREE_MAIN_EFFECT | XMAP11_TREE_LOW_GFX_EFFECT
        ) || self.unlit.effect_name != XMAP11_TREE_UNLIT_EFFECT
            || self.shadow.effect_name != XMAP11_TREE_SHADOW_EFFECT
        {
            return Err("XMAP11 compiled program roles do not match the exact tree effects".into());
        }
        validate_tree_sampler_descriptors(&self.pixel_samplers)?;
        self.main_pipeline.validate()?;
        validate_sha256(&self.deterministic_sha256, "tree program set")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP11 tree program-set digest mismatch".into());
        }
        Ok(())
    }
}

/// Exact half-resolution A8 tree-exclusion mask generated by
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeMaskTexture {
    pub schema: String,
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub pixels: Vec<u8>,
    pub payload_sha256: String,
}

impl TreeMaskTexture {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP11_TREE_MASK_TEXTURE_SCHEMA
            || self.source_width == 0
            || self.source_height == 0
            || self.source_width & 1 != 0
            || self.source_height & 1 != 0
            || self.width != self.source_width / 2
            || self.height != self.source_height / 2
            || self.dxgi_format != XMAP11_TREE_MASK_DXGI_FORMAT
        {
            return Err("XMAP11 tree-mask descriptor is invalid".into());
        }
        let expected = usize::try_from(self.width)?
            .checked_mul(usize::try_from(self.height)?)
            .ok_or("XMAP11 tree-mask extent overflowed")?;
        if self.pixels.len() != expected || self.payload_sha256 != Sha256::hex(&self.pixels) {
            return Err("XMAP11 tree-mask payload identity is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreePreparedAsset {
    pub asset_index: u32,
    pub palette_index: u8,
    pub selected_lod_index: u32,
    pub selected_shape: String,
    pub selected_mesh_ordinal: u32,
    pub mesh_source: UsedContentKey,
    pub mesh: TreeMeshVertexBuffer,
    pub ordered_material_textures: Vec<UsedContentKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTreeBatch {
    pub source_row_ordinal: u32,
    pub sort_byte: u8,
    pub sort_value_f32_bits: u32,
    pub partition_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub selected_lod_index: u32,
    pub asset_index: u32,
    pub instance_count: u32,
    pub instance_bytes: Vec<u8>,
    pub object_draw: PreparedMapObjectDraw,
}

impl PreparedTreeBatch {
    pub(crate) fn l03_shadow_input(
        &self,
        asset: &TreePreparedAsset,
    ) -> Result<L03TreeBatchInput> {
        if self.asset_index != asset.asset_index
            || self.selected_lod_index != asset.selected_lod_index
            || self.partition_mask.count_ones() != 1
            || self.partition_mask & !L03_PARTITION_MASK_ALL != 0
            || !f32::from_bits(self.sort_value_f32_bits).is_finite()
        {
            return Err(
                "XMAP11 tree caster row has invalid asset, partition, or comparator identity"
                    .into(),
            );
        }
        let render = &self.object_draw.render;
        let submesh = match render.groups.as_slice() {
            [group] => match group.submeshes.as_slice() {
                [submesh] => submesh,
                _ => return Err("XMAP11 tree caster requires exactly one selected submesh".into()),
            },
            _ => return Err("XMAP11 tree caster requires exactly one selected mesh group".into()),
        };
        if self.object_draw.partition_mask != self.partition_mask
            || render.selected_mesh != asset.selected_shape
            || render.mesh_source != asset.mesh_source
            || render.skinned
            || submesh.material_family != MapObjectMaterialFamily::Tree
            || submesh.selected_material != asset.mesh.material_shader
            || submesh.material_source != asset.mesh_source
        {
            return Err("XMAP11 tree caster row drifts from its prepared MainScene object".into());
        }
        let draw = match submesh.draw {
            MapObjectDrawArguments::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                start_index,
                base_vertex,
                start_instance,
            } if index_count_per_instance == u32::try_from(asset.mesh.index_count)?
                && instance_count == self.instance_count =>
            {
                L03DrawArguments::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index,
                    base_vertex,
                    start_instance,
                }
            }
            _ => {
                return Err(
                    "XMAP11 tree caster draw drifts from its exact instanced MainScene draw".into(),
                )
            }
        };
        Ok(L03TreeBatchInput {
            source_row_ordinal: self.source_row_ordinal,
            selected_mesh_ordinal: asset.selected_mesh_ordinal,
            sort_byte: self.sort_byte,
            sort_value_f32_bits: self.sort_value_f32_bits,
            partition_mask: self.partition_mask,
            selected_lod_index: self.selected_lod_index,
            selected_mesh: render.selected_mesh.clone(),
            selected_material: submesh.selected_material.clone(),
            draw,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PreparedTreeStaticPayload {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub map_width: u32,
    pub map_height: u32,
    pub viewer: TreeViewerState,
    pub tree_viewer_state_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub tree_fade: TreeFadeWindow,
    pub tree_fade_f32_bits: u32,
    pub season_lerp_f32_bits: u32,
    pub season_column_f32_bits: u32,
    pub shader_programs: TreeShaderPrograms,
    pub tree_mask: TreeMaskTexture,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub assets: Vec<TreePreparedAsset>,
    pub selected_batches: Vec<PreparedTreeBatch>,
    pub ordered_main_draw_batch_indices: Vec<u32>,
    pub static_payload_sha256: String,
    pub native_resource_closure_sha256: String,
    pub foundation_resource_census_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTreeFrame {
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub viewer_state_sha256: String,
    static_payload: Arc<PreparedTreeStaticPayload>,
    pub prepared_frame_sha256: String,
}

impl std::ops::Deref for PreparedTreeFrame {
    type Target = PreparedTreeStaticPayload;

    fn deref(&self) -> &Self::Target {
        &self.static_payload
    }
}

#[cfg(test)]
thread_local! {
    static XMAP11_IMMUTABLE_SEAL_COMPUTATIONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

impl PreparedTreeStaticPayload {
    fn computed_native_resource_closure_sha256(&self) -> Result<String> {
        #[cfg(test)]
        XMAP11_IMMUTABLE_SEAL_COMPUTATIONS
            .with(|count| count.set(count.get() + 1));
        let used_asset_indices = self
            .ordered_main_draw_batch_indices
            .iter()
            .map(|index| self.selected_batches[*index as usize].asset_index)
            .collect::<BTreeSet<_>>();
        let used_assets = self
            .assets
            .iter()
            .filter(|asset| used_asset_indices.contains(&asset.asset_index))
            .collect::<Vec<_>>();
        let retained_instances = self
            .ordered_main_draw_batch_indices
            .iter()
            .map(|index| {
                let batch = &self.selected_batches[*index as usize];
                let asset = &self.assets[batch.asset_index as usize];
                (
                    batch.source_row_ordinal,
                    asset.selected_mesh_ordinal,
                    batch.partition_mask,
                    batch.asset_index,
                    batch.instance_count,
                    Sha256::hex(&batch.instance_bytes),
                )
            })
            .collect::<Vec<_>>();
        Ok(Sha256::hex(&serde_json::to_vec(&(
            self.schema.as_str(),
            self.content_manifest_sha256.as_str(),
            self.effective_vfs_manifest_sha256.as_str(),
            &self.shader_programs,
            &self.tree_mask,
            &self.ordered_used_content,
            used_assets,
            retained_instances,
        ))?))
    }

    fn computed_static_payload_sha256(&self) -> Result<String> {
        #[cfg(test)]
        XMAP11_IMMUTABLE_SEAL_COMPUTATIONS
            .with(|count| count.set(count.get() + 1));
        Ok(Sha256::hex(&serde_json::to_vec(&(
            (
                self.schema.as_str(),
                self.content_manifest_sha256.as_str(),
                self.effective_vfs_manifest_sha256.as_str(),
                self.map_width,
                self.map_height,
            ),
            (
                &self.viewer,
                self.tree_viewer_state_sha256.as_str(),
                self.map_copy_receipt_set_sha256.as_str(),
                self.tree_fade,
                self.tree_fade_f32_bits,
                self.season_lerp_f32_bits,
                self.season_column_f32_bits,
            ),
            (
                &self.shader_programs,
                &self.tree_mask,
                &self.ordered_used_content,
                &self.assets,
                &self.selected_batches,
                &self.ordered_main_draw_batch_indices,
                self.native_resource_closure_sha256.as_str(),
                self.foundation_resource_census_sha256.as_str(),
            ),
        ))?))
    }
}

impl PreparedTreeFrame {
    pub fn native_resource_closure_sha256(&self) -> &str {
        &self.static_payload.native_resource_closure_sha256
    }

    fn computed_prepared_frame_sha256(&self) -> Result<String> {
        Ok(Sha256::hex(&serde_json::to_vec(&(
            XMAP11_TREE_FRAME_SCHEMA,
            self.hour_index,
            self.replay_frame_identity_sha256.as_str(),
            self.camera_frame_sha256.as_str(),
            self.viewer_state_sha256.as_str(),
            self.static_payload.static_payload_sha256.as_str(),
        ))?))
    }

    pub(crate) fn reseal_dynamic_authority(&mut self) -> Result<()> {
        self.prepared_frame_sha256 = self.computed_prepared_frame_sha256()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        validate_sha256(&self.replay_frame_identity_sha256, "replay frame")?;
        validate_sha256(&self.camera_frame_sha256, "camera frame")?;
        validate_sha256(&self.viewer_state_sha256, "viewer state")?;
        validate_sha256(&self.static_payload.static_payload_sha256, "static tree payload")?;
        validate_sha256(
            &self.static_payload.native_resource_closure_sha256,
            "native tree resource closure",
        )?;
        validate_sha256(
            &self.static_payload.foundation_resource_census_sha256,
            "tree foundation resource census",
        )?;
        validate_sha256(&self.prepared_frame_sha256, "prepared tree frame")?;
        if self.prepared_frame_sha256 != self.computed_prepared_frame_sha256()? {
            return Err("XMAP11 prepared tree frame digest mismatch".into());
        }
        Ok(())
    }

    fn validate_complete(&self) -> Result<()> {
        self.validate()?;
        if self.schema != XMAP11_TREE_FRAME_SCHEMA {
            return Err("XMAP11 tree frame schema mismatch".into());
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "effective-VFS manifest",
        )?;
        if self.static_payload.native_resource_closure_sha256
            != self.static_payload.computed_native_resource_closure_sha256()?
            || self.static_payload.static_payload_sha256
                != self.static_payload.computed_static_payload_sha256()?
            || self.static_payload.foundation_resource_census_sha256
                != Sha256::hex(&serde_json::to_vec(&(
                    &self.ordered_used_content,
                    &self.assets,
                    &self.selected_batches,
                ))?)
        {
            return Err("XMAP11 immutable tree payload seal mismatch".into());
        }
        if self.map_width == 0 || self.map_height == 0 || self.ordered_used_content.is_empty() {
            return Err("XMAP11 tree frame has no effective-VFS content authority".into());
        }
        self.viewer.validate()?;
        validate_sha256(&self.tree_viewer_state_sha256, "tree viewer state")?;
        validate_sha256(
            &self.map_copy_receipt_set_sha256,
            "tree map-copy receipt set",
        )?;
        if self.tree_viewer_state_sha256 != Sha256::hex(&serde_json::to_vec(&self.viewer)?)
            || self.map_copy_receipt_set_sha256
                != xmap04_map_copy_receipt_set_sha256(
                    self.map_width,
                    &self
                        .viewer
                        .ordered_partitions
                        .iter()
                        .map(|partition| Xmap04MapCopyReceiptRow {
                            render_view_mask: partition.render_view_mask,
                            map_copy_offset_f32_bits: partition.map_copy_offset_f32_bits,
                        })
                        .collect::<Vec<_>>(),
                )?
        {
            return Err(
                "XMAP11 viewer or map-copy identity is not derived from exact partition state"
                    .into(),
            );
        }
        self.tree_mask.validate()?;
        self.shader_programs.validate()?;
        self.tree_fade
            .value(f32::from_bits(self.tree_fade.near_f32_bits))?;
        if self.season_lerp_f32_bits != 0.0f32.to_bits()
            || self.season_column_f32_bits != 0.0f32.to_bits()
        {
            return Err("XMAP11 tree fade or exact literal-zero season law is invalid".into());
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("invalid XMAP11 UsedContentKey: {error}"))?;
            if content.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256
                || !paths.insert(content.normalized_game_relative_path.clone())
            {
                return Err("XMAP11 tree content authority drifts or repeats a path".into());
            }
        }
        for required in self
            .shader_programs
            .ordered_shader_content
            .iter()
            .chain(std::iter::once(&self.shader_programs.shader_source))
        {
            if !self.ordered_used_content.contains(required) {
                return Err("XMAP11 shader authority is absent from ordered used content".into());
            }
        }
        for (index, asset) in self.assets.iter().enumerate() {
            validate_tree_asset(
                asset,
                u32::try_from(index)?,
                &self.effective_vfs_manifest_sha256,
                &self.ordered_used_content,
            )?;
        }
        let mut source_submeshes = BTreeSet::new();
        let mut stable_ids = BTreeSet::new();
        let mut previous_sort_key: Option<(u8, u32, u32, u32)> = None;
        for (visible_order, batch) in self.selected_batches.iter().enumerate() {
            batch
                .object_draw
                .validate(visible_order as u32, &self.effective_vfs_manifest_sha256)
                .map_err(|error| format!("invalid XMAP11 map-object draw: {error}"))?;
            let asset = self
                .assets
                .get(batch.asset_index as usize)
                .ok_or("XMAP11 tree batch references a missing asset")?;
            if batch.instance_count == 0
                || batch.instance_bytes.len()
                    != usize::try_from(batch.instance_count)?
                        .checked_mul(TREE_INSTANCE_STRIDE_BYTES as usize)
                        .ok_or("XMAP11 tree instance extent overflowed")?
                || batch.selected_lod_index != asset.selected_lod_index
                || batch.sort_byte != asset.palette_index - 1
                || self
                    .viewer
                    .ordered_partitions
                    .iter()
                    .find(|partition| partition.render_view_mask == batch.partition_mask)
                    .is_none_or(|partition| {
                        partition.map_copy_offset_f32_bits != batch.map_copy_offset_f32_bits
                    })
                || !source_submeshes.insert((batch.source_row_ordinal, asset.selected_mesh_ordinal))
                || !stable_ids.insert(batch.object_draw.stable_typed_id.clone())
            {
                return Err("XMAP11 tree batch instance, LOD, row, or identity is invalid".into());
            }
            let distance = f32::from_bits(batch.sort_value_f32_bits);
            if !distance.is_finite() || distance < 0.0 {
                return Err("XMAP11 tree batch comparator distance is invalid".into());
            }
            if let Some((previous_byte, previous_distance_bits, previous_row, previous_mesh)) =
                previous_sort_key
            {
                let previous_distance = f32::from_bits(previous_distance_bits);
                if (
                    batch.sort_byte,
                    distance,
                    batch.source_row_ordinal,
                    asset.selected_mesh_ordinal,
                ) < (
                    previous_byte,
                    previous_distance,
                    previous_row,
                    previous_mesh,
                ) {
                    return Err(
                        "XMAP11 selected tree batches are not in exact comparator order".into(),
                    );
                }
            }
            previous_sort_key = Some((
                batch.sort_byte,
                batch.sort_value_f32_bits,
                batch.source_row_ordinal,
                asset.selected_mesh_ordinal,
            ));
            batch.l03_shadow_input(asset)?;
        }
        for (ordinal, batch_index) in self
            .ordered_main_draw_batch_indices
            .iter()
            .copied()
            .enumerate()
        {
            if batch_index as usize >= self.selected_batches.len()
                || (ordinal > 0 && self.ordered_main_draw_batch_indices[ordinal - 1] >= batch_index)
            {
                return Err("XMAP11 main tree draw indices are not ordered and unique".into());
            }
        }
        validate_main_tree_visibility_gate(
            &self.viewer,
            self.tree_fade,
            self.tree_fade_f32_bits,
            self.selected_batches.len(),
            &self.ordered_main_draw_batch_indices,
        )?;
        Ok(())
    }

    pub fn l03_shadow_batches(&self) -> Result<Vec<L03TreeBatchInput>> {
        self.validate()?;
        l03_shadow_batches_for_main_draws(
            &self.assets,
            &self.selected_batches,
            &self.ordered_main_draw_batch_indices,
        )
    }
}

fn l03_shadow_batches_for_main_draws(
    assets: &[TreePreparedAsset],
    selected_batches: &[PreparedTreeBatch],
    ordered_main_draw_batch_indices: &[u32],
) -> Result<Vec<L03TreeBatchInput>> {
    ordered_main_draw_batch_indices
        .iter()
        .map(|batch_index| {
            let batch = selected_batches
                .get(*batch_index as usize)
                .ok_or("XMAP11 admitted MainScene tree batch disappeared")?;
            let asset = assets
                .get(batch.asset_index as usize)
                .ok_or("XMAP11 tree batch references a missing asset")?;
            batch.l03_shadow_input(asset)
        })
        .collect()
}

pub fn xmap11_l03_shadow_batches(frame: &PreparedTreeFrame) -> Result<Vec<L03TreeBatchInput>> {
    frame.l03_shadow_batches()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeActualDrawExecution {
    pub hour_index: i64,
    pub draw_ordinal: u32,
    pub operation_id: String,
    pub content_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub foundation_authorization_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub prepared_frame_sha256: String,
    pub source_row_ordinal: u32,
    pub render_view_mask: u8,
    pub executor_draw_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeTargetBoundDrawReceipt {
    pub schema: String,
    pub operation_id: String,
    pub stage_name: String,
    pub assigned_ordinal: u32,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub draw_ordinal: u32,
    pub content_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub foundation_authorization_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub prepared_frame_sha256: String,
    pub source_row_ordinal: u32,
    pub render_view_mask: u8,
    pub executor_draw_receipt_sha256: String,
    pub packet_draw_receipt_sha256: String,
}

impl TreeTargetBoundDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP11_TREE_TARGET_BOUND_DRAW_SCHEMA
            || self.operation_id != XMAP11_TREE_OPERATION_NAME
            || self.stage_name != XMAP11_TREE_STAGE_NAME
            || self.assigned_ordinal == 0
            || self.render_view_mask.count_ones() != 1
            || self.render_view_mask & !L03_PARTITION_MASK_ALL != 0
        {
            return Err("XMAP11 target-bound tree receipt shape is invalid".into());
        }
        for (value, label) in [
            (&self.topology_sha256, "schedule topology"),
            (&self.assignment_sha256, "schedule assignment"),
            (&self.content_manifest_sha256, "content manifest"),
            (&self.replay_frame_identity_sha256, "replay frame"),
            (&self.program_sha256, "tree program"),
            (&self.ordered_resources_sha256, "tree ordered resources"),
            (
                &self.foundation_content_operation_sha256,
                "foundation content operation",
            ),
            (
                &self.foundation_authorization_sha256,
                "foundation family authorization",
            ),
            (&self.viewer_state_sha256, "viewer state"),
            (&self.main_scene_target_sha256, "MainScene target"),
            (&self.map_copy_receipt_set_sha256, "map-copy receipt set"),
            (&self.prepared_frame_sha256, "prepared tree frame"),
            (
                &self.executor_draw_receipt_sha256,
                "executor-native draw receipt",
            ),
            (&self.packet_draw_receipt_sha256, "packet draw receipt"),
        ] {
            validate_sha256(value, label)?;
        }
        let mut unsigned = self.clone();
        unsigned.packet_draw_receipt_sha256.clear();
        if self.packet_draw_receipt_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP11 target-bound tree receipt digest mismatch".into());
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        frame: &PreparedTreeFrame,
        schedule: &AssignedStage,
    ) -> Result<()> {
        self.validate()?;
        frame.validate()?;
        schedule
            .validate()
            .map_err(|error| format!("XMAP11 assigned stage: {error}"))?;
        if schedule.stage_name != XMAP11_TREE_STAGE_NAME
            || schedule.operation_name != XMAP11_TREE_OPERATION_NAME
            || self.stage_name != schedule.stage_name
            || self.operation_id != schedule.operation_name
            || self.assigned_ordinal != schedule.assigned_ordinal
            || self.topology_sha256 != schedule.topology_sha256
            || self.assignment_sha256 != schedule.assignment_sha256
            || self.hour_index != frame.hour_index
            || self.content_manifest_sha256 != frame.content_manifest_sha256
            || self.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
            || self.program_sha256 != frame.shader_programs.deterministic_sha256
            || self.viewer_state_sha256 != frame.viewer_state_sha256
            || self.map_copy_receipt_set_sha256 != frame.map_copy_receipt_set_sha256
            || self.prepared_frame_sha256 != frame.prepared_frame_sha256
        {
            return Err(
                "XMAP11 target-bound receipt drifts from its frame or assigned stage".into(),
            );
        }
        let batch_index = *frame
            .ordered_main_draw_batch_indices
            .get(self.draw_ordinal as usize)
            .ok_or("XMAP11 target-bound receipt names an unissued MainScene draw")?;
        let batch = &frame.selected_batches[batch_index as usize];
        if self.source_row_ordinal != batch.source_row_ordinal
            || self.render_view_mask != batch.partition_mask
        {
            return Err("XMAP11 target-bound receipt drifts from its prepared tree batch".into());
        }
        let foundation = schedule
            .foundation_operation_identity(
                self.program_sha256.clone(),
                self.ordered_resources_sha256.clone(),
                self.main_scene_target_sha256.clone(),
            )
            .map_err(|error| format!("XMAP11 assigned foundation identity: {error}"))?;
        if self.foundation_content_operation_sha256
            != Sha256::hex(&serde_json::to_vec(&foundation)?)
        {
            return Err("XMAP11 target-bound receipt has a foreign foundation identity".into());
        }
        if self.foundation_authorization_sha256
            != crate::xmap11_foundation_authorization_sha256(frame, schedule)?
        {
            return Err("XMAP11 target-bound receipt has a foreign family authorization".into());
        }
        Ok(())
    }
}

pub fn prepare_xmap11_tree_frame(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    input: &TreeFrameInput,
) -> Result<PreparedTreeFrame> {
    if !resolver.has_content_admission() {
        return Err("XMAP11 tree preparation requires an admission-bound resolver".into());
    }
    if input.map_width == 0 || input.map_height == 0 {
        return Err("XMAP11 tree map dimensions are zero".into());
    }
    validate_sha256(&input.replay_frame_identity_sha256, "replay frame")?;
    validate_sha256(&input.camera_frame_sha256, "camera frame")?;
    validate_sha256(&input.viewer_state_sha256, "viewer state")?;
    let low_end_gfx = input
        .low_end_gfx
        .ok_or("XMAP11 tree LOW_END_GFX setting is missing")?;
    let camera = input.viewer.camera()?;

    let required_core = [
        TREES_BMP_SOURCE,
        TREE_HEIGHTMAP_SOURCE,
        RIVERS_BMP_SOURCE,
        TREE_DEFAULT_MAP_SOURCE,
        TREE_GFX_BINDING_SOURCE,
        TREE_TINT_TEXTURE_SOURCE,
        TREE_SEASON_TEXTURE_SOURCE,
        TREE_SHADER_SOURCE,
        XMAP11_TREE_GRAPHICS_DEFINES_SOURCE,
    ];
    let mut content_by_path = BTreeMap::new();
    for path in required_core {
        admit_content(resolver, admission, path, &mut content_by_path)?;
    }
    if !low_end_gfx {
        admit_content(
            resolver,
            admission,
            TREE_ENVIRONMENT_TEXTURE_SOURCE,
            &mut content_by_path,
        )?;
    }
    let effective_vfs_manifest_sha256 = content_by_path
        .values()
        .next()
        .ok_or("XMAP11 tree content admission is empty")?
        .effective_vfs_manifest_sha256
        .clone();
    let shader_translation_unit = effect_translation_unit_files(resolver, TREE_SHADER_SOURCE)?;
    for (path, _) in &shader_translation_unit {
        admit_content(resolver, admission, path, &mut content_by_path)?;
    }

    let fade = tree_fade_window_from_resolver(resolver)?;
    let fade_value = fade.value(camera[1])?;
    let synthesis = synthesize_tree_instances_from_resolver(
        resolver,
        input.map_width as usize,
        input.map_height as usize,
    )?;
    let rivers_path = resolver.resolve_required(RIVERS_BMP_SOURCE, "rivers.bmp")?;
    let rivers = load_bmp_indexed(&rivers_path)?;
    if rivers.width != input.map_width as usize || rivers.height != input.map_height as usize {
        return Err(format!(
            "XMAP11 rivers.bmp is {}x{}, expected {}x{}",
            rivers.width, rivers.height, input.map_width, input.map_height
        )
        .into());
    }
    let tree_mask = synthesize_tree_mask_texture(&rivers)?;
    let default_palette = default_map_tree_indices_from_resolver(resolver)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let bindings = tree_gfx_bindings_from_resolver(resolver)?;
    let mut binding_by_palette = BTreeMap::new();
    for binding in bindings {
        let index = binding.object_index;
        if binding_by_palette.insert(index, binding).is_some() {
            return Err(format!("XMAP11 trees.gfx repeats mapobject_{index}").into());
        }
    }

    let mut models = BTreeMap::new();
    for palette_index in synthesis.stats.tree_index_counts.keys().copied() {
        if !default_palette.contains(&palette_index) {
            return Err(format!(
                "XMAP11 trees.bmp uses palette {palette_index} absent from default.map tree list"
            )
            .into());
        }
        let binding = binding_by_palette.get(&palette_index).ok_or_else(|| {
            format!("XMAP11 trees.bmp palette {palette_index} has no trees.gfx binding")
        })?;
        let mesh_content = admit_content(resolver, admission, &binding.mesh, &mut content_by_path)?;
        let model = pdx_model_from_resolver(resolver, &binding.mesh)?;
        validate_tree_lod_model(&model, &binding.mesh)?;
        models.insert(palette_index, (binding.clone(), mesh_content, model));
    }

    struct Selected {
        source_row_ordinal: u32,
        sort_byte: u8,
        distance_squared: f32,
        partition_mask: u8,
        map_copy_offset_f32_bits: u32,
        batch: TreeInstanceBatch,
        selected_lod_index: usize,
        selected_mesh_index: usize,
    }
    let mut selected = Vec::new();
    for (source_row_ordinal, batch) in synthesis.batches.into_iter().enumerate() {
        let Some((partition_mask, distance_squared)) = select_tree_chunk_partition(
            input.map_width,
            input.map_height,
            batch.chunk_x,
            batch.chunk_z,
            &input.viewer,
        )?
        else {
            continue;
        };
        let map_copy_offset_f32_bits = input
            .viewer
            .ordered_partitions
            .iter()
            .find(|partition| partition.render_view_mask == partition_mask)
            .ok_or("XMAP11 selected tree partition disappeared")?
            .map_copy_offset_f32_bits;
        let (_, _, model) = models
            .get(&batch.palette_index)
            .ok_or("XMAP11 selected tree palette has no loaded model")?;
        let selected_lod_index = select_tree_lod_index(model, distance_squared)?;
        let shape = &model.shapes[selected_lod_index];
        for selected_mesh_index in drawable_tree_mesh_indices(shape, selected_lod_index)? {
            selected.push(Selected {
                source_row_ordinal: source_row_ordinal as u32,
                sort_byte: batch.palette_index - 1,
                distance_squared,
                partition_mask,
                map_copy_offset_f32_bits,
                batch: batch.clone(),
                selected_lod_index,
                selected_mesh_index,
            });
        }
    }
    selected.sort_by(|left, right| {
        left.sort_byte
            .cmp(&right.sort_byte)
            .then_with(|| left.distance_squared.total_cmp(&right.distance_squared))
            .then_with(|| left.source_row_ordinal.cmp(&right.source_row_ordinal))
            .then_with(|| left.selected_mesh_index.cmp(&right.selected_mesh_index))
    });

    let entity_source = content_by_path
        .get(&TREE_GFX_BINDING_SOURCE.to_ascii_lowercase())
        .ok_or("XMAP11 trees.gfx authority disappeared")?
        .clone();
    let tint_source = content_by_path
        .get(&TREE_TINT_TEXTURE_SOURCE.to_ascii_lowercase())
        .ok_or("XMAP11 tint authority disappeared")?
        .clone();
    let season_source = content_by_path
        .get(&TREE_SEASON_TEXTURE_SOURCE.to_ascii_lowercase())
        .ok_or("XMAP11 season authority disappeared")?
        .clone();
    let shader_source = content_by_path
        .get(&TREE_SHADER_SOURCE.to_ascii_lowercase())
        .ok_or("XMAP11 shader authority disappeared")?
        .clone();
    let ordered_shader_content = shader_translation_unit
        .iter()
        .map(|(path, _)| {
            content_by_path
                .get(&path.to_ascii_lowercase())
                .cloned()
                .ok_or_else(|| format!("XMAP11 shader include authority disappeared for {path}"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let shader_programs = prepare_tree_shader_programs(
        resolver,
        &shader_translation_unit,
        shader_source,
        ordered_shader_content,
        input.map_width,
        input.map_height,
        low_end_gfx,
    )?;

    let mut asset_lookup = BTreeMap::new();
    let mut assets = Vec::new();
    let mut prepared_batches = Vec::with_capacity(selected.len());
    for (visible_order, row) in selected.into_iter().enumerate() {
        let (binding, mesh_source, model) = models
            .get(&row.batch.palette_index)
            .ok_or("XMAP11 selected tree model disappeared")?;
        let asset_key = (
            row.batch.palette_index,
            row.selected_lod_index,
            row.selected_mesh_index,
        );
        let asset_index = if let Some(index) = asset_lookup.get(&asset_key).copied() {
            index
        } else {
            let mesh = tree_mesh_vertex_buffer_for_shape(
                model,
                binding.clone(),
                row.selected_lod_index,
                row.selected_mesh_index,
            )?;
            let material_refs = [
                mesh.material_diffuse.as_deref(),
                mesh.material_normal_map.as_deref(),
                mesh.material_specular.as_deref(),
            ];
            if material_refs[0].is_none() || material_refs[1].is_none() {
                return Err(format!(
                    "XMAP11 tree mesh {} LOD {} lacks required diffuse/normal material",
                    binding.mesh, row.selected_lod_index
                )
                .into());
            }
            let mut ordered_material_textures = Vec::new();
            for reference in material_refs.into_iter().flatten() {
                let path = admitted_material_texture_path(admission, &binding.mesh, reference)?;
                ordered_material_textures.push(admit_content(
                    resolver,
                    admission,
                    &path,
                    &mut content_by_path,
                )?);
            }
            let index = assets.len() as u32;
            assets.push(TreePreparedAsset {
                asset_index: index,
                palette_index: row.batch.palette_index,
                selected_lod_index: row.selected_lod_index as u32,
                selected_shape: model.shapes[row.selected_lod_index].name.clone(),
                selected_mesh_ordinal: row.selected_mesh_index as u32,
                mesh_source: mesh_source.clone(),
                mesh,
                ordered_material_textures,
            });
            asset_lookup.insert(asset_key, index);
            index
        };
        let asset = &assets[asset_index as usize];
        let instance_count = u32::try_from(row.batch.instance_count)?;
        let draw = MapObjectDrawArguments::DrawIndexedInstanced {
            index_count_per_instance: u32::try_from(asset.mesh.index_count)?,
            instance_count,
            start_index: 0,
            base_vertex: 0,
            start_instance: 0,
        };
        let mut ordered_textures = asset.ordered_material_textures.clone();
        ordered_textures.push(tint_source.clone());
        ordered_textures.push(season_source.clone());
        let render = MapObjectRenderContent {
            entity_name: binding.name.clone(),
            entity_source: entity_source.clone(),
            selected_mesh: asset.selected_shape.clone(),
            mesh_source: mesh_source.clone(),
            groups: vec![MapObjectMeshGroupContent {
                submeshes: vec![MapObjectSubmeshContent {
                    selected_material: asset.mesh.material_shader.clone(),
                    material_source: mesh_source.clone(),
                    material_family: MapObjectMaterialFamily::Tree,
                    ordered_texture_sources: ordered_textures,
                    draw,
                }],
            }],
            world_matrix_f32_bits: identity_matrix_bits(),
            skinned: false,
        };
        prepared_batches.push(PreparedTreeBatch {
            source_row_ordinal: row.source_row_ordinal,
            sort_byte: row.sort_byte,
            sort_value_f32_bits: row.distance_squared.to_bits(),
            partition_mask: row.partition_mask,
            map_copy_offset_f32_bits: row.map_copy_offset_f32_bits,
            selected_lod_index: row.selected_lod_index as u32,
            asset_index,
            instance_count,
            instance_bytes: row.batch.instances,
            object_draw: PreparedMapObjectDraw {
                visible_order: visible_order as u32,
                stable_typed_id: format!(
                    "tree:bucket:{}:mesh:{}",
                    row.batch.bucket_id, row.selected_mesh_index
                ),
                partition_mask: row.partition_mask,
                render,
            },
        });
    }

    let ordered_main_draw_batch_indices =
        main_tree_draw_batch_indices(camera[1], fade.far_f32_bits, prepared_batches.len())?;
    let mut ordered_used_content = content_by_path.into_values().collect::<Vec<_>>();
    ordered_used_content.sort_by(|left, right| {
        left.normalized_game_relative_path
            .cmp(&right.normalized_game_relative_path)
    });
    let mut static_payload = PreparedTreeStaticPayload {
        schema: XMAP11_TREE_FRAME_SCHEMA.into(),
        content_manifest_sha256: admission.manifest_digest_sha256().into(),
        effective_vfs_manifest_sha256,
        map_width: input.map_width,
        map_height: input.map_height,
        viewer: input.viewer.clone(),
        tree_viewer_state_sha256: Sha256::hex(&serde_json::to_vec(&input.viewer)?),
        map_copy_receipt_set_sha256: xmap04_map_copy_receipt_set_sha256(
            input.map_width,
            &input
                .viewer
                .ordered_partitions
                .iter()
                .map(|partition| Xmap04MapCopyReceiptRow {
                    render_view_mask: partition.render_view_mask,
                    map_copy_offset_f32_bits: partition.map_copy_offset_f32_bits,
                })
                .collect::<Vec<_>>(),
        )?,
        tree_fade: fade,
        tree_fade_f32_bits: fade_value.to_bits(),
        season_lerp_f32_bits: 0.0f32.to_bits(),
        season_column_f32_bits: 0.0f32.to_bits(),
        shader_programs,
        tree_mask,
        ordered_used_content,
        assets,
        selected_batches: prepared_batches,
        ordered_main_draw_batch_indices,
        static_payload_sha256: String::new(),
        native_resource_closure_sha256: String::new(),
        foundation_resource_census_sha256: String::new(),
    };
    static_payload.native_resource_closure_sha256 =
        static_payload.computed_native_resource_closure_sha256()?;
    static_payload.foundation_resource_census_sha256 = Sha256::hex(&serde_json::to_vec(&(
        &static_payload.ordered_used_content,
        &static_payload.assets,
        &static_payload.selected_batches,
    ))?);
    static_payload.static_payload_sha256 = static_payload.computed_static_payload_sha256()?;
    let mut frame = PreparedTreeFrame {
        hour_index: input.hour_index,
        replay_frame_identity_sha256: input.replay_frame_identity_sha256.clone(),
        camera_frame_sha256: input.camera_frame_sha256.clone(),
        viewer_state_sha256: input.viewer_state_sha256.clone(),
        static_payload: Arc::new(static_payload),
        prepared_frame_sha256: String::new(),
    };
    frame.prepared_frame_sha256 = frame.computed_prepared_frame_sha256()?;
    frame.validate_complete()?;
    Ok(frame)
}

fn main_tree_draw_batch_indices(
    camera_y: f32,
    fade_far_f32_bits: u32,
    selected_batch_count: usize,
) -> Result<Vec<u32>> {
    let fade_far = f32::from_bits(fade_far_f32_bits);
    if !camera_y.is_finite() || !fade_far.is_finite() {
        return Err("XMAP11 MainScene tree draw gate has non-finite input".into());
    }
    if camera_y > fade_far {
        Ok(Vec::new())
    } else {
        (0..selected_batch_count)
            .map(u32::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn validate_main_tree_visibility_gate(
    viewer: &TreeViewerState,
    fade: TreeFadeWindow,
    tree_fade_f32_bits: u32,
    selected_batch_count: usize,
    ordered_main_draw_batch_indices: &[u32],
) -> Result<()> {
    let camera_y = viewer.camera()?[1];
    let expected_fade = fade.value(camera_y)?;
    let expected_draws =
        main_tree_draw_batch_indices(camera_y, fade.far_f32_bits, selected_batch_count)?;
    if tree_fade_f32_bits != expected_fade.to_bits()
        || ordered_main_draw_batch_indices != expected_draws
    {
        return Err(
            "XMAP11 tree fade scalar or MainScene draw set is not derived from the exact viewer camera"
                .into(),
        );
    }
    Ok(())
}

pub fn tree_target_bound_draw_receipt(
    frame: &PreparedTreeFrame,
    schedule: &AssignedStage,
    execution: &TreeActualDrawExecution,
) -> Result<TreeTargetBoundDrawReceipt> {
    frame.validate()?;
    schedule
        .validate()
        .map_err(|error| format!("XMAP11 assigned stage: {error}"))?;
    if schedule.stage_name != XMAP11_TREE_STAGE_NAME
        || schedule.operation_name != XMAP11_TREE_OPERATION_NAME
    {
        return Err("XMAP11 tree schedule must be exactly trees/trees.prepare-and-draw".into());
    }
    if schedule.operation_name.trim().is_empty()
        || execution.operation_id != schedule.operation_name
    {
        return Err(
            "XMAP11 tree executor operation_id must equal the assigned operation_name".into(),
        );
    }
    validate_sha256(&schedule.topology_sha256, "schedule topology")?;
    validate_sha256(&schedule.assignment_sha256, "schedule assignment")?;
    for (value, label) in [
        (&execution.content_manifest_sha256, "content manifest"),
        (&execution.replay_frame_identity_sha256, "replay frame"),
        (&execution.program_sha256, "tree program"),
        (
            &execution.ordered_resources_sha256,
            "tree ordered resources",
        ),
        (
            &execution.foundation_content_operation_sha256,
            "foundation content operation",
        ),
        (
            &execution.foundation_authorization_sha256,
            "foundation family authorization",
        ),
        (&execution.viewer_state_sha256, "viewer state"),
        (&execution.main_scene_target_sha256, "MainScene target"),
        (
            &execution.map_copy_receipt_set_sha256,
            "map-copy receipt set",
        ),
        (&execution.prepared_frame_sha256, "prepared tree frame"),
        (
            &execution.executor_draw_receipt_sha256,
            "executor-native draw receipt",
        ),
    ] {
        validate_sha256(value, label)?;
    }
    if execution.content_manifest_sha256 != frame.content_manifest_sha256
        || execution.hour_index != frame.hour_index
        || execution.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
        || execution.program_sha256 != frame.shader_programs.deterministic_sha256
        || execution.viewer_state_sha256 != frame.viewer_state_sha256
        || execution.map_copy_receipt_set_sha256 != frame.map_copy_receipt_set_sha256
        || execution.prepared_frame_sha256 != frame.prepared_frame_sha256
    {
        return Err("XMAP11 executed draw drifts from its prepared/content authority".into());
    }
    let batch_index = *frame
        .ordered_main_draw_batch_indices
        .get(execution.draw_ordinal as usize)
        .ok_or("XMAP11 executor draw ordinal is not an admitted MainScene tree draw")?;
    let batch = &frame.selected_batches[batch_index as usize];
    if execution.source_row_ordinal != batch.source_row_ordinal
        || execution.render_view_mask != batch.partition_mask
    {
        return Err("XMAP11 executor draw identity does not match its prepared tree batch".into());
    }
    let foundation_identity = schedule
        .foundation_operation_identity(
            execution.program_sha256.clone(),
            execution.ordered_resources_sha256.clone(),
            execution.main_scene_target_sha256.clone(),
        )
        .map_err(|error| format!("XMAP11 assigned foundation identity: {error}"))?;
    if execution.foundation_content_operation_sha256
        != Sha256::hex(&serde_json::to_vec(&foundation_identity)?)
    {
        return Err(
            "XMAP11 executor foundation identity was not derived from its assigned stage".into(),
        );
    }
    if execution.foundation_authorization_sha256
        != crate::xmap11_foundation_authorization_sha256(frame, schedule)?
    {
        return Err("XMAP11 executor draw was not authorized by its exact family disposition".into());
    }

    let mut receipt = TreeTargetBoundDrawReceipt {
        schema: XMAP11_TREE_TARGET_BOUND_DRAW_SCHEMA.into(),
        operation_id: execution.operation_id.clone(),
        stage_name: schedule.stage_name.clone(),
        assigned_ordinal: schedule.assigned_ordinal,
        topology_sha256: schedule.topology_sha256.clone(),
        assignment_sha256: schedule.assignment_sha256.clone(),
        hour_index: execution.hour_index,
        draw_ordinal: execution.draw_ordinal,
        content_manifest_sha256: execution.content_manifest_sha256.clone(),
        replay_frame_identity_sha256: execution.replay_frame_identity_sha256.clone(),
        program_sha256: execution.program_sha256.clone(),
        ordered_resources_sha256: execution.ordered_resources_sha256.clone(),
        foundation_content_operation_sha256: execution.foundation_content_operation_sha256.clone(),
        foundation_authorization_sha256: execution.foundation_authorization_sha256.clone(),
        viewer_state_sha256: execution.viewer_state_sha256.clone(),
        main_scene_target_sha256: execution.main_scene_target_sha256.clone(),
        map_copy_receipt_set_sha256: execution.map_copy_receipt_set_sha256.clone(),
        prepared_frame_sha256: execution.prepared_frame_sha256.clone(),
        source_row_ordinal: execution.source_row_ordinal,
        render_view_mask: execution.render_view_mask,
        executor_draw_receipt_sha256: execution.executor_draw_receipt_sha256.clone(),
        packet_draw_receipt_sha256: String::new(),
    };
    receipt.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    receipt.validate_against(frame, schedule)?;
    Ok(receipt)
}

pub fn synthesize_tree_mask_texture(rivers: &IndexImage) -> Result<TreeMaskTexture> {
    if rivers.width == 0
        || rivers.height == 0
        || rivers.width & 1 != 0
        || rivers.height & 1 != 0
        || rivers.pixels.len()
            != rivers
                .width
                .checked_mul(rivers.height)
                .ok_or("XMAP11 rivers.bmp extent overflowed")?
    {
        return Err("XMAP11 tree-mask generation requires a nonempty even-sized rivers.bmp".into());
    }
    let width = rivers.width / 2;
    let height = rivers.height / 2;
    let source_len = rivers
        .width
        .checked_mul(rivers.height)
        .ok_or("XMAP11 tree-mask source extent overflowed")?;
    let output_len = width
        .checked_mul(height)
        .ok_or("XMAP11 tree-mask output extent overflowed")?;
    if output_len <= rivers.width + 1 {
        return Err(
            "XMAP11 rivers.bmp is too small for the exact a729 adjacent-output read".into(),
        );
    }
    let arena_len = source_len
        .checked_add(output_len)
        .ok_or("XMAP11 tree-mask arena extent overflowed")?;
    let mut arena = vec![0u8; arena_len];
    for (output, source) in arena[..source_len].iter_mut().zip(&rivers.pixels) {
        *output = if *source < 0xfe { 0xff } else { 0 };
    }
    for y in 0..rivers.height {
        let output_row = source_len + (y >> 1) * width;
        for x in 0..rivers.width {
            let source = y * rivers.width + x;
            let sum = u16::from(arena[source])
                + u16::from(arena[source + 1])
                + u16::from(arena[source + rivers.width])
                + u16::from(arena[source + rivers.width + 1]);
            arena[output_row + (x >> 1)] = (sum >> 2) as u8;
        }
    }
    let pixels = arena.split_off(source_len);
    let texture = TreeMaskTexture {
        schema: XMAP11_TREE_MASK_TEXTURE_SCHEMA.into(),
        source_width: u32::try_from(rivers.width)?,
        source_height: u32::try_from(rivers.height)?,
        width: u32::try_from(width)?,
        height: u32::try_from(height)?,
        dxgi_format: XMAP11_TREE_MASK_DXGI_FORMAT,
        payload_sha256: Sha256::hex(&pixels),
        pixels,
    };
    texture.validate()?;
    Ok(texture)
}

fn prepare_tree_shader_programs(
    resolver: &GameFileResolver,
    translation_unit: &[(String, EffectFile)],
    shader_source: UsedContentKey,
    ordered_shader_content: Vec<UsedContentKey>,
    map_width: u32,
    map_height: u32,
    low_end_gfx: bool,
) -> Result<TreeShaderPrograms> {
    let root = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP11 tree shader translation unit is empty")?;
    for name in [
        XMAP11_TREE_MAIN_EFFECT,
        XMAP11_TREE_LOW_GFX_EFFECT,
        XMAP11_TREE_UNLIT_EFFECT,
        XMAP11_TREE_SHADOW_EFFECT,
    ] {
        let effect = root
            .effect(name)
            .ok_or_else(|| format!("XMAP11 effective tree.shader has no Effect {name}"))?;
        if effect
            .vertex_shader
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
            || effect
                .pixel_shader
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
        {
            return Err(format!(
                "XMAP11 effective tree.shader Effect {name} lacks a vertex or pixel program"
            )
            .into());
        }
    }
    let constants = derive_a729_engine_map_shader_constants(map_width, map_height, false, 0)?;
    let defines = engine_map_shader_defines(constants);
    let main_effect = if low_end_gfx {
        XMAP11_TREE_LOW_GFX_EFFECT
    } else {
        XMAP11_TREE_MAIN_EFFECT
    };
    let mut programs = TreeShaderPrograms {
        shader_source,
        ordered_shader_content,
        pixel_samplers: tree_sampler_descriptors(root)?,
        main_pipeline: tree_pipeline_state(root, main_effect)?,
        main: compile_tree_effect(resolver, root, main_effect, &defines)?,
        unlit: compile_tree_effect(resolver, root, XMAP11_TREE_UNLIT_EFFECT, &defines)?,
        shadow: compile_tree_effect(resolver, root, XMAP11_TREE_SHADOW_EFFECT, &defines)?,
        deterministic_sha256: String::new(),
    };
    programs.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&programs)?);
    programs.validate()?;
    Ok(programs)
}

fn tree_sampler_descriptors(root: &EffectFile) -> Result<Vec<TreeSamplerStateDescriptor>> {
    root.pixel_stage
        .samplers
        .iter()
        .map(|sampler| {
            let required = |value: &Option<String>, field: &str| -> Result<String> {
                value
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
                    .ok_or_else(|| format!("XMAP11 sampler {} has no {field}", sampler.name).into())
            };
            Ok(TreeSamplerStateDescriptor {
                name: sampler.name.clone(),
                shader_register: sampler.index,
                resource_type: sampler
                    .sampler_type
                    .clone()
                    .unwrap_or_else(|| "Texture2D".into()),
                mag_filter: required(&sampler.mag_filter, "MagFilter")?,
                min_filter: required(&sampler.min_filter, "MinFilter")?,
                mip_filter: required(&sampler.mip_filter, "MipFilter")?,
                address_u: required(&sampler.address_u, "AddressU")?,
                address_v: required(&sampler.address_v, "AddressV")?,
            })
        })
        .collect()
}

fn tree_pipeline_state(
    root: &EffectFile,
    effect_name: &str,
) -> Result<TreePipelineStateDescriptor> {
    let effect = root
        .effect(effect_name)
        .ok_or_else(|| format!("XMAP11 effective tree effect {effect_name} disappeared"))?;
    if effect.depth_stencil_state.is_some()
        || effect.rasterizer_state.is_some()
        || !root.depth_stencil_states.is_empty()
        || !root.rasterizer_states.is_empty()
    {
        return Err(
            "XMAP11 effective tree.shader declares depth/rasterizer state whose exact engine translation is not proven"
                .into(),
        );
    }
    let blend_name = effect
        .blend_state
        .as_deref()
        .or_else(|| {
            root.blend_states
                .contains_key("BlendState")
                .then_some("BlendState")
        })
        .ok_or("XMAP11 effective tree.shader has no selected BlendState")?;
    let pairs = root.blend_states.get(blend_name).ok_or_else(|| {
        format!("XMAP11 effective tree.shader names missing blend state {blend_name}")
    })?;
    let mut descriptor = TreePipelineStateDescriptor {
        blend_enable: false,
        alpha_test: false,
        write_mask: String::new(),
        source_blend: String::new(),
        dest_blend: String::new(),
        depth_enable: true,
        depth_write: true,
        depth_func: "LESS_EQUAL".into(),
        fill_mode: "SOLID".into(),
        cull_mode: "BACK".into(),
        front_counter_clockwise: false,
        depth_clip_enable: true,
    };
    let parse_yes_no = |value: &str, field: &str| -> Result<bool> {
        match value {
            "yes" => Ok(true),
            "no" => Ok(false),
            _ => Err(format!("XMAP11 {field} has unsupported value {value:?}").into()),
        }
    };
    let mut seen = BTreeSet::new();
    for (key, value) in pairs {
        if !seen.insert(key.as_str()) {
            return Err(format!("XMAP11 effective blend state repeats {key}").into());
        }
        match key.as_str() {
            "BlendEnable" => descriptor.blend_enable = parse_yes_no(value, key)?,
            "AlphaTest" => descriptor.alpha_test = parse_yes_no(value, key)?,
            "WriteMask" => descriptor.write_mask = value.clone(),
            "SourceBlend" => descriptor.source_blend = value.clone(),
            "DestBlend" => descriptor.dest_blend = value.clone(),
            _ => {
                return Err(format!(
                    "XMAP11 effective blend state key {key:?} has no proven a729 translation"
                )
                .into())
            }
        }
    }
    descriptor.validate()?;
    Ok(descriptor)
}

fn compile_tree_effect(
    resolver: &GameFileResolver,
    root: &EffectFile,
    effect_name: &str,
    engine_defines: &[(String, String)],
) -> Result<TreeCompiledProgram> {
    let effect = root
        .effect(effect_name)
        .ok_or_else(|| format!("XMAP11 tree effect {effect_name} disappeared"))?;
    let vertex_entry = effect
        .vertex_shader
        .clone()
        .ok_or_else(|| format!("XMAP11 tree effect {effect_name} has no vertex entry"))?;
    let pixel_entry = effect
        .pixel_shader
        .clone()
        .ok_or_else(|| format!("XMAP11 tree effect {effect_name} has no pixel entry"))?;
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TREE_SHADER_SOURCE,
        effect_name,
        ShaderStage::Vertex,
        engine_defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TREE_SHADER_SOURCE,
        effect_name,
        ShaderStage::Pixel,
        engine_defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) = compile_tree_program_stages(&vertex_hlsl, &pixel_hlsl)?;
    Ok(TreeCompiledProgram {
        effect_name: effect_name.into(),
        vertex_entry,
        pixel_entry,
        ordered_defines: effect.defines.clone(),
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        vertex_bytecode,
        pixel_bytecode,
    })
}

#[cfg(windows)]
fn compile_tree_program_stages(vertex_hlsl: &str, pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        d3d_compile_stage(
            vertex_hlsl,
            ShaderStage::Vertex,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
        d3d_compile_stage(
            pixel_hlsl,
            ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
    ))
}

#[cfg(not(windows))]
fn compile_tree_program_stages(
    _vertex_hlsl: &str,
    _pixel_hlsl: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP11 executable tree-program handoff requires Windows D3DCompile".into())
}

pub fn tree_fade_window_from_resolver(resolver: &GameFileResolver) -> Result<TreeFadeWindow> {
    let path =
        resolver.resolve_required(XMAP11_TREE_GRAPHICS_DEFINES_SOURCE, "tree graphics defines")?;
    let source = read_latin1(&path)?;
    tree_fade_window_from_graphics_defines(&source)
}

pub fn tree_fade_window_from_graphics_defines(source: &str) -> Result<TreeFadeWindow> {
    let near = parse_lua_number_define(source, "TREE_FADE_NEAR")?;
    let far = parse_lua_number_define(source, "TREE_FADE_FAR")?;
    if !near.is_finite() || !far.is_finite() || near < 0.0 || far <= near {
        return Err("XMAP11 TREE_FADE_NEAR/FAR values are invalid".into());
    }
    Ok(TreeFadeWindow {
        near_f32_bits: near.to_bits(),
        far_f32_bits: far.to_bits(),
    })
}

pub fn select_tree_chunk_partition(
    map_width: u32,
    map_height: u32,
    chunk_x: u32,
    chunk_z: u32,
    viewer: &TreeViewerState,
) -> Result<Option<(u8, f32)>> {
    let camera = viewer.camera()?;
    let min_x = chunk_x as f32 * TREE_CHUNK_WORLD_UNITS;
    let min_z = chunk_z as f32 * TREE_CHUNK_WORLD_UNITS;
    if min_x >= map_width as f32 || min_z >= map_height as f32 {
        return Err("XMAP11 tree batch chunk lies outside the map".into());
    }
    let max_x = (min_x + TREE_CHUNK_WORLD_UNITS).min(map_width as f32);
    let max_z = (min_z + TREE_CHUNK_WORLD_UNITS).min(map_height as f32);
    let mut closest: Option<(u8, f32)> = None;
    for partition in &viewer.ordered_partitions {
        let offset = f32::from_bits(partition.map_copy_offset_f32_bits);
        let shifted_min_x = min_x + offset;
        let shifted_max_x = max_x + offset;
        let mut outside = false;
        for plane in partition.side_planes {
            let [x, y, z, w] = plane.values()?;
            let corner_x = if x <= 0.0 {
                shifted_max_x
            } else {
                shifted_min_x
            };
            let corner_y = if y <= 0.0 {
                f32::from_bits(XMAP11_TREE_CULL_MAX_Y_BITS)
            } else {
                0.0
            };
            let corner_z = if z <= 0.0 { max_z } else { min_z };
            if x * corner_x + y * corner_y + z * corner_z + w > 0.0 {
                outside = true;
                break;
            }
        }
        if outside {
            continue;
        }
        let dx = camera[0] - camera[0].clamp(shifted_min_x, shifted_max_x);
        let dy = camera[1] - f32::from_bits(XMAP11_TREE_DISTANCE_Y_BITS);
        let dz = camera[2] - camera[2].clamp(min_z, max_z);
        let distance_squared = dx * dx + dy * dy + dz * dz;
        if !distance_squared.is_finite() {
            return Err("XMAP11 tree chunk distance overflowed".into());
        }
        if closest
            .as_ref()
            .map(|(_, distance)| distance_squared < *distance)
            .unwrap_or(true)
        {
            closest = Some((partition.render_view_mask, distance_squared));
        }
    }
    Ok(closest)
}

pub fn select_tree_lod_index(model: &PdxModel, distance_squared: f32) -> Result<usize> {
    validate_tree_lod_model(model, "tree model")?;
    if !distance_squared.is_finite() || distance_squared < 0.0 {
        return Err("XMAP11 tree LOD distance is invalid".into());
    }
    let requested = model
        .lod_distances
        .iter()
        .rposition(|distance| distance * distance <= distance_squared)
        .unwrap_or(0);
    Ok(requested.min(model.shapes.len() - 1))
}

fn validate_tree_lod_model(model: &PdxModel, source: &str) -> Result<()> {
    if model.shapes.is_empty() || model.lod_distances.is_empty() {
        return Err(format!("XMAP11 {source} has no shapes or LOD thresholds").into());
    }
    let mut previous = None;
    for distance in &model.lod_distances {
        if !distance.is_finite() || *distance < 0.0 {
            return Err(format!("XMAP11 {source} has an invalid LOD threshold").into());
        }
        if previous.map(|value| *distance < value).unwrap_or(false) {
            return Err(format!("XMAP11 {source} LOD thresholds are not ordered").into());
        }
        previous = Some(*distance);
    }
    for (index, shape) in model.shapes.iter().enumerate() {
        if let Some(lod) = shape.lod {
            if lod != index as i32 {
                return Err(format!(
                    "XMAP11 {source} shape {index} declares noncanonical LOD {lod}"
                )
                .into());
            }
        }
    }
    Ok(())
}

fn drawable_tree_mesh_indices(shape: &PdxShape, lod_index: usize) -> Result<Vec<usize>> {
    let drawable = shape
        .meshes
        .iter()
        .enumerate()
        .filter(|(_, mesh)| !mesh.material.is_collision() && !mesh.verts.is_empty())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if drawable.is_empty() {
        Err(format!(
            "XMAP11 selected tree LOD {lod_index} shape {:?} has no drawable mesh",
            shape.name
        )
        .into())
    } else {
        Ok(drawable)
    }
}

fn admit_content(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    path: &str,
    content_by_path: &mut BTreeMap<String, UsedContentKey>,
) -> Result<UsedContentKey> {
    let key = admission
        .require_used_content(path)
        .map_err(|error| format!("XMAP11 content admission: {error}"))?
        .clone();
    key.validate()
        .map_err(|error| format!("invalid XMAP11 UsedContentKey: {error}"))?;
    resolver.read_admitted_used_content(&key)?;
    if let Some(first) = content_by_path.values().next() {
        if first.effective_vfs_manifest_sha256 != key.effective_vfs_manifest_sha256 {
            return Err("XMAP11 content spans different effective-VFS manifests".into());
        }
    }
    content_by_path
        .entry(key.normalized_game_relative_path.clone())
        .or_insert_with(|| key.clone());
    Ok(key)
}

fn admitted_material_texture_path(
    admission: &ValidatedContentAdmission,
    mesh_path: &str,
    reference: &str,
) -> Result<String> {
    for candidate in material_texture_candidates(mesh_path, reference) {
        if let Some(content) = admission.used_content(&candidate) {
            return Ok(content.normalized_game_relative_path.clone());
        }
    }
    Err(format!(
        "XMAP11 material texture {reference:?} from {mesh_path} does not resolve by the proven mesh-relative or game-root path law"
    )
    .into())
}

fn material_texture_candidates(mesh_path: &str, reference: &str) -> Vec<String> {
    let normalized = reference.replace('\\', "/");
    let mesh_dir = mesh_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let relative = if normalized.starts_with("gfx/") || mesh_dir.is_empty() {
        normalized.clone()
    } else {
        format!("{mesh_dir}/{normalized}")
    };
    if relative == normalized {
        vec![normalized]
    } else {
        vec![relative, normalized]
    }
}

fn identity_matrix_bits() -> [u32; 16] {
    [
        1.0f32.to_bits(),
        0,
        0,
        0,
        0,
        1.0f32.to_bits(),
        0,
        0,
        0,
        0,
        1.0f32.to_bits(),
        0,
        0,
        0,
        0,
        1.0f32.to_bits(),
    ]
}

fn validate_tree_asset(
    asset: &TreePreparedAsset,
    expected_index: u32,
    effective_vfs_manifest_sha256: &str,
    ordered_used_content: &[UsedContentKey],
) -> Result<()> {
    let mesh = &asset.mesh;
    asset
        .mesh_source
        .validate()
        .map_err(|error| format!("invalid XMAP11 tree mesh authority: {error}"))?;
    if asset.asset_index != expected_index
        || asset.palette_index == 0
        || asset.selected_shape.trim().is_empty()
        || mesh.material_shader.trim().is_empty()
        || mesh.source_binding.object_index != asset.palette_index
        || mesh.source_binding.mesh != mesh.source_mesh
        || mesh.source_mesh.to_ascii_lowercase() != asset.mesh_source.normalized_game_relative_path
        || asset.mesh_source.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256
        || mesh.vertex_stride_bytes != TREE_MESH_VERTEX_STRIDE_BYTES
        || mesh.vertex_count == 0
        || mesh.index_count == 0
        || mesh.vertices.len()
            != mesh
                .vertex_count
                .checked_mul(TREE_MESH_VERTEX_STRIDE_BYTES as usize)
                .ok_or("XMAP11 tree vertex extent overflowed")?
        || mesh.indices.len() != mesh.index_count
        || mesh
            .indices
            .iter()
            .any(|index| *index as usize >= mesh.vertex_count || *index > u16::MAX as u32)
        || !(2..=3).contains(&asset.ordered_material_textures.len())
        || !ordered_used_content.contains(&asset.mesh_source)
    {
        return Err("XMAP11 tree asset geometry or selected content identity is invalid".into());
    }
    let mut material_paths = BTreeSet::new();
    for texture in &asset.ordered_material_textures {
        texture
            .validate()
            .map_err(|error| format!("invalid XMAP11 tree material authority: {error}"))?;
        if texture.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256
            || !ordered_used_content.contains(texture)
            || !material_paths.insert(texture.normalized_game_relative_path.clone())
        {
            return Err("XMAP11 tree material authority drifts, repeats, or is unadmitted".into());
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("XMAP11 {label} SHA-256 is not canonical lowercase hex").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn inclusive_plane() -> TreeFrustumPlane {
        TreeFrustumPlane {
            x_f32_bits: 0,
            y_f32_bits: 0,
            z_f32_bits: 0,
            w_f32_bits: (-1.0f32).to_bits(),
        }
    }

    fn viewer(camera: [f32; 3]) -> TreeViewerState {
        TreeViewerState {
            camera_x_f32_bits: camera[0].to_bits(),
            camera_y_f32_bits: camera[1].to_bits(),
            camera_z_f32_bits: camera[2].to_bits(),
            ordered_partitions: vec![TreeViewerPartition {
                render_view_mask: 1,
                map_copy_offset_f32_bits: 0.0f32.to_bits(),
                side_planes: [inclusive_plane(); 4],
            }],
        }
    }

    fn content(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 1,
        }
    }

    fn caster_fixture() -> (TreePreparedAsset, PreparedTreeBatch) {
        let mesh_source = content("gfx/models/mapitems/trees/test.mesh");
        let asset = TreePreparedAsset {
            asset_index: 0,
            palette_index: 1,
            selected_lod_index: 0,
            selected_shape: "lod0".into(),
            selected_mesh_ordinal: 0,
            mesh_source: mesh_source.clone(),
            mesh: TreeMeshVertexBuffer {
                source_mesh: mesh_source.normalized_game_relative_path.clone(),
                source_binding: TreeGfxBinding {
                    object_index: 1,
                    name: "mapobject_1".into(),
                    mesh: mesh_source.normalized_game_relative_path.clone(),
                    scale: "1".into(),
                    source: TREE_GFX_BINDING_SOURCE.into(),
                    block_line: 1,
                    name_line: 1,
                    file_line: 1,
                    scale_line: None,
                },
                material_shader: "tree".into(),
                material_diffuse: Some("diffuse.dds".into()),
                material_normal_map: Some("normal.dds".into()),
                material_specular: None,
                vertex_stride_bytes: TREE_MESH_VERTEX_STRIDE_BYTES,
                vertex_count: 1,
                index_count: 1,
                vertices: vec![0; TREE_MESH_VERTEX_STRIDE_BYTES as usize],
                indices: vec![0],
            },
            ordered_material_textures: vec![
                content("gfx/models/mapitems/trees/diffuse.dds"),
                content("gfx/models/mapitems/trees/normal.dds"),
            ],
        };
        let draw = MapObjectDrawArguments::DrawIndexedInstanced {
            index_count_per_instance: 1,
            instance_count: 1,
            start_index: 0,
            base_vertex: 0,
            start_instance: 0,
        };
        let batch = PreparedTreeBatch {
            source_row_ordinal: 0,
            sort_byte: 0,
            sort_value_f32_bits: 1.0f32.to_bits(),
            partition_mask: L03_PARTITION_PRIMARY,
            map_copy_offset_f32_bits: 0.0f32.to_bits(),
            selected_lod_index: 0,
            asset_index: 0,
            instance_count: 1,
            instance_bytes: vec![0; TREE_INSTANCE_STRIDE_BYTES as usize],
            object_draw: PreparedMapObjectDraw {
                visible_order: 0,
                stable_typed_id: "tree:bucket:0".into(),
                partition_mask: L03_PARTITION_PRIMARY,
                render: MapObjectRenderContent {
                    entity_name: "mapobject_1".into(),
                    entity_source: content(TREE_GFX_BINDING_SOURCE),
                    selected_mesh: "lod0".into(),
                    mesh_source,
                    groups: vec![MapObjectMeshGroupContent {
                        submeshes: vec![MapObjectSubmeshContent {
                            selected_material: "tree".into(),
                            material_source: content("gfx/models/mapitems/trees/test.mesh"),
                            material_family: MapObjectMaterialFamily::Tree,
                            ordered_texture_sources: Vec::new(),
                            draw,
                        }],
                    }],
                    world_matrix_f32_bits: identity_matrix_bits(),
                    skinned: false,
                },
            },
        };
        (asset, batch)
    }

    fn inert_compiled_program(effect_name: &str) -> TreeCompiledProgram {
        TreeCompiledProgram {
            effect_name: effect_name.into(),
            vertex_entry: "VS".into(),
            pixel_entry: "PS".into(),
            ordered_defines: Vec::new(),
            vertex_translation_unit_sha256: "a".repeat(64),
            pixel_translation_unit_sha256: "a".repeat(64),
            vertex_bytecode_sha256: "a".repeat(64),
            pixel_bytecode_sha256: "a".repeat(64),
            vertex_bytecode: vec![1],
            pixel_bytecode: vec![1],
        }
    }

    #[test]
    fn changing_dynamic_authority_arc_reuses_static_payload_without_resealing_it() {
        let shader_source = content(TREE_SHADER_SOURCE);
        let static_payload = Arc::new(PreparedTreeStaticPayload {
            schema: XMAP11_TREE_FRAME_SCHEMA.into(),
            content_manifest_sha256: "a".repeat(64),
            effective_vfs_manifest_sha256: "a".repeat(64),
            map_width: 1,
            map_height: 1,
            viewer: viewer([0.0, 0.0, 0.0]),
            tree_viewer_state_sha256: "a".repeat(64),
            map_copy_receipt_set_sha256: "a".repeat(64),
            tree_fade: TreeFadeWindow {
                near_f32_bits: 0.0f32.to_bits(),
                far_f32_bits: 1.0f32.to_bits(),
            },
            tree_fade_f32_bits: 1.0f32.to_bits(),
            season_lerp_f32_bits: 0.0f32.to_bits(),
            season_column_f32_bits: 0.0f32.to_bits(),
            shader_programs: TreeShaderPrograms {
                shader_source: shader_source.clone(),
                ordered_shader_content: vec![shader_source],
                pixel_samplers: Vec::new(),
                main_pipeline: TreePipelineStateDescriptor {
                    blend_enable: true,
                    alpha_test: false,
                    write_mask: "RED|GREEN|BLUE".into(),
                    source_blend: "SRC_ALPHA".into(),
                    dest_blend: "INV_SRC_ALPHA".into(),
                    depth_enable: true,
                    depth_write: true,
                    depth_func: "LESS_EQUAL".into(),
                    fill_mode: "SOLID".into(),
                    cull_mode: "BACK".into(),
                    front_counter_clockwise: false,
                    depth_clip_enable: true,
                },
                main: inert_compiled_program(XMAP11_TREE_MAIN_EFFECT),
                unlit: inert_compiled_program(XMAP11_TREE_UNLIT_EFFECT),
                shadow: inert_compiled_program(XMAP11_TREE_SHADOW_EFFECT),
                deterministic_sha256: "a".repeat(64),
            },
            tree_mask: TreeMaskTexture {
                schema: XMAP11_TREE_MASK_TEXTURE_SCHEMA.into(),
                source_width: 2,
                source_height: 2,
                width: 1,
                height: 1,
                dxgi_format: XMAP11_TREE_MASK_DXGI_FORMAT,
                pixels: vec![0],
                payload_sha256: Sha256::hex(&[0]),
            },
            ordered_used_content: Vec::new(),
            assets: Vec::new(),
            selected_batches: Vec::new(),
            ordered_main_draw_batch_indices: Vec::new(),
            static_payload_sha256: "a".repeat(64),
            native_resource_closure_sha256: "b".repeat(64),
            foundation_resource_census_sha256: "c".repeat(64),
        });
        let mut first = PreparedTreeFrame {
            hour_index: 1,
            replay_frame_identity_sha256: "d".repeat(64),
            camera_frame_sha256: "e".repeat(64),
            viewer_state_sha256: "f".repeat(64),
            static_payload,
            prepared_frame_sha256: String::new(),
        };
        first.reseal_dynamic_authority().unwrap();
        let immutable_seals_before = XMAP11_IMMUTABLE_SEAL_COMPUTATIONS.with(std::cell::Cell::get);
        let first_prepared_sha256 = first.prepared_frame_sha256.clone();

        let mut next = first.clone();
        next.hour_index = 2;
        next.replay_frame_identity_sha256 = "0".repeat(64);
        next.camera_frame_sha256 = "1".repeat(64);
        next.viewer_state_sha256 = "2".repeat(64);
        next.reseal_dynamic_authority().unwrap();

        assert!(Arc::ptr_eq(&first.static_payload, &next.static_payload));
        assert_eq!(
            XMAP11_IMMUTABLE_SEAL_COMPUTATIONS.with(std::cell::Cell::get),
            immutable_seals_before,
            "dynamic resealing must not recompute either immutable tree seal"
        );
        assert_eq!(
            first.native_resource_closure_sha256(),
            next.native_resource_closure_sha256()
        );
        assert_ne!(first_prepared_sha256, next.prepared_frame_sha256);
    }

    fn selected_light_fixture() -> L03SelectedLightFrame {
        let light_view_projection_f32_bits = identity_matrix_bits();
        let matrix = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                f32::from_bits(light_view_projection_f32_bits[row * 4 + column])
            })
        });
        let texture = crate::shadow_law::compose_shadow_map_texture_matrix(matrix).unwrap();
        L03SelectedLightFrame {
            selected_light: L03SelectedLight::Sun,
            light_view_projection_f32_bits,
            shadow_map_texture_matrix_f32_bits: std::array::from_fn(|lane| {
                texture[lane / 4][lane % 4].to_bits()
            }),
            light_depth_target: L03LightDepthTarget {
                stable_resource_id: "xmap11/test/light-depth".into(),
                width: L03_LIGHT_DEPTH_WIDTH,
                height: L03_LIGHT_DEPTH_HEIGHT,
                dxgi_format: L03_LIGHT_DEPTH_DXGI_FORMAT,
                mip_levels: L03_LIGHT_DEPTH_MIP_LEVELS,
            },
        }
    }

    fn drawable_mesh(shader: &str) -> PdxMeshData {
        PdxMeshData {
            verts: vec![[0.0, 0.0, 0.0]],
            normals: None,
            tangents: None,
            uv0: None,
            uv1: None,
            indices: vec![0, 0, 0],
            aabb_min: [0.0; 3],
            aabb_max: [0.0; 3],
            bounding_sphere: None,
            material: PdxMaterial {
                shader: shader.into(),
                diffuse: None,
                normal_map: None,
                specular: None,
            },
            skin: None,
        }
    }

    #[test]
    fn every_valid_lod_submesh_is_selected_and_collision_is_not_drawn() {
        let shape = PdxShape {
            name: "lod0".into(),
            lod: Some(0),
            meshes: vec![
                drawable_mesh("tree"),
                drawable_mesh("tree"),
                drawable_mesh("Collision"),
            ],
            skeleton: Vec::new(),
        };
        assert_eq!(drawable_tree_mesh_indices(&shape, 0).unwrap(), vec![0, 1]);
    }

    #[test]
    fn material_texture_candidates_never_invent_a_basename_lookup() {
        assert_eq!(
            material_texture_candidates(
                "gfx/models/mapitems/trees/forest.mesh",
                "textures/shared.dds",
            ),
            vec![
                "gfx/models/mapitems/trees/textures/shared.dds",
                "textures/shared.dds",
            ]
        );
        assert!(!material_texture_candidates(
            "gfx/models/mapitems/trees/forest.mesh",
            "textures/shared.dds",
        )
        .iter()
        .any(|candidate| candidate == "shared.dds"));
    }

    #[test]
    fn exact_chunk_distance_uses_a729_reference_height_and_clamped_xz() {
        let selected = select_tree_chunk_partition(1024, 512, 1, 0, &viewer([300.0, 109.5, 12.0]))
            .unwrap()
            .unwrap();
        assert_eq!(selected.0, 1);
        assert_eq!(selected.1.to_bits(), (100.0f32 * 100.0).to_bits());
    }

    #[test]
    fn partition_tie_keeps_engine_order_and_strictly_closer_replaces_it() {
        let mut state = viewer([10.0, 9.5, 10.0]);
        state.ordered_partitions.push(TreeViewerPartition {
            render_view_mask: 2,
            map_copy_offset_f32_bits: 0.0f32.to_bits(),
            side_planes: [inclusive_plane(); 4],
        });
        assert_eq!(
            select_tree_chunk_partition(512, 512, 0, 0, &state)
                .unwrap()
                .unwrap()
                .0,
            1
        );
        state.ordered_partitions[1].map_copy_offset_f32_bits = (-256.0f32).to_bits();
        state.camera_x_f32_bits = (-10.0f32).to_bits();
        assert_eq!(
            select_tree_chunk_partition(512, 512, 0, 0, &state)
                .unwrap()
                .unwrap()
                .0,
            2
        );
    }

    #[test]
    fn fade_is_exact_linear_window_and_skips_above_far() {
        let fade =
            tree_fade_window_from_graphics_defines("TREE_FADE_NEAR = 250\nTREE_FADE_FAR = 350")
                .unwrap();
        assert_eq!(fade.value(200.0).unwrap().to_bits(), 1.0f32.to_bits());
        assert_eq!(fade.value(300.0).unwrap().to_bits(), 0.5f32.to_bits());
        assert_eq!(fade.value(351.0).unwrap().to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn tree_mask_preserves_a729_overlapping_filter_and_adjacent_output_read() {
        let mut pixels = vec![0xfe; 8 * 8];
        for (x, y) in [
            (0usize, 0usize),
            (1, 0),
            (1, 1),
            (3, 2),
            (4, 3),
            (5, 4),
            (6, 6),
            (7, 7),
        ] {
            pixels[y * 8 + x] = 0;
        }
        let texture = synthesize_tree_mask_texture(&IndexImage {
            width: 8,
            height: 8,
            pixels,
        })
        .unwrap();
        assert_eq!(texture.width, 4);
        assert_eq!(texture.height, 4);
        assert_eq!(
            texture.pixels,
            vec![63, 63, 0, 0, 0, 63, 63, 0, 0, 0, 63, 0, 15, 0, 31, 79]
        );
    }

    #[test]
    fn assigned_stage_component_exposes_exact_row_and_no_packet_owned_edges() {
        assert_eq!(
            xmap11_assigned_stage_rows(),
            [AssignedStageIdentity::new(
                "trees",
                "trees.prepare-and-draw"
            )]
        );
        assert!(xmap11_assigned_stage_edges().is_empty());
    }

    #[test]
    fn l03_caster_adapter_is_derived_from_the_same_prepared_object_draw() {
        let (asset, batch) = caster_fixture();
        let caster = batch.l03_shadow_input(&asset).unwrap();
        assert_eq!(caster.selected_mesh_ordinal, 0);
        assert_eq!(caster.selected_mesh, "lod0");
        assert_eq!(caster.selected_material, "tree");
        assert!(matches!(
            caster.draw,
            L03DrawArguments::DrawIndexedInstanced {
                index_count_per_instance: 1,
                instance_count: 1,
                ..
            }
        ));

        let mut drifted = batch;
        drifted.object_draw.render.groups[0].submeshes[0].selected_material = "foreign".into();
        assert!(drifted.l03_shadow_input(&asset).is_err());
    }

    #[test]
    fn multi_submesh_xmap11_casters_enter_promoted_xmap09_projection() {
        let (asset0, batch0) = caster_fixture();
        let mut asset1 = asset0.clone();
        asset1.asset_index = 1;
        asset1.selected_mesh_ordinal = 1;
        let mut batch1 = batch0.clone();
        batch1.asset_index = 1;
        batch1.object_draw.stable_typed_id = "tree:bucket:0:mesh:1".into();

        let casters =
            l03_shadow_batches_for_main_draws(&[asset0, asset1], &[batch0, batch1], &[0, 1])
                .unwrap();
        let projected = project_l03_light_depth(&L03ProjectionRequest {
            shadow_pass_active: Some(true),
            selected_light_frame: Some(selected_light_fixture()),
            enabled_partition_mask: Some(L03_PARTITION_PRIMARY),
            semantic_rows: Some(Vec::new()),
            tree_batches: Some(casters),
        })
        .unwrap();
        let keys = projected
            .submissions
            .iter()
            .map(|submission| match submission {
                L03LightDepthSubmission::Tree {
                    source_row_ordinal,
                    selected_mesh_ordinal,
                    ..
                } => (*source_row_ordinal, *selected_mesh_ordinal),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, [(0, 0), (0, 1)]);
    }

    #[test]
    fn above_fade_main_scene_gate_exports_no_xmap09_shadow_casters() {
        let (asset, batch) = caster_fixture();
        let main_draws = main_tree_draw_batch_indices(351.0, 350.0f32.to_bits(), 1).unwrap();
        assert!(main_draws.is_empty());
        assert!(
            l03_shadow_batches_for_main_draws(&[asset], &[batch], &main_draws)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            main_tree_draw_batch_indices(350.0, 350.0f32.to_bits(), 1).unwrap(),
            [0]
        );
    }

    #[test]
    fn frame_visibility_gate_rejects_resealed_camera_fade_and_draw_set_tamper() {
        let fade = TreeFadeWindow {
            near_f32_bits: 250.0f32.to_bits(),
            far_f32_bits: 350.0f32.to_bits(),
        };
        let below_far = viewer([10.0, 300.0, 10.0]);
        validate_main_tree_visibility_gate(&below_far, fade, 0.5f32.to_bits(), 2, &[0, 1]).unwrap();
        assert!(
            validate_main_tree_visibility_gate(&below_far, fade, 0.6f32.to_bits(), 2, &[0, 1],)
                .is_err()
        );
        assert!(
            validate_main_tree_visibility_gate(&below_far, fade, 0.5f32.to_bits(), 2, &[],)
                .is_err()
        );

        let above_far = viewer([10.0, 351.0, 10.0]);
        validate_main_tree_visibility_gate(&above_far, fade, 0.0f32.to_bits(), 2, &[]).unwrap();
        assert!(
            validate_main_tree_visibility_gate(&above_far, fade, 0.0f32.to_bits(), 2, &[0, 1],)
                .is_err()
        );
    }
}
