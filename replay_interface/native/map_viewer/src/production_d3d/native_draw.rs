
use std::collections::BTreeMap;
use std::ffi::CString;
use std::mem;
use std::ptr;

use crate::GameFileResolver;
use replay_core::{
    AdditionalShaderStage, AddressMode, BlendFactor, BlendOperation, BufferSource, Comparison,
    ConstantBufferBinding, CullMode, DrawCommand, FillMode, Filter, GeometryBinding, IndexFormat,
    PipelineDesc, PreparedRenderTarget, PrimitiveTopology, RasterizerDesc, RenderOutputState,
    RenderTargetBlendDesc, SamplerDesc, ShaderProgram, ShaderResourceViewDesc, ShaderStage,
    StencilFaceDesc, StencilOperation, TextureBinding, TextureResourceDesc,
    TextureResourceDimension, TextureSubresourcePayload,
};
use snapshot::sha256::Sha256;
use winapi::shared::dxgiformat::{DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32_UINT};
use winapi::shared::windef::RECT;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;

use crate::d3d11_device::D3dPtr as ComPtr;
use crate::Result;

#[derive(Clone, Copy)]
pub(super) struct RawD3dPtr<T>(*mut T);

impl<T> RawD3dPtr<T> {
    fn as_raw(&self) -> *mut T {
        self.0
    }
}

pub(super) struct Gfx {
    pub device: RawD3dPtr<ID3D11Device>,
    pub context: RawD3dPtr<ID3D11DeviceContext>,
}

impl Gfx {
    pub(super) fn borrowed(
        device: *mut ID3D11Device,
        context: *mut ID3D11DeviceContext,
    ) -> Result<Self> {
        if device.is_null() || context.is_null() {
            return Err("portable D3D executor received null app device/context".into());
        }
        Ok(Self {
            device: RawD3dPtr(device),
            context: RawD3dPtr(context),
        })
    }
}

fn hr_ok(value: i32, role: &str) -> Result<()> {
    if value < 0 {
        Err(format!("{role} failed with HRESULT 0x{:08x}", value as u32).into())
    } else {
        Ok(())
    }
}

pub struct SubmittedMainSceneTarget {
    pub color_identity: PreparedRenderTarget,
    pub depth_identity: Option<PreparedRenderTarget>,
    pub color_rtv: *mut ID3D11RenderTargetView,
    pub depth_dsv: *mut ID3D11DepthStencilView,
    pub color_width: u32,
    pub color_height: u32,
    pub color_format: u32,
    pub color_samples: u32,
    pub color_quality: u32,
    pub depth_width: u32,
    pub depth_height: u32,
    pub depth_format: u32,
    pub depth_samples: u32,
    pub depth_quality: u32,
}

pub struct SubmittedColorTarget {
    pub identity: PreparedRenderTarget,
    pub rtv: *mut ID3D11RenderTargetView,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub samples: u32,
    pub quality: u32,
}

pub(super) struct PreparedTargetTexture {
    pub target: PreparedRenderTarget,
    pub producer: replay_core::RenderExecutionRef,
    pub resource: *mut ID3D11Resource,
}

pub(super) struct AuthenticatedTextureGpu {
    pub snapshot_sha256: String,
    resource: TextureResourceGpu,
}

#[allow(clippy::too_many_arguments)]
unsafe fn execute_draw(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    scene_constant_buffers: &[ConstantBufferBinding],
    shader: &ShaderProgram,
    textures: &[TextureBinding],
    pipeline: &PipelineDesc,
    geometry: &GeometryBinding,
    draw_constant_buffers: &[ConstantBufferBinding],
    particle_records: Option<&[u8]>,
    extra_texture_views: Vec<TextureViewGpu>,
    output: &RenderOutputState,
    target: &SubmittedMainSceneTarget,
    additional_colors: &[SubmittedColorTarget],
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    resolver.read_admitted_used_content(&shader.source)?;
    bind_output(gfx, output, target, additional_colors)?;

    execute_bound_draw(gfx, resolver, scene_constant_buffers, shader, textures,
        pipeline, geometry, draw_constant_buffers, particle_records, extra_texture_views,
        stream_output)
}

#[allow(clippy::too_many_arguments)]
unsafe fn execute_bound_draw(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    scene_constant_buffers: &[ConstantBufferBinding],
    shader: &ShaderProgram,
    textures: &[TextureBinding],
    pipeline: &PipelineDesc,
    geometry: &GeometryBinding,
    draw_constant_buffers: &[ConstantBufferBinding],
    particle_records: Option<&[u8]>,
    extra_texture_views: Vec<TextureViewGpu>,
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    let vertex_shader = create_vertex_shader(gfx, &shader.compiled_vertex_shader.bytecode)?;
    let pixel_shader = create_pixel_shader(gfx, &shader.compiled_pixel_shader.bytecode)?;
    let geometry_shader = create_geometry_shader(gfx, shader.geometry_shader.as_ref())?;
    let hull_shader = create_hull_shader(gfx, shader.hull_shader.as_ref())?;
    let domain_shader = create_domain_shader(gfx, shader.domain_shader.as_ref())?;
    let compute_shader = create_compute_shader(gfx, shader.compute_shader.as_ref())?;
    let input_layout = create_input_layout(gfx, pipeline, &shader.compiled_vertex_shader.bytecode)?;
    let vertex_buffers = create_vertex_buffers(gfx, resolver, geometry, particle_records)?;
    let index_buffer = create_index_buffer(gfx, resolver, geometry, particle_records)?;
    let scene_cbs = create_constant_buffers(gfx, scene_constant_buffers)?;
    let draw_cbs = create_constant_buffers(gfx, draw_constant_buffers)?;
    let mut texture_views = create_texture_views(gfx, resolver, textures, particle_records)?;
    texture_views.extend(extra_texture_views);
    let samplers = create_samplers(gfx, &pipeline.samplers)?;
    let rasterizer = create_rasterizer(gfx, &pipeline.rasterizer)?;
    let blend = create_blend(gfx, pipeline)?;
    let depth = create_depth(gfx, pipeline)?;

    let context = gfx.context.as_raw();
    clear_shader_bindings(context);
    (*context).VSSetShader(vertex_shader.as_raw(), ptr::null(), 0);
    (*context).PSSetShader(pixel_shader.as_raw(), ptr::null(), 0);
    (*context).GSSetShader(
        geometry_shader
            .as_ref()
            .map_or(ptr::null_mut(), ComPtr::as_raw),
        ptr::null(),
        0,
    );
    (*context).HSSetShader(
        hull_shader.as_ref().map_or(ptr::null_mut(), ComPtr::as_raw),
        ptr::null(),
        0,
    );
    (*context).DSSetShader(
        domain_shader
            .as_ref()
            .map_or(ptr::null_mut(), ComPtr::as_raw),
        ptr::null(),
        0,
    );
    (*context).CSSetShader(
        compute_shader
            .as_ref()
            .map_or(ptr::null_mut(), ComPtr::as_raw),
        ptr::null(),
        0,
    );
    (*context).IASetInputLayout(input_layout.as_raw());
    (*context).IASetPrimitiveTopology(topology(pipeline.topology));
    for buffer in &vertex_buffers {
        let raw = [buffer.buffer.as_raw()];
        let stride = [buffer.stride];
        let offset = [buffer.offset];
        (*context).IASetVertexBuffers(
            buffer.slot,
            1,
            raw.as_ptr(),
            stride.as_ptr(),
            offset.as_ptr(),
        );
    }
    if let Some(index) = &index_buffer {
        (*context).IASetIndexBuffer(index.buffer.as_raw(), index.format, index.offset);
    } else {
        (*context).IASetIndexBuffer(ptr::null_mut(), 0, 0);
    }
    stream_output.bind(gfx, context, &geometry.stream_output_targets)?;
    bind_constant_buffers(context, &scene_cbs);
    bind_constant_buffers(context, &draw_cbs);
    bind_texture_views(context, &texture_views);
    bind_samplers(context, &samplers);
    (*context).RSSetState(rasterizer.as_raw());
    let blend_factor = pipeline
        .blend
        .blend_factor_rgba_f32_bits
        .map(f32::from_bits);
    (*context).OMSetBlendState(
        blend.as_raw(),
        &blend_factor,
        pipeline.blend.sample_mask_u32,
    );
    (*context).OMSetDepthStencilState(depth.as_raw(), pipeline.depth.stencil_reference_u32);

    match geometry.draw {
        DrawCommand::Draw {
            vertex_count,
            start_vertex,
        } => (*context).Draw(vertex_count, start_vertex),
        DrawCommand::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        } => (*context).DrawIndexed(index_count, start_index, base_vertex),
        DrawCommand::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        } => (*context).DrawInstanced(
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        ),
        DrawCommand::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        } => (*context).DrawIndexedInstanced(
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        ),
    }
    (*context).SOSetTargets(0, ptr::null(), ptr::null());
    clear_shader_bindings(context);
    Ok(())
}

/// Private runtime target binding. Recorded wrappers still go through
/// bind_output and all their prepared-target identity checks.
pub(super) unsafe fn execute_ui_draw(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    input: &super::ui::NativeUiDrawInputs<'_>,
    rtv: *mut ID3D11RenderTargetView,
) -> Result<()> {
    if rtv.is_null() { return Err("native UI target view is null".into()); }
    let context=gfx.context.as_raw();
    let v=input.viewport;
    let viewport=D3D11_VIEWPORT {
        TopLeftX:f32::from_bits(v.top_left_x_f32_bits),
        TopLeftY:f32::from_bits(v.top_left_y_f32_bits),
        Width:f32::from_bits(v.width_f32_bits), Height:f32::from_bits(v.height_f32_bits),
        MinDepth:f32::from_bits(v.min_depth_f32_bits), MaxDepth:f32::from_bits(v.max_depth_f32_bits),
    };
    (*context).OMSetRenderTargets(1,&rtv,ptr::null_mut());
    (*context).RSSetViewports(1,&viewport);
    if let Some(s)=input.scissor {
        let rect=RECT {left:s.left_i32,top:s.top_i32,right:s.right_i32,bottom:s.bottom_i32};
        (*context).RSSetScissorRects(1,&rect);
    } else { (*context).RSSetScissorRects(0,ptr::null()); }
    let result=execute_bound_draw(gfx,resolver,input.scene_constants,input.shader,input.textures,
        input.pipeline,input.geometry,input.draw_constants,None,Vec::new(),&mut StreamOutputBuffers::default());
    // Also clean bindings on a rejected resource/device operation.
    clear_shader_bindings(context);
    result
}

