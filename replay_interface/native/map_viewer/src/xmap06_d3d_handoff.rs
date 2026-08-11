//! App-device upload handoff for XMAP06 thin-border resources.
//!
//! This module owns the exact resources that [`PreparedThinBorderFrame`]
//! proves: the captured 16-byte interleaved vertex layout, twelve runtime-
//! partitioned `R32_UINT` index buffers, exact ordered per-record draw ranges,
//! the authenticated selection palette and camera-independent selector state,
//! replay-viewer camera fade evaluation, the 32-byte CB2 allocation, the
//! compiled `Effect border` stages, and all six effective-VFS `BorderDiffuse`
//! DDS resources.
//!
//! The runtime bucket carrier is mandatory at this boundary; a missing carrier
//! fails before any D3D mutation. The handoff does not own the foundation scene
//! CB0, render
//! target, or depth target; callers supply those shared scene inputs to the
//! phase executor.

#[cfg(test)]
use crate::PreparedThinBorderFadeWindow;
use crate::{
    AssignedStage, BorderMaterial, GameFileResolver, PreparedThinBorderBucketDraw,
    PreparedThinBorderFadeWindows, PreparedThinBorderFrame, Result, XMAP06_EARLY_OPERATION_NAME,
    XMAP06_EARLY_STAGE_NAME, XMAP06_LATE_OPERATION_NAME, XMAP06_LATE_STAGE_NAME,
};
use serde::Serialize;
use snapshot::sha256::Sha256;
#[cfg(windows)]
use std::sync::Arc;

pub const XMAP06_BORDER_VERTEX_STRIDE_BYTES: u32 = 16;
pub const XMAP06_BORDER_CB2_BYTE_WIDTH: u32 = 32;
pub const XMAP06_BORDER_INDEX_FORMAT_DXGI: u32 = 42; // DXGI_FORMAT_R32_UINT
pub const XMAP06_PHASE_DRAW_RECEIPT_SCHEMA: &str = "hoi4.xmap06.phase-draw-receipt.v4";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Xmap06BorderExecutionPhase {
    Early,
    Late,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap06PhaseExecutionIdentity {
    pub hour_index: i64,
    /// Replay's single gameplay-view target. XMAP06 accepts only main view bit 1;
    /// recorder frustum masks are not playback authority.
    pub active_view_mask: u8,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap06PhaseDrawReceipt {
    pub schema: String,
    pub phase: Xmap06BorderExecutionPhase,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub stage_name: String,
    pub assigned_ordinal: u32,
    pub operation_id: String,
    pub identity: Xmap06PhaseExecutionIdentity,
    pub prepared_frame_sha256: String,
    pub first_bucket: u32,
    pub bucket_count: u32,
    pub active_view_mask: u8,
    pub issued_draw_count: u32,
    pub issued_cb2_update_count: u32,
    pub issued_index_count: u64,
    pub ordered_draws_sha256: String,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap06PhaseDrawReceipt {
    /// Validate the complete receipt before a compositor admits it into the
    /// shared schedule chain. This verifies both the frozen body contract and
    /// the self-excluding canonical receipt digest.
    pub fn validate(&self) -> Result<()> {
        validate_phase_draw_receipt_body(self)?;
        validate_canonical_sha256(&self.packet_draw_receipt_sha256, "packet draw receipt")?;
        if self.packet_draw_receipt_sha256 != packet_draw_receipt_sha256(self)? {
            return Err("XMAP06 packet draw receipt digest mismatch".into());
        }
        Ok(())
    }
}

/// The two exact submit phases recovered from exact-a729.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap06BorderDrawPhase {
    Early,
    Late,
}

/// Exact runtime bucket-bank identity. The authenticated carrier partitions
/// each stored border record into one bank without duplicating geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap06BorderBucketBank {
    Base,
    Secondary,
}

/// One entry in exact-a729's twelve-bucket border submission plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap06BorderDrawBucket {
    pub bucket_index: u32,
    pub phase: Xmap06BorderDrawPhase,
    pub bank: Xmap06BorderBucketBank,
    pub material: BorderMaterial,
}

