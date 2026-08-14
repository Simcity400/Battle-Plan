//! Exact execution of portable particle/entity draw submissions.
//!
//! The replay carrier is already structurally validated by `replay_core`.
//! This module supplies the remaining production boundary: every referenced
//! game payload is reopened only through a validated effective-VFS admission,
//! every D3D descriptor is reconstructed literally, and draw interleave is
//! taken only from `RenderSubmit::ordered_draws`.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::mem;
use std::ptr;

use map_viewer::{GameFileResolver, NativeProductionMainSceneTarget};
use replay_core::{
    AdditionalShaderStage, AddressMode, BlendFactor, BlendOperation, BufferSource, Comparison,
    ConstantBufferBinding, CullMode, DrawCommand, EntitySubmission, FillMode, Filter,
    GeometryBinding, IndexFormat, ParticlePacket, PipelineDesc, PreparedRenderTarget,
    PrimitiveTopology, RasterizerDesc, RenderOutputRef, RenderOutputState, RenderSubmit,
    RenderTargetBlendDesc, SamplerDesc, ShaderProgram, ShaderResourceViewDesc, ShaderStage,
    StencilFaceDesc, StencilOperation, TextureBinding, TextureResourceDesc,
    TextureResourceDimension,
};
use sha2::{Digest, Sha256};
use winapi::shared::dxgiformat::{DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32_UINT};
use winapi::shared::windef::RECT;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;

use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::Result;

/// Borrowed render-graph target that already contains the exact map-object
/// predecessors of the submitted particle/entity sequence. Portable attachment
/// identities are optional only so legacy debug callers can represent their
/// lack of authority; execution rejects such callers before binding a view.
pub struct SubmittedMainSceneTarget {
    pub(crate) color_identity: Option<PreparedRenderTarget>,
    pub(crate) depth_identity: Option<PreparedRenderTarget>,
    pub(crate) color_rtv: *mut ID3D11RenderTargetView,
    pub(crate) depth_dsv: *mut ID3D11DepthStencilView,
    pub(crate) color_width: u32,
    pub(crate) color_height: u32,
    pub(crate) color_format: u32,
    pub(crate) color_samples: u32,
    pub(crate) color_quality: u32,
    pub(crate) depth_width: u32,
    pub(crate) depth_height: u32,
    pub(crate) depth_format: u32,
    pub(crate) depth_samples: u32,
    pub(crate) depth_quality: u32,
}

