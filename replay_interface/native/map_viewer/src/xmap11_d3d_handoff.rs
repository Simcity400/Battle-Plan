//! Normal MainScene execution for the file/state-derived XMAP11 tree frame.
//!
//! Material and shared map resources are borrowed from the product resource
//! registry, but every pointer is paired with a pointer-free content/descriptor
//! identity and checked against the prepared frame before use. TreeMask,
//! geometry, instances, CB1, shaders, and fixed pipeline state remain XMAP11
//! owned.

use crate::{
    PreparedTreeFrame, Result, TreeActualDrawExecution, TreePipelineStateDescriptor,
    TreeSamplerStateDescriptor, TreeTargetBoundDrawReceipt, XMAP11_TREE_OPERATION_NAME,
    XMAP11_TREE_STAGE_NAME,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::AssignedStage;
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP11_TREE_NATIVE_EXECUTION_SCHEMA: &str = "hoi4.xmap11.tree-native-execution.a729.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeGpuResourceIdentity {
    pub semantic: String,
    pub slot: u8,
    pub content_payload_sha256: String,
    pub descriptor_sha256: String,
}

impl TreeGpuResourceIdentity {
    fn validate(&self) -> Result<()> {
        if self.semantic.is_empty() {
            return Err("XMAP11 GPU resource semantic is blank".into());
        }
        validate_sha256(&self.content_payload_sha256, "tree GPU resource payload")?;
        validate_sha256(&self.descriptor_sha256, "tree GPU resource descriptor")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeNativeExecutionAuthority {
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    /// Digest of the complete Trees `FamilyDisposition` sealed by the
    /// production-foundation adapter for this frame.
    pub foundation_authorization_sha256: String,
    /// Explicit effect state: `true` means t5 is intentionally null.
    pub color_map_second_is_null: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeNativeExecutionReceipt {
    pub schema: String,
    pub operation_id: String,
    pub stage_name: String,
    pub assigned_ordinal: u32,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub prepared_frame_sha256: String,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub foundation_authorization_sha256: String,
    pub viewer_state_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub main_scene_target_sha256: String,
    pub issued_draw_count: u32,
    pub ordered_draw_receipts: Vec<TreeTargetBoundDrawReceipt>,
    pub deterministic_sha256: String,
}

impl TreeNativeExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP11_TREE_NATIVE_EXECUTION_SCHEMA
            || self.operation_id != XMAP11_TREE_OPERATION_NAME
            || self.stage_name != XMAP11_TREE_STAGE_NAME
            || self.assigned_ordinal == 0
            || self.issued_draw_count as usize != self.ordered_draw_receipts.len()
        {
            return Err("XMAP11 native execution receipt shape is invalid".into());
        }
        for (value, label) in [
            (&self.topology_sha256, "tree assigned-stage topology"),
            (&self.assignment_sha256, "tree assigned-stage assignment"),
            (&self.content_manifest_sha256, "tree content manifest"),
            (&self.replay_frame_identity_sha256, "tree replay frame"),
            (&self.prepared_frame_sha256, "tree prepared frame"),
            (&self.program_sha256, "tree program"),
            (&self.ordered_resources_sha256, "tree ordered resources"),
            (
                &self.foundation_content_operation_sha256,
                "tree foundation operation",
            ),
            (
                &self.foundation_authorization_sha256,
                "tree foundation family authorization",
            ),
            (&self.viewer_state_sha256, "tree viewer state"),
            (
                &self.map_copy_receipt_set_sha256,
                "tree map-copy receipt set",
            ),
            (&self.main_scene_target_sha256, "tree MainScene target"),
            (&self.deterministic_sha256, "tree native execution receipt"),
        ] {
            validate_sha256(value, label)?;
        }
        for (ordinal, draw) in self.ordered_draw_receipts.iter().enumerate() {
            draw.validate()?;
            if draw.draw_ordinal as usize != ordinal
                || draw.operation_id != self.operation_id
                || draw.stage_name != self.stage_name
                || draw.assigned_ordinal != self.assigned_ordinal
                || draw.topology_sha256 != self.topology_sha256
                || draw.assignment_sha256 != self.assignment_sha256
                || draw.hour_index != self.hour_index
                || draw.content_manifest_sha256 != self.content_manifest_sha256
                || draw.replay_frame_identity_sha256 != self.replay_frame_identity_sha256
                || draw.prepared_frame_sha256 != self.prepared_frame_sha256
                || draw.program_sha256 != self.program_sha256
                || draw.ordered_resources_sha256 != self.ordered_resources_sha256
                || draw.foundation_content_operation_sha256
                    != self.foundation_content_operation_sha256
                || draw.foundation_authorization_sha256
                    != self.foundation_authorization_sha256
                || draw.viewer_state_sha256 != self.viewer_state_sha256
                || draw.map_copy_receipt_set_sha256 != self.map_copy_receipt_set_sha256
                || draw.main_scene_target_sha256 != self.main_scene_target_sha256
            {
                return Err("XMAP11 native draw receipts drift from their aggregate".into());
            }
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP11 native execution receipt digest mismatch".into());
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        frame: &PreparedTreeFrame,
        schedule: &AssignedStage,
        authority: &TreeNativeExecutionAuthority,
    ) -> Result<()> {
        self.validate()?;
        frame.validate()?;
        schedule
            .validate()
            .map_err(|error| format!("XMAP11 assigned stage: {error}"))?;
        if self.stage_name != schedule.stage_name
            || self.operation_id != schedule.operation_name
            || self.assigned_ordinal != schedule.assigned_ordinal
            || self.topology_sha256 != schedule.topology_sha256
            || self.assignment_sha256 != schedule.assignment_sha256
            || self.hour_index != authority.hour_index
            || self.hour_index != frame.hour_index
            || self.content_manifest_sha256 != frame.content_manifest_sha256
            || self.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
            || self.replay_frame_identity_sha256 != authority.replay_frame_identity_sha256
            || self.prepared_frame_sha256 != frame.prepared_frame_sha256
            || self.program_sha256 != frame.shader_programs.deterministic_sha256
            || self.viewer_state_sha256 != frame.viewer_state_sha256
            || self.viewer_state_sha256 != authority.viewer_state_sha256
            || self.map_copy_receipt_set_sha256 != frame.map_copy_receipt_set_sha256
            || self.map_copy_receipt_set_sha256 != authority.map_copy_receipt_set_sha256
            || self.main_scene_target_sha256 != authority.main_scene_target_sha256
            || self.foundation_authorization_sha256
                != authority.foundation_authorization_sha256
            || self.issued_draw_count as usize != frame.ordered_main_draw_batch_indices.len()
        {
            return Err(
                "XMAP11 aggregate receipt drifts from frame, schedule, authority, or exact issued draw set"
                    .into(),
            );
        }
        for draw in &self.ordered_draw_receipts {
            draw.validate_against(frame, schedule)?;
        }
        let foundation = schedule
            .foundation_operation_identity(
                self.program_sha256.clone(),
                self.ordered_resources_sha256.clone(),
                self.main_scene_target_sha256.clone(),
            )
            .map_err(|error| format!("XMAP11 aggregate foundation identity: {error}"))?;
        if self.foundation_content_operation_sha256
            != Sha256::hex(&serde_json::to_vec(&foundation)?)
        {
            return Err(
                "XMAP11 aggregate foundation identity drifts from the assigned stage".into(),
            );
        }
        if self.foundation_authorization_sha256
            != crate::xmap11_foundation_authorization_sha256(frame, schedule)?
        {
            return Err(
                "XMAP11 aggregate receipt drifts from the foundation family authorization".into(),
            );
        }
        Ok(())
    }
}

#[cfg(windows)]
pub struct NativeTreeShaderResource {
    pub identity: TreeGpuResourceIdentity,
    pub view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
}

#[cfg(windows)]
pub struct NativeTreeAssetResources {
    pub asset_index: u32,
    pub diffuse: NativeTreeShaderResource,
    pub normal: NativeTreeShaderResource,
}

#[cfg(windows)]
pub struct NativeTreePartitionSceneBinding {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub viewer_state_sha256: String,
    pub scene_cb0_payload_sha256: String,
    pub scene_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
}

#[cfg(windows)]
pub struct NativeTreeMainSceneBindings {
    pub color_rtv: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    pub depth_dsv: *mut winapi::um::d3d11::ID3D11DepthStencilView,
    pub width: u32,
    pub height: u32,
    /// One exact SceneCB0 per admitted XMAP04 map-copy row.
    pub ordered_partition_scene_cb0: Vec<NativeTreePartitionSceneBinding>,
    /// Exact tree/fog PS CB2.
    pub pixel_cb2: *mut winapi::um::d3d11::ID3D11Buffer,
    pub shadow_map_texture_matrix_f32_bits: [u32; 16],
    /// t2,t3,t4,(optional t5),t6,t8..t13,t15 in slot order.
    pub ordered_shared_resources: Vec<NativeTreeShaderResource>,
    pub ordered_asset_resources: Vec<NativeTreeAssetResources>,
}

#[cfg(windows)]
struct OwnedTreeTexture {
    identity: TreeGpuResourceIdentity,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
}

#[cfg(windows)]
struct OwnedTreeAssetTextures {
    asset_index: u32,
    diffuse: OwnedTreeTexture,
    normal: OwnedTreeTexture,
}

#[cfg(windows)]
struct OwnedTreeGeometry {
    asset_index: u32,
    vertex: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    index: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
}

#[cfg(windows)]
struct OwnedTreeInstances {
    source_row_ordinal: u32,
    selected_mesh_ordinal: u32,
    partition_mask: u8,
    buffer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
}

/// Same-device XMAP11 owner for all immutable MainScene and TreeShadow GPU
/// resources. Per-frame execution creates only the target/light-dependent CB1.
#[cfg(windows)]
pub struct NativeXmap11ResourceOwner {
    device_identity: usize,
    resource_closure_sha256: String,
    content_manifest_sha256: String,
    effective_vfs_manifest_sha256: String,
    private_shared: Vec<OwnedTreeTexture>,
    assets: Vec<OwnedTreeAssetTextures>,
    main_vertex: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    main_pixel: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    main_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    main_blend: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    main_depth: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    main_raster: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    main_samplers:
        BTreeMap<u8, crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>>,
    shadow_vertex: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    shadow_pixel: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    shadow_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    shadow_depth: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    shadow_raster: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    shadow_samplers:
        BTreeMap<u8, crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>>,
    _tree_mask_texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    tree_mask_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    geometries: Vec<OwnedTreeGeometry>,
    instances: Vec<OwnedTreeInstances>,
}

#[cfg(windows)]
impl NativeXmap11ResourceOwner {
    /// # Safety
    /// `device` must remain live for this call. Returned COM objects retain
    /// their resources and are accepted only for the exact prepared frame.
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &crate::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        frame: &PreparedTreeFrame,
    ) -> Result<Self> {
        frame.validate()?;
        if device.is_null()
            || !resolver.has_content_admission()
            || admission.manifest_digest_sha256() != frame.content_manifest_sha256
        {
            return Err("XMAP11 resource owner requires the frame's app device and admission-bound effective VFS".into());
        }
        let content = |path: &str| -> Result<&UsedContentKey> {
            used_content_by_game_path(&frame.ordered_used_content, path)
                .ok_or_else(|| format!("XMAP11 frame omitted admitted content {path}").into())
        };
        let tint_content = content(crate::TREE_TINT_TEXTURE_SOURCE)?;
        let season_content = content(crate::TREE_SEASON_TEXTURE_SOURCE)?;
        let tint = create_tree_bmp_texture(
            device,
            resolver,
            tint_content,
            "TintMap",
            2,
        )?;
        let season = create_tree_bmp_texture(
            device,
            resolver,
            season_content,
            "SeasonMap",
            3,
        )?;
        let mut private_shared = vec![tint, season];
        if frame.shader_programs.main.effect_name == crate::XMAP11_TREE_MAIN_EFFECT {
            let environment = content(crate::TREE_ENVIRONMENT_TEXTURE_SOURCE)?;
            let (texture, view, descriptor_sha256) =
                crate::xmap12_d3d_handoff::create_admitted_environment_cube_on_device(
                    device, resolver, admission, environment,
                )?;
            private_shared.push(OwnedTreeTexture {
                identity: TreeGpuResourceIdentity {
                    semantic: "EnvironmentMap".into(),
                    slot: 9,
                    content_payload_sha256: environment.payload_sha256.clone(),
                    descriptor_sha256,
                },
                _texture: texture,
                view,
            });
        }
        private_shared.sort_by_key(|row| row.identity.slot);

        let used_asset_indices = frame
            .ordered_main_draw_batch_indices
            .iter()
            .map(|index| frame.selected_batches[*index as usize].asset_index)
            .collect::<BTreeSet<_>>();
        let mut assets = Vec::with_capacity(used_asset_indices.len());
        for asset_index in used_asset_indices.iter().copied() {
            let asset = &frame.assets[asset_index as usize];
            let diffuse_content = asset
                .ordered_material_textures
                .first()
                .ok_or("XMAP11 asset omitted diffuse content")?;
            let normal_content = asset
                .ordered_material_textures
                .get(1)
                .ok_or("XMAP11 asset omitted normal content")?;
            let (diffuse_texture, diffuse_view, diffuse_descriptor) =
                crate::xmap12_d3d_handoff::create_admitted_material_dds_on_device(
                    device,
                    resolver,
                    admission,
                    diffuse_content,
                )?;
            let (normal_texture, normal_view, normal_descriptor) =
                crate::xmap12_d3d_handoff::create_admitted_material_dds_on_device(
                    device,
                    resolver,
                    admission,
                    normal_content,
                )?;
            assets.push(OwnedTreeAssetTextures {
                asset_index,
                diffuse: OwnedTreeTexture {
                    identity: TreeGpuResourceIdentity {
                        semantic: "DiffuseMap".into(),
                        slot: 0,
                        content_payload_sha256: diffuse_content.payload_sha256.clone(),
                        descriptor_sha256: diffuse_descriptor,
                    },
                    _texture: diffuse_texture,
                    view: diffuse_view,
                },
                normal: OwnedTreeTexture {
                    identity: TreeGpuResourceIdentity {
                        semantic: "NormalMap".into(),
                        slot: 1,
                        content_payload_sha256: normal_content.payload_sha256.clone(),
                        descriptor_sha256: normal_descriptor,
                    },
                    _texture: normal_texture,
                    view: normal_view,
                },
            });
        }

        let mut geometries = Vec::with_capacity(used_asset_indices.len());
        for asset_index in used_asset_indices {
            let asset = &frame.assets[asset_index as usize];
            let indices = asset
                .mesh
                .indices
                .iter()
                .map(|value| u16::try_from(*value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| "XMAP11 tree mesh index exceeds exact R16 binding")?;
            let index_bytes = indices
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            geometries.push(OwnedTreeGeometry {
                asset_index: asset.asset_index,
                vertex: create_buffer(
                    device,
                    winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
                    &asset.mesh.vertices,
                    "tree shadow vertex",
                )?,
                index: create_buffer(
                    device,
                    winapi::um::d3d11::D3D11_BIND_INDEX_BUFFER,
                    &index_bytes,
                    "tree shadow index",
                )?,
            });
        }
        let mut instances = Vec::with_capacity(frame.ordered_main_draw_batch_indices.len());
        for batch_index in &frame.ordered_main_draw_batch_indices {
            let batch = &frame.selected_batches[*batch_index as usize];
            let asset = &frame.assets[batch.asset_index as usize];
            instances.push(OwnedTreeInstances {
                source_row_ordinal: batch.source_row_ordinal,
                selected_mesh_ordinal: asset.selected_mesh_ordinal,
                partition_mask: batch.partition_mask,
                buffer: create_buffer(
                    device,
                    winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
                    &batch.instance_bytes,
                    "tree shadow instances",
                )?,
            });
        }
        let main_vertex = create_vertex_shader(device, &frame.shader_programs.main.vertex_bytecode)?;
        let main_pixel = create_pixel_shader(device, &frame.shader_programs.main.pixel_bytecode)?;
        let main_layout = create_tree_input_layout(device, &frame.shader_programs.main.vertex_bytecode)?;
        let main_blend = create_blend_state(device, &frame.shader_programs.main_pipeline)?;
        let main_depth = create_depth_state(device, &frame.shader_programs.main_pipeline)?;
        let main_raster = create_rasterizer_state(device, &frame.shader_programs.main_pipeline)?;
        let main_samplers = create_samplers(device, &frame.shader_programs.pixel_samplers)?
            .into_iter()
            .collect();
        let shadow_vertex = create_vertex_shader(device, &frame.shader_programs.shadow.vertex_bytecode)?;
        let shadow_pixel = create_pixel_shader(device, &frame.shader_programs.shadow.pixel_bytecode)?;
        let shadow_layout = create_tree_input_layout(device, &frame.shader_programs.shadow.vertex_bytecode)?;
        let shadow_depth = create_default_shadow_depth_state(device)?;
        let shadow_raster = create_default_shadow_rasterizer_state(device)?;
        let shadow_samplers = create_samplers(device, &frame.shader_programs.pixel_samplers)?
            .into_iter()
            .collect();
        let (_tree_mask_texture, tree_mask_view) = create_tree_mask(device, frame)?;
        Ok(Self {
            device_identity: device as usize,
            resource_closure_sha256: frame.native_resource_closure_sha256().to_owned(),
            content_manifest_sha256: frame.content_manifest_sha256.clone(),
            effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
            private_shared,
            assets,
            main_vertex,
            main_pixel,
            main_layout,
            main_blend,
            main_depth,
            main_raster,
            main_samplers,
            shadow_vertex,
            shadow_pixel,
            shadow_layout,
            shadow_depth,
            shadow_raster,
            shadow_samplers,
            _tree_mask_texture,
            tree_mask_view,
            geometries,
            instances,
        })
    }

    pub fn matches(
        &self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedTreeFrame,
    ) -> bool {
        self.device_identity == device as usize
            && self.resource_closure_sha256 == frame.native_resource_closure_sha256()
            && self.content_manifest_sha256 == frame.content_manifest_sha256
            && self.effective_vfs_manifest_sha256 == frame.effective_vfs_manifest_sha256
    }

    pub fn private_shared_resources(&self) -> Vec<NativeTreeShaderResource> {
        self.private_shared
            .iter()
            .map(|row| NativeTreeShaderResource {
                identity: row.identity.clone(),
                view: row.view.as_raw(),
            })
            .collect()
    }

    pub fn asset_resources(&self) -> Vec<NativeTreeAssetResources> {
        self.assets
            .iter()
            .map(|row| NativeTreeAssetResources {
                asset_index: row.asset_index,
                diffuse: NativeTreeShaderResource {
                    identity: row.diffuse.identity.clone(),
                    view: row.diffuse.view.as_raw(),
                },
                normal: NativeTreeShaderResource {
                    identity: row.normal.identity.clone(),
                    view: row.normal.view.as_raw(),
                },
            })
            .collect()
    }
}

#[cfg(windows)]
pub struct NativeXmap11LightDepthCaster<'a> {
    owner: &'a NativeXmap11ResourceOwner,
    frame: &'a PreparedTreeFrame,
    selected_light_frame_sha256: String,
    partitions: &'a [crate::NativeXmap12ShadowPartitionBinding],
    registered_submission_sha256s: Vec<String>,
}

#[cfg(windows)]
impl<'a> NativeXmap11LightDepthCaster<'a> {
    pub fn new(
        device: *mut winapi::um::d3d11::ID3D11Device,
        owner: &'a NativeXmap11ResourceOwner,
        frame: &'a PreparedTreeFrame,
        selected_light: &crate::L03SelectedLightFrame,
        projection: &crate::L03LightDepthProjection,
        partitions: &'a [crate::NativeXmap12ShadowPartitionBinding],
    ) -> Result<Self> {
        frame.validate()?;
        if !owner.matches(device, frame)
            || projection.inactive_excluded
            || projection.selected_light != Some(selected_light.selected_light)
            || projection.light_depth_target.as_ref() != Some(&selected_light.light_depth_target)
        {
            return Err("XMAP11 L03 caster owner/projection does not match the prepared tree frame".into());
        }
        let selected_light_frame_sha256 = Sha256::hex(&serde_json::to_vec(selected_light)?);
        let mut registered_submission_sha256s = Vec::new();
        let mut expected = BTreeSet::new();
        for submission in &projection.submissions {
            let crate::L03LightDepthSubmission::Tree {
                partition_mask,
                source_row_ordinal,
                selected_mesh_ordinal,
                selected_lod_index,
                selected_mesh,
                selected_material,
                pipeline,
                draw,
            } = submission
            else {
                continue;
            };
            if *pipeline != crate::L03ShadowPipeline::TreeShadow {
                return Err("XMAP11 L03 projection selected a foreign shadow pipeline".into());
            }
            let (batch, asset) = tree_batch_for_submission(
                frame,
                *partition_mask,
                *source_row_ordinal,
                *selected_mesh_ordinal,
            )?;
            if batch.selected_lod_index != *selected_lod_index
                || asset.selected_shape != *selected_mesh
                || asset.mesh.material_shader != *selected_material
                || batch.l03_shadow_input(asset)?.draw != *draw
            {
                return Err("XMAP11 L03 projection differs from prepared tree geometry".into());
            }
            if partitions.iter().all(|row| {
                row.partition_mask != *partition_mask
                    || row.selected_light_frame_sha256 != selected_light_frame_sha256
            }) {
                return Err("XMAP11 L03 caster lacks the selected-light partition SceneCB0".into());
            }
            if !expected.insert((*source_row_ordinal, *selected_mesh_ordinal, *partition_mask)) {
                return Err("XMAP11 L03 projection repeats a tree submission".into());
            }
            registered_submission_sha256s.push(Sha256::hex(&serde_json::to_vec(submission)?));
        }
        let frame_expected = frame
            .l03_shadow_batches()?
            .into_iter()
            .map(|row| (row.source_row_ordinal, row.selected_mesh_ordinal, row.partition_mask))
            .collect::<BTreeSet<_>>();
        if expected != frame_expected {
            return Err("XMAP11 L03 projection does not exactly cover the admitted tree caster census".into());
        }
        Ok(Self {
            owner,
            frame,
            selected_light_frame_sha256,
            partitions,
            registered_submission_sha256s,
        })
    }
}

#[cfg(windows)]
impl crate::NativeXmap09LightDepthCaster for NativeXmap11LightDepthCaster<'_> {
    fn owner_packet(&self) -> &str {
        "XMAP11"
    }