/// Exact bucket/material ordering. Buckets 0..=3 submit in the early phase;
/// buckets 4..=11 submit in the late phase. The prepared frame's authenticated
/// runtime partition supplies every base- and secondary-bank index stream.
pub const XMAP06_BORDER_DRAW_PLAN: [Xmap06BorderDrawBucket; 12] = [
    Xmap06BorderDrawBucket {
        bucket_index: 0,
        phase: Xmap06BorderDrawPhase::Early,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::Province,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 1,
        phase: Xmap06BorderDrawPhase::Early,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::State,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 2,
        phase: Xmap06BorderDrawPhase::Early,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::Country,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 3,
        phase: Xmap06BorderDrawPhase::Early,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::Impassable,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 4,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::SeaCoast,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 5,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Base,
        material: BorderMaterial::SeaRegion,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 6,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::Province,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 7,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::State,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 8,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::Country,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 9,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::Impassable,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 10,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::SeaCoast,
    },
    Xmap06BorderDrawBucket {
        bucket_index: 11,
        phase: Xmap06BorderDrawPhase::Late,
        bank: Xmap06BorderBucketBank::Secondary,
        material: BorderMaterial::SeaRegion,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap06BorderDdsMipDescriptor {
    pub width: u32,
    pub height: u32,
    pub source_offset: u64,
    pub row_pitch: u32,
    pub slice_pitch: u32,
}

/// Descriptor derived from, and checked against, one admitted DDS payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap06BorderDdsDescriptor {
    pub material: BorderMaterial,
    pub normalized_game_relative_path: String,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub dxgi_format: u32,
    pub payload_sha256: String,
    pub mips: Vec<Xmap06BorderDdsMipDescriptor>,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap06D3dHandoffReceipt {
    pub prepared_frame_sha256: String,
    pub replacement_generation: u64,
    pub vertex_shader_sha256: String,
    pub pixel_shader_sha256: String,
    pub vertex_payload_sha256: String,
    pub ordered_index_payload_sha256: Vec<String>,
    pub cb2_byte_width: u32,
    pub cb2_initial_payload_sha256: String,
    pub ordered_texture_payload_sha256: Vec<String>,
    pub ordered_texture_descriptor_sha256: Vec<String>,
    pub draw_plan_sha256: String,
    pub compiled_programs: u32,
    pub uploaded_buffers: u32,
    pub uploaded_textures: u32,
    pub created_shader_resource_views: u32,
    /// True only when all twelve recorded runtime buckets were uploaded.
    pub scene_execution_ready: bool,
    pub deterministic_sha256: String,
}

#[cfg(windows)]
pub struct NativeXmap06ThinBorderHandoff {
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    vertex_buffer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    cb2: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    samplers: [crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>; 4],
    blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    hour_index: i64,
    early_stage: AssignedStage,
    late_stage: AssignedStage,
    fade_windows: PreparedThinBorderFadeWindows,
    viewer_map_mode_id: String,
    province_fade_uses_viewer_map_mode: bool,
    province_fade_alternate_suppressed: bool,
    province_fade_suppressed: bool,
    selection_palette_f32_bits: Vec<[u32; 4]>,
    classes: Vec<NativeXmap06ClassUpload>,
    buckets: Vec<NativeXmap06BucketUpload>,
    receipt: Xmap06D3dHandoffReceipt,
}

#[cfg(windows)]
struct NativeXmap06ClassUpload {
    material: BorderMaterial,
    _dds_payload: Arc<[u8]>,
    descriptor: Xmap06BorderDdsDescriptor,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    shader_resource_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
}

#[cfg(windows)]
struct NativeXmap06BucketUpload {
    bucket: Xmap06BorderDrawBucket,
    index_count: u32,
    ordered_draws: Vec<PreparedThinBorderBucketDraw>,
    index_buffer: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
}

#[cfg(windows)]
impl NativeXmap06ThinBorderHandoff {
    /// Compile, validate, and upload every XMAP06 resource that the prepared
    /// frame currently owns. No draw is submitted.
    ///
    /// # Safety
    /// `device` must be a live `ID3D11Device` for the duration of this call.
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedThinBorderFrame,
    ) -> Result<Self> {
        Self::create_generation(device, resolver, frame, 0)
    }

    unsafe fn create_generation(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedThinBorderFrame,
        replacement_generation: u64,
    ) -> Result<Self> {
        use crate::shader_compile::{
            build_stage_hlsl_with_defines, d3d_compile_stage,
            derive_a729_engine_map_shader_constants, engine_map_shader_defines, ShaderStage,
            ENGINE_COMPILE_FLAGS,
        };
        use crate::BORDER_SHADER_SOURCE;

        frame.validate()?;
        validate_draw_plan()?;
        if device.is_null() {
            return Err("XMAP06 app-device handoff received a null D3D11 device".into());
        }
        if frame.vertex_stride_bytes != XMAP06_BORDER_VERTEX_STRIDE_BYTES
            || frame.cb2_byte_width != XMAP06_BORDER_CB2_BYTE_WIDTH
        {
            return Err("XMAP06 prepared frame differs from the captured IA/CB2 layout".into());
        }
        let runtime_partition = frame
            .runtime_partition
            .as_ref()
            .ok_or("XMAP06 app-device handoff requires an authenticated runtime bucket carrier")?;

        let engine_defines = engine_map_shader_defines(derive_a729_engine_map_shader_constants(
            frame.map_width,
            frame.map_height,
            false,
            0,
        )?);
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            BORDER_SHADER_SOURCE,
            "border",
            ShaderStage::Vertex,
            &engine_defines,
        )?;
        let pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            BORDER_SHADER_SOURCE,
            "border",
            ShaderStage::Pixel,
            &engine_defines,
        )?;
        let vertex_bytecode =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytecode =
            d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let vertex_shader = create_vertex_shader(device, &vertex_bytecode)?;
        let pixel_shader = create_pixel_shader(device, &pixel_bytecode)?;
        let input_layout = create_input_layout(device, &vertex_bytecode)?;
        let vertex_buffer = create_immutable_buffer(
            device,
            &frame.vertices,
            winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
            "XMAP06 captured-layout vertex buffer",
        )?;
        let cb2 = create_cb2(device)?;
        let samplers = create_border_samplers(device)?;
        let blend_state = create_border_blend_state(device)?;
        let depth_state = create_border_depth_state(device)?;
        let rasterizer_state = create_border_rasterizer_state(device)?;

        let mut classes = Vec::with_capacity(frame.classes.len());
        for class in &frame.classes {
            let payload: Arc<[u8]> = resolver
                .read_admitted_used_content(&class.texture_content)?
                .into();
            let descriptor = parse_border_dds_descriptor(
                class.material,
                &class.texture_content.normalized_game_relative_path,
                &payload,
                &class.texture_content.payload_sha256,
            )?;
            let (texture, shader_resource_view) =
                create_dds_texture(device, &payload, &descriptor)?;
            classes.push(NativeXmap06ClassUpload {
                material: class.material,
                _dds_payload: payload,
                descriptor,
                _texture: texture,
                shader_resource_view,
            });
        }
        let mut buckets = Vec::with_capacity(runtime_partition.buckets.len());
        for geometry in &runtime_partition.buckets {
            let plan = XMAP06_BORDER_DRAW_PLAN
                .get(geometry.bucket.bucket_index as usize)
                .copied()
                .ok_or("XMAP06 runtime bucket ordinal exceeds the draw plan")?;
            let index_buffer = if geometry.indices.is_empty() {
                None
            } else {
                Some(create_immutable_buffer(
                    device,
                    &geometry.indices,
                    winapi::um::d3d11::D3D11_BIND_INDEX_BUFFER,
                    &format!(
                        "XMAP06 runtime bucket {} R32 index buffer",
                        plan.bucket_index
                    ),
                )?)
            };
            buckets.push(NativeXmap06BucketUpload {
                bucket: plan,
                index_count: geometry.index_count,
                ordered_draws: geometry.ordered_draws.clone(),
                index_buffer,
            });
        }

        let receipt = handoff_receipt(
            frame,
            replacement_generation,
            &vertex_bytecode,
            &pixel_bytecode,
            &classes,
            &buckets,
        )?;
        Ok(Self {
            vertex_shader,
            pixel_shader,
            input_layout,
            vertex_buffer,
            cb2,
            samplers,
            blend_state,
            depth_state,
            rasterizer_state,
            hour_index: frame.hour_index,
            early_stage: frame.early_stage.clone(),
            late_stage: frame.late_stage.clone(),
            fade_windows: frame.fade_windows,
            viewer_map_mode_id: runtime_partition.viewer_map_mode_id.clone(),
            province_fade_uses_viewer_map_mode: runtime_partition
                .province_fade_uses_viewer_map_mode,
            province_fade_alternate_suppressed: runtime_partition
                .province_fade_alternate_suppressed,
            province_fade_suppressed: runtime_partition.province_fade_suppressed,
            selection_palette_f32_bits: runtime_partition.selection_palette_f32_bits.clone(),
            classes,
            buckets,
            receipt,
        })
    }

    /// Replace all XMAP06-owned resources as one constructed candidate. The
    /// old resource set remains live if any replacement upload fails.
    ///
    /// # Safety
    /// `device` must be the same live device used by `create_on_device`.
    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedThinBorderFrame,
    ) -> Result<&Xmap06D3dHandoffReceipt> {
        frame.validate()?;
        if frame.deterministic_sha256 == self.receipt.prepared_frame_sha256 {
            return Ok(&self.receipt);
        }
        let generation = self
            .receipt
            .replacement_generation
            .checked_add(1)
            .ok_or("XMAP06 replacement generation overflow")?;
        let replacement = Self::create_generation(device, resolver, frame, generation)?;
        *self = replacement;
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap06D3dHandoffReceipt {
        &self.receipt
    }

    pub(crate) fn vertex_shader(&self) -> *mut winapi::um::d3d11::ID3D11VertexShader {
        self.vertex_shader.as_raw()
    }

    pub(crate) fn pixel_shader(&self) -> *mut winapi::um::d3d11::ID3D11PixelShader {
        self.pixel_shader.as_raw()
    }

    pub(crate) fn input_layout(&self) -> *mut winapi::um::d3d11::ID3D11InputLayout {
        self.input_layout.as_raw()
    }

    pub(crate) fn vertex_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.vertex_buffer.as_raw()
    }

    pub(crate) fn cb2(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.cb2.as_raw()
    }

    pub(crate) fn bucket_resources(
        &self,
        bucket_index: u32,
    ) -> Result<(
        Option<*mut winapi::um::d3d11::ID3D11Buffer>,
        u32,
        *mut winapi::um::d3d11::ID3D11ShaderResourceView,
        &Xmap06BorderDdsDescriptor,
    )> {
        let bucket = self
            .buckets
            .get(bucket_index as usize)
            .ok_or("XMAP06 native handoff bucket ordinal is out of range")?;
        let class = self
            .classes
            .iter()
            .find(|class| class.material == bucket.bucket.material)
            .ok_or_else(|| {
                format!(
                    "XMAP06 native handoff omitted {} resources",
                    bucket.bucket.material.as_str()
                )
            })?;
        Ok((
            bucket.index_buffer.as_ref().map(|buffer| buffer.as_raw()),
            bucket.index_count,
            class.shader_resource_view.as_raw(),
            &class.descriptor,
        ))
    }

    /// Execute one compositor-authorized border phase on the shared MainScene
    /// target. The centralized compositor owns ordering and constructs its
    /// sequence token from the returned actual-draw receipt; this method
    /// neither creates nor guesses a packet-local sequence token.
    ///
    /// # Safety
    /// Every D3D pointer must belong to the same live device/context as this
    /// handoff and remain valid for the duration of the call.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn execute_phase(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        render_target: *mut winapi::um::d3d11::ID3D11RenderTargetView,
        depth_target: *mut winapi::um::d3d11::ID3D11DepthStencilView,
        viewports: &[winapi::um::d3d11::D3D11_VIEWPORT],
        vs_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
        ps_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
        vs_cb1: Option<*mut winapi::um::d3d11::ID3D11Buffer>,
        ps_cb1: Option<*mut winapi::um::d3d11::ID3D11Buffer>,
        shadow_map: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
        camera_height: f32,
        clock: f32,
        phase: Xmap06BorderExecutionPhase,
        identity: Xmap06PhaseExecutionIdentity,
    ) -> Result<Xmap06PhaseDrawReceipt> {
        use crate::d3d11_device::native_d3d11_hr;
        use std::ptr;
        use winapi::shared::dxgiformat::DXGI_FORMAT_R32_UINT;
        use winapi::um::d3d11::{D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD};
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

        validate_phase_identity(&identity)?;
        if identity.hour_index != self.hour_index {
            return Err("XMAP06 phase identity hour differs from the prepared frame".into());
        }
        if context.is_null()
            || render_target.is_null()
            || depth_target.is_null()
            || vs_cb0.is_null()
            || ps_cb0.is_null()
            || shadow_map.is_null()
            || viewports.is_empty()
            || !camera_height.is_finite()
            || !clock.is_finite()
        {
            return Err("XMAP06 phase execution received incomplete scene bindings".into());
        }

        (*context).VSSetShader(self.vertex_shader.as_raw(), ptr::null_mut(), 0);
        (*context).PSSetShader(self.pixel_shader.as_raw(), ptr::null_mut(), 0);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        let mut vertex_buffer = self.vertex_buffer.as_raw();
        let mut stride = XMAP06_BORDER_VERTEX_STRIDE_BYTES;
        let mut offset = 0u32;
        (*context).IASetVertexBuffers(0, 1, &mut vertex_buffer, &mut stride, &mut offset);
        let mut vs_cb0 = vs_cb0;
        let mut ps_cb0 = ps_cb0;
        (*context).VSSetConstantBuffers(0, 1, &mut vs_cb0);
        (*context).PSSetConstantBuffers(0, 1, &mut ps_cb0);
        if let Some(mut cb1) = vs_cb1 {
            (*context).VSSetConstantBuffers(1, 1, &mut cb1);
        }
        if let Some(mut cb1) = ps_cb1 {
            (*context).PSSetConstantBuffers(1, 1, &mut cb1);
        }
        for (slot, sampler) in self.samplers.iter().enumerate() {
            let mut raw = sampler.as_raw();
            (*context).PSSetSamplers(slot as u32, 1, &mut raw);
        }
        let mut shadow = shadow_map;
        (*context).PSSetShaderResources(3, 1, &mut shadow);
        (*context).RSSetState(self.rasterizer_state.as_raw());
        (*context).RSSetViewports(viewports.len() as u32, viewports.as_ptr());
        (*context).RSSetScissorRects(0, ptr::null());
        (*context).OMSetBlendState(self.blend_state.as_raw(), &[1.0, 1.0, 1.0, 1.0], u32::MAX);
        (*context).OMSetDepthStencilState(self.depth_state.as_raw(), 0);
        let mut rtv = render_target;
        (*context).OMSetRenderTargets(1, &mut rtv, depth_target);

        let (first_bucket, end_bucket, assigned_stage) = match phase {
            Xmap06BorderExecutionPhase::Early => (0usize, 4usize, &self.early_stage),
            Xmap06BorderExecutionPhase::Late => (4usize, 12usize, &self.late_stage),
        };
        assigned_stage
            .validate()
            .map_err(|error| format!("XMAP06 assigned phase: {error}"))?;
        let (expected_stage_name, expected_operation_name) = match phase {
            Xmap06BorderExecutionPhase::Early => {
                (XMAP06_EARLY_STAGE_NAME, XMAP06_EARLY_OPERATION_NAME)
            }
            Xmap06BorderExecutionPhase::Late => {
                (XMAP06_LATE_STAGE_NAME, XMAP06_LATE_OPERATION_NAME)
            }
        };
        if assigned_stage.stage_name != expected_stage_name
            || assigned_stage.operation_name != expected_operation_name
        {
            return Err("XMAP06 native phase received the wrong prepared assignment".into());
        }
        let mut current_cb2 = [0u8; XMAP06_BORDER_CB2_BYTE_WIDTH as usize];
        current_cb2[16..20].copy_from_slice(&clock.to_le_bytes());
        current_cb2[20..24].copy_from_slice(&1.0f32.to_le_bytes());
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
        native_d3d11_hr(
            (*context).Map(
                self.cb2.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            ),
            "Map(XMAP06 initial border CB2)",
        )?;
        ptr::copy_nonoverlapping(
            current_cb2.as_ptr(),
            mapped.pData.cast::<u8>(),
            current_cb2.len(),
        );
        (*context).Unmap(self.cb2.as_raw() as *mut _, 0);
        let mut cb2_raw = self.cb2.as_raw();
        (*context).PSSetConstantBuffers(2, 1, &mut cb2_raw);

        let mut issued_draw_count = 0u32;
        let mut issued_cb2_update_count = 1u32;
        let mut issued_index_count = 0u64;
        let mut ordered_draw_identity = b"XMAP06_ORDERED_PHASE_DRAWS_V3\0".to_vec();
        ordered_draw_identity.push(match phase {
            Xmap06BorderExecutionPhase::Early => 0,
            Xmap06BorderExecutionPhase::Late => 1,
        });
        ordered_draw_identity.push(identity.active_view_mask);
        ordered_draw_identity.extend_from_slice(&camera_height.to_bits().to_le_bytes());
        ordered_draw_identity.extend_from_slice(self.viewer_map_mode_id.as_bytes());
        ordered_draw_identity.push(0);
        ordered_draw_identity.push(u8::from(self.province_fade_uses_viewer_map_mode));
        ordered_draw_identity.push(u8::from(self.province_fade_alternate_suppressed));
        ordered_draw_identity.push(u8::from(self.province_fade_suppressed));
        for bucket in &self.buckets[first_bucket..end_bucket] {
            let Some(index_buffer) = &bucket.index_buffer else {
                if !bucket.ordered_draws.is_empty() {
                    return Err("XMAP06 bucket has draws but no native index buffer".into());
                }
                continue;
            };
            let class = self
                .classes
                .iter()
                .find(|class| class.material == bucket.bucket.material)
                .ok_or("XMAP06 phase execution omitted a material resource")?;
            let mut diffuse = class.shader_resource_view.as_raw();
            (*context).PSSetShaderResources(0, 1, &mut diffuse);
            (*context).IASetIndexBuffer(index_buffer.as_raw(), DXGI_FORMAT_R32_UINT, 0);

            let mut previous_reason = u8::MAX;
            let mut previous_transparency = 1.0f32;
            for draw in &bucket.ordered_draws {
                let transparency = replay_draw_transparency(
                    self.fade_windows,
                    bucket.bucket.bucket_index,
                    draw.selector_reason_code_raw,
                    self.province_fade_suppressed,
                    camera_height,
                )?;
                if transparency <= 0.0 {
                    continue;
                }
                let transparency_f32_bits = transparency.to_bits();
                if cb2_requires_refresh(
                    previous_reason,
                    previous_transparency,
                    draw.selector_reason_code_raw,
                    transparency_f32_bits,
                ) {
                    let color = self
                        .selection_palette_f32_bits
                        .get(draw.selector_reason_code_raw as usize)
                        .ok_or("XMAP06 draw reason exceeds the admitted selection palette")?;
                    for (lane, bits) in color.iter().enumerate() {
                        current_cb2[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
                    }
                    current_cb2[16..20].copy_from_slice(&clock.to_le_bytes());
                    current_cb2[20..24].copy_from_slice(&transparency_f32_bits.to_le_bytes());
                    let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
                    native_d3d11_hr(
                        (*context).Map(
                            self.cb2.as_raw() as *mut _,
                            0,
                            D3D11_MAP_WRITE_DISCARD,
                            0,
                            &mut mapped,
                        ),
                        "Map(XMAP06 per-record border CB2)",
                    )?;
                    ptr::copy_nonoverlapping(
                        current_cb2.as_ptr(),
                        mapped.pData.cast::<u8>(),
                        current_cb2.len(),
                    );
                    (*context).Unmap(self.cb2.as_raw() as *mut _, 0);
                    let mut cb2_raw = self.cb2.as_raw();
                    (*context).PSSetConstantBuffers(2, 1, &mut cb2_raw);
                    issued_cb2_update_count = issued_cb2_update_count
                        .checked_add(1)
                        .ok_or("XMAP06 phase CB2-update count overflow")?;
                    previous_reason = draw.selector_reason_code_raw;
                    previous_transparency = transparency;
                }

                (*context).DrawIndexed(draw.index_count, draw.first_index, 0);
                issued_draw_count = issued_draw_count
                    .checked_add(1)
                    .ok_or("XMAP06 phase draw-count overflow")?;
                issued_index_count = issued_index_count
                    .checked_add(draw.index_count as u64)
                    .ok_or("XMAP06 phase index-count overflow")?;
                ordered_draw_identity.extend_from_slice(&bucket.bucket.bucket_index.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.province_low.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.province_high.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.pair_record_ordinal.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.bucket_draw_ordinal.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.first_index.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&draw.index_count.to_le_bytes());
                ordered_draw_identity.push(draw.selector_reason_code_raw);
                ordered_draw_identity.extend_from_slice(&transparency_f32_bits.to_le_bytes());
                ordered_draw_identity.extend_from_slice(&current_cb2);
            }
        }

        let mut null_srv = ptr::null_mut();
        (*context).PSSetShaderResources(0, 1, &mut null_srv);
        (*context).PSSetShaderResources(3, 1, &mut null_srv);
        let active_view_mask = identity.active_view_mask;
        let mut receipt = Xmap06PhaseDrawReceipt {
            schema: XMAP06_PHASE_DRAW_RECEIPT_SCHEMA.into(),
            phase,
            topology_sha256: assigned_stage.topology_sha256.clone(),
            assignment_sha256: assigned_stage.assignment_sha256.clone(),
            stage_name: assigned_stage.stage_name.clone(),
            assigned_ordinal: assigned_stage.assigned_ordinal,
            operation_id: assigned_stage.operation_name.clone(),
            identity,
            prepared_frame_sha256: self.receipt.prepared_frame_sha256.clone(),
            first_bucket: first_bucket as u32,
            bucket_count: (end_bucket - first_bucket) as u32,
            active_view_mask,
            issued_draw_count,
            issued_cb2_update_count,
            issued_index_count,
            ordered_draws_sha256: Sha256::hex(&ordered_draw_identity),
            packet_draw_receipt_sha256: String::new(),
        };
        receipt.packet_draw_receipt_sha256 = packet_draw_receipt_sha256(&receipt)?;
        Ok(receipt)
    }
}

