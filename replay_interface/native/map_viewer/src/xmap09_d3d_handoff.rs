//! App-device providers for XMAP09 lighting and shadow resources.
//!
//! XMAP09 owns the point-light textures/CB2 and its exact SceneCB0 field
//! contribution. It does not claim camera, FoW, snow/mud, or post-light bytes:
//! consumers apply the authenticated field patch to their reflected aggregate
//! SceneCB0. This module executes XMAP09's owned 2048x2048 light-depth pass.
//! XMAP10 owns the downstream combined terrainunlit/blur screen `ShadowMap`;
//! XMAP09 never fabricates its non-owned SceneCB0 or fog inputs.

use crate::{
    L03LightDepthProjection, L03LightDepthSubmission, L03SelectedLightFrame,
    PreparedXmap09LightingFrame, Result, Xmap09SceneCb0Patch, Xmap09ShadowMatrixBinding,
    Xmap09TextureResource, XMAP09_LIGHT_DATA_MAP_SYMBOL, XMAP09_LIGHT_INDEX_MAP_SYMBOL,
    XMAP09_POINT_LIGHT_CB2_SHADER_REGISTER, XMAP09_POINT_LIGHT_CB2_SYMBOL,
};
use serde::Serialize;
use snapshot::sha256::Sha256;
#[cfg(windows)]
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09NativeShaderResourceBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub camera_frame_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub stable_resource_id: String,
    /// Authenticated payload identity for immutable CPU uploads, or the sealed
    /// native execution identity for GPU-produced light depth.
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09NativeConstantBufferBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub camera_frame_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub shader_register: u32,
    pub byte_len: u32,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09LightDepthExecutionReceipt {
    pub input_payload_sha256: String,
    pub prepared_frame_sha256: String,
    pub camera_frame_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub assigned_stage: crate::AssignedStage,
    pub active: bool,
    pub projection_sha256: String,
    pub target_descriptor_sha256: String,
    pub ordered_registered_caster_sha256: Vec<String>,
    pub ordered_submission_sha256: Vec<String>,
    pub cleared_depth_f32_bits: Option<u32>,
    pub binding: Option<Xmap09NativeShaderResourceBinding>,
    pub deterministic_sha256: String,
}

impl Xmap09LightDepthExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        validate_sha256(&self.input_payload_sha256, "execution input")?;
        validate_sha256(&self.prepared_frame_sha256, "execution frame")?;
        validate_sha256(&self.camera_frame_sha256, "execution camera")?;
        validate_sha256(&self.replay_frame_identity_sha256, "execution replay frame")?;
        validate_sha256(&self.projection_sha256, "execution projection")?;
        validate_sha256(
            &self.target_descriptor_sha256,
            "execution target descriptor",
        )?;
        self.assigned_stage
            .validate()
            .map_err(|error| format!("XMAP09 execution assignment: {error}"))?;
        if self.assigned_stage.stage_name != crate::XMAP09_EXECUTE_STAGE_NAME
            || self.assigned_stage.operation_name != crate::XMAP09_EXECUTE_OPERATION_NAME
        {
            return Err("XMAP09 execution receipt carries the wrong assigned stage".into());
        }
        for identity in self
            .ordered_registered_caster_sha256
            .iter()
            .chain(&self.ordered_submission_sha256)
        {
            validate_sha256(identity, "execution ordered identity")?;
        }
        match (
            self.active,
            self.cleared_depth_f32_bits,
            self.binding.as_ref(),
        ) {
            (true, Some(bits), Some(binding))
                if bits == 1.0f32.to_bits()
                    && binding.symbol == "LightDepthMap"
                    && binding.owner_packet == "XMAP09"
                    && binding.camera_frame_sha256 == self.camera_frame_sha256
                    && binding.replay_frame_identity_sha256
                        == self.replay_frame_identity_sha256
                    && binding.descriptor_sha256 == self.target_descriptor_sha256 =>
            {
                validate_sha256(&binding.resource_sha256, "executed light depth")?;
            }
            (false, None, None)
                if self.ordered_registered_caster_sha256.is_empty()
                    && self.ordered_submission_sha256.is_empty() => {}
            _ => return Err("XMAP09 execution active/inactive target law mismatch".into()),
        }
        validate_sha256(&self.deterministic_sha256, "execution receipt")?;
        if self.deterministic_sha256 != light_depth_execution_digest(self)? {
            return Err("XMAP09 execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09D3dHandoffReceipt {
    pub prepared_frame_sha256: String,
    pub camera_frame_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub uploaded_point_light_textures: u32,
    pub uploaded_point_light_constant_buffers: u32,
    pub scene_cb0_patch_sha256: String,
    pub shadow_matrix_payload_sha256: String,
    pub point_light_bindings: Vec<Xmap09NativeShaderResourceBinding>,
    pub point_lights_cb2: Xmap09NativeConstantBufferBinding,
    pub light_depth_execution: Option<Xmap09LightDepthExecutionReceipt>,
    pub deterministic_sha256: String,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap09ShaderResourceView<'a> {
    pub binding: Xmap09NativeShaderResourceBinding,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    shader_register: Option<u32>,
    _owner: std::marker::PhantomData<&'a NativeXmap09LightingHandoff>,
}

#[cfg(windows)]
impl NativeXmap09ShaderResourceView<'_> {
    pub fn frame_identity_sha256(&self) -> &str {
        &self.binding.camera_frame_sha256
    }

    pub fn replay_frame_identity_sha256(&self) -> &str {
        &self.binding.replay_frame_identity_sha256
    }

    /// Borrowed pointer; the `NativeXmap09LightingHandoff` must outlive use.
    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }

    pub fn at_shader_register(mut self, shader_register: u32) -> Self {
        self.shader_register = Some(shader_register);
        self
    }
}