    fn camera_frame_sha256(&self) -> &str {
        &self.frame.camera_frame_sha256
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.frame.replay_frame_identity_sha256
    }

    fn registered_submission_sha256s(&self) -> &[String] {
        &self.registered_submission_sha256s
    }

    unsafe fn draw_light_depth_submission(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        selected_light: &crate::L03SelectedLightFrame,
        submission: &crate::L03LightDepthSubmission,
    ) -> Result<()> {
        use winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT;
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
        if context.is_null()
            || Sha256::hex(&serde_json::to_vec(selected_light)?)
                != self.selected_light_frame_sha256
        {
            return Err("XMAP11 L03 selected-light/context identity changed after registration".into());
        }
        let mut context_device = std::ptr::null_mut();
        (*context).GetDevice(&mut context_device);
        let context_device = crate::d3d11_device::D3dPtr::from_raw(
            context_device,
            "XMAP11 L03 context device",
        )?;
        if context_device.as_raw() as usize != self.owner.device_identity {
            return Err("XMAP11 L03 caster is bound to another D3D11 device".into());
        }
        let submission_sha256 = Sha256::hex(&serde_json::to_vec(submission)?);
        if !self
            .registered_submission_sha256s
            .iter()
            .any(|registered| registered == &submission_sha256)
        {
            return Err("XMAP11 L03 draw is outside the registered tree census".into());
        }
        let crate::L03LightDepthSubmission::Tree {
            partition_mask,
            source_row_ordinal,
            selected_mesh_ordinal,
            selected_lod_index,
            selected_mesh,
            selected_material,
            pipeline,
            draw,
        } = submission
        else {
            return Err("XMAP11 L03 caster cannot execute a PdxMesh submission".into());
        };
        if *pipeline != crate::L03ShadowPipeline::TreeShadow {
            return Err("XMAP11 L03 draw selected a foreign pipeline".into());
        }
        let (batch, asset) = tree_batch_for_submission(
            self.frame,
            *partition_mask,
            *source_row_ordinal,
            *selected_mesh_ordinal,
        )?;
        if batch.selected_lod_index != *selected_lod_index
            || asset.selected_shape != *selected_mesh
            || asset.mesh.material_shader != *selected_material
            || batch.l03_shadow_input(asset)?.draw != *draw
        {
            return Err("XMAP11 L03 draw differs from prepared tree geometry".into());
        }
        let partition = self
            .partitions
            .iter()
            .find(|row| row.partition_mask == *partition_mask)
            .ok_or("XMAP11 L03 partition SceneCB0 disappeared")?;
        let geometry = self
            .owner
            .geometries
            .iter()
            .find(|row| row.asset_index == batch.asset_index)
            .ok_or("XMAP11 L03 retained geometry disappeared")?;
        let instances = self
            .owner
            .instances
            .iter()
            .find(|row| {
                row.source_row_ordinal == *source_row_ordinal
                    && row.selected_mesh_ordinal == *selected_mesh_ordinal
                    && row.partition_mask == *partition_mask
            })
            .ok_or("XMAP11 L03 retained instance buffer disappeared")?;
        let material = self
            .owner
            .assets
            .iter()
            .find(|row| row.asset_index == batch.asset_index)
            .ok_or("XMAP11 L03 retained diffuse texture disappeared")?;

        (*context).VSSetShader(self.owner.shadow_vertex.as_raw(), std::ptr::null(), 0);
        (*context).PSSetShader(self.owner.shadow_pixel.as_raw(), std::ptr::null(), 0);
        (*context).GSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).HSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).DSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).IASetInputLayout(self.owner.shadow_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        (*context).RSSetState(self.owner.shadow_raster.as_raw());
        (*context).OMSetDepthStencilState(self.owner.shadow_depth.as_raw(), 0);
        let scene = [partition.scene_cb0];
        (*context).VSSetConstantBuffers(0, 1, scene.as_ptr());
        (*context).PSSetConstantBuffers(0, 1, scene.as_ptr());
        for slot in [0u8, 14u8] {
            let sampler = self
                .owner
                .shadow_samplers
                .get(&slot)
                .ok_or("XMAP11 TreeShadow sampler disappeared")?;
            let raw = [sampler.as_raw()];
            (*context).PSSetSamplers(u32::from(slot), 1, raw.as_ptr());
        }
        let diffuse = [material.diffuse.view.as_raw()];
        let mask = [self.owner.tree_mask_view.as_raw()];
        (*context).PSSetShaderResources(0, 1, diffuse.as_ptr());
        (*context).PSSetShaderResources(14, 1, mask.as_ptr());
        let buffers = [geometry.vertex.as_raw(), instances.buffer.as_raw()];
        let strides = [crate::TREE_MESH_VERTEX_STRIDE_BYTES, crate::TREE_INSTANCE_STRIDE_BYTES];
        let offsets = [0u32, 0u32];
        (*context).IASetVertexBuffers(
            0,
            2,
            buffers.as_ptr(),
            strides.as_ptr() as *mut _,
            offsets.as_ptr() as *mut _,
        );
        (*context).IASetIndexBuffer(geometry.index.as_raw(), DXGI_FORMAT_R16_UINT, 0);
        match draw {
            crate::L03DrawArguments::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                start_index,
                base_vertex,
                start_instance,
            } => (*context).DrawIndexedInstanced(
                *index_count_per_instance,
                *instance_count,
                *start_index,
                *base_vertex,
                *start_instance,
            ),
            _ => return Err("XMAP11 TreeShadow requires DrawIndexedInstanced".into()),
        }
        let null = [std::ptr::null_mut(); 15];
        (*context).PSSetShaderResources(0, null.len() as u32, null.as_ptr());
        Ok(())
    }
}

