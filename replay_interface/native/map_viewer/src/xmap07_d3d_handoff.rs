
use crate::{
    GameFileResolver, PreparedWaterFrame, PreparedWaterSampler, ReadyWaterDrawContract, Result,
    WaterDependentResourceBinding, WaterVisibleDrawDependencies,
};
use serde::Serialize;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeXmap07ShaderResourceBinding {
    pub symbol: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub owner_packet: String,
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap07ResourceView<'a> {
    pub binding: NativeXmap07ShaderResourceBinding,
    pub replay_frame_identity_sha256: String,
    pub device_identity: usize,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    _owner: std::marker::PhantomData<&'a NativeXmap07WaterHandoff>,
}

#[cfg(windows)]
impl NativeXmap07ResourceView<'_> {
    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }

    /// and handoff lifetime remain unchanged.
    pub fn at_shader_register(&self, shader_register: u32) -> Self {
        let mut view = self.clone();
        view.binding.shader_register = shader_register;
        view
    }
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap07DependentResourceView<'a> {
    pub binding: WaterDependentResourceBinding,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    _owner: std::marker::PhantomData<&'a ()>,
}

#[cfg(windows)]
impl<'a> NativeXmap07DependentResourceView<'a> {
    fn from_parts(
        binding: WaterDependentResourceBinding,
        raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    ) -> Result<Self> {
        if raw_srv.is_null()
            || binding.owner_packet == "XMAP07"
            || binding.symbol.is_empty()
            || !canonical_sha256(&binding.replay_frame_identity_sha256)
            || !canonical_sha256(&binding.resource_sha256)
            || !canonical_sha256(&binding.descriptor_sha256)
            || match binding.owner_packet.as_str() {
                "XMAP09" | "XMAP10" => binding
                    .camera_view_sha256
                    .as_deref()
                    .is_none_or(|camera| !canonical_sha256(camera)),
                "XMAP05" => binding.camera_view_sha256.is_some(),
                _ => true,
            }
        {
            return Err("XMAP07 received an invalid downstream-owned SRV".into());
        }
        Ok(Self {
            binding,
            raw_srv,
            _owner: std::marker::PhantomData,
        })
    }

    pub fn from_camera_owner<T>(owner: &'a T) -> Result<Self>
    where
        T: crate::NativeXmap04UnderwaterShaderResource + ?Sized,
    {
        Self::from_parts(
            WaterDependentResourceBinding {
                symbol: owner.symbol().into(),
                owner_packet: owner.owner_packet().into(),
                replay_frame_identity_sha256: owner.replay_frame_identity_sha256().into(),
                camera_view_sha256: Some(owner.frame_identity_sha256().into()),
                resource_sha256: owner.resource_sha256().into(),
                descriptor_sha256: owner.descriptor_sha256().into(),
            },
            owner.raw_shader_resource_view(),
        )
    }

    pub fn from_xmap05(
        owner: crate::NativeXmap05ShaderResource<'a>,
        replay_frame_identity_sha256: impl Into<String>,
    ) -> Result<Self> {
        Self::from_parts(
            WaterDependentResourceBinding {
                symbol: owner.symbol.into(),
                owner_packet: owner.owner_packet.into(),
                replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
                camera_view_sha256: None,
                resource_sha256: owner.resource_sha256.into(),
                descriptor_sha256: owner.descriptor_sha256.into(),
            },
            owner.shader_resource_view,
        )
    }

    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }
}

#[cfg(windows)]
pub unsafe trait NativeXmap07SceneConstantBufferView {
    fn symbol(&self) -> &str;
    fn owner_packet(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn byte_len(&self) -> u32;
    fn payload_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn composition_receipt_sha256(&self) -> &str;
    fn reflected_layout_sha256(&self) -> &str;
    fn date(&self) -> &str;
    fn hour_index(&self) -> i64;
    fn map_width(&self) -> u32;
    fn map_height(&self) -> u32;
    fn viewport_width(&self) -> u32;
    fn viewport_height(&self) -> u32;
    fn camera_view_sha256(&self) -> &str;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn map_copy_offset_f32_bits(&self) -> u32;
    fn snow_mud_input_sha256(&self) -> &str;
    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer;
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap07PointLightsConstantBufferView<'a> {
    pub symbol: String,
    pub owner_packet: String,
    pub shader_register: u32,
    pub byte_len: u32,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub camera_view_sha256: String,
    raw_buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    _owner: std::marker::PhantomData<&'a crate::NativeXmap09ConstantBufferView<'a>>,
}

#[cfg(windows)]
impl<'a> NativeXmap07PointLightsConstantBufferView<'a> {
    fn from_parts(
        byte_len: u32,
        payload_sha256: impl Into<String>,
        descriptor_sha256: impl Into<String>,
        replay_frame_identity_sha256: impl Into<String>,
        camera_view_sha256: impl Into<String>,
        raw_buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    ) -> Result<Self> {
        let result = Self {
            symbol: "PointLightsCB2".into(),
            owner_packet: "XMAP09".into(),
            shader_register: 2,
            byte_len,
            payload_sha256: payload_sha256.into(),
            descriptor_sha256: descriptor_sha256.into(),
            replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
            camera_view_sha256: camera_view_sha256.into(),
            raw_buffer,
            _owner: std::marker::PhantomData,
        };
        if result.raw_buffer.is_null()
            || result.byte_len == 0
            || result.byte_len % 16 != 0
            || !canonical_sha256(&result.payload_sha256)
            || !canonical_sha256(&result.descriptor_sha256)
            || !canonical_sha256(&result.replay_frame_identity_sha256)
            || !canonical_sha256(&result.camera_view_sha256)
        {
            return Err("XMAP07 received an invalid XMAP09 PointLightsCB2".into());
        }
        Ok(result)
    }

    pub fn from_xmap09(owner: &'a crate::NativeXmap09ConstantBufferView<'a>) -> Result<Self> {
        let binding = &owner.binding;
        if binding.owner_packet != "XMAP09"
            || binding.symbol != "PointLightsCB2"
            || binding.shader_register != 2
        {
            return Err("XMAP07 received the wrong XMAP09 constant buffer".into());
        }
        Self::from_parts(
            binding.byte_len,
            binding.payload_sha256.clone(),
            binding.descriptor_sha256.clone(),
            binding.replay_frame_identity_sha256.clone(),
            binding.camera_frame_sha256.clone(),
            owner.as_raw(),
        )
    }

    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.raw_buffer
    }
}

#[cfg(windows)]
pub struct NativeXmap07SurfaceDrawInputs<'a> {
    pub target: &'a crate::NativeProductionMainSceneTarget,
    pub foundation: &'a portable_render_stream::FoundationRenderEnvelope,
    pub underwater_terrain_frame: Option<&'a crate::PreparedTerrainFrame>,
    pub viewer: &'a crate::ViewerOwnedMapState,
    pub scene_cb0: &'a dyn NativeXmap07SceneConstantBufferView,
    pub point_lights_cb2: Option<&'a NativeXmap07PointLightsConstantBufferView<'a>>,
    pub dependent_resources: &'a [NativeXmap07DependentResourceView<'a>],
    pub map_copy_receipts: &'a [crate::Xmap04MapCopyReceiptRow],
    pub underwater_draw_receipt: Option<&'a crate::Xmap04UnderwaterDrawReceipt>,
    pub draw_ordinal: u32,
    pub map_copy_offset_f32_bits: u32,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap07RefractionTargetView<'a> {
    contract: crate::Xmap04UnderwaterTargetContract,
    raw_rtv: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    _owner: std::marker::PhantomData<&'a NativeXmap07WaterHandoff>,
}

#[cfg(windows)]
impl NativeXmap07RefractionTargetView<'_> {
    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView {
        self.raw_rtv
    }

    pub fn contract(&self) -> &crate::Xmap04UnderwaterTargetContract {
        &self.contract
    }
}