fn validate_phase_identity(identity: &Xmap06PhaseExecutionIdentity) -> Result<()> {
    if identity.active_view_mask != 1 {
        return Err("XMAP06 phase is not the replay-owned main gameplay view".into());
    }
    for (value, label) in [
        (&identity.content_manifest_sha256, "content manifest"),
        (
            &identity.foundation_content_operation_sha256,
            "foundation content operation",
        ),
        (&identity.viewer_state_sha256, "viewer state"),
        (&identity.main_scene_target_sha256, "MainScene target"),
        (
            &identity.map_copy_receipt_set_sha256,
            "map-copy receipt set",
        ),
    ] {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || value.bytes().all(|byte| byte == b'0')
        {
            return Err(format!("XMAP06 phase {label} identity is not canonical SHA-256").into());
        }
    }
    Ok(())
}

fn validate_canonical_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("XMAP06 phase {label} is not canonical SHA-256").into());
    }
    Ok(())
}

fn replay_draw_transparency(
    fade_windows: PreparedThinBorderFadeWindows,
    absolute_bucket: u32,
    selector_reason_code_raw: u8,
    province_fade_suppressed: bool,
    camera_height: f32,
) -> Result<f32> {
    if absolute_bucket >= XMAP06_BORDER_DRAW_PLAN.len() as u32 {
        return Err("XMAP06 draw transparency received an invalid absolute bucket".into());
    }
    if selector_reason_code_raw != 0 {
        return Ok(1.0);
    }
    match absolute_bucket {
        0 if province_fade_suppressed => Ok(0.0),
        0 => fade_windows.transparency(BorderMaterial::Province, camera_height),
        1 => fade_windows.transparency(BorderMaterial::State, camera_height),
        2..=11 => Ok(1.0),
        _ => unreachable!("absolute bucket range checked above"),
    }
}