#[cfg(windows)]
unsafe impl crate::NativeXmap04UnderwaterShaderResource for NativeXmap09ShaderResourceView<'_> {
    fn owner_packet(&self) -> &str {
        &self.binding.owner_packet
    }

    fn symbol(&self) -> &str {
        &self.binding.symbol
    }

    fn shader_register(&self) -> u32 {
        self.shader_register.unwrap_or(u32::MAX)
    }

    fn frame_identity_sha256(&self) -> &str {
        &self.binding.camera_frame_sha256
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.binding.replay_frame_identity_sha256
    }

    fn resource_sha256(&self) -> &str {
        &self.binding.resource_sha256
    }

    fn descriptor_sha256(&self) -> &str {
        &self.binding.descriptor_sha256
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap09ConstantBufferView<'a> {
    pub binding: Xmap09NativeConstantBufferBinding,
    raw_buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    _owner: std::marker::PhantomData<&'a NativeXmap09LightingHandoff>,
}

#[cfg(windows)]
impl NativeXmap09ConstantBufferView<'_> {
    pub fn frame_identity_sha256(&self) -> &str {
        &self.binding.camera_frame_sha256
    }

    pub fn replay_frame_identity_sha256(&self) -> &str {
        &self.binding.replay_frame_identity_sha256
    }

    /// Borrowed pointer; the `NativeXmap09LightingHandoff` must outlive use.
    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.raw_buffer
    }
}

#[cfg(windows)]
unsafe impl crate::NativeXmap04UnderwaterPointLightsConstantBuffer
    for NativeXmap09ConstantBufferView<'_>
{
    fn owner_packet(&self) -> &str {
        &self.binding.owner_packet
    }

    fn symbol(&self) -> &str {
        &self.binding.symbol
    }

    fn shader_register(&self) -> u32 {
        self.binding.shader_register
    }

    fn byte_len(&self) -> u32 {
        self.binding.byte_len
    }

    fn frame_identity_sha256(&self) -> &str {
        &self.binding.camera_frame_sha256
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.binding.replay_frame_identity_sha256
    }

    fn payload_sha256(&self) -> &str {
        &self.binding.payload_sha256
    }

    fn descriptor_sha256(&self) -> &str {
        &self.binding.descriptor_sha256
    }

    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.raw_buffer
    }
}

#[cfg(windows)]
struct NativeImmutableTexture {
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    binding: Xmap09NativeShaderResourceBinding,
}

