use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
use crate::particle_authored::ParticleResult;
use crate::particle_runtime::{ParticleGpuSubsystem, ParticleSortCamera};
use crate::Result;
use std::{mem, ptr};
use winapi::um::d3d11::*;

pub(crate) struct ParticleRecordUpload {
    device: D3dPtr<ID3D11Device>,
    buffer: Option<D3dPtr<ID3D11Buffer>>,
    capacity_bytes: usize,
}

pub(crate) struct UploadedParticleRecords<'buffer> {
    pub buffer: &'buffer D3dPtr<ID3D11Buffer>,
    pub count: u32,
    pub payload_sha256: String,
    pub stride: u32,
    pub trail: bool,
    pub world_matrix_f32_bits: [u32; 16],
}

impl ParticleRecordUpload {
    pub(crate) unsafe fn new(device: &D3dPtr<ID3D11Device>) -> Result<Self> {
        (*device.as_raw()).AddRef();
        Ok(Self {
            device: D3dPtr::from_raw(device.as_raw(), "particle retained device")?,
            buffer: None,
            capacity_bytes: 0,
        })
    }

    pub(crate) unsafe fn upload(
        &mut self,
        context: &D3dPtr<ID3D11DeviceContext>,
        subsystem: ParticleGpuSubsystem<'_>,
        presentation_id: u64,
        camera: ParticleSortCamera,
        scale_f32_bits: u32,
    ) -> Result<Option<UploadedParticleRecords<'_>>> {
        subsystem.validate_draw_presentation(presentation_id, camera)?;
        let world_matrix_f32_bits = subsystem.draw_world_matrix_f32_bits()?;
        let trail = subsystem.trail();
        let stride = if trail { 28 } else { 48 };
        let count = if trail {
            if subsystem.live_count() < 2 {
                0
            } else {
                subsystem
                    .live_count()
                    .checked_mul(2)
                    .ok_or("PARTICLE_GPU_UPLOAD trail count overflow")?
            }
        } else {
            subsystem.live_count()
        };
        let count = u32::try_from(count)?;
        if count == 0 {
            return Ok(None);
        }
        if subsystem.live_count() > subsystem.authored_capacity() {
            return Err("PARTICLE_GPU_UPLOAD live prefix exceeds authored capacity".into());
        }
        // Validate the complete live prefix before changing the GPU resource.
        visit_upload_bytes(subsystem, camera, scale_f32_bits, |_| Ok(()))?;
        let mut context_device = ptr::null_mut();
        (*context.as_raw()).GetDevice(&mut context_device);
        let context_device = D3dPtr::from_raw(context_device, "particle context device")?;
        if context_device.as_raw() != self.device.as_raw() {
            return Err("PARTICLE_GPU_UPLOAD context belongs to another device".into());
        }
        let capacity_bytes = subsystem
            .authored_capacity()
            .checked_mul(if trail { 56 } else { 48 })
            .ok_or("PARTICLE_GPU_UPLOAD authored capacity byte overflow")?;
        let byte_width = u32::try_from(capacity_bytes)?;
        if capacity_bytes > self.capacity_bytes {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: byte_width,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut buffer = ptr::null_mut();
            native_d3d11_hr(
                (*self.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer),
                "particle record buffer",
            )?;
            self.buffer = Some(D3dPtr::from_raw(buffer, "particle record buffer")?);
            self.capacity_bytes = capacity_bytes;
        }
        let buffer = self
            .buffer
            .as_ref()
            .ok_or("PARTICLE_GPU_UPLOAD missing allocated buffer")?;
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
        native_d3d11_hr(
            (*context.as_raw()).Map(
                buffer.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            ),
            "particle record upload",
        )?;
        let mapping = ParticleBufferMapping { context, buffer };
        if mapped.pData.is_null() {
            return Err("PARTICLE_GPU_UPLOAD mapped buffer returned null".into());
        }
        let mut offset = 0;
        let mut payload = snapshot::sha256::Sha256::new();
        visit_upload_bytes(subsystem, camera, scale_f32_bits, |bytes| {
            ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (mapped.pData as *mut u8).add(offset),
                bytes.len(),
            );
            payload.update(bytes);
            offset += bytes.len();
            Ok(())
        })?;
        drop(mapping);
        Ok(Some(UploadedParticleRecords {
            buffer,
            count,
            payload_sha256: snapshot::sha256::to_hex(&payload.finalize()),
            stride,
            trail,
            world_matrix_f32_bits,
        }))
    }
}

struct ParticleBufferMapping<'a> {
    context: &'a D3dPtr<ID3D11DeviceContext>,
    buffer: &'a D3dPtr<ID3D11Buffer>,
}

impl Drop for ParticleBufferMapping<'_> {
    fn drop(&mut self) {
        unsafe {
            (*self.context.as_raw()).Unmap(self.buffer.as_raw() as *mut _, 0);
        }
    }
}

fn visit_upload_bytes(
    subsystem: ParticleGpuSubsystem<'_>,
    camera: ParticleSortCamera,
    scale_f32_bits: u32,
    mut visit: impl FnMut(&[u8]) -> ParticleResult<()>,
) -> ParticleResult<()> {
    if subsystem.trail() {
        subsystem.visit_trail_vertices(camera.position_f32_bits, scale_f32_bits, |bytes| {
            visit(&bytes)
        })
    } else {
        for record in subsystem.records() {
            visit(&record?.0)?;
        }
        Ok(())
    }
}

pub(crate) struct ParticleMaterial {
    authored: crate::particle_authored::ParticleTexture,
    trail: bool,
    billboard: bool,
    pub vertex_shader: D3dPtr<ID3D11VertexShader>,
    pub pixel_shader: D3dPtr<ID3D11PixelShader>,
    pub input_layout: D3dPtr<ID3D11InputLayout>,
    pub vertex_reflection: crate::shader_compile::DxbcReflection,
    pub pixel_reflection: crate::shader_compile::DxbcReflection,
    pub samplers: Vec<(u32, D3dPtr<ID3D11SamplerState>)>,
    pub blend: D3dPtr<ID3D11BlendState>,
    pub rasterizer: D3dPtr<ID3D11RasterizerState>,
    pub texture: D3dPtr<ID3D11Texture2D>,
    pub texture_view: D3dPtr<ID3D11ShaderResourceView>,
    pub texture_dimensions: [u32; 2],
    pub identity_sha256: String,
}

pub(crate) struct ParticleShaderCatalog {
    lease: crate::mod_resolver::ContentAdmissionLease,
    key: portable_content_manifest::UsedContentKey,
    file: crate::EffectFile,
}

impl ParticleShaderCatalog {
    pub(crate) fn load(resolver: &crate::GameFileResolver) -> Result<Self> {
        let lease = resolver
            .content_admission_lease()
            .ok_or("PARTICLE_MATERIAL content admission required")?;
        let key = resolver
            .admitted_used_content_key("gfx/FX/particle.shader")?
            .ok_or("PARTICLE_MATERIAL gfx/FX/particle.shader is not admitted")?;
        let bytes = lease.read_used_content(&key)?;
        let file =
            crate::shader_compile::parse_effect_file(&crate::latin1_bytes_to_string(&bytes))?;
        Ok(Self { lease, key, file })
    }