#[derive(Default)]
pub(super) struct StreamOutputBuffers {
    by_identity: BTreeMap<String, (u32, ComPtr<ID3D11Buffer>)>,
}

pub(super) unsafe fn execute_prepared_runtime_draw_mrt(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    scene_constant_buffers: &[ConstantBufferBinding],
    draw: &crate::PreparedRuntimeDraw,
    target: &SubmittedMainSceneTarget,
    additional_colors: &[SubmittedColorTarget],
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    match draw {
        crate::PreparedRuntimeDraw::Particle { packet, .. } => execute_draw(
            gfx,
            resolver,
            scene_constant_buffers,
            &packet.shader_program,
            &packet.ordered_texture_bindings,
            &packet.pipeline,
            &packet.geometry,
            &packet.ordered_constant_buffers,
            Some(&packet.ordered_gpu_instance_records),
            Vec::new(),
            &packet.output_state,
            target,
            additional_colors,
            stream_output,
        ),
        crate::PreparedRuntimeDraw::Entity { submission, .. }
        | crate::PreparedRuntimeDraw::O04City { submission, .. }
        | crate::PreparedRuntimeDraw::O05Building { submission, .. } => {
            resolver
                .read_admitted_used_content(&submission.content_identity.resolved_entity_asset)?;
            resolver.read_admitted_used_content(&submission.content_identity.mesh)?;
            execute_draw(
                gfx,
                resolver,
                scene_constant_buffers,
                &submission.shader_program,
                &submission.ordered_texture_bindings,
                &submission.pipeline,
                &submission.geometry,
                &submission.ordered_constant_buffers,
                None,
                Vec::new(),
                &submission.output_state,
                target,
                additional_colors,
                stream_output,
            )
        }
    }
}

pub(super) unsafe fn execute_static_exact_used_draw(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    scene_constant_buffers: &[ConstantBufferBinding],
    draw: &replay_core::StaticMapDraw,
    political_textures: &[replay_core::PoliticalTexturePacket],
    prepared_targets: &[PreparedTargetTexture],
    target: &SubmittedMainSceneTarget,
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    let mut snow_mud_proof = None;
    let mut snow_mud_binding_count = 0;
    let mut occupation_secondary_proof = None;
    let mut occupation_terrain_t6_binding_count = 0;
    let mut occupation_water_t3_binding_count = 0;
    execute_static_exact_used_draw_mrt(
        gfx,
        resolver,
        scene_constant_buffers,
        draw,
        None,
        &mut snow_mud_proof,
        &mut snow_mud_binding_count,
        false,
        &mut occupation_secondary_proof,
        &mut occupation_terrain_t6_binding_count,
        &mut occupation_water_t3_binding_count,
        &[],
        political_textures,
        prepared_targets,
        target,
        &[],
        stream_output,
    )
}

pub(super) unsafe fn execute_static_exact_used_draw_mrt(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    scene_constant_buffers: &[ConstantBufferBinding],
    draw: &replay_core::StaticMapDraw,
    snow_mud_texture: Option<&replay_core::SnowMudTexturePacket>,
    snow_mud_proof: &mut Option<super::SnowMudD3dExecutionProof>,
    snow_mud_binding_count: &mut u32,
    collect_snow_mud_h2: bool,
    occupation_secondary_proof: &mut Option<super::OccupationSecondaryD3dExecutionProof>,
    occupation_terrain_t6_binding_count: &mut u32,
    occupation_water_t3_binding_count: &mut u32,
    authenticated_textures: &[AuthenticatedTextureGpu],
    political_textures: &[replay_core::PoliticalTexturePacket],
    prepared_targets: &[PreparedTargetTexture],
    target: &SubmittedMainSceneTarget,
    additional_colors: &[SubmittedColorTarget],
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    let geometry = match &draw.geometry {
        replay_core::StaticGeometrySource::Exact { geometry } => geometry,
        replay_core::StaticGeometrySource::InterpreterOutput { .. } => {
            return Err(
                "static geometry requires an interpreter output that is not materialized".into(),
            )
        }
    };
    let mut textures = Vec::with_capacity(draw.ordered_shader_resources.len());
    let mut dynamic_views = Vec::new();
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            replay_core::StaticShaderResourceSource::UsedContent {
                content,
                resource,
                view,
            } => textures.push(TextureBinding {
                stage: binding.stage,
                slot: binding.slot,
                semantic_token: binding.semantic_token.clone(),
                content: content.clone(),
                resource: resource.clone(),
                view: view.clone(),
            }),
            replay_core::StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256,
                view,
            } => {
                let snapshot = authenticated_textures
                    .iter()
                    .find(|snapshot| snapshot.snapshot_sha256 == *snapshot_sha256)
                    .ok_or(
                        "authenticated draw-time texture snapshot has no materialized frame resource",
                    )?;
                dynamic_views.push(create_authenticated_snapshot_texture_view(
                    gfx,
                    binding.stage,
                    binding.slot.into(),
                    snapshot,
                    view,
                )?);
            }
            replay_core::StaticShaderResourceSource::PreparedTarget {
                target,
                resource: _,
                view,
                producer,
            } => {
                let source = prepared_targets
                    .iter()
                    .find(|value| value.target == *target && value.producer == *producer)
                    .ok_or("prepared-target SRV producer/resource is not materialized")?;
                let native_desc = srv_desc(view)?;
                let mut native_view = ptr::null_mut();
                hr_ok(
                    (*gfx.device.as_raw()).CreateShaderResourceView(
                        source.resource,
                        &native_desc,
                        &mut native_view,
                    ),
                    "CreateShaderResourceView(prepared render graph target)",
                )?;
                dynamic_views.push(TextureViewGpu {
                    stage: binding.stage,
                    slot: binding.slot.into(),
                    _resource: None,
                    view: ComPtr::from_raw(native_view, "prepared render-graph SRV")?,
                });
            }
            replay_core::StaticShaderResourceSource::SnowMudTexture {
                producer_ordinal,
                payload_sha256,
                resource,
                view,
            } => {
                let packet = snow_mud_texture
                    .filter(|packet| {
                        packet.producer_ordinal == *producer_ordinal
                            && packet.payload_sha256 == *payload_sha256
                    })
                    .ok_or("SnowMudTexture source has no exact H packet")?;
                if packet.width != resource.width || packet.height != resource.height {
                    return Err("SnowMudTexture H packet/resource descriptor mismatch".into());
                }
                let mut immutable_resource = resource.clone();
                immutable_resource.usage_u32 = D3D11_USAGE_IMMUTABLE;
                immutable_resource.cpu_access_flags_u32 = 0;
                let inline = create_inline_texture_view(
                    gfx,
                    resolver,
                    binding.stage,
                    binding.slot.into(),
                    &immutable_resource,
                    view,
                    &packet.bytes,
                    packet.row_pitch,
                    packet
                        .row_pitch
                        .checked_mul(packet.height)
                        .ok_or("SnowMudTexture slice overflow")?,
                    payload_sha256,
                )?;
                *snow_mud_binding_count = snow_mud_binding_count
                    .checked_add(1)
                    .ok_or("SnowMudTexture executed binding count overflow")?;
                if collect_snow_mud_h2 {
                    let proof = verify_snow_mud_inline_readback(gfx, &inline, packet)?;
                    merge_snow_mud_execution_proof(snow_mud_proof, proof)?;
                }
                dynamic_views.push(inline);
            }
            replay_core::StaticShaderResourceSource::PoliticalTexture {
                texture_kind,
                target_binding_identity,
                upload_order,
                payload_sha256,
                resource,
                view,
            } => {
                let packet = political_textures
                    .iter()
                    .find(|packet| {
                        packet.kind == *texture_kind
                            && packet.target_binding_identity == *target_binding_identity
                            && packet.upload_order == *upload_order
                            && packet.sha256 == *payload_sha256
                    })
                    .ok_or("political texture source has no exact packet")?;
                if packet.width != resource.width || packet.height != resource.height {
                    return Err("political texture packet/resource descriptor mismatch".into());
                }
                let inline = create_inline_texture_view(
                    gfx,
                    resolver,
                    binding.stage,
                    binding.slot.into(),
                    resource,
                    view,
                    &packet.bytes,
                    packet.row_pitch,
                    packet
                        .row_pitch
                        .checked_mul(packet.height)
                        .ok_or("political texture slice overflow")?,
                    payload_sha256,
                )?;
                if *texture_kind == replay_core::PoliticalTextureKind::Secondary
                    && target_binding_identity == replay_core::OCCUPATION_SECONDARY_TARGET_BINDING
                {
                    let (terrain_t6_increment, water_t3_increment) = match (
                        binding.stage,
                        binding.slot,
                    ) {
                        (replay_core::ShaderStage::Pixel, 6) => (1, 0),
                        (replay_core::ShaderStage::Pixel, 3) => (0, 1),
                        _ => return Err(
                            "occupation secondary replay binding is outside terrain-t6/water-t3"
                                .into(),
                        ),
                    };
                    *occupation_terrain_t6_binding_count = occupation_terrain_t6_binding_count
                        .checked_add(terrain_t6_increment)
                        .ok_or("occupation terrain-t6 replay binding count overflow")?;
                    *occupation_water_t3_binding_count = occupation_water_t3_binding_count
                        .checked_add(water_t3_increment)
                        .ok_or("occupation water-t3 replay binding count overflow")?;
                    if collect_snow_mud_h2 {
                        let proof = verify_occupation_secondary_inline_readback(
                            gfx,
                            &inline,
                            packet,
                            terrain_t6_increment,
                            water_t3_increment,
                        )?;
                        merge_occupation_secondary_execution_proof(
                            occupation_secondary_proof,
                            proof,
                        )?;
                    }
                }
                dynamic_views.push(inline);
            }
            // Frame validation already refuses these sources: no lawful
            // payload exists until the owner proves the semantic point-light
            // census. Unreachable here; fail closed rather than bind a
            // neutral stand-in.
            replay_core::StaticShaderResourceSource::TiledLightIndex { .. }
            | replay_core::StaticShaderResourceSource::TiledLightData { .. } => {
                return Err(
                    "tiled-light texture has no lawful payload until the owner proves the semantic point-light census"
                        .into(),
                );
            }
        }
    }
    execute_draw(
        gfx,
        resolver,
        scene_constant_buffers,
        &draw.shader_program,
        &textures,
        &draw.pipeline,
        geometry,
        &draw.ordered_constant_buffers,
        None,
        dynamic_views,
        &draw.output_state,
        target,
        additional_colors,
        stream_output,
    )
}