#[cfg(windows)]
unsafe impl crate::NativeXmap04UnderwaterTarget for NativeXmap07RefractionTargetView<'_> {
    fn target_contract(&self) -> &crate::Xmap04UnderwaterTargetContract {
        &self.contract
    }

    fn raw_render_target_view(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView {
        self.raw_rtv
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap07D3dHandoffReceipt {
    pub replay_frame_identity_sha256: String,
    pub snow_mud_authority_sha256: String,
    pub water_frame_sha256: String,
    pub water_program_sha256: String,
    pub refraction_producer_frame_sha256: String,
    pub refraction_producer_allocation_generation: u64,
    pub refraction_producer_allocation_identity_sha256: String,
    pub refraction_resource_identity_sha256: String,
    pub refraction_texture_descriptor_sha256: String,
    pub refraction_rtv_descriptor_sha256: String,
    pub refraction_srv_descriptor_sha256: String,
    pub compiled_programs: u32,
    pub uploaded_static_resources: u32,
    pub uploaded_generated_resources: u32,
    pub created_refraction_targets: u32,
    pub exposed_bindings: Vec<NativeXmap07ShaderResourceBinding>,
    pub deterministic_sha256: String,
}

impl Xmap07D3dHandoffReceipt {
    pub fn validate(&self) -> Result<()> {
        for (digest, label) in [
            (
                &self.replay_frame_identity_sha256,
                "XMAP07 handoff replay frame",
            ),
            (&self.water_frame_sha256, "XMAP07 handoff water frame"),
            (&self.water_program_sha256, "XMAP07 handoff water program"),
            (
                &self.snow_mud_authority_sha256,
                "XMAP07 handoff SnowMud authority",
            ),
            (
                &self.refraction_producer_frame_sha256,
                "XMAP07 refraction producer frame",
            ),
            (
                &self.refraction_producer_allocation_identity_sha256,
                "XMAP07 refraction allocation",
            ),
            (
                &self.refraction_resource_identity_sha256,
                "XMAP07 refraction resource",
            ),
            (
                &self.refraction_texture_descriptor_sha256,
                "XMAP07 refraction texture descriptor",
            ),
            (
                &self.refraction_rtv_descriptor_sha256,
                "XMAP07 refraction RTV descriptor",
            ),
            (
                &self.refraction_srv_descriptor_sha256,
                "XMAP07 refraction SRV descriptor",
            ),
        ] {
            if !canonical_sha256(digest) {
                return Err(format!("{label} is invalid").into());
            }
        }
        let refraction = self
            .exposed_bindings
            .iter()
            .find(|binding| binding.symbol == "WaterRefraction")
            .ok_or("XMAP07 handoff receipt omitted WaterRefraction")?;
        let mut symbols = std::collections::BTreeSet::new();
        if self.exposed_bindings.iter().any(|binding| {
            binding.symbol.is_empty()
                || binding.owner_packet != "XMAP07"
                || binding.resource_type.is_empty()
                || !symbols.insert(binding.symbol.as_str())
                || !canonical_sha256(&binding.resource_sha256)
                || !canonical_sha256(&binding.descriptor_sha256)
        }) {
            return Err("XMAP07 handoff receipt contains an invalid resource binding".into());
        }
        if self.refraction_producer_allocation_generation == 0
            || self.compiled_programs != 1
            || self.uploaded_generated_resources != 1
            || self.created_refraction_targets != 1
            || refraction.owner_packet != "XMAP07"
            || refraction.resource_sha256 != self.refraction_resource_identity_sha256
            || refraction.descriptor_sha256 != self.refraction_srv_descriptor_sha256
            || self.refraction_producer_allocation_identity_sha256
                != crate::xmap04_underwater_producer_allocation_identity_sha256(
                    &self.refraction_producer_frame_sha256,
                    self.refraction_producer_allocation_generation,
                    &self.refraction_texture_descriptor_sha256,
                )?
            || self.refraction_resource_identity_sha256
                != crate::xmap04_underwater_resource_identity_sha256(
                    &self.refraction_producer_frame_sha256,
                    &self.refraction_producer_allocation_identity_sha256,
                    &self.refraction_texture_descriptor_sha256,
                )?
            || self.deterministic_sha256 != xmap07_handoff_receipt_digest(self)
        {
            return Err("XMAP07 native handoff receipt is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap07SurfaceDrawReceipt {
    pub schema: String,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub operation_id: String,
    pub stage_name: String,
    pub assigned_ordinal: u32,
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub draw_ordinal: u32,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub foundation_target_requirement_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub main_scene_allocation_generation: u64,
    pub map_copy_receipt_set_sha256: String,
    pub underwater_draw_receipt_sha256: Option<String>,
    pub ready_contract: ReadyWaterDrawContract,
    pub camera_view_sha256: String,
    pub map_copy_offset_f32_bits: u32,
    pub vertex_count: u32,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap07SurfaceDrawReceipt {
    fn seal(mut self) -> Result<Self> {
        self.packet_draw_receipt_sha256.clear();
        self.validate_body()?;
        self.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        if !canonical_sha256(&self.packet_draw_receipt_sha256) {
            return Err("XMAP07 surface draw receipt digest is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.packet_draw_receipt_sha256.clear();
        if self.packet_draw_receipt_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP07 surface draw receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        self.ready_contract.validate()?;
        let assignment = portable_render_stream::AssignedStage::seal(
            self.topology_sha256.clone(),
            self.stage_name.clone(),
            self.operation_id.clone(),
            self.assigned_ordinal,
        )
        .map_err(|error| format!("XMAP07 surface assigned stage: {error}"))?;
        if self.schema != "hoi4.xmap07.surface-target-bound-draw.v1"
            || !canonical_sha256(&self.topology_sha256)
            || !canonical_sha256(&self.assignment_sha256)
            || assignment.assignment_sha256 != self.assignment_sha256
            || self.operation_id != "water.surface.draw"
            || self.stage_name != "water_surface"
            || self.vertex_count != crate::XMAP07_WATER_VERTEX_COUNT
            || !f32::from_bits(self.map_copy_offset_f32_bits).is_finite()
            || [
                self.content_manifest_sha256.as_str(),
                self.foundation_content_operation_sha256.as_str(),
                self.viewer_state_sha256.as_str(),
                self.foundation_target_requirement_sha256.as_str(),
                self.main_scene_target_sha256.as_str(),
                self.main_scene_resource_identity_sha256.as_str(),
                self.map_copy_receipt_set_sha256.as_str(),
                self.ready_contract.deterministic_sha256.as_str(),
                self.camera_view_sha256.as_str(),
                self.replay_frame_identity_sha256.as_str(),
            ]
            .into_iter()
            .any(|digest| !canonical_sha256(digest))
        {
            return Err("XMAP07 surface draw receipt body is invalid".into());
        }
        if self.main_scene_allocation_generation == 0 {
            return Err(
                "XMAP07 surface draw receipt has no MainScene allocation generation".into(),
            );
        }
        match (
            self.underwater_draw_receipt_sha256.as_deref(),
            self.ready_contract
                .underwater_draw_receipt_sha256
                .as_deref(),
        ) {
            (Some(receipt), Some(ready)) if canonical_sha256(receipt) && receipt == ready => {}
            (None, None) => {}
            _ => {
                return Err(
                    "XMAP07 surface receipt does not bind the accepted underwater completion"
                        .into(),
                )
            }
        }
        if self.ready_contract.replay_frame_identity_sha256 != self.replay_frame_identity_sha256
            || self.ready_contract.camera_view_sha256 != self.camera_view_sha256
        {
            return Err("XMAP07 surface draw receipt mixes replay/camera frames".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
struct NativeWaterTexture {
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    binding: NativeXmap07ShaderResourceBinding,
    refraction_allocation: Option<NativeWaterRefractionAllocation>,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct NativeWaterRefractionAllocation {
    producer_frame_sha256: String,
    producer_allocation_generation: u64,
    producer_allocation_identity_sha256: String,
    resource_identity_sha256: String,
    texture_descriptor_sha256: String,
    rtv_descriptor_sha256: String,
    srv_descriptor_sha256: String,
}

#[cfg(windows)]
pub struct NativeXmap07WaterHandoff {
    device_identity: usize,
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    water_quad: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    water_cb3: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    samplers: Vec<(
        u32,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
    blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    resources: BTreeMap<String, NativeWaterTexture>,
    refraction_rtv: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RenderTargetView>,
    receipt: Xmap07D3dHandoffReceipt,
}

#[cfg(windows)]
impl NativeXmap07WaterHandoff {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedWaterFrame,
    ) -> Result<Self> {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ffi::CString;
        use std::ptr;
        use winapi::um::d3d11::{
            ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader, D3D11_INPUT_ELEMENT_DESC,
        };

        frame.validate()?;
        if device.is_null() {
            return Err("XMAP07 app-device handoff received a null D3D11 device".into());
        }
        let mut vertex = ptr::null_mut::<ID3D11VertexShader>();
        native_d3d11_hr(
            (*device).CreateVertexShader(
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                ptr::null_mut(),
                &mut vertex,
            ),
            "CreateVertexShader(XMAP07 water)",
        )?;
        let vertex_shader = D3dPtr::from_raw(vertex, "XMAP07 water vertex shader")?;
        let mut pixel = ptr::null_mut::<ID3D11PixelShader>();
        native_d3d11_hr(
            (*device).CreatePixelShader(
                frame.program.pixel_bytecode.as_ptr() as *const _,
                frame.program.pixel_bytecode.len(),
                ptr::null_mut(),
                &mut pixel,
            ),
            "CreatePixelShader(XMAP07 water)",
        )?;
        let pixel_shader = D3dPtr::from_raw(pixel, "XMAP07 water pixel shader")?;

        let position = CString::new("POSITION")?;
        let element = D3D11_INPUT_ELEMENT_DESC {
            SemanticName: position.as_ptr(),
            SemanticIndex: 0,
            Format: crate::XMAP07_WATER_VERTEX_FORMAT as _,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: winapi::um::d3d11::D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let mut layout = ptr::null_mut::<ID3D11InputLayout>();
        native_d3d11_hr(
            (*device).CreateInputLayout(
                &element,
                1,
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                &mut layout,
            ),
            "CreateInputLayout(XMAP07 signed-short water quad)",
        )?;
        let input_layout = D3dPtr::from_raw(layout, "XMAP07 water input layout")?;
        let water_quad = create_buffer(
            device,
            &frame.water_quad,
            winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
            "XMAP07 water quad",
        )?;
        let water_cb3 = create_buffer(
            device,
            &frame.water_cb3,
            winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
            "XMAP07 water CB3",
        )?;
        let samplers = frame
            .program
            .samplers
            .iter()
            .map(|sampler| Ok((sampler.shader_register, create_sampler(device, sampler)?)))
            .collect::<Result<Vec<_>>>()?;
        let blend_state = create_water_blend_state(device)?;
        let depth_state = create_water_depth_state(device)?;
        let rasterizer_state = create_water_rasterizer_state(device)?;

        let mut resources = BTreeMap::new();
        let mut static_symbols = frame
            .program
            .samplers
            .iter()
            .filter(|sampler| requires_static_upload(sampler))
            .map(|sampler| sampler.symbol.as_str())
            .collect::<Vec<_>>();
        static_symbols.push("WaterColor");
        static_symbols.sort_unstable();
        static_symbols.dedup();
        for symbol in static_symbols {
            let content = frame.static_resource(symbol).ok_or_else(|| {
                format!("XMAP07 prepared frame omitted static content for {symbol}")
            })?;
            let bytes = resolver.read_admitted_used_content(content)?;
            let register = frame
                .program
                .samplers
                .iter()
                .find(|sampler| sampler.symbol == symbol)
                .map_or(0, |sampler| sampler.shader_register);
            let resource_type = frame
                .program
                .samplers
                .iter()
                .find(|sampler| sampler.symbol == symbol)
                .map_or("Texture2D", |sampler| sampler.resource_type.as_str());
            let texture = if symbol == "HeightTexture" {
                create_height_texture(device, &bytes, content.payload_sha256.clone(), register)?
            } else {
                create_dds_texture(
                    device,
                    &bytes,
                    symbol,
                    resource_type,
                    content.payload_sha256.clone(),
                    register,
                )?
            };
            resources.insert(symbol.to_string(), texture);
        }
        resources.insert(
            "SnowMudTexture".into(),
            create_generated_texture(
                device,
                &frame.snow_mud,
                frame
                    .program
                    .samplers
                    .iter()
                    .find(|sampler| sampler.symbol == "SnowMudTexture")
                    .map_or(9, |sampler| sampler.shader_register),
            )?,
        );
        let (refraction, refraction_rtv) = create_refraction_texture(device, frame)?;
        resources.insert("WaterRefraction".into(), refraction);
        let receipt = handoff_receipt(frame, &resources)?;
        Ok(Self {
            device_identity: device as usize,
            vertex_shader,
            pixel_shader,
            input_layout,
            water_quad,
            water_cb3,
            samplers,
            blend_state,
            depth_state,
            rasterizer_state,
            resources,
            refraction_rtv,
            receipt,
        })
    }

    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedWaterFrame,
    ) -> Result<&Xmap07D3dHandoffReceipt> {
        frame.validate()?;
        if device.is_null() || device as usize != self.device_identity {
            return Err("XMAP07 update received a different D3D11 device".into());
        }
        if frame.program.deterministic_sha256 != self.receipt.water_program_sha256 {
            return Err("XMAP07 water program changed; construct a fresh handoff".into());
        }
        let refraction = self
            .resources
            .get("WaterRefraction")
            .ok_or("XMAP07 retained handoff lost WaterRefraction")?;
        let expected_refraction_srv = crate::xmap04_underwater_srv_descriptor_sha256(
            frame.refraction_dxgi_format,
            4,
            0,
            1,
            0,
            1,
        );
        let expected_texture_descriptor = expected_refraction_texture_descriptor_sha256(frame);
        let recreate_refraction = refraction.binding.descriptor_sha256 != expected_refraction_srv
            || refraction
                .refraction_allocation
                .as_ref()
                .is_none_or(|allocation| {
                    allocation.texture_descriptor_sha256 != expected_texture_descriptor
                });
        if recreate_refraction {
            let (replacement, replacement_rtv) = create_refraction_texture(device, frame)?;
            self.resources.insert("WaterRefraction".into(), replacement);
            self.refraction_rtv = replacement_rtv;
        }
        if frame.deterministic_sha256 == self.receipt.water_frame_sha256 {
            return Ok(&self.receipt);
        }
        self.water_cb3 = create_buffer(
            device,
            &frame.water_cb3,
            winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
            "XMAP07 water CB3",
        )?;
        let snow_register = frame
            .program
            .samplers
            .iter()
            .find(|sampler| sampler.symbol == "SnowMudTexture")
            .map_or(9, |sampler| sampler.shader_register);
        self.resources.insert(
            "SnowMudTexture".into(),
            create_generated_texture(device, &frame.snow_mud, snow_register)?,
        );
        self.receipt = handoff_receipt(frame, &self.resources)?;
        Ok(&self.receipt)
    }

    pub fn resource_view(&self, symbol: &str) -> Result<NativeXmap07ResourceView<'_>> {
        self.receipt.validate()?;
        let resource = self
            .resources
            .get(symbol)
            .ok_or_else(|| format!("XMAP07 has no uploaded resource named {symbol}"))?;
        Ok(NativeXmap07ResourceView {
            binding: resource.binding.clone(),
            replay_frame_identity_sha256: self.receipt.replay_frame_identity_sha256.clone(),
            device_identity: self.device_identity,
            raw_srv: resource.view.as_raw(),
            _owner: std::marker::PhantomData,
        })
    }

    /// Terrain calls this alias because pdxmap names the same generated
    pub fn terrain_snow_mud_view(&self) -> Result<NativeXmap07ResourceView<'_>> {
        let mut view = self.resource_view("SnowMudTexture")?;
        view.binding.symbol = "SnowMudData".into();
        view.binding.shader_register = 1;
        Ok(view)
    }

    pub unsafe fn draw_surface(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        frame: &PreparedWaterFrame,
        inputs: NativeXmap07SurfaceDrawInputs<'_>,
    ) -> Result<Xmap07SurfaceDrawReceipt> {
        use winapi::shared::dxgiformat::DXGI_FORMAT_UNKNOWN;
        use winapi::um::d3d11::D3D11_VIEWPORT;
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

        if context.is_null() {
            return Err("XMAP07 surface draw received a null D3D11 context".into());
        }
        let mut context_device = std::ptr::null_mut();
        (*context).GetDevice(&mut context_device);
        let context_device =
            crate::d3d11_device::D3dPtr::from_raw(context_device, "XMAP07 surface context device")?;
        if context_device.as_raw() as usize != self.device_identity {
            return Err("XMAP07 surface draw context belongs to a different D3D11 device".into());
        }
        if render_target_view_device_identity(inputs.target.color_rtv())? != self.device_identity
            || depth_stencil_view_device_identity(inputs.target.depth_dsv())?
                != self.device_identity
        {
            return Err("XMAP07 surface draw target belongs to a different D3D11 device".into());
        }
        frame.validate()?;
        self.receipt.validate()?;
        if frame.deterministic_sha256 != self.receipt.water_frame_sha256
            || frame.program.deterministic_sha256 != self.receipt.water_program_sha256
            || frame.replay_frame_identity_sha256 != self.receipt.replay_frame_identity_sha256
        {
            return Err("XMAP07 surface draw frame does not match retained GPU resources".into());
        }
        inputs
            .foundation
            .validate()
            .map_err(|error| format!("XMAP07 foundation envelope: {error}"))?;
        if inputs.foundation.hour_index != frame.hour_index {
            return Err("XMAP07 foundation envelope belongs to a different replay hour".into());
        }
        inputs.viewer.validate()?;
        let viewer_camera = inputs
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP07 surface draw requires the XMAP03 viewer camera")?;
        if viewer_camera.deterministic_sha256 != frame.camera_view_sha256
            || inputs.viewer.viewport_width != frame.viewport_width
            || inputs.viewer.viewport_height != frame.viewport_height
            || inputs.viewer.animation_time_f32_bits != frame.temporal.animation_time_f32_bits
        {
            return Err(
                "XMAP07 surface draw viewer state does not match the prepared frame".into(),
            );
        }
        let water_family = inputs
            .foundation
            .families
            .iter()
            .find(|row| row.family == portable_render_stream::MapVisualFamily::WaterIce)
            .ok_or("XMAP07 foundation envelope omitted the WaterIce family")?;
        let water_operation = match &water_family.disposition {
            portable_render_stream::FamilyDisposition::Present {
                ordered_operations, ..
            } => ordered_operations
                .iter()
                .find(|operation| operation.operation_id == "water.surface.draw")
                .ok_or("XMAP07 foundation envelope omitted water.surface.draw")?,
            portable_render_stream::FamilyDisposition::Unavailable { .. } => {
                return Err("XMAP07 cannot draw an unavailable WaterIce family".into())
            }
            portable_render_stream::FamilyDisposition::AuthenticatedEmpty { .. } => {
                return Err("XMAP07 cannot draw an authenticated-empty WaterIce family".into())
            }
            portable_render_stream::FamilyDisposition::NotApplicable { .. } => {
                return Err("XMAP07 cannot draw a not-applicable WaterIce family".into())
            }
            portable_render_stream::FamilyDisposition::DeferredToDraw { .. } => {
                return Err(
                    "XMAP07 water is a foundation-time family and can never defer to draw time"
                        .into(),
                )
            }
        };
        let expected_resources_sha256 =
            crate::production_features::xmap07::ordered_resources_digest(frame);
        let expected_target_sha256 =
            crate::production_features::xmap07::surface_target_digest(frame);
        let expected_operation = frame
            .assigned_stage
            .foundation_operation_identity(
                frame.program.deterministic_sha256.clone(),
                expected_resources_sha256,
                expected_target_sha256,
            )
            .map_err(|error| format!("XMAP07 assigned water operation: {error}"))?;
        if water_operation != &expected_operation {
            return Err("XMAP07 water.surface.draw foundation identity drifted".into());
        }
        let map_copy_receipt_set_sha256 =
            crate::xmap04_map_copy_receipt_set_sha256(frame.map_width, inputs.map_copy_receipts)?;
        let map_copy_row = inputs
            .map_copy_receipts
            .get(usize::try_from(inputs.draw_ordinal)?)
            .ok_or("XMAP07 surface draw ordinal is outside the XMAP04 map-copy receipt set")?;
        if map_copy_row.map_copy_offset_f32_bits != inputs.map_copy_offset_f32_bits {
            return Err(
                "XMAP07 surface draw offset disagrees with its typed XMAP04 map-copy row".into(),
            );
        }
        let target = inputs.target.desc();
        frame.surface_target_requirement.validate_actual(target)?;
        let viewer_state_sha256 = viewer_state_digest(inputs.viewer)?;
        let main_scene_target_sha256 = main_scene_target_digest(target)?;
        let snow_mud_input = frame.scene_snow_mud_input()?;
        let reflected_point_lights_len = (frame.program.effect_name == "water")
            .then(|| reflected_constant_buffer_size(&frame.program.pixel_bytecode, 2))
            .transpose()?;
        exact_map_copy_offset(frame.map_width, inputs.map_copy_offset_f32_bits)?;
        let scene_device = buffer_device_identity(inputs.scene_cb0.raw_buffer())?;
        if inputs.scene_cb0.owner_packet() != "XMAP10"
            || inputs.scene_cb0.symbol() != "SceneCB0"
            || inputs.scene_cb0.shader_register() != 0
            || inputs.scene_cb0.byte_len() != 416
            || scene_device != self.device_identity
            || inputs.scene_cb0.raw_buffer().is_null()
            || !canonical_sha256(inputs.scene_cb0.payload_sha256())
            || !canonical_sha256(inputs.scene_cb0.descriptor_sha256())
            || !canonical_sha256(inputs.scene_cb0.composition_receipt_sha256())
            || !canonical_sha256(inputs.scene_cb0.reflected_layout_sha256())
            || inputs.scene_cb0.reflected_layout_sha256()
                != frame.program.scene_cb0_reflected_layout_sha256
            || !canonical_sha256(inputs.scene_cb0.replay_frame_identity_sha256())
            || inputs.scene_cb0.date() != frame.date.as_str()
            || inputs.scene_cb0.hour_index() != frame.hour_index
            || inputs.scene_cb0.map_width() != frame.map_width
            || inputs.scene_cb0.map_height() != frame.map_height
            || inputs.scene_cb0.viewport_width() != frame.viewport_width
            || inputs.scene_cb0.viewport_height() != frame.viewport_height
            || inputs.scene_cb0.camera_view_sha256() != frame.camera_view_sha256.as_str()
            || inputs.scene_cb0.replay_frame_identity_sha256()
                != frame.replay_frame_identity_sha256.as_str()
            || inputs.scene_cb0.map_copy_offset_f32_bits() != inputs.map_copy_offset_f32_bits
            || inputs.scene_cb0.snow_mud_input_sha256()
                != snow_mud_input.deterministic_sha256.as_str()
        {
            return Err(
                "XMAP07 surface draw requires the matching XMAP10 aggregate SceneCB0 at slot 0"
                    .into(),
            );
        }
        match (frame.program.effect_name.as_str(), inputs.point_lights_cb2) {
            ("water", Some(cb2))
                if cb2.owner_packet == "XMAP09"
                    && cb2.symbol == "PointLightsCB2"
                    && cb2.shader_register == 2
                    && Some(cb2.byte_len) == reflected_point_lights_len
                    && canonical_sha256(&cb2.payload_sha256)
                    && canonical_sha256(&cb2.descriptor_sha256)
                    && cb2.replay_frame_identity_sha256 == frame.replay_frame_identity_sha256
                    && cb2.camera_view_sha256 == frame.camera_view_sha256
                    && buffer_device_identity(cb2.raw_buffer)? == self.device_identity => {}
            ("water_low_gfx", None) => {}
            ("water", _) => {
                return Err("high-gfx XMAP07 surface draw requires XMAP09 PointLightsCB2".into())
            }
            ("water_low_gfx", Some(_)) => {
                return Err("low-gfx XMAP07 surface draw received an unused PointLightsCB2".into())
            }
            _ => return Err("XMAP07 surface draw has an invalid effect selection".into()),
        }
        let bindings = inputs
            .dependent_resources
            .iter()
            .map(|view| view.binding.clone())
            .collect::<Vec<_>>();
        if bindings.iter().any(|binding| {
            binding.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
        }) {
            return Err("XMAP07 surface draw received an SRV from another replay frame".into());
        }
        for view in inputs.dependent_resources {
            if shader_resource_view_device_identity(view.raw_srv)? != self.device_identity {
                return Err("XMAP07 surface draw received an SRV from another D3D11 device".into());
            }
            match view.binding.owner_packet.as_str() {
                "XMAP09" | "XMAP10"
                    if view.binding.camera_view_sha256.as_deref()
                        == Some(frame.camera_view_sha256.as_str()) => {}
                "XMAP05" if view.binding.camera_view_sha256.is_none() => {}
                _ => {
                    return Err(
                        "XMAP07 surface draw received a resource with the wrong camera identity"
                            .into(),
                    )
                }
            }
        }
        let underwater_draw_receipt_sha256 = validate_underwater_completion(
            self,
            frame,
            inputs.foundation,
            target,
            inputs.underwater_terrain_frame,
            inputs.scene_cb0,
            inputs.point_lights_cb2,
            &bindings,
            inputs.underwater_draw_receipt,
        )?;
        let ready_contract =
            frame.admit_visible_draw_dependencies(WaterVisibleDrawDependencies {
                replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
                scene_cb0_sha256: inputs.scene_cb0.payload_sha256().into(),
                scene_cb0_descriptor_sha256: inputs.scene_cb0.descriptor_sha256().into(),
                scene_cb0_composition_receipt_sha256: inputs
                    .scene_cb0
                    .composition_receipt_sha256()
                    .into(),
                scene_cb0_reflected_layout_sha256: inputs
                    .scene_cb0
                    .reflected_layout_sha256()
                    .into(),
                point_lights_cb2_sha256: inputs
                    .point_lights_cb2
                    .map(|buffer| buffer.payload_sha256.clone()),
                point_lights_cb2_descriptor_sha256: inputs
                    .point_lights_cb2
                    .map(|buffer| buffer.descriptor_sha256.clone()),
                point_lights_cb2_replay_frame_identity_sha256: inputs
                    .point_lights_cb2
                    .map(|buffer| buffer.replay_frame_identity_sha256.clone()),
                point_lights_cb2_camera_view_sha256: inputs
                    .point_lights_cb2
                    .map(|buffer| buffer.camera_view_sha256.clone()),
                underwater_draw_receipt_sha256: underwater_draw_receipt_sha256.clone(),
                resources: bindings,
            })?;

        let mut dependent_by_symbol = BTreeMap::new();
        for view in inputs.dependent_resources {
            if dependent_by_symbol
                .insert(view.binding.symbol.as_str(), view)
                .is_some()
            {
                return Err("XMAP07 surface draw received a duplicate dependent SRV".into());
            }
        }
        for sampler in &frame.program.samplers {
            let raw = if sampler.owner_packet == "XMAP07" {
                self.resources
                    .get(&sampler.symbol)
                    .ok_or_else(|| format!("XMAP07 retained resources omitted {}", sampler.symbol))?
                    .view
                    .as_raw()
            } else {
                dependent_by_symbol
                    .get(sampler.symbol.as_str())
                    .ok_or_else(|| {
                        format!(
                            "XMAP07 surface draw omitted downstream SRV {}",
                            sampler.symbol
                        )
                    })?
                    .as_raw()
            };
            (*context).PSSetShaderResources(sampler.shader_register, 1, &raw);
        }

        let mut rtv = inputs.target.color_rtv();
        (*context).OMSetRenderTargets(1, &mut rtv, inputs.target.depth_dsv());
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: frame.viewport_width as f32,
            Height: frame.viewport_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        (*context).RSSetViewports(1, &viewport);
        (*context).RSSetState(self.rasterizer_state.as_raw());
        (*context).OMSetDepthStencilState(self.depth_state.as_raw(), 0);
        (*context).OMSetBlendState(self.blend_state.as_raw(), &[0.0, 0.0, 0.0, 0.0], u32::MAX);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        let vertex_buffer = self.water_quad.as_raw();
        let stride = crate::XMAP07_WATER_VERTEX_STRIDE;
        let offset = 0u32;
        (*context).IASetVertexBuffers(0, 1, &vertex_buffer, &stride, &offset);
        (*context).IASetIndexBuffer(std::ptr::null_mut(), DXGI_FORMAT_UNKNOWN, 0);
        (*context).VSSetShader(self.vertex_shader.as_raw(), std::ptr::null_mut(), 0);
        (*context).PSSetShader(self.pixel_shader.as_raw(), std::ptr::null_mut(), 0);
        let scene_cb0 = inputs.scene_cb0.raw_buffer();
        (*context).VSSetConstantBuffers(0, 1, &scene_cb0);
        (*context).PSSetConstantBuffers(0, 1, &scene_cb0);
        if let Some(point_lights) = inputs.point_lights_cb2 {
            let cb2 = point_lights.as_raw();
            (*context).VSSetConstantBuffers(2, 1, &cb2);
            (*context).PSSetConstantBuffers(2, 1, &cb2);
        }
        let cb3 = self.water_cb3.as_raw();
        (*context).VSSetConstantBuffers(3, 1, &cb3);
        (*context).PSSetConstantBuffers(3, 1, &cb3);
        for (slot, sampler) in &self.samplers {
            let raw = sampler.as_raw();
            (*context).PSSetSamplers(*slot, 1, &raw);
        }
        (*context).Draw(crate::XMAP07_WATER_VERTEX_COUNT, 0);

        Xmap07SurfaceDrawReceipt {
            schema: "hoi4.xmap07.surface-target-bound-draw.v1".into(),
            topology_sha256: frame.assigned_stage.topology_sha256.clone(),
            assignment_sha256: frame.assigned_stage.assignment_sha256.clone(),
            operation_id: frame.assigned_stage.operation_name.clone(),
            stage_name: frame.assigned_stage.stage_name.clone(),
            assigned_ordinal: frame.assigned_stage.assigned_ordinal,
            hour_index: frame.hour_index,
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            draw_ordinal: inputs.draw_ordinal,
            content_manifest_sha256: inputs.foundation.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: inputs
                .foundation
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256,
            foundation_target_requirement_sha256: frame
                .surface_target_requirement
                .deterministic_sha256
                .clone(),
            main_scene_target_sha256,
            main_scene_resource_identity_sha256: inputs
                .target
                .resource_identity_sha256()
                .to_owned(),
            main_scene_allocation_generation: inputs.target.allocation_generation(),
            map_copy_receipt_set_sha256,
            underwater_draw_receipt_sha256,
            ready_contract,
            camera_view_sha256: frame.camera_view_sha256.clone(),
            map_copy_offset_f32_bits: inputs.map_copy_offset_f32_bits,
            vertex_count: crate::XMAP07_WATER_VERTEX_COUNT,
            packet_draw_receipt_sha256: String::new(),
        }
        .seal()
    }

    /// The handoff retains ownership.
    pub fn refraction_render_target(
        &self,
        frame: &PreparedWaterFrame,
        main_scene_target: &crate::ProductionMainSceneTargetDesc,
    ) -> Result<NativeXmap07RefractionTargetView<'_>> {
        frame.validate()?;
        self.receipt.validate()?;
        frame
            .surface_target_requirement
            .validate_actual(main_scene_target)?;
        if frame.deterministic_sha256 != self.receipt.water_frame_sha256 {
            return Err(
                "XMAP07 refraction target frame does not match the retained handoff".into(),
            );
        }
        if main_scene_target.depth_samples != 1
            || main_scene_target.depth_quality != 0
            || !matches!(main_scene_target.depth_format, 20 | 40 | 45 | 55)
        {
            return Err(
                "XMAP07 refraction target requires the authenticated single-sample MainScene depth descriptor"
                    .into(),
            );
        }
        let resource = self
            .resources
            .get("WaterRefraction")
            .ok_or("XMAP07 retained handoff lost WaterRefraction")?;
        let allocation = resource
            .refraction_allocation
            .as_ref()
            .ok_or("XMAP07 WaterRefraction omitted its allocation identity")?;
        let depth_has_stencil = matches!(main_scene_target.depth_format, 20 | 45);
        let depth_descriptor_sha256 = crate::xmap04_underwater_depth_descriptor_sha256(
            frame.refraction_width,
            frame.refraction_height,
            main_scene_target.depth_format,
            main_scene_target.depth_samples,
            main_scene_target.depth_quality,
            1,
            1,
            depth_has_stencil,
        );
        let contract = crate::Xmap04UnderwaterTargetContract {
            owner_packet: "XMAP07".into(),
            width: frame.refraction_width,
            height: frame.refraction_height,
            dxgi_format: frame.refraction_dxgi_format,
            sample_count: 1,
            sample_quality: 0,
            descriptor_sha256: frame.refraction_descriptor_sha256.clone(),
            producer_frame_sha256: allocation.producer_frame_sha256.clone(),
            producer_allocation_generation: allocation.producer_allocation_generation,
            producer_allocation_identity_sha256: allocation
                .producer_allocation_identity_sha256
                .clone(),
            resource_identity_sha256: allocation.resource_identity_sha256.clone(),
            texture_mip_levels: 1,
            texture_array_size: 1,
            texture_usage: 0,
            texture_bind_flags: 0x28,
            texture_cpu_access_flags: 0,
            texture_misc_flags: 0,
            texture_descriptor_sha256: allocation.texture_descriptor_sha256.clone(),
            rtv_dxgi_format: frame.refraction_dxgi_format,
            rtv_view_dimension: 4,
            rtv_mip_slice: 0,
            rtv_first_array_slice: 0,
            rtv_array_size: 1,
            rtv_descriptor_sha256: allocation.rtv_descriptor_sha256.clone(),
            srv_dxgi_format: frame.refraction_dxgi_format,
            srv_view_dimension: 4,
            srv_most_detailed_mip: 0,
            srv_mip_levels: 1,
            srv_first_array_slice: 0,
            srv_array_size: 1,
            srv_descriptor_sha256: allocation.srv_descriptor_sha256.clone(),
            depth_dxgi_format: main_scene_target.depth_format,
            depth_sample_count: main_scene_target.depth_samples,
            depth_sample_quality: main_scene_target.depth_quality,
            depth_mip_levels: 1,
            depth_array_size: 1,
            depth_has_stencil,
            depth_descriptor_sha256,
        };
        contract.validate()?;
        Ok(NativeXmap07RefractionTargetView {
            contract,
            raw_rtv: self.refraction_rtv.as_raw(),
            raw_srv: resource.view.as_raw(),
            _owner: std::marker::PhantomData,
        })
    }

    pub fn receipt(&self) -> &Xmap07D3dHandoffReceipt {
        &self.receipt
    }
}

fn requires_static_upload(sampler: &PreparedWaterSampler) -> bool {
    requires_static_upload_identity(
        &sampler.owner_packet,
        sampler.admitted_static_content.is_some(),
    )
}

fn requires_static_upload_identity(owner_packet: &str, has_static_content: bool) -> bool {
    owner_packet == "XMAP07" && has_static_content
}

#[cfg(windows)]
unsafe fn create_sampler(
    device: *mut winapi::um::d3d11::ID3D11Device,
    sampler: &PreparedWaterSampler,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11SamplerState, D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
        D3D11_TEXTURE_ADDRESS_WRAP,
    };
    let filter = match (
        sampler.min_filter.as_str(),
        sampler.mag_filter.as_str(),
        sampler.mip_filter.as_str(),
    ) {
        ("Linear", "Linear", "Linear") => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        ("Point", "Point", "Point") => D3D11_FILTER_MIN_MAG_MIP_POINT,
        other => {
            return Err(format!(
                "XMAP07 sampler {} has unsupported exact filter tuple {other:?}",
                sampler.symbol
            )
            .into())
        }
    };
    let address = |value: &str| -> Result<_> {
        match value {
            "Wrap" => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
            "Clamp" => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
            _ => Err(format!("XMAP07 sampler address mode {value} is unsupported").into()),
        }
    };
    let parse_f32 = |value: &Option<String>, default: f32, label: &str| -> Result<f32> {
        value.as_deref().map_or(Ok(default), |raw| {
            raw.trim_matches('"')
                .parse::<f32>()
                .map_err(|error| format!("XMAP07 sampler {label} {raw}: {error}").into())
        })
    };
    let parse_u32 = |value: &Option<String>, default: u32, label: &str| -> Result<u32> {
        value.as_deref().map_or(Ok(default), |raw| {
            raw.trim_matches('"')
                .parse::<u32>()
                .map_err(|error| format!("XMAP07 sampler {label} {raw}: {error}").into())
        })
    };
    let desc = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: address(&sampler.address_u)?,
        AddressV: address(&sampler.address_v)?,
        AddressW: address(&sampler.address_u)?,
        MipLODBias: parse_f32(&sampler.mip_map_lod_bias, 0.0, "MipLODBias")?,
        MaxAnisotropy: parse_u32(&sampler.max_anisotropy, 1, "MaxAnisotropy")?,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0, 0.0, 0.0, 0.0],
        MinLOD: parse_f32(&sampler.mip_map_min_lod, 0.0, "MinLOD")?,
        MaxLOD: parse_f32(&sampler.mip_map_max_lod, 1024.0, "MaxLOD")?,
    };
    let mut state = ptr::null_mut::<ID3D11SamplerState>();
    native_d3d11_hr(
        (*device).CreateSamplerState(&desc, &mut state),
        &format!("CreateSamplerState(XMAP07 {})", sampler.symbol),
    )?;
    D3dPtr::from_raw(state, &format!("XMAP07 {} sampler", sampler.symbol))
}

#[cfg(windows)]
unsafe fn create_water_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::mem::zeroed;
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11BlendState, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
        D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_COLOR_WRITE_ENABLE_BLUE,
        D3D11_COLOR_WRITE_ENABLE_GREEN, D3D11_COLOR_WRITE_ENABLE_RED,
    };
    let mut desc: D3D11_BLEND_DESC = zeroed();
    desc.AlphaToCoverageEnable = 0;
    desc.IndependentBlendEnable = 0;
    desc.RenderTarget[0].BlendEnable = 1;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].RenderTargetWriteMask = (D3D11_COLOR_WRITE_ENABLE_RED
        | D3D11_COLOR_WRITE_ENABLE_GREEN
        | D3D11_COLOR_WRITE_ENABLE_BLUE) as u8;
    let mut state = ptr::null_mut::<ID3D11BlendState>();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut state),
        "CreateBlendState(XMAP07 water)",
    )?;
    D3dPtr::from_raw(state, "XMAP07 water blend state")
}

#[cfg(windows)]
unsafe fn create_water_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::mem::zeroed;
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11DepthStencilState, D3D11_COMPARISON_LESS, D3D11_DEPTH_STENCIL_DESC,
        D3D11_DEPTH_WRITE_MASK_ALL,
    };
    let mut desc: D3D11_DEPTH_STENCIL_DESC = zeroed();
    desc.DepthEnable = 1;
    desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
    desc.DepthFunc = D3D11_COMPARISON_LESS;
    desc.StencilEnable = 0;
    let mut state = ptr::null_mut::<ID3D11DepthStencilState>();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut state),
        "CreateDepthStencilState(XMAP07 water)",
    )?;
    D3dPtr::from_raw(state, "XMAP07 water depth state")
}