#[cfg(windows)]
struct NativeLightDepthTarget {
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    depth_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilView>,
    shader_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    binding: Xmap09NativeShaderResourceBinding,
}

#[cfg(windows)]
pub trait NativeXmap09LightDepthCaster {
    fn owner_packet(&self) -> &str;
    fn camera_frame_sha256(&self) -> &str;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn registered_submission_sha256s(&self) -> &[String];

    /// Draw one exact registered L03 submission. XMAP09 has already bound the
    /// current frame's light-depth DSV and 2048x2048 viewport.
    ///
    /// # Safety
    /// The implementation must use only resources from `context`'s device and
    /// must issue exactly the supplied submission.
    unsafe fn draw_light_depth_submission(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        selected_light: &L03SelectedLightFrame,
        submission: &L03LightDepthSubmission,
    ) -> Result<()>;
}

#[cfg(windows)]
pub struct NativeXmap09LightingHandoff {
    device: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Device>,
    light_index_map: NativeImmutableTexture,
    light_data_map: NativeImmutableTexture,
    point_lights_cb2: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    point_lights_cb2_binding: Xmap09NativeConstantBufferBinding,
    light_depth_target: NativeLightDepthTarget,
    scene_cb0_patch: Xmap09SceneCb0Patch,
    shadow_matrix: Xmap09ShadowMatrixBinding,
    frame: PreparedXmap09LightingFrame,
    light_depth_execution: Option<Xmap09LightDepthExecutionReceipt>,
    receipt: Xmap09D3dHandoffReceipt,
}