    pub(crate) unsafe fn create_material(
        &self,
        device: &D3dPtr<ID3D11Device>,
        resolver: &crate::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        subsystem: crate::particle_runtime::ParticleGpuSubsystem<'_>,
        engine_defines: &[(String, String)],
    ) -> Result<ParticleMaterial> {
        use crate::shader_compile::{
            build_stage_hlsl_with_defines, d3d_compile_stage, parse_dxbc_reflection, ShaderStage,
            ENGINE_COMPILE_FLAGS,
        };
        use crate::xmap12_d3d_handoff::{
            create_admitted_material_dds_on_device, create_effect_sampler_on_device,
            create_effect_states,
        };
        if subsystem.content_lease() != &self.lease
            || resolver.content_admission_lease().as_ref() != Some(&self.lease)
        {
            return Err("PARTICLE_MATERIAL content session mismatch".into());
        }
        let texture = subsystem
            .texture()
            .ok_or("PARTICLE_MATERIAL authored texture unavailable")?;
        let mut shader_name = texture.shader.clone();
        if subsystem.trail() {
            shader_name.push_str("Trail");
        }
        if !subsystem.billboard() {
            shader_name.push_str("NoBillboard");
        }
        if self.file.effect_count(&shader_name) != 1 {
            return Err(format!(
                "PARTICLE_MATERIAL shader effect must resolve exactly once: {shader_name}"
            )
            .into());
        }
        let key = &self.key;
        let file = &self.file;
        let effect = file.effect(&shader_name).unwrap();
        let path = &key.normalized_game_relative_path;
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            path,
            &shader_name,
            ShaderStage::Vertex,
            engine_defines,
        )?;
        let pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            path,
            &shader_name,
            ShaderStage::Pixel,
            engine_defines,
        )?;
        let vertex_bytes =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytes = d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let input_layout = crate::d3d11_device::native_d3d11_create_input_layout_from_elements(
            device,
            &particle_input_elements(subsystem.trail()),
            &vertex_bytes,
        )?;
        let vertex_reflection = parse_dxbc_reflection(&vertex_bytes)?;
        let pixel_reflection = parse_dxbc_reflection(&pixel_bytes)?;
        if vertex_reflection
            .resources
            .iter()
            .any(|resource| resource.resource_type == 3)
        {
            return Err("PARTICLE_MATERIAL vertex sampler binding is unavailable".into());
        }
        let mut samplers = Vec::new();
        let mut sampler_identities = Vec::new();
        for resource in pixel_reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 3)
        {
            if resource.bind_count != 1 {
                return Err(format!(
                    "PARTICLE_MATERIAL sampler array unavailable: {}",
                    resource.name
                )
                .into());
            }
            let (sampler, identity) = create_effect_sampler_on_device(
                device.as_raw(),
                resolver,
                path,
                resource.bind_point,
            )?;
            samplers.push((resource.bind_point, sampler));
            sampler_identities.push((resource.bind_point, identity));
        }
        let (blend, _depth, rasterizer, state_identity) =
            create_effect_states(device.as_raw(), file, effect)?;
        let texture_key = resolver
            .admitted_used_content_key(&texture.file)?
            .ok_or_else(|| {
                format!(
                    "PARTICLE_MATERIAL texture is not admitted: {}",
                    texture.file
                )
            })?;
        let (texture_resource, texture_view, texture_identity) =
            create_admitted_material_dds_on_device(
                device.as_raw(),
                resolver,
                admission,
                &texture_key,
            )?;
        let mut descriptor: D3D11_TEXTURE2D_DESC = mem::zeroed();
        (*texture_resource.as_raw()).GetDesc(&mut descriptor);
        let mut vertex_shader = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateVertexShader(
                vertex_bytes.as_ptr() as *const _,
                vertex_bytes.len(),
                ptr::null_mut(),
                &mut vertex_shader,
            ),
            "particle vertex shader",
        )?;
        let vertex_shader = D3dPtr::from_raw(vertex_shader, "particle vertex shader")?;
        let mut pixel_shader = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreatePixelShader(
                pixel_bytes.as_ptr() as *const _,
                pixel_bytes.len(),
                ptr::null_mut(),
                &mut pixel_shader,
            ),
            "particle pixel shader",
        )?;
        let pixel_shader = D3dPtr::from_raw(pixel_shader, "particle pixel shader")?;
        let identity_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
            key,
            texture_key,
            shader_name.as_str(),
            texture.columns,
            texture.rows,
            snapshot::sha256::Sha256::hex(&vertex_bytes),
            snapshot::sha256::Sha256::hex(&pixel_bytes),
            sampler_identities,
            state_identity,
            texture_identity,
        ))?);
        Ok(ParticleMaterial {
            authored: texture.clone(),
            trail: subsystem.trail(),
            billboard: subsystem.billboard(),
            vertex_shader,
            pixel_shader,
            input_layout,
            vertex_reflection,
            pixel_reflection,
            samplers,
            blend,
            rasterizer,
            texture: texture_resource,
            texture_view,
            texture_dimensions: [descriptor.Width, descriptor.Height],
            identity_sha256,
        })
    }
}