impl SubmittedMainSceneTarget {
    pub(crate) fn from_native(target: &NativeProductionMainSceneTarget) -> Self {
        let desc = target.desc();
        Self {
            color_identity: Some(PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: desc.color_attachment_ordinal,
            }),
            depth_identity: Some(PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: desc.depth_attachment_ordinal,
            }),
            color_rtv: target.color_rtv(),
            depth_dsv: target.depth_dsv(),
            color_width: desc.color_width,
            color_height: desc.color_height,
            color_format: desc.color_format,
            color_samples: desc.color_samples,
            color_quality: desc.color_quality,
            depth_width: desc.depth_width,
            depth_height: desc.depth_height,
            depth_format: desc.depth_format,
            depth_samples: desc.depth_samples,
            depth_quality: desc.depth_quality,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmittedDrawStats {
    pub particle_draws: usize,
    pub entity_draws: usize,
    pub direct_visual_joins: usize,
    pub weather_joins: usize,
}

#[derive(Debug)]
enum OrderedDraw<'a> {
    Particle(&'a ParticlePacket),
    Entity(&'a EntitySubmission),
}

enum SubmittedOutput<'a> {
    AuthenticatedPacket(&'a RenderOutputState),
    ViewerOwnedMainScene,
}

/// Resolve the authoritative cross-family sequence without sorting or
/// inference. `portable_render_stream` validation guarantees every submitted
/// particle/entity appears exactly once; this lookup repeats the identity
/// check at the execution boundary so malformed in-memory callers fail closed.
fn ordered_draws<'a>(submit: &'a RenderSubmit) -> Result<Vec<OrderedDraw<'a>>> {
    let mut out = Vec::with_capacity(submit.ordered_draws.len());
    let mut seen_particles = vec![false; submit.particles.len()];
    let mut seen_entities = vec![false; submit.entities.len()];
    for draw in &submit.ordered_draws {
        match draw {
            RenderOutputRef::Particle {
                effect_instance_id,
                bucket,
                packet_ordinal,
                draw_order,
            } => {
                let (packet_index, packet) = submit
                    .particles
                    .iter()
                    .enumerate()
                    .find(|packet| {
                        packet.1.effect_instance_id == *effect_instance_id
                            && packet.1.bucket == *bucket
                            && packet.1.packet_ordinal == *packet_ordinal
                            && packet.1.draw_order == *draw_order
                    })
                    .ok_or("ordered particle draw references an absent packet")?;
                if !mark_once(&mut seen_particles, packet_index) {
                    return Err("ordered particle draw references the same packet twice".into());
                }
                out.push(OrderedDraw::Particle(packet));
            }
            RenderOutputRef::Entity {
                pass_ordinal,
                entity_instance_id,
                draw_order,
            } => {
                let (entity_index, entity) = submit
                    .entities
                    .iter()
                    .enumerate()
                    .find(|entity| {
                        entity.1.pass_ordinal == *pass_ordinal
                            && entity.1.entity_instance_id == *entity_instance_id
                            && entity.1.draw_order == *draw_order
                    })
                    .ok_or("ordered entity draw references an absent submission")?;
                if !mark_once(&mut seen_entities, entity_index) {
                    return Err("ordered entity draw references the same submission twice".into());
                }
                out.push(OrderedDraw::Entity(entity));
            }
        }
    }
    if seen_particles.iter().any(|seen| !seen) || seen_entities.iter().any(|seen| !seen) {
        return Err("ordered draw list does not cover the complete submitted draw set".into());
    }
    Ok(out)
}

fn mark_once(seen: &mut [bool], index: usize) -> bool {
    !std::mem::replace(&mut seen[index], true)
}

/// Execute the selected submit into an already prepared opaque target.
///
/// # Safety
/// `target` views must belong to `gfx.device`, remain alive for this call, and
/// refer to the descriptors supplied alongside them.
pub unsafe fn execute_submitted_draws(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    submit: &RenderSubmit,
    target: &SubmittedMainSceneTarget,
) -> Result<SubmittedDrawStats> {
    if !resolver.has_content_admission() {
        return Err("submitted draw execution requires validated effective-VFS admission".into());
    }
    for weather in &submit.weather_membership {
        resolver.read_admitted_used_content(&weather.weather_position_source_content)?;
        for variant in &weather.ordered_variant_content {
            for entity in std::iter::once(&variant.primary).chain(variant.fading.iter()) {
                for content in &entity.ordered_content {
                    resolver.read_admitted_used_content(content)?;
                }
            }
        }
    }
    let ordered = ordered_draws(submit)?;
    let mut stream_output = StreamOutputBuffers::default();
    let mut stats = SubmittedDrawStats {
        direct_visual_joins: submit.direct_visuals.len(),
        weather_joins: submit
            .weather_membership
            .iter()
            .map(|row| row.ordered_weather_instances.len())
            .sum(),
        ..SubmittedDrawStats::default()
    };
    for draw in ordered {
        match draw {
            OrderedDraw::Particle(packet) => {
                execute_draw(
                    gfx,
                    resolver,
                    &submit.ordered_scene_constant_buffers,
                    &packet.shader_program,
                    &packet.ordered_texture_bindings,
                    &packet.pipeline,
                    &packet.geometry,
                    &packet.ordered_constant_buffers,
                    Some(&packet.ordered_gpu_instance_records),
                    SubmittedOutput::AuthenticatedPacket(&packet.output_state),
                    target,
                    &mut stream_output,
                )?;
                stats.particle_draws += 1;
            }
            OrderedDraw::Entity(entity) => {
                // Shader/material/mesh identity is a VFS admission gate even
                // when the exact IA bytes are carried inline.
                resolver
                    .read_admitted_used_content(&entity.content_identity.resolved_entity_asset)?;
                resolver.read_admitted_used_content(&entity.content_identity.mesh)?;
                execute_draw(
                    gfx,
                    resolver,
                    &submit.ordered_scene_constant_buffers,
                    &entity.shader_program,
                    &entity.ordered_texture_bindings,
                    &entity.pipeline,
                    &entity.geometry,
                    &entity.ordered_constant_buffers,
                    None,
                    SubmittedOutput::AuthenticatedPacket(&entity.output_state),
                    target,
                    &mut stream_output,
                )?;
                stats.entity_draws += 1;
            }
        }
    }
    Ok(stats)
}