fn tree_batch_for_submission<'a>(
    frame: &'a PreparedTreeFrame,
    partition_mask: u8,
    source_row_ordinal: u32,
    selected_mesh_ordinal: u32,
) -> Result<(&'a crate::PreparedTreeBatch, &'a crate::TreePreparedAsset)> {
    let batch = frame
        .ordered_main_draw_batch_indices
        .iter()
        .map(|index| &frame.selected_batches[*index as usize])
        .find(|batch| {
            let asset = &frame.assets[batch.asset_index as usize];
            batch.partition_mask == partition_mask
                && batch.source_row_ordinal == source_row_ordinal
                && asset.selected_mesh_ordinal == selected_mesh_ordinal
        })
        .ok_or("XMAP11 L03 submission has no admitted MainScene batch")?;
    Ok((batch, &frame.assets[batch.asset_index as usize]))
}

#[cfg(windows)]
pub unsafe fn execute_xmap11_tree_frame(
    device: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    frame: &PreparedTreeFrame,
    schedule: &AssignedStage,
    authority: &TreeNativeExecutionAuthority,
    owner: &NativeXmap11ResourceOwner,
    bindings: &NativeTreeMainSceneBindings,
) -> Result<TreeNativeExecutionReceipt> {
    use std::ptr;
    use winapi::shared::dxgiformat::*;
    use winapi::shared::windef::RECT;
    use winapi::um::d3d11::*;
    use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

    frame.validate()?;
    if !owner.matches(device, frame) {
        return Err("XMAP11 MainScene execution requires the exact retained resource owner".into());
    }
    schedule
        .validate()
        .map_err(|error| format!("XMAP11 assigned stage: {error}"))?;
    if schedule.stage_name != XMAP11_TREE_STAGE_NAME
        || schedule.operation_name != XMAP11_TREE_OPERATION_NAME
    {
        return Err("XMAP11 native execution received the wrong assigned stage".into());
    }
    if authority.foundation_authorization_sha256
        != crate::xmap11_foundation_authorization_sha256(frame, schedule)?
    {
        return Err("XMAP11 native execution was not authorized by the exact Trees disposition".into());
    }
    for (value, label) in [
        (&authority.replay_frame_identity_sha256, "tree replay frame"),
        (&authority.viewer_state_sha256, "tree viewer state"),
        (&authority.main_scene_target_sha256, "tree MainScene target"),
        (
            &authority.foundation_authorization_sha256,
            "tree foundation family authorization",
        ),
        (
            &authority.map_copy_receipt_set_sha256,
            "tree map-copy receipt set",
        ),
    ] {
        validate_sha256(value, label)?;
    }
    if authority.viewer_state_sha256 != frame.viewer_state_sha256
        || authority.hour_index != frame.hour_index
        || authority.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
        || authority.map_copy_receipt_set_sha256 != frame.map_copy_receipt_set_sha256
    {
        return Err(
            "XMAP11 native authority does not match the frame-derived viewer/map-copy identity"
                .into(),
        );
    }
    if device.is_null()
        || context.is_null()
        || bindings.color_rtv.is_null()
        || bindings.depth_dsv.is_null()
        || (frame.shader_programs.main.effect_name == crate::XMAP11_TREE_MAIN_EFFECT
            && bindings.pixel_cb2.is_null())
        || (frame.shader_programs.main.effect_name != crate::XMAP11_TREE_MAIN_EFFECT
            && !bindings.pixel_cb2.is_null())
        || bindings.width == 0
        || bindings.height == 0
    {
        return Err("XMAP11 native execution received a null/zero MainScene binding".into());
    }

    let partition_scenes = validate_partition_scene_bindings(frame, bindings)?;
    let shared = validate_shared_resources(frame, authority, bindings)?;
    let assets = validate_asset_resources(frame, bindings)?;
    let ordered_resources_sha256 =
        ordered_resource_set_sha256(frame, authority, &partition_scenes, &shared, &assets)?;
    let foundation = schedule
        .foundation_operation_identity(
            frame.shader_programs.deterministic_sha256.clone(),
            ordered_resources_sha256.clone(),
            authority.main_scene_target_sha256.clone(),
        )
        .map_err(|error| format!("XMAP11 foundation identity: {error}"))?;
    let foundation_sha256 = Sha256::hex(&serde_json::to_vec(&foundation)?);

    let cb1_bytes = tree_cb1_bytes(
        bindings.shadow_map_texture_matrix_f32_bits,
        frame.season_lerp_f32_bits,
        frame.season_column_f32_bits,
        frame.tree_fade_f32_bits,
    );
    let cb1 = create_buffer(device, D3D11_BIND_CONSTANT_BUFFER, &cb1_bytes, "tree CB1")?;

    let render_targets = [bindings.color_rtv];
    (*context).OMSetRenderTargets(1, render_targets.as_ptr(), bindings.depth_dsv);
    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: bindings.width as f32,
        Height: bindings.height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    (*context).RSSetViewports(1, &viewport);
    let scissor = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(bindings.width)?,
        bottom: i32::try_from(bindings.height)?,
    };
    (*context).RSSetScissorRects(1, &scissor);
    (*context).VSSetShader(owner.main_vertex.as_raw(), ptr::null(), 0);
    (*context).PSSetShader(owner.main_pixel.as_raw(), ptr::null(), 0);
    (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).SOSetTargets(0, ptr::null(), ptr::null());
    (*context).IASetInputLayout(owner.main_layout.as_raw());
    (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    (*context).RSSetState(owner.main_raster.as_raw());
    let blend_factor = [1.0f32; 4];
    (*context).OMSetBlendState(owner.main_blend.as_raw(), &blend_factor, u32::MAX);
    (*context).OMSetDepthStencilState(owner.main_depth.as_raw(), 0);
    let tree = [cb1.as_raw()];
    (*context).VSSetConstantBuffers(1, 1, tree.as_ptr());
    (*context).PSSetConstantBuffers(1, 1, tree.as_ptr());
    if !bindings.pixel_cb2.is_null() {
        let pixel_cb2 = [bindings.pixel_cb2];
        (*context).PSSetConstantBuffers(2, 1, pixel_cb2.as_ptr());
    }
    for (slot, sampler) in &owner.main_samplers {
        let raw = [sampler.as_raw()];
        (*context).PSSetSamplers(u32::from(*slot), 1, raw.as_ptr());
    }
    for resource in &bindings.ordered_shared_resources {
        let raw = [resource.view];
        (*context).PSSetShaderResources(u32::from(resource.identity.slot), 1, raw.as_ptr());
    }
    let mask = [owner.tree_mask_view.as_raw()];
    (*context).PSSetShaderResources(14, 1, mask.as_ptr());
    if authority.color_map_second_is_null {
        let null = [ptr::null_mut()];
        (*context).PSSetShaderResources(5, 1, null.as_ptr());
    }

    let mut draw_receipts = Vec::new();
    for (draw_ordinal, batch_index) in frame
        .ordered_main_draw_batch_indices
        .iter()
        .copied()
        .enumerate()
    {
        let batch = &frame.selected_batches[batch_index as usize];
        let partition_scene = partition_scenes
            .get(&batch.partition_mask)
            .ok_or("XMAP11 validated partition SceneCB0 disappeared")?;
        let scene = [partition_scene.scene_cb0];
        (*context).VSSetConstantBuffers(0, 1, scene.as_ptr());
        (*context).PSSetConstantBuffers(0, 1, scene.as_ptr());
        let asset = &frame.assets[batch.asset_index as usize];
        let gpu = owner
            .geometries
            .iter()
            .find(|row| row.asset_index == batch.asset_index)
            .ok_or("XMAP11 retained MainScene geometry disappeared")?;
        let material = assets
            .get(&batch.asset_index)
            .ok_or("XMAP11 validated asset GPU resources disappeared")?;
        for resource in [&material.diffuse, &material.normal] {
            let raw = [resource.view];
            (*context).PSSetShaderResources(u32::from(resource.identity.slot), 1, raw.as_ptr());
        }
        let instances = owner
            .instances
            .iter()
            .find(|row| {
                row.source_row_ordinal == batch.source_row_ordinal
                    && row.selected_mesh_ordinal == asset.selected_mesh_ordinal
                    && row.partition_mask == batch.partition_mask
            })
            .ok_or("XMAP11 retained MainScene instances disappeared")?;
        let buffers = [gpu.vertex.as_raw(), instances.buffer.as_raw()];
        let strides = [asset.mesh.vertex_stride_bytes, 24];
        let offsets = [0u32, 0u32];
        (*context).IASetVertexBuffers(
            0,
            buffers.len() as u32,
            buffers.as_ptr(),
            strides.as_ptr(),
            offsets.as_ptr(),
        );
        (*context).IASetIndexBuffer(gpu.index.as_raw(), DXGI_FORMAT_R16_UINT, 0);
        (*context).DrawIndexedInstanced(
            u32::try_from(asset.mesh.index_count)?,
            batch.instance_count,
            0,
            0,
            0,
        );

        let executor_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&(
            XMAP11_TREE_NATIVE_EXECUTION_SCHEMA,
            schedule.assignment_sha256.as_str(),
            authority.hour_index,
            draw_ordinal,
            batch.source_row_ordinal,
            batch.partition_mask,
            asset.asset_index,
            asset.mesh_source.payload_sha256.as_str(),
            Sha256::hex(&batch.instance_bytes),
            ordered_resources_sha256.as_str(),
            authority.main_scene_target_sha256.as_str(),
        ))?);
        let execution = TreeActualDrawExecution {
            hour_index: authority.hour_index,
            draw_ordinal: u32::try_from(draw_ordinal)?,
            operation_id: XMAP11_TREE_OPERATION_NAME.into(),
            content_manifest_sha256: frame.content_manifest_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            program_sha256: frame.shader_programs.deterministic_sha256.clone(),
            ordered_resources_sha256: ordered_resources_sha256.clone(),
            foundation_content_operation_sha256: foundation_sha256.clone(),
            foundation_authorization_sha256: authority
                .foundation_authorization_sha256
                .clone(),
            viewer_state_sha256: authority.viewer_state_sha256.clone(),
            main_scene_target_sha256: authority.main_scene_target_sha256.clone(),
            map_copy_receipt_set_sha256: authority.map_copy_receipt_set_sha256.clone(),
            prepared_frame_sha256: frame.prepared_frame_sha256.clone(),
            source_row_ordinal: batch.source_row_ordinal,
            render_view_mask: batch.partition_mask,
            executor_draw_receipt_sha256,
        };
        draw_receipts.push(crate::tree_target_bound_draw_receipt(
            frame, schedule, &execution,
        )?);
    }

    let nulls = [ptr::null_mut(); 16];
    (*context).PSSetShaderResources(0, nulls.len() as u32, nulls.as_ptr());
    let mut receipt = TreeNativeExecutionReceipt {
        schema: XMAP11_TREE_NATIVE_EXECUTION_SCHEMA.into(),
        operation_id: XMAP11_TREE_OPERATION_NAME.into(),
        stage_name: XMAP11_TREE_STAGE_NAME.into(),
        assigned_ordinal: schedule.assigned_ordinal,
        topology_sha256: schedule.topology_sha256.clone(),
        assignment_sha256: schedule.assignment_sha256.clone(),
        hour_index: authority.hour_index,
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
        prepared_frame_sha256: frame.prepared_frame_sha256.clone(),
        program_sha256: frame.shader_programs.deterministic_sha256.clone(),
        ordered_resources_sha256,
        foundation_content_operation_sha256: foundation_sha256,
        foundation_authorization_sha256: authority
            .foundation_authorization_sha256
            .clone(),
        viewer_state_sha256: frame.viewer_state_sha256.clone(),
        map_copy_receipt_set_sha256: frame.map_copy_receipt_set_sha256.clone(),
        main_scene_target_sha256: authority.main_scene_target_sha256.clone(),
        issued_draw_count: u32::try_from(draw_receipts.len())?,
        ordered_draw_receipts: draw_receipts,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    receipt.validate_against(frame, schedule, authority)?;
    Ok(receipt)
}