#[cfg(windows)]
impl NativeXmap09LightingHandoff {
    /// Upload XMAP09's immutable point-light resources and create its exact
    /// 2048x2048 light-depth target on the app device.
    ///
    /// # Safety
    /// `device` must be a live D3D11 device for this call and every use of the
    /// returned borrowed provider views.
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedXmap09LightingFrame,
    ) -> Result<Self> {
        use crate::d3d11_device::D3dPtr;
        use winapi::um::unknwnbase::IUnknown;

        frame.validate()?;
        if device.is_null() {
            return Err("XMAP09 app-device handoff received a null D3D11 device".into());
        }
        (*(device as *mut IUnknown)).AddRef();
        let retained_device = D3dPtr::from_raw(device, "XMAP09 retained D3D11 device")?;
        let light_index_map = create_immutable_texture(
            device,
            XMAP09_LIGHT_INDEX_MAP_SYMBOL,
            &frame.camera_frame_sha256,
            &frame.replay_frame_identity_sha256,
            &frame.light_index_map,
        )?;
        let light_data_map = create_immutable_texture(
            device,
            XMAP09_LIGHT_DATA_MAP_SYMBOL,
            &frame.camera_frame_sha256,
            &frame.replay_frame_identity_sha256,
            &frame.light_data_map,
        )?;
        let cb2_bytes = frame.point_lights_cb2_bytes()?;
        let point_lights_cb2 = create_immutable_constant_buffer(
            device,
            &cb2_bytes,
            "XMAP09 PointLightsCB2 GridStart_InvCellSize",
        )?;
        let point_lights_cb2_binding = constant_buffer_binding(
            XMAP09_POINT_LIGHT_CB2_SYMBOL,
            XMAP09_POINT_LIGHT_CB2_SHADER_REGISTER,
            &frame.camera_frame_sha256,
            &frame.replay_frame_identity_sha256,
            &cb2_bytes,
        );
        let light_depth_target = create_light_depth_target(
            device,
            &frame.camera_frame_sha256,
            &frame.replay_frame_identity_sha256,
            &frame.shadow.selected_light_frame.light_depth_target,
            frame.shadow.light_depth_bind_flags,
        )?;
        let scene_cb0_patch = frame.scene_cb0_patch()?;
        let shadow_matrix = frame.shadow_map_texture_matrix()?;
        let mut receipt = Xmap09D3dHandoffReceipt {
            prepared_frame_sha256: frame.deterministic_sha256.clone(),
            camera_frame_sha256: frame.camera_frame_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            uploaded_point_light_textures: 2,
            uploaded_point_light_constant_buffers: 1,
            scene_cb0_patch_sha256: scene_cb0_patch.deterministic_sha256.clone(),
            shadow_matrix_payload_sha256: shadow_matrix.payload_sha256.clone(),
            point_light_bindings: vec![
                light_index_map.binding.clone(),
                light_data_map.binding.clone(),
            ],
            point_lights_cb2: point_lights_cb2_binding.clone(),
            light_depth_execution: None,
            deterministic_sha256: String::new(),
        };
        reseal_receipt(&mut receipt)?;
        Ok(Self {
            device: retained_device,
            light_index_map,
            light_data_map,
            point_lights_cb2,
            point_lights_cb2_binding,
            light_depth_target,
            scene_cb0_patch,
            shadow_matrix,
            frame: frame.clone(),
            light_depth_execution: None,
            receipt,
        })
    }

    pub fn receipt(&self) -> &Xmap09D3dHandoffReceipt {
        &self.receipt
    }

    pub fn scene_cb0_patch(&self) -> &Xmap09SceneCb0Patch {
        &self.scene_cb0_patch
    }

    pub fn shadow_map_texture_matrix(&self) -> &Xmap09ShadowMatrixBinding {
        &self.shadow_matrix
    }

    pub fn light_index_map_view(&self) -> NativeXmap09ShaderResourceView<'_> {
        shader_resource_view(&self.light_index_map)
    }

    pub fn light_data_map_view(&self) -> NativeXmap09ShaderResourceView<'_> {
        shader_resource_view(&self.light_data_map)
    }

    pub fn point_lights_cb2_view(&self) -> NativeXmap09ConstantBufferView<'_> {
        NativeXmap09ConstantBufferView {
            binding: self.point_lights_cb2_binding.clone(),
            raw_buffer: self.point_lights_cb2.as_raw(),
            _owner: std::marker::PhantomData,
        }
    }

    /// Return XMAP09's sampled light-depth target only after the current
    /// frame's owned native pass has completed. Inactive and unexecuted frames
    /// fail closed; XMAP10 consumes this view to publish the screen ShadowMap.
    pub fn light_depth_view(&self) -> Result<NativeXmap09ShaderResourceView<'_>> {
        let execution = self
            .light_depth_execution
            .as_ref()
            .ok_or("XMAP09 light-depth pass has not executed")?;
        if !execution.active {
            return Err("XMAP09 inactive light-depth frame has no sampled output".into());
        }
        Ok(NativeXmap09ShaderResourceView {
            binding: execution
                .binding
                .clone()
                .ok_or("XMAP09 active light-depth receipt omitted its binding")?,
            raw_srv: self.light_depth_target.shader_view.as_raw(),
            shader_register: None,
            _owner: std::marker::PhantomData,
        })
    }

    /// Execute XMAP09's owned light-depth pass on the app device. The method
    /// clears depth to 1.0 and dispatches every exact projected submission to
    /// the one caster that registered its canonical submission hash.
    ///
    /// # Safety
    /// `context` must belong to this handoff's device. Every caster must issue
    /// only the supplied submission using resources from that same device.
    pub unsafe fn execute_light_depth(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        projection: &L03LightDepthProjection,
        casters: &mut [&mut dyn NativeXmap09LightDepthCaster],
    ) -> Result<&Xmap09LightDepthExecutionReceipt> {
        validate_context_device(self.device.as_raw(), context)?;
        let projection_sha256 = Sha256::hex(&serde_json::to_vec(projection)?);
        let plan = self.frame.plan_shadow_consumer(projection)?;
        let mut registration_by_submission = BTreeMap::new();
        let mut ordered_registered_caster_sha256 = Vec::with_capacity(casters.len());
        let mut registered_owners = BTreeSet::new();
        let mut prior_owner_rank = None;
        for (caster_index, caster) in casters.iter().enumerate() {
            let Some(owner_rank) = self
                .frame
                .shadow
                .exact_caster_owner_packets
                .iter()
                .position(|owner| owner == &caster.owner_packet())
            else {
                return Err("XMAP09 caster registration has an unknown owner".into());
            };
            if caster.owner_packet().trim().is_empty()
                || caster.camera_frame_sha256() != self.frame.camera_frame_sha256
                || caster.replay_frame_identity_sha256() != self.frame.replay_frame_identity_sha256
                || caster.registered_submission_sha256s().is_empty()
                || prior_owner_rank.is_some_and(|prior| owner_rank <= prior)
                || !registered_owners.insert(caster.owner_packet().to_owned())
            {
                return Err("XMAP09 caster registration has an invalid/duplicate owner, empty census, or another camera frame".into());
            }
            prior_owner_rank = Some(owner_rank);
            let registration_sha256 = Sha256::hex(&serde_json::to_vec(&(
                caster.owner_packet(),
                caster.camera_frame_sha256(),
                caster.replay_frame_identity_sha256(),
                caster.registered_submission_sha256s(),
            ))?);
            ordered_registered_caster_sha256.push(registration_sha256);
            for submission_sha256 in caster.registered_submission_sha256s() {
                validate_sha256(submission_sha256, "registered caster submission")?;
                if registration_by_submission
                    .insert(submission_sha256.clone(), caster_index)
                    .is_some()
                {
                    return Err("XMAP09 submission is registered by more than one caster".into());
                }
            }
        }

        let ordered_submission_sha256 = projection
            .submissions
            .iter()
            .map(|submission| {
                serde_json::to_vec(submission)
                    .map(|bytes| Sha256::hex(&bytes))
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        let projected = ordered_submission_sha256
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let registered = registration_by_submission
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if projected != registered {
            return Err("XMAP09 registered caster census differs from the exact projection".into());
        }

        let (active, cleared_depth_f32_bits, mut binding) = if plan.is_none() {
            if !projection.submissions.is_empty() || !casters.is_empty() {
                return Err("XMAP09 inactive pass contains casters or submissions".into());
            }
            (false, None, None)
        } else {
            bind_and_clear_light_depth(
                context,
                self.light_depth_target.depth_view.as_raw(),
                self.frame
                    .shadow
                    .selected_light_frame
                    .light_depth_target
                    .width,
                self.frame
                    .shadow
                    .selected_light_frame
                    .light_depth_target
                    .height,
            )?;
            let mut draw_error = None;
            for (submission, submission_sha256) in projection
                .submissions
                .iter()
                .zip(&ordered_submission_sha256)
            {
                let caster_index = *registration_by_submission
                    .get(submission_sha256)
                    .ok_or("XMAP09 projection submission has no registered caster")?;
                if let Err(error) = casters[caster_index].draw_light_depth_submission(
                    context,
                    &self.frame.shadow.selected_light_frame,
                    submission,
                ) {
                    draw_error = Some(error);
                    break;
                }
            }
            (*context).OMSetRenderTargets(0, std::ptr::null(), std::ptr::null_mut());
            if let Some(error) = draw_error {
                return Err(error);
            }
            (
                true,
                Some(1.0f32.to_bits()),
                Some(self.light_depth_target.binding.clone()),
            )
        };
        if let Some(binding) = binding.as_mut() {
            binding.resource_sha256 = Sha256::hex(&serde_json::to_vec(&(
                &self.frame.input_payload_sha256,
                &self.frame.deterministic_sha256,
                &self.frame.camera_frame_sha256,
                &self.frame.replay_frame_identity_sha256,
                &self.frame.shadow_execute_stage,
                &projection_sha256,
                &binding.descriptor_sha256,
                &ordered_registered_caster_sha256,
                &ordered_submission_sha256,
                1.0f32.to_bits(),
            ))?);
        }

        let mut receipt = Xmap09LightDepthExecutionReceipt {
            input_payload_sha256: self.frame.input_payload_sha256.clone(),
            prepared_frame_sha256: self.frame.deterministic_sha256.clone(),
            camera_frame_sha256: self.frame.camera_frame_sha256.clone(),
            replay_frame_identity_sha256: self.frame.replay_frame_identity_sha256.clone(),
            assigned_stage: self.frame.shadow_execute_stage.clone(),
            active,
            projection_sha256,
            target_descriptor_sha256: self.light_depth_target.binding.descriptor_sha256.clone(),
            ordered_registered_caster_sha256,
            ordered_submission_sha256,
            cleared_depth_f32_bits,
            binding,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = light_depth_execution_digest(&receipt)?;
        receipt.validate()?;
        self.light_depth_execution = Some(receipt);
        self.receipt.light_depth_execution = self.light_depth_execution.clone();
        reseal_receipt(&mut self.receipt)?;
        Ok(self
            .light_depth_execution
            .as_ref()
            .expect("execution inserted"))
    }
}

#[cfg(windows)]
fn shader_resource_view(texture: &NativeImmutableTexture) -> NativeXmap09ShaderResourceView<'_> {
    NativeXmap09ShaderResourceView {
        binding: texture.binding.clone(),
        raw_srv: texture.view.as_raw(),
        shader_register: None,
        _owner: std::marker::PhantomData,
    }
}

#[cfg(windows)]
unsafe fn create_immutable_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    symbol: &str,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    resource: &Xmap09TextureResource,
) -> Result<NativeImmutableTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_IMMUTABLE,
    };

    let desc = D3D11_TEXTURE2D_DESC {
        Width: resource.width,
        Height: resource.height,
        MipLevels: resource.mip_levels,
        ArraySize: 1,
        Format: resource.dxgi_format as _,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: resource.bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: resource.bytes.as_ptr() as *const _,
        SysMemPitch: resource.row_pitch,
        SysMemSlicePitch: u32::try_from(resource.bytes.len())?,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut raw_texture),
        &format!("CreateTexture2D(XMAP09 {symbol})"),
    )?;
    let texture = D3dPtr::from_raw(raw_texture, &format!("XMAP09 {symbol} texture"))?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(raw_texture as *mut _, ptr::null(), &mut raw_view),
        &format!("CreateShaderResourceView(XMAP09 {symbol})"),
    )?;
    let view = D3dPtr::from_raw(raw_view, &format!("XMAP09 {symbol} SRV"))?;
    Ok(NativeImmutableTexture {
        _texture: texture,
        view,
        binding: Xmap09NativeShaderResourceBinding {
            symbol: symbol.into(),
            owner_packet: "XMAP09".into(),
            camera_frame_sha256: camera_frame_sha256.into(),
            replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
            stable_resource_id: resource.stable_resource_id.clone(),
            resource_sha256: resource.payload_sha256.clone(),
            descriptor_sha256: resource.descriptor_sha256.clone(),
        },
    })
}