/// Execute XMAP20's presentation-independent particle snapshot into the
/// viewer-owned MainScene target.
///
/// Recorder SceneCB0, recorder projection CB1, viewport and attachment
/// identity are intentionally ignored. The caller supplies the exact current
/// XMAP03/XMAP09/XMAP10 buffers and target after their provider receipts have
/// been checked by `xmap20_effects`.
pub unsafe fn execute_xmap20_particles(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    prepared: &map_viewer::PreparedXmap20Frame,
    scene_constant_buffers: &[ConstantBufferBinding],
    viewer_projection_cb1: &ConstantBufferBinding,
    target: &SubmittedMainSceneTarget,
) -> Result<SubmittedDrawStats> {
    prepared.validate()?;
    if !resolver.has_content_admission() {
        return Err("XMAP20 execution requires validated effective-VFS admission".into());
    }
    validate_xmap20_provider_buffers(scene_constant_buffers, viewer_projection_cb1)?;
    let mut stream_output = StreamOutputBuffers::default();
    let mut stats = SubmittedDrawStats::default();
    for packet in &prepared.ordered_particles {
        let draw_constant_buffers = packet
            .ordered_constant_buffers
            .iter()
            .map(|binding| {
                if binding.slot == 1 {
                    viewer_projection_cb1.clone()
                } else {
                    binding.clone()
                }
            })
            .collect::<Vec<_>>();
        if draw_constant_buffers
            .iter()
            .filter(|binding| binding.slot == 1)
            .count()
            != 1
        {
            return Err("XMAP20 packet does not have one replaceable projection CB1".into());
        }
        execute_draw(
            gfx,
            resolver,
            scene_constant_buffers,
            &packet.shader_program,
            &packet.ordered_texture_bindings,
            &packet.pipeline,
            &packet.geometry,
            &draw_constant_buffers,
            Some(&packet.ordered_gpu_instance_records),
            SubmittedOutput::ViewerOwnedMainScene,
            target,
            &mut stream_output,
        )?;
        stats.particle_draws += 1;
    }
    Ok(stats)
}

fn validate_xmap20_provider_buffers(
    scene_constant_buffers: &[ConstantBufferBinding],
    projection: &ConstantBufferBinding,
) -> Result<()> {
    if scene_constant_buffers.len() != 1 {
        return Err("XMAP20 requires exactly one viewer-owned SceneCB0 binding".into());
    }
    let scene = &scene_constant_buffers[0];
    if scene.stage_mask_u8 != 0b11
        || scene.slot != 0
        || scene.byte_offset != 0
        || scene.byte_count != 416
        || scene.raw_bytes.len() != 416
        || scene.semantic_layout.trim().is_empty()
        || format!("{:x}", Sha256::digest(&scene.raw_bytes)) != scene.sha256
    {
        return Err("XMAP20 SceneCB0 is not the exact 416-byte VS/PS provider payload".into());
    }
    if projection.stage_mask_u8 != 0b1
        || projection.slot != 1
        || projection.byte_offset != 0
        || projection.byte_count != 64
        || projection.raw_bytes.len() != 64
        || projection.semantic_layout.trim().is_empty()
        || format!("{:x}", Sha256::digest(&projection.raw_bytes)) != projection.sha256
    {
        return Err("XMAP20 projection CB1 is not the exact viewer-camera 64-byte payload".into());
    }
    Ok(())
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
    output: SubmittedOutput<'_>,
    target: &SubmittedMainSceneTarget,
    stream_output: &mut StreamOutputBuffers,
) -> Result<()> {
    // The source key is not decorative provenance: exact VFS admission is
    // required before captured compiled bytecode may execute.
    resolver.read_admitted_used_content(&shader.source)?;
    match output {
        SubmittedOutput::AuthenticatedPacket(output) => bind_output(gfx, output, target)?,
        SubmittedOutput::ViewerOwnedMainScene => bind_viewer_owned_main_scene(gfx, target)?,
    }

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
    let texture_views = create_texture_views(gfx, resolver, textures, particle_records)?;
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
    // Avoid retaining replay-owned resources into a later pass. All objects
    // remain alive until after the immediate-context draw call has consumed
    // the bindings.
    clear_shader_bindings(context);
    Ok(())
}