#[cfg(windows)]
fn validate_shared_resources<'a>(
    frame: &PreparedTreeFrame,
    authority: &TreeNativeExecutionAuthority,
    bindings: &'a NativeTreeMainSceneBindings,
) -> Result<BTreeMap<u8, &'a NativeTreeShaderResource>> {
    const LOW_GFX_REQUIRED: [(&str, u8); 8] = [
        ("TintMap", 2),
        ("SeasonMap", 3),
        ("ColorMap", 4),
        ("ShadowMap", 8),
        ("GradientBorderChannel1", 12),
        ("GradientBorderChannel2", 13),
        ("ProvinceSecondaryColorMap", 15),
        ("TreeMaskTexture", 14),
    ];
    const FULL_GFX_ONLY: [(&str, u8); 4] = [
        ("SnowMudData", 6),
        ("EnvironmentMap", 9),
        ("LightIndexMap", 10),
        ("LightDataMap", 11),
    ];
    let mut by_slot = BTreeMap::new();
    for resource in &bindings.ordered_shared_resources {
        resource.identity.validate()?;
        if resource.view.is_null() || by_slot.insert(resource.identity.slot, resource).is_some() {
            return Err("XMAP11 shared GPU resource is null or repeats a slot".into());
        }
    }
    // TreeMask is generated and bound internally, never accepted from caller.
    if by_slot.contains_key(&14) {
        return Err("XMAP11 caller attempted to replace the generated TreeMaskTexture".into());
    }
    let required = LOW_GFX_REQUIRED
        .into_iter()
        .chain(
            (frame.shader_programs.main.effect_name == crate::XMAP11_TREE_MAIN_EFFECT)
                .then_some(FULL_GFX_ONLY)
                .into_iter()
                .flatten(),
        );
    for (semantic, slot) in required.filter(|(_, slot)| *slot != 14) {
        if by_slot
            .get(&slot)
            .is_none_or(|row| row.identity.semantic != semantic)
        {
            return Err(
                format!("XMAP11 shared resource {semantic} is missing from t{slot}").into(),
            );
        }
    }
    match (authority.color_map_second_is_null, by_slot.get(&5)) {
        (true, None) => {}
        (false, Some(row)) if row.identity.semantic == "ColorMapSecond" => {}
        _ => return Err("XMAP11 ColorMapSecond null/present authority is inconsistent".into()),
    }
    let mut allowed_slots = LOW_GFX_REQUIRED
        .into_iter()
        .filter_map(|(_, slot)| (slot != 14).then_some(slot))
        .collect::<std::collections::BTreeSet<_>>();
    if frame.shader_programs.main.effect_name == crate::XMAP11_TREE_MAIN_EFFECT {
        allowed_slots.extend(FULL_GFX_ONLY.into_iter().map(|(_, slot)| slot));
    }
    if !authority.color_map_second_is_null {
        allowed_slots.insert(5);
    }
    if by_slot.keys().any(|slot| !allowed_slots.contains(slot)) {
        return Err("XMAP11 shared GPU resource set contains an unowned shader slot".into());
    }
    let tint =
        used_content_by_game_path(&frame.ordered_used_content, crate::TREE_TINT_TEXTURE_SOURCE)
            .ok_or("XMAP11 frame has no TintMap content")?;
    let season = used_content_by_game_path(
        &frame.ordered_used_content,
        crate::TREE_SEASON_TEXTURE_SOURCE,
    )
    .ok_or("XMAP11 frame has no SeasonMap content")?;
    if by_slot[&2].identity.content_payload_sha256 != tint.payload_sha256
        || by_slot[&3].identity.content_payload_sha256 != season.payload_sha256
    {
        return Err("XMAP11 TintMap/SeasonMap GPU resources drift from effective VFS".into());
    }
    Ok(by_slot)
}