impl StreamOutputBuffers {
    unsafe fn bind(
        &mut self,
        gfx: &Gfx,
        context: *mut ID3D11DeviceContext,
        targets: &[replay_core::StreamOutputTargetBinding],
    ) -> Result<()> {
        if targets.is_empty() {
            (*context).SOSetTargets(0, ptr::null(), ptr::null());
            return Ok(());
        }
        let (count, slots, offsets) = stream_output_layout(targets)?;
        for target in targets {
            match self.by_identity.get(&target.buffer_identity) {
                Some((width, _)) if *width != target.buffer_byte_width => {
                    return Err(
                        "stream-output buffer identity changed byte width within one submit".into(),
                    );
                }
                Some(_) => {}
                None => {
                    let desc = D3D11_BUFFER_DESC {
                        ByteWidth: target.buffer_byte_width,
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: D3D11_BIND_STREAM_OUTPUT,
                        CPUAccessFlags: 0,
                        MiscFlags: 0,
                        StructureByteStride: 0,
                    };
                    let mut buffer = ptr::null_mut();
                    hr_ok(
                        (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer),
                        "CreateBuffer for portable stream-output target",
                    )?;
                    self.by_identity.insert(
                        target.buffer_identity.clone(),
                        (
                            target.buffer_byte_width,
                            ComPtr::from_raw(buffer, "portable stream-output target")?,
                        ),
                    );
                }
            }
        }
        let mut buffers = [ptr::null_mut(); D3D11_SO_BUFFER_SLOT_COUNT as usize];
        for (slot, target_index) in slots.into_iter().enumerate() {
            let Some(target_index) = target_index else {
                continue;
            };
            buffers[slot] = self
                .by_identity
                .get(&targets[target_index].buffer_identity)
                .expect("stream-output buffer inserted above")
                .1
                .as_raw();
        }
        (*context).SOSetTargets(count, buffers.as_ptr(), offsets.as_ptr());
        Ok(())
    }
}

type StreamOutputLayout = (
    u32,
    [Option<usize>; D3D11_SO_BUFFER_SLOT_COUNT as usize],
    [u32; D3D11_SO_BUFFER_SLOT_COUNT as usize],
);

fn stream_output_layout(
    targets: &[replay_core::StreamOutputTargetBinding],
) -> Result<StreamOutputLayout> {
    let mut slots = [None; D3D11_SO_BUFFER_SLOT_COUNT as usize];
    let mut offsets = [0u32; D3D11_SO_BUFFER_SLOT_COUNT as usize];
    let mut count = 0;
    for (target_index, target) in targets.iter().enumerate() {
        let slot = usize::try_from(target.slot)?;
        let entry = slots
            .get_mut(slot)
            .ok_or("stream-output target slot exceeds D3D11 capacity")?;
        if entry.replace(target_index).is_some() {
            return Err("stream-output target slot is duplicated".into());
        }
        offsets[slot] = target.byte_offset;
        count = count.max(target.slot + 1);
    }
    Ok((count, slots, offsets))
}

pub(super) fn validate_output_viewport_scissor(output: &RenderOutputState) -> Result<()> {
    let viewport = [
        f32::from_bits(output.viewport.top_left_x_f32_bits),
        f32::from_bits(output.viewport.top_left_y_f32_bits),
        f32::from_bits(output.viewport.width_f32_bits),
        f32::from_bits(output.viewport.height_f32_bits),
        f32::from_bits(output.viewport.min_depth_f32_bits),
        f32::from_bits(output.viewport.max_depth_f32_bits),
    ];
    if !viewport.into_iter().all(f32::is_finite) || viewport[2] <= 0.0 || viewport[3] <= 0.0 {
        return Err("portable viewport is non-finite or empty".into());
    }
    if let Some(scissor) = &output.scissor {
        if scissor.right_i32 <= scissor.left_i32 || scissor.bottom_i32 <= scissor.top_i32 {
            return Err("portable scissor is empty or inverted".into());
        }
    }
    Ok(())
}

unsafe fn bind_output(
    gfx: &Gfx,
    output: &RenderOutputState,
    target: &SubmittedMainSceneTarget,
    additional_colors: &[SubmittedColorTarget],
) -> Result<()> {
    validate_output_viewport_scissor(output)?;
    if output.ordered_color_targets.len() != 1 + additional_colors.len() {
        return Err("submitted draw color-target count disagrees with prepared MRT target".into());
    }
    let primary = SubmittedColorTarget {
        identity: target.color_identity.clone(),
        rtv: target.color_rtv,
        width: target.color_width,
        height: target.color_height,
        format: target.color_format,
        samples: target.color_samples,
        quality: target.color_quality,
    };
    let mut rtvs = Vec::with_capacity(1 + additional_colors.len());
    for (color, prepared) in output
        .ordered_color_targets
        .iter()
        .zip(std::iter::once(&primary).chain(additional_colors))
    {
        if color.prepared_target != prepared.identity
            || color.persistent_content.is_some()
            || color.mip_level != 0
            || color.width != prepared.width
            || color.height != prepared.height
            || color.dxgi_format_u32 != prepared.format
            || color.sample_count != prepared.samples
            || color.sample_quality != prepared.quality
            || prepared.rtv.is_null()
        {
            return Err("submitted draw MRT color binding disagrees with prepared target".into());
        }
        rtvs.push(prepared.rtv);
    }
    let dsv = if let Some(depth) = &output.depth_target {
        if Some(depth.prepared_target.clone()) != target.depth_identity
            || depth.persistent_content.is_some()
            || depth.mip_level != 0
            || depth.width != target.depth_width
            || depth.height != target.depth_height
            || depth.dxgi_format_u32 != target.depth_format
            || depth.sample_count != target.depth_samples
            || depth.sample_quality != target.depth_quality
            || target.depth_dsv.is_null()
        {
            return Err(
                "submitted draw MainScene depth binding disagrees with prepared target".into(),
            );
        }
        target.depth_dsv
    } else {
        ptr::null_mut()
    };
    let context = gfx.context.as_raw();
    let target_count =
        u32::try_from(rtvs.len()).map_err(|_| "submitted MRT target count does not fit D3D11")?;
    if target_count > D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT {
        return Err("submitted MRT target count exceeds D3D11 capacity".into());
    }
    (*context).OMSetRenderTargets(target_count, rtvs.as_ptr(), dsv);
    let viewport = D3D11_VIEWPORT {
        TopLeftX: f32::from_bits(output.viewport.top_left_x_f32_bits),
        TopLeftY: f32::from_bits(output.viewport.top_left_y_f32_bits),
        Width: f32::from_bits(output.viewport.width_f32_bits),
        Height: f32::from_bits(output.viewport.height_f32_bits),
        MinDepth: f32::from_bits(output.viewport.min_depth_f32_bits),
        MaxDepth: f32::from_bits(output.viewport.max_depth_f32_bits),
    };
    (*context).RSSetViewports(1, &viewport);
    if let Some(scissor) = &output.scissor {
        let rect = RECT {
            left: scissor.left_i32,
            top: scissor.top_i32,
            right: scissor.right_i32,
            bottom: scissor.bottom_i32,
        };
        (*context).RSSetScissorRects(1, &rect);
    } else {
        (*context).RSSetScissorRects(0, ptr::null());
    }
    Ok(())
}

unsafe fn create_vertex_shader(gfx: &Gfx, bytes: &[u8]) -> Result<ComPtr<ID3D11VertexShader>> {
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateVertexShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreateVertexShader for portable submitted draw",
    )?;
    ComPtr::from_raw(shader, "portable submitted vertex shader")
}

unsafe fn create_pixel_shader(gfx: &Gfx, bytes: &[u8]) -> Result<ComPtr<ID3D11PixelShader>> {
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreatePixelShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreatePixelShader for portable submitted draw",
    )?;
    ComPtr::from_raw(shader, "portable submitted pixel shader")
}

unsafe fn create_geometry_shader(
    gfx: &Gfx,
    stage: Option<&AdditionalShaderStage>,
) -> Result<Option<ComPtr<ID3D11GeometryShader>>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateGeometryShader(
            stage.compiled.bytecode.as_ptr() as *const _,
            stage.compiled.bytecode.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreateGeometryShader for portable submitted draw",
    )?;
    Ok(Some(ComPtr::from_raw(
        shader,
        "portable submitted geometry shader",
    )?))
}

unsafe fn create_hull_shader(
    gfx: &Gfx,
    stage: Option<&AdditionalShaderStage>,
) -> Result<Option<ComPtr<ID3D11HullShader>>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateHullShader(
            stage.compiled.bytecode.as_ptr() as *const _,
            stage.compiled.bytecode.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreateHullShader for portable submitted draw",
    )?;
    Ok(Some(ComPtr::from_raw(
        shader,
        "portable submitted hull shader",
    )?))
}

unsafe fn create_domain_shader(
    gfx: &Gfx,
    stage: Option<&AdditionalShaderStage>,
) -> Result<Option<ComPtr<ID3D11DomainShader>>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateDomainShader(
            stage.compiled.bytecode.as_ptr() as *const _,
            stage.compiled.bytecode.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreateDomainShader for portable submitted draw",
    )?;
    Ok(Some(ComPtr::from_raw(
        shader,
        "portable submitted domain shader",
    )?))
}