fn cb2_requires_refresh(
    previous_reason: u8,
    previous_transparency: f32,
    next_reason: u8,
    next_transparency_f32_bits: u32,
) -> bool {
    previous_reason != next_reason
        || previous_transparency != f32::from_bits(next_transparency_f32_bits)
}

fn packet_draw_receipt_sha256(receipt: &Xmap06PhaseDrawReceipt) -> Result<String> {
    validate_phase_draw_receipt_body(receipt)?;
    let mut canonical = receipt.clone();
    canonical.packet_draw_receipt_sha256.clear();
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("serialize XMAP06 packet draw receipt: {error}"))?;
    Ok(Sha256::hex(&bytes))
}

fn validate_phase_draw_receipt_body(receipt: &Xmap06PhaseDrawReceipt) -> Result<()> {
    validate_phase_identity(&receipt.identity)?;
    for (value, label) in [
        (&receipt.topology_sha256, "topology"),
        (&receipt.assignment_sha256, "assignment"),
        (&receipt.prepared_frame_sha256, "prepared frame"),
        (&receipt.ordered_draws_sha256, "ordered draws"),
    ] {
        validate_canonical_sha256(value, label)?;
    }
    let (stage_name, operation_id, first_bucket, bucket_count) = match receipt.phase {
        Xmap06BorderExecutionPhase::Early => {
            (XMAP06_EARLY_STAGE_NAME, XMAP06_EARLY_OPERATION_NAME, 0, 4)
        }
        Xmap06BorderExecutionPhase::Late => {
            (XMAP06_LATE_STAGE_NAME, XMAP06_LATE_OPERATION_NAME, 4, 8)
        }
    };
    if receipt.schema != XMAP06_PHASE_DRAW_RECEIPT_SCHEMA
        || receipt.stage_name != stage_name
        || receipt.operation_id != operation_id
        || receipt.assigned_ordinal == 0
        || receipt.first_bucket != first_bucket
        || receipt.bucket_count != bucket_count
        || receipt.active_view_mask != receipt.identity.active_view_mask
        || receipt.issued_index_count % 3 != 0
        || (receipt.issued_draw_count == 0) != (receipt.issued_index_count == 0)
        || receipt.issued_cb2_update_count == 0
        || receipt.issued_cb2_update_count
            > receipt.issued_draw_count.checked_add(1).unwrap_or(u32::MAX)
    {
        return Err("XMAP06 phase draw receipt body is invalid".into());
    }
    Ok(())
}