fn used_content_by_game_path<'a>(
    ordered_used_content: &'a [UsedContentKey],
    game_relative_path: &str,
) -> Option<&'a UsedContentKey> {
    let normalized = game_relative_path.replace('\\', "/").to_ascii_lowercase();
    ordered_used_content
        .iter()
        .find(|row| row.normalized_game_relative_path == normalized)
}

#[cfg(windows)]
fn validate_partition_scene_bindings<'a>(
    frame: &PreparedTreeFrame,
    bindings: &'a NativeTreeMainSceneBindings,
) -> Result<BTreeMap<u8, &'a NativeTreePartitionSceneBinding>> {
    if bindings.ordered_partition_scene_cb0.len() != frame.viewer.ordered_partitions.len() {
        return Err(
            "XMAP11 partition SceneCB0 set does not exactly cover the viewer partitions".into(),
        );
    }
    let mut out = BTreeMap::new();
    for (binding, partition) in bindings
        .ordered_partition_scene_cb0
        .iter()
        .zip(&frame.viewer.ordered_partitions)
    {
        validate_sha256(
            &binding.scene_cb0_payload_sha256,
            "tree partition SceneCB0 payload",
        )?;
        if binding.scene_cb0.is_null()
            || binding.render_view_mask != partition.render_view_mask
            || binding.map_copy_offset_f32_bits != partition.map_copy_offset_f32_bits
            || binding.viewer_state_sha256 != frame.viewer_state_sha256
            || out.insert(binding.render_view_mask, binding).is_some()
        {
            return Err(
                "XMAP11 partition SceneCB0 order, offset, viewer authority, or pointer is invalid"
                    .into(),
            );
        }
    }
    Ok(out)
}