unsafe fn create_compute_shader(
    gfx: &Gfx,
    stage: Option<&AdditionalShaderStage>,
) -> Result<Option<ComPtr<ID3D11ComputeShader>>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let mut shader = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateComputeShader(
            stage.compiled.bytecode.as_ptr() as *const _,
            stage.compiled.bytecode.len(),
            ptr::null_mut(),
            &mut shader,
        ),
        "CreateComputeShader for portable submitted draw",
    )?;
    Ok(Some(ComPtr::from_raw(
        shader,
        "portable submitted compute shader",
    )?))
}

unsafe fn create_input_layout(
    gfx: &Gfx,
    pipeline: &PipelineDesc,
    vs_bytes: &[u8],
) -> Result<ComPtr<ID3D11InputLayout>> {
    let semantics = pipeline
        .input_layout
        .iter()
        .map(|element| CString::new(element.semantic_token.as_str()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let native = pipeline
        .input_layout
        .iter()
        .zip(&semantics)
        .map(|(element, semantic)| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: semantic.as_ptr(),
            SemanticIndex: element.semantic_index,
            Format: element.dxgi_format_u32 as _,
            InputSlot: element.input_slot,
            AlignedByteOffset: element.aligned_byte_offset,
            InputSlotClass: if element.per_instance {
                D3D11_INPUT_PER_INSTANCE_DATA
            } else {
                D3D11_INPUT_PER_VERTEX_DATA
            },
            InstanceDataStepRate: element.instance_step_rate,
        })
        .collect::<Vec<_>>();
    let mut layout = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateInputLayout(
            native.as_ptr(),
            native.len() as u32,
            vs_bytes.as_ptr() as *const _,
            vs_bytes.len(),
            &mut layout,
        ),
        "CreateInputLayout for portable submitted draw",
    )?;
    ComPtr::from_raw(layout, "portable submitted input layout")
}

struct VertexBufferGpu {
    slot: u32,
    stride: u32,
    offset: u32,
    buffer: ComPtr<ID3D11Buffer>,
}

struct IndexBufferGpu {
    format: u32,
    offset: u32,
    buffer: ComPtr<ID3D11Buffer>,
}

unsafe fn create_vertex_buffers(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    geometry: &GeometryBinding,
    particle_records: Option<&[u8]>,
) -> Result<Vec<VertexBufferGpu>> {
    geometry
        .ordered_vertex_buffers
        .iter()
        .map(|binding| {
            let bytes = materialize_buffer(&binding.source, resolver, particle_records)?;
            Ok(VertexBufferGpu {
                slot: binding.input_slot,
                stride: binding.stride_bytes,
                offset: binding.byte_offset,
                buffer: create_buffer(gfx, D3D11_BIND_VERTEX_BUFFER, &bytes, "vertex")?,
            })
        })
        .collect()
}

unsafe fn create_index_buffer(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    geometry: &GeometryBinding,
    particle_records: Option<&[u8]>,
) -> Result<Option<IndexBufferGpu>> {
    let Some(binding) = &geometry.index_buffer else {
        return Ok(None);
    };
    let bytes = materialize_buffer(&binding.source, resolver, particle_records)?;
    let format = match binding.format {
        IndexFormat::R16Uint => DXGI_FORMAT_R16_UINT,
        IndexFormat::R32Uint => DXGI_FORMAT_R32_UINT,
        IndexFormat::None => return Err("portable index buffer has IndexFormat::None".into()),
    };
    Ok(Some(IndexBufferGpu {
        format,
        offset: binding.byte_offset,
        buffer: create_buffer(gfx, D3D11_BIND_INDEX_BUFFER, &bytes, "index")?,
    }))
}

unsafe fn create_buffer(
    gfx: &Gfx,
    bind_flags: u32,
    bytes: &[u8],
    role: &str,
) -> Result<ComPtr<ID3D11Buffer>> {
    let byte_width = u32::try_from(bytes.len())?;
    if byte_width == 0 {
        return Err(format!("portable submitted {role} buffer is empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: byte_width,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer),
        &format!("CreateBuffer for portable submitted {role}"),
    )?;
    ComPtr::from_raw(buffer, &format!("portable submitted {role} buffer"))
}

pub(super) fn materialize_buffer(
    source: &BufferSource,
    resolver: &GameFileResolver,
    particle_records: Option<&[u8]>,
) -> Result<Vec<u8>> {
    match source {
        BufferSource::Inline { bytes, .. } => Ok(bytes.clone()),
        BufferSource::ParticleInstanceRecords => particle_records
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "entity draw referenced particle instance records".into()),
        BufferSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            let bytes = resolver.read_admitted_used_content(content)?;
            let start = usize::try_from(*source_byte_offset)?;
            let count = usize::try_from(*byte_count)?;
            let end = start
                .checked_add(count)
                .ok_or("portable content range overflows")?;
            let range = bytes
                .get(start..end)
                .ok_or("portable content range exceeds admitted payload")?;
            if Sha256::hex(range) != *range_sha256 {
                return Err("portable content range SHA-256 mismatch during GPU upload".into());
            }
            Ok(range.to_vec())
        }
    }
}

struct ConstantBufferGpu {
    stage_mask: u8,
    slot: u32,
    buffer: ComPtr<ID3D11Buffer>,
}

unsafe fn create_constant_buffers(
    gfx: &Gfx,
    bindings: &[ConstantBufferBinding],
) -> Result<Vec<ConstantBufferGpu>> {
    bindings
        .iter()
        .map(|binding| {
            let start = usize::try_from(binding.byte_offset)?;
            let count = usize::try_from(binding.byte_count)?;
            let end = start
                .checked_add(count)
                .ok_or("constant-buffer range overflow")?;
            let selected = binding
                .raw_bytes
                .get(start..end)
                .ok_or("constant-buffer range exceeds payload")?;
            let padded_len = selected
                .len()
                .checked_add(15)
                .map(|value| value & !15)
                .ok_or("constant-buffer padding overflow")?;
            let mut bytes = vec![0u8; padded_len];
            bytes[..selected.len()].copy_from_slice(selected);
            Ok(ConstantBufferGpu {
                stage_mask: binding.stage_mask_u8,
                slot: binding.slot.into(),
                buffer: create_buffer(gfx, D3D11_BIND_CONSTANT_BUFFER, &bytes, "constant")?,
            })
        })
        .collect()
}