#[cfg(windows)]
unsafe fn create_water_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11RasterizerState, D3D11_CULL_BACK, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC,
    };
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
    let mut state = ptr::null_mut::<ID3D11RasterizerState>();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut state),
        "CreateRasterizerState(XMAP07 water)",
    )?;
    D3dPtr::from_raw(state, "XMAP07 water rasterizer state")
}

#[cfg(windows)]
unsafe fn create_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    bind_flags: u32,
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{ID3D11Buffer, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA};
    if bytes.is_empty() || bytes.len() > u32::MAX as usize {
        return Err(format!("{role} has an invalid byte length").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes.len() as u32,
        Usage: winapi::um::d3d11::D3D11_USAGE_IMMUTABLE,
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
    let mut buffer = ptr::null_mut::<ID3D11Buffer>();
    native_d3d11_hr(
        (*device).CreateBuffer(&desc, &initial, &mut buffer),
        &format!("CreateBuffer({role})"),
    )?;
    D3dPtr::from_raw(buffer, role)
}

#[cfg(windows)]
unsafe fn create_generated_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    generated: &crate::PreparedWaterGeneratedTexture,
    shader_register: u32,
) -> Result<NativeWaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    };
    let desc = D3D11_TEXTURE2D_DESC {
        Width: generated.width,
        Height: generated.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: generated.dxgi_format as _,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: winapi::um::d3d11::D3D11_USAGE_IMMUTABLE,
        BindFlags: winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: generated.payload.as_ptr() as *const _,
        SysMemPitch: generated.row_pitch,
        SysMemSlicePitch: generated.row_pitch.saturating_mul(generated.height),
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut texture),
        "CreateTexture2D(XMAP07 SnowMudTexture)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP07 SnowMudTexture")?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP07 SnowMudTexture)",
    )?;
    Ok(NativeWaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(view, "XMAP07 SnowMudTexture SRV")?,
        binding: NativeXmap07ShaderResourceBinding {
            symbol: "SnowMudTexture".into(),
            shader_register,
            resource_type: "Texture2D".into(),
            owner_packet: "XMAP07".into(),
            resource_sha256: generated.payload_sha256.clone(),
            descriptor_sha256: generated.descriptor_sha256.clone(),
        },
        refraction_allocation: None,
    })
}