#[cfg(windows)]
fn validate_asset_resources<'a>(
    frame: &PreparedTreeFrame,
    bindings: &'a NativeTreeMainSceneBindings,
) -> Result<BTreeMap<u32, &'a NativeTreeAssetResources>> {
    let mut out = BTreeMap::new();
    for resources in &bindings.ordered_asset_resources {
        resources.diffuse.identity.validate()?;
        resources.normal.identity.validate()?;
        if resources.diffuse.view.is_null()
            || resources.normal.view.is_null()
            || resources.diffuse.identity.semantic != "DiffuseMap"
            || resources.diffuse.identity.slot != 0
            || resources.normal.identity.semantic != "NormalMap"
            || resources.normal.identity.slot != 1
            || out.insert(resources.asset_index, resources).is_some()
        {
            return Err("XMAP11 asset GPU resource set is malformed or duplicated".into());
        }
        let asset = frame
            .assets
            .get(resources.asset_index as usize)
            .ok_or("XMAP11 GPU resource set names an absent asset")?;
        if asset.ordered_material_textures.len() < 2
            || resources.diffuse.identity.content_payload_sha256
                != asset.ordered_material_textures[0].payload_sha256
            || resources.normal.identity.content_payload_sha256
                != asset.ordered_material_textures[1].payload_sha256
        {
            return Err("XMAP11 material GPU resources drift from selected mesh content".into());
        }
    }
    let used_assets = frame
        .ordered_main_draw_batch_indices
        .iter()
        .map(|index| frame.selected_batches[*index as usize].asset_index)
        .collect::<BTreeSet<_>>();
    if used_assets != out.keys().copied().collect() {
        return Err(
            "XMAP11 material GPU resource set does not exactly cover visible assets".into(),
        );
    }
    Ok(out)
}

#[cfg(windows)]
fn ordered_resource_set_sha256(
    frame: &PreparedTreeFrame,
    authority: &TreeNativeExecutionAuthority,
    partition_scenes: &BTreeMap<u8, &NativeTreePartitionSceneBinding>,
    shared: &BTreeMap<u8, &NativeTreeShaderResource>,
    assets: &BTreeMap<u32, &NativeTreeAssetResources>,
) -> Result<String> {
    let partition_rows = partition_scenes
        .values()
        .map(|row| {
            (
                row.render_view_mask,
                row.map_copy_offset_f32_bits,
                row.viewer_state_sha256.as_str(),
                row.scene_cb0_payload_sha256.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let shared_rows = shared.values().map(|row| &row.identity).collect::<Vec<_>>();
    let asset_rows = assets
        .iter()
        .map(|(index, rows)| (*index, &rows.diffuse.identity, &rows.normal.identity))
        .collect::<Vec<_>>();
    Ok(Sha256::hex(&serde_json::to_vec(&(
        partition_rows,
        shared_rows,
        asset_rows,
        authority.color_map_second_is_null,
        frame.tree_mask.payload_sha256.as_str(),
    ))?))
}

#[cfg(windows)]
unsafe fn create_vertex_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::ID3D11VertexShader;
    let mut raw = ptr::null_mut::<ID3D11VertexShader>();
    native_d3d11_hr(
        (*device).CreateVertexShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            ptr::null_mut(),
            &mut raw,
        ),
        "CreateVertexShader(XMAP11 tree)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 tree VS")
}

#[cfg(windows)]
unsafe fn create_pixel_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::ID3D11PixelShader;
    let mut raw = ptr::null_mut::<ID3D11PixelShader>();
    native_d3d11_hr(
        (*device).CreatePixelShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            ptr::null_mut(),
            &mut raw,
        ),
        "CreatePixelShader(XMAP11 tree)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 tree PS")
}

#[cfg(windows)]
fn input(
    semantic: *const i8,
    semantic_index: u32,
    format: winapi::shared::dxgiformat::DXGI_FORMAT,
    input_slot: u32,
    offset: u32,
    class: winapi::um::d3d11::D3D11_INPUT_CLASSIFICATION,
    step: u32,
) -> winapi::um::d3d11::D3D11_INPUT_ELEMENT_DESC {
    winapi::um::d3d11::D3D11_INPUT_ELEMENT_DESC {
        SemanticName: semantic,
        SemanticIndex: semantic_index,
        Format: format,
        InputSlot: input_slot,
        AlignedByteOffset: offset,
        InputSlotClass: class,
        InstanceDataStepRate: step,
    }
}

#[cfg(windows)]
unsafe fn create_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bind_flags: u32,
    bytes: &[u8],
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::*;
    if bytes.is_empty() {
        return Err(format!("XMAP11 {role} payload is empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateBuffer(&desc, &initial, &mut raw),
        &format!("CreateBuffer(XMAP11 {role})"),
    )?;
    D3dPtr::from_raw(raw, role)
}

#[cfg(windows)]
unsafe fn create_tree_input_layout(
    device: *mut winapi::um::d3d11::ID3D11Device,
    vertex_bytecode: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ffi::CString;
    use winapi::shared::dxgiformat::*;
    use winapi::um::d3d11::*;
    let position = CString::new("POSITION")?;
    let texcoord = CString::new("TEXCOORD")?;
    let pv = D3D11_INPUT_PER_VERTEX_DATA;
    let pi = D3D11_INPUT_PER_INSTANCE_DATA;
    let elements = [
        input(position.as_ptr(), 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, pv, 0),
        input(texcoord.as_ptr(), 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 12, pv, 0),
        input(texcoord.as_ptr(), 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 24, pv, 0),
        input(texcoord.as_ptr(), 2, DXGI_FORMAT_R32G32_FLOAT, 0, 40, pv, 0),
        input(texcoord.as_ptr(), 3, DXGI_FORMAT_R32G32_FLOAT, 0, 48, pv, 0),
        input(texcoord.as_ptr(), 4, DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 0, pi, 1),
        input(texcoord.as_ptr(), 5, DXGI_FORMAT_R32G32_FLOAT, 1, 16, pi, 1),
    ];
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateInputLayout(
            elements.as_ptr(),
            elements.len() as u32,
            vertex_bytecode.as_ptr() as *const _,
            vertex_bytecode.len(),
            &mut raw,
        ),
        "CreateInputLayout(XMAP11 TreeShadow)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 retained TreeShadow input layout")
}