#[derive(Default)]
struct StreamOutputBuffers {
    by_identity: BTreeMap<String, (u32, ComPtr<ID3D11Buffer>)>,
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

unsafe fn bind_output(
    gfx: &Gfx,
    output: &RenderOutputState,
    target: &SubmittedMainSceneTarget,
) -> Result<()> {
    let has_depth = validate_output_binding(output, target)?;
    let dsv = if has_depth {
        target.depth_dsv
    } else {
        ptr::null_mut()
    };
    if target.color_rtv.is_null() {
        return Err("submitted draw prepared opaque color RTV is null".into());
    }
    let context = gfx.context.as_raw();
    let rtvs = [target.color_rtv];
    (*context).OMSetRenderTargets(1, rtvs.as_ptr(), dsv);
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

unsafe fn bind_viewer_owned_main_scene(
    gfx: &Gfx,
    target: &SubmittedMainSceneTarget,
) -> Result<()> {
    if !matches!(
        target.color_identity.as_ref(),
        Some(PreparedRenderTarget::OpaqueRenderTarget { .. })
    ) || !matches!(
        target.depth_identity.as_ref(),
        Some(PreparedRenderTarget::OpaqueRenderTarget { .. })
    ) || target.color_rtv.is_null()
        || target.depth_dsv.is_null()
        || target.color_identity == target.depth_identity
        || target.color_width == 0
        || target.color_height == 0
        || target.color_format == 0
        || target.depth_format == 0
        || target.depth_width != target.color_width
        || target.depth_height != target.color_height
        || target.color_samples == 0
        || target.depth_samples != target.color_samples
        || target.depth_quality != target.color_quality
    {
        return Err("XMAP20 viewer-owned MainScene target identity/descriptor is invalid".into());
    }
    let context = gfx.context.as_raw();
    let rtvs = [target.color_rtv];
    (*context).OMSetRenderTargets(1, rtvs.as_ptr(), target.depth_dsv);
    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: target.color_width as f32,
        Height: target.color_height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    (*context).RSSetViewports(1, &viewport);
    let rect = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(target.color_width)?,
        bottom: i32::try_from(target.color_height)?,
    };
    (*context).RSSetScissorRects(1, &rect);
    Ok(())
}

fn validate_output_binding(
    output: &RenderOutputState,
    target: &SubmittedMainSceneTarget,
) -> Result<bool> {
    if output.ordered_color_targets.len() != 1 {
        return Err("submitted opaque draw requires exactly one color target".into());
    }
    let color = &output.ordered_color_targets[0];
    let color_identity = target
        .color_identity
        .as_ref()
        .ok_or("submitted-draw backend has no authenticated opaque color attachment identity")?;
    if !matches!(
        color_identity,
        PreparedRenderTarget::OpaqueRenderTarget { .. }
    ) {
        return Err("submitted-draw backend color identity is not an opaque attachment".into());
    }
    if &color.prepared_target != color_identity
        || color.persistent_content.is_some()
        || color.mip_level != 0
        || color.width != target.color_width
        || color.height != target.color_height
        || color.dxgi_format_u32 != target.color_format
        || color.sample_count != target.color_samples
        || color.sample_quality != target.color_quality
    {
        return Err("submitted draw opaque color binding disagrees with prepared target".into());
    }
    if let Some(depth) = &output.depth_target {
        let depth_identity = target.depth_identity.as_ref().ok_or(
            "submitted-draw backend has no authenticated opaque depth attachment identity",
        )?;
        if !matches!(
            depth_identity,
            PreparedRenderTarget::OpaqueRenderTarget { .. }
        ) {
            return Err("submitted-draw backend depth identity is not an opaque attachment".into());
        }
        if &depth.prepared_target != depth_identity
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
                "submitted draw opaque depth binding disagrees with prepared target".into(),
            );
        }
        Ok(true)
    } else {
        Ok(false)
    }
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