#[cfg(windows)]
unsafe fn create_height_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    payload_sha256: String,
    shader_register: u32,
) -> Result<NativeWaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::path::Path;
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    };
    let image = crate::decode_bmp_indexed(bytes, Path::new("map/heightmap.bmp"))?;
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: 61,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: winapi::um::d3d11::D3D11_USAGE_IMMUTABLE,
        BindFlags: winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: image.pixels.as_ptr() as *const _,
        SysMemPitch: width,
        SysMemSlicePitch: width.saturating_mul(height),
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut texture),
        "CreateTexture2D(XMAP07 HeightTexture)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP07 HeightTexture")?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP07 HeightTexture)",
    )?;
    let descriptor_sha256 =
        Sha256::hex(format!("HeightTexture\0{width}\0{height}\061\01\01").as_bytes());
    Ok(NativeWaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(view, "XMAP07 HeightTexture SRV")?,
        binding: NativeXmap07ShaderResourceBinding {
            symbol: "HeightTexture".into(),
            shader_register,
            resource_type: "Texture2D".into(),
            owner_packet: "XMAP07".into(),
            resource_sha256: payload_sha256,
            descriptor_sha256,
        },
        refraction_allocation: None,
    })
}

#[cfg(windows)]
fn expected_refraction_texture_descriptor_sha256(frame: &PreparedWaterFrame) -> String {
    expected_refraction_texture_descriptor_sha256_for(
        frame.refraction_width,
        frame.refraction_height,
        frame.refraction_dxgi_format,
    )
}