fn particle_input_elements(trail: bool) -> Vec<crate::RuntimeInputElementMeta> {
    use winapi::shared::dxgiformat::*;
    let rows: &[(&str, u32, u32, u32, u32, bool)] = if trail {
        &[
            ("POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, false),
            ("TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0, 12, false),
            ("TEXCOORD", 1, DXGI_FORMAT_R8_UINT, 0, 20, false),
            ("COLOR", 0, DXGI_FORMAT_B8G8R8A8_UNORM, 0, 24, false),
        ]
    } else {
        &[
            ("TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0, 0, false),
            ("TEXCOORD", 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 0, true),
            ("TEXCOORD", 2, DXGI_FORMAT_R32G32B32_FLOAT, 1, 16, true),
            ("TEXCOORD", 3, DXGI_FORMAT_R8_UINT, 1, 28, true),
            ("COLOR", 0, DXGI_FORMAT_B8G8R8A8_UNORM, 1, 32, true),
        ]
    };
    rows.iter()
        .map(
            |&(name, index, format, slot, offset, instanced)| crate::RuntimeInputElementMeta {
                semantic_name: name.into(),
                semantic_index: index,
                format,
                input_slot: slot,
                aligned_byte_offset: offset,
                input_slot_class: u32::from(instanced),
                instance_data_step_rate: u32::from(instanced),
            },
        )
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ParticleDrawCounts {
    pub packed_subsystems: u64,
    pub draw_calls: u64,
    pub normal_instances: u64,
    pub trail_vertices: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleDrawReceipt {
    counts: ParticleDrawCounts,
    submission_sha256: String,
}

impl ParticleDrawReceipt {
    pub fn counts(&self) -> ParticleDrawCounts {
        self.counts
    }

    pub fn submission_sha256(&self) -> &str {
        &self.submission_sha256
    }
}

pub struct ParticleNativeDraw {
    upload: ParticleRecordUpload,
    catalog: ParticleShaderCatalog,
    engine_defines: Vec<(String, String)>,
    materials: Vec<ParticleMaterial>,
    quad: D3dPtr<ID3D11Buffer>,
    indices: D3dPtr<ID3D11Buffer>,
    projection: D3dPtr<ID3D11Buffer>,
    scale: D3dPtr<ID3D11Buffer>,
    instance: D3dPtr<ID3D11Buffer>,
    depth: D3dPtr<ID3D11DepthStencilState>,
}

pub struct ParticleSceneBindings<'a> {
    pub target: &'a dyn crate::NativeXmap16MainSceneTargetView,
    pub scene: &'a dyn crate::NativeXmap16SceneConstantBufferView,
    pub camera: crate::ProductionMapCamera,
    pub shared: &'a dyn Fn(
        &crate::NativeXmap12ReflectedSharedResource,
    ) -> Result<crate::NativeXmap12SharedResourceBinding>,
}

impl ParticleNativeDraw {
    pub unsafe fn create_on_device(
        raw_device: *mut ID3D11Device,
        resolver: &crate::GameFileResolver,
        engine_defines: Vec<(String, String)>,
    ) -> Result<Self> {
        use crate::d3d11_device::{
            native_d3d11_create_buffer, native_d3d11_create_exact_constant_buffer,
            native_d3d11_create_generated_vertex_buffer,
        };
        if raw_device.is_null() {
            return Err("PARTICLE_DRAW device unavailable".into());
        }
        (*raw_device).AddRef();
        let device_owner = D3dPtr::from_raw(raw_device, "particle creation device")?;
        let device = &device_owner;
        let quad_bytes: Vec<u8> = [0.0_f32, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let quad = native_d3d11_create_generated_vertex_buffer(
            device,
            0,
            8,
            &quad_bytes,
            "particle quad",
        )?
        .buffer;
        let index_bytes: Vec<u8> = [0_u16, 1, 2, 2, 1, 3]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        let indices = native_d3d11_create_buffer(
            device,
            &crate::MapFrameBufferBinding {
                stage: "IA".into(),
                slot: 0,
                declared_name: None,
                file: None,
                byte_len: index_bytes.len(),
                fnv1a64: None,
                stride: None,
                offset: Some(0),
                dxgi_format: Some(winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT),
                bind_flags: D3D11_BIND_INDEX_BUFFER,
                usage: D3D11_USAGE_DEFAULT,
                cpu_access_flags: 0,
            },
            &index_bytes,
            "particle quad indices",
        )?;
        Ok(Self {
            depth: crate::xmap12_d3d_handoff::create_depth_state(
                device.as_raw(),
                &[("DepthWriteMask".into(), "ZERO".into())],
            )?,
            upload: ParticleRecordUpload::new(device)?,
            catalog: ParticleShaderCatalog::load(resolver)?,
            engine_defines,
            materials: Vec::new(),
            quad,
            indices,
            projection: native_d3d11_create_exact_constant_buffer(
                device,
                &[0; 64],
                "particle projection",
            )?,
            scale: native_d3d11_create_exact_constant_buffer(device, &[0; 16], "particle scale")?,
            instance: native_d3d11_create_exact_constant_buffer(
                device,
                &[0; 96],
                "particle instance",
            )?,
        })
    }

    pub unsafe fn draw_instance(
        &mut self,
        raw_context: *mut ID3D11DeviceContext,
        resolver: &crate::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        capacity: &crate::particle_authored::ParticleDrawCapacity,
        view: crate::particle_runtime::ParticleGpuView<'_>,
        presentation_id: u64,
        sort_camera: ParticleSortCamera,
        bindings: &ParticleSceneBindings<'_>,
    ) -> Result<ParticleDrawReceipt> {
        if raw_context.is_null()
            || view.content_lease() != &capacity.lease
            || view.content_lease() != &self.catalog.lease
        {
            return Err("PARTICLE_DRAW context or catalog capacity identity mismatch".into());
        }
        (*raw_context).AddRef();
        let context = D3dPtr::from_raw(raw_context, "particle draw context")?;
        let scale = view.draw_scale_f32_bits()?;
        let mut counts = ParticleDrawCounts::default();
        let mut submission_sha256 =
            snapshot::sha256::Sha256::hex(b"PARTICLE_DRAW.native-submissions.a729.v1");
        for trail in [false, true] {
            let packed =
                view.visit_draw_subsystems(capacity.packets, trail, |ordinal, subsystem| {
                    if subsystem.authored_capacity() > capacity.particles {
                        return Err(
                            "PARTICLE_DRAW subsystem exceeds admitted global particle capacity"
                                .into(),
                        );
                    }
                    let submitted = self.draw_subsystem(
                        &context,
                        resolver,
                        admission,
                        subsystem,
                        presentation_id,
                        sort_camera,
                        scale,
                        bindings,
                    )?;
                    if let Some((count, digest)) = submitted {
                        submission_sha256 = replay_core::canonical_json_sha256(&(
                            &submission_sha256,
                            trail,
                            ordinal,
                            count,
                            digest,
                        ))?;
                        counts.draw_calls = counts
                            .draw_calls
                            .checked_add(1)
                            .ok_or("PARTICLE_DRAW draw count overflow")?;
                        let total = if trail {
                            &mut counts.trail_vertices
                        } else {
                            &mut counts.normal_instances
                        };
                        *total = total
                            .checked_add(u64::from(count))
                            .ok_or("PARTICLE_DRAW record count overflow")?;
                    }
                    Ok(())
                })?;
            counts.packed_subsystems = u64::try_from(packed)?;
        }
        Ok(ParticleDrawReceipt {
            counts,
            submission_sha256,
        })
    }

    pub(crate) unsafe fn draw_subsystem(
        &mut self,
        context: &D3dPtr<ID3D11DeviceContext>,
        resolver: &crate::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        subsystem: ParticleGpuSubsystem<'_>,
        presentation_id: u64,
        sort_camera: ParticleSortCamera,
        scale_f32_bits: [u32; 2],
        bindings: &ParticleSceneBindings<'_>,
    ) -> Result<Option<(u32, String)>> {
        subsystem.validate_draw_presentation(presentation_id, sort_camera)?;
        bindings.camera.validate()?;
        bindings.target.desc().validate()?;
        if bindings.camera.position.map(f32::to_bits) != sort_camera.position_f32_bits
            || !scale_f32_bits
                .into_iter()
                .all(|value| f32::from_bits(value).is_finite())
            || bindings.target.color_rtv().is_null()
            || bindings.target.depth_dsv().is_null()
            || bindings.scene.raw_buffer().is_null()
            || bindings.scene.shader_register() != 0
            || resolver.content_admission_lease().as_ref() != Some(&self.catalog.lease)
        {
            return Err("PARTICLE_DRAW scene/camera/content binding mismatch".into());
        }
        if subsystem.live_count() == 0 || (subsystem.trail() && subsystem.live_count() < 2) {
            return Ok(None);
        }
        let authored = subsystem
            .texture()
            .ok_or("PARTICLE_DRAW authored texture unavailable")?;
        let index = match self.materials.iter().position(|material| {
            &material.authored == authored
                && material.trail == subsystem.trail()
                && material.billboard == subsystem.billboard()
        }) {
            Some(index) => index,
            None => {
                let material = self.catalog.create_material(
                    &self.upload.device,
                    resolver,
                    admission,
                    subsystem,
                    &self.engine_defines,
                )?;
                self.materials.push(material);
                self.materials.len() - 1
            }
        };
        let material = &self.materials[index];
        let shared = material
            .pixel_reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 2 && resource.bind_point != 0)
            .map(|resource| {
                (bindings.shared)(&crate::NativeXmap12ReflectedSharedResource {
                    semantic: resource.name.clone(),
                    shader_register: resource.bind_point,
                    view_dimension: resource.dimension,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        validate_particle_resources(
            &material.vertex_reflection,
            &material.pixel_reflection,
            bindings.scene.byte_len(),
            material.texture_view.as_raw(),
            &shared,
        )?;
        let cb3 = particle_instance_bytes(subsystem, material.texture_dimensions)?;
        let uploaded = self
            .upload
            .upload(
                context,
                subsystem,
                presentation_id,
                sort_camera,
                scale_f32_bits[1],
            )?
            .ok_or("PARTICLE_DRAW live subsystem yielded no upload")?;
        let projection: Vec<u8> = bindings
            .camera
            .view_projection_matrix
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut scale = [0_u8; 16];
        for (index, bits) in scale_f32_bits.into_iter().enumerate() {
            scale[index * 4..index * 4 + 4].copy_from_slice(&bits.to_le_bytes());
        }
        let shared_identities: Vec<_> = shared
            .iter()
            .map(|binding| {
                (
                    &binding.semantic,
                    binding.shader_register,
                    &binding.payload_sha256,
                    &binding.descriptor_sha256,
                    binding.view_dimension,
                )
            })
            .collect();
        let submission_sha256 = replay_core::canonical_json_sha256(&(
            "PARTICLE_DRAW.native-payload.a729.v1",
            (
                &uploaded.payload_sha256,
                uploaded.count,
                uploaded.stride,
                uploaded.trail,
            ),
            (&material.identity_sha256, material.texture_dimensions),
            (&projection, &scale, cb3.as_slice()),
            (
                bindings.scene.payload_sha256(),
                bindings.scene.descriptor_sha256(),
                bindings.scene.composition_receipt_sha256(),
                bindings.scene.reflected_layout_sha256(),
            ),
            shared_identities,
            bindings.target.desc(),
            "depth-enabled/write-zero/less-equal/stencil-disabled/ref-zero",
        ))?;
        let raw = context.as_raw();
        for (buffer, bytes) in [
            (&self.projection, projection.as_slice()),
            (&self.scale, &scale[..]),
            (&self.instance, &cb3[..]),
        ] {
            (*raw).UpdateSubresource(
                buffer.as_raw() as *mut _,
                0,
                ptr::null(),
                bytes.as_ptr() as *const _,
                0,
                0,
            );
        }
        (*raw).OMSetRenderTargets(1, &bindings.target.color_rtv(), bindings.target.depth_dsv());
        (*raw).RSSetViewports(
            1,
            &D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: bindings.target.desc().color_width as f32,
                Height: bindings.target.desc().color_height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            },
        );
        let constant_buffers = [
            bindings.scene.raw_buffer(),
            self.projection.as_raw(),
            self.scale.as_raw(),
            self.instance.as_raw(),
        ];
        (*raw).VSSetConstantBuffers(0, 4, constant_buffers.as_ptr());
        (*raw).PSSetConstantBuffers(0, 3, constant_buffers.as_ptr());
        bind_particle_stages(
            raw,
            material.vertex_shader.as_raw(),
            material.pixel_shader.as_raw(),
        );
        (*raw).IASetInputLayout(material.input_layout.as_raw());
        (*raw).OMSetBlendState(material.blend.as_raw(), &[0.0; 4], u32::MAX);
        (*raw).OMSetDepthStencilState(self.depth.as_raw(), 0);
        (*raw).RSSetState(material.rasterizer.as_raw());
        for (slot, sampler) in &material.samplers {
            (*raw).PSSetSamplers(*slot, 1, &sampler.as_raw());
        }
        for resource in material
            .pixel_reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 2)
        {
            let view = particle_texture_view(resource, material.texture_view.as_raw(), &shared)?;
            (*raw).PSSetShaderResources(resource.bind_point, 1, &view);
        }
        let count = uploaded.count;
        if uploaded.trail {
            (*raw).IASetPrimitiveTopology(
                winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            );
            (*raw).IASetVertexBuffers(0, 1, &uploaded.buffer.as_raw(), &uploaded.stride, &0);
            (*raw).IASetIndexBuffer(
                ptr::null_mut(),
                winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT,
                0,
            );
            (*raw).Draw(count, 0);
        } else {
            (*raw).IASetPrimitiveTopology(
                winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            );
            (*raw).IASetVertexBuffers(
                0,
                2,
                [self.quad.as_raw(), uploaded.buffer.as_raw()].as_ptr(),
                [8, uploaded.stride].as_ptr(),
                [0, 0].as_ptr(),
            );
            (*raw).IASetIndexBuffer(
                self.indices.as_raw(),
                winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT,
                0,
            );
            (*raw).DrawIndexedInstanced(6, count, 0, 0, 0);
        }
        for resource in material
            .pixel_reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 2)
        {
            (*raw).PSSetShaderResources(resource.bind_point, 1, &ptr::null_mut());
        }
        crate::d3d11_device::native_d3d11_check_void_upload_device(
            &self.upload.device,
            "particle authored draw",
        )?;
        Ok(Some((count, submission_sha256)))
    }
}

fn particle_instance_bytes(
    subsystem: ParticleGpuSubsystem<'_>,
    dimensions: [u32; 2],
) -> Result<[u8; 96]> {
    if dimensions.contains(&0) {
        return Err("PARTICLE_DRAW texture dimensions must be nonzero".into());
    }
    let texture = subsystem
        .texture()
        .ok_or("PARTICLE_DRAW authored atlas unavailable")?;
    let matrix = subsystem.draw_world_matrix_f32_bits()?;
    let mut bytes = [0; 96];
    for (index, bits) in matrix
        .into_iter()
        .chain([
            (f32::from_bits(0x3f00_0000) / dimensions[0] as f32).to_bits(),
            (f32::from_bits(0x3f00_0000) / dimensions[1] as f32).to_bits(),
            (texture.columns as f32).to_bits(),
            (texture.rows as f32).to_bits(),
            subsystem.local_time_bits(),
        ])
        .enumerate()
    {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    Ok(bytes)
}

unsafe fn bind_particle_stages(
    context: *mut ID3D11DeviceContext,
    vertex: *mut ID3D11VertexShader,
    pixel: *mut ID3D11PixelShader,
) {
    (*context).VSSetShader(vertex, ptr::null(), 0);
    (*context).PSSetShader(pixel, ptr::null(), 0);
    (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).SOSetTargets(0, ptr::null(), ptr::null());
}

fn particle_texture_view(
    resource: &crate::shader_compile::DxbcResource,
    authored_view: *mut ID3D11ShaderResourceView,
    shared: &[crate::NativeXmap12SharedResourceBinding],
) -> Result<*mut ID3D11ShaderResourceView> {
    if resource.bind_count != 1 || resource.resource_type != 2 {
        return Err(format!(
            "PARTICLE_DRAW texture binding unavailable: {}",
            resource.name
        )
        .into());
    }
    if resource.bind_point == 0 {
        if resource.dimension != 4 || authored_view.is_null() {
            return Err(format!(
                "PARTICLE_DRAW authored t0 requires Texture2D: {}",
                resource.name
            )
            .into());
        }
        return Ok(authored_view);
    }
    let mut matching = shared
        .iter()
        .filter(|binding| binding.shader_register == resource.bind_point);
    let binding = matching.next().ok_or_else(|| {
        format!(
            "PARTICLE_DRAW shared texture unavailable: {}",
            resource.name
        )
    })?;
    if matching.next().is_some()
        || binding.view.is_null()
        || binding.semantic != resource.name
        || binding.view_dimension != resource.dimension
    {
        return Err(format!("PARTICLE_DRAW shared texture mismatch: {}", resource.name).into());
    }
    Ok(binding.view)
}

fn validate_particle_resources(
    vertex_reflection: &crate::shader_compile::DxbcReflection,
    pixel_reflection: &crate::shader_compile::DxbcReflection,
    scene_byte_len: u32,
    authored_view: *mut ID3D11ShaderResourceView,
    shared: &[crate::NativeXmap12SharedResourceBinding],
) -> Result<()> {
    for (vertex, reflection) in [(true, vertex_reflection), (false, pixel_reflection)] {
        for resource in &reflection.resources {
            if resource.bind_count != 1 {
                return Err(format!(
                    "PARTICLE_DRAW resource array unavailable: {}",
                    resource.name
                )
                .into());
            }
            match resource.resource_type {
                0 => {
                    let limit = match resource.bind_point {
                        0 => scene_byte_len,
                        1 => 64,
                        2 => 8,
                        3 if vertex => 84,
                        _ => {
                            return Err(format!(
                                "PARTICLE_DRAW constant buffer unavailable: {}",
                                resource.name
                            )
                            .into())
                        }
                    };
                    let buffer = reflection
                        .cbuffers
                        .iter()
                        .find(|buffer| buffer.name == resource.name)
                        .ok_or("PARTICLE_DRAW constant buffer reflection unavailable")?;
                    if buffer
                        .vars
                        .iter()
                        .filter(|variable| variable.used)
                        .any(|variable| {
                            variable
                                .offset
                                .checked_add(variable.size)
                                .is_none_or(|end| end > limit)
                        })
                    {
                        return Err(format!(
                            "PARTICLE_DRAW constant buffer reads unavailable bytes: {}",
                            buffer.name
                        )
                        .into());
                    }
                }
                2 if !vertex => {
                    particle_texture_view(resource, authored_view, shared)?;
                }
                3 if !vertex => {}
                _ => {
                    return Err(format!(
                        "PARTICLE_DRAW resource binding unavailable: {}",
                        resource.name
                    )
                    .into())
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_input_layout_matches_native_instruction_written_declarations() {
        let fixture: serde_json::Value =
            serde_json::from_str(portable_engine_evidence::particle_runtime::INPUT_LAYOUT_CASES)
                .unwrap();
        for (trail, key) in [(false, "normal"), (true, "trail")] {
            let rows: Vec<[u32; 8]> = serde_json::from_value(fixture[key].clone()).unwrap();
            let elements = particle_input_elements(trail);
            assert_eq!(elements.len(), rows.len());
            for (element, row) in elements.iter().zip(rows) {
                assert_eq!(element.input_slot, row[0]);
                assert_eq!(element.aligned_byte_offset, row[1]);
                let format = match (row[2], row[3]) {
                    (0, 2) => 16,
                    (0, 3) => 6,
                    (0, 4) => 2,
                    (5, 1) => 62,
                    (6, 1) => 87,
                    other => panic!("unproved native format {other:?}"),
                };
                assert_eq!(element.format, format);
                assert_eq!(
                    element.semantic_name,
                    ["POSITION", "COLOR", "TEXCOORD"][row[4] as usize]
                );
                assert_eq!(element.semantic_index, row[5]);
                assert_eq!(element.input_slot_class, u32::from(row[6] != 0));
                assert_eq!(element.instance_data_step_rate, u32::from(row[6] != 0));
            }
        }
    }

    unsafe fn particle_test_target(
        device: &D3dPtr<ID3D11Device>,
        array_size: u32,
        cube: bool,
    ) -> (
        D3dPtr<ID3D11Texture2D>,
        D3dPtr<ID3D11ShaderResourceView>,
        D3dPtr<ID3D11RenderTargetView>,
    ) {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: 8,
            Height: 8,
            MipLevels: 1,
            ArraySize: array_size,
            Format: winapi::shared::dxgiformat::DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET,
            CPUAccessFlags: 0,
            MiscFlags: if cube {
                D3D11_RESOURCE_MISC_TEXTURECUBE
            } else {
                0
            },
        };
        let mut texture = ptr::null_mut();
        let mut view = ptr::null_mut();
        let mut target = ptr::null_mut();
        native_d3d11_hr(
            (*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut texture),
            "particle test texture",
        )
        .unwrap();
        let texture = D3dPtr::from_raw(texture, "particle test texture").unwrap();
        native_d3d11_hr(
            (*device.as_raw()).CreateShaderResourceView(
                texture.as_raw().cast(),
                ptr::null(),
                &mut view,
            ),
            "particle test SRV",
        )
        .unwrap();
        native_d3d11_hr(
            (*device.as_raw()).CreateRenderTargetView(
                texture.as_raw().cast(),
                ptr::null(),
                &mut target,
            ),
            "particle test RTV",
        )
        .unwrap();
        (
            texture,
            D3dPtr::from_raw(view, "particle test SRV").unwrap(),
            D3dPtr::from_raw(target, "particle test RTV").unwrap(),
        )
    }

    #[test]
    fn particle_normal_and_trail_cb3_use_material_half_texels() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        use crate::particle_runtime::{ParticleRuntime, ViewerParticleSeed};
        for trail in ["no", "yes"] {
            let fixture = Fixture::new(REGISTRATION, &format!("particle={{name=effect subsystem={{name=s max_amount=1 trail={trail} texture={{file=gfx/image.dds x=3 y=7}}}}}}"));
            let mut definitions = fixture.definitions();
            let mut runtime = ParticleRuntime::new(definitions.content_lease(), 1).unwrap();
            let identity = [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]
            .map(f32::to_bits);
            let handle = runtime
                .spawn(
                    1,
                    definitions.resolve("paired").unwrap(),
                    ViewerParticleSeed {
                        counter: 0,
                        seed: 1,
                    },
                    identity,
                    1.,
                )
                .unwrap();
            let subsystem = runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap();
            let cb = particle_instance_bytes(subsystem, [2, 4]).unwrap();
            let words: Vec<_> = cb[64..84]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert_eq!(
                words,
                [
                    0.25f32.to_bits(),
                    0.125f32.to_bits(),
                    3f32.to_bits(),
                    7f32.to_bits(),
                    (-0.0f32).to_bits()
                ]
            );
            assert!(particle_instance_bytes(subsystem, [0, 4]).is_err());
        }
    }

    #[test]
    fn particle_global_lifecycle_upload_material_and_native_draw_produce_pixels() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        use crate::xmap09_light_lifecycle::{ViewerLightLifecycle, Xmap09PlacedEntity};
        use winapi::um::d3dcommon::{D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0};
        const HEADER: &[u8] = br#"#define PDX_POSITION SV_POSITION
#define PDX_COLOR SV_TARGET
struct STextureSampler2D { Texture2D _Texture; SamplerState _Sampler; };
STextureSampler2D CreateSampler2D(Texture2D t, SamplerState s) { STextureSampler2D r={t,s}; return r; }
#define sampler2D STextureSampler2D
#define tex2D(samp,uv) samp._Texture.Sample(samp._Sampler, uv)
"#;
        const SHADER: &str = r#"
VertexStruct Input { float2 uv : TEXCOORD0; float4 position_size : TEXCOORD1; float3 rest : TEXCOORD2; uint atlas : TEXCOORD3; float4 color : COLOR0; };
VertexStruct Output { float4 position : PDX_POSITION; float2 uv : TEXCOORD0; float4 color : COLOR0; };
ConstantBuffer(3, 96) { float4x4 World; float2 Reciprocal; float2 Atlas; float Time; };
VertexShader = { MainCode VS [[
    Output main(Input v) { Output o; o.position=float4((v.uv*2-1)*v.position_size.w+v.position_size.xy, 0.5, 1); o.uv=v.uv; o.color=v.color*float4(Reciprocal.x*4,Reciprocal.y*8,1,1); return o; }
]] }
PixelShader = {
    Samplers = { Diffuse = { Index=0 MagFilter="Point" MinFilter="Point" MipFilter="Point" AddressU="Clamp" AddressV="Clamp" } }
    MainCode PS [[ float4 main(Output v) : PDX_COLOR { return tex2D(Diffuse,v.uv)*v.color; } ]]
}
BlendState BlendState { BlendEnable=no }
DepthStencilState DepthStencilState { DepthEnable=yes DepthWriteMask="ALL" DepthFunc="NEVER" }
RasterizerState RasterizerState { CullMode="NONE" }
Effect fixture { VertexShader="VS" PixelShader="PS" }
"#;
        let mut dds = vec![0u8; 128 + 2 * 4 * 4];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124u32),
            (8, 0x2100f),
            (12, 4),
            (16, 2),
            (20, 8),
            (28, 1),
            (76, 32),
            (80, 0x41),
            (88, 32),
            (92, 0xff0000),
            (96, 0xff00),
            (100, 0xff),
            (104, 0xff000000),
            (108, 0x1000),
        ] {
            dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        dds[128..].copy_from_slice(&[192, 128, 64, 255].repeat(8));
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=1 duration=0 emission=2 life=2 size=0.5 color={r=255 g=255 b=255 a=255} texture={file=gfx/fixture.dds shader=fixture x=1 y=1}}}",
            &[("gfx/fixture.dds",&dds),("gfx/FX/particle.shader",SHADER.as_bytes()),("gfx/FX/defines_hlsl_dx11.fxh",HEADER),
            ("gfx/entity.asset",b"entity={name=entity default_state=idle state={name=idle event={time=0 particle=paired light=flash}}} light={name=flash color={r=1 g=1 b=1} intensity=1 radius=8 falloff=1}")]);
        let paths = [
            "gfx/particles.gfx",
            "gfx/particles.asset",
            "gfx/entity.asset",
            "gfx/fixture.dds",
            "gfx/FX/particle.shader",
            "gfx/FX/defines_hlsl_dx11.fxh",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect();
        let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(
            &content.resolver,
            &paths,
            "particle-global-draw",
        );
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]
        .map(f32::to_bits);
        let placed = [Xmap09PlacedEntity {
            id: "draw-entity".into(),
            entity_token: "entity".into(),
            state: None,
            animate: true,
            world_matrix_f32_bits: identity,
        }];
        let viewer = crate::ViewerOwnedMapState {
            sky_enabled: true,
            viewport_width: 8,
            viewport_height: 8,
            camera: None,
            observer_country_id: None,
            map_mode_id: "political".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: vec![],
            input_revision: 0,
            animation_time_f32_bits: 0.5f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.5f32.to_bits(),
        };
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(rows.authored_particles.len(), 1);
        let paired = rows.authored_particles[0].particle_phase;
        unsafe {
            let mut device = ptr::null_mut();
            let mut context = ptr::null_mut();
            native_d3d11_hr(
                D3D11CreateDevice(
                    ptr::null_mut(),
                    D3D_DRIVER_TYPE_WARP,
                    ptr::null_mut(),
                    0,
                    &D3D_FEATURE_LEVEL_11_0,
                    1,
                    D3D11_SDK_VERSION,
                    &mut device,
                    ptr::null_mut(),
                    &mut context,
                ),
                "particle global WARP",
            )
            .unwrap();
            let device = D3dPtr::from_raw(device, "particle global device").unwrap();
            let context = D3dPtr::from_raw(context, "particle global context").unwrap();
            let target = crate::NativeProductionMainSceneTarget::create_on_device(
                device.as_raw(),
                crate::ProductionMainSceneTargetDesc {
                    color_attachment_ordinal: 0,
                    color_width: 8,
                    color_height: 8,
                    color_format: 28,
                    color_samples: 1,
                    color_quality: 0,
                    depth_attachment_ordinal: 1,
                    depth_width: 8,
                    depth_height: 8,
                    depth_format: 45,
                    depth_samples: 1,
                    depth_quality: 0,
                },
                "a".repeat(64),
                1,
            )
            .unwrap();
            struct Scene(D3dPtr<ID3D11Buffer>);
            unsafe impl crate::NativeXmap16SceneConstantBufferView for Scene {
                fn symbol(&self) -> &str {
                    "Scene"
                }
                fn owner_packet(&self) -> &str {
                    "XMAP10"
                }
                fn shader_register(&self) -> u32 {
                    0
                }
                fn byte_len(&self) -> u32 {
                    16
                }
                fn payload_sha256(&self) -> &str {
                    ""
                }
                fn descriptor_sha256(&self) -> &str {
                    ""
                }
                fn composition_receipt_sha256(&self) -> &str {
                    ""
                }
                fn reflected_layout_sha256(&self) -> &str {
                    ""
                }
                fn application_elapsed_seconds(&self) -> f32 {
                    0.5
                }
                fn raw_buffer(&self) -> *mut ID3D11Buffer {
                    self.0.as_raw()
                }
            }
            let scene = Scene(
                crate::d3d11_device::native_d3d11_create_exact_constant_buffer(
                    &device,
                    &[0; 16],
                    "particle test scene",
                )
                .unwrap(),
            );
            let camera = crate::ProductionMapCamera::new(
                [
                    [1., 0., 0., 0.],
                    [0., 1., 0., 0.],
                    [0., 0., 1., 0.],
                    [0., 0., 0., 1.],
                ],
                [0., 0., 1.],
            )
            .unwrap();
            let sort = ParticleSortCamera {
                position_f32_bits: camera.position.map(f32::to_bits),
                forward_f32_bits: [0, 0, (-1f32).to_bits()],
            };
            let bindings = ParticleSceneBindings {
                target: &target,
                scene: &scene,
                camera,
                shared: &|_| panic!("fixture needs only authored t0"),
            };
            let mut renderer =
                ParticleNativeDraw::create_on_device(device.as_raw(), &resolver, vec![]).unwrap();
            (*context.as_raw()).ClearRenderTargetView(target.color_rtv(), &[0.; 4]);
            (*context.as_raw()).ClearDepthStencilView(
                target.depth_dsv(),
                D3D11_CLEAR_DEPTH,
                1.0,
                0,
            );
            let mut counts = Vec::new();
            lifecycle
                .visit_particles_for_draw(0.5, sort, &mut |_, capacity, view| {
                    assert_eq!(view.presentation_id, paired.presentation_id);
                    assert_eq!(view.world_matrix_f32_bits(), paired.world_matrix_f32_bits);
                    counts.push(renderer.draw_instance(
                        context.as_raw(),
                        &resolver,
                        &admission,
                        capacity,
                        view,
                        0.5f32.to_bits() as u64,
                        sort,
                        &bindings,
                    )?);
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                counts
                    .iter()
                    .map(ParticleDrawReceipt::counts)
                    .collect::<Vec<_>>(),
                [ParticleDrawCounts {
                    packed_subsystems: 1,
                    draw_calls: 1,
                    normal_instances: 1,
                    trail_vertices: 0
                }]
            );
            let mut desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
            (*target.color_texture()).GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            let mut staging = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging),
                "particle readback",
            )
            .unwrap();
            let staging = D3dPtr::from_raw(staging, "particle readback").unwrap();
            (*context.as_raw())
                .CopyResource(staging.as_raw().cast(), target.color_texture().cast());
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            native_d3d11_hr(
                (*context.as_raw()).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped),
                "particle readback map",
            )
            .unwrap();
            let mut pixels = Vec::new();
            for y in 0..8 {
                pixels.extend_from_slice(std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    32,
                ));
            }
            (*context.as_raw()).Unmap(staging.as_raw().cast(), 0);
            for y in 0..8 {
                for x in 0..8 {
                    let expected = if (2..6).contains(&x) && (2..6).contains(&y) {
                        [64u8, 128, 192, 255]
                    } else {
                        [0; 4]
                    };
                    assert_eq!(&pixels[(y * 8 + x) * 4..(y * 8 + x + 1) * 4], &expected);
                }
            }
            assert_eq!(renderer.materials.len(), 1);
            let mut depth = ptr::null_mut();
            let mut stencil_ref = 0;
            (*context.as_raw()).OMGetDepthStencilState(&mut depth, &mut stencil_ref);
            let depth = D3dPtr::from_raw(depth, "bound particle depth").unwrap();
            let mut depth_desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
            (*depth.as_raw()).GetDesc(&mut depth_desc);
            assert_eq!(depth.as_raw(), renderer.depth.as_raw());
            assert_eq!(depth_desc.DepthEnable, 1);
            assert_eq!(depth_desc.DepthWriteMask, D3D11_DEPTH_WRITE_MASK_ZERO);
            assert_eq!(depth_desc.DepthFunc, D3D11_COMPARISON_LESS_EQUAL);
            assert_eq!(depth_desc.StencilEnable, 0);
            (*context.as_raw()).ClearRenderTargetView(target.color_rtv(), &[0.; 4]);
            (*context.as_raw()).ClearDepthStencilView(
                target.depth_dsv(),
                D3D11_CLEAR_DEPTH,
                0.25,
                0,
            );
            lifecycle
                .visit_particles_for_draw(0.5, sort, &mut |_, capacity, view| {
                    renderer.draw_instance(
                        context.as_raw(),
                        &resolver,
                        &admission,
                        capacity,
                        view,
                        0.5f32.to_bits() as u64,
                        sort,
                        &bindings,
                    )?;
                    Ok(())
                })
                .unwrap();
            (*context.as_raw())
                .CopyResource(staging.as_raw().cast(), target.color_texture().cast());
            native_d3d11_hr(
                (*context.as_raw()).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped),
                "particle occlusion map",
            )
            .unwrap();
            let mut occluded = Vec::new();
            for y in 0..8 {
                occluded.extend_from_slice(std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    32,
                ));
            }
            (*context.as_raw()).Unmap(staging.as_raw().cast(), 0);
            assert_eq!(occluded, vec![0; 256]);
            let mut definitions =
                crate::particle_authored::ParticleDefinitions::load(&resolver).unwrap();
            let capacity = definitions.draw_capacity().unwrap();
            let presentation = u64::from(0.5f32.to_bits());
            let mut submitted = Vec::new();
            let mut alpha = Vec::new();
            for opacity in [1.0, 0.5, 0.5] {
                let mut runtime =
                    crate::particle_runtime::ParticleRuntime::new(definitions.content_lease(), 1)
                        .unwrap();
                let handle = runtime
                    .spawn(
                        1,
                        definitions.resolve("paired").unwrap(),
                        crate::particle_runtime::ViewerParticleSeed {
                            counter: 0,
                            seed: 1,
                        },
                        identity,
                        1.,
                    )
                    .unwrap();
                runtime.set_opacity(handle, opacity).unwrap();
                runtime.advance(presentation, 0.5, &[]).unwrap();
                runtime.sort_for_draw(handle, presentation, sort).unwrap();
                let view = runtime.gpu_view(handle).unwrap();
                assert_eq!(view.presentation_id, Some(presentation));
                assert_eq!(view.world_matrix_f32_bits(), identity);
                (*context.as_raw()).ClearRenderTargetView(target.color_rtv(), &[0.; 4]);
                (*context.as_raw()).ClearDepthStencilView(
                    target.depth_dsv(),
                    D3D11_CLEAR_DEPTH,
                    1.,
                    0,
                );
                submitted.push(
                    renderer
                        .draw_instance(
                            context.as_raw(),
                            &resolver,
                            &admission,
                            &capacity,
                            view,
                            presentation,
                            sort,
                            &bindings,
                        )
                        .unwrap(),
                );
                (*context.as_raw())
                    .CopyResource(staging.as_raw().cast(), target.color_texture().cast());
                native_d3d11_hr(
                    (*context.as_raw()).Map(
                        staging.as_raw().cast(),
                        0,
                        D3D11_MAP_READ,
                        0,
                        &mut mapped,
                    ),
                    "particle altered-state readback",
                )
                .unwrap();
                alpha.push(
                    *((mapped.pData as *const u8).add(3 * mapped.RowPitch as usize + 3 * 4 + 3)),
                );
                (*context.as_raw()).Unmap(staging.as_raw().cast(), 0);
            }
            assert_eq!(submitted[0].counts(), submitted[1].counts());
            assert_eq!(submitted[0].counts().draw_calls, 1);
            assert_ne!(
                submitted[0].submission_sha256(),
                submitted[1].submission_sha256()
            );
            assert_eq!(submitted[1], submitted[2]);
            assert_eq!(alpha, [255, 128, 128]);
            (*context.as_raw()).ClearState();
        }
    }

    #[test]
    fn particle_validation_and_native_binding_reject_non_2d_t0_and_clear_hostile_stages() {
        use crate::shader_compile::{
            d3d_compile_stage, parse_dxbc_reflection, ShaderStage, ENGINE_COMPILE_FLAGS,
        };
        use winapi::um::d3dcommon::{D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0};
        unsafe {
            let mut device = ptr::null_mut();
            let mut context = ptr::null_mut();
            native_d3d11_hr(
                D3D11CreateDevice(
                    ptr::null_mut(),
                    D3D_DRIVER_TYPE_WARP,
                    ptr::null_mut(),
                    0,
                    &D3D_FEATURE_LEVEL_11_0,
                    1,
                    D3D11_SDK_VERSION,
                    &mut device,
                    ptr::null_mut(),
                    &mut context,
                ),
                "particle WARP",
            )
            .unwrap();
            let device = D3dPtr::from_raw(device, "particle device").unwrap();
            let context_owner = D3dPtr::from_raw(context, "particle context").unwrap();
            let vertex = d3d_compile_stage("float4 main(uint id : SV_VertexID) : SV_POSITION { return float4(id == 2 ? 3 : -1, id == 1 ? 3 : -1, 0, 1); }", ShaderStage::Vertex, ENGINE_COMPILE_FLAGS).unwrap();
            let pixel = d3d_compile_stage("Texture2D image : register(t0); float4 main() : SV_Target { return image.Load(int3(0,0,0)); }", ShaderStage::Pixel, ENGINE_COMPILE_FLAGS).unwrap();
            let vertex_reflection = parse_dxbc_reflection(&vertex).unwrap();
            let pixel_reflection = parse_dxbc_reflection(&pixel).unwrap();
            let mut vs = ptr::null_mut();
            let mut ps = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateVertexShader(
                    vertex.as_ptr().cast(),
                    vertex.len(),
                    ptr::null_mut(),
                    &mut vs,
                ),
                "particle test VS",
            )
            .unwrap();
            native_d3d11_hr(
                (*device.as_raw()).CreatePixelShader(
                    pixel.as_ptr().cast(),
                    pixel.len(),
                    ptr::null_mut(),
                    &mut ps,
                ),
                "particle test PS",
            )
            .unwrap();
            let vs = D3dPtr::from_raw(vs, "particle test VS").unwrap();
            let ps = D3dPtr::from_raw(ps, "particle test PS").unwrap();
            let authored = particle_test_target(&device, 1, false);
            let alternate = particle_test_target(&device, 1, false);
            let target = particle_test_target(&device, 1, false);
            (*context).ClearRenderTargetView(authored.2.as_raw(), &[0.25, 0.5, 0.75, 1.0]);
            (*context).ClearRenderTargetView(alternate.2.as_raw(), &[1.0, 0.0, 0.0, 1.0]);
            (*context).ClearRenderTargetView(target.2.as_raw(), &[0.0; 4]);
            let shared_binding = |view, dimension, slot| crate::NativeXmap12SharedResourceBinding {
                semantic: "image".into(),
                shader_register: slot,
                payload_sha256: "a".repeat(64),
                descriptor_sha256: "b".repeat(64),
                view_dimension: dimension,
                view,
            };
            for (kind, location, array_size, cube) in [
                ("TextureCube", "float3(1,0,0)", 6, true),
                ("Texture2DArray", "float3(0.5,0.5,0)", 2, false),
            ] {
                let texture = particle_test_target(&device, array_size, cube);
                for slot in [0, 6] {
                    let code = d3d_compile_stage(&format!("{kind} image : register(t{slot}); SamplerState samp : register(s0); float4 main() : SV_Target {{ return image.SampleLevel(samp, {location}, 0); }}"), ShaderStage::Pixel, ENGINE_COMPILE_FLAGS).unwrap();
                    let reflection = parse_dxbc_reflection(&code).unwrap();
                    let resource = reflection
                        .resources
                        .iter()
                        .find(|r| r.resource_type == 2)
                        .unwrap();
                    let shared = [shared_binding(texture.1.as_raw(), resource.dimension, slot)];
                    let admission = validate_particle_resources(
                        &vertex_reflection,
                        &reflection,
                        416,
                        authored.1.as_raw(),
                        &shared,
                    );
                    let selected = particle_texture_view(resource, authored.1.as_raw(), &shared);
                    if slot == 0 {
                        assert!(admission
                            .unwrap_err()
                            .to_string()
                            .contains("authored t0 requires Texture2D"));
                        assert!(selected.is_err());
                    } else {
                        admission.unwrap();
                        assert_eq!(selected.unwrap(), texture.1.as_raw());
                    }
                }
            }
            let shared = [shared_binding(alternate.1.as_raw(), 4, 0)];
            validate_particle_resources(
                &vertex_reflection,
                &pixel_reflection,
                416,
                authored.1.as_raw(),
                &shared,
            )
            .unwrap();
            let resource = pixel_reflection
                .resources
                .iter()
                .find(|r| r.resource_type == 2)
                .unwrap();
            let selected = particle_texture_view(resource, authored.1.as_raw(), &shared).unwrap();
            assert_eq!(selected, authored.1.as_raw());
            let gs = crate::d3d11_device::native_d3d11_compile_shader("struct V { float4 p : SV_POSITION; }; [maxvertexcount(3)] void main(triangle V input[3], inout TriangleStream<V> stream) {}", "main", "gs_5_0").unwrap();
            let mut geometry = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateGeometryShader(
                    (*gs.as_raw()).GetBufferPointer(),
                    (*gs.as_raw()).GetBufferSize(),
                    ptr::null_mut(),
                    &mut geometry,
                ),
                "particle hostile GS",
            )
            .unwrap();
            let geometry = D3dPtr::from_raw(geometry, "particle hostile GS").unwrap();
            let tess = "struct V { float4 p : SV_POSITION; }; struct C { float edges[3] : SV_TessFactor; float inside : SV_InsideTessFactor; }; C patch(InputPatch<V, 3> input) { C c; c.edges[0]=c.edges[1]=c.edges[2]=c.inside=1; return c; } [domain(\"tri\")] [partitioning(\"integer\")] [outputtopology(\"triangle_cw\")] [outputcontrolpoints(3)] [patchconstantfunc(\"patch\")] V hull(InputPatch<V,3> input, uint id : SV_OutputControlPointID) { return input[id]; } [domain(\"tri\")] V domain(C c, float3 uv : SV_DomainLocation, const OutputPatch<V,3> input) { return input[0]; }";
            let hs =
                crate::d3d11_device::native_d3d11_compile_shader(tess, "hull", "hs_5_0").unwrap();
            let ds =
                crate::d3d11_device::native_d3d11_compile_shader(tess, "domain", "ds_5_0").unwrap();
            let mut hull = ptr::null_mut();
            let mut domain = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateHullShader(
                    (*hs.as_raw()).GetBufferPointer(),
                    (*hs.as_raw()).GetBufferSize(),
                    ptr::null_mut(),
                    &mut hull,
                ),
                "particle hostile HS",
            )
            .unwrap();
            native_d3d11_hr(
                (*device.as_raw()).CreateDomainShader(
                    (*ds.as_raw()).GetBufferPointer(),
                    (*ds.as_raw()).GetBufferSize(),
                    ptr::null_mut(),
                    &mut domain,
                ),
                "particle hostile DS",
            )
            .unwrap();
            let hull = D3dPtr::from_raw(hull, "particle hostile HS").unwrap();
            let domain = D3dPtr::from_raw(domain, "particle hostile DS").unwrap();
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: 256,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_STREAM_OUTPUT,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut output = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut output),
                "particle stream output",
            )
            .unwrap();
            let output = D3dPtr::from_raw(output, "particle stream output").unwrap();
            (*context).GSSetShader(geometry.as_raw(), ptr::null(), 0);
            (*context).HSSetShader(hull.as_raw(), ptr::null(), 0);
            (*context).DSSetShader(domain.as_raw(), ptr::null(), 0);
            (*context).SOSetTargets(1, &output.as_raw(), &0);

            bind_particle_stages(context, vs.as_raw(), ps.as_raw());
            let mut bound_gs = ptr::null_mut();
            let mut bound_hs = ptr::null_mut();
            let mut bound_ds = ptr::null_mut();
            let mut bound_so = ptr::null_mut();
            (*context).GSGetShader(&mut bound_gs, ptr::null_mut(), ptr::null_mut());
            (*context).HSGetShader(&mut bound_hs, ptr::null_mut(), ptr::null_mut());
            (*context).DSGetShader(&mut bound_ds, ptr::null_mut(), ptr::null_mut());
            (*context).SOGetTargets(1, &mut bound_so);
            assert!(
                bound_gs.is_null()
                    && bound_hs.is_null()
                    && bound_ds.is_null()
                    && bound_so.is_null()
            );
            (*context).RSSetViewports(
                1,
                &D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: 8.0,
                    Height: 8.0,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                },
            );
            (*context).OMSetRenderTargets(1, &target.2.as_raw(), ptr::null_mut());
            (*context).PSSetShaderResources(resource.bind_point, 1, &selected);
            (*context).IASetPrimitiveTopology(
                winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            );
            (*context).Draw(3, 0);
            (*context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
            (*context).PSSetShaderResources(0, 1, &ptr::null_mut());
            let mut desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
            (*target.0.as_raw()).GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            let mut staging = ptr::null_mut();
            native_d3d11_hr(
                (*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging),
                "particle staging",
            )
            .unwrap();
            let staging = D3dPtr::from_raw(staging, "particle staging").unwrap();
            (*context).CopyResource(staging.as_raw().cast(), target.0.as_raw().cast());
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            native_d3d11_hr(
                (*context).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped),
                "particle readback",
            )
            .unwrap();
            let rows: Vec<Vec<u8>> = (0..8)
                .map(|row| {
                    std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                        32,
                    )
                    .to_vec()
                })
                .collect();
            (*context).Unmap(staging.as_raw().cast(), 0);
            assert!(
                rows.iter().all(|row| row
                    .chunks_exact(4)
                    .all(|pixel| pixel == [64, 128, 191, 255])),
                "{rows:?}"
            );
            drop(context_owner);
        }
    }
}