fn materialize_buffer(
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
            if format!("{:x}", Sha256::digest(range)) != *range_sha256 {
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

enum TextureResourceGpu {
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
    _resource: TextureResourceGpu,
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
            // Keep upload storage alive through Create*; it can be dropped once
            // the immutable/default resource has copied the initial data.
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
                _resource: resource,
                view: ComPtr::from_raw(view, "portable submitted texture view")?,
            })
        })
        .collect()
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
    // Any programmable stage retained from a preceding static map pass would
    // alter the draw. Null every stage before binding the exact carried
    // VS/PS plus each optional authenticated stage (and again after drawing).
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

    fn empty_submit() -> RenderSubmit {
        RenderSubmit {
            submit_index: 1,
            global_order: 1,
            presentation_qpc_ticks: 1,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_scene_constant_buffers: Vec::new(),
            ordered_static_map_programs: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
            tree_runtime: None,
            s1_shadow: None,
            particles: Vec::new(),
            entity_presentations: Vec::new(),
            particle_packet_set_sha256: "0".repeat(64),
            entities: Vec::new(),
            rail_construction_arrows: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: Vec::new(),
            direct_visuals: Vec::new(),
            tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }
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
    fn ordered_draw_execution_never_accepts_unknown_or_inferred_rows() {
        assert!(ordered_draws(&empty_submit())
            .expect("empty order")
            .is_empty());
        let mut submit = empty_submit();
        submit.ordered_draws.push(RenderOutputRef::Particle {
            effect_instance_id: 9,
            bucket: 2,
            packet_ordinal: 3,
            draw_order: 4,
        });
        assert!(ordered_draws(&submit)
            .expect_err("unknown ordered draw")
            .to_string()
            .contains("absent packet"));
    }

    #[test]
    fn duplicate_reference_marker_rejects_a_second_visit() {
        let mut seen = [false, false];
        assert!(mark_once(&mut seen, 0));
        assert!(!mark_once(&mut seen, 0));
        assert!(mark_once(&mut seen, 1));
        assert!(seen.into_iter().all(|value| value));
    }

    #[test]
    fn exact_opaque_binding_rejects_absent_or_substituted_ordinals() {
        let color_identity = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 4,
        };
        let depth_identity = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 9,
        };
        let output = RenderOutputState {
            ordered_color_targets: vec![replay_core::RenderTargetDesc {
                prepared_target: color_identity.clone(),
                width: 1280,
                height: 720,
                dxgi_format_u32: 10,
                mip_level: 0,
                sample_count: 4,
                sample_quality: 7,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: Some(replay_core::RenderTargetDesc {
                prepared_target: depth_identity.clone(),
                width: 1280,
                height: 720,
                dxgi_format_u32: 45,
                mip_level: 0,
                sample_count: 4,
                sample_quality: 7,
                persistent_content: None,
                exact_resource: None,
            }),
            viewport: replay_core::ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 1280.0f32.to_bits(),
                height_f32_bits: 720.0f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        let target = SubmittedMainSceneTarget {
            color_identity: Some(color_identity),
            depth_identity: Some(depth_identity),
            color_rtv: 1usize as *mut _,
            depth_dsv: 1usize as *mut _,
            color_width: 1280,
            color_height: 720,
            color_format: 10,
            color_samples: 4,
            color_quality: 7,
            depth_width: 1280,
            depth_height: 720,
            depth_format: 45,
            depth_samples: 4,
            depth_quality: 7,
        };
        assert!(validate_output_binding(&output, &target).unwrap());

        let mut absent = SubmittedMainSceneTarget {
            color_identity: None,
            ..target
        };
        assert!(validate_output_binding(&output, &absent)
            .unwrap_err()
            .to_string()
            .contains("no authenticated opaque color"));
        absent.color_identity = Some(PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 5,
        });
        assert!(validate_output_binding(&output, &absent)
            .unwrap_err()
            .to_string()
            .contains("disagrees"));
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