fn validate_draw_plan() -> Result<()> {
    const EXPECTED_BASE: [BorderMaterial; 6] = [
        BorderMaterial::Province,
        BorderMaterial::State,
        BorderMaterial::Country,
        BorderMaterial::Impassable,
        BorderMaterial::SeaCoast,
        BorderMaterial::SeaRegion,
    ];
    for (index, bucket) in XMAP06_BORDER_DRAW_PLAN.iter().enumerate() {
        if bucket.bucket_index != index as u32
            || bucket.material != EXPECTED_BASE[index % 6]
            || bucket.bank
                != if index < 6 {
                    Xmap06BorderBucketBank::Base
                } else {
                    Xmap06BorderBucketBank::Secondary
                }
            || bucket.phase
                != if index < 4 {
                    Xmap06BorderDrawPhase::Early
                } else {
                    Xmap06BorderDrawPhase::Late
                }
        {
            return Err("XMAP06 twelve-bucket draw plan is malformed".into());
        }
    }
    Ok(())
}

fn draw_plan_sha256() -> String {
    let mut identity = b"XMAP06_TWO_PHASE_TWELVE_BUCKET_DRAW_PLAN_V1\0".to_vec();
    for bucket in XMAP06_BORDER_DRAW_PLAN {
        identity.extend_from_slice(&bucket.bucket_index.to_le_bytes());
        identity.push(match bucket.phase {
            Xmap06BorderDrawPhase::Early => 0,
            Xmap06BorderDrawPhase::Late => 1,
        });
        identity.push(match bucket.bank {
            Xmap06BorderBucketBank::Base => 0,
            Xmap06BorderBucketBank::Secondary => 1,
        });
        identity.extend_from_slice(&(bucket.material as u32).to_le_bytes());
    }
    Sha256::hex(&identity)
}