unsafe fn bind_constant_buffers(context: *mut ID3D11DeviceContext, buffers: &[ConstantBufferGpu]) {
    for binding in buffers {
        let raw = [binding.buffer.as_raw()];
        if binding.stage_mask & 0x01 != 0 {
            (*context).VSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
        if binding.stage_mask & 0x02 != 0 {
            (*context).PSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
        if binding.stage_mask & 0x04 != 0 {
            (*context).GSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
        if binding.stage_mask & 0x08 != 0 {
            (*context).HSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
        if binding.stage_mask & 0x10 != 0 {
            (*context).DSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
        if binding.stage_mask & 0x20 != 0 {
            (*context).CSSetConstantBuffers(binding.slot, 1, raw.as_ptr());
        }
    }
}

pub(super) enum TextureResourceGpu {
    Texture1d(ComPtr<ID3D11Texture1D>),
    Texture2d(ComPtr<ID3D11Texture2D>),
    Texture3d(ComPtr<ID3D11Texture3D>),
}

impl TextureResourceGpu {
    fn as_resource(&self) -> *mut ID3D11Resource {
        match self {
            Self::Texture1d(value) => value.as_raw() as *mut _,
            Self::Texture2d(value) => value.as_raw() as *mut _,
            Self::Texture3d(value) => value.as_raw() as *mut _,
        }
    }
}

struct TextureViewGpu {
    stage: ShaderStage,
    slot: u32,
    _resource: Option<TextureResourceGpu>,
    view: ComPtr<ID3D11ShaderResourceView>,
}

unsafe fn create_texture_views(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    bindings: &[TextureBinding],
    particle_records: Option<&[u8]>,
) -> Result<Vec<TextureViewGpu>> {
    bindings
        .iter()
        .map(|binding| {
            resolver.read_admitted_used_content(&binding.content)?;
            let (resource, storage, init) =
                create_texture_resource(gfx, resolver, &binding.resource, particle_records)?;
            let _storage = storage;
            let _init = init;
            let desc = srv_desc(&binding.view)?;
            let mut view = ptr::null_mut();
            hr_ok(
                (*gfx.device.as_raw()).CreateShaderResourceView(
                    resource.as_resource(),
                    &desc,
                    &mut view,
                ),
                "CreateShaderResourceView for portable submitted draw",
            )?;
            Ok(TextureViewGpu {
                stage: binding.stage,
                slot: binding.slot.into(),
                _resource: Some(resource),
                view: ComPtr::from_raw(view, "portable submitted texture view")?,
            })
        })
        .collect()
}

pub(super) unsafe fn create_authenticated_snapshot_texture_gpu(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    packet: &replay_core::AuthenticatedTextureSnapshotPacket,
) -> Result<AuthenticatedTextureGpu> {
    let resource_desc = &packet.resource;
    if resource_desc.sample_count != 1 || resource_desc.ordered_subresources.is_empty() {
        return Err(
            "authenticated draw-time texture snapshot is empty or unexpectedly multisampled".into(),
        );
    }
    let (resource, storage, init) = create_texture_resource(gfx, resolver, resource_desc, None)?;
    let _storage = storage;
    let _init = init;
    Ok(AuthenticatedTextureGpu {
        snapshot_sha256: packet.snapshot_sha256.clone(),
        resource,
    })
}

unsafe fn create_authenticated_snapshot_texture_view(
    gfx: &Gfx,
    stage: ShaderStage,
    slot: u32,
    snapshot: &AuthenticatedTextureGpu,
    view_desc: &ShaderResourceViewDesc,
) -> Result<TextureViewGpu> {
    let native_desc = srv_desc(view_desc)?;
    let mut view = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateShaderResourceView(
            snapshot.resource.as_resource(),
            &native_desc,
            &mut view,
        ),
        "CreateShaderResourceView for authenticated draw-time texture snapshot",
    )?;
    Ok(TextureViewGpu {
        stage,
        slot,
        _resource: None,
        view: ComPtr::from_raw(view, "authenticated draw-time texture snapshot view")?,
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_inline_texture_view(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    stage: ShaderStage,
    slot: u32,
    resource_desc: &TextureResourceDesc,
    view_desc: &ShaderResourceViewDesc,
    bytes: &[u8],
    row_pitch: u32,
    slice_pitch: u32,
    payload_sha256: &str,
) -> Result<TextureViewGpu> {
    if Sha256::hex(bytes) != payload_sha256 {
        return Err("dynamic texture payload sha256 mismatch".into());
    }
    if resource_desc.dimension != TextureResourceDimension::Texture2d
        || resource_desc.mip_levels != 1
        || resource_desc.array_size != 1
        || resource_desc.depth != 1
        || resource_desc.sample_count != 1
    {
        return Err("dynamic texture upload requires an exact single-subresource Texture2D".into());
    }
    let expected_len = usize::try_from(slice_pitch)
        .map_err(|_| "dynamic texture slice pitch does not fit usize")?;
    if row_pitch == 0 || slice_pitch == 0 || bytes.len() != expected_len {
        return Err("dynamic texture payload length/pitch mismatch".into());
    }
    let mut desc = resource_desc.clone();
    desc.ordered_subresources = vec![TextureSubresourcePayload {
        subresource_index: 0,
        mip_level: 0,
        array_slice: 0,
        row_pitch,
        slice_pitch,
        source: BufferSource::Inline {
            bytes: bytes.to_vec(),
            sha256: payload_sha256.to_string(),
        },
    }];
    let (resource, storage, init) = create_texture_resource(gfx, resolver, &desc, None)?;
    let _storage = storage;
    let _init = init;
    let native_desc = srv_desc(view_desc)?;
    let mut view = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateShaderResourceView(
            resource.as_resource(),
            &native_desc,
            &mut view,
        ),
        "CreateShaderResourceView for dynamic portable map texture",
    )?;
    Ok(TextureViewGpu {
        stage,
        slot,
        _resource: Some(resource),
        view: ComPtr::from_raw(view, "dynamic portable map texture view")?,
    })
}

fn merge_snow_mud_execution_proof(
    accumulated: &mut Option<super::SnowMudD3dExecutionProof>,
    next: super::SnowMudD3dExecutionProof,
) -> Result<()> {
    match accumulated {
        None => {
            *accumulated = Some(next);
        }
        Some(current) => {
            if current.tight_readback_bytes != next.tight_readback_bytes
                || current.readback_sha256 != next.readback_sha256
                || current.mapped_row_pitch != next.mapped_row_pitch
                || current.verified_row_count != next.verified_row_count
                || current.verified_tight_byte_count != next.verified_tight_byte_count
            {
                return Err("SnowMudTexture consumers produced inconsistent GPU readbacks".into());
            }
            current.replaced_binding_count = current
                .replaced_binding_count
                .checked_add(next.replaced_binding_count)
                .ok_or("SnowMudTexture replacement count overflow")?;
        }
    }
    Ok(())
}

unsafe fn verify_snow_mud_inline_readback(
    gfx: &Gfx,
    inline: &TextureViewGpu,
    packet: &replay_core::SnowMudTexturePacket,
) -> Result<super::SnowMudD3dExecutionProof> {
    let Some(TextureResourceGpu::Texture2d(texture)) = inline._resource.as_ref() else {
        return Err("SnowMudTexture replay upload is not a Texture2D".into());
    };
    let mut source_desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
    (*texture.as_raw()).GetDesc(&mut source_desc);
    if source_desc.Width != packet.width
        || source_desc.Height != packet.height
        || source_desc.Format as u32 != 87
        || source_desc.MipLevels != 1
        || source_desc.ArraySize != 1
        || source_desc.SampleDesc.Count != 1
    {
        return Err("SnowMudTexture replay upload descriptor changed before readback".into());
    }
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ,
        MiscFlags: 0,
        ..source_desc
    };
    let mut staging = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateTexture2D(&staging_desc, ptr::null(), &mut staging),
        "CreateTexture2D staging for product SnowMudTexture H2",
    )?;
    let staging = ComPtr::from_raw(staging, "product SnowMudTexture H2 staging")?;
    (*gfx.context.as_raw()).CopyResource(staging.as_raw() as *mut _, texture.as_raw() as *mut _);
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    hr_ok(
        (*gfx.context.as_raw()).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        ),
        "Map product SnowMudTexture H2 staging",
    )?;
    let verification = (|| -> Result<super::SnowMudD3dExecutionProof> {
        let source_pitch = usize::try_from(packet.row_pitch)
            .map_err(|_| "SnowMudTexture source pitch exceeds usize")?;
        let mapped_pitch = usize::try_from(mapped.RowPitch)
            .map_err(|_| "SnowMudTexture mapped pitch exceeds usize")?;
        if mapped.pData.is_null() || mapped_pitch < source_pitch {
            return Err("SnowMudTexture product readback returned a null/short row".into());
        }
        let mapped_len = mapped_pitch
            .checked_mul(packet.height as usize)
            .ok_or("SnowMudTexture mapped byte length overflow")?;
        let mapped_bytes = std::slice::from_raw_parts(mapped.pData as *const u8, mapped_len);
        let mut tight = Vec::with_capacity(packet.bytes.len());
        for row in 0..packet.height as usize {
            let mapped_start = row * mapped_pitch;
            let source_start = row * source_pitch;
            let row_bytes = &mapped_bytes[mapped_start..mapped_start + source_pitch];
            if row_bytes != &packet.bytes[source_start..source_start + source_pitch] {
                return Err(
                    format!("SnowMudTexture product readback differs from H at row {row}").into(),
                );
            }
            tight.extend_from_slice(row_bytes);
        }
        let readback_sha256 = Sha256::hex(&tight);
        Ok(super::SnowMudD3dExecutionProof {
            tight_readback_bytes: tight,
            readback_sha256,
            mapped_row_pitch: mapped.RowPitch,
            verified_row_count: packet.height,
            verified_tight_byte_count: u64::from(packet.row_pitch) * u64::from(packet.height),
            replaced_binding_count: 1,
        })
    })();
    (*gfx.context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
    verification
}

fn merge_occupation_secondary_execution_proof(
    accumulated: &mut Option<super::OccupationSecondaryD3dExecutionProof>,
    next: super::OccupationSecondaryD3dExecutionProof,
) -> Result<()> {
    match accumulated {
        None => *accumulated = Some(next),
        Some(current) => {
            if current.tight_readback_bytes != next.tight_readback_bytes
                || current.readback_sha256 != next.readback_sha256
                || current.mapped_row_pitch != next.mapped_row_pitch
                || current.verified_row_count != next.verified_row_count
                || current.verified_tight_byte_count != next.verified_tight_byte_count
            {
                return Err(
                    "occupation secondary consumers produced inconsistent GPU readbacks".into(),
                );
            }
            current.terrain_t6_binding_count = current
                .terrain_t6_binding_count
                .checked_add(next.terrain_t6_binding_count)
                .ok_or("occupation terrain-t6 proof count overflow")?;
            current.water_t3_binding_count = current
                .water_t3_binding_count
                .checked_add(next.water_t3_binding_count)
                .ok_or("occupation water-t3 proof count overflow")?;
        }
    }
    Ok(())
}

unsafe fn verify_occupation_secondary_inline_readback(
    gfx: &Gfx,
    inline: &TextureViewGpu,
    packet: &replay_core::PoliticalTexturePacket,
    terrain_t6_binding_count: u32,
    water_t3_binding_count: u32,
) -> Result<super::OccupationSecondaryD3dExecutionProof> {
    let Some(TextureResourceGpu::Texture2d(texture)) = inline._resource.as_ref() else {
        return Err("occupation secondary replay upload is not a Texture2D".into());
    };
    let mut source_desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
    (*texture.as_raw()).GetDesc(&mut source_desc);
    if source_desc.Width != replay_core::OCCUPATION_SECONDARY_WIDTH
        || source_desc.Height != replay_core::OCCUPATION_SECONDARY_HEIGHT
        || source_desc.Format as u32 != 91
        || source_desc.MipLevels != 1
        || source_desc.ArraySize != 1
        || source_desc.SampleDesc.Count != 1
        || packet.width != source_desc.Width
        || packet.height != source_desc.Height
    {
        return Err("occupation secondary replay upload descriptor changed before readback".into());
    }
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ,
        MiscFlags: 0,
        ..source_desc
    };
    let mut staging = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateTexture2D(&staging_desc, ptr::null(), &mut staging),
        "CreateTexture2D staging for occupation secondary H2",
    )?;
    let staging = ComPtr::from_raw(staging, "occupation secondary H2 staging")?;
    (*gfx.context.as_raw()).CopyResource(staging.as_raw() as *mut _, texture.as_raw() as *mut _);
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    hr_ok(
        (*gfx.context.as_raw()).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        ),
        "Map occupation secondary H2 staging",
    )?;
    let verification = (|| -> Result<super::OccupationSecondaryD3dExecutionProof> {
        let source_pitch = usize::try_from(packet.row_pitch)
            .map_err(|_| "occupation secondary source pitch exceeds usize")?;
        let mapped_pitch = usize::try_from(mapped.RowPitch)
            .map_err(|_| "occupation secondary mapped pitch exceeds usize")?;
        if mapped.pData.is_null() || mapped_pitch < source_pitch {
            return Err("occupation secondary readback returned a null/short row".into());
        }
        let mapped_len = mapped_pitch
            .checked_mul(packet.height as usize)
            .ok_or("occupation secondary mapped byte length overflow")?;
        let mapped_bytes = std::slice::from_raw_parts(mapped.pData as *const u8, mapped_len);
        let mut tight = Vec::with_capacity(packet.bytes.len());
        for row in 0..packet.height as usize {
            let mapped_start = row * mapped_pitch;
            let source_start = row * source_pitch;
            let row_bytes = &mapped_bytes[mapped_start..mapped_start + source_pitch];
            if row_bytes != &packet.bytes[source_start..source_start + source_pitch] {
                return Err(format!(
                    "occupation secondary replay readback differs from the packet at row {row}"
                )
                .into());
            }
            tight.extend_from_slice(row_bytes);
        }
        Ok(super::OccupationSecondaryD3dExecutionProof {
            readback_sha256: Sha256::hex(&tight),
            tight_readback_bytes: tight,
            mapped_row_pitch: mapped.RowPitch,
            verified_row_count: packet.height,
            verified_tight_byte_count: u64::from(packet.row_pitch) * u64::from(packet.height),
            terrain_t6_binding_count,
            water_t3_binding_count,
        })
    })();
    (*gfx.context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
    verification
}

fn is_block_compressed_dxgi_format(format: u32) -> bool {
    matches!(format, 70..=84 | 94..=99)
}

fn physical_row_count(format: u32, mip_height: u32) -> u32 {
    if is_block_compressed_dxgi_format(format) {
        mip_height.max(1).div_ceil(4)
    } else {
        mip_height
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_texture_subresource_payload(
    dimension: TextureResourceDimension,
    dxgi_format_u32: u32,
    width: u32,
    height: u32,
    depth: u32,
    mip_level: u32,
    row_pitch: u32,
    slice_pitch: u32,
    payload_len: usize,
) -> Result<()> {
    let mip_w = width.checked_shr(mip_level).unwrap_or(0).max(1);
    let mip_h = height.checked_shr(mip_level).unwrap_or(0).max(1);
    let mip_d = depth.checked_shr(mip_level).unwrap_or(0).max(1);
    if row_pitch == 0 {
        return Err(format!("texture subresource mip {mip_level} carries a zero row pitch").into());
    }
    if let Ok(minimum) = crate::d3d11_replay::native_d3d11_minimum_row_pitch(dxgi_format_u32, mip_w)
    {
        if row_pitch < minimum {
            return Err(format!(
                "texture subresource mip {mip_level} row_pitch {row_pitch} is below minimum \
                 {minimum} for DXGI format {dxgi_format_u32}"
            )
            .into());
        }
    }
    let rows = u64::from(physical_row_count(dxgi_format_u32, mip_h));
    let row_pitch_u64 = u64::from(row_pitch);
    let required = match dimension {
        TextureResourceDimension::Texture1d => row_pitch_u64,
        TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube => {
            row_pitch_u64
                .checked_mul(rows)
                .ok_or("texture subresource required size overflows")?
        }
        TextureResourceDimension::Texture3d => {
            let slice_rows = row_pitch_u64
                .checked_mul(rows)
                .ok_or("texture subresource required size overflows")?;
            if u64::from(slice_pitch) < slice_rows {
                return Err(format!(
                    "texture subresource mip {mip_level} slice_pitch {slice_pitch} is below \
                     row_pitch {row_pitch} * rows {rows}"
                )
                .into());
            }
            u64::from(slice_pitch)
                .checked_mul(u64::from(mip_d) - 1)
                .and_then(|value| value.checked_add(slice_rows))
                .ok_or("texture subresource required size overflows")?
        }
    };
    if u64::try_from(payload_len)? < required {
        return Err(format!(
            "texture subresource mip {mip_level} payload has {payload_len} bytes but the D3D11 \
             runtime reads {required}"
        )
        .into());
    }
    Ok(())
}

unsafe fn create_texture_resource(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    desc: &TextureResourceDesc,
    particle_records: Option<&[u8]>,
) -> Result<(
    TextureResourceGpu,
    Vec<Vec<u8>>,
    Vec<D3D11_SUBRESOURCE_DATA>,
)> {
    let storage = desc
        .ordered_subresources
        .iter()
        .map(|subresource| materialize_buffer(&subresource.source, resolver, particle_records))
        .collect::<Result<Vec<_>>>()?;
    for (subresource, bytes) in desc.ordered_subresources.iter().zip(&storage) {
        validate_texture_subresource_payload(
            desc.dimension,
            desc.dxgi_format_u32,
            desc.width,
            desc.height,
            desc.depth,
            subresource.mip_level,
            subresource.row_pitch,
            subresource.slice_pitch,
            bytes.len(),
        )?;
    }
    let init = desc
        .ordered_subresources
        .iter()
        .zip(&storage)
        .map(|(subresource, bytes)| D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: subresource.row_pitch,
            SysMemSlicePitch: subresource.slice_pitch,
        })
        .collect::<Vec<_>>();
    let resource = match desc.dimension {
        TextureResourceDimension::Texture1d => {
            let native = D3D11_TEXTURE1D_DESC {
                Width: desc.width,
                MipLevels: desc.mip_levels,
                ArraySize: desc.array_size,
                Format: desc.dxgi_format_u32 as _,
                Usage: desc.usage_u32,
                BindFlags: desc.bind_flags_u32,
                CPUAccessFlags: desc.cpu_access_flags_u32,
                MiscFlags: desc.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr_ok(
                (*gfx.device.as_raw()).CreateTexture1D(&native, init.as_ptr(), &mut texture),
                "CreateTexture1D for portable submitted draw",
            )?;
            TextureResourceGpu::Texture1d(ComPtr::from_raw(
                texture,
                "portable submitted Texture1D",
            )?)
        }
        TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube => {
            let native = D3D11_TEXTURE2D_DESC {
                Width: desc.width,
                Height: desc.height,
                MipLevels: desc.mip_levels,
                ArraySize: desc.array_size,
                Format: desc.dxgi_format_u32 as _,
                SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC {
                    Count: desc.sample_count,
                    Quality: desc.sample_quality,
                },
                Usage: desc.usage_u32,
                BindFlags: desc.bind_flags_u32,
                CPUAccessFlags: desc.cpu_access_flags_u32,
                MiscFlags: desc.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr_ok(
                (*gfx.device.as_raw()).CreateTexture2D(&native, init.as_ptr(), &mut texture),
                "CreateTexture2D for portable submitted draw",
            )?;
            TextureResourceGpu::Texture2d(ComPtr::from_raw(
                texture,
                "portable submitted Texture2D",
            )?)
        }
        TextureResourceDimension::Texture3d => {
            let native = D3D11_TEXTURE3D_DESC {
                Width: desc.width,
                Height: desc.height,
                Depth: desc.depth,
                MipLevels: desc.mip_levels,
                Format: desc.dxgi_format_u32 as _,
                Usage: desc.usage_u32,
                BindFlags: desc.bind_flags_u32,
                CPUAccessFlags: desc.cpu_access_flags_u32,
                MiscFlags: desc.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr_ok(
                (*gfx.device.as_raw()).CreateTexture3D(&native, init.as_ptr(), &mut texture),
                "CreateTexture3D for portable submitted draw",
            )?;
            TextureResourceGpu::Texture3d(ComPtr::from_raw(
                texture,
                "portable submitted Texture3D",
            )?)
        }
    };
    Ok((resource, storage, init))
}

unsafe fn srv_desc(view: &ShaderResourceViewDesc) -> Result<D3D11_SHADER_RESOURCE_VIEW_DESC> {
    let mut desc: D3D11_SHADER_RESOURCE_VIEW_DESC = mem::zeroed();
    match view {
        ShaderResourceViewDesc::Buffer {
            dxgi_format_u32,
            first_element,
            element_count,
            flags_u32,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_BUFFEREX;
            *desc.u.BufferEx_mut() = D3D11_BUFFEREX_SRV {
                FirstElement: *first_element,
                NumElements: *element_count,
                Flags: *flags_u32,
            };
        }
        ShaderResourceViewDesc::Texture1d {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE1D;
            *desc.u.Texture1D_mut() = D3D11_TEX1D_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
            };
        }
        ShaderResourceViewDesc::Texture1dArray {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
            first_array_slice,
            array_size,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE1DARRAY;
            *desc.u.Texture1DArray_mut() = D3D11_TEX1D_ARRAY_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        ShaderResourceViewDesc::Texture2d {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
            *desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
            };
        }
        ShaderResourceViewDesc::Texture2dArray {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
            first_array_slice,
            array_size,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DARRAY;
            *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        ShaderResourceViewDesc::Texture2dMultisampled { dxgi_format_u32 } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DMS;
        }
        ShaderResourceViewDesc::Texture2dMultisampledArray {
            dxgi_format_u32,
            first_array_slice,
            array_size,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DMSARRAY;
            *desc.u.Texture2DMSArray_mut() = D3D11_TEX2DMS_ARRAY_SRV {
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        ShaderResourceViewDesc::Texture3d {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE3D;
            *desc.u.Texture3D_mut() = D3D11_TEX3D_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
            };
        }
        ShaderResourceViewDesc::TextureCube {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURECUBE;
            *desc.u.TextureCube_mut() = D3D11_TEXCUBE_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
            };
        }
        ShaderResourceViewDesc::TextureCubeArray {
            dxgi_format_u32,
            most_detailed_mip,
            mip_levels,
            first_face,
            cube_count,
        } => {
            desc.Format = *dxgi_format_u32 as _;
            desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURECUBEARRAY;
            *desc.u.TextureCubeArray_mut() = D3D11_TEXCUBE_ARRAY_SRV {
                MostDetailedMip: *most_detailed_mip,
                MipLevels: *mip_levels,
                First2DArrayFace: *first_face,
                NumCubes: *cube_count,
            };
        }
    }
    Ok(desc)
}

unsafe fn bind_texture_views(context: *mut ID3D11DeviceContext, views: &[TextureViewGpu]) {
    for binding in views {
        let raw = [binding.view.as_raw()];
        match binding.stage {
            ShaderStage::Vertex => (*context).VSSetShaderResources(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Pixel => (*context).PSSetShaderResources(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Geometry => (*context).GSSetShaderResources(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Hull => (*context).HSSetShaderResources(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Domain => (*context).DSSetShaderResources(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Compute => (*context).CSSetShaderResources(binding.slot, 1, raw.as_ptr()),
        }
    }
}

struct SamplerGpu {
    stage: ShaderStage,
    slot: u32,
    sampler: ComPtr<ID3D11SamplerState>,
}

unsafe fn create_samplers(gfx: &Gfx, samplers: &[SamplerDesc]) -> Result<Vec<SamplerGpu>> {
    samplers
        .iter()
        .map(|sampler| {
            let native = D3D11_SAMPLER_DESC {
                Filter: filter(sampler)?,
                AddressU: address(sampler.address_u),
                AddressV: address(sampler.address_v),
                AddressW: address(sampler.address_w),
                MipLODBias: f32::from_bits(sampler.mip_lod_bias_f32_bits),
                MaxAnisotropy: sampler.max_anisotropy,
                ComparisonFunc: sampler
                    .comparison
                    .map(comparison)
                    .unwrap_or(D3D11_COMPARISON_NEVER),
                BorderColor: sampler.border_color_rgba_f32_bits.map(f32::from_bits),
                MinLOD: f32::from_bits(sampler.min_lod_f32_bits),
                MaxLOD: f32::from_bits(sampler.max_lod_f32_bits),
            };
            let mut state = ptr::null_mut();
            hr_ok(
                (*gfx.device.as_raw()).CreateSamplerState(&native, &mut state),
                "CreateSamplerState for portable submitted draw",
            )?;
            Ok(SamplerGpu {
                stage: sampler.stage,
                slot: sampler.slot.into(),
                sampler: ComPtr::from_raw(state, "portable submitted sampler")?,
            })
        })
        .collect()
}

unsafe fn bind_samplers(context: *mut ID3D11DeviceContext, samplers: &[SamplerGpu]) {
    for binding in samplers {
        let raw = [binding.sampler.as_raw()];
        match binding.stage {
            ShaderStage::Vertex => (*context).VSSetSamplers(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Pixel => (*context).PSSetSamplers(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Geometry => (*context).GSSetSamplers(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Hull => (*context).HSSetSamplers(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Domain => (*context).DSSetSamplers(binding.slot, 1, raw.as_ptr()),
            ShaderStage::Compute => (*context).CSSetSamplers(binding.slot, 1, raw.as_ptr()),
        }
    }
}

unsafe fn create_rasterizer(
    gfx: &Gfx,
    rasterizer: &RasterizerDesc,
) -> Result<ComPtr<ID3D11RasterizerState>> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: match rasterizer.fill {
            FillMode::Solid => D3D11_FILL_SOLID,
            FillMode::Wireframe => D3D11_FILL_WIREFRAME,
        },
        CullMode: match rasterizer.cull {
            CullMode::None => D3D11_CULL_NONE,
            CullMode::Front => D3D11_CULL_FRONT,
            CullMode::Back => D3D11_CULL_BACK,
        },
        FrontCounterClockwise: rasterizer.front_counter_clockwise.into(),
        DepthBias: rasterizer.depth_bias_i32,
        DepthBiasClamp: f32::from_bits(rasterizer.depth_bias_clamp_f32_bits),
        SlopeScaledDepthBias: f32::from_bits(rasterizer.slope_scaled_depth_bias_f32_bits),
        DepthClipEnable: rasterizer.depth_clip_enabled.into(),
        ScissorEnable: rasterizer.scissor_enabled.into(),
        MultisampleEnable: rasterizer.multisample_enabled.into(),
        AntialiasedLineEnable: rasterizer.antialiased_line_enabled.into(),
    };
    let mut state = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateRasterizerState(&desc, &mut state),
        "CreateRasterizerState for portable submitted draw",
    )?;
    ComPtr::from_raw(state, "portable submitted rasterizer")
}

unsafe fn create_blend(gfx: &Gfx, pipeline: &PipelineDesc) -> Result<ComPtr<ID3D11BlendState>> {
    let blend = &pipeline.blend;
    let mut desc: D3D11_BLEND_DESC = mem::zeroed();
    desc.AlphaToCoverageEnable = blend.alpha_to_coverage_enabled.into();
    desc.IndependentBlendEnable = blend.independent_blend_enabled.into();
    desc.RenderTarget[0] = render_target_blend(
        blend.enabled,
        blend.source_color,
        blend.destination_color,
        blend.color_operation,
        blend.source_alpha,
        blend.destination_alpha,
        blend.alpha_operation,
        blend.write_mask_rgba_u8,
    );
    for (index, target) in blend.additional_targets.iter().enumerate() {
        let slot = index + 1;
        if slot >= desc.RenderTarget.len() {
            return Err("portable blend state has more than eight render targets".into());
        }
        desc.RenderTarget[slot] = render_target_blend_desc(target);
    }
    let mut state = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateBlendState(&desc, &mut state),
        "CreateBlendState for portable submitted draw",
    )?;
    ComPtr::from_raw(state, "portable submitted blend state")
}

fn render_target_blend_desc(value: &RenderTargetBlendDesc) -> D3D11_RENDER_TARGET_BLEND_DESC {
    render_target_blend(
        value.enabled,
        value.source_color,
        value.destination_color,
        value.color_operation,
        value.source_alpha,
        value.destination_alpha,
        value.alpha_operation,
        value.write_mask_rgba_u8,
    )
}

fn render_target_blend(
    enabled: bool,
    src: BlendFactor,
    dst: BlendFactor,
    op: BlendOperation,
    src_alpha: BlendFactor,
    dst_alpha: BlendFactor,
    op_alpha: BlendOperation,
    mask: u8,
) -> D3D11_RENDER_TARGET_BLEND_DESC {
    D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: enabled.into(),
        SrcBlend: blend_factor(src),
        DestBlend: blend_factor(dst),
        BlendOp: blend_operation(op),
        SrcBlendAlpha: blend_factor(src_alpha),
        DestBlendAlpha: blend_factor(dst_alpha),
        BlendOpAlpha: blend_operation(op_alpha),
        RenderTargetWriteMask: mask,
    }
}

unsafe fn create_depth(
    gfx: &Gfx,
    pipeline: &PipelineDesc,
) -> Result<ComPtr<ID3D11DepthStencilState>> {
    let depth = &pipeline.depth;
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: depth.enabled.into(),
        DepthWriteMask: if depth.write_enabled {
            D3D11_DEPTH_WRITE_MASK_ALL
        } else {
            D3D11_DEPTH_WRITE_MASK_ZERO
        },
        DepthFunc: comparison(depth.comparison),
        StencilEnable: depth.stencil_enabled.into(),
        StencilReadMask: depth.stencil_read_mask_u8,
        StencilWriteMask: depth.stencil_write_mask_u8,
        FrontFace: stencil_face(&depth.front_face),
        BackFace: stencil_face(&depth.back_face),
    };
    let mut state = ptr::null_mut();
    hr_ok(
        (*gfx.device.as_raw()).CreateDepthStencilState(&desc, &mut state),
        "CreateDepthStencilState for portable submitted draw",
    )?;
    ComPtr::from_raw(state, "portable submitted depth state")
}

fn stencil_face(value: &StencilFaceDesc) -> D3D11_DEPTH_STENCILOP_DESC {
    D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: stencil_operation(value.fail_operation),
        StencilDepthFailOp: stencil_operation(value.depth_fail_operation),
        StencilPassOp: stencil_operation(value.pass_operation),
        StencilFunc: comparison(value.comparison),
    }
}

unsafe fn clear_shader_bindings(context: *mut ID3D11DeviceContext) {
    let null_srvs = [ptr::null_mut(); D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT as usize];
    let null_samplers = [ptr::null_mut(); D3D11_COMMONSHADER_SAMPLER_SLOT_COUNT as usize];
    let null_cbs = [ptr::null_mut(); D3D11_COMMONSHADER_CONSTANT_BUFFER_API_SLOT_COUNT as usize];
    (*context).VSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).PSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).CSSetShader(ptr::null_mut(), ptr::null(), 0);
    (*context).VSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).PSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).GSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).HSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).DSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).CSSetShaderResources(0, null_srvs.len() as u32, null_srvs.as_ptr());
    (*context).VSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).PSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).GSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).HSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).DSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).CSSetSamplers(0, null_samplers.len() as u32, null_samplers.as_ptr());
    (*context).VSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
    (*context).PSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
    (*context).GSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
    (*context).HSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
    (*context).DSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
    (*context).CSSetConstantBuffers(0, null_cbs.len() as u32, null_cbs.as_ptr());
}