fn expected_refraction_texture_descriptor_sha256_for(
    width: u32,
    height: u32,
    dxgi_format: u32,
) -> String {
    crate::xmap04_underwater_texture_descriptor_sha256(
        width,
        height,
        dxgi_format,
        1,
        0,
        1,
        1,
        0,
        0x28,
        0,
        0,
    )
}

#[cfg(windows)]
unsafe fn create_refraction_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    frame: &PreparedWaterFrame,
) -> Result<(
    NativeWaterTexture,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RenderTargetView>,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_TEXTURE2D_DESC,
    };
    let desc = D3D11_TEXTURE2D_DESC {
        Width: frame.refraction_width,
        Height: frame.refraction_height,
        MipLevels: 1,
        ArraySize: 1,
        Format: frame.refraction_dxgi_format as _,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: winapi::um::d3d11::D3D11_USAGE_DEFAULT,
        BindFlags: winapi::um::d3d11::D3D11_BIND_RENDER_TARGET
            | winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, ptr::null(), &mut texture),
        "CreateTexture2D(XMAP07 WaterRefraction)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP07 WaterRefraction")?;
    let producer_allocation_generation = next_water_refraction_allocation_generation()?;
    let texture_descriptor_sha256 = expected_refraction_texture_descriptor_sha256(frame);
    let producer_allocation_identity_sha256 =
        crate::xmap04_underwater_producer_allocation_identity_sha256(
            &frame.deterministic_sha256,
            producer_allocation_generation,
            &texture_descriptor_sha256,
        )?;
    let resource_identity_sha256 = crate::xmap04_underwater_resource_identity_sha256(
        &frame.deterministic_sha256,
        &producer_allocation_identity_sha256,
        &texture_descriptor_sha256,
    )?;
    let rtv_descriptor_sha256 =
        crate::xmap04_underwater_rtv_descriptor_sha256(frame.refraction_dxgi_format, 4, 0, 0, 1);
    let srv_descriptor_sha256 =
        crate::xmap04_underwater_srv_descriptor_sha256(frame.refraction_dxgi_format, 4, 0, 1, 0, 1);
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP07 WaterRefraction)",
    )?;
    let mut rtv = ptr::null_mut::<ID3D11RenderTargetView>();
    native_d3d11_hr(
        (*device).CreateRenderTargetView(texture.as_raw() as *mut _, ptr::null(), &mut rtv),
        "CreateRenderTargetView(XMAP07 WaterRefraction)",
    )?;
    Ok((
        NativeWaterTexture {
            _texture: texture,
            view: D3dPtr::from_raw(view, "XMAP07 WaterRefraction SRV")?,
            binding: NativeXmap07ShaderResourceBinding {
                symbol: "WaterRefraction".into(),
                shader_register: frame
                    .program
                    .samplers
                    .iter()
                    .find(|sampler| sampler.symbol == "WaterRefraction")
                    .map_or(5, |sampler| sampler.shader_register),
                resource_type: "Texture2D".into(),
                owner_packet: "XMAP07".into(),
                resource_sha256: resource_identity_sha256.clone(),
                descriptor_sha256: srv_descriptor_sha256.clone(),
            },
            refraction_allocation: Some(NativeWaterRefractionAllocation {
                producer_frame_sha256: frame.deterministic_sha256.clone(),
                producer_allocation_generation,
                producer_allocation_identity_sha256,
                resource_identity_sha256,
                texture_descriptor_sha256,
                rtv_descriptor_sha256,
                srv_descriptor_sha256,
            }),
        },
        D3dPtr::from_raw(rtv, "XMAP07 WaterRefraction RTV")?,
    ))
}

#[cfg(windows)]
fn next_water_refraction_allocation_generation() -> Result<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
    NEXT_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .map_err(|_| "XMAP07 WaterRefraction allocation generation exhausted".into())
}

#[cfg(windows)]
unsafe fn create_dds_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    symbol: &str,
    resource_type: &str,
    payload_sha256: String,
    shader_register: u32,
) -> Result<NativeWaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    };
    let mut parsed = parse_dds_upload(bytes)?;
    parsed.dxgi_format = engine_registered_format(symbol, parsed.dxgi_format)?;
    if resource_type == "TextureCube" && !parsed.is_cube {
        return Err(format!("XMAP07 {symbol} requires a DDS cubemap").into());
    }
    if resource_type != "TextureCube" && parsed.is_cube {
        return Err(format!("XMAP07 {symbol} unexpectedly resolved a DDS cubemap").into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: parsed.width,
        Height: parsed.height,
        MipLevels: parsed.mip_levels,
        ArraySize: parsed.array_size,
        Format: parsed.dxgi_format as _,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: winapi::um::d3d11::D3D11_USAGE_IMMUTABLE,
        BindFlags: winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: if parsed.is_cube {
            winapi::um::d3d11::D3D11_RESOURCE_MISC_TEXTURECUBE
        } else {
            0
        },
    };
    let initial = parsed
        .subresources
        .iter()
        .map(|subresource| D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes[subresource.offset..].as_ptr() as *const _,
            SysMemPitch: subresource.row_pitch,
            SysMemSlicePitch: subresource.slice_pitch,
        })
        .collect::<Vec<_>>();
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, initial.as_ptr(), &mut texture),
        &format!("CreateTexture2D(XMAP07 {symbol})"),
    )?;
    let texture = D3dPtr::from_raw(texture, &format!("XMAP07 {symbol}"))?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        &format!("CreateShaderResourceView(XMAP07 {symbol})"),
    )?;
    let descriptor_sha256 = Sha256::hex(
        format!(
            "{symbol}\0{}\0{}\0{}\0{}\0{}\0{}",
            parsed.width,
            parsed.height,
            parsed.mip_levels,
            parsed.array_size,
            parsed.dxgi_format,
            parsed.is_cube
        )
        .as_bytes(),
    );
    Ok(NativeWaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(view, &format!("XMAP07 {symbol} SRV"))?,
        binding: NativeXmap07ShaderResourceBinding {
            symbol: symbol.into(),
            shader_register,
            resource_type: resource_type.into(),
            owner_packet: "XMAP07".into(),
            resource_sha256: payload_sha256,
            descriptor_sha256,
        },
        refraction_allocation: None,
    })
}

#[cfg(windows)]
struct ParsedDdsUpload {
    width: u32,
    height: u32,
    mip_levels: u32,
    array_size: u32,
    dxgi_format: u32,
    is_cube: bool,
    subresources: Vec<DdsSubresource>,
}

#[cfg(windows)]
struct DdsSubresource {
    offset: usize,
    row_pitch: u32,
    slice_pitch: u32,
}