#[cfg(windows)]
fn handoff_receipt(
    frame: &PreparedThinBorderFrame,
    replacement_generation: u64,
    vertex_bytecode: &[u8],
    pixel_bytecode: &[u8],
    classes: &[NativeXmap06ClassUpload],
    buckets: &[NativeXmap06BucketUpload],
) -> Result<Xmap06D3dHandoffReceipt> {
    if classes.len() != BorderMaterial::ALL.len()
        || classes
            .iter()
            .zip(&frame.classes)
            .any(|(upload, class)| upload.material != class.material)
    {
        return Err("XMAP06 native upload material census is incomplete or misordered".into());
    }
    if buckets.len() != XMAP06_BORDER_DRAW_PLAN.len()
        || buckets
            .iter()
            .zip(XMAP06_BORDER_DRAW_PLAN)
            .any(|(upload, expected)| upload.bucket != expected)
    {
        return Err("XMAP06 native upload bucket census is incomplete or misordered".into());
    }
    let runtime_partition = frame
        .runtime_partition
        .as_ref()
        .ok_or("XMAP06 handoff receipt requires a runtime partition")?;
    let cb2_initial = [0u8; XMAP06_BORDER_CB2_BYTE_WIDTH as usize];
    let mut receipt = Xmap06D3dHandoffReceipt {
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        replacement_generation,
        vertex_shader_sha256: Sha256::hex(vertex_bytecode),
        pixel_shader_sha256: Sha256::hex(pixel_bytecode),
        vertex_payload_sha256: Sha256::hex(&frame.vertices),
        ordered_index_payload_sha256: runtime_partition
            .buckets
            .iter()
            .map(|bucket| bucket.indices_sha256.clone())
            .collect(),
        cb2_byte_width: XMAP06_BORDER_CB2_BYTE_WIDTH,
        cb2_initial_payload_sha256: Sha256::hex(&cb2_initial),
        ordered_texture_payload_sha256: classes
            .iter()
            .map(|class| class.descriptor.payload_sha256.clone())
            .collect(),
        ordered_texture_descriptor_sha256: classes
            .iter()
            .map(|class| class.descriptor.descriptor_sha256.clone())
            .collect(),
        draw_plan_sha256: draw_plan_sha256(),
        compiled_programs: 2,
        uploaded_buffers: u32::try_from(
            2usize
                .checked_add(
                    buckets
                        .iter()
                        .filter(|bucket| bucket.index_buffer.is_some())
                        .count(),
                )
                .ok_or("XMAP06 uploaded-buffer census overflow")?,
        )?,
        uploaded_textures: 6,
        created_shader_resource_views: 6,
        scene_execution_ready: true,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = handoff_receipt_sha256(&receipt);
    Ok(receipt)
}

fn handoff_receipt_sha256(receipt: &Xmap06D3dHandoffReceipt) -> String {
    let mut identity = b"XMAP06_D3D_HANDOFF_RECEIPT_V5\0".to_vec();
    identity.extend_from_slice(receipt.prepared_frame_sha256.as_bytes());
    identity.extend_from_slice(&receipt.replacement_generation.to_le_bytes());
    identity.extend_from_slice(receipt.vertex_shader_sha256.as_bytes());
    identity.extend_from_slice(receipt.pixel_shader_sha256.as_bytes());
    identity.extend_from_slice(receipt.vertex_payload_sha256.as_bytes());
    for hash in receipt
        .ordered_index_payload_sha256
        .iter()
        .chain(&receipt.ordered_texture_payload_sha256)
        .chain(&receipt.ordered_texture_descriptor_sha256)
    {
        identity.extend_from_slice(hash.as_bytes());
    }
    identity.extend_from_slice(&receipt.cb2_byte_width.to_le_bytes());
    identity.extend_from_slice(receipt.cb2_initial_payload_sha256.as_bytes());
    identity.extend_from_slice(receipt.draw_plan_sha256.as_bytes());
    identity.extend_from_slice(&receipt.compiled_programs.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_buffers.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_textures.to_le_bytes());
    identity.extend_from_slice(&receipt.created_shader_resource_views.to_le_bytes());
    identity.push(u8::from(receipt.scene_execution_ready));
    Sha256::hex(&identity)
}

fn parse_border_dds_descriptor(
    material: BorderMaterial,
    source: &str,
    bytes: &[u8],
    admitted_payload_sha256: &str,
) -> Result<Xmap06BorderDdsDescriptor> {
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err(format!("XMAP06 {source} is not a complete DDS").into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP06 DDS header is truncated")?
                .try_into()?,
        ))
    };
    if read_u32(4)? != 124 || read_u32(76)? != 32 {
        return Err(format!("XMAP06 {source} has noncanonical DDS header sizes").into());
    }
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let mip_levels = read_u32(28)?.max(1);
    if width == 0 || height == 0 || read_u32(24)?.max(1) != 1 {
        return Err(format!("XMAP06 {source} is not a nonempty Texture2D").into());
    }
    let max_mip_levels = u32::BITS - width.max(height).leading_zeros();
    if mip_levels > max_mip_levels {
        return Err(format!(
            "XMAP06 {source} declares {mip_levels} mip levels, but its {width}x{height} dimensions admit at most {max_mip_levels}"
        )
        .into());
    }
    let caps2 = read_u32(112)?;
    if caps2 & 0x0000_fe00 != 0 {
        return Err(format!("XMAP06 {source} is a cube/volume DDS").into());
    }

    let pixel_flags = read_u32(80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP06 DDS pixel format is truncated")?;
    let (dxgi_format, block_bytes, bytes_per_pixel, payload_offset) = if fourcc == b"DX10" {
        if bytes.len() < 148 {
            return Err(format!("XMAP06 {source} has a truncated DX10 header").into());
        }
        if read_u32(132)? != 3 || read_u32(140)? != 1 || read_u32(136)? & 0x4 != 0 {
            return Err(format!("XMAP06 {source} is not one non-cube Texture2D").into());
        }
        let format = match read_u32(128)? {
            71 | 72 => 72, // BC1_UNORM_SRGB
            74 | 75 => 75, // BC2_UNORM_SRGB
            77 | 78 => 78, // BC3_UNORM_SRGB
            28 | 29 => 29, // R8G8B8A8_UNORM_SRGB
            87 | 91 => 91, // B8G8R8A8_UNORM_SRGB
            other => {
                return Err(format!("XMAP06 {source} has unsupported DXGI format {other}").into())
            }
        };
        let (block, pixel) = match format {
            72 => (8, 0),
            75 | 78 => (16, 0),
            29 | 91 => (0, 4),
            _ => unreachable!(),
        };
        (format, block, pixel, 148usize)
    } else if pixel_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => (72, 8, 0, 128),
            b"DXT3" => (75, 16, 0, 128),
            b"DXT5" => (78, 16, 0, 128),
            _ => {
                return Err(format!(
                    "XMAP06 {source} has unsupported legacy DDS fourcc {:?}",
                    String::from_utf8_lossy(fourcc)
                )
                .into())
            }
        }
    } else {
        let bits = read_u32(88)?;
        let masks = (read_u32(92)?, read_u32(96)?, read_u32(100)?, read_u32(104)?);
        let format = match (bits, masks) {
            (32, (0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000)) => 29,
            (32, (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000)) => 91,
            _ => {
                return Err(format!(
                    "XMAP06 {source} has unsupported RGB DDS layout {bits}/{masks:08x?}"
                )
                .into())
            }
        };
        (format, 0, 4, 128)
    };

    let actual_payload_sha256 = Sha256::hex(bytes);
    if actual_payload_sha256 != admitted_payload_sha256 {
        return Err(format!("XMAP06 {source} differs from its admitted payload identity").into());
    }
    let mut offset = payload_offset;
    let mut mips = Vec::with_capacity(mip_levels as usize);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let rows = mip_height.div_ceil(4).max(1);
            let row = mip_width
                .div_ceil(4)
                .max(1)
                .checked_mul(block_bytes)
                .ok_or("XMAP06 DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(rows)
                    .ok_or("XMAP06 DDS slice pitch overflow")?,
            )
        } else {
            let row = mip_width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP06 DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(mip_height)
                    .ok_or("XMAP06 DDS slice pitch overflow")?,
            )
        };
        let end = offset
            .checked_add(slice_pitch as usize)
            .ok_or("XMAP06 DDS payload range overflow")?;
        bytes
            .get(offset..end)
            .ok_or_else(|| format!("XMAP06 {source} DDS mip {mip} is truncated"))?;
        mips.push(Xmap06BorderDdsMipDescriptor {
            width: mip_width,
            height: mip_height,
            source_offset: offset as u64,
            row_pitch,
            slice_pitch,
        });
        offset = end;
    }

    let mut descriptor = Xmap06BorderDdsDescriptor {
        material,
        normalized_game_relative_path: source.to_string(),
        width,
        height,
        mip_levels,
        dxgi_format,
        payload_sha256: actual_payload_sha256,
        mips,
        descriptor_sha256: String::new(),
    };
    descriptor.descriptor_sha256 = dds_descriptor_sha256(&descriptor);
    Ok(descriptor)
}

fn dds_descriptor_sha256(descriptor: &Xmap06BorderDdsDescriptor) -> String {
    let mut identity = b"XMAP06_BORDER_DDS_DESCRIPTOR_V1\0".to_vec();
    identity.extend_from_slice(&(descriptor.material as u32).to_le_bytes());
    identity.extend_from_slice(descriptor.normalized_game_relative_path.as_bytes());
    identity.extend_from_slice(&descriptor.width.to_le_bytes());
    identity.extend_from_slice(&descriptor.height.to_le_bytes());
    identity.extend_from_slice(&descriptor.mip_levels.to_le_bytes());
    identity.extend_from_slice(&descriptor.dxgi_format.to_le_bytes());
    identity.extend_from_slice(descriptor.payload_sha256.as_bytes());
    for mip in &descriptor.mips {
        identity.extend_from_slice(&mip.width.to_le_bytes());
        identity.extend_from_slice(&mip.height.to_le_bytes());
        identity.extend_from_slice(&mip.source_offset.to_le_bytes());
        identity.extend_from_slice(&mip.row_pitch.to_le_bytes());
        identity.extend_from_slice(&mip.slice_pitch.to_le_bytes());
    }
    Sha256::hex(&identity)
}

#[cfg(windows)]
unsafe fn create_border_samplers(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<[crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>; 4]> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_SAMPLER_DESC,
        D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP,
    };

    let make = |address_v: u32, lod_bias: f32, max_lod: f32, role: &'static str| {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: address_v,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MipLODBias: lod_bias,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: max_lod,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateSamplerState(&desc, &mut raw),
            &format!("CreateSamplerState({role})"),
        )?;
        D3dPtr::from_raw(raw, role)
    };
    Ok([
        make(
            D3D11_TEXTURE_ADDRESS_CLAMP,
            -0.5,
            4.0,
            "XMAP06 BorderDiffuse sampler",
        )?,
        make(
            D3D11_TEXTURE_ADDRESS_WRAP,
            0.0,
            f32::MAX,
            "XMAP06 SnowMudData sampler",
        )?,
        make(
            D3D11_TEXTURE_ADDRESS_WRAP,
            0.0,
            f32::MAX,
            "XMAP06 IntelMap sampler",
        )?,
        make(
            D3D11_TEXTURE_ADDRESS_WRAP,
            0.0,
            f32::MAX,
            "XMAP06 ShadowMap sampler",
        )?,
    ])
}