fn topology(value: PrimitiveTopology) -> D3D_PRIMITIVE_TOPOLOGY {
    match value {
        PrimitiveTopology::TriangleList => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
        PrimitiveTopology::TriangleStrip => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
        PrimitiveTopology::LineList => D3D11_PRIMITIVE_TOPOLOGY_LINELIST,
        PrimitiveTopology::LineStrip => D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP,
        PrimitiveTopology::PointList => D3D11_PRIMITIVE_TOPOLOGY_POINTLIST,
    }
}

fn comparison(value: Comparison) -> D3D11_COMPARISON_FUNC {
    match value {
        Comparison::Never => D3D11_COMPARISON_NEVER,
        Comparison::Less => D3D11_COMPARISON_LESS,
        Comparison::Equal => D3D11_COMPARISON_EQUAL,
        Comparison::LessEqual => D3D11_COMPARISON_LESS_EQUAL,
        Comparison::Greater => D3D11_COMPARISON_GREATER,
        Comparison::NotEqual => D3D11_COMPARISON_NOT_EQUAL,
        Comparison::GreaterEqual => D3D11_COMPARISON_GREATER_EQUAL,
        Comparison::Always => D3D11_COMPARISON_ALWAYS,
    }
}

fn stencil_operation(value: StencilOperation) -> D3D11_STENCIL_OP {
    match value {
        StencilOperation::Keep => D3D11_STENCIL_OP_KEEP,
        StencilOperation::Zero => D3D11_STENCIL_OP_ZERO,
        StencilOperation::Replace => D3D11_STENCIL_OP_REPLACE,
        StencilOperation::IncrementSaturate => D3D11_STENCIL_OP_INCR_SAT,
        StencilOperation::DecrementSaturate => D3D11_STENCIL_OP_DECR_SAT,
        StencilOperation::Invert => D3D11_STENCIL_OP_INVERT,
        StencilOperation::Increment => D3D11_STENCIL_OP_INCR,
        StencilOperation::Decrement => D3D11_STENCIL_OP_DECR,
    }
}

