use crate::{L03SelectedLightFrame, PreparedXmap10SceneConstantBuffer, Result};
use snapshot::sha256::Sha256;

pub fn map_arrow_light_scene_cb0(
    scene: &PreparedXmap10SceneConstantBuffer,
    light: &L03SelectedLightFrame,
) -> Result<Vec<u8>> {
    scene.validate()?;
    light_scene_bytes(&scene.bytes, &light.light_view_projection_f32_bits)
}

fn light_scene_bytes(scene: &[u8], matrix: &[u32; 16]) -> Result<Vec<u8>> {
    if scene.len() != 416 || matrix.iter().any(|bits| !f32::from_bits(*bits).is_finite()) {
        return Err("map-arrow light SceneCB0 has an invalid source or matrix".into());
    }
    let mut bytes = vec![0; 416];
    for range in [
        128..208,
        220..224,
        236..284,
        288..300,
        304..316,
        320..332,
        336..408,
    ] {
        bytes[range.clone()].copy_from_slice(&scene[range]);
    }
    for (lane, bits) in matrix.iter().enumerate() {
        bytes[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    bytes[332..336].copy_from_slice(&1.0f32.to_le_bytes());
    Ok(bytes)
}

#[cfg(windows)]
pub struct NativeMapArrowLightScene {
    buffer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    pub(crate) payload_sha256: String,
    selected_light_sha256: String,
    pub(crate) camera_frame_sha256: String,
    pub(crate) replay_frame_identity_sha256: String,
    pub(crate) application_elapsed_seconds: f32,
    device_identity: usize,
}

#[cfg(windows)]
impl NativeMapArrowLightScene {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        scene: &PreparedXmap10SceneConstantBuffer,
        light: &L03SelectedLightFrame,
    ) -> Result<Self> {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use winapi::um::d3d11::*;
        if device.is_null() {
            return Err("map-arrow light SceneCB0 has no device".into());
        }
        let bytes = map_arrow_light_scene_cb0(scene, light)?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: 416,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut raw = std::ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateBuffer(&desc, &data, &mut raw),
            "CreateBuffer(map-arrow light SceneCB0)",
        )?;
        Ok(Self {
            buffer: D3dPtr::from_raw(raw, "map-arrow light SceneCB0")?,
            payload_sha256: Sha256::hex(&bytes),
            selected_light_sha256: Sha256::hex(&serde_json::to_vec(light)?),
            camera_frame_sha256: scene.frame_identity.camera_view_sha256.clone(),
            replay_frame_identity_sha256: scene.frame_identity.replay_frame_identity_sha256.clone(),
            application_elapsed_seconds: f32::from_le_bytes(bytes[236..240].try_into().unwrap()),
            device_identity: device as usize,
        })
    }

    pub fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.buffer.as_raw()
    }

    pub unsafe fn validate_binding(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        light: &L03SelectedLightFrame,
        camera: &str,
        replay: &str,
    ) -> Result<()> {
        if context.is_null()
            || self.camera_frame_sha256 != camera
            || self.replay_frame_identity_sha256 != replay
            || self.selected_light_sha256 != Sha256::hex(&serde_json::to_vec(light)?)
        {
            return Err("map-arrow light binding belongs to another frame".into());
        }
        let mut device = std::ptr::null_mut();
        (*context).GetDevice(&mut device);
        let owner =
            crate::d3d11_device::D3dPtr::from_raw(device, "map-arrow light context device")?;
        if owner.as_raw() as usize != self.device_identity {
            return Err("map-arrow light binding belongs to another device".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_scene_clears_camera_fields_and_preserves_common_scene_inputs() {
        let source = [2.0f32.to_le_bytes(); 104].concat();
        let matrix = [3.0f32.to_bits(); 16];
        let bytes = light_scene_bytes(&source, &matrix).unwrap();
        assert_eq!(&bytes[..64], &[3.0f32.to_le_bytes(); 16].concat());
        assert!(bytes[64..128].iter().all(|b| *b == 0));
        assert!(bytes[208..220].iter().all(|b| *b == 0));
        assert!(bytes[224..236].iter().all(|b| *b == 0));
        assert_eq!(&bytes[128..208], &source[128..208]);
        assert_eq!(&bytes[236..284], &source[236..284]);
        assert_eq!(&bytes[332..336], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[336..408], &source[336..408]);
        for range in [284..288, 300..304, 316..320, 408..416] {
            assert!(bytes[range].iter().all(|b| *b == 0));
        }
        assert!(light_scene_bytes(&source[..400], &matrix).is_err());
        assert!(light_scene_bytes(&source, &[f32::NAN.to_bits(); 16]).is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MapArrowLightDraw {
    pub owner_packet: String,
    pub prepared_frame_sha256: String,
    pub row_ordinal: u32,
    pub outer: bool,
    pub geometry_sha256: String,
    pub scene_cb0_sha256: String,
    pub cb3_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub vertex_count: u32,
    pub first_vertex: u32,
    pub retained_shadow: Option<crate::xmap10_d3d_handoff::Xmap10RetainedShadowReadReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MapArrowLightDrawRefusal {
    pub owner_packet: String,
    pub prepared_frame_sha256: String,
    pub row_ordinal: u32,
    pub outer: bool,
    pub reason: crate::xmap10_d3d_handoff::Xmap10RetainedShadowRefusal,
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) enum MapArrowLightPixel {
    T2Independent(Vec<u8>),
    RetainedT2Required(Vec<u8>),
}

#[cfg(windows)]
impl MapArrowLightPixel {
    fn bytecode(&self) -> &[u8] {
        match self { Self::T2Independent(bytes) | Self::RetainedT2Required(bytes) => bytes }
    }
}

#[cfg(windows)]
pub(crate) fn compile_light_pixel(hlsl: &str, original: &[u8]) -> Result<MapArrowLightPixel> {
    let original_reflection = crate::parse_dxbc_reflection(original)?;
    if !original_reflection
        .resources
        .iter()
        .any(|r| r.resource_type == 2 && r.bind_point == 2)
    {
        return Ok(MapArrowLightPixel::T2Independent(original.to_vec()));
    }
    if original_reflection
        .outputs
        .iter()
        .any(|p| !p.semantic_name.eq_ignore_ascii_case("SV_TARGET"))
        || original_reflection
            .resources
            .iter()
            .any(|r| r.resource_type > 3)
    {
        return Ok(MapArrowLightPixel::RetainedT2Required(original.to_vec()));
    }
    let depth_hlsl = format!("#define main map_arrow_colour_main\n{hlsl}\n#undef main\nvoid main(VS_OUTPUT_MAPARROW Input) {{ map_arrow_colour_main(Input); }}\n");
    let bytecode = match crate::d3d_compile_stage(
        &depth_hlsl,
        crate::ShaderStage::Pixel,
        crate::shader_compile::ENGINE_COMPILE_FLAGS,
    ) {
        Ok(bytecode) => bytecode,
        Err(_) => return Ok(MapArrowLightPixel::RetainedT2Required(original.to_vec())),
    };
    let reflection = crate::parse_dxbc_reflection(&bytecode)?;
    if !reflection.outputs.is_empty()
        || reflection
            .resources
            .iter()
            .any(|r| r.resource_type == 2 && r.bind_point == 2)
    {
        return Ok(MapArrowLightPixel::RetainedT2Required(original.to_vec()));
    }
    Ok(MapArrowLightPixel::T2Independent(bytecode))
}

#[cfg(windows)]
pub(crate) struct MapArrowLightProgram {
    pub pixel: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    pub pixel_bytecode_sha256: String,
    pub external_resources: Vec<u32>,
    pub retained_t2_required: bool,
}

#[cfg(windows)]
impl MapArrowLightProgram {
    pub unsafe fn create(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &crate::GameFileResolver,
        effect: &str,
        defines: &[(String, String)],
        original: &[u8],
    ) -> Result<Self> {
        let hlsl = crate::xmap16_supply_arrow_handoff::build_supply_arrow_stage_hlsl(
            resolver,
            effect,
            crate::ShaderStage::Pixel,
            defines,
        )?;
        let bytecode = compile_light_pixel(&hlsl, original)?;
        Self::from_pixel(device, bytecode)
    }

    unsafe fn from_pixel(
        device: *mut winapi::um::d3d11::ID3D11Device,
        bytecode: MapArrowLightPixel,
    ) -> Result<Self> {
        let retained_t2_required = matches!(&bytecode, MapArrowLightPixel::RetainedT2Required(_));
        let bytecode = bytecode.bytecode();
        let reflection = crate::parse_dxbc_reflection(bytecode)?;
        if reflection.target >> 16 != 0xffff {
            return Err("map-arrow light resources require a pixel shader".into());
        }
        let mut external_resources = Vec::new();
        let mut bindings = std::collections::BTreeSet::new();
        for resource in &reflection.resources {
            if !bindings.insert((resource.resource_type, resource.bind_point)) {
                return Err("map-arrow light reflected resource register repeats".into());
            }
            if resource.resource_type == 0 {
                let expected = match resource.bind_point { 0 => 416, 2 => 16, 3 => 128,
                    slot => return Err(format!("map-arrow light constant buffer b{slot} is not supplied").into()),
                };
                if resource.bind_count != 1 || reflection.cbuffers.iter()
                    .find(|cb| cb.name == resource.name).is_none_or(|cb| cb.size != expected) {
                    return Err(format!("map-arrow light constant buffer b{} layout size is not supplied", resource.bind_point).into());
                }
                continue;
            }
            let slot = crate::xmap16_supply_arrow_handoff::map_arrow_pixel_texture_register(resource)?;
            if resource.resource_type == 2 && slot >= 2 { external_resources.push(slot); }
        }
        let mut raw = std::ptr::null_mut();
        crate::d3d11_device::native_d3d11_hr(
            (*device).CreatePixelShader(
                bytecode.as_ptr() as *const _,
                bytecode.len(),
                std::ptr::null_mut(),
                &mut raw,
            ),
            "CreatePixelShader(map-arrow light)",
        )?;
        Ok(Self {
            pixel: crate::d3d11_device::D3dPtr::from_raw(raw, "map-arrow light pixel")?,
            pixel_bytecode_sha256: Sha256::hex(&bytecode),
            external_resources,
            retained_t2_required,
        })
    }

    pub fn external_views(
        &self,
        views: &[crate::NativeXmap16ExternalResourceView],
        retained: Option<&crate::xmap10_d3d_handoff::NativeXmap10RetainedShadowView<'_>>,
        scene: &NativeMapArrowLightScene,
    ) -> Result<[*mut winapi::um::d3d11::ID3D11ShaderResourceView; 11]> {
        let mut srvs = [std::ptr::null_mut(); 11];
        if self.retained_t2_required {
            let view = retained.ok_or(crate::xmap10_d3d_handoff::Xmap10RetainedShadowRefusal::FirstAllocationUndefined)?;
            view.receipt().validate()?;
            if view.device_identity() != scene.device_identity
                || view.receipt().consumer.camera_sha256 != scene.camera_frame_sha256
                || view.receipt().consumer.replay_sha256 != scene.replay_frame_identity_sha256 {
                return Err("map-arrow retained shadow belongs to another device or frame".into());
            }
            srvs[2] = view.as_raw();
        }
        for slot in &self.external_resources {
            if *slot == 2 {
                if !self.retained_t2_required || srvs[2].is_null() {
                    return Err("map-arrow retained t2 lacks its typed history borrow".into());
                }
                continue;
            }
            let matching = views
                .iter()
                .filter(|v| v.identity.shader_register == *slot)
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(format!(
                    "map-arrow-light-depth-resource-unavailable: t{slot}"
                )
                .into());
            }
            matching[0].identity.validate()?;
            crate::xmap16_supply_arrow_handoff::validate_external_resource_identity(
                &matching[0].identity,
            )?;
            srvs[*slot as usize] = matching[0].as_raw();
        }
        Ok(srvs)
    }
}

#[cfg(windows)]
pub(crate) struct MapArrowLightCommand {
    pub submission: MapArrowLightDraw,
    pub vertex: *mut winapi::um::d3d11::ID3D11VertexShader,
    pub pixel: *mut winapi::um::d3d11::ID3D11PixelShader,
    pub layout: *mut winapi::um::d3d11::ID3D11InputLayout,
    pub vertices: *mut winapi::um::d3d11::ID3D11Buffer,
    pub stride: u32,
    pub depth: *mut winapi::um::d3d11::ID3D11DepthStencilState,
    pub stencil: u32,
    pub rasterizer: *mut winapi::um::d3d11::ID3D11RasterizerState,
    pub srvs: [*mut winapi::um::d3d11::ID3D11ShaderResourceView; 11],
    pub samplers: Vec<(u32, *mut winapi::um::d3d11::ID3D11SamplerState)>,
    pub cb3_buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    pub cb3: Vec<u8>,
}

#[cfg(windows)]
pub struct NativeMapArrowLightCaster<'a> {
    owner_packet: &'static str,
    scene: &'a NativeMapArrowLightScene,
    point_cb2: &'a crate::NativeXmap16ExternalConstantBufferView,
    commands: Vec<MapArrowLightCommand>,
    submissions: Vec<crate::L03LightDepthSubmission>,
    hashes: Vec<String>,
    refusals: Vec<MapArrowLightDrawRefusal>,
    owner: std::marker::PhantomData<&'a ()>,
}

#[cfg(windows)]
impl<'a> NativeMapArrowLightCaster<'a> {
    pub(crate) unsafe fn new(
        owner_packet: &'static str,
        scene: &'a NativeMapArrowLightScene,
        point_cb2: &'a crate::NativeXmap16ExternalConstantBufferView,
        commands: Vec<MapArrowLightCommand>,
        refusals: Vec<MapArrowLightDrawRefusal>,
    ) -> Result<Self> {
        if !matches!(owner_packet, "XMAP15" | "XMAP16") {
            return Err("map-arrow light caster has invalid owner".into());
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut submissions = Vec::new();
        for command in &commands {
            let draw = &command.submission;
            if draw.owner_packet != owner_packet
                || draw.scene_cb0_sha256 != scene.payload_sha256
                || draw.cb3_sha256 != Sha256::hex(&command.cb3)
                || draw.vertex_count == 0
                || !keys.insert((draw.row_ordinal, draw.outer))
            {
                return Err("map-arrow light command differs from its exact census".into());
            }
            if let Some(retained) = &draw.retained_shadow {
                retained.validate()?;
                if command.srvs[2].is_null()
                    || retained.consumer.camera_sha256 != scene.camera_frame_sha256
                    || retained.consumer.replay_sha256 != scene.replay_frame_identity_sha256 {
                    return Err("map-arrow retained shadow receipt differs from native command".into());
                }
            } else if !command.srvs[2].is_null() {
                return Err("map-arrow t2 has no retained allocation witness".into());
            }
            for raw in [
                command.vertex as *mut winapi::um::d3d11::ID3D11DeviceChild,
                command.pixel as *mut _,
                command.layout as *mut _,
                command.vertices as *mut _,
                command.depth as *mut _,
                command.rasterizer as *mut _,
                command.cb3_buffer as *mut _,
                point_cb2.as_raw() as *mut _,
            ]
            .into_iter()
            .chain(
                command
                    .srvs
                    .iter()
                    .filter(|p| !p.is_null())
                    .map(|p| *p as *mut _),
            )
            .chain(command.samplers.iter().map(|(_, p)| *p as *mut _))
            {
                if raw.is_null() {
                    return Err("map-arrow light command has a null native binding".into());
                }
                let mut device = std::ptr::null_mut();
                (*raw).GetDevice(&mut device);
                let owner =
                    crate::d3d11_device::D3dPtr::from_raw(device, "map-arrow binding device")?;
                if owner.as_raw() as usize != scene.device_identity {
                    return Err("map-arrow light binding belongs to another device".into());
                }
            }
            submissions.push(crate::L03LightDepthSubmission::MapArrow { draw: draw.clone() });
        }
        for refusal in &refusals {
            if refusal.owner_packet != owner_packet
                || !keys.insert((refusal.row_ordinal, refusal.outer))
                || refusal.prepared_frame_sha256.len() != 64 {
                return Err("map-arrow refusal duplicates or misidentifies an admitted draw".into());
            }
        }
        let hashes = submissions
            .iter()
            .map(|s| Ok(Sha256::hex(&serde_json::to_vec(s)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            owner_packet,
            scene,
            point_cb2,
            commands,
            submissions,
            hashes,
            refusals,
            owner: std::marker::PhantomData,
        })
    }

    pub fn refusals(&self) -> &[MapArrowLightDrawRefusal] { &self.refusals }

    pub fn submissions(&self) -> &[crate::L03LightDepthSubmission] {
        &self.submissions
    }
}

#[cfg(windows)]
impl crate::NativeXmap09LightDepthCaster for NativeMapArrowLightCaster<'_> {
    fn owner_packet(&self) -> &str {
        self.owner_packet
    }
    fn camera_frame_sha256(&self) -> &str {
        &self.scene.camera_frame_sha256
    }
    fn replay_frame_identity_sha256(&self) -> &str {
        &self.scene.replay_frame_identity_sha256
    }
    fn registered_submission_sha256s(&self) -> &[String] {
        &self.hashes
    }
    fn refused_map_arrow_draws(&self) -> &[MapArrowLightDrawRefusal] { &self.refusals }

    unsafe fn draw_light_depth_submission(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        light: &L03SelectedLightFrame,
        submission: &crate::L03LightDepthSubmission,
    ) -> Result<()> {
        use std::ptr;
        self.scene.validate_binding(
            context,
            light,
            self.camera_frame_sha256(),
            self.replay_frame_identity_sha256(),
        )?;
        let index = self
            .submissions
            .iter()
            .position(|s| s == submission)
            .ok_or("map-arrow light submission was not registered")?;
        let command = &self.commands[index];
        let mut rtv = ptr::null_mut();
        let mut dsv = ptr::null_mut();
        (*context).OMGetRenderTargets(1, &mut rtv, &mut dsv);
        let valid_target = rtv.is_null() && !dsv.is_null();
        if !rtv.is_null() {
            (*rtv).Release();
        }
        if !dsv.is_null() {
            (*dsv).Release();
        }
        if !valid_target {
            return Err("map-arrow light submission requires a DSV-only target".into());
        }
        (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).SOSetTargets(0, ptr::null(), ptr::null());
        (*context).VSSetShader(command.vertex, ptr::null_mut(), 0);
        (*context).PSSetShader(command.pixel, ptr::null_mut(), 0);
        (*context).IASetInputLayout(command.layout);
        (*context)
            .IASetPrimitiveTopology(winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        (*context).IASetVertexBuffers(0, 1, &command.vertices, &command.stride, &0u32);
        (*context).OMSetDepthStencilState(command.depth, command.stencil);
        (*context).OMSetBlendState(ptr::null_mut(), &[0.0f32; 4], u32::MAX);
        (*context).RSSetState(command.rasterizer);
        let scene = self.scene.raw_buffer();
        let point = self.point_cb2.as_raw();
        (*context).VSSetConstantBuffers(0, 1, &scene);
        (*context).PSSetConstantBuffers(0, 1, &scene);
        (*context).PSSetConstantBuffers(2, 1, &point);
        (*context).VSSetConstantBuffers(3, 1, &command.cb3_buffer);
        (*context).PSSetConstantBuffers(3, 1, &command.cb3_buffer);
        (*context).UpdateSubresource(
            command.cb3_buffer as *mut _,
            0,
            ptr::null(),
            command.cb3.as_ptr() as *const _,
            0,
            0,
        );
        (*context).PSSetShaderResources(0, 11, command.srvs.as_ptr());
        for (slot, sampler) in &command.samplers {
            (*context).PSSetSamplers(*slot, 1, sampler);
        }
        (*context).Draw(
            command.submission.vertex_count,
            command.submission.first_vertex,
        );
        (*context).PSSetShaderResources(0, 11, [ptr::null_mut(); 11].as_ptr());
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod shader_tests {
    use super::*;
    const HEADER: &str = "struct VS_OUTPUT_MAPARROW { float4 pos:SV_Position; float2 uv:TEXCOORD0; }; Texture2D Mask:register(t0); Texture2D Shadow:register(t2); SamplerState S:register(s0);";
    fn compiled(body: &str) -> (String, Vec<u8>) {
        let hlsl = format!("{HEADER}\n{body}");
        let bytes = crate::d3d_compile_stage(
            &hlsl,
            crate::ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )
        .unwrap();
        (hlsl, bytes)
    }
    #[test]
    fn light_pixel_eliminates_color_only_t2_and_preserves_discard_dependency() {
        let (hlsl, original) = compiled("float4 main(VS_OUTPUT_MAPARROW Input):SV_Target { clip(Mask.Sample(S,Input.uv).a-0.5); return Shadow.Sample(S,Input.uv); }");
        assert!(crate::parse_dxbc_reflection(&original)
            .unwrap()
            .resources
            .iter()
            .any(|r| r.resource_type == 2 && r.bind_point == 2));
        let depth = compile_light_pixel(&hlsl, &original).unwrap();
        let reflection = crate::parse_dxbc_reflection(depth.bytecode()).unwrap();
        assert!(reflection.outputs.is_empty());
        assert!(reflection
            .resources
            .iter()
            .any(|r| r.resource_type == 2 && r.bind_point == 0));
        assert!(!reflection
            .resources
            .iter()
            .any(|r| r.resource_type == 2 && r.bind_point == 2));
    }
    #[test]
    fn light_pixel_retains_authored_program_when_input_type_prevents_optimization() {
        let hlsl = "struct AuthoredInput { float4 position:SV_POSITION; float2 uv:TEXCOORD0; }; Texture2D AuthoredShadow:register(t2); SamplerState S:register(s2); float4 main(AuthoredInput Input):SV_Target { return AuthoredShadow.Sample(S,Input.uv); }";
        let original = crate::d3d_compile_stage(hlsl, crate::ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS).unwrap();
        assert!(compile_light_pixel(hlsl, b"invalid original DXBC").is_err());
        let classified = compile_light_pixel(hlsl, &original).unwrap();
        assert!(matches!(&classified, MapArrowLightPixel::RetainedT2Required(_)));
        assert_eq!(classified.bytecode(), original);
        assert!(crate::parse_dxbc_reflection(classified.bytecode()).unwrap().resources.iter()
            .any(|resource| resource.resource_type == 2 && resource.bind_point == 2));
    }

    #[test]
    fn light_pixel_requires_retained_t2_for_authored_discard_and_depth() {
        let (hlsl, original) = compiled("float4 main(VS_OUTPUT_MAPARROW Input):SV_Target { clip(Shadow.Sample(S,Input.uv).a-0.5); return float4(1,1,1,1); }");
        let classified = compile_light_pixel(&hlsl, &original).unwrap();
        assert!(matches!(&classified, MapArrowLightPixel::RetainedT2Required(_)));
        assert_eq!(classified.bytecode(), original);
        let (hlsl, original) = compiled(
            "float main(VS_OUTPUT_MAPARROW Input):SV_Depth { return Mask.Sample(S,Input.uv).r; }",
        );
        assert_eq!(compile_light_pixel(&hlsl, &original).unwrap().bytecode(), original);
        let (hlsl, original) = compiled(
            "float main(VS_OUTPUT_MAPARROW Input):SV_Depth { return Shadow.Sample(S,Input.uv).r; }",
        );
        let classified = compile_light_pixel(&hlsl, &original).unwrap();
        assert!(matches!(&classified, MapArrowLightPixel::RetainedT2Required(_)));
        assert_eq!(classified.bytecode(), original);
    }
}

#[cfg(all(test, windows))]
mod native_tests {
    use super::*;
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use crate::NativeXmap09LightDepthCaster;
    use std::{mem, ptr};
    use winapi::shared::dxgiformat::*;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::*;
    unsafe fn buffer(device: *mut ID3D11Device, bytes: &[u8], bind: u32) -> D3dPtr<ID3D11Buffer> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: bytes.len() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: bind,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateBuffer(&desc, &data, &mut raw),
            "test buffer",
        )
        .unwrap();
        D3dPtr::from_raw(raw, "test buffer").unwrap()
    }
    #[test]
    fn light_pixel_resources_bind_by_register_with_typed_provider_custody() {
        unsafe {
            let mut device=ptr::null_mut();let mut context=ptr::null_mut();
            native_d3d11_hr(D3D11CreateDevice(ptr::null_mut(),winapi::um::d3dcommon::D3D_DRIVER_TYPE_WARP,
                ptr::null_mut(),0,ptr::null(),0,D3D11_SDK_VERSION,&mut device,ptr::null_mut(),&mut context),"light positional WARP").unwrap();
            let device=D3dPtr::from_raw(device,"light positional device").unwrap();let context=D3dPtr::from_raw(context,"light positional context").unwrap();
            let mut desc:D3D11_TEXTURE2D_DESC=mem::zeroed();desc.Width=1;desc.Height=1;desc.MipLevels=1;
            desc.ArraySize=1;desc.Format=DXGI_FORMAT_R8G8B8A8_UNORM;desc.SampleDesc.Count=1;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            let mut texture=ptr::null_mut();native_d3d11_hr((*device.as_raw()).CreateTexture2D(&desc,ptr::null(),&mut texture),"light positional texture").unwrap();
            let texture=D3dPtr::from_raw(texture,"light positional texture").unwrap();let mut srv=ptr::null_mut();
            native_d3d11_hr((*device.as_raw()).CreateShaderResourceView(texture.as_raw().cast(),ptr::null(),&mut srv),"light positional SRV").unwrap();
            let srv=D3dPtr::from_raw(srv,"light positional SRV").unwrap();
            let provider=crate::NativeXmap16ExternalResourceView::from_provider(crate::Xmap16SupplyArrowResourceIdentity {
                symbol:"HeightMap".into(),shader_register:4,owner_packet:"XMAP04".into(),payload_sha256:"a".repeat(64),descriptor_sha256:"b".repeat(64),
            },srv.as_raw()).unwrap();
            let scene=NativeMapArrowLightScene {
                buffer:buffer(device.as_raw(),&[0;416],D3D11_BIND_CONSTANT_BUFFER),payload_sha256:"a".repeat(64),selected_light_sha256:"b".repeat(64),
                camera_frame_sha256:"c".repeat(64),replay_frame_identity_sha256:"d".repeat(64),application_elapsed_seconds:0.0,device_identity:device.as_raw() as usize,
            };
            let source="Texture2D HeightMap_Texture:register(t4); SamplerState HeightMap_Sampler:register(s4); float main(float4 uv:TEXCOORD0):SV_Depth { return HeightMap_Texture.Sample(HeightMap_Sampler,uv.xy).r; }";
            let compile=|source:&str,stage|crate::d3d_compile_stage(source,stage,crate::shader_compile::ENGINE_COMPILE_FLAGS).unwrap();
            for hlsl in [source.to_string(),source.replace("HeightMap_Texture","AuthoredImage").replace("HeightMap_Sampler","AuthoredFilter")] {
                let original=compile(&hlsl,crate::ShaderStage::Pixel);
                let pixel=compile_light_pixel(&hlsl,&original).unwrap();
                let program=MapArrowLightProgram::from_pixel(device.as_raw(),pixel).unwrap();
                assert_eq!(program.external_resources,[4]);
                let views=program.external_views(&[provider.clone()],None,&scene).unwrap();
                assert_eq!(views[4],srv.as_raw());assert_eq!(views.iter().filter(|v|!v.is_null()).count(),1);
                (*context.as_raw()).PSSetShader(program.pixel.as_raw(),ptr::null(),0);
                (*context.as_raw()).PSSetShaderResources(0,11,views.as_ptr());
                let mut actual=ptr::null_mut();(*context.as_raw()).PSGetShaderResources(4,1,&mut actual);
                assert_eq!(actual,srv.as_raw());drop(D3dPtr::from_raw(actual,"light positional bound SRV").unwrap());
                assert!(program.external_views(&[],None,&scene).is_err());
                assert!(program.external_views(&[provider.clone(),provider.clone()],None,&scene).is_err());
                let mut wrong_slot=provider.clone();wrong_slot.identity.shader_register=3;wrong_slot.identity.symbol="TerrainIDMap".into();
                assert!(program.external_views(&[wrong_slot],None,&scene).is_err());
                let mut wrong_owner=provider.clone();wrong_owner.identity.owner_packet="XMAP09".into();
                assert!(program.external_views(&[wrong_owner],None,&scene).is_err());
                let mut wrong_symbol=provider.clone();wrong_symbol.identity.symbol="AuthoredImage".into();
                assert!(program.external_views(&[wrong_symbol],None,&scene).is_err());
                let mut wrong_payload=provider.clone();wrong_payload.identity.payload_sha256="invalid".into();
                assert!(program.external_views(&[wrong_payload],None,&scene).is_err());
            }
            for changed in [source.replace("register(t4)","register(t11)"),source.replace("register(s4)","register(s11)"),
                source.replace("Texture2D HeightMap_Texture","Texture2DArray HeightMap_Texture").replace("uv.xy","float3(uv.xy,0)"),
                "cbuffer Missing:register(b1) {float4 v;}; float main():SV_Depth {return v.x;}".into(),
                "cbuffer WrongSize:register(b2) {float4 v;float4 w;}; float main():SV_Depth {return v.x+w.x;}".into(),
            ] {
                let pixel=MapArrowLightPixel::T2Independent(compile(&changed,crate::ShaderStage::Pixel));
                assert!(MapArrowLightProgram::from_pixel(device.as_raw(),pixel).is_err(),"{changed}");
            }
            let vertex=compile("float4 main(float3 p:POSITION):SV_POSITION {return float4(p,1);}",crate::ShaderStage::Vertex);
            assert!(MapArrowLightProgram::from_pixel(device.as_raw(),MapArrowLightPixel::T2Independent(vertex)).is_err());
            let retained=source.replace("register(t4)","register(t2)");let original=compile(&retained,crate::ShaderStage::Pixel);
            let program=MapArrowLightProgram::from_pixel(device.as_raw(),compile_light_pixel(&retained,&original).unwrap()).unwrap();
            assert!(program.retained_t2_required);
            let mut ordinary_shadow=provider;ordinary_shadow.identity.symbol="ShadowMap".into();ordinary_shadow.identity.shader_register=2;ordinary_shadow.identity.owner_packet="XMAP10".into();
            assert!(program.external_views(&[ordinary_shadow],None,&scene).is_err());
        }
    }

    #[test]
    fn light_arrow_native_dsv_draw_preserves_authored_clip_and_target() {
        unsafe {
            let mut raw_device = ptr::null_mut();
            let mut raw_context = ptr::null_mut();
            native_d3d11_hr(
                D3D11CreateDevice(
                    ptr::null_mut(),
                    winapi::um::d3dcommon::D3D_DRIVER_TYPE_WARP,
                    ptr::null_mut(),
                    0,
                    ptr::null(),
                    0,
                    D3D11_SDK_VERSION,
                    &mut raw_device,
                    ptr::null_mut(),
                    &mut raw_context,
                ),
                "WARP light test",
            )
            .unwrap();
            let device = D3dPtr::from_raw(raw_device, "test device").unwrap();
            let context = D3dPtr::from_raw(raw_context, "test context").unwrap();
            let header = "struct VS_OUTPUT_MAPARROW { float4 pos:SV_Position; };";
            let vs = crate::d3d_compile_stage(&format!("{header} VS_OUTPUT_MAPARROW main(float4 p:POSITION) {{ VS_OUTPUT_MAPARROW o; o.pos=p; return o; }}"), crate::ShaderStage::Vertex, crate::shader_compile::ENGINE_COMPILE_FLAGS).unwrap();
            let pixel_hlsl = format!("{header} Texture2D Shadow:register(t2); float4 main(VS_OUTPUT_MAPARROW Input):SV_Target {{ clip(Input.pos.x-2.0); return Shadow.Load(int3(Input.pos.xy,0)); }}");
            let original = crate::d3d_compile_stage(
                &pixel_hlsl,
                crate::ShaderStage::Pixel,
                crate::shader_compile::ENGINE_COMPILE_FLAGS,
            )
            .unwrap();
            let classified = compile_light_pixel(&pixel_hlsl, &original).unwrap();
            let ps = classified.bytecode();
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateVertexShader(
                    vs.as_ptr() as *const _,
                    vs.len(),
                    ptr::null_mut(),
                    &mut raw,
                ),
                "test VS",
            )
            .unwrap();
            let vertex = D3dPtr::from_raw(raw, "test VS").unwrap();
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreatePixelShader(
                    ps.as_ptr() as *const _,
                    ps.len(),
                    ptr::null_mut(),
                    &mut raw,
                ),
                "test PS",
            )
            .unwrap();
            let pixel = D3dPtr::from_raw(raw, "test PS").unwrap();
            let input = D3D11_INPUT_ELEMENT_DESC {
                SemanticName: b"POSITION\0".as_ptr() as *const _,
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateInputLayout(
                    &input,
                    1,
                    vs.as_ptr() as *const _,
                    vs.len(),
                    &mut raw,
                ),
                "test layout",
            )
            .unwrap();
            let layout = D3dPtr::from_raw(raw, "test layout").unwrap();
            let vertices = [
                -1.0f32, -1.0, 0.25, 1.0, -1.0, 3.0, 0.25, 1.0, 3.0, -1.0, 0.25, 1.0,
            ]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
            let vertices = buffer(raw_device, &vertices, D3D11_BIND_VERTEX_BUFFER);
            let cb3 = buffer(raw_device, &[0; 128], D3D11_BIND_CONSTANT_BUFFER);
            let point_buffer = buffer(raw_device, &[0; 16], D3D11_BIND_CONSTANT_BUFFER);
            let point = crate::NativeXmap16ExternalConstantBufferView::from_provider(
                "PointLightsCB2",
                2,
                "XMAP09",
                16,
                Sha256::hex(&[0; 16]),
                "a".repeat(64),
                point_buffer.as_raw(),
            )
            .unwrap();
            let light = L03SelectedLightFrame {
                selected_light: crate::L03SelectedLight::Sun,
                light_view_projection_f32_bits: [0; 16],
                shadow_map_texture_matrix_f32_bits: [0; 16],
                light_depth_target: crate::L03LightDepthTarget {
                    stable_resource_id: "test".into(),
                    width: 4,
                    height: 4,
                    dxgi_format: 41,
                    mip_levels: 1,
                },
            };
            let scene = NativeMapArrowLightScene {
                buffer: buffer(raw_device, &[0; 416], D3D11_BIND_CONSTANT_BUFFER),
                payload_sha256: Sha256::hex(&[0; 416]),
                selected_light_sha256: Sha256::hex(&serde_json::to_vec(&light).unwrap()),
                camera_frame_sha256: "a".repeat(64),
                replay_frame_identity_sha256: "b".repeat(64),
                application_elapsed_seconds: 0.0,
                device_identity: raw_device as usize,
            };
            let mut desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
            desc.DepthEnable = 1;
            desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
            desc.DepthFunc = D3D11_COMPARISON_LESS;
            desc.FrontFace.StencilFunc = D3D11_COMPARISON_ALWAYS;
            desc.FrontFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
            desc.FrontFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
            desc.FrontFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
            desc.BackFace = desc.FrontFace;
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateDepthStencilState(&desc, &mut raw),
                "test depth",
            )
            .unwrap();
            let depth = D3dPtr::from_raw(raw, "test depth").unwrap();
            let mut desc: D3D11_RASTERIZER_DESC = mem::zeroed();
            desc.FillMode = D3D11_FILL_SOLID;
            desc.CullMode = D3D11_CULL_NONE;
            desc.DepthClipEnable = 1;
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateRasterizerState(&desc, &mut raw),
                "test raster",
            )
            .unwrap();
            let raster = D3dPtr::from_raw(raw, "test raster").unwrap();
            let submission = MapArrowLightDraw {
                owner_packet: "XMAP15".into(),
                prepared_frame_sha256: "c".repeat(64),
                row_ordinal: 0,
                outer: false,
                geometry_sha256: "d".repeat(64),
                scene_cb0_sha256: scene.payload_sha256.clone(),
                cb3_sha256: Sha256::hex(&[0; 128]),
                pixel_bytecode_sha256: Sha256::hex(&ps),
                vertex_count: 3,
                first_vertex: 0,
                retained_shadow: None,
            };
            let command = MapArrowLightCommand {
                submission,
                vertex: vertex.as_raw(),
                pixel: pixel.as_raw(),
                layout: layout.as_raw(),
                vertices: vertices.as_raw(),
                stride: 16,
                depth: depth.as_raw(),
                stencil: 0,
                rasterizer: raster.as_raw(),
                srvs: [ptr::null_mut(); 11],
                samplers: Vec::new(),
                cb3_buffer: cb3.as_raw(),
                cb3: vec![0; 128],
            };
            let mut caster =
                NativeMapArrowLightCaster::new("XMAP15", &scene, &point, vec![command], Vec::new()).unwrap();
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 4,
                Height: 4,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_D32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_DEPTH_STENCIL,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateTexture2D(&desc, ptr::null(), &mut raw),
                "test depth texture",
            )
            .unwrap();
            let target = D3dPtr::from_raw(raw, "test depth texture").unwrap();
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateDepthStencilView(
                    target.as_raw() as *mut _,
                    ptr::null(),
                    &mut raw,
                ),
                "test DSV",
            )
            .unwrap();
            let dsv = D3dPtr::from_raw(raw, "test DSV").unwrap();
            (*raw_context).OMSetRenderTargets(0, ptr::null(), dsv.as_raw());
            (*raw_context).ClearDepthStencilView(dsv.as_raw(), D3D11_CLEAR_DEPTH, 1.0, 0);
            (*raw_context).RSSetViewports(
                1,
                &D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: 4.0,
                    Height: 4.0,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                },
            );
            let gs = crate::d3d11_device::native_d3d11_compile_shader(
                "struct V { float4 p : SV_POSITION; }; [maxvertexcount(3)] void main(triangle V input[3], inout TriangleStream<V> stream) { }",
                "main", "gs_5_0",
            ).unwrap();
            let mut geometry = ptr::null_mut();
            native_d3d11_hr((*raw_device).CreateGeometryShader(
                (*gs.as_raw()).GetBufferPointer(), (*gs.as_raw()).GetBufferSize(),
                ptr::null_mut(), &mut geometry), "light hostile GS").unwrap();
            let geometry = D3dPtr::from_raw(geometry, "light hostile GS").unwrap();
            let stream_output = buffer(raw_device, &[0; 256], D3D11_BIND_STREAM_OUTPUT);
            (*raw_context).GSSetShader(geometry.as_raw(), ptr::null(), 0);
            (*raw_context).SOSetTargets(1, &stream_output.as_raw(), &0);
            let submission = caster.submissions()[0].clone();
            caster
                .draw_light_depth_submission(raw_context, &light, &submission)
                .unwrap();
            let mut observed_geometry = ptr::null_mut();
            (*raw_context).GSGetShader(&mut observed_geometry, ptr::null_mut(), ptr::null_mut());
            assert!(observed_geometry.is_null());
            let mut observed_stream = ptr::null_mut();
            (*raw_context).SOGetTargets(1, &mut observed_stream);
            assert!(observed_stream.is_null());
            let mut observed = ptr::null_mut();
            (*raw_context).OMGetRenderTargets(0, ptr::null_mut(), &mut observed);
            assert_eq!(observed, dsv.as_raw());
            (*observed).Release();
            (*raw_context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
            assert!(caster
                .draw_light_depth_submission(raw_context, &light, &submission)
                .is_err());
            let desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ,
                ..desc
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*raw_device).CreateTexture2D(&desc, ptr::null(), &mut raw),
                "test staging",
            )
            .unwrap();
            let staging = D3dPtr::from_raw(raw, "test staging").unwrap();
            (*raw_context).CopyResource(staging.as_raw() as *mut _, target.as_raw() as *mut _);
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            native_d3d11_hr(
                (*raw_context).Map(
                    staging.as_raw() as *mut _,
                    0,
                    D3D11_MAP_READ,
                    0,
                    &mut mapped,
                ),
                "test Map",
            )
            .unwrap();
            for y in 0..4 {
                for x in 0..4 {
                    let value = *((mapped.pData as *const u8)
                        .add(y * mapped.RowPitch as usize + x * 4)
                        as *const f32);
                    assert_eq!(value, if x < 2 { 1.0 } else { 0.25 });
                }
            }
            (*raw_context).Unmap(staging.as_raw() as *mut _, 0);
            (*raw_context).ClearState();
            drop(context);
            drop(device);
        }
    }
}