#[cfg(windows)]
unsafe fn create_border_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD,
        D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO,
    };
    let mut desc: D3D11_BLEND_DESC = std::mem::zeroed();
    desc.RenderTarget[0].BlendEnable = 1;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].RenderTargetWriteMask = 0x7;
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut raw),
        "CreateBlendState(XMAP06 captured alpha blend)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 captured alpha blend")
}

#[cfg(windows)]
unsafe fn create_border_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        D3D11_COMPARISON_LESS_EQUAL, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC,
        D3D11_DEPTH_WRITE_MASK_ZERO, D3D11_STENCIL_OP_KEEP,
    };
    let stencil = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_LESS_EQUAL,
    };
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 1,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
        DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
        StencilEnable: 0,
        StencilReadMask: 0,
        StencilWriteMask: 0,
        FrontFace: stencil,
        BackFace: stencil,
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut raw),
        "CreateDepthStencilState(XMAP06 read-only LESS_EQUAL)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 read-only LESS_EQUAL depth")
}

#[cfg(windows)]
unsafe fn create_border_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{D3D11_CULL_BACK, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC};
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 1,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut raw),
        "CreateRasterizerState(XMAP06 captured border state)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 captured border rasterizer")
}

#[cfg(windows)]
unsafe fn create_vertex_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytecode: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateVertexShader(
            bytecode.as_ptr() as *const _,
            bytecode.len(),
            ptr::null_mut(),
            &mut raw,
        ),
        "CreateVertexShader(XMAP06 Effect border)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 border vertex shader")
}

#[cfg(windows)]
unsafe fn create_pixel_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytecode: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreatePixelShader(
            bytecode.as_ptr() as *const _,
            bytecode.len(),
            ptr::null_mut(),
            &mut raw,
        ),
        "CreatePixelShader(XMAP06 Effect border)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 border pixel shader")
}

#[cfg(windows)]
unsafe fn create_input_layout(
    device: *mut winapi::um::d3d11::ID3D11Device,
    vertex_bytecode: &[u8],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ffi::CString;
    use std::ptr;
    use winapi::um::d3d11::{D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA};

    let position = CString::new("POSITION")?;
    let texcoord = CString::new("TEXCOORD")?;
    let elements = [
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: position.as_ptr(),
            SemanticIndex: 0,
            Format: crate::DXGI_FORMAT_R32G32B32_FLOAT as _,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 0,
            Format: crate::DXGI_FORMAT_R16G16_SINT as _,
            InputSlot: 0,
            AlignedByteOffset: 12,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateInputLayout(
            elements.as_ptr(),
            elements.len() as u32,
            vertex_bytecode.as_ptr() as *const _,
            vertex_bytecode.len(),
            &mut raw,
        ),
        "CreateInputLayout(XMAP06 float3 + R16G16_SINT)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 captured border input layout")
}

#[cfg(windows)]
unsafe fn create_immutable_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    bind_flags: u32,
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_IMMUTABLE};
    if bytes.is_empty() {
        return Err(format!("{role} is empty").into());
    }
    let descriptor = D3D11_BUFFER_DESC {
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
        (*device).CreateBuffer(&descriptor, &initial, &mut raw),
        &format!("CreateBuffer({role})"),
    )?;
    D3dPtr::from_raw(raw, role)
}

#[cfg(windows)]
unsafe fn create_cb2(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_WRITE,
        D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DYNAMIC,
    };
    let zero = [0u8; XMAP06_BORDER_CB2_BYTE_WIDTH as usize];
    let descriptor = D3D11_BUFFER_DESC {
        ByteWidth: XMAP06_BORDER_CB2_BYTE_WIDTH,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: zero.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateBuffer(&descriptor, &initial, &mut raw),
        "CreateBuffer(XMAP06 captured 32-byte CB2)",
    )?;
    D3dPtr::from_raw(raw, "XMAP06 border CB2")
}