fn blend_factor(value: BlendFactor) -> D3D11_BLEND {
    match value {
        BlendFactor::Zero => D3D11_BLEND_ZERO,
        BlendFactor::One => D3D11_BLEND_ONE,
        BlendFactor::SourceColor => D3D11_BLEND_SRC_COLOR,
        BlendFactor::InverseSourceColor => D3D11_BLEND_INV_SRC_COLOR,
        BlendFactor::SourceAlpha => D3D11_BLEND_SRC_ALPHA,
        BlendFactor::InverseSourceAlpha => D3D11_BLEND_INV_SRC_ALPHA,
        BlendFactor::DestinationAlpha => D3D11_BLEND_DEST_ALPHA,
        BlendFactor::InverseDestinationAlpha => D3D11_BLEND_INV_DEST_ALPHA,
        BlendFactor::DestinationColor => D3D11_BLEND_DEST_COLOR,
        BlendFactor::InverseDestinationColor => D3D11_BLEND_INV_DEST_COLOR,
        BlendFactor::SourceAlphaSaturate => D3D11_BLEND_SRC_ALPHA_SAT,
        BlendFactor::BlendFactor => D3D11_BLEND_BLEND_FACTOR,
        BlendFactor::InverseBlendFactor => D3D11_BLEND_INV_BLEND_FACTOR,
        BlendFactor::Source1Color => D3D11_BLEND_SRC1_COLOR,
        BlendFactor::InverseSource1Color => D3D11_BLEND_INV_SRC1_COLOR,
        BlendFactor::Source1Alpha => D3D11_BLEND_SRC1_ALPHA,
        BlendFactor::InverseSource1Alpha => D3D11_BLEND_INV_SRC1_ALPHA,
    }
}