#[cfg(windows)]
unsafe fn create_immutable_constant_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_IMMUTABLE,
    };

    if bytes.is_empty() || bytes.len() % 16 != 0 {
        return Err(format!("{role} byte length is not a non-zero multiple of 16").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut raw = ptr::null_mut::<ID3D11Buffer>();
    native_d3d11_hr(
        (*device).CreateBuffer(&desc, &initial, &mut raw),
        &format!("CreateBuffer({role})"),
    )?;
    D3dPtr::from_raw(raw, role)
}

fn constant_buffer_binding(
    symbol: &str,
    shader_register: u32,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    bytes: &[u8],
) -> Xmap09NativeConstantBufferBinding {
    let payload_sha256 = Sha256::hex(bytes);
    let descriptor_sha256 = Sha256::hex(
        format!(
            "XMAP09\0{symbol}\0b{shader_register}\0{camera_frame_sha256}\0{replay_frame_identity_sha256}\0{}\0{}",
            bytes.len(),
            payload_sha256
        )
        .as_bytes(),
    );
    Xmap09NativeConstantBufferBinding {
        symbol: symbol.into(),
        owner_packet: "XMAP09".into(),
        camera_frame_sha256: camera_frame_sha256.into(),
        replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
        shader_register,
        byte_len: bytes.len() as u32,
        payload_sha256,
        descriptor_sha256,
    }
}

#[cfg(windows)]
unsafe fn create_light_depth_target(
    device: *mut winapi::um::d3d11::ID3D11Device,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    target: &crate::L03LightDepthTarget,
    bind_flags: u32,
) -> Result<NativeLightDepthTarget> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11DepthStencilView, ID3D11ShaderResourceView, ID3D11Texture2D,
        D3D11_DEPTH_STENCIL_VIEW_DESC, D3D11_DSV_DIMENSION_TEXTURE2D,
        D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_TEX2D_DSV, D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };

    if target.width != crate::L03_LIGHT_DEPTH_WIDTH
        || target.height != crate::L03_LIGHT_DEPTH_HEIGHT
        || target.dxgi_format != crate::L03_LIGHT_DEPTH_DXGI_FORMAT
        || target.mip_levels != crate::L03_LIGHT_DEPTH_MIP_LEVELS
        || bind_flags
            != (winapi::um::d3d11::D3D11_BIND_DEPTH_STENCIL
                | winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE)
    {
        return Err("XMAP09 light-depth resource contract mismatch".into());
    }
    let texture_desc = D3D11_TEXTURE2D_DESC {
        Width: target.width,
        Height: target.height,
        MipLevels: target.mip_levels,
        ArraySize: 1,
        Format: winapi::shared::dxgiformat::DXGI_FORMAT_R32_TYPELESS,
        SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&texture_desc, ptr::null(), &mut raw_texture),
        "CreateTexture2D(XMAP09 light depth)",
    )?;
    let texture = D3dPtr::from_raw(raw_texture, "XMAP09 light-depth texture")?;

    let mut dsv_desc: D3D11_DEPTH_STENCIL_VIEW_DESC = std::mem::zeroed();
    dsv_desc.Format = winapi::shared::dxgiformat::DXGI_FORMAT_D32_FLOAT;
    dsv_desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2D;
    dsv_desc.Flags = 0;
    *dsv_desc.u.Texture2D_mut() = D3D11_TEX2D_DSV { MipSlice: 0 };
    let mut raw_dsv = ptr::null_mut::<ID3D11DepthStencilView>();
    native_d3d11_hr(
        (*device).CreateDepthStencilView(raw_texture as *mut _, &dsv_desc, &mut raw_dsv),
        "CreateDepthStencilView(XMAP09 light depth)",
    )?;
    let depth_view = D3dPtr::from_raw(raw_dsv, "XMAP09 light-depth DSV")?;

    let mut srv_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = std::mem::zeroed();
    srv_desc.Format = winapi::shared::dxgiformat::DXGI_FORMAT_R32_FLOAT;
    srv_desc.ViewDimension = winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;
    *srv_desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
        MostDetailedMip: 0,
        MipLevels: 1,
    };
    let mut raw_srv = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(raw_texture as *mut _, &srv_desc, &mut raw_srv),
        "CreateShaderResourceView(XMAP09 light depth)",
    )?;
    let shader_view = D3dPtr::from_raw(raw_srv, "XMAP09 light-depth SRV")?;
    let descriptor_sha256 = Sha256::hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            target.stable_resource_id,
            target.width,
            target.height,
            winapi::shared::dxgiformat::DXGI_FORMAT_R32_TYPELESS as u32,
            winapi::shared::dxgiformat::DXGI_FORMAT_D32_FLOAT as u32,
            target.dxgi_format,
            target.mip_levels,
            bind_flags
        )
        .as_bytes(),
    );
    Ok(NativeLightDepthTarget {
        _texture: texture,
        depth_view,
        shader_view,
        binding: Xmap09NativeShaderResourceBinding {
            symbol: "LightDepthMap".into(),
            owner_packet: "XMAP09".into(),
            camera_frame_sha256: camera_frame_sha256.into(),
            replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
            stable_resource_id: target.stable_resource_id.clone(),
            resource_sha256: String::new(),
            descriptor_sha256,
        },
    })
}