#[cfg(windows)]
unsafe fn create_dds_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    descriptor: &Xmap06BorderDdsDescriptor,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_IMMUTABLE,
    };

    let initial = descriptor
        .mips
        .iter()
        .map(|mip| {
            let offset = usize::try_from(mip.source_offset)?;
            let end = offset
                .checked_add(mip.slice_pitch as usize)
                .ok_or("XMAP06 admitted DDS mip range overflow")?;
            let payload = bytes
                .get(offset..end)
                .ok_or("XMAP06 admitted DDS mip payload is truncated")?;
            Ok(D3D11_SUBRESOURCE_DATA {
                pSysMem: payload.as_ptr() as *const _,
                SysMemPitch: mip.row_pitch,
                SysMemSlicePitch: mip.slice_pitch,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: descriptor.dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture_raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, initial.as_ptr(), &mut texture_raw),
        &format!(
            "CreateTexture2D(XMAP06 {})",
            descriptor.normalized_game_relative_path
        ),
    )?;
    let texture = D3dPtr::from_raw(texture_raw, "XMAP06 admitted border DDS")?;
    let mut view_raw = ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view_raw),
        &format!(
            "CreateShaderResourceView(XMAP06 {})",
            descriptor.normalized_game_relative_path
        ),
    )?;
    let view = D3dPtr::from_raw(view_raw, "XMAP06 admitted BorderDiffuse SRV")?;
    Ok((texture, view))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_phase_identity() -> Xmap06PhaseExecutionIdentity {
        Xmap06PhaseExecutionIdentity {
            hour_index: 42,
            active_view_mask: 1,
            content_manifest_sha256: "1".repeat(64),
            foundation_content_operation_sha256: "2".repeat(64),
            viewer_state_sha256: "3".repeat(64),
            main_scene_target_sha256: "4".repeat(64),
            map_copy_receipt_set_sha256: "5".repeat(64),
        }
    }

    fn phase_draw_receipt_fixture() -> Xmap06PhaseDrawReceipt {
        Xmap06PhaseDrawReceipt {
            schema: XMAP06_PHASE_DRAW_RECEIPT_SCHEMA.to_owned(),
            phase: Xmap06BorderExecutionPhase::Early,
            topology_sha256: "7".repeat(64),
            assignment_sha256: "8".repeat(64),
            stage_name: XMAP06_EARLY_STAGE_NAME.to_owned(),
            assigned_ordinal: 1,
            operation_id: XMAP06_EARLY_OPERATION_NAME.to_owned(),
            identity: canonical_phase_identity(),
            prepared_frame_sha256: "6".repeat(64),
            first_bucket: 0,
            bucket_count: 4,
            active_view_mask: 1,
            issued_draw_count: 9,
            issued_cb2_update_count: 6,
            issued_index_count: 108,
            ordered_draws_sha256: "9".repeat(64),
            packet_draw_receipt_sha256: String::new(),
        }
    }

    #[test]
    fn phase_identity_requires_canonical_lowercase_nonzero_hashes() {
        let canonical = canonical_phase_identity();
        validate_phase_identity(&canonical).unwrap();

        let mut uppercase = canonical.clone();
        uppercase.content_manifest_sha256 = "A".repeat(64);
        assert!(validate_phase_identity(&uppercase).is_err());

        let mut noncanonical = canonical.clone();
        noncanonical.viewer_state_sha256 = format!("{}g", "3".repeat(63));
        assert!(validate_phase_identity(&noncanonical).is_err());

        let mut wrong_length = canonical.clone();
        wrong_length.main_scene_target_sha256 = "4".repeat(63);
        assert!(validate_phase_identity(&wrong_length).is_err());

        let mut zero = canonical;
        zero.map_copy_receipt_set_sha256 = "0".repeat(64);
        assert!(validate_phase_identity(&zero).is_err());
    }

    #[test]
    fn replay_camera_drives_exact_per_record_alpha_and_cb2_refresh() {
        let fade_windows = PreparedThinBorderFadeWindows {
            province: PreparedThinBorderFadeWindow {
                near_f32_bits: 200.0f32.to_bits(),
                far_f32_bits: 300.0f32.to_bits(),
            },
            state: PreparedThinBorderFadeWindow {
                near_f32_bits: 400.0f32.to_bits(),
                far_f32_bits: 500.0f32.to_bits(),
            },
        };
        assert_eq!(
            replay_draw_transparency(fade_windows, 0, 0, false, 250.0).unwrap(),
            0.5
        );
        assert_eq!(
            replay_draw_transparency(fade_windows, 0, 0, true, 50.0).unwrap(),
            0.0
        );
        assert_eq!(
            replay_draw_transparency(fade_windows, 1, 0, false, 450.0).unwrap(),
            0.5
        );
        assert_eq!(
            replay_draw_transparency(fade_windows, 6, 0, false, 50_000.0).unwrap(),
            1.0
        );
        assert_eq!(
            replay_draw_transparency(fade_windows, 0, 1, true, 50_000.0).unwrap(),
            1.0
        );
        assert!(replay_draw_transparency(fade_windows, 0, 0, false, f32::NAN).is_err());
        assert!(replay_draw_transparency(fade_windows, 12, 0, false, 250.0).is_err());

        assert!(!cb2_requires_refresh(3, 0.0, 3, (-0.0f32).to_bits()));
        assert!(cb2_requires_refresh(3, 1.0, 4, 1.0f32.to_bits()));
        assert!(cb2_requires_refresh(3, 1.0, 3, 0.5f32.to_bits()));
        let nan_bits = f32::NAN.to_bits();
        assert!(cb2_requires_refresh(3, f32::NAN, 3, nan_bits));
    }

    #[test]
    fn packet_draw_receipt_hash_tracks_seam_ordinal_and_issued_counts() {
        let receipt = phase_draw_receipt_fixture();
        let baseline = packet_draw_receipt_sha256(&receipt).unwrap();
        assert_eq!(baseline, packet_draw_receipt_sha256(&receipt).unwrap());
        assert_eq!(baseline.len(), 64);

        let mut changed_seam_identity = receipt.clone();
        changed_seam_identity.identity.map_copy_receipt_set_sha256 = "7".repeat(64);
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_seam_identity).unwrap()
        );

        let mut changed_ordinal = receipt.clone();
        changed_ordinal.assigned_ordinal += 1;
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_ordinal).unwrap()
        );

        let mut changed_assignment = receipt.clone();
        changed_assignment.assignment_sha256 = "9".repeat(64);
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_assignment).unwrap()
        );

        let mut changed_draw_count = receipt.clone();
        changed_draw_count.issued_draw_count -= 1;
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_draw_count).unwrap()
        );

        let mut changed_index_count = receipt.clone();
        changed_index_count.issued_index_count += 3;
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_index_count).unwrap()
        );

        let mut changed_ordered_draws = receipt.clone();
        changed_ordered_draws.ordered_draws_sha256 = "a".repeat(64);
        assert_ne!(
            baseline,
            packet_draw_receipt_sha256(&changed_ordered_draws).unwrap()
        );

        let mut prefilled_hash = receipt;
        prefilled_hash.packet_draw_receipt_sha256 = "f".repeat(64);
        assert_eq!(
            baseline,
            packet_draw_receipt_sha256(&prefilled_hash).unwrap()
        );
    }

    #[test]
    fn complete_phase_draw_receipt_rejects_body_and_digest_tampering() {
        let mut receipt = phase_draw_receipt_fixture();
        receipt.packet_draw_receipt_sha256 = packet_draw_receipt_sha256(&receipt).unwrap();
        receipt.validate().unwrap();

        let mut wrong_schema = receipt.clone();
        wrong_schema.schema = "hoi4.xmap06.phase-draw-receipt.v0".to_owned();
        assert!(wrong_schema.validate().is_err());

        let mut wrong_view_mask = receipt.clone();
        wrong_view_mask.active_view_mask = 2;
        assert!(wrong_view_mask.validate().is_err());

        let mut impossible_view_mask = receipt.clone();
        impossible_view_mask.identity.active_view_mask = 3;
        impossible_view_mask.active_view_mask = 3;
        assert!(impossible_view_mask.validate().is_err());

        let mut missing_cb2_updates = receipt.clone();
        missing_cb2_updates.issued_cb2_update_count = 0;
        assert!(missing_cb2_updates.validate().is_err());

        let mut changed_body = receipt.clone();
        changed_body.issued_index_count += 1;
        assert!(changed_body.validate().is_err());

        let mut noncanonical_digest = receipt.clone();
        noncanonical_digest.packet_draw_receipt_sha256 = "A".repeat(64);
        assert!(noncanonical_digest.validate().is_err());

        receipt.packet_draw_receipt_sha256 = "f".repeat(64);
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn exact_two_phase_twelve_bucket_plan_is_complete() {
        validate_draw_plan().unwrap();
        assert_eq!(
            XMAP06_BORDER_DRAW_PLAN
                .iter()
                .filter(|bucket| bucket.phase == Xmap06BorderDrawPhase::Early)
                .count(),
            4
        );
        assert_eq!(
            XMAP06_BORDER_DRAW_PLAN
                .iter()
                .filter(|bucket| bucket.bank == Xmap06BorderBucketBank::Secondary)
                .count(),
            6
        );
        assert_eq!(draw_plan_sha256().len(), 64);
    }

    #[test]
    fn descriptor_parser_binds_admitted_dxt5_bytes_and_all_mips() {
        let mut bytes = vec![0u8; 128 + 64 + 16 + 16];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&8u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&8u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&3u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DXT5");
        let payload_sha256 = Sha256::hex(&bytes);
        let descriptor = parse_border_dds_descriptor(
            BorderMaterial::State,
            "map/terrain/border_state_0.dds",
            &bytes,
            &payload_sha256,
        )
        .unwrap();
        assert_eq!(descriptor.dxgi_format, 78);
        assert_eq!(descriptor.mips.len(), 3);
        assert_eq!(descriptor.mips[0].row_pitch, 32);
        assert_eq!(descriptor.mips[0].slice_pitch, 64);
        assert_eq!(descriptor.mips[1].source_offset, 192);
        assert_eq!(descriptor.mips[2].source_offset, 208);
        assert_eq!(descriptor.descriptor_sha256.len(), 64);
    }

    #[test]
    fn descriptor_parser_rejects_bytes_outside_admission() {
        let mut bytes = vec![0u8; 144];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DXT5");
        assert!(parse_border_dds_descriptor(
            BorderMaterial::Province,
            "map/terrain/border_province_0.dds",
            &bytes,
            &"1".repeat(64),
        )
        .is_err());
    }

    #[test]
    fn descriptor_parser_rejects_mip_count_larger_than_full_chain() {
        let mut bytes = vec![0u8; 144];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&4u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&4u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DXT5");
        let payload_sha256 = Sha256::hex(&bytes);
        let error = parse_border_dds_descriptor(
            BorderMaterial::Province,
            "map/terrain/border_province_0.dds",
            &bytes,
            &payload_sha256,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("declares 4 mip levels, but its 4x4 dimensions admit at most 3"));
    }
}