#[cfg(windows)]
unsafe fn create_tree_bmp_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    content: &UsedContentKey,
    semantic: &str,
    slot: u8,
) -> Result<OwnedTreeTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::*;
    let admitted = resolver.read_admitted_used_content(content)?;
    if Sha256::hex(&admitted) != content.payload_sha256 {
        return Err(format!("XMAP11 {semantic} differs from admitted effective-VFS bytes").into());
    }
    let decoded = crate::tree_files::tree_texture_from_admitted_bytes(
        &content.normalized_game_relative_path,
        &admitted,
    )?;
    if decoded.bgra8.len()
        != usize::try_from(decoded.width)?
            .checked_mul(usize::try_from(decoded.height)?)
            .and_then(|value| value.checked_mul(4))
            .ok_or("XMAP11 BMP texture extent overflowed")?
    {
        return Err(format!("XMAP11 {semantic} decoded payload is incomplete").into());
    }
    let descriptor_sha256 = Sha256::hex(&serde_json::to_vec(&(
        "hoi4.xmap11.tree-bmp-texture.a729.v1",
        content,
        decoded.width,
        decoded.height,
        decoded.dxgi_format,
        Sha256::hex(&decoded.bgra8),
    ))?);
    let desc = D3D11_TEXTURE2D_DESC {
        Width: decoded.width,
        Height: decoded.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: decoded.dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: decoded.bgra8.as_ptr() as *const _,
        SysMemPitch: decoded.width.checked_mul(4).ok_or("XMAP11 BMP pitch overflowed")?,
        SysMemSlicePitch: u32::try_from(decoded.bgra8.len())?,
    };
    let mut texture_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut texture_raw),
        &format!("CreateTexture2D(XMAP11 {semantic})"),
    )?;
    let texture = D3dPtr::from_raw(texture_raw, "XMAP11 retained BMP texture")?;
    let mut view_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, std::ptr::null(), &mut view_raw),
        &format!("CreateShaderResourceView(XMAP11 {semantic})"),
    )?;
    Ok(OwnedTreeTexture {
        identity: TreeGpuResourceIdentity {
            semantic: semantic.into(),
            slot,
            content_payload_sha256: content.payload_sha256.clone(),
            descriptor_sha256,
        },
        _texture: texture,
        view: D3dPtr::from_raw(view_raw, "XMAP11 retained BMP SRV")?,
    })
}

#[cfg(windows)]
unsafe fn create_default_shadow_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let stencil = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_ALWAYS,
    };
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 1,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_LESS,
        StencilEnable: 0,
        StencilReadMask: u8::MAX,
        StencilWriteMask: u8::MAX,
        FrontFace: stencil,
        BackFace: stencil,
    };
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut raw),
        "CreateDepthStencilState(XMAP11 TreeShadow default)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 retained TreeShadow depth state")
}

#[cfg(windows)]
unsafe fn create_default_shadow_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 0,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut raw),
        "CreateRasterizerState(XMAP11 TreeShadow default)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 retained TreeShadow rasterizer state")
}

#[cfg(windows)]
unsafe fn create_tree_mask(
    device: *mut winapi::um::d3d11::ID3D11Device,
    frame: &PreparedTreeFrame,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::*;
    frame.tree_mask.validate()?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: frame.tree_mask.width,
        Height: frame.tree_mask.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: frame.tree_mask.dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: frame.tree_mask.pixels.as_ptr() as *const _,
        SysMemPitch: frame.tree_mask.width,
        SysMemSlicePitch: u32::try_from(frame.tree_mask.pixels.len())?,
    };
    let mut texture = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut texture),
        "CreateTexture2D(XMAP11 TreeMaskTexture)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP11 TreeMaskTexture")?;
    let mut view = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP11 TreeMaskTexture)",
    )?;
    Ok((
        texture,
        D3dPtr::from_raw(view, "XMAP11 TreeMaskTexture SRV")?,
    ))
}

fn tree_cb1_bytes(
    shadow_matrix: [u32; 16],
    season_lerp_f32_bits: u32,
    season_column_f32_bits: u32,
    tree_fade_f32_bits: u32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(80);
    for bits in shadow_matrix {
        bytes.extend_from_slice(&bits.to_le_bytes());
    }
    bytes.extend_from_slice(&season_lerp_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&season_column_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&tree_fade_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes
}

#[cfg(windows)]
unsafe fn create_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    pipeline: &TreePipelineStateDescriptor,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::mem;
    use std::ptr;
    use winapi::um::d3d11::*;
    pipeline.validate()?;
    let mut desc: D3D11_BLEND_DESC = mem::zeroed();
    desc.AlphaToCoverageEnable = 0;
    desc.IndependentBlendEnable = 0;
    desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: i32::from(pipeline.blend_enable),
        SrcBlend: match pipeline.source_blend.as_str() {
            "SRC_ALPHA" => D3D11_BLEND_SRC_ALPHA,
            _ => return Err("XMAP11 effective source blend has no D3D11 translation".into()),
        },
        DestBlend: match pipeline.dest_blend.as_str() {
            "INV_SRC_ALPHA" => D3D11_BLEND_INV_SRC_ALPHA,
            _ => return Err("XMAP11 effective destination blend has no D3D11 translation".into()),
        },
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_SRC_ALPHA,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: match pipeline.write_mask.as_str() {
            "RED|GREEN|BLUE" => 0x07,
            _ => return Err("XMAP11 effective color write mask has no D3D11 translation".into()),
        },
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut raw),
        "CreateBlendState(XMAP11 tree)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 tree blend")
}

#[cfg(windows)]
unsafe fn create_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    pipeline: &TreePipelineStateDescriptor,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::mem;
    use std::ptr;
    use winapi::um::d3d11::*;
    pipeline.validate()?;
    let mut desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
    desc.DepthEnable = i32::from(pipeline.depth_enable);
    desc.DepthWriteMask = if pipeline.depth_write {
        D3D11_DEPTH_WRITE_MASK_ALL
    } else {
        D3D11_DEPTH_WRITE_MASK_ZERO
    };
    desc.DepthFunc = match pipeline.depth_func.as_str() {
        "LESS_EQUAL" => D3D11_COMPARISON_LESS_EQUAL,
        _ => return Err("XMAP11 effective depth comparison has no D3D11 translation".into()),
    };
    desc.StencilEnable = 0;
    desc.StencilReadMask = u8::MAX;
    desc.StencilWriteMask = u8::MAX;
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut raw),
        "CreateDepthStencilState(XMAP11 tree)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 tree depth")
}

#[cfg(windows)]
unsafe fn create_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    pipeline: &TreePipelineStateDescriptor,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::*;
    pipeline.validate()?;
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: match pipeline.fill_mode.as_str() {
            "SOLID" => D3D11_FILL_SOLID,
            _ => return Err("XMAP11 effective fill mode has no D3D11 translation".into()),
        },
        CullMode: match pipeline.cull_mode.as_str() {
            "BACK" => D3D11_CULL_BACK,
            _ => return Err("XMAP11 effective cull mode has no D3D11 translation".into()),
        },
        FrontCounterClockwise: i32::from(pipeline.front_counter_clockwise),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: i32::from(pipeline.depth_clip_enable),
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut raw),
        "CreateRasterizerState(XMAP11 tree)",
    )?;
    D3dPtr::from_raw(raw, "XMAP11 tree rasterizer")
}