#[cfg(windows)]
unsafe fn validate_context_device(
    expected_device: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
) -> Result<()> {
    use crate::d3d11_device::D3dPtr;
    if context.is_null() {
        return Err("XMAP09 light-depth execution received a null context".into());
    }
    let mut raw_device = std::ptr::null_mut();
    (*context).GetDevice(&mut raw_device);
    let device = D3dPtr::from_raw(raw_device, "XMAP09 execution context device")?;
    if device.as_raw() != expected_device {
        return Err("XMAP09 light-depth context belongs to a different device".into());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn bind_and_clear_light_depth(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    depth_view: *mut winapi::um::d3d11::ID3D11DepthStencilView,
    width: u32,
    height: u32,
) -> Result<()> {
    if depth_view.is_null() || width != 2048 || height != 2048 {
        return Err("XMAP09 light-depth target is null or not 2048x2048".into());
    }
    let viewport = winapi::um::d3d11::D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width as f32,
        Height: height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    (*context).RSSetViewports(1, &viewport);
    (*context).OMSetRenderTargets(0, std::ptr::null(), depth_view);
    (*context).ClearDepthStencilView(depth_view, winapi::um::d3d11::D3D11_CLEAR_DEPTH, 1.0, 0);
    Ok(())
}

fn reseal_receipt(receipt: &mut Xmap09D3dHandoffReceipt) -> Result<()> {
    let mut unsigned = receipt.clone();
    unsigned.deterministic_sha256.clear();
    receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&unsigned)?);
    Ok(())
}