#[cfg(windows)]
fn parse_dds_upload(bytes: &[u8]) -> Result<ParsedDdsUpload> {
    if bytes.get(..4) != Some(b"DDS ") || bytes.len() < 128 {
        return Err("XMAP07 admitted texture is not a complete DDS".into());
    }
    let height = dds_u32(bytes, 12)?;
    let width = dds_u32(bytes, 16)?;
    let mip_levels = dds_u32(bytes, 28)?.max(1);
    let fourcc = dds_u32(bytes, 84)?;
    let caps2 = dds_u32(bytes, 112)?;
    let (data_offset, dxgi_format, array_size, is_cube) = if fourcc == u32::from_le_bytes(*b"DX10")
    {
        if bytes.len() < 148 {
            return Err("XMAP07 DX10 DDS header is truncated".into());
        }
        let format = dds_u32(bytes, 128)?;
        let resource_dimension = dds_u32(bytes, 132)?;
        let misc_flag = dds_u32(bytes, 136)?;
        let declared_array = dds_u32(bytes, 140)?;
        if resource_dimension != 3 || declared_array == 0 {
            return Err("XMAP07 DDS is not a nonempty Texture2D resource".into());
        }
        let cube = misc_flag & 4 != 0;
        let slices = if cube {
            declared_array
                .checked_mul(6)
                .ok_or("XMAP07 DDS cube array size overflow")?
        } else {
            declared_array
        };
        (148usize, format, slices, cube)
    } else {
        let format = legacy_dds_format(bytes, fourcc)?;
        let cube = caps2 & 0xfe00 == 0xfe00;
        (128usize, format, if cube { 6 } else { 1 }, cube)
    };
    if width == 0 || height == 0 {
        return Err("XMAP07 DDS has zero extent".into());
    }
    let mut offset = data_offset;
    let mut subresources = Vec::new();
    for _slice in 0..array_size {
        let mut mip_width = width;
        let mut mip_height = height;
        for _mip in 0..mip_levels {
            let (row_pitch, slice_pitch) = dds_surface_pitch(dxgi_format, mip_width, mip_height)?;
            let end = offset
                .checked_add(slice_pitch as usize)
                .ok_or("XMAP07 DDS subresource offset overflow")?;
            if end > bytes.len() {
                return Err("XMAP07 DDS mip/array payload is truncated".into());
            }
            subresources.push(DdsSubresource {
                offset,
                row_pitch,
                slice_pitch,
            });
            offset = end;
            mip_width = (mip_width / 2).max(1);
            mip_height = (mip_height / 2).max(1);
        }
    }
    if offset != bytes.len() {
        return Err(format!(
            "XMAP07 DDS has {} unconsumed payload bytes",
            bytes.len() - offset
        )
        .into());
    }
    Ok(ParsedDdsUpload {
        width,
        height,
        mip_levels,
        array_size,
        dxgi_format,
        is_cube,
        subresources,
    })
}

#[cfg(windows)]
fn legacy_dds_format(bytes: &[u8], fourcc: u32) -> Result<u32> {
    let named = [
        (u32::from_le_bytes(*b"DXT1"), 71),
        (u32::from_le_bytes(*b"DXT3"), 74),
        (u32::from_le_bytes(*b"DXT5"), 77),
        (u32::from_le_bytes(*b"ATI1"), 80),
        (u32::from_le_bytes(*b"BC4U"), 80),
        (u32::from_le_bytes(*b"ATI2"), 83),
        (u32::from_le_bytes(*b"BC5U"), 83),
    ];
    if let Some((_, format)) = named.iter().find(|(tag, _)| *tag == fourcc) {
        return Ok(*format);
    }
    match fourcc {
        111 => return Ok(54),
        112 => return Ok(34),
        113 => return Ok(10),
        _ if fourcc != 0 => {
            return Err(format!("XMAP07 unsupported legacy DDS fourcc {fourcc:#010x}").into())
        }
        _ => {}
    }
    let bits = dds_u32(bytes, 88)?;
    let masks = (
        dds_u32(bytes, 92)?,
        dds_u32(bytes, 96)?,
        dds_u32(bytes, 100)?,
        dds_u32(bytes, 104)?,
    );
    match (bits, masks) {
        (32, (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000)) => Ok(87),
        (32, (0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000)) => Ok(28),
        (32, (0x00ff0000, 0x0000ff00, 0x000000ff, 0)) => Ok(88),
        (16, (0x000000ff, 0x0000ff00, 0, 0)) => Ok(49),
        (8, (0x000000ff, 0, 0, 0)) => Ok(61),
        _ => Err(format!("XMAP07 unsupported legacy DDS pixel masks {masks:?}/{bits}bpp").into()),
    }
}

#[cfg(windows)]
fn dds_surface_pitch(format: u32, width: u32, height: u32) -> Result<(u32, u32)> {
    let block_bytes = match format {
        70..=72 | 79..=81 => Some(8u32),
        73..=78 | 82..=84 | 94..=99 => Some(16u32),
        _ => None,
    };
    if let Some(block_bytes) = block_bytes {
        let blocks_w = width.div_ceil(4).max(1);
        let blocks_h = height.div_ceil(4).max(1);
        let row = blocks_w
            .checked_mul(block_bytes)
            .ok_or("XMAP07 DDS block row pitch overflow")?;
        return Ok((
            row,
            row.checked_mul(blocks_h)
                .ok_or("XMAP07 DDS block slice pitch overflow")?,
        ));
    }
    let bytes_per_pixel = match format {
        2 => 16,
        10 | 11 | 12 | 13 | 14 => 8,
        27 | 28 | 29 | 34 | 35 | 41 | 42 | 43 | 44 | 45 | 46 | 87 | 88 | 89 | 90 | 91 | 92 | 93 => {
            4
        }
        49 | 50 | 54 | 56 | 57 => 2,
        61 | 62 | 63 | 64 | 65 => 1,
        _ => return Err(format!("XMAP07 unsupported DDS DXGI format {format}").into()),
    };
    let row = width
        .checked_mul(bytes_per_pixel)
        .ok_or("XMAP07 DDS row pitch overflow")?;
    Ok((
        row,
        row.checked_mul(height)
            .ok_or("XMAP07 DDS slice pitch overflow")?,
    ))
}

#[cfg(windows)]
fn engine_registered_format(symbol: &str, file_format: u32) -> Result<u32> {
    if !matches!(symbol, "WaterColor" | "ReflectionCubeMap") {
        return Ok(file_format);
    }
    let srgb = match file_format {
        27..=29 => 29,
        70..=72 => 72,
        73..=75 => 75,
        76..=78 => 78,
        87 | 90 | 91 => 91,
        88 | 92 | 93 => 93,
        97..=99 => 99,
        _ => {
            return Err(format!(
                "XMAP07 {symbol} DDS format {file_format} has no compatible typed sRGB view"
            )
            .into())
        }
    };
    Ok(srgb)
}

#[cfg(windows)]
fn dds_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("XMAP07 DDS header is truncated")?
            .try_into()?,
    ))
}

