
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{
    load_map_furniture_sprite_assets, GameFileResolver, MapFurnitureGuiGeometry,
    MapFurnitureTarget, PreparedMapFurniturePacket, ProjectedMapFurnitureIcon,
    VP_STRIP_FRAME_COUNT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use winapi::shared::dxgiformat::{DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM};
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11InputLayout, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC,
    D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_NEVER, D3D11_CPU_ACCESS_WRITE,
    D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR, D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
    D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::render::{compile_shader, create_immutable_texture};
use crate::Result;

pub(crate) const MAP_SCENE_QUAD_BACKEND_SCHEMA: &str =
    "hoi4.replay.map-scene-textured-quad-backend.v2";
pub(crate) const MAP_FURNITURE_EXECUTION_SCHEMA: &str =
    "hoi4.replay.xmap21-map-furniture-execution.v5";
const VP_RESOURCE_BUCKETS: [u8; 2] = [2, 3];
const COMBAT_BUCKETS: [u8; 3] = [9, 10, 11];
const RAID_BUCKETS: [u8; 1] = [29];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MapSceneTargetKind {
    MainScene,
    FinalImage,
}

pub(crate) struct AuthenticatedMapSceneTarget {
    pub(crate) kind: MapSceneTargetKind,
    pub(crate) target_identity_sha256: String,
    pub(crate) render_target_view: *mut ID3D11RenderTargetView,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) dxgi_format: u32,
    pub(crate) sample_count: u32,
    pub(crate) sample_quality: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct NeutralMapSceneQuadVertex {
    pub(crate) position_px: [f32; 2],
    pub(crate) uv: [f32; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct NeutralMapSceneQuad {
    pub(crate) texture_role: String,
    pub(crate) vertices: [NeutralMapSceneQuadVertex; 4],
}

pub(crate) struct NativeMapSceneTextureBinding {
    pub(crate) role: String,
    pub(crate) descriptor_sha256: String,
    pub(crate) shader_resource_view: *mut ID3D11ShaderResourceView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MapSceneQuadExecutionReceipt {
    pub(crate) schema: String,
    pub(crate) backend_identity_sha256: String,
    pub(crate) target_kind: MapSceneTargetKind,
    pub(crate) target_identity_sha256: String,
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) target_dxgi_format: u32,
    pub(crate) target_sample_count: u32,
    pub(crate) target_sample_quality: u32,
    pub(crate) batch_identity_sha256: String,
    pub(crate) texture_descriptor_set_sha256: String,
    pub(crate) ordered_quads_sha256: String,
    pub(crate) submitted_quad_count: usize,
    pub(crate) masked_flag_quad_count: usize,
    pub(crate) submitted_draw_count: usize,
    pub(crate) deterministic_sha256: String,
}

impl MapSceneQuadExecutionReceipt {
    fn validate(&self) -> Result<()> {
        if self.schema != MAP_SCENE_QUAD_BACKEND_SCHEMA
            || self.target_width == 0
            || self.target_height == 0
            || self.target_sample_count == 0
            || self.masked_flag_quad_count > self.submitted_quad_count
            || self.submitted_draw_count > self.submitted_quad_count + self.masked_flag_quad_count
        {
            return Err("XMAP21 neutral quad receipt body is invalid".into());
        }
        for (value, role) in [
            (&self.backend_identity_sha256, "quad backend"),
            (&self.target_identity_sha256, "quad target"),
            (&self.batch_identity_sha256, "quad batch"),
            (
                &self.texture_descriptor_set_sha256,
                "quad texture descriptor set",
            ),
            (&self.ordered_quads_sha256, "ordered quads"),
            (&self.deterministic_sha256, "quad receipt"),
        ] {
            validate_sha256(value, role)?;
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != sha256_json(&unsigned)? {
            return Err("XMAP21 neutral quad receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MapFurnitureOrderedChildKind {
    Quad,
    VpCapitalText,
    ResourceText,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MapFurnitureOrderedCommand {
    pub(crate) root_stable_id: String,
    pub(crate) child_stable_id: String,
    pub(crate) child_kind: MapFurnitureOrderedChildKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MapFurnitureBucketPhaseReceipt {
    pub(crate) schema: String,
    pub(crate) bucket_ordinals: Vec<u8>,
    pub(crate) execution_target_sha256: String,
    pub(crate) first_command_index: usize,
    pub(crate) command_count: usize,
    pub(crate) ordered_commands_sha256: String,
    pub(crate) quad_executions_sha256: String,
    pub(crate) deterministic_sha256: String,
}

impl MapFurnitureBucketPhaseReceipt {
    fn seal(
        bucket_ordinals: Vec<u8>,
        execution_target_sha256: String,
        first_command_index: usize,
        commands: &[MapFurnitureOrderedCommand],
        quad_executions: &[MapSceneQuadExecutionReceipt],
    ) -> Result<Self> {
        let mut receipt = Self {
            schema: "hoi4.replay.xmap21-map-icon-bucket-phase.v1".into(),
            bucket_ordinals,
            execution_target_sha256,
            first_command_index,
            command_count: commands.len(),
            ordered_commands_sha256: sha256_json(&commands)?,
            quad_executions_sha256: sha256_json(&quad_executions)?,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = sha256_json(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != "hoi4.replay.xmap21-map-icon-bucket-phase.v1"
            || self.bucket_ordinals.is_empty()
        {
            return Err("XMAP21 map-icon bucket phase receipt is incomplete".into());
        }
        for (value, role) in [
            (&self.execution_target_sha256, "bucket phase target"),
            (&self.ordered_commands_sha256, "bucket phase commands"),
            (&self.quad_executions_sha256, "bucket phase quads"),
            (&self.deterministic_sha256, "bucket phase receipt"),
        ] {
            validate_sha256(value, role)?;
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != sha256_json(&unsigned)? {
            return Err("XMAP21 map-icon bucket phase receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MapFurnitureExecutionReceipt {
    pub(crate) schema: String,
    pub(crate) update_index: u64,
    pub(crate) content_manifest_sha256: String,
    pub(crate) foundation_content_operation_sha256: String,
    pub(crate) viewer_state_sha256: String,
    pub(crate) main_scene_target_sha256: String,
    pub(crate) execution_target_sha256: String,
    pub(crate) map_copy_receipt_set_sha256: String,
    pub(crate) packet_sha256: String,
    pub(crate) vp_capital_text_batch_sha256: String,
    pub(crate) vp_capital_text_execution: crate::map_labels::Xmap21VpCapitalTextDrawReceipt,
    pub(crate) resource_text_batch_sha256: Option<String>,
    pub(crate) resource_text_execution: Option<crate::map_labels::Xmap21ResourceTextDrawReceipt>,
    pub(crate) assignment: map_viewer::AssignedStage,
    pub(crate) foundation_operation_identity: map_viewer::FoundationOperationIdentity,
    pub(crate) texture_descriptor_set_sha256: String,
    pub(crate) ordered_commands: Vec<MapFurnitureOrderedCommand>,
    pub(crate) ordered_commands_sha256: String,
    pub(crate) quad_executions: Vec<MapSceneQuadExecutionReceipt>,
    pub(crate) counter_execution_sha256: String,
    pub(crate) combat_execution_sha256: String,
    pub(crate) hub_execution_sha256: String,
    pub(crate) vp_resource_phase: MapFurnitureBucketPhaseReceipt,
    pub(crate) raid_phase: MapFurnitureBucketPhaseReceipt,
    pub(crate) physical_bucket_sequence_sha256: String,
    pub(crate) deterministic_sha256: String,
}

impl MapFurnitureExecutionReceipt {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.schema != MAP_FURNITURE_EXECUTION_SCHEMA
            || self.assignment.stage_name != map_viewer::XMAP21_EXECUTE_STAGE_NAME
            || self.assignment.operation_name != map_viewer::XMAP21_EXECUTE_OPERATION_NAME
            || self.foundation_operation_identity.operation_id
                != map_viewer::XMAP21_EXECUTE_OPERATION_NAME
            || self.foundation_operation_identity.global_order != self.assignment.assigned_ordinal
            || self.vp_capital_text_batch_sha256
                != self.vp_capital_text_execution.vp_capital_text_batch_sha256
            || self.resource_text_batch_sha256.as_deref()
                != self
                    .resource_text_execution
                    .as_ref()
                    .map(|receipt| receipt.resource_text_batch_sha256.as_str())
        {
            return Err("XMAP21 furniture execution receipt body is invalid".into());
        }
        self.vp_resource_phase.validate()?;
        self.raid_phase.validate()?;
        let vp_resource_quad_count = self.ordered_commands[..self
            .vp_resource_phase
            .command_count
            .min(self.ordered_commands.len())]
            .iter()
            .filter(|command| command.child_kind == MapFurnitureOrderedChildKind::Quad)
            .count();
        if self.vp_resource_phase.bucket_ordinals != VP_RESOURCE_BUCKETS
            || self.raid_phase.bucket_ordinals != RAID_BUCKETS
            || self.vp_resource_phase.first_command_index != 0
            || self.raid_phase.first_command_index != self.vp_resource_phase.command_count
            || self.vp_resource_phase.command_count + self.raid_phase.command_count
                != self.ordered_commands.len()
            || self.vp_resource_phase.execution_target_sha256 != self.execution_target_sha256
            || self.raid_phase.execution_target_sha256 != self.execution_target_sha256
            || self.vp_resource_phase.ordered_commands_sha256
                != sha256_json(&self.ordered_commands[..self.vp_resource_phase.command_count])?
            || self.raid_phase.ordered_commands_sha256
                != sha256_json(&self.ordered_commands[self.vp_resource_phase.command_count..])?
            || self.vp_resource_phase.quad_executions_sha256
                != sha256_json(&self.quad_executions[..vp_resource_quad_count])?
            || self.raid_phase.quad_executions_sha256
                != sha256_json(&self.quad_executions[vp_resource_quad_count..])?
            || self.physical_bucket_sequence_sha256
                != physical_map_icon_sequence_sha256(
                    &self.counter_execution_sha256,
                    &self.vp_resource_phase.deterministic_sha256,
                    &self.combat_execution_sha256,
                    &self.hub_execution_sha256,
                    &self.raid_phase.deterministic_sha256,
                )?
        {
            return Err("XMAP21 physical map-icon bucket sequence is invalid".into());
        }
        self.assignment
            .validate()
            .map_err(|error| format!("XMAP21 furniture execution assignment: {error}"))?;
        validate_sha256(
            &self.texture_descriptor_set_sha256,
            "execution texture descriptor set",
        )?;
        validate_sha256(&self.ordered_commands_sha256, "ordered GUI commands")?;
        if self.ordered_commands_sha256 != sha256_json(&self.ordered_commands)? {
            return Err("XMAP21 ordered GUI command digest mismatch".into());
        }
        let quad_commands: Vec<_> = self
            .ordered_commands
            .iter()
            .enumerate()
            .filter(|(_, command)| command.child_kind == MapFurnitureOrderedChildKind::Quad)
            .collect();
        if quad_commands.len() != self.quad_executions.len()
            || self
                .ordered_commands
                .iter()
                .filter(|command| command.child_kind == MapFurnitureOrderedChildKind::VpCapitalText)
                .count()
                != self.vp_capital_text_execution.operation_count
            || self
                .ordered_commands
                .iter()
                .filter(|command| command.child_kind == MapFurnitureOrderedChildKind::ResourceText)
                .count()
                != self
                    .resource_text_execution
                    .as_ref()
                    .map_or(0, |receipt| receipt.operation_count)
        {
            return Err("XMAP21 ordered GUI command execution frontier is incomplete".into());
        }
        for ((command_index, command), execution) in
            quad_commands.into_iter().zip(&self.quad_executions)
        {
            execution.validate()?;
            if execution.target_identity_sha256 != self.execution_target_sha256
                || execution.texture_descriptor_set_sha256 != self.texture_descriptor_set_sha256
                || execution.batch_identity_sha256
                    != sha256_json(&(self.packet_sha256.as_str(), command_index, command))?
            {
                return Err("XMAP21 ordered quad execution receipt mismatch".into());
            }
        }
        self.vp_capital_text_execution.validate()?;
        if self.vp_capital_text_execution.main_scene_target_sha256 != self.execution_target_sha256 {
            return Err("XMAP21 VP/capital-text execution targeted another MainScene".into());
        }
        if let Some(resource_text) = &self.resource_text_execution {
            resource_text.validate()?;
            if resource_text.main_scene_target_sha256 != self.execution_target_sha256 {
                return Err("XMAP21 resource-text execution targeted another MainScene".into());
            }
        }
        for (value, role) in [
            (&self.content_manifest_sha256, "execution content manifest"),
            (
                &self.foundation_content_operation_sha256,
                "execution foundation content operation",
            ),
            (&self.viewer_state_sha256, "execution viewer state"),
            (&self.main_scene_target_sha256, "execution main scene"),
            (&self.execution_target_sha256, "execution final image"),
            (
                &self.map_copy_receipt_set_sha256,
                "execution map-copy receipt set",
            ),
            (&self.packet_sha256, "execution packet"),
            (&self.ordered_commands_sha256, "execution ordered commands"),
            (&self.counter_execution_sha256, "counter bucket execution"),
            (&self.combat_execution_sha256, "combat bucket execution"),
            (&self.hub_execution_sha256, "hub bucket execution"),
            (
                &self.physical_bucket_sequence_sha256,
                "physical bucket sequence",
            ),
            (&self.deterministic_sha256, "execution receipt"),
        ] {
            validate_sha256(value, role)?;
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != sha256_json(&unsigned)? {
            return Err("XMAP21 furniture execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

struct NativeFurnitureTexture {
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
    descriptor_sha256: String,
    width: u32,
    height: u32,
    frames: Vec<map_viewer::AtlasFrame>,
}

struct NativeMaskedFlagTexture {
    base: NativeFurnitureTexture,
    mask: NativeFurnitureTexture,
    overlay: NativeFurnitureTexture,
    descriptor_sha256: String,
}

#[derive(Clone)]
enum FurnitureDrawPayload {
    Quad(NeutralMapSceneQuad),
    VpCapitalText(map_viewer::Xmap21VpCapitalTextOperation),
    ResourceText(map_viewer::Xmap21ResourceTextOperation),
}

#[derive(Clone)]
struct FurnitureDrawCommand {
    receipt: MapFurnitureOrderedCommand,
    payload: FurnitureDrawPayload,
}

struct FurnitureRootCommands {
    engine_sort_key: f32,
    map_icon_ordinal: u32,
    commands: Vec<FurnitureDrawCommand>,
}

struct ResourceFurnitureQuads {
    row_flags: Vec<NeutralMapSceneQuad>,
    root_flag: Option<NeutralMapSceneQuad>,
}

const FURNITURE_ICON_SHADER_HLSL: &str = r#"
cbuffer ScreenCB : register(b0) {
    float2 screen_size;
    float2 _pad;
};

Texture2D Strip : register(t0);
SamplerState FurnitureSampler : register(s0);

struct VsIn {
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
};

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(VsIn input) {
    VsOut output;
    // DERIVED coordinate transform: window pixels (Y down) to D3D clip space.
    float2 clip = float2(
        input.pos.x / screen_size.x * 2.0f - 1.0f,
        1.0f - input.pos.y / screen_size.y * 2.0f);
    output.pos = float4(clip, 0.0f, 1.0f);
    output.uv = input.uv;
    return output;
}

float4 ps_main(VsOut input) : SV_Target {
    return Strip.Sample(FurnitureSampler, input.uv);
}
"#;

const MASKED_FLAG_PIXEL_SHADER_HLSL: &str = r#"
Texture2D BaseTexture : register(t0);
Texture2D MaskTexture : register(t1);
SamplerState BaseSampler : register(s0);
SamplerState MaskSampler : register(s1);

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

float4 ps_main(VsOut input) : SV_Target {
    float4 OutColor = BaseTexture.Sample(BaseSampler, input.uv);
    float4 MaskColor = MaskTexture.Sample(MaskSampler, input.uv);
    OutColor.a = MaskColor.a;
    return OutColor;
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct FurnitureIconVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScreenConstants {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

struct FurnitureIconRenderer {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    masked_flag_ps: ComPtr<ID3D11PixelShader>,
    input_layout: ComPtr<ID3D11InputLayout>,
    sampler: ComPtr<ID3D11SamplerState>,
    masked_flag_base_sampler: ComPtr<ID3D11SamplerState>,
    masked_flag_mask_sampler: ComPtr<ID3D11SamplerState>,
    blend: ComPtr<ID3D11BlendState>,
    screen_cb: ComPtr<ID3D11Buffer>,
    vertex_buffer: ComPtr<ID3D11Buffer>,
    vertex_capacity: usize,
    _strip_texture: ComPtr<ID3D11Texture2D>,
    strip_srv: ComPtr<ID3D11ShaderResourceView>,
    strip_descriptor_sha256: String,
}

impl FurnitureIconRenderer {
    fn new(
        gfx: &Gfx,
        strip: &map_viewer::MapFurnitureSpriteAsset,
        maximum_marker_count: usize,
    ) -> Result<Self> {
        unsafe {
            let vs_blob = compile_shader(FURNITURE_ICON_SHADER_HLSL, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(FURNITURE_ICON_SHADER_HLSL, "ps_main", "ps_4_0")?;
            let masked_flag_ps_blob =
                compile_shader(MASKED_FLAG_PIXEL_SHADER_HLSL, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for map furniture icons")?;
            let vs = ComPtr::from_raw(vs, "map-furniture icon vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for map furniture icons")?;
            let ps = ComPtr::from_raw(ps, "map-furniture icon pixel shader")?;

            let mut masked_flag_ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*masked_flag_ps_blob.as_raw()).GetBufferPointer(),
                (*masked_flag_ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut masked_flag_ps,
            );
            hr_ok(hr, "CreatePixelShader for XMAP21 masked flags")?;
            let masked_flag_ps =
                ComPtr::from_raw(masked_flag_ps, "XMAP21 masked-flag pixel shader")?;

            let position = CString::new("POSITION").expect("static semantic");
            let texcoord = CString::new("TEXCOORD").expect("static semantic");
            let elements = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: position.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: texcoord.as_ptr(),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];
            let mut input_layout: *mut ID3D11InputLayout = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as UINT,
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                &mut input_layout,
            );
            hr_ok(hr, "CreateInputLayout for map furniture icons")?;
            let input_layout = ComPtr::from_raw(input_layout, "map-furniture icon input layout")?;

            let mut sampler_desc: D3D11_SAMPLER_DESC = mem::zeroed();
            sampler_desc.Filter = D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT;
            sampler_desc.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
            sampler_desc.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
            sampler_desc.MipLODBias = -0.8;
            sampler_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
            sampler_desc.MinLOD = 0.0;
            sampler_desc.MaxLOD = 1024.0;
            let mut sampler: *mut ID3D11SamplerState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateSamplerState(&sampler_desc, &mut sampler);
            hr_ok(hr, "CreateSamplerState for map furniture icons")?;
            let sampler = ComPtr::from_raw(sampler, "map-furniture icon sampler")?;

            let mut masked_base_desc: D3D11_SAMPLER_DESC = mem::zeroed();
            masked_base_desc.Filter = D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR;
            masked_base_desc.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
            masked_base_desc.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
            masked_base_desc.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
            masked_base_desc.ComparisonFunc = D3D11_COMPARISON_NEVER;
            masked_base_desc.MinLOD = 0.0;
            masked_base_desc.MaxLOD = f32::MAX;
            let mut masked_flag_base_sampler: *mut ID3D11SamplerState = ptr::null_mut();
            let hr = (*gfx.device.as_raw())
                .CreateSamplerState(&masked_base_desc, &mut masked_flag_base_sampler);
            hr_ok(hr, "CreateSamplerState for XMAP21 masked-flag base")?;
            let masked_flag_base_sampler =
                ComPtr::from_raw(masked_flag_base_sampler, "XMAP21 masked-flag base sampler")?;

            let mut masked_mask_desc = masked_base_desc;
            masked_mask_desc.Filter = D3D11_FILTER_MIN_MAG_MIP_POINT;
            masked_mask_desc.MaxLOD = 0.0;
            let mut masked_flag_mask_sampler: *mut ID3D11SamplerState = ptr::null_mut();
            let hr = (*gfx.device.as_raw())
                .CreateSamplerState(&masked_mask_desc, &mut masked_flag_mask_sampler);
            hr_ok(hr, "CreateSamplerState for XMAP21 masked-flag mask")?;
            let masked_flag_mask_sampler =
                ComPtr::from_raw(masked_flag_mask_sampler, "XMAP21 masked-flag mask sampler")?;

            let mut blend_desc: D3D11_BLEND_DESC = mem::zeroed();
            blend_desc.RenderTarget[0].BlendEnable = 1;
            blend_desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
            blend_desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_SRC_ALPHA;
            blend_desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
            blend_desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
            blend_desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
            let mut blend: *mut ID3D11BlendState = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBlendState(&blend_desc, &mut blend);
            hr_ok(hr, "CreateBlendState for map furniture icons")?;
            let blend = ComPtr::from_raw(blend, "map-furniture icon blend state")?;

            let cb_desc = D3D11_BUFFER_DESC {
                ByteWidth: mem::size_of::<ScreenConstants>() as UINT,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut screen_cb: *mut ID3D11Buffer = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBuffer(&cb_desc, ptr::null(), &mut screen_cb);
            hr_ok(hr, "CreateBuffer for map furniture screen constants")?;
            let screen_cb = ComPtr::from_raw(screen_cb, "map-furniture screen constants")?;

            let vertex_capacity = maximum_marker_count
                .max(1)
                .checked_mul(6)
                .ok_or("map-furniture icon vertex capacity overflow")?;
            let byte_width = vertex_capacity
                .checked_mul(mem::size_of::<FurnitureIconVertex>())
                .and_then(|bytes| u32::try_from(bytes).ok())
                .ok_or("map-furniture icon vertex buffer exceeds D3D11 limits")?;
            let vb_desc = D3D11_BUFFER_DESC {
                ByteWidth: byte_width,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut vertex_buffer: *mut ID3D11Buffer = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateBuffer(&vb_desc, ptr::null(), &mut vertex_buffer);
            hr_ok(hr, "CreateBuffer for map furniture icon vertices")?;
            let vertex_buffer =
                ComPtr::from_raw(vertex_buffer, "map-furniture icon vertex buffer")?;

            let (strip_texture, strip_srv) = create_immutable_texture(
                gfx,
                strip.image.width,
                strip.image.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                &strip.image.pixels,
                "map-furniture VP strip atlas",
            )?;
            let strip_descriptor_sha256 = strip.descriptor_sha256.clone();

            Ok(Self {
                vs,
                ps,
                masked_flag_ps,
                input_layout,
                sampler,
                masked_flag_base_sampler,
                masked_flag_mask_sampler,
                blend,
                screen_cb,
                vertex_buffer,
                vertex_capacity,
                _strip_texture: strip_texture,
                strip_srv,
                strip_descriptor_sha256,
            })
        }
    }

    pub(crate) unsafe fn execute_neutral(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        target: &AuthenticatedMapSceneTarget,
        textures: &[NativeMapSceneTextureBinding],
        quads: &[NeutralMapSceneQuad],
        batch_identity_sha256: &str,
    ) -> Result<MapSceneQuadExecutionReceipt> {
        validate_neutral_batch(context, target, textures, quads, batch_identity_sha256)?;
        let backend_identity_sha256 = map_scene_quad_backend_identity();
        let ordered_quads_sha256 = sha256_json(quads)?;
        let texture_descriptor_set_sha256 = sha256_json(
            &textures
                .iter()
                .map(|binding| (&binding.role, &binding.descriptor_sha256))
                .collect::<Vec<_>>(),
        )?;
        let mut submitted_draw_count = 0usize;
        let mut masked_flag_quad_count = 0usize;
        if !quads.is_empty() {
            let constants = ScreenConstants {
                screen_size: [target.width as f32, target.height as f32],
                _pad: [0.0; 2],
            };
            write_dynamic_buffer(
                context,
                &self.screen_cb,
                &constants,
                "Map map-scene textured-quad screen constants",
            )?;
            let rtvs = [target.render_target_view];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: target.width as f32,
                Height: target.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(self.input_layout.as_raw());
            let stride = mem::size_of::<FurnitureIconVertex>() as UINT;
            let offset = 0u32;
            let buffers = [self.vertex_buffer.as_raw()];
            (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), &stride, &offset);
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            (*context).VSSetConstantBuffers(0, 1, [self.screen_cb.as_raw()].as_ptr());
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            (*context).PSSetSamplers(0, 1, [self.sampler.as_raw()].as_ptr());
            let blend_factor = [0.0f32; 4];
            (*context).OMSetBlendState(self.blend.as_raw(), &blend_factor, u32::MAX);

            let quad_capacity = (self.vertex_capacity / 6).max(1);
            for chunk in quads.chunks(quad_capacity) {
                let mut vertices = Vec::with_capacity(chunk.len() * 6);
                for quad in chunk {
                    let vertex = |index: usize| FurnitureIconVertex {
                        position: quad.vertices[index].position_px,
                        uv: quad.vertices[index].uv,
                    };
                    vertices.extend([
                        vertex(0),
                        vertex(1),
                        vertex(2),
                        vertex(1),
                        vertex(3),
                        vertex(2),
                    ]);
                }
                write_dynamic_vertices(context, &self.vertex_buffer, &vertices)?;
                let mut first_quad = 0usize;
                while first_quad < chunk.len() {
                    let role = &chunk[first_quad].texture_role;
                    let end_quad = chunk[first_quad..]
                        .iter()
                        .position(|quad| &quad.texture_role != role)
                        .map(|relative| first_quad + relative)
                        .unwrap_or(chunk.len());
                    let srv = textures
                        .iter()
                        .find(|binding| &binding.role == role)
                        .ok_or_else(|| format!("neutral map-scene quad role {role:?} is unbound"))?
                        .shader_resource_view;
                    let mask_role = resource_flag_mask_role(role);
                    let overlay_role = resource_flag_overlay_role(role);
                    let mask = textures.iter().find(|binding| binding.role == mask_role);
                    let overlay = textures.iter().find(|binding| binding.role == overlay_role);
                    match (is_resource_flag_role(role), mask, overlay) {
                        (true, Some(mask), Some(overlay)) => {
                            (*context).PSSetShader(self.masked_flag_ps.as_raw(), ptr::null(), 0);
                            (*context).PSSetSamplers(
                                0,
                                2,
                                [
                                    self.masked_flag_base_sampler.as_raw(),
                                    self.masked_flag_mask_sampler.as_raw(),
                                ]
                                .as_ptr(),
                            );
                            (*context).PSSetShaderResources(
                                0,
                                2,
                                [srv, mask.shader_resource_view].as_ptr(),
                            );
                            (*context).Draw(
                                ((end_quad - first_quad) * 6) as UINT,
                                (first_quad * 6) as UINT,
                            );
                            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
                            (*context).PSSetSamplers(
                                0,
                                1,
                                [self.masked_flag_mask_sampler.as_raw()].as_ptr(),
                            );
                            (*context).PSSetShaderResources(
                                0,
                                1,
                                [overlay.shader_resource_view].as_ptr(),
                            );
                            (*context).Draw(
                                ((end_quad - first_quad) * 6) as UINT,
                                (first_quad * 6) as UINT,
                            );
                            submitted_draw_count += 2;
                            masked_flag_quad_count += end_quad - first_quad;
                        }
                        (false, None, None) => {
                            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
                            (*context).PSSetSamplers(
                                0,
                                1,
                                [self.sampler.as_raw()].as_ptr(),
                            );
                            (*context).PSSetShaderResources(0, 1, [srv].as_ptr());
                            (*context).Draw(
                                ((end_quad - first_quad) * 6) as UINT,
                                (first_quad * 6) as UINT,
                            );
                            submitted_draw_count += 1;
                        }
                        (true, _, _) => {
                            return Err(format!(
                                "XMAP21 masked-flag role {role:?} has an incomplete GPU binding"
                            )
                            .into())
                        }
                        (false, _, _) => {
                            return Err(format!(
                                "ordinary map-scene quad role {role:?} has unexpected masked-flag bindings"
                            )
                            .into())
                        }
                    }
                    first_quad = end_quad;
                }
            }

            let null_srvs: [*mut ID3D11ShaderResourceView; 2] = [ptr::null_mut(), ptr::null_mut()];
            (*context).PSSetShaderResources(0, 2, null_srvs.as_ptr());
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        }
        let mut receipt = MapSceneQuadExecutionReceipt {
            schema: MAP_SCENE_QUAD_BACKEND_SCHEMA.into(),
            backend_identity_sha256,
            target_kind: target.kind,
            target_identity_sha256: target.target_identity_sha256.clone(),
            target_width: target.width,
            target_height: target.height,
            target_dxgi_format: target.dxgi_format,
            target_sample_count: target.sample_count,
            target_sample_quality: target.sample_quality,
            batch_identity_sha256: batch_identity_sha256.into(),
            texture_descriptor_set_sha256,
            ordered_quads_sha256,
            submitted_quad_count: quads.len(),
            masked_flag_quad_count,
            submitted_draw_count,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = sha256_json(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }
}

pub(crate) struct FurnitureRenderPass {
    gui: MapFurnitureGuiGeometry,
    vp_defines: map_viewer::VpIconDefines,
    distance_defines: map_viewer::MapFurnitureDistanceDefines,
    icons: FurnitureIconRenderer,
    vp_capital_text: crate::map_labels::Xmap21VpCapitalTextRenderer,
    resource_text: crate::map_labels::Xmap21ResourceTextRenderer,
    dynamic_textures: BTreeMap<String, NativeFurnitureTexture>,
    masked_flags: BTreeMap<String, NativeMaskedFlagTexture>,
}

impl FurnitureRenderPass {
    pub(crate) fn new(gfx: &Gfx, resolver: &GameFileResolver, language: &str) -> Result<Self> {
        let gui = MapFurnitureGuiGeometry::from_resolver(resolver)?;
        let vp_defines = map_viewer::vp_icon_defines_from_resolver(resolver)?;
        let distance_defines = map_viewer::MapFurnitureDistanceDefines::from_resolver(resolver)?;
        let strip = map_viewer::load_vp_strip_asset(resolver)?;
        let icons = FurnitureIconRenderer::new(gfx, &strip, 0x100 + 0xc0)?;
        let resource_text =
            crate::map_labels::Xmap21ResourceTextRenderer::new(gfx, resolver, language)?;
        let vp_capital_text =
            crate::map_labels::Xmap21VpCapitalTextRenderer::new(gfx, resolver, language)?;
        Ok(Self {
            gui,
            vp_defines,
            distance_defines,
            icons,
            vp_capital_text,
            resource_text,
            dynamic_textures: BTreeMap::new(),
            masked_flags: BTreeMap::new(),
        })
    }

    pub(crate) fn prepare_packet(
        &self,
        world: &replay_core::WorldFrame,
        resolver: &GameFileResolver,
        language: &str,
        request: &map_viewer::MapFurniturePrepareRequest,
        camera: &map_viewer::ProductionMapCamera,
        screen_width: f32,
        screen_height: f32,
    ) -> Result<PreparedMapFurniturePacket> {
        map_viewer::prepare_map_furniture_packet(
            world,
            resolver,
            language,
            request,
            self.gui,
            &self.vp_defines,
            self.distance_defines,
            camera,
            screen_width,
            screen_height,
        )
    }

    pub(crate) unsafe fn execute_prepared_packet_interleaved<C, H, FC, FH>(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        target: &AuthenticatedMapSceneTarget,
        packet: &PreparedMapFurniturePacket,
        counter_execution_sha256: &str,
        execute_combat_buckets: FC,
        execute_hub_bucket: FH,
    ) -> Result<(MapFurnitureExecutionReceipt, C, H)>
    where
        FC: FnOnce() -> Result<(C, String)>,
        FH: FnOnce() -> Result<(H, String)>,
    {
        packet.validate()?;
        validate_sha256(counter_execution_sha256, "counter bucket execution")?;
        if target.kind != MapSceneTargetKind::FinalImage {
            return Err("XMAP21 bucket phases require the authenticated final image".into());
        }
        let current_strip = map_viewer::load_vp_strip_asset(resolver)?;
        if current_strip.descriptor_sha256 != packet.vp_strip_descriptor_sha256
            || self.icons.strip_descriptor_sha256 != packet.vp_strip_descriptor_sha256
        {
            return Err("XMAP21 VP strip asset changed after preparation".into());
        }
        self.ensure_furniture_textures(gfx, resolver, packet)?;
        let mut vp_text_by_parent = BTreeMap::new();
        for operation in &packet.vp_capital_text.operations {
            if vp_text_by_parent
                .insert(operation.parent_map_icon_ordinal, operation.clone())
                .is_some()
            {
                return Err("XMAP21 VP/capital root has multiple text children".into());
            }
        }
        let mut resource_text_by_parent_row = BTreeMap::new();
        if let Some(batch) = &packet.resource_text {
            for operation in &batch.operations {
                if resource_text_by_parent_row
                    .insert(
                        (operation.parent_map_icon_ordinal, operation.row_index),
                        operation.clone(),
                    )
                    .is_some()
                {
                    return Err("XMAP21 resource row has multiple desc children".into());
                }
            }
        }
        let mut vp_groups: Vec<FurnitureRootCommands> = Vec::new();
        for icon in &packet.victory_points_and_capitals {
            let root_stable_id = icon.target.canonical_id();
            let mut commands = Vec::new();
            if icon.dot_visible {
                commands.push(FurnitureDrawCommand {
                    receipt: MapFurnitureOrderedCommand {
                        root_stable_id: root_stable_id.clone(),
                        child_stable_id: format!("{root_stable_id}:icon"),
                        child_kind: MapFurnitureOrderedChildKind::Quad,
                    },
                    payload: FurnitureDrawPayload::Quad(neutral_vp_quad(icon)),
                });
            }
            let text_operation = vp_text_by_parent.remove(&icon.map_icon_ordinal);
            if icon.text_visible && text_operation.is_none() {
                return Err("XMAP21 visible VP/capital root is missing its text child".into());
            }
            if !icon.text_visible && text_operation.is_some() {
                return Err("XMAP21 hidden VP/capital root carries a text child".into());
            }
            if let Some(operation) = text_operation {
                commands.push(FurnitureDrawCommand {
                    receipt: MapFurnitureOrderedCommand {
                        root_stable_id: root_stable_id.clone(),
                        child_stable_id: operation.stable_id.clone(),
                        child_kind: MapFurnitureOrderedChildKind::VpCapitalText,
                    },
                    payload: FurnitureDrawPayload::VpCapitalText(operation),
                });
            }
            if !commands.is_empty() {
                vp_groups.push(FurnitureRootCommands {
                    engine_sort_key: icon.engine_sort_key,
                    map_icon_ordinal: icon.map_icon_ordinal,
                    commands,
                });
            }
        }
        if !vp_text_by_parent.is_empty() {
            return Err("XMAP21 VP/capital text references an absent root".into());
        }
        let mut raid_groups: Vec<FurnitureRootCommands> = Vec::new();
        for icon in &packet.raids {
            let root_stable_id = MapFurnitureTarget::Raid {
                raid_identities: icon.raid_identities.clone(),
            }
            .canonical_id();
            let commands = self
                .neutral_raid_quads(icon)?
                .into_iter()
                .map(|(child_name, quad)| FurnitureDrawCommand {
                    receipt: MapFurnitureOrderedCommand {
                        root_stable_id: root_stable_id.clone(),
                        child_stable_id: format!("{root_stable_id}:{child_name}"),
                        child_kind: MapFurnitureOrderedChildKind::Quad,
                    },
                    payload: FurnitureDrawPayload::Quad(quad),
                })
                .collect();
            raid_groups.push(FurnitureRootCommands {
                engine_sort_key: icon.engine_sort_key,
                map_icon_ordinal: icon.map_icon_ordinal,
                commands,
            });
        }
        let mut resource_groups: Vec<FurnitureRootCommands> = Vec::new();
        for icon in &packet.resources {
            let root_stable_id = format!("xmap21.resource:{}", icon.state_id);
            let mut commands = Vec::new();
            let resource_quads = self.neutral_resource_quads(icon)?;
            for row_index in 0..icon.row_count {
                let operation = resource_text_by_parent_row
                    .remove(&(icon.map_icon_ordinal, row_index))
                    .ok_or("XMAP21 resource row lacks its authored desc child")?;
                commands.push(FurnitureDrawCommand {
                    receipt: MapFurnitureOrderedCommand {
                        root_stable_id: root_stable_id.clone(),
                        child_stable_id: operation.stable_id.clone(),
                        child_kind: MapFurnitureOrderedChildKind::ResourceText,
                    },
                    payload: FurnitureDrawPayload::ResourceText(operation),
                });
                if let Some(quad) = resource_quads.row_flags.get(row_index as usize) {
                    commands.push(FurnitureDrawCommand {
                        receipt: MapFurnitureOrderedCommand {
                            root_stable_id: root_stable_id.clone(),
                            child_stable_id: format!("{root_stable_id}:row:{row_index}:flag"),
                            child_kind: MapFurnitureOrderedChildKind::Quad,
                        },
                        payload: FurnitureDrawPayload::Quad(quad.clone()),
                    });
                }
            }
            if let Some(quad) = resource_quads.root_flag {
                commands.push(FurnitureDrawCommand {
                    receipt: MapFurnitureOrderedCommand {
                        root_stable_id: root_stable_id.clone(),
                        child_stable_id: format!("{root_stable_id}:flag"),
                        child_kind: MapFurnitureOrderedChildKind::Quad,
                    },
                    payload: FurnitureDrawPayload::Quad(quad),
                });
            }
            resource_groups.push(FurnitureRootCommands {
                engine_sort_key: icon.engine_sort_key,
                map_icon_ordinal: icon.map_icon_ordinal,
                commands,
            });
        }
        if !resource_text_by_parent_row.is_empty() {
            return Err("XMAP21 resource text references an absent row".into());
        }
        let sort_bucket = |groups: &mut Vec<FurnitureRootCommands>| {
            groups.sort_by(|left, right| {
                left.engine_sort_key
                    .partial_cmp(&right.engine_sort_key)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.map_icon_ordinal.cmp(&right.map_icon_ordinal))
            });
            groups.reverse();
        };
        sort_bucket(&mut vp_groups);
        sort_bucket(&mut resource_groups);
        sort_bucket(&mut raid_groups);
        let vp_resource_commands: Vec<_> = vp_groups
            .into_iter()
            .chain(resource_groups)
            .flat_map(|group| group.commands)
            .collect();
        let raid_commands: Vec<_> = raid_groups
            .into_iter()
            .flat_map(|group| group.commands)
            .collect();
        let vp_resource_command_count = vp_resource_commands.len();
        let commands: Vec<_> = vp_resource_commands
            .into_iter()
            .chain(raid_commands)
            .collect();
        let used_roles: BTreeSet<_> = commands
            .iter()
            .filter_map(|command| match &command.payload {
                FurnitureDrawPayload::Quad(quad) => Some(quad.texture_role.as_str()),
                _ => None,
            })
            .collect();
        let mut textures = Vec::new();
        if used_roles.contains("xmap21.vp-capital-strip") {
            textures.push(NativeMapSceneTextureBinding {
                role: "xmap21.vp-capital-strip".into(),
                descriptor_sha256: self.icons.strip_descriptor_sha256.clone(),
                shader_resource_view: self.icons.strip_srv.as_raw(),
            });
        }
        for (role, texture) in &self.dynamic_textures {
            if used_roles.contains(role.as_str()) {
                textures.push(NativeMapSceneTextureBinding {
                    role: role.clone(),
                    descriptor_sha256: texture.descriptor_sha256.clone(),
                    shader_resource_view: texture.srv.as_raw(),
                });
            }
        }
        for (role, flag) in &self.masked_flags {
            if !used_roles.contains(role.as_str()) {
                continue;
            }
            textures.extend([
                NativeMapSceneTextureBinding {
                    role: role.clone(),
                    descriptor_sha256: flag.base.descriptor_sha256.clone(),
                    shader_resource_view: flag.base.srv.as_raw(),
                },
                NativeMapSceneTextureBinding {
                    role: resource_flag_mask_role(role),
                    descriptor_sha256: flag.mask.descriptor_sha256.clone(),
                    shader_resource_view: flag.mask.srv.as_raw(),
                },
                NativeMapSceneTextureBinding {
                    role: resource_flag_overlay_role(role),
                    descriptor_sha256: flag.overlay.descriptor_sha256.clone(),
                    shader_resource_view: flag.overlay.srv.as_raw(),
                },
            ]);
        }
        let texture_descriptor_set_sha256 = sha256_json(
            &textures
                .iter()
                .map(|binding| (&binding.role, &binding.descriptor_sha256))
                .collect::<Vec<_>>(),
        )?;
        let ordered_commands: Vec<_> = commands
            .iter()
            .map(|command| command.receipt.clone())
            .collect();
        let ordered_commands_sha256 = sha256_json(&ordered_commands)?;
        let mut quad_executions = Vec::new();
        let mut vp_glyphs = 0usize;
        let mut vp_pages = 0usize;
        let mut resource_glyphs = 0usize;
        let mut resource_pages = 0usize;
        let mut combat_outcome = None;
        let mut hub_outcome = None;
        let mut execute_combat_buckets = Some(execute_combat_buckets);
        let mut execute_hub_bucket = Some(execute_hub_bucket);
        let mut vp_resource_quad_count = 0usize;
        for (command_index, command) in commands.iter().enumerate() {
            if command_index == vp_resource_command_count {
                combat_outcome = Some(execute_combat_buckets
                    .take()
                    .ok_or("XMAP14 combat bucket callback executed twice")?(
                )?);
                hub_outcome = Some(execute_hub_bucket
                    .take()
                    .ok_or("XMAP21 hub bucket callback executed twice")?(
                )?);
            }
            match &command.payload {
                FurnitureDrawPayload::Quad(quad) => {
                    let command_identity = sha256_json(&(
                        packet.deterministic_sha256.as_str(),
                        command_index,
                        &command.receipt,
                    ))?;
                    quad_executions.push(self.icons.execute_neutral(
                        gfx.context.as_raw(),
                        target,
                        &textures,
                        std::slice::from_ref(quad),
                        &command_identity,
                    )?);
                    if command_index < vp_resource_command_count {
                        vp_resource_quad_count += 1;
                    }
                }
                FurnitureDrawPayload::VpCapitalText(operation) => {
                    let (glyphs, pages) =
                        self.vp_capital_text.draw_xmap21_vp_capital_text_operation(
                            gfx,
                            operation,
                            target.render_target_view,
                            target.width,
                            target.height,
                        )?;
                    vp_glyphs = vp_glyphs
                        .checked_add(glyphs)
                        .ok_or("XMAP21 VP glyph overflow")?;
                    vp_pages = vp_pages
                        .checked_add(pages)
                        .ok_or("XMAP21 VP page overflow")?;
                }
                FurnitureDrawPayload::ResourceText(operation) => {
                    let (glyphs, pages) = self.resource_text.draw_xmap21_resource_text_operation(
                        gfx,
                        operation,
                        target.render_target_view,
                        target.width,
                        target.height,
                    )?;
                    resource_glyphs = resource_glyphs
                        .checked_add(glyphs)
                        .ok_or("XMAP21 resource glyph overflow")?;
                    resource_pages = resource_pages
                        .checked_add(pages)
                        .ok_or("XMAP21 resource page overflow")?;
                }
            }
        }
        if combat_outcome.is_none() {
            combat_outcome = Some(execute_combat_buckets
                .take()
                .ok_or("XMAP14 combat bucket callback disappeared")?(
            )?);
        }
        if hub_outcome.is_none() {
            hub_outcome = Some(execute_hub_bucket
                .take()
                .ok_or("XMAP21 hub bucket callback disappeared")?(
            )?);
        }
        let (combat_outcome, combat_execution_sha256) =
            combat_outcome.ok_or("XMAP14 combat bucket callback did not execute")?;
        validate_sha256(&combat_execution_sha256, "combat bucket execution")?;
        let (hub_outcome, hub_execution_sha256) =
            hub_outcome.ok_or("XMAP21 hub bucket callback did not execute")?;
        validate_sha256(&hub_execution_sha256, "hub bucket execution")?;
        let vp_capital_text_execution = self.vp_capital_text.seal_xmap21_vp_capital_text_receipt(
            &packet.vp_capital_text,
            &target.target_identity_sha256,
            vp_glyphs,
            vp_pages,
        )?;
        let resource_text_execution = packet
            .resource_text
            .as_ref()
            .map(|batch| {
                self.resource_text.seal_xmap21_resource_text_receipt(
                    batch,
                    &target.target_identity_sha256,
                    resource_glyphs,
                    resource_pages,
                )
            })
            .transpose()?;
        let foundation_operation_identity = packet
            .execute_assignment
            .foundation_operation_identity(
                map_scene_quad_backend_identity(),
                texture_descriptor_set_sha256.clone(),
                target.target_identity_sha256.clone(),
            )
            .map_err(|error| format!("XMAP21 execute operation identity: {error}"))?;
        let vp_resource_phase = MapFurnitureBucketPhaseReceipt::seal(
            VP_RESOURCE_BUCKETS.to_vec(),
            target.target_identity_sha256.clone(),
            0,
            &ordered_commands[..vp_resource_command_count],
            &quad_executions[..vp_resource_quad_count],
        )?;
        let raid_phase = MapFurnitureBucketPhaseReceipt::seal(
            RAID_BUCKETS.to_vec(),
            target.target_identity_sha256.clone(),
            vp_resource_command_count,
            &ordered_commands[vp_resource_command_count..],
            &quad_executions[vp_resource_quad_count..],
        )?;
        let mut receipt = MapFurnitureExecutionReceipt {
            schema: MAP_FURNITURE_EXECUTION_SCHEMA.into(),
            update_index: packet.update_index,
            content_manifest_sha256: packet.shared.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: packet
                .shared
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: packet.shared.viewer_state_sha256.clone(),
            main_scene_target_sha256: packet.shared.main_scene_target_sha256.clone(),
            execution_target_sha256: target.target_identity_sha256.clone(),
            map_copy_receipt_set_sha256: packet.shared.map_copy_receipt_set_sha256.clone(),
            packet_sha256: packet.deterministic_sha256.clone(),
            vp_capital_text_batch_sha256: packet.vp_capital_text.deterministic_sha256.clone(),
            vp_capital_text_execution,
            resource_text_batch_sha256: packet
                .resource_text
                .as_ref()
                .map(|batch| batch.deterministic_sha256.clone()),
            resource_text_execution,
            assignment: packet.execute_assignment.clone(),
            foundation_operation_identity,
            texture_descriptor_set_sha256,
            ordered_commands,
            ordered_commands_sha256,
            quad_executions,
            counter_execution_sha256: counter_execution_sha256.to_owned(),
            combat_execution_sha256,
            hub_execution_sha256,
            vp_resource_phase,
            raid_phase,
            physical_bucket_sequence_sha256: String::new(),
            deterministic_sha256: String::new(),
        };
        receipt.physical_bucket_sequence_sha256 = physical_map_icon_sequence_sha256(
            &receipt.counter_execution_sha256,
            &receipt.vp_resource_phase.deterministic_sha256,
            &receipt.combat_execution_sha256,
            &receipt.hub_execution_sha256,
            &receipt.raid_phase.deterministic_sha256,
        )?;
        receipt.deterministic_sha256 = sha256_json(&receipt)?;
        receipt.validate()?;
        Ok((receipt, combat_outcome, hub_outcome))
    }

    fn ensure_furniture_textures(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        packet: &PreparedMapFurniturePacket,
    ) -> Result<()> {
        let names: BTreeSet<_> = packet
            .raids
            .iter()
            .flat_map(|raid| {
                [
                    raid.presentation.frame_sprite_name.as_ref(),
                    raid.presentation.button_sprite_name.as_ref(),
                    raid.presentation.plus_sprite_name.as_ref(),
                ]
                .into_iter()
                .flatten()
                .cloned()
            })
            .collect();
        let assets = load_map_furniture_sprite_assets(resolver, &names)?;
        let descriptor_set_sha256 = sha256_json(
            &assets
                .values()
                .map(|asset| (&asset.name, &asset.descriptor_sha256))
                .collect::<Vec<_>>(),
        )?;
        if descriptor_set_sha256 != packet.raid_sprite_descriptor_set_sha256 {
            return Err("XMAP21 raid sprite descriptor set changed after preparation".into());
        }
        for (name, asset) in assets {
            if let Some(existing) = self.dynamic_textures.get(&name) {
                if existing.descriptor_sha256 != asset.descriptor_sha256 {
                    return Err(
                        format!("XMAP21 cached raid sprite {name:?} changed descriptor").into(),
                    );
                }
                continue;
            }
            let width =
                u32::try_from(asset.image.width).map_err(|_| "raid sprite width exceeds u32")?;
            let height =
                u32::try_from(asset.image.height).map_err(|_| "raid sprite height exceeds u32")?;
            let (texture, srv) = unsafe {
                create_immutable_texture(
                    gfx,
                    usize::try_from(width).map_err(|_| "raid sprite width exceeds usize")?,
                    usize::try_from(height).map_err(|_| "raid sprite height exceeds usize")?,
                    DXGI_FORMAT_R8G8B8A8_UNORM,
                    &asset.image.pixels,
                    &format!("XMAP21 raid sprite {name}"),
                )?
            };
            self.dynamic_textures.insert(
                name,
                NativeFurnitureTexture {
                    _texture: texture,
                    srv,
                    descriptor_sha256: asset.descriptor_sha256,
                    width,
                    height,
                    frames: asset.frames,
                },
            );
        }
        let resolved_flag_shield = if packet.resource_flag_masked_shield.is_some() {
            let (resolved_shield, resolved_flags) =
                map_viewer::resolve_resource_flag_assets(resolver, &packet.resources)?;
            if packet.resource_flag_masked_shield.as_ref() != Some(&resolved_shield)
                || packet.resource_flag_assets != resolved_flags
            {
                return Err("XMAP21 resource flag assets changed after preparation".into());
            }
            Some(resolved_shield)
        } else {
            None
        };
        for resource in &packet.resources {
            let Some(path) = resource.controller_flag_texture_path.as_deref() else {
                continue;
            };
            let role = resource_flag_role(path);
            let resolved_shield = resolved_flag_shield
                .as_ref()
                .ok_or("XMAP21 resource flag has no masked-shield descriptor")?;
            let source_descriptor = packet
                .resource_flag_assets
                .iter()
                .find(|asset| asset.logical_path == path)
                .ok_or("XMAP21 resource flag lacks its prepared source descriptor")?;
            let flag = decode_game_texture(resolver, path)?;
            let mask = decode_game_texture(resolver, &resolved_shield.mask_texture_path)?;
            let overlay = decode_game_texture(resolver, &resolved_shield.overlay_texture_path)?;
            if flag.width == 0
                || flag.height == 0
                || mask.width == 0
                || mask.height == 0
                || overlay.width == 0
                || overlay.height == 0
            {
                return Err("XMAP21 GFX_flag_smallest has an empty GPU source".into());
            }
            let descriptor_sha256 = sha256_json(&(
                map_viewer::XMAP21_RESOURCE_FLAG_SAMPLING_LAW,
                source_descriptor,
                resolved_shield,
                flag.width,
                flag.height,
                mask.width,
                mask.height,
                overlay.width,
                overlay.height,
                MASKED_FLAG_PIXEL_SHADER_HLSL,
                D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR,
                D3D11_FILTER_MIN_MAG_MIP_POINT,
            ))?;
            if let Some(existing) = self.masked_flags.get(&role) {
                if existing.descriptor_sha256 != descriptor_sha256 {
                    return Err(
                        format!("XMAP21 cached resource flag {path:?} changed descriptor").into(),
                    );
                }
                continue;
            }
            let upload = |image: &game_ui::dds::RgbaImage,
                          descriptor_sha256: String,
                          upload_role: &str|
             -> Result<NativeFurnitureTexture> {
                let width = u32::try_from(image.width)
                    .map_err(|_| format!("{upload_role} width exceeds u32"))?;
                let height = u32::try_from(image.height)
                    .map_err(|_| format!("{upload_role} height exceeds u32"))?;
                let (texture, srv) = unsafe {
                    create_immutable_texture(
                        gfx,
                        image.width,
                        image.height,
                        DXGI_FORMAT_R8G8B8A8_UNORM,
                        &image.pixels,
                        upload_role,
                    )?
                };
                Ok(NativeFurnitureTexture {
                    _texture: texture,
                    srv,
                    descriptor_sha256,
                    width,
                    height,
                    frames: Vec::new(),
                })
            };
            let base = upload(
                &flag,
                sha256_json(&(descriptor_sha256.as_str(), "base", source_descriptor))?,
                &format!("XMAP21 resource controller flag {path}"),
            )?;
            let mask = upload(
                &mask,
                sha256_json(&(
                    descriptor_sha256.as_str(),
                    "mask",
                    resolved_shield.mask_sha256.as_str(),
                ))?,
                "XMAP21 resource flag mask",
            )?;
            let overlay = upload(
                &overlay,
                sha256_json(&(
                    descriptor_sha256.as_str(),
                    "overlay",
                    resolved_shield.overlay_sha256.as_str(),
                ))?,
                "XMAP21 resource flag overlay",
            )?;
            self.masked_flags.insert(
                role,
                NativeMaskedFlagTexture {
                    base,
                    mask,
                    overlay,
                    descriptor_sha256,
                },
            );
        }
        Ok(())
    }

    fn neutral_resource_quads(
        &self,
        icon: &map_viewer::ProjectedResourceFurnitureIcon,
    ) -> Result<ResourceFurnitureQuads> {
        let Some(path) = icon.controller_flag_texture_path.as_deref() else {
            return Ok(ResourceFurnitureQuads {
                row_flags: Vec::new(),
                root_flag: None,
            });
        };
        let role = resource_flag_role(path);
        let texture = self
            .masked_flags
            .get(&role)
            .ok_or_else(|| format!("XMAP21 resource flag {path:?} is not uploaded"))?;
        let root_left = icon.root_hit_rect.left;
        let root_top = icon.root_hit_rect.top;
        let flag_quad = |offset: [i32; 2], row_y: f32| {
            let left = root_left + offset[0] as f32;
            let top = row_y + offset[1] as f32;
            neutral_rect_quad(
                &role,
                [
                    left,
                    top,
                    left + texture.overlay.width as f32,
                    top + texture.overlay.height as f32,
                ],
                [0.0, 0.0, 1.0, 1.0],
            )
        };
        let mut row_flags = Vec::with_capacity(icon.row_count as usize);
        for row_index in 0..icon.row_count {
            let row_top = root_top + row_index as f32 * self.gui.resource_item.height as f32;
            row_flags.push(flag_quad(self.gui.resource_item_flag_offset, row_top));
        }
        Ok(ResourceFurnitureQuads {
            row_flags,
            root_flag: Some(flag_quad(self.gui.resource_flag_offset, root_top)),
        })
    }

    fn neutral_raid_quads(
        &self,
        icon: &map_viewer::ProjectedRaidFurnitureIcon,
    ) -> Result<Vec<(String, NeutralMapSceneQuad)>> {
        let center = [
            icon.root_screen_px[0] + self.gui.raid.content_center_offset[0] as f32,
            icon.root_screen_px[1] + self.gui.raid.content_center_offset[1] as f32,
        ];
        let mut quads = Vec::new();
        for (child_name, name, frame_index) in [
            (
                "raid_map_icon_frame",
                icon.presentation.frame_sprite_name.as_deref(),
                icon.presentation.frame_index,
            ),
            (
                "raid_map_icon_button",
                icon.presentation.button_sprite_name.as_deref(),
                icon.presentation.button_frame,
            ),
            (
                "plus_icon",
                icon.presentation.plus_sprite_name.as_deref(),
                Some(0),
            ),
        ] {
            let Some(name) = name else {
                continue;
            };
            let frame_index = frame_index
                .ok_or_else(|| format!("XMAP21 raid sprite {name:?} has no frame index"))?;
            let texture = self
                .dynamic_textures
                .get(name)
                .ok_or_else(|| format!("XMAP21 raid sprite {name:?} is not uploaded"))?;
            let frame = texture
                .frames
                .get(frame_index as usize)
                .ok_or_else(|| format!("XMAP21 raid sprite {name:?} frame is out of range"))?;
            let half_width = frame.rect.width as f32 * 0.5;
            let half_height = frame.rect.height as f32 * 0.5;
            quads.push((
                child_name.into(),
                neutral_rect_quad(
                    name,
                    [
                        center[0] - half_width,
                        center[1] - half_height,
                        center[0] + half_width,
                        center[1] + half_height,
                    ],
                    [frame.uv.u0, frame.uv.v0, frame.uv.u1, frame.uv.v1],
                ),
            ));
        }
        Ok(quads)
    }

    pub(crate) unsafe fn execute_neutral_quads(
        &self,
        gfx: &Gfx,
        target: &AuthenticatedMapSceneTarget,
        textures: &[NativeMapSceneTextureBinding],
        quads: &[NeutralMapSceneQuad],
        batch_identity_sha256: &str,
    ) -> Result<MapSceneQuadExecutionReceipt> {
        self.icons.execute_neutral(
            gfx.context.as_raw(),
            target,
            textures,
            quads,
            batch_identity_sha256,
        )
    }
}

fn neutral_vp_quad(icon: &ProjectedMapFurnitureIcon) -> NeutralMapSceneQuad {
    let rect = icon.icon_rect;
    let u0 = icon.strip_frame_index as f32 / VP_STRIP_FRAME_COUNT as f32;
    let u1 = (icon.strip_frame_index + 1) as f32 / VP_STRIP_FRAME_COUNT as f32;
    neutral_rect_quad(
        "xmap21.vp-capital-strip",
        [rect.left, rect.top, rect.right, rect.bottom],
        [u0, 0.0, u1, 1.0],
    )
}

fn neutral_rect_quad(texture_role: &str, rect: [f32; 4], uv: [f32; 4]) -> NeutralMapSceneQuad {
    NeutralMapSceneQuad {
        texture_role: texture_role.into(),
        vertices: [
            NeutralMapSceneQuadVertex {
                position_px: [rect[0], rect[1]],
                uv: [uv[0], uv[1]],
            },
            NeutralMapSceneQuadVertex {
                position_px: [rect[2], rect[1]],
                uv: [uv[2], uv[1]],
            },
            NeutralMapSceneQuadVertex {
                position_px: [rect[0], rect[3]],
                uv: [uv[0], uv[3]],
            },
            NeutralMapSceneQuadVertex {
                position_px: [rect[2], rect[3]],
                uv: [uv[2], uv[3]],
            },
        ],
    }
}

fn resource_flag_role(path: &str) -> String {
    format!("xmap21.resource-flag:{path}")
}

fn is_resource_flag_role(role: &str) -> bool {
    role.starts_with("xmap21.resource-flag:")
}

fn resource_flag_mask_role(base_role: &str) -> String {
    format!("{base_role}:mask")
}

fn resource_flag_overlay_role(base_role: &str) -> String {
    format!("{base_role}:overlay")
}

fn decode_game_texture(
    resolver: &GameFileResolver,
    logical_path: &str,
) -> Result<game_ui::dds::RgbaImage> {
    let path = resolver.resolve_required(logical_path, logical_path)?;
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("dds") => Ok(game_ui::dds::decode_file(&path)?.image),
        Some("tga") => Ok(game_ui::tga::decode_file(&path)?.image),
        _ => Err(format!("XMAP21 dynamic flag texture {logical_path:?} is not DDS or TGA").into()),
    }
}

fn map_scene_quad_backend_identity() -> String {
    sha256_text(&format!(
        "{MAP_SCENE_QUAD_BACKEND_SCHEMA}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        FURNITURE_ICON_SHADER_HLSL,
        MASKED_FLAG_PIXEL_SHADER_HLSL,
        D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
        D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR,
        D3D11_FILTER_MIN_MAG_MIP_POINT,
        D3D11_TEXTURE_ADDRESS_CLAMP,
        D3D11_BLEND_SRC_ALPHA,
        D3D11_BLEND_INV_SRC_ALPHA,
        D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST
    ))
}

fn validate_neutral_batch(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    target: &AuthenticatedMapSceneTarget,
    textures: &[NativeMapSceneTextureBinding],
    quads: &[NeutralMapSceneQuad],
    batch_identity_sha256: &str,
) -> Result<()> {
    if context.is_null() {
        return Err("XMAP21 neutral map-scene context is null".into());
    }
    if target.render_target_view.is_null() {
        return Err("XMAP21 neutral map-scene target RTV is null".into());
    }
    if target.width == 0 || target.height == 0 || target.sample_count == 0 {
        return Err("XMAP21 neutral map-scene target descriptor is invalid".into());
    }
    validate_sha256(
        &target.target_identity_sha256,
        "neutral map-scene target identity",
    )?;
    validate_sha256(batch_identity_sha256, "neutral map-scene batch identity")?;
    let mut roles = std::collections::BTreeSet::new();
    for texture in textures {
        if texture.role.is_empty()
            || texture.role.len() > 768
            || texture.role.bytes().any(|byte| byte.is_ascii_control())
            || !roles.insert(texture.role.as_str())
        {
            return Err(format!(
                "XMAP21 neutral map-scene texture role {:?} is invalid or duplicated",
                texture.role
            )
            .into());
        }
        validate_sha256(
            &texture.descriptor_sha256,
            "neutral map-scene texture descriptor",
        )?;
        if texture.shader_resource_view.is_null() {
            return Err(format!(
                "XMAP21 neutral map-scene texture role {:?} has a null SRV",
                texture.role
            )
            .into());
        }
    }
    for quad in quads {
        if !roles.contains(quad.texture_role.as_str()) {
            return Err(format!(
                "XMAP21 neutral map-scene quad role {:?} is unbound",
                quad.texture_role
            )
            .into());
        }
        if quad.vertices.iter().any(|vertex| {
            vertex
                .position_px
                .iter()
                .chain(vertex.uv.iter())
                .any(|value| !value.is_finite())
        }) {
            return Err("XMAP21 neutral map-scene quad contains a non-finite value".into());
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, role: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{role} is not a lowercase SHA-256").into());
    }
    Ok(())
}

fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn sha256_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("serialize XMAP21 identity payload: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn physical_map_icon_sequence_sha256(
    counter_execution_sha256: &str,
    vp_resource_phase_sha256: &str,
    combat_execution_sha256: &str,
    hub_execution_sha256: &str,
    raid_phase_sha256: &str,
) -> Result<String> {
    for (value, role) in [
        (counter_execution_sha256, "counter bucket execution"),
        (vp_resource_phase_sha256, "VP/resource bucket phase"),
        (combat_execution_sha256, "combat bucket execution"),
        (hub_execution_sha256, "hub bucket execution"),
        (raid_phase_sha256, "raid bucket phase"),
    ] {
        validate_sha256(value, role)?;
    }
    sha256_json(&(
        "hoi4.replay.xmap22-map-icon-physical-order.v1",
        1u8,
        counter_execution_sha256,
        VP_RESOURCE_BUCKETS,
        vp_resource_phase_sha256,
        COMBAT_BUCKETS,
        combat_execution_sha256,
        26u8,
        hub_execution_sha256,
        RAID_BUCKETS,
        raid_phase_sha256,
    ))
}

unsafe fn write_dynamic_buffer<T: Copy>(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    buffer: &ComPtr<ID3D11Buffer>,
    value: &T,
    role: &str,
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        buffer.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, role)?;
    ptr::copy_nonoverlapping(
        value as *const T as *const u8,
        mapped.pData as *mut u8,
        mem::size_of::<T>(),
    );
    (*context).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

unsafe fn write_dynamic_vertices(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    buffer: &ComPtr<ID3D11Buffer>,
    vertices: &[FurnitureIconVertex],
) -> Result<()> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        buffer.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, "Map map-furniture icon vertex buffer")?;
    ptr::copy_nonoverlapping(
        vertices.as_ptr() as *const u8,
        mapped.pData as *mut u8,
        vertices.len() * mem::size_of::<FurnitureIconVertex>(),
    );
    (*context).Unmap(buffer.as_raw() as *mut _, 0);
    Ok(())
}

#[cfg(test)]
mod neutral_backend_tests {
    use super::*;

    #[test]
    fn backend_identity_is_deterministic_and_valid() {
        assert_eq!(
            map_scene_quad_backend_identity(),
            map_scene_quad_backend_identity()
        );
        validate_sha256(&map_scene_quad_backend_identity(), "test backend").unwrap();
    }

    #[test]
    fn masked_flag_backend_carries_the_exact_shader_and_sampler_law() {
        assert!(MASKED_FLAG_PIXEL_SHADER_HLSL.contains("OutColor.a = MaskColor.a;"));
        assert!(MASKED_FLAG_PIXEL_SHADER_HLSL.contains("BaseTexture.Sample(BaseSampler, input.uv)"));
        assert_eq!(D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR, 0x11);
        assert_eq!(D3D11_FILTER_MIN_MAG_MIP_POINT, 0);
        assert!(is_resource_flag_role(&resource_flag_role(
            "gfx/flags/AAA.tga"
        )));
        assert!(!is_resource_flag_role("xmap21.victory-point-strip"));
    }

    #[test]
    fn xmap22_physical_bucket_sequence_is_fixed_and_identity_bound() {
        let counter = "1".repeat(64);
        let vp_resource = "2".repeat(64);
        let combat = "3".repeat(64);
        let hub = "4".repeat(64);
        let raid = "5".repeat(64);
        let exact = physical_map_icon_sequence_sha256(
            &counter,
            &vp_resource,
            &combat,
            &hub,
            &raid,
        )
        .unwrap();
        assert_eq!(exact.len(), 64);
        assert_ne!(
            exact,
            physical_map_icon_sequence_sha256(&counter, &vp_resource, &hub, &combat, &raid)
                .unwrap()
        );
        assert!(
            physical_map_icon_sequence_sha256("bad", &vp_resource, &combat, &hub, &raid)
                .is_err()
        );
    }
}