fn blend_operation(value: BlendOperation) -> D3D11_BLEND_OP {
    match value {
        BlendOperation::Add => D3D11_BLEND_OP_ADD,
        BlendOperation::Subtract => D3D11_BLEND_OP_SUBTRACT,
        BlendOperation::ReverseSubtract => D3D11_BLEND_OP_REV_SUBTRACT,
        BlendOperation::Minimum => D3D11_BLEND_OP_MIN,
        BlendOperation::Maximum => D3D11_BLEND_OP_MAX,
    }
}

fn address(value: AddressMode) -> D3D11_TEXTURE_ADDRESS_MODE {
    match value {
        AddressMode::Wrap => D3D11_TEXTURE_ADDRESS_WRAP,
        AddressMode::Mirror => D3D11_TEXTURE_ADDRESS_MIRROR,
        AddressMode::Clamp => D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressMode::Border => D3D11_TEXTURE_ADDRESS_BORDER,
    }
}

fn filter(value: &SamplerDesc) -> Result<D3D11_FILTER> {
    let base = match (value.min_filter, value.mag_filter, value.mip_filter) {
        (Filter::Point, Filter::Point, Filter::Point) => D3D11_FILTER_MIN_MAG_MIP_POINT,
        (Filter::Point, Filter::Point, Filter::Linear) => D3D11_FILTER_MIN_MAG_POINT_MIP_LINEAR,
        (Filter::Point, Filter::Linear, Filter::Point) => {
            D3D11_FILTER_MIN_POINT_MAG_LINEAR_MIP_POINT
        }
        (Filter::Point, Filter::Linear, Filter::Linear) => D3D11_FILTER_MIN_POINT_MAG_MIP_LINEAR,
        (Filter::Linear, Filter::Point, Filter::Point) => D3D11_FILTER_MIN_LINEAR_MAG_MIP_POINT,
        (Filter::Linear, Filter::Point, Filter::Linear) => {
            D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR
        }
        (Filter::Linear, Filter::Linear, Filter::Point) => D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
        (Filter::Linear, Filter::Linear, Filter::Linear) => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        (Filter::Anisotropic, Filter::Anisotropic, Filter::Anisotropic) => D3D11_FILTER_ANISOTROPIC,
        _ => return Err("D3D11 sampler has a non-representable mixed anisotropic filter".into()),
    };
    Ok(if value.comparison.is_some() {
        base | 0x80
    } else {
        base
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_subresource_admission_refuses_payloads_smaller_than_driver_reads() {
        let error = validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            0,
            32,
            128,
            100,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("runtime reads 128"), "{error}");
        let error = validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            256,
            1,
            0,
            4096,
            4096 * 256,
            4,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("payload has 4 bytes"), "{error}");
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            0,
            32,
            128,
            128,
        )
        .unwrap();
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            0,
            32,
            128,
            256,
        )
        .unwrap();
        assert!(validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            0,
            0,
            128,
            128,
        )
        .unwrap_err()
        .to_string()
        .contains("zero row pitch"));
    }

    #[test]
    fn texture_subresource_admission_uses_bc_block_rows_not_texel_rows() {
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            71,
            8,
            8,
            1,
            0,
            16,
            32,
            32,
        )
        .unwrap();
        let error = validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            71,
            8,
            8,
            1,
            0,
            16,
            32,
            31,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("runtime reads 32"), "{error}");
    }

    #[test]
    fn texture_subresource_admission_enforces_known_format_minimum_row_pitch() {
        let error = validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            0,
            16,
            64,
            1024,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("below minimum 32"), "{error}");
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            24,
            4,
            4,
            1,
            0,
            4,
            16,
            16,
        )
        .unwrap();
        assert!(validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            24,
            4,
            4,
            1,
            0,
            4,
            16,
            15,
        )
        .is_err());
    }

    #[test]
    fn texture3d_subresource_admission_covers_slice_pitch_and_depth() {
        let error = validate_texture_subresource_payload(
            TextureResourceDimension::Texture3d,
            28,
            4,
            4,
            2,
            0,
            16,
            16,
            1 << 20,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("slice_pitch 16 is below"), "{error}");
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture3d,
            28,
            4,
            4,
            2,
            0,
            16,
            64,
            128,
        )
        .unwrap();
        assert!(validate_texture_subresource_payload(
            TextureResourceDimension::Texture3d,
            28,
            4,
            4,
            2,
            0,
            16,
            64,
            127,
        )
        .is_err());
    }

    #[test]
    fn texture_subresource_admission_shrinks_geometry_per_mip() {
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture2d,
            28,
            8,
            4,
            1,
            1,
            16,
            32,
            32,
        )
        .unwrap();
        validate_texture_subresource_payload(
            TextureResourceDimension::Texture3d,
            28,
            8,
            4,
            4,
            1,
            16,
            32,
            64,
        )
        .unwrap();
        assert!(validate_texture_subresource_payload(
            TextureResourceDimension::Texture3d,
            28,
            8,
            4,
            4,
            1,
            16,
            32,
            63,
        )
        .is_err());
    }

    fn viewport_output(
        width_f32_bits: u32,
        height_f32_bits: u32,
        scissor: Option<replay_core::ScissorDesc>,
    ) -> RenderOutputState {
        RenderOutputState {
            ordered_color_targets: Vec::new(),
            depth_target: None,
            viewport: replay_core::ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits,
                height_f32_bits,
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor,
        }
    }

    #[test]
    fn output_admission_rejects_non_finite_empty_or_inverted_state() {
        validate_output_viewport_scissor(&viewport_output(8f32.to_bits(), 4f32.to_bits(), None))
            .unwrap();
        let nan = validate_output_viewport_scissor(&viewport_output(
            f32::NAN.to_bits(),
            4f32.to_bits(),
            None,
        ))
        .unwrap_err()
        .to_string();
        assert!(nan.contains("non-finite or empty"), "{nan}");
        assert!(
            validate_output_viewport_scissor(&viewport_output(8f32.to_bits(), 0, None)).is_err()
        );
        assert!(validate_output_viewport_scissor(&viewport_output(
            8f32.to_bits(),
            (-4f32).to_bits(),
            None,
        ))
        .is_err());
        let inverted = validate_output_viewport_scissor(&viewport_output(
            8f32.to_bits(),
            4f32.to_bits(),
            Some(replay_core::ScissorDesc {
                left_i32: 4,
                top_i32: 0,
                right_i32: 4,
                bottom_i32: 4,
            }),
        ))
        .unwrap_err()
        .to_string();
        assert!(inverted.contains("empty or inverted"), "{inverted}");
        validate_output_viewport_scissor(&viewport_output(
            8f32.to_bits(),
            4f32.to_bits(),
            Some(replay_core::ScissorDesc {
                left_i32: 0,
                top_i32: 0,
                right_i32: 8,
                bottom_i32: 4,
            }),
        ))
        .unwrap();
    }

    #[test]
    fn d3d_enum_mappings_cover_exact_extended_state() {
        assert_eq!(
            blend_factor(BlendFactor::Source1Alpha),
            D3D11_BLEND_SRC1_ALPHA
        );
        assert_eq!(
            stencil_operation(StencilOperation::Invert),
            D3D11_STENCIL_OP_INVERT
        );
        assert_eq!(
            comparison(Comparison::LessEqual),
            D3D11_COMPARISON_LESS_EQUAL
        );
        assert_eq!(address(AddressMode::Border), D3D11_TEXTURE_ADDRESS_BORDER);
    }

    #[test]
    fn stream_output_layout_preserves_sparse_slots_and_append_offset() {
        let targets = vec![
            replay_core::StreamOutputTargetBinding {
                slot: 1,
                buffer_identity: "first".into(),
                buffer_byte_width: 256,
                byte_offset: u32::MAX,
            },
            replay_core::StreamOutputTargetBinding {
                slot: 3,
                buffer_identity: "second".into(),
                buffer_byte_width: 512,
                byte_offset: 32,
            },
        ];
        let (count, slots, offsets) = stream_output_layout(&targets).expect("SO layout");
        assert_eq!(count, 4);
        assert_eq!(slots, [None, Some(0), None, Some(1)]);
        assert_eq!(offsets, [0, u32::MAX, 0, 32]);

        let mut duplicate = targets;
        duplicate[1].slot = 1;
        assert!(stream_output_layout(&duplicate)
            .expect_err("duplicate SO slot")
            .to_string()
            .contains("duplicated"));
    }
}