fn light_depth_execution_digest(receipt: &Xmap09LightDepthExecutionReceipt) -> Result<String> {
    let mut unsigned = receipt.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("XMAP09 {label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_light_cb2_descriptor_binds_symbol_slot_length_and_payload() {
        let bytes = [7u8; 16];
        let camera = "c".repeat(64);
        let replay = "e".repeat(64);
        let binding =
            constant_buffer_binding(XMAP09_POINT_LIGHT_CB2_SYMBOL, 2, &camera, &replay, &bytes);
        assert_eq!(binding.owner_packet, "XMAP09");
        assert_eq!(binding.camera_frame_sha256, camera);
        assert_eq!(binding.replay_frame_identity_sha256, replay);
        assert_eq!(binding.shader_register, 2);
        assert_eq!(binding.byte_len, 16);
        assert_eq!(binding.payload_sha256, Sha256::hex(&bytes));
        assert_ne!(binding.descriptor_sha256, binding.payload_sha256);
        let other = constant_buffer_binding(
            XMAP09_POINT_LIGHT_CB2_SYMBOL,
            2,
            &"d".repeat(64),
            &replay,
            &bytes,
        );
        assert_ne!(binding.descriptor_sha256, other.descriptor_sha256);
        let other_replay_frame = constant_buffer_binding(
            XMAP09_POINT_LIGHT_CB2_SYMBOL,
            2,
            &binding.camera_frame_sha256,
            &"f".repeat(64),
            &bytes,
        );
        assert_ne!(
            binding.descriptor_sha256,
            other_replay_frame.descriptor_sha256
        );
    }

    #[test]
    fn inactive_execution_receipt_is_typed_and_has_no_fake_binding() {
        let mut receipt = Xmap09LightDepthExecutionReceipt {
            input_payload_sha256: "1".repeat(64),
            prepared_frame_sha256: "2".repeat(64),
            camera_frame_sha256: "3".repeat(64),
            replay_frame_identity_sha256: "7".repeat(64),
            assigned_stage: crate::AssignedStage::seal(
                "4".repeat(64),
                crate::XMAP09_EXECUTE_STAGE_NAME,
                crate::XMAP09_EXECUTE_OPERATION_NAME,
                2,
            )
            .unwrap(),
            active: false,
            projection_sha256: "5".repeat(64),
            target_descriptor_sha256: "6".repeat(64),
            ordered_registered_caster_sha256: Vec::new(),
            ordered_submission_sha256: Vec::new(),
            cleared_depth_f32_bits: None,
            binding: None,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = light_depth_execution_digest(&receipt).unwrap();
        receipt.validate().unwrap();
        let mut forged = receipt;
        forged.cleared_depth_f32_bits = Some(1.0f32.to_bits());
        forged.deterministic_sha256 = light_depth_execution_digest(&forged).unwrap();
        assert!(forged.validate().is_err());
    }
}