#[cfg(windows)]
unsafe fn create_samplers(
    device: *mut winapi::um::d3d11::ID3D11Device,
    samplers: &[TreeSamplerStateDescriptor],
) -> Result<
    Vec<(
        u8,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::*;
    crate::tree_runtime_selection::validate_tree_sampler_descriptors(samplers)?;
    samplers
        .iter()
        .map(|sampler| {
            let comparison = sampler.resource_type == "Shadow";
            let filter = match (
                comparison,
                sampler.min_filter.as_str(),
                sampler.mag_filter.as_str(),
                sampler.mip_filter.as_str(),
            ) {
                (true, "Linear", "Linear", "Linear") => D3D11_FILTER_COMPARISON_MIN_MAG_MIP_LINEAR,
                (false, "Point", "Point", "Point") => D3D11_FILTER_MIN_MAG_MIP_POINT,
                (false, "Linear", "Linear", "Linear") => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                _ => {
                    return Err(format!(
                        "XMAP11 effective sampler {} has an unproven mixed D3D11 filter",
                        sampler.name
                    )
                    .into())
                }
            };
            let address = |value: &str| -> Result<D3D11_TEXTURE_ADDRESS_MODE> {
                match value {
                    "Wrap" => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
                    "Clamp" => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
                    _ => Err(format!(
                        "XMAP11 effective sampler {} has an unproven address mode",
                        sampler.name
                    )
                    .into()),
                }
            };
            let desc = D3D11_SAMPLER_DESC {
                Filter: filter,
                AddressU: address(&sampler.address_u)?,
                AddressV: address(&sampler.address_v)?,
                AddressW: address(&sampler.address_u)?,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: if comparison {
                    D3D11_COMPARISON_LESS_EQUAL
                } else {
                    D3D11_COMPARISON_NEVER
                },
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*device).CreateSamplerState(&desc, &mut raw),
                &format!("CreateSamplerState(XMAP11 t{})", sampler.shader_register),
            )?;
            Ok((
                u8::try_from(sampler.shader_register)?,
                D3dPtr::from_raw(raw, "XMAP11 tree sampler")?,
            ))
        })
        .collect()
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

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

    fn source_section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let start = source.find(start).expect("source section start");
        let end = source[start..]
            .find(end)
            .map(|offset| start + offset)
            .expect("source section end");
        &source[start..end]
    }

    #[test]
    fn retained_owner_reuses_immutable_main_and_shadow_resources() {
        let source = include_str!("xmap11_d3d_handoff.rs");
        let runtime = include_str!("tree_runtime_selection.rs");
        let resource_key = source_section(
            runtime,
            "fn computed_native_resource_closure_sha256",
            "fn computed_static_payload_sha256",
        );
        for static_input in [
            "&self.shader_programs",
            "&self.tree_mask",
            "&self.ordered_used_content",
            "used_assets",
            "retained_instances",
        ] {
            assert!(resource_key.contains(static_input));
        }
        for dynamic_input in [
            "self.hour_index",
            "self.replay_frame_identity_sha256",
            "self.viewer_state_sha256",
            "self.tree_fade_f32_bits",
        ] {
            assert!(!resource_key.contains(dynamic_input));
        }
        let resource_key_accessor = source_section(
            runtime,
            "pub fn native_resource_closure_sha256",
            "fn computed_prepared_frame_sha256",
        );
        assert!(resource_key_accessor.contains("static_payload.native_resource_closure_sha256"));
        assert!(!resource_key_accessor.contains("serde_json"));
        let dynamic_reseal = source_section(
            runtime,
            "pub(crate) fn reseal_dynamic_authority",
            "pub fn validate(&self)",
        );
        assert!(dynamic_reseal.contains("computed_prepared_frame_sha256"));
        assert!(!dynamic_reseal.contains("computed_native_resource_closure_sha256"));
        assert!(!dynamic_reseal.contains("computed_static_payload_sha256"));
        let original = Sha256::hex(&serde_json::to_vec(&("program", "mask", "instances")).unwrap());
        let changed = Sha256::hex(&serde_json::to_vec(&("program", "changed-mask", "instances")).unwrap());
        assert_ne!(original, changed, "an immutable resource-key input must invalidate reuse");

        let owner = source_section(
            source,
            "pub struct NativeXmap11ResourceOwner",
            "pub struct NativeXmap11LightDepthCaster",
        );
        assert!(owner.contains("resource_closure_sha256"));
        assert!(!owner.contains("prepared_frame_sha256: String"));
        for retained in [
            "main_vertex:",
            "main_pixel:",
            "main_layout:",
            "main_blend:",
            "main_depth:",
            "main_raster:",
            "main_samplers:",
            "geometries:",
            "instances:",
        ] {
            assert!(owner.contains(retained), "missing retained {retained}");
        }

        let execute = source_section(
            source,
            "pub unsafe fn execute_xmap11_tree_frame",
            "fn validate_shared_resources",
        );
        assert!(execute.contains("owner.main_vertex.as_raw()"));
        assert!(execute.contains("owner.tree_mask_view.as_raw()"));
        assert!(!execute.contains("create_vertex_shader(device"));
        assert!(!execute.contains("create_tree_mask(device"));
        assert!(!execute.contains("\"tree instances\""));
    }

    #[test]
    fn native_execution_recomputes_complete_foundation_authorization() {
        let source = include_str!("xmap11_d3d_handoff.rs");
        let execute = source_section(
            source,
            "pub unsafe fn execute_xmap11_tree_frame",
            "fn validate_shared_resources",
        );
        assert!(execute.contains("authority.foundation_authorization_sha256"));
        assert!(execute.contains("xmap11_foundation_authorization_sha256(frame, schedule)"));

        let adapter = include_str!("production_features/xmap11.rs");
        assert!(adapter.contains("xmap11_foundation_disposition(&frame, assigned)"));
        let app = include_str!("../../replay_app/src/app.rs");
        assert!(app.contains("row.disposition == map_viewer::xmap11_foundation_disposition"));
        assert!(app.contains("foundation_authorization_sha256:"));
    }

    #[test]
    fn production_tree_preparation_has_no_invented_date_or_seasons_dependency() {
        let adapter = include_str!("production_features/xmap11.rs");
        let runtime = include_str!("tree_runtime_selection.rs");
        let replay_date_parser = ["replay_", "month_day"].concat();
        let seasons_source = ["XMAP11_TREE_", "SEASONS_SOURCE"].concat();
        assert!(!adapter.contains(&replay_date_parser));
        assert!(!runtime.contains(&seasons_source));
        assert!(runtime.contains("season_lerp_f32_bits: 0.0f32.to_bits()"));
        assert!(runtime.contains("season_column_f32_bits: 0.0f32.to_bits()"));
    }

    #[test]
    fn tree_shadow_draw_clears_all_unused_programmable_stages() {
        let source = include_str!("xmap11_d3d_handoff.rs");
        let draw = source_section(
            source,
            "unsafe fn draw_light_depth_submission",
            "fn tree_batch_for_submission",
        );
        for stage in ["GSSetShader", "HSSetShader", "DSSetShader"] {
            assert!(
                draw.contains(&format!("{stage}(std::ptr::null_mut()")),
                "TreeShadow omitted explicit {stage} clearing"
            );
        }
    }

    #[test]
    fn mixed_case_engine_texture_constants_find_normalized_content_rows() {
        let rows = vec![
            content(&crate::TREE_TINT_TEXTURE_SOURCE.to_ascii_lowercase()),
            content(&crate::TREE_SEASON_TEXTURE_SOURCE.to_ascii_lowercase()),
        ];
        assert_eq!(
            used_content_by_game_path(&rows, crate::TREE_TINT_TEXTURE_SOURCE),
            Some(&rows[0])
        );
        assert_eq!(
            used_content_by_game_path(&rows, crate::TREE_SEASON_TEXTURE_SOURCE),
            Some(&rows[1])
        );
    }

    #[test]
    fn tree_gpu_resource_identity_requires_pointer_free_hashes() {
        let valid = TreeGpuResourceIdentity {
            semantic: "TintMap".into(),
            slot: 2,
            content_payload_sha256: "1".repeat(64),
            descriptor_sha256: "2".repeat(64),
        };
        valid.validate().unwrap();

        let mut invalid = valid;
        invalid.descriptor_sha256 = "not-a-hash".into();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn tree_cb1_is_exact_80_byte_matrix_and_scalar_layout() {
        let matrix = std::array::from_fn(|index| index as u32);
        let bytes = tree_cb1_bytes(matrix, 4.0f32.to_bits(), 5.0f32.to_bits(), 3.0f32.to_bits());
        assert_eq!(bytes.len(), 80);
        assert_eq!(&bytes[64..68], &4.0f32.to_bits().to_le_bytes());
        assert_eq!(&bytes[68..72], &5.0f32.to_bits().to_le_bytes());
        assert_eq!(&bytes[72..76], &3.0f32.to_bits().to_le_bytes());
        assert_eq!(&bytes[76..80], &[0; 4]);
    }

    #[test]
    fn zero_draw_native_receipt_is_still_target_and_program_bound() {
        let mut receipt = TreeNativeExecutionReceipt {
            schema: XMAP11_TREE_NATIVE_EXECUTION_SCHEMA.into(),
            operation_id: XMAP11_TREE_OPERATION_NAME.into(),
            stage_name: XMAP11_TREE_STAGE_NAME.into(),
            assigned_ordinal: 7,
            topology_sha256: "0".repeat(64),
            assignment_sha256: "f".repeat(64),
            hour_index: 123,
            replay_frame_identity_sha256: "e".repeat(64),
            content_manifest_sha256: "a".repeat(64),
            prepared_frame_sha256: "1".repeat(64),
            program_sha256: "2".repeat(64),
            ordered_resources_sha256: "3".repeat(64),
            foundation_content_operation_sha256: "b".repeat(64),
            foundation_authorization_sha256: "6".repeat(64),
            viewer_state_sha256: "c".repeat(64),
            map_copy_receipt_set_sha256: "d".repeat(64),
            main_scene_target_sha256: "4".repeat(64),
            issued_draw_count: 0,
            ordered_draw_receipts: Vec::new(),
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt).unwrap());
        receipt.validate().unwrap();

        receipt.main_scene_target_sha256 = "5".repeat(64);
        assert!(receipt.validate().is_err());
    }
}