#[cfg(windows)]
fn handoff_receipt(
    frame: &PreparedWaterFrame,
    resources: &BTreeMap<String, NativeWaterTexture>,
) -> Result<Xmap07D3dHandoffReceipt> {
    let refraction_allocation = resources
        .get("WaterRefraction")
        .and_then(|resource| resource.refraction_allocation.as_ref())
        .ok_or("XMAP07 retained resources omitted the WaterRefraction allocation")?;
    let exposed_bindings = resources
        .values()
        .map(|resource| resource.binding.clone())
        .collect::<Vec<_>>();
    let mut receipt = Xmap07D3dHandoffReceipt {
        replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
        snow_mud_authority_sha256: frame.snow_mud_authority_sha256.clone(),
        water_frame_sha256: frame.deterministic_sha256.clone(),
        water_program_sha256: frame.program.deterministic_sha256.clone(),
        refraction_producer_frame_sha256: refraction_allocation.producer_frame_sha256.clone(),
        refraction_producer_allocation_generation: refraction_allocation
            .producer_allocation_generation,
        refraction_producer_allocation_identity_sha256: refraction_allocation
            .producer_allocation_identity_sha256
            .clone(),
        refraction_resource_identity_sha256: refraction_allocation.resource_identity_sha256.clone(),
        refraction_texture_descriptor_sha256: refraction_allocation
            .texture_descriptor_sha256
            .clone(),
        refraction_rtv_descriptor_sha256: refraction_allocation.rtv_descriptor_sha256.clone(),
        refraction_srv_descriptor_sha256: refraction_allocation.srv_descriptor_sha256.clone(),
        compiled_programs: 1,
        uploaded_static_resources: resources
            .keys()
            .filter(|symbol| *symbol != "SnowMudTexture" && *symbol != "WaterRefraction")
            .count() as u32,
        uploaded_generated_resources: 1,
        created_refraction_targets: 1,
        exposed_bindings,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = xmap07_handoff_receipt_digest(&receipt);
    receipt.validate()?;
    Ok(receipt)
}

fn xmap07_handoff_receipt_digest(receipt: &Xmap07D3dHandoffReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.replay_frame_identity_sha256.as_bytes());
    identity.extend_from_slice(receipt.snow_mud_authority_sha256.as_bytes());
    identity.extend_from_slice(receipt.water_frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.water_program_sha256.as_bytes());
    identity.extend_from_slice(receipt.refraction_producer_frame_sha256.as_bytes());
    identity.extend_from_slice(
        &receipt
            .refraction_producer_allocation_generation
            .to_le_bytes(),
    );
    identity.extend_from_slice(
        receipt
            .refraction_producer_allocation_identity_sha256
            .as_bytes(),
    );
    identity.extend_from_slice(receipt.refraction_resource_identity_sha256.as_bytes());
    identity.extend_from_slice(receipt.refraction_texture_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.refraction_rtv_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.refraction_srv_descriptor_sha256.as_bytes());
    identity.extend_from_slice(&receipt.compiled_programs.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_static_resources.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_generated_resources.to_le_bytes());
    identity.extend_from_slice(&receipt.created_refraction_targets.to_le_bytes());
    for binding in &receipt.exposed_bindings {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.extend_from_slice(&binding.shader_register.to_le_bytes());
        identity.extend_from_slice(binding.resource_type.as_bytes());
        identity.extend_from_slice(binding.owner_packet.as_bytes());
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    Sha256::hex(&identity)
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !value.bytes().all(|byte| byte == b'0')
}

#[cfg(windows)]
fn reflected_constant_buffer_size(bytecode: &[u8], shader_register: u32) -> Result<u32> {
    let reflection = crate::parse_dxbc_reflection(bytecode)?;
    let resource = reflection
        .resources
        .iter()
        .find(|resource| {
            resource.resource_type == 0
                && resource.bind_point == shader_register
                && resource.bind_count == 1
        })
        .ok_or_else(|| {
            format!("XMAP07 pixel program omitted constant buffer register {shader_register}")
        })?;
    let cbuffer = reflection
        .cbuffers
        .iter()
        .find(|cbuffer| cbuffer.name == resource.name)
        .ok_or_else(|| {
            format!(
                "XMAP07 reflection omitted constant buffer layout {}",
                resource.name
            )
        })?;
    if cbuffer.size == 0 || cbuffer.size % 16 != 0 {
        return Err(format!(
            "XMAP07 reflected constant buffer {} has invalid byte length {}",
            resource.name, cbuffer.size
        )
        .into());
    }
    Ok(cbuffer.size)
}

#[cfg(windows)]
unsafe fn buffer_device_identity(buffer: *mut winapi::um::d3d11::ID3D11Buffer) -> Result<usize> {
    if buffer.is_null() {
        return Err("XMAP07 received a null D3D11 buffer".into());
    }
    let mut device = std::ptr::null_mut();
    (*buffer).GetDevice(&mut device);
    let device = crate::d3d11_device::D3dPtr::from_raw(device, "XMAP07 buffer owning device")?;
    Ok(device.as_raw() as usize)
}

#[cfg(windows)]
unsafe fn shader_resource_view_device_identity(
    view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
) -> Result<usize> {
    if view.is_null() {
        return Err("XMAP07 received a null D3D11 shader-resource view".into());
    }
    let mut device = std::ptr::null_mut();
    (*view).GetDevice(&mut device);
    let device = crate::d3d11_device::D3dPtr::from_raw(device, "XMAP07 SRV owning device")?;
    Ok(device.as_raw() as usize)
}

#[cfg(windows)]
unsafe fn render_target_view_device_identity(
    view: *mut winapi::um::d3d11::ID3D11RenderTargetView,
) -> Result<usize> {
    if view.is_null() {
        return Err("XMAP07 received a null D3D11 render-target view".into());
    }
    let mut device = std::ptr::null_mut();
    (*view).GetDevice(&mut device);
    let device = crate::d3d11_device::D3dPtr::from_raw(device, "XMAP07 RTV owning device")?;
    Ok(device.as_raw() as usize)
}

#[cfg(windows)]
unsafe fn depth_stencil_view_device_identity(
    view: *mut winapi::um::d3d11::ID3D11DepthStencilView,
) -> Result<usize> {
    if view.is_null() {
        return Err("XMAP07 received a null D3D11 depth-stencil view".into());
    }
    let mut device = std::ptr::null_mut();
    (*view).GetDevice(&mut device);
    let device = crate::d3d11_device::D3dPtr::from_raw(device, "XMAP07 DSV owning device")?;
    Ok(device.as_raw() as usize)
}

#[cfg(windows)]
fn validate_underwater_completion(
    handoff: &NativeXmap07WaterHandoff,
    frame: &PreparedWaterFrame,
    foundation: &portable_render_stream::FoundationRenderEnvelope,
    target: &crate::ProductionMainSceneTargetDesc,
    terrain_frame: Option<&crate::PreparedTerrainFrame>,
    scene_cb0: &dyn NativeXmap07SceneConstantBufferView,
    point_lights_cb2: Option<&NativeXmap07PointLightsConstantBufferView>,
    dependent_resources: &[WaterDependentResourceBinding],
    receipt: Option<&crate::Xmap04UnderwaterDrawReceipt>,
) -> Result<Option<String>> {
    if !frame.underwater_required {
        if receipt.is_some() || terrain_frame.is_some() {
            return Err(
                "low-GFX XMAP07 surface draw received an unused underwater dependency".into(),
            );
        }
        return Ok(None);
    }
    let terrain_frame = terrain_frame
        .ok_or("high-GFX XMAP07 surface draw requires the current XMAP04 terrain frame")?;
    terrain_frame.validate()?;
    let underwater_program = terrain_frame
        .underwater_program
        .as_ref()
        .ok_or("high-GFX XMAP07 terrain frame omitted its underwater program")?;
    let terrain_family = foundation
        .families
        .iter()
        .find(|row| row.family == portable_render_stream::MapVisualFamily::Terrain)
        .ok_or("XMAP07 foundation envelope omitted the XMAP04 Terrain family")?;
    let _terrain_operation = match &terrain_family.disposition {
        portable_render_stream::FamilyDisposition::Present {
            payload_schema,
            payload_sha256,
            ordered_operations,
            ..
        } if payload_schema == terrain_frame.schema.as_str()
            && payload_sha256 == terrain_frame.deterministic_sha256.as_str() =>
        {
            ordered_operations
                .iter()
                .find(|operation| operation.operation_id == "terrain.geometry.prepare")
                .filter(|operation| {
                    operation.program_sha256 == terrain_frame.program.deterministic_sha256
                })
                .ok_or(
                    "XMAP07 foundation Terrain operation does not match the current XMAP04 program",
                )?
        }
        _ => {
            return Err(
                "XMAP07 foundation Terrain payload does not match the current XMAP04 frame".into(),
            )
        }
    };
    let receipt =
        receipt.ok_or("high-GFX XMAP07 surface draw requires completed XMAP04 underwater draw")?;
    receipt.validate()?;
    let point_lights =
        point_lights_cb2.ok_or("high-GFX XMAP07 underwater receipt requires PointLightsCB2")?;
    let resource = handoff
        .resources
        .get("WaterRefraction")
        .ok_or("XMAP07 retained handoff lost WaterRefraction")?;
    let allocation = resource
        .refraction_allocation
        .as_ref()
        .ok_or("XMAP07 WaterRefraction omitted its allocation identity")?;
    let depth_has_stencil = matches!(target.depth_format, 20 | 45);
    let depth_descriptor_sha256 = crate::xmap04_underwater_depth_descriptor_sha256(
        frame.refraction_width,
        frame.refraction_height,
        target.depth_format,
        target.depth_samples,
        target.depth_quality,
        1,
        1,
        depth_has_stencil,
    );
    let expected_draw_count = u32::try_from(terrain_frame.draws.len())?;
    let expected_light_resources = ordered_underwater_light_resources(dependent_resources)?;
    if terrain_frame.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
        || terrain_frame.camera_frame_sha256 != frame.camera_view_sha256
        || receipt.terrain_frame_sha256 != terrain_frame.deterministic_sha256
        || receipt.underwater_program_sha256 != underwater_program.deterministic_sha256
        || receipt.draw_count != expected_draw_count
        || receipt.ordered_light_resources != expected_light_resources
        || receipt.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
        || receipt.camera_frame_sha256 != frame.camera_view_sha256
        || receipt.xmap09_camera_frame_sha256 != frame.camera_view_sha256
        || receipt.target_resource_identity_sha256 != allocation.resource_identity_sha256
        || receipt.target_producer_frame_sha256 != allocation.producer_frame_sha256
        || receipt.target_producer_allocation_generation
            != allocation.producer_allocation_generation
        || receipt.target_producer_allocation_identity_sha256
            != allocation.producer_allocation_identity_sha256
        || receipt.target_texture_descriptor_sha256 != allocation.texture_descriptor_sha256
        || receipt.target_rtv_descriptor_sha256 != allocation.rtv_descriptor_sha256
        || receipt.target_srv_descriptor_sha256 != allocation.srv_descriptor_sha256
        || receipt.target_descriptor_sha256 != frame.refraction_descriptor_sha256
        || receipt.depth_descriptor_sha256 != depth_descriptor_sha256
        || receipt.scene_cb0_sha256 != scene_cb0.payload_sha256()
        || receipt.scene_cb0_descriptor_sha256 != scene_cb0.descriptor_sha256()
        || receipt.point_lights_cb2_sha256 != point_lights.payload_sha256
        || receipt.point_lights_cb2_descriptor_sha256 != point_lights.descriptor_sha256
    {
        return Err(
            "XMAP07 surface draw received a stale or mismatched XMAP04 underwater receipt".into(),
        );
    }
    Ok(Some(receipt.deterministic_sha256.clone()))
}

fn ordered_underwater_light_resources(
    dependent_resources: &[WaterDependentResourceBinding],
) -> Result<Vec<crate::TerrainDependentResourceBinding>> {
    const EXPECTED: [(&str, &str); 3] = [
        ("LightIndexMap", "XMAP09"),
        ("LightDataMap", "XMAP09"),
        ("ShadowMap", "XMAP10"),
    ];
    EXPECTED
        .iter()
        .map(|(symbol, owner)| {
            let mut matches = dependent_resources
                .iter()
                .filter(|binding| binding.symbol == *symbol && binding.owner_packet == *owner);
            let binding = matches
                .next()
                .ok_or_else(|| format!("XMAP07 underwater dependency omitted {owner}:{symbol}"))?;
            if matches.next().is_some() {
                return Err(
                    format!("XMAP07 underwater dependency duplicated {owner}:{symbol}").into(),
                );
            }
            Ok(crate::TerrainDependentResourceBinding {
                symbol: binding.symbol.clone(),
                owner_packet: binding.owner_packet.clone(),
                resource_sha256: binding.resource_sha256.clone(),
                descriptor_sha256: binding.descriptor_sha256.clone(),
            })
        })
        .collect()
}

fn exact_map_copy_offset(map_width: u32, bits: u32) -> Result<f32> {
    if map_width == 0 {
        return Err("XMAP07 map-copy validation received a zero map width".into());
    }
    let width = map_width as f32;
    let permitted = [0.0f32.to_bits(), (-width).to_bits(), width.to_bits()];
    if !permitted.contains(&bits) {
        return Err("XMAP07 map-copy offset is not the exact center/west/east wrap offset".into());
    }
    Ok(f32::from_bits(bits))
}

#[cfg(windows)]
fn viewer_state_digest(viewer: &crate::ViewerOwnedMapState) -> Result<String> {
    viewer.validate()?;
    Ok(Sha256::hex(&serde_json::to_vec(viewer)?))
}

#[cfg(windows)]
fn main_scene_target_digest(target: &crate::ProductionMainSceneTargetDesc) -> Result<String> {
    target.validate()?;
    Ok(Sha256::hex(&serde_json::to_vec(target)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_and_refraction_resources_are_not_reopened_as_static_content() {
        assert!(requires_static_upload_identity("XMAP07", true));
        assert!(!requires_static_upload_identity("XMAP07", false));
        assert!(!requires_static_upload_identity("XMAP09", false));
    }

    #[test]
    fn map_copy_offset_accepts_only_exact_center_west_east_bits() {
        for offset in [0.0f32, -5632.0, 5632.0] {
            assert_eq!(
                exact_map_copy_offset(5632, offset.to_bits()).unwrap(),
                offset
            );
        }
        assert!(exact_map_copy_offset(5632, (-0.0f32).to_bits()).is_err());
        assert!(exact_map_copy_offset(5632, 1.0f32.to_bits()).is_err());
        assert!(exact_map_copy_offset(5632, f32::NAN.to_bits()).is_err());
        assert!(exact_map_copy_offset(0, 0.0f32.to_bits()).is_err());
    }

    fn water_dependency(
        symbol: &str,
        owner_packet: &str,
        seed: &str,
    ) -> WaterDependentResourceBinding {
        WaterDependentResourceBinding {
            symbol: symbol.into(),
            owner_packet: owner_packet.into(),
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            camera_view_sha256: (owner_packet != "XMAP05").then(|| Sha256::hex(b"camera-frame")),
            resource_sha256: Sha256::hex(format!("{seed}-resource").as_bytes()),
            descriptor_sha256: Sha256::hex(format!("{seed}-descriptor").as_bytes()),
        }
    }

    #[test]
    fn underwater_receipt_lights_are_derived_from_current_xmap07_views() {
        let resources = vec![
            water_dependency("ProvinceSecondaryColorMap", "XMAP05", "political"),
            water_dependency("LightIndexMap", "XMAP09", "index"),
            water_dependency("LightDataMap", "XMAP09", "data"),
            water_dependency("GradientBorderChannel1", "XMAP05", "gradient"),
            water_dependency("ShadowMap", "XMAP10", "shadow"),
        ];
        let actual = ordered_underwater_light_resources(&resources).unwrap();
        assert_eq!(
            actual,
            vec![
                crate::TerrainDependentResourceBinding {
                    symbol: "LightIndexMap".into(),
                    owner_packet: "XMAP09".into(),
                    resource_sha256: Sha256::hex(b"index-resource"),
                    descriptor_sha256: Sha256::hex(b"index-descriptor"),
                },
                crate::TerrainDependentResourceBinding {
                    symbol: "LightDataMap".into(),
                    owner_packet: "XMAP09".into(),
                    resource_sha256: Sha256::hex(b"data-resource"),
                    descriptor_sha256: Sha256::hex(b"data-descriptor"),
                },
                crate::TerrainDependentResourceBinding {
                    symbol: "ShadowMap".into(),
                    owner_packet: "XMAP10".into(),
                    resource_sha256: Sha256::hex(b"shadow-resource"),
                    descriptor_sha256: Sha256::hex(b"shadow-descriptor"),
                },
            ]
        );

        let mut missing = resources.clone();
        missing.retain(|binding| binding.symbol != "LightDataMap");
        assert!(ordered_underwater_light_resources(&missing).is_err());

        let mut duplicate = resources;
        duplicate.push(water_dependency("ShadowMap", "XMAP10", "other-shadow"));
        assert!(ordered_underwater_light_resources(&duplicate).is_err());
    }

    #[test]
    fn surface_draw_source_checks_target_devices_and_current_xmap04_before_mutation() {
        let source = include_str!("xmap07_d3d_handoff.rs");
        let draw_source = &source[source.find("pub unsafe fn draw_surface").unwrap()..];
        let rtv_device = draw_source
            .find("render_target_view_device_identity(inputs.target.color_rtv())?")
            .unwrap();
        let dsv_device = draw_source
            .find("depth_stencil_view_device_identity(inputs.target.depth_dsv())?")
            .unwrap();
        let completion_validation = draw_source
            .find("let underwater_draw_receipt_sha256 = validate_underwater_completion(")
            .unwrap();
        let first_mutation = draw_source.find("(*context).PSSetShaderResources").unwrap();
        assert!(rtv_device < first_mutation);
        assert!(dsv_device < first_mutation);
        assert!(completion_validation < first_mutation);
        for required_check in [
            "receipt.terrain_frame_sha256 != terrain_frame.deterministic_sha256",
            "receipt.underwater_program_sha256 != underwater_program.deterministic_sha256",
            "receipt.draw_count != expected_draw_count",
            "receipt.ordered_light_resources != expected_light_resources",
        ] {
            assert!(source.contains(required_check));
        }
    }

    #[test]
    fn refraction_allocation_identity_distinguishes_same_frame_allocations() {
        let frame = Sha256::hex(b"water-frame");
        let texture = crate::xmap04_underwater_texture_descriptor_sha256(
            1920, 1080, 87, 1, 0, 1, 1, 0, 0x28, 0, 0,
        );
        let first =
            crate::xmap04_underwater_producer_allocation_identity_sha256(&frame, 1, &texture)
                .unwrap();
        let second =
            crate::xmap04_underwater_producer_allocation_identity_sha256(&frame, 2, &texture)
                .unwrap();
        assert_ne!(first, second);
        assert_ne!(
            crate::xmap04_underwater_resource_identity_sha256(&frame, &first, &texture).unwrap(),
            crate::xmap04_underwater_resource_identity_sha256(&frame, &second, &texture).unwrap()
        );
    }

    #[test]
    fn xmap07_refraction_resize_changes_full_texture_identity_not_srv_shape() {
        let full_hd = expected_refraction_texture_descriptor_sha256_for(1920, 1080, 87);
        assert_eq!(
            full_hd,
            expected_refraction_texture_descriptor_sha256_for(1920, 1080, 87)
        );
        assert_ne!(
            full_hd,
            expected_refraction_texture_descriptor_sha256_for(1280, 720, 87)
        );
        assert_eq!(
            crate::xmap04_underwater_srv_descriptor_sha256(87, 4, 0, 1, 0, 1),
            crate::xmap04_underwater_srv_descriptor_sha256(87, 4, 0, 1, 0, 1),
            "the dimensionless SRV descriptor cannot detect resize"
        );
    }

    #[test]
    #[cfg(windows)]
    fn xmap07_borrowed_gpu_views_require_typed_identity_and_live_pointer() {
        let binding = WaterDependentResourceBinding {
            symbol: "ProvinceSecondaryColorMap".into(),
            owner_packet: "XMAP05".into(),
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            camera_view_sha256: None,
            resource_sha256: Sha256::hex(b"political"),
            descriptor_sha256: Sha256::hex(b"political-descriptor"),
        };
        assert!(NativeXmap07DependentResourceView::from_parts(
            binding.clone(),
            std::ptr::null_mut()
        )
        .is_err());
        let fake_srv = std::ptr::dangling_mut();
        assert!(NativeXmap07DependentResourceView::from_parts(binding, fake_srv).is_ok());

        let fake_buffer = std::ptr::dangling_mut();
        assert!(NativeXmap07PointLightsConstantBufferView::from_parts(
            16,
            Sha256::hex(b"point-lights"),
            Sha256::hex(b"point-lights-descriptor"),
            Sha256::hex(b"replay-frame"),
            "not-a-camera",
            fake_buffer,
        )
        .is_err());
        assert!(NativeXmap07PointLightsConstantBufferView::from_parts(
            16,
            Sha256::hex(b"point-lights"),
            Sha256::hex(b"point-lights-descriptor"),
            Sha256::hex(b"replay-frame"),
            Sha256::hex(b"camera"),
            fake_buffer,
        )
        .is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn xmap07_resource_register_remap_preserves_authenticated_identity() {
        let view = NativeXmap07ResourceView {
            binding: NativeXmap07ShaderResourceBinding {
                symbol: "WaterColor".into(),
                shader_register: 0,
                resource_type: "Texture2D".into(),
                owner_packet: "XMAP07".into(),
                resource_sha256: Sha256::hex(b"water-color"),
                descriptor_sha256: Sha256::hex(b"water-color-descriptor"),
            },
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            device_identity: 1,
            raw_srv: std::ptr::dangling_mut(),
            _owner: std::marker::PhantomData,
        };
        let remapped = view.at_shader_register(7);
        assert_eq!(view.binding.shader_register, 0);
        assert_eq!(remapped.binding.shader_register, 7);
        assert_eq!(remapped.binding.symbol, view.binding.symbol);
        assert_eq!(remapped.binding.owner_packet, view.binding.owner_packet);
        assert_eq!(
            remapped.binding.resource_sha256,
            view.binding.resource_sha256
        );
        assert_eq!(
            remapped.binding.descriptor_sha256,
            view.binding.descriptor_sha256
        );
        assert_eq!(
            remapped.replay_frame_identity_sha256,
            view.replay_frame_identity_sha256
        );
        assert_eq!(remapped.device_identity, view.device_identity);
        assert_eq!(remapped.as_raw(), view.as_raw());
    }

    #[test]
    fn surface_draw_receipt_rejects_tampered_stage_assignment() {
        let topology_sha256 = Sha256::hex(b"shared-topology");
        let assignment = portable_render_stream::AssignedStage::seal(
            topology_sha256.clone(),
            "water_surface",
            "water.surface.draw",
            7,
        )
        .unwrap();
        let mut ready_contract = ReadyWaterDrawContract {
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            camera_view_sha256: Sha256::hex(b"camera"),
            water_frame_sha256: Sha256::hex(b"water-frame"),
            water_program_sha256: Sha256::hex(b"water-program"),
            scene_cb0_sha256: Sha256::hex(b"scene-cb0"),
            scene_cb0_descriptor_sha256: Sha256::hex(b"scene-cb0-descriptor"),
            scene_cb0_composition_receipt_sha256: Sha256::hex(b"scene-composition"),
            scene_cb0_reflected_layout_sha256: Sha256::hex(b"scene-layout"),
            point_lights_cb2_sha256: None,
            point_lights_cb2_descriptor_sha256: None,
            point_lights_cb2_camera_view_sha256: None,
            underwater_draw_receipt_sha256: None,
            ordered_dependency_resources: vec![WaterDependentResourceBinding {
                symbol: "ProvinceSecondaryColorMap".into(),
                owner_packet: "XMAP05".into(),
                replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
                camera_view_sha256: None,
                resource_sha256: Sha256::hex(b"political"),
                descriptor_sha256: Sha256::hex(b"political-descriptor"),
            }],
            deterministic_sha256: String::new(),
        };
        ready_contract.deterministic_sha256 =
            crate::production_features::xmap07::ready_water_draw_contract_digest(&ready_contract);
        let receipt = Xmap07SurfaceDrawReceipt {
            schema: "hoi4.xmap07.surface-target-bound-draw.v1".into(),
            topology_sha256,
            assignment_sha256: assignment.assignment_sha256,
            operation_id: "water.surface.draw".into(),
            stage_name: "water_surface".into(),
            assigned_ordinal: 7,
            hour_index: 1,
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            draw_ordinal: 0,
            content_manifest_sha256: Sha256::hex(b"content-manifest"),
            foundation_content_operation_sha256: Sha256::hex(b"foundation-operation"),
            viewer_state_sha256: Sha256::hex(b"viewer"),
            foundation_target_requirement_sha256: Sha256::hex(b"target-requirement"),
            main_scene_target_sha256: Sha256::hex(b"target"),
            main_scene_resource_identity_sha256: Sha256::hex(b"target-resource"),
            main_scene_allocation_generation: 1,
            map_copy_receipt_set_sha256: Sha256::hex(b"copy-set"),
            underwater_draw_receipt_sha256: None,
            ready_contract,
            camera_view_sha256: Sha256::hex(b"camera"),
            map_copy_offset_f32_bits: 0.0f32.to_bits(),
            vertex_count: crate::XMAP07_WATER_VERTEX_COUNT,
            packet_draw_receipt_sha256: String::new(),
        }
        .seal()
        .unwrap();
        receipt.validate().unwrap();

        let mut tampered = receipt;
        tampered.assignment_sha256 = Sha256::hex(b"other-assignment");
        assert!(tampered.validate().is_err());
    }

    #[cfg(windows)]
    fn dx10_dds(format: u32, width: u32, height: u32, payload: Vec<u8>) -> Vec<u8> {
        let mut bytes = vec![0u8; 148];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes[84..88].copy_from_slice(b"DX10");
        bytes[128..132].copy_from_slice(&format.to_le_bytes());
        bytes[132..136].copy_from_slice(&3u32.to_le_bytes());
        bytes[140..144].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[test]
    #[cfg(windows)]
    fn dx10_upload_parser_preserves_format_and_pitch() {
        let bytes = dx10_dds(83, 8, 8, vec![0x5a; 64]);
        let parsed = parse_dds_upload(&bytes).unwrap();
        assert_eq!(parsed.dxgi_format, 83);
        assert_eq!(parsed.subresources.len(), 1);
        assert_eq!(parsed.subresources[0].row_pitch, 32);
        assert_eq!(parsed.subresources[0].slice_pitch, 64);
    }

    #[test]
    #[cfg(windows)]
    fn parser_rejects_trailing_or_truncated_dds_bytes() {
        let mut bytes = dx10_dds(28, 2, 2, vec![0; 16]);
        bytes.push(0);
        assert!(parse_dds_upload(&bytes).is_err());
        bytes.truncate(150);
        assert!(parse_dds_upload(&bytes).is_err());
    }

    #[test]
    #[cfg(windows)]
    fn xmap07_engine_srgb_slots_accept_every_compatible_family_and_reject_others() {
        for (file_format, expected) in [
            (27, 29),
            (28, 29),
            (29, 29),
            (70, 72),
            (71, 72),
            (72, 72),
            (73, 75),
            (74, 75),
            (75, 75),
            (76, 78),
            (77, 78),
            (78, 78),
            (87, 91),
            (90, 91),
            (91, 91),
            (88, 93),
            (92, 93),
            (93, 93),
            (97, 99),
            (98, 99),
            (99, 99),
        ] {
            assert_eq!(
                engine_registered_format("WaterColor", file_format).unwrap(),
                expected
            );
            assert_eq!(
                engine_registered_format("ReflectionCubeMap", file_format).unwrap(),
                expected
            );
        }
        for incompatible in [10, 83] {
            assert!(engine_registered_format("WaterColor", incompatible).is_err());
            assert!(engine_registered_format("ReflectionCubeMap", incompatible).is_err());
        }
        assert_eq!(engine_registered_format("IceDiffuse", 77).unwrap(), 77);
        assert_eq!(engine_registered_format("LeanTexture1", 87).unwrap(), 87);
    }
}
