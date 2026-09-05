use super::*;
use crate::d3d11_device::{
    native_d3d11_check_void_upload_device, native_d3d11_create_device, D3dPtr, NativeD3d11Device,
};
use replay_core::RenderOutputState;
use replay_core::{
    ConstantBufferBinding, DepthStencilAttachmentView, PreparedRenderTarget,
    RenderTargetAttachmentView, RenderTargetExactResource, RenderTargetInitialSource,
    RenderTargetViewDesc, StaticMapCommand, StaticMapOperation, StaticMapPassProgram,
    TextureResourceDimension,
};
use snapshot::sha256::Sha256;
use std::{collections::BTreeMap, mem, ptr, slice};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::um::d3d11::*;
use winapi::um::unknwnbase::IUnknown;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dReadback {
    pub width: u32,
    pub height: u32,
    pub dxgi_format_u32: u32,
    pub tight_row_pitch: u32,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub struct NativeProductionD3dBackend {
    device: NativeD3d11Device,
    color_texture: D3dPtr<ID3D11Texture2D>,
    color_rtv: D3dPtr<ID3D11RenderTargetView>,
    _depth_texture: D3dPtr<ID3D11Texture2D>,
    depth_dsv: D3dPtr<ID3D11DepthStencilView>,
    presentation: Option<NativePresentationTarget>,
    desc: ProductionMainSceneTargetDesc,
}

struct NativePresentationTarget {
    rtv: D3dPtr<ID3D11RenderTargetView>,
    width: u32,
    height: u32,
    format: u32,
    samples: u32,
    quality: u32,
}

struct NativeGraphColorTarget {
    texture: NativeGraphTexture,
    rtv: D3dPtr<ID3D11RenderTargetView>,
    desc: replay_core::RenderTargetDesc,
}

struct NativeGraphDepthTarget {
    _texture: NativeGraphTexture,
    dsv: D3dPtr<ID3D11DepthStencilView>,
    desc: replay_core::RenderTargetDesc,
}

enum NativeGraphTexture {
    Texture1d(D3dPtr<ID3D11Texture1D>),
    Texture2d(D3dPtr<ID3D11Texture2D>),
    Texture3d(D3dPtr<ID3D11Texture3D>),
}

impl NativeGraphTexture {
    fn as_resource(&self) -> *mut ID3D11Resource {
        match self {
            Self::Texture1d(texture) => texture.as_raw() as *mut _,
            Self::Texture2d(texture) => texture.as_raw() as *mut _,
            Self::Texture3d(texture) => texture.as_raw() as *mut _,
        }
    }
}

pub struct NativeProductionD3dExecutor<'a> {
    backend: &'a mut NativeProductionD3dBackend,
    resolver: &'a crate::GameFileResolver,
    resources: &'a BTreeMap<String, ProductionMapResource>,
    gfx: super::native_draw::Gfx,
    stream_output: super::native_draw::StreamOutputBuffers,
    snow_mud_texture: Option<replay_core::SnowMudTexturePacket>,
    snow_mud_proof: Option<SnowMudD3dExecutionProof>,
    snow_mud_binding_count: u32,
    collect_snow_mud_h2: bool,
    occupation_secondary_proof: Option<OccupationSecondaryD3dExecutionProof>,
    occupation_terrain_t6_binding_count: u32,
    occupation_water_t3_binding_count: u32,
    authenticated_textures: Vec<super::native_draw::AuthenticatedTextureGpu>,
    political_textures: Vec<replay_core::PoliticalTexturePacket>,
    graph_colors: BTreeMap<PreparedRenderTarget, NativeGraphColorTarget>,
    graph_depths: BTreeMap<PreparedRenderTarget, NativeGraphDepthTarget>,
}

impl<'a> NativeProductionD3dExecutor<'a> {
    pub fn new(
        backend: &'a mut NativeProductionD3dBackend,
        resolver: &'a crate::GameFileResolver,
        resources: &'a BTreeMap<String, ProductionMapResource>,
    ) -> Result<Self> {
        Self::new_with_snow_mud_h2(backend, resolver, resources, false)
    }

    pub fn new_with_snow_mud_h2(
        backend: &'a mut NativeProductionD3dBackend,
        resolver: &'a crate::GameFileResolver,
        resources: &'a BTreeMap<String, ProductionMapResource>,
        collect_snow_mud_h2: bool,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err("native production D3D execution requires an admission-bound VFS".into());
        }
        let gfx = super::native_draw::Gfx::borrowed(backend.device(), backend.context())?;
        Ok(Self {
            backend,
            resolver,
            resources,
            gfx,
            stream_output: Default::default(),
            snow_mud_texture: None,
            snow_mud_proof: None,
            snow_mud_binding_count: 0,
            collect_snow_mud_h2,
            occupation_secondary_proof: None,
            occupation_terrain_t6_binding_count: 0,
            occupation_water_t3_binding_count: 0,
            authenticated_textures: Vec::new(),
            political_textures: Vec::new(),
            graph_colors: BTreeMap::new(),
            graph_depths: BTreeMap::new(),
        })
    }

    pub fn snow_mud_execution_proof(&self) -> Option<&SnowMudD3dExecutionProof> {
        self.snow_mud_proof.as_ref()
    }

    pub fn occupation_secondary_execution_proof(
        &self,
    ) -> Option<&OccupationSecondaryD3dExecutionProof> {
        self.occupation_secondary_proof.as_ref()
    }

    fn submitted_target(
        &self,
        output: &RenderOutputState,
    ) -> Result<(
        super::native_draw::SubmittedMainSceneTarget,
        Vec<super::native_draw::SubmittedColorTarget>,
    )> {
        if output.ordered_color_targets.is_empty() {
            return Err("draw has no color target".into());
        }
        if output
            .ordered_color_targets
            .iter()
            .any(|target| target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer)
        {
            if output.ordered_color_targets.len() != 1 || output.depth_target.is_some() {
                return Err(
                    "swapchain output must be the sole carried color target without depth".into(),
                );
            }
            return Ok((self.backend.submitted_target(output)?, Vec::new()));
        }
        let mut ordered_colors = Vec::with_capacity(output.ordered_color_targets.len());
        for color in &output.ordered_color_targets {
            let prepared = match &color.prepared_target {
                PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                    if *attachment_ordinal == self.backend.desc().color_attachment_ordinal
                        && color.exact_resource.is_none() =>
                {
                    validate_color_target_desc(color, self.backend.desc())?;
                    super::native_draw::SubmittedColorTarget {
                        identity: color.prepared_target.clone(),
                        rtv: self.backend.color_rtv(),
                        width: color.width,
                        height: color.height,
                        format: color.dxgi_format_u32,
                        samples: color.sample_count,
                        quality: color.sample_quality,
                    }
                }
                identity => {
                    let graph = self
                        .graph_colors
                        .get(identity)
                        .filter(|graph| graph.desc == *color)
                        .ok_or("carried MRT output references an absent render-graph target")?;
                    super::native_draw::SubmittedColorTarget {
                        identity: color.prepared_target.clone(),
                        rtv: graph.rtv.as_raw(),
                        width: color.width,
                        height: color.height,
                        format: color.dxgi_format_u32,
                        samples: color.sample_count,
                        quality: color.sample_quality,
                    }
                }
            };
            ordered_colors.push(prepared);
        }
        let (
            depth_identity,
            depth_dsv,
            depth_width,
            depth_height,
            depth_format,
            depth_samples,
            depth_quality,
        ) = if let Some(depth) = &output.depth_target {
            let dsv = match &depth.prepared_target {
                PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                    if *attachment_ordinal == self.backend.desc().depth_attachment_ordinal
                        && depth.exact_resource.is_none() =>
                {
                    validate_depth_target_desc(depth, self.backend.desc())?;
                    self.backend.depth_dsv()
                }
                identity => self
                    .graph_depths
                    .get(identity)
                    .filter(|graph| graph.desc == *depth)
                    .ok_or("carried output references an absent render-graph depth target")?
                    .dsv
                    .as_raw(),
            };
            (
                Some(depth.prepared_target.clone()),
                dsv,
                depth.width,
                depth.height,
                depth.dxgi_format_u32,
                depth.sample_count,
                depth.sample_quality,
            )
        } else {
            (None, ptr::null_mut(), 0, 0, 0, 0, 0)
        };
        let primary = ordered_colors.remove(0);
        Ok((
            super::native_draw::SubmittedMainSceneTarget {
                color_identity: primary.identity,
                depth_identity,
                color_rtv: primary.rtv,
                depth_dsv,
                color_width: primary.width,
                color_height: primary.height,
                color_format: primary.format,
                color_samples: primary.samples,
                color_quality: primary.quality,
                depth_width,
                depth_height,
                depth_format,
                depth_samples,
                depth_quality,
            },
            ordered_colors,
        ))
    }

    fn prepared_textures(
        &self,
        draw: &replay_core::StaticMapDraw,
    ) -> Result<Vec<super::native_draw::PreparedTargetTexture>> {
        draw.ordered_shader_resources
            .iter()
            .filter_map(|binding| match &binding.source {
                replay_core::StaticShaderResourceSource::PreparedTarget {
                    target,
                    producer,
                    ..
                } => Some((target, producer)),
                _ => None,
            })
            .map(|(target, producer)| {
                let resource = match target {
                    PreparedRenderTarget::SwapchainBackBuffer => {
                        return Err(
                            "prepared-target SRV cannot name the swapchain backbuffer".into()
                        )
                    }
                    identity => {
                        if let Some(value) = self.graph_colors.get(identity) {
                            value.texture.as_resource()
                        } else if matches!(
                            identity,
                            PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                                if *attachment_ordinal
                                    == self.backend.desc().color_attachment_ordinal
                        ) {
                            self.backend.color_texture() as *mut ID3D11Resource
                        } else {
                            return Err(
                                "prepared-target SRV names an absent render-graph texture".into()
                            );
                        }
                    }
                };
                Ok(super::native_draw::PreparedTargetTexture {
                    target: target.clone(),
                    producer: producer.clone(),
                    resource,
                })
            })
            .collect()
    }

    fn ensure_graph_color(
        &mut self,
        target: &replay_core::RenderTargetDesc,
        expected_consumer: Option<&str>,
    ) -> Result<()> {
        let PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } =
            &target.prepared_target
        else {
            return Ok(());
        };
        if *attachment_ordinal == self.backend.desc().color_attachment_ordinal
            && target.exact_resource.is_none()
        {
            validate_color_target_desc(target, self.backend.desc())?;
            return Ok(());
        }
        validate_exact_target_consumer_binding(target, self.resources, expected_consumer)?;
        if let Some(existing) = self.graph_colors.get(&target.prepared_target) {
            if existing.desc != *target {
                return Err("opaque target descriptor changes within frame".into());
            }
            return Ok(());
        }
        let native = self.backend.create_graph_color(
            self.resolver,
            self.resources,
            expected_consumer,
            target,
        )?;
        self.graph_colors
            .insert(target.prepared_target.clone(), native);
        Ok(())
    }

    fn ensure_graph_depth(
        &mut self,
        target: &replay_core::RenderTargetDesc,
        expected_consumer: Option<&str>,
    ) -> Result<()> {
        let PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } =
            &target.prepared_target
        else {
            return Err("depth target cannot be the swapchain backbuffer".into());
        };
        if *attachment_ordinal == self.backend.desc().depth_attachment_ordinal
            && target.exact_resource.is_none()
        {
            validate_depth_target_desc(target, self.backend.desc())?;
            return Ok(());
        }
        validate_exact_target_consumer_binding(target, self.resources, expected_consumer)?;
        if let Some(existing) = self.graph_depths.get(&target.prepared_target) {
            if existing.desc != *target {
                return Err("opaque depth-target descriptor changes within frame".into());
            }
            return Ok(());
        }
        let native = self.backend.create_graph_depth(
            self.resolver,
            self.resources,
            expected_consumer,
            target,
        )?;
        self.graph_depths
            .insert(target.prepared_target.clone(), native);
        Ok(())
    }
}

impl ProductionD3dExecutionBridge for NativeProductionD3dExecutor<'_> {
    fn begin_frame(&mut self, frame: &PreparedProductionD3dFrame) -> Result<()> {
        self.authenticated_textures.clear();
        for packet in &frame.authenticated_texture_snapshots {
            self.authenticated_textures.push(unsafe {
                super::native_draw::create_authenticated_snapshot_texture_gpu(
                    &self.gfx,
                    self.resolver,
                    packet,
                )?
            });
        }
        self.political_textures = frame.political_textures.clone();
        self.snow_mud_texture = frame.snow_mud_texture.clone();
        self.snow_mud_proof = None;
        self.snow_mud_binding_count = 0;
        self.occupation_secondary_proof = None;
        self.occupation_terrain_t6_binding_count = 0;
        self.occupation_water_t3_binding_count = 0;
        self.graph_colors.clear();
        self.graph_depths.clear();
        for program in &frame.ordered_static_programs {
            let expected_consumer =
                format!("production_map.static_program/{}", program.payload_sha256);
            for operation in &program.ordered_operations {
                match &operation.command {
                    StaticMapCommand::ClearColor { target, .. } => {
                        self.ensure_graph_color(target, Some(&expected_consumer))?;
                    }
                    StaticMapCommand::Draw { draw } => {
                        for target in &draw.output_state.ordered_color_targets {
                            self.ensure_graph_color(target, Some(&expected_consumer))?;
                        }
                        if let Some(target) = &draw.output_state.depth_target {
                            self.ensure_graph_depth(target, Some(&expected_consumer))?;
                        }
                    }
                    StaticMapCommand::ClearDepthStencil { target, .. } => {
                        self.ensure_graph_depth(target, Some(&expected_consumer))?;
                    }
                }
            }
        }
        if let Some(m03) = &frame.m03_arrows {
            for packet in &m03.draws {
                for target in &packet.draw.output_state.ordered_color_targets {
                    self.ensure_graph_color(target, None)?;
                }
                if let Some(target) = &packet.draw.output_state.depth_target {
                    self.ensure_graph_depth(target, None)?;
                }
            }
        }
        for draw in &frame.ordered_runtime_draws {
            let output = match draw {
                PreparedRuntimeDraw::Particle { packet, .. } => &packet.output_state,
                PreparedRuntimeDraw::Entity { submission, .. }
                | PreparedRuntimeDraw::O04City { submission, .. }
                | PreparedRuntimeDraw::O05Building { submission, .. } => &submission.output_state,
            };
            for target in &output.ordered_color_targets {
                self.ensure_graph_color(target, None)?;
            }
            if let Some(target) = &output.depth_target {
                self.ensure_graph_depth(target, None)?;
            }
        }
        match frame.target.as_ref() {
            Some(target) if target == self.backend.desc() => Ok(()),
            Some(_) => Err("prepared frame target differs from native allocation".into()),
            None => Err("native production D3D frame has no authenticated opaque target".into()),
        }
    }

    fn execute_static_operation(
        &mut self,
        _global_ordinal: u32,
        _pass: &StaticMapPassProgram,
        _ordered_scene_constant_buffers: &[ConstantBufferBinding],
        operation: &StaticMapOperation,
    ) -> Result<()> {
        match &operation.command {
            StaticMapCommand::ClearColor {
                target,
                rgba_f32_bits,
            } => {
                let rgba = rgba_f32_bits.map(f32::from_bits);
                if !rgba.into_iter().all(f32::is_finite) {
                    return Err("static color clear contains non-finite component".into());
                }
                let rtv = match &target.prepared_target {
                    PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                        if *attachment_ordinal == self.backend.desc().color_attachment_ordinal
                            && target.exact_resource.is_none() =>
                    {
                        validate_color_target_desc(target, self.backend.desc())?;
                        self.backend.color_rtv()
                    }
                    PreparedRenderTarget::SwapchainBackBuffer => {
                        let presentation =
                            self.backend.presentation.as_ref().ok_or(
                                "carried presentation clear has no attached app back buffer",
                            )?;
                        if target.width != presentation.width
                            || target.height != presentation.height
                            || target.dxgi_format_u32 != presentation.format
                            || target.sample_count != presentation.samples
                            || target.sample_quality != presentation.quality
                            || target.mip_level != 0
                            || target.persistent_content.is_some()
                        {
                            return Err(
                                "carried presentation clear descriptor differs from the attached app back buffer"
                                    .into(),
                            );
                        }
                        presentation.rtv.as_raw()
                    }
                    identity => self
                        .graph_colors
                        .get(identity)
                        .filter(|value| value.desc == *target)
                        .ok_or("clear references an absent render-graph color target")?
                        .rtv
                        .as_raw(),
                };
                unsafe {
                    (*self.backend.context()).ClearRenderTargetView(rtv, &rgba);
                }
                Ok(())
            }
            StaticMapCommand::ClearDepthStencil {
                target,
                clear_depth,
                clear_stencil,
                depth_f32_bits,
                stencil_u8,
            } => {
                let dsv = match &target.prepared_target {
                    PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                        if *attachment_ordinal == self.backend.desc().depth_attachment_ordinal
                            && target.exact_resource.is_none() =>
                    {
                        validate_depth_target_desc(target, self.backend.desc())?;
                        self.backend.depth_dsv()
                    }
                    PreparedRenderTarget::SwapchainBackBuffer => {
                        return Err("depth clear cannot reference the swapchain backbuffer".into())
                    }
                    identity => self
                        .graph_depths
                        .get(identity)
                        .filter(|value| value.desc == *target)
                        .ok_or("depth clear references an absent render-graph depth target")?
                        .dsv
                        .as_raw(),
                };
                let depth = f32::from_bits(*depth_f32_bits);
                if !depth.is_finite() || !(0.0..=1.0).contains(&depth) {
                    return Err("static depth clear is non-finite or outside [0,1]".into());
                }
                let mut flags = 0;
                if *clear_depth {
                    flags |= D3D11_CLEAR_DEPTH;
                }
                if *clear_stencil {
                    flags |= D3D11_CLEAR_STENCIL;
                }
                unsafe {
                    (*self.backend.context()).ClearDepthStencilView(dsv, flags, depth, *stencil_u8);
                }
                Ok(())
            }
            StaticMapCommand::Draw { draw } => {
                let (target, additional_colors) = self.submitted_target(&draw.output_state)?;
                let prepared_targets = self.prepared_textures(draw)?;
                unsafe {
                    super::native_draw::execute_static_exact_used_draw_mrt(
                        &self.gfx,
                        self.resolver,
                        _ordered_scene_constant_buffers,
                        draw,
                        self.snow_mud_texture.as_ref(),
                        &mut self.snow_mud_proof,
                        &mut self.snow_mud_binding_count,
                        self.collect_snow_mud_h2,
                        &mut self.occupation_secondary_proof,
                        &mut self.occupation_terrain_t6_binding_count,
                        &mut self.occupation_water_t3_binding_count,
                        &self.authenticated_textures,
                        &self.political_textures,
                        &prepared_targets,
                        &target,
                        &additional_colors,
                        &mut self.stream_output,
                    )
                }
            }
        }
    }

    fn execute_runtime_draw(
        &mut self,
        _global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        draw: &PreparedRuntimeDraw,
    ) -> Result<()> {
        let output = match draw {
            PreparedRuntimeDraw::Particle { packet, .. } => &packet.output_state,
            PreparedRuntimeDraw::Entity { submission, .. }
            | PreparedRuntimeDraw::O04City { submission, .. }
            | PreparedRuntimeDraw::O05Building { submission, .. } => &submission.output_state,
        };
        let (target, additional_colors) = self.submitted_target(output)?;
        unsafe {
            super::native_draw::execute_prepared_runtime_draw_mrt(
                &self.gfx,
                self.resolver,
                ordered_scene_constant_buffers,
                draw,
                &target,
                &additional_colors,
                &mut self.stream_output,
            )
        }
    }

    fn execute_m03_draw(
        &mut self,
        _global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        packet: &replay_core::M03ArrowDraw,
    ) -> Result<()> {
        let (target, additional_colors) = self.submitted_target(&packet.draw.output_state)?;
        let prepared_targets = self.prepared_textures(&packet.draw)?;
        unsafe {
            super::native_draw::execute_static_exact_used_draw_mrt(
                &self.gfx,
                self.resolver,
                ordered_scene_constant_buffers,
                &packet.draw,
                self.snow_mud_texture.as_ref(),
                &mut self.snow_mud_proof,
                &mut self.snow_mud_binding_count,
                self.collect_snow_mud_h2,
                &mut self.occupation_secondary_proof,
                &mut self.occupation_terrain_t6_binding_count,
                &mut self.occupation_water_t3_binding_count,
                &self.authenticated_textures,
                &self.political_textures,
                &prepared_targets,
                &target,
                &additional_colors,
                &mut self.stream_output,
            )
        }
    }

    fn finish_frame(&mut self) -> Result<()> {
        match (&self.snow_mud_texture, self.snow_mud_binding_count) {
            (None, 0) => {}
            (Some(packet), count) if count == packet.snow_mud_fade_consumer_count => {}
            (Some(_), _) => {
                return Err(
                    "product SnowMudTexture binding/fade consumer cardinality mismatch".into(),
                )
            }
            (None, _) => return Err("product SnowMudTexture binding has no H packet".into()),
        }
        match (
            self.collect_snow_mud_h2,
            &self.snow_mud_texture,
            &self.snow_mud_proof,
        ) {
            (false, _, None) | (true, None, None) => {}
            (true, Some(packet), Some(proof))
                if proof.replaced_binding_count == packet.snow_mud_fade_consumer_count => {}
            _ => return Err("product SnowMudTexture H2 proof collection is incomplete".into()),
        }
        let mut occupation_packets = self.political_textures.iter().filter(|packet| {
            packet.kind == replay_core::PoliticalTextureKind::Secondary
                && packet.target_binding_identity
                    == replay_core::OCCUPATION_SECONDARY_TARGET_BINDING
        });
        let occupation_packet = occupation_packets.next();
        if occupation_packets.next().is_some() {
            return Err("product has repeated occupation secondary packets".into());
        }
        match (
            occupation_packet,
            self.occupation_terrain_t6_binding_count,
            self.occupation_water_t3_binding_count,
        ) {
            (None, 0, 0) => {}
            (Some(_), terrain, water) if terrain > 0 && water > 0 => {}
            _ => return Err("product occupation secondary binding topology is incomplete".into()),
        }
        match (
            self.collect_snow_mud_h2,
            occupation_packet,
            &self.occupation_secondary_proof,
        ) {
            (false, _, None) | (true, None, None) => {}
            (true, Some(_), Some(proof))
                if proof.terrain_t6_binding_count == self.occupation_terrain_t6_binding_count
                    && proof.water_t3_binding_count == self.occupation_water_t3_binding_count => {}
            _ => {
                return Err("product occupation secondary H2 proof collection is incomplete".into())
            }
        }
        unsafe { (*self.backend.context()).Flush() };
        Ok(())
    }
}

impl NativeProductionD3dBackend {
    fn create_graph_color(
        &self,
        resolver: &crate::GameFileResolver,
        resources: &BTreeMap<String, ProductionMapResource>,
        expected_consumer: Option<&str>,
        desc: &replay_core::RenderTargetDesc,
    ) -> Result<NativeGraphColorTarget> {
        if let Some(exact) = &desc.exact_resource {
            let (texture, rtv) =
                self.create_exact_color_target(resolver, resources, expected_consumer, exact)?;
            return Ok(NativeGraphColorTarget {
                texture,
                rtv,
                desc: desc.clone(),
            });
        }
        unsafe {
            let native_desc = texture_desc(
                desc.width,
                desc.height,
                desc.dxgi_format_u32,
                desc.sample_count,
                desc.sample_quality,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
                0,
            );
            let texture =
                create_texture(&self.device, &native_desc, "portable render-graph color")?;
            let mut rtv = ptr::null_mut();
            hr(
                (*self.device.device.as_raw()).CreateRenderTargetView(
                    texture.as_raw() as *mut _,
                    ptr::null(),
                    &mut rtv,
                ),
                "CreateRenderTargetView(portable render graph)",
            )?;
            Ok(NativeGraphColorTarget {
                texture: NativeGraphTexture::Texture2d(texture),
                rtv: D3dPtr::from_raw(rtv, "portable render-graph RTV")?,
                desc: desc.clone(),
            })
        }
    }

    fn create_graph_depth(
        &self,
        resolver: &crate::GameFileResolver,
        resources: &BTreeMap<String, ProductionMapResource>,
        expected_consumer: Option<&str>,
        desc: &replay_core::RenderTargetDesc,
    ) -> Result<NativeGraphDepthTarget> {
        if let Some(exact) = &desc.exact_resource {
            let (texture, dsv) =
                self.create_exact_depth_target(resolver, resources, expected_consumer, exact)?;
            return Ok(NativeGraphDepthTarget {
                _texture: texture,
                dsv,
                desc: desc.clone(),
            });
        }
        unsafe {
            let native_desc = texture_desc(
                desc.width,
                desc.height,
                desc.dxgi_format_u32,
                desc.sample_count,
                desc.sample_quality,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_DEPTH_STENCIL,
                0,
            );
            let texture =
                create_texture(&self.device, &native_desc, "portable render-graph depth")?;
            let mut dsv = ptr::null_mut();
            hr(
                (*self.device.device.as_raw()).CreateDepthStencilView(
                    texture.as_raw() as *mut _,
                    ptr::null(),
                    &mut dsv,
                ),
                "CreateDepthStencilView(portable render graph)",
            )?;
            Ok(NativeGraphDepthTarget {
                _texture: NativeGraphTexture::Texture2d(texture),
                dsv: D3dPtr::from_raw(dsv, "portable render-graph DSV")?,
                desc: desc.clone(),
            })
        }
    }

    fn create_exact_color_target(
        &self,
        resolver: &crate::GameFileResolver,
        resources: &BTreeMap<String, ProductionMapResource>,
        expected_consumer: Option<&str>,
        exact: &RenderTargetExactResource,
    ) -> Result<(NativeGraphTexture, D3dPtr<ID3D11RenderTargetView>)> {
        unsafe {
            let texture = create_exact_graph_texture(
                &self.device,
                resolver,
                resources,
                expected_consumer,
                exact,
            )?;
            let view = exact
                .ordered_views
                .iter()
                .find_map(|view| match view {
                    RenderTargetViewDesc::RenderTarget {
                        dxgi_format_u32,
                        view,
                    } => Some((*dxgi_format_u32, view)),
                    _ => None,
                })
                .ok_or("exact color target has no RTV descriptor")?;
            let native = render_target_view_desc(view.0, view.1)?;
            let mut rtv = ptr::null_mut();
            hr(
                (*self.device.device.as_raw()).CreateRenderTargetView(
                    texture.as_resource(),
                    &native,
                    &mut rtv,
                ),
                "CreateRenderTargetView(exact render graph)",
            )?;
            Ok((
                texture,
                D3dPtr::from_raw(rtv, "exact portable render-graph RTV")?,
            ))
        }
    }

    fn create_exact_depth_target(
        &self,
        resolver: &crate::GameFileResolver,
        resources: &BTreeMap<String, ProductionMapResource>,
        expected_consumer: Option<&str>,
        exact: &RenderTargetExactResource,
    ) -> Result<(NativeGraphTexture, D3dPtr<ID3D11DepthStencilView>)> {
        unsafe {
            let texture = create_exact_graph_texture(
                &self.device,
                resolver,
                resources,
                expected_consumer,
                exact,
            )?;
            let view = exact
                .ordered_views
                .iter()
                .find_map(|view| match view {
                    RenderTargetViewDesc::DepthStencil {
                        dxgi_format_u32,
                        flags_u32,
                        view,
                    } => Some((*dxgi_format_u32, *flags_u32, view)),
                    _ => None,
                })
                .ok_or("exact depth target has no DSV descriptor")?;
            let native = depth_stencil_view_desc(view.0, view.1, view.2)?;
            let mut dsv = ptr::null_mut();
            hr(
                (*self.device.device.as_raw()).CreateDepthStencilView(
                    texture.as_resource(),
                    &native,
                    &mut dsv,
                ),
                "CreateDepthStencilView(exact render graph)",
            )?;
            Ok((
                texture,
                D3dPtr::from_raw(dsv, "exact portable render-graph DSV")?,
            ))
        }
    }

    pub fn create_offscreen(desc: ProductionMainSceneTargetDesc) -> Result<Self> {
        desc.validate()?;
        unsafe {
            let device = native_d3d11_create_device()?;
            Self::create_with_owned_device(device, desc)
        }
    }

    /// The backend AddRefs both interfaces and releases them on drop.
    pub unsafe fn create_on_device(
        device: *mut ID3D11Device,
        context: *mut ID3D11DeviceContext,
        desc: ProductionMainSceneTargetDesc,
    ) -> Result<Self> {
        desc.validate()?;
        if device.is_null() || context.is_null() {
            return Err("cannot create production D3D backend from null app interfaces".into());
        }
        (*(device as *mut IUnknown)).AddRef();
        (*(context as *mut IUnknown)).AddRef();
        let owned = NativeD3d11Device {
            device: D3dPtr::from_raw(device, "app D3D11 device")?,
            context: D3dPtr::from_raw(context, "app D3D11 context")?,
        };
        Self::create_with_owned_device(owned, desc)
    }

    unsafe fn create_with_owned_device(
        device: NativeD3d11Device,
        desc: ProductionMainSceneTargetDesc,
    ) -> Result<Self> {
        let color_desc = texture_desc(
            desc.color_width,
            desc.color_height,
            desc.color_format,
            desc.color_samples,
            desc.color_quality,
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            0,
        );
        let color_texture = create_texture(&device, &color_desc, "opaque production color")?;
        let mut color_rtv = ptr::null_mut();
        hr(
            (*device.device.as_raw()).CreateRenderTargetView(
                color_texture.as_raw() as *mut _,
                ptr::null(),
                &mut color_rtv,
            ),
            "CreateRenderTargetView(opaque production color)",
        )?;
        let color_rtv = D3dPtr::from_raw(color_rtv, "opaque production color RTV")?;

        let depth_desc = texture_desc(
            desc.depth_width,
            desc.depth_height,
            desc.depth_format,
            desc.depth_samples,
            desc.depth_quality,
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_DEPTH_STENCIL,
            0,
        );
        let depth_texture = create_texture(&device, &depth_desc, "opaque production depth")?;
        let mut depth_dsv = ptr::null_mut();
        hr(
            (*device.device.as_raw()).CreateDepthStencilView(
                depth_texture.as_raw() as *mut _,
                ptr::null(),
                &mut depth_dsv,
            ),
            "CreateDepthStencilView(opaque production depth)",
        )?;
        let depth_dsv = D3dPtr::from_raw(depth_dsv, "opaque production depth DSV")?;
        Ok(Self {
            device,
            color_texture,
            color_rtv,
            _depth_texture: depth_texture,
            depth_dsv,
            presentation: None,
            desc,
        })
    }

    pub unsafe fn attach_presentation_target(
        &mut self,
        rtv: *mut ID3D11RenderTargetView,
        width: u32,
        height: u32,
        format: u32,
        samples: u32,
        quality: u32,
    ) -> Result<()> {
        if rtv.is_null() || width == 0 || height == 0 || samples == 0 {
            return Err("presentation target descriptor is null or empty".into());
        }
        (*(rtv as *mut IUnknown)).AddRef();
        self.presentation = Some(NativePresentationTarget {
            rtv: D3dPtr::from_raw(rtv, "app presentation RTV")?,
            width,
            height,
            format,
            samples,
            quality,
        });
        Ok(())
    }

    fn submitted_target(
        &self,
        output: &RenderOutputState,
    ) -> Result<super::native_draw::SubmittedMainSceneTarget> {
        let color = output
            .ordered_color_targets
            .first()
            .ok_or("draw has no color target")?;
        if output.ordered_color_targets.len() != 1 {
            return Err(
                "direct production backend target supports one carried color target".into(),
            );
        }
        match &color.prepared_target {
            PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal }
                if *attachment_ordinal == self.desc.color_attachment_ordinal =>
            {
                validate_output_for_target(output, &self.desc)?;
                Ok(super::native_draw::SubmittedMainSceneTarget {
                    color_identity: color.prepared_target.clone(),
                    depth_identity: output
                        .depth_target
                        .as_ref()
                        .map(|target| target.prepared_target.clone()),
                    color_rtv: self.color_rtv(),
                    depth_dsv: self.depth_dsv(),
                    color_width: self.desc.color_width,
                    color_height: self.desc.color_height,
                    color_format: self.desc.color_format,
                    color_samples: self.desc.color_samples,
                    color_quality: self.desc.color_quality,
                    depth_width: self.desc.depth_width,
                    depth_height: self.desc.depth_height,
                    depth_format: self.desc.depth_format,
                    depth_samples: self.desc.depth_samples,
                    depth_quality: self.desc.depth_quality,
                })
            }
            PreparedRenderTarget::SwapchainBackBuffer => {
                if output.depth_target.is_some() {
                    return Err("carried presentation draw cannot infer a depth attachment".into());
                }
                let target = self
                    .presentation
                    .as_ref()
                    .ok_or("carried presentation draw has no attached app back buffer")?;
                Ok(super::native_draw::SubmittedMainSceneTarget {
                    color_identity: PreparedRenderTarget::SwapchainBackBuffer,
                    depth_identity: None,
                    color_rtv: target.rtv.as_raw(),
                    depth_dsv: ptr::null_mut(),
                    color_width: target.width,
                    color_height: target.height,
                    color_format: target.format,
                    color_samples: target.samples,
                    color_quality: target.quality,
                    depth_width: 0,
                    depth_height: 0,
                    depth_format: 0,
                    depth_samples: 0,
                    depth_quality: 0,
                })
            }
            PreparedRenderTarget::OpaqueRenderTarget { .. } => {
                Err("carried draw requires an unmaterialized opaque color attachment".into())
            }
        }
    }

    pub fn desc(&self) -> &ProductionMainSceneTargetDesc {
        &self.desc
    }

    pub fn device(&self) -> *mut ID3D11Device {
        self.device.device.as_raw()
    }

    pub fn context(&self) -> *mut ID3D11DeviceContext {
        self.device.context.as_raw()
    }

    pub fn color_rtv(&self) -> *mut ID3D11RenderTargetView {
        self.color_rtv.as_raw()
    }

    pub fn depth_dsv(&self) -> *mut ID3D11DepthStencilView {
        self.depth_dsv.as_raw()
    }

    pub fn color_texture(&self) -> *mut ID3D11Texture2D {
        self.color_texture.as_raw()
    }

    pub fn clear(&mut self, rgba: [f32; 4], depth: f32, stencil: u8) -> Result<()> {
        if !rgba.into_iter().all(f32::is_finite)
            || !depth.is_finite()
            || !(0.0..=1.0).contains(&depth)
        {
            return Err("MainScene clear contains non-finite color or invalid depth".into());
        }
        unsafe {
            (*self.device.context.as_raw()).ClearRenderTargetView(self.color_rtv.as_raw(), &rgba);
            (*self.device.context.as_raw()).ClearDepthStencilView(
                self.depth_dsv.as_raw(),
                D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL,
                depth,
                stencil,
            );
        }
        Ok(())
    }

    pub fn bind_output_state(&mut self, output: &RenderOutputState) -> Result<()> {
        super::native_draw::validate_output_viewport_scissor(output)?;
        validate_output_for_target(output, &self.desc)?;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: f32::from_bits(output.viewport.top_left_x_f32_bits),
            TopLeftY: f32::from_bits(output.viewport.top_left_y_f32_bits),
            Width: f32::from_bits(output.viewport.width_f32_bits),
            Height: f32::from_bits(output.viewport.height_f32_bits),
            MinDepth: f32::from_bits(output.viewport.min_depth_f32_bits),
            MaxDepth: f32::from_bits(output.viewport.max_depth_f32_bits),
        };
        unsafe {
            let rtvs = [self.color_rtv.as_raw()];
            (*self.device.context.as_raw()).OMSetRenderTargets(
                1,
                rtvs.as_ptr(),
                self.depth_dsv.as_raw(),
            );
            (*self.device.context.as_raw()).RSSetViewports(1, &viewport);
            if let Some(value) = &output.scissor {
                let rect = winapi::shared::windef::RECT {
                    left: value.left_i32,
                    top: value.top_i32,
                    right: value.right_i32,
                    bottom: value.bottom_i32,
                };
                (*self.device.context.as_raw()).RSSetScissorRects(1, &rect);
            } else {
                (*self.device.context.as_raw()).RSSetScissorRects(0, ptr::null());
            }
        }
        Ok(())
    }

    pub unsafe fn copy_main_scene_to_proof_texture(
        &mut self,
        destination: *mut ID3D11Texture2D,
    ) -> Result<()> {
        if destination.is_null() {
            return Err("proof destination texture is null".into());
        }
        let mut destination_desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
        (*destination).GetDesc(&mut destination_desc);
        if destination_desc.Width != self.desc.color_width
            || destination_desc.Height != self.desc.color_height
            || destination_desc.Format as u32 != self.desc.color_format
            || destination_desc.SampleDesc.Count != 1
        {
            return Err("proof destination is incompatible with MainScene".into());
        }
        if self.desc.color_samples == 1 {
            (*self.device.context.as_raw())
                .CopyResource(destination as *mut _, self.color_texture.as_raw() as *mut _);
        } else {
            (*self.device.context.as_raw()).ResolveSubresource(
                destination as *mut _,
                0,
                self.color_texture.as_raw() as *mut _,
                0,
                self.desc.color_format as _,
            );
        }
        Ok(())
    }

    pub fn readback(&mut self) -> Result<ProductionD3dReadback> {
        unsafe {
            let row_bytes = self
                .desc
                .color_width
                .checked_mul(bytes_per_pixel(self.desc.color_format)?)
                .ok_or("MainScene readback row size overflow")?;
            let source = if self.desc.color_samples == 1 {
                self.color_texture.as_raw()
            } else {
                let resolve_desc = texture_desc(
                    self.desc.color_width,
                    self.desc.color_height,
                    self.desc.color_format,
                    1,
                    0,
                    D3D11_USAGE_DEFAULT,
                    0,
                    0,
                );
                let resolve = create_texture(&self.device, &resolve_desc, "MainScene resolve")?;
                (*self.device.context.as_raw()).ResolveSubresource(
                    resolve.as_raw() as *mut _,
                    0,
                    self.color_texture.as_raw() as *mut _,
                    0,
                    self.desc.color_format as _,
                );
                return readback_texture(
                    &self.device,
                    resolve.as_raw(),
                    self.desc.color_width,
                    self.desc.color_height,
                    self.desc.color_format,
                    row_bytes,
                );
            };
            readback_texture(
                &self.device,
                source,
                self.desc.color_width,
                self.desc.color_height,
                self.desc.color_format,
                row_bytes,
            )
        }
    }
}

fn validate_output_for_target(
    output: &RenderOutputState,
    target: &ProductionMainSceneTargetDesc,
) -> Result<()> {
    let color = output
        .ordered_color_targets
        .first()
        .ok_or("draw output does not bind a color attachment")?;
    if output.ordered_color_targets.len() != 1 {
        return Err("opaque production draw must bind exactly one color attachment".into());
    }
    let expected_color = PreparedRenderTarget::OpaqueRenderTarget {
        attachment_ordinal: target.color_attachment_ordinal,
    };
    if color.prepared_target != expected_color {
        return Err("draw output does not bind the allocated opaque color attachment".into());
    }
    let depth = output
        .depth_target
        .as_ref()
        .ok_or("draw output does not bind the allocated opaque depth attachment")?;
    let expected_depth = PreparedRenderTarget::OpaqueRenderTarget {
        attachment_ordinal: target.depth_attachment_ordinal,
    };
    if depth.prepared_target != expected_depth {
        return Err("draw output does not bind the allocated opaque depth attachment".into());
    }
    validate_color_target_desc(color, target)?;
    validate_depth_target_desc(depth, target)?;
    let actual = ProductionMainSceneTargetDesc {
        color_attachment_ordinal: target.color_attachment_ordinal,
        color_width: color.width,
        color_height: color.height,
        color_format: color.dxgi_format_u32,
        color_samples: color.sample_count,
        color_quality: color.sample_quality,
        depth_attachment_ordinal: target.depth_attachment_ordinal,
        depth_width: depth.width,
        depth_height: depth.height,
        depth_format: depth.dxgi_format_u32,
        depth_samples: depth.sample_count,
        depth_quality: depth.sample_quality,
    };
    if &actual != target {
        return Err("draw output descriptor differs from allocated opaque target".into());
    }
    Ok(())
}

fn validate_color_target_desc(
    target: &replay_core::RenderTargetDesc,
    main: &ProductionMainSceneTargetDesc,
) -> Result<()> {
    if target.width != main.color_width
        || target.height != main.color_height
        || target.dxgi_format_u32 != main.color_format
        || target.mip_level != 0
        || target.sample_count != main.color_samples
        || target.sample_quality != main.color_quality
        || target.persistent_content.is_some()
        || target.prepared_target
            != (PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: main.color_attachment_ordinal,
            })
    {
        return Err("static opaque color descriptor differs from native target".into());
    }
    Ok(())
}

fn validate_depth_target_desc(
    target: &replay_core::RenderTargetDesc,
    main: &ProductionMainSceneTargetDesc,
) -> Result<()> {
    if target.width != main.depth_width
        || target.height != main.depth_height
        || target.dxgi_format_u32 != main.depth_format
        || target.mip_level != 0
        || target.sample_count != main.depth_samples
        || target.sample_quality != main.depth_quality
        || target.persistent_content.is_some()
        || target.prepared_target
            != (PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: main.depth_attachment_ordinal,
            })
    {
        return Err("static opaque depth descriptor differs from native target".into());
    }
    Ok(())
}

fn validate_exact_target_consumer_binding(
    target: &replay_core::RenderTargetDesc,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: Option<&str>,
) -> Result<()> {
    let Some(expected_consumer) = expected_consumer else {
        return Ok(());
    };
    if let Some(content) = &target.persistent_content {
        require_exact_consumer_binding(content, resources, expected_consumer)?;
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                require_exact_consumer_binding(content, resources, expected_consumer)?;
            }
        }
    }
    Ok(())
}

fn require_exact_consumer_binding<'a>(
    content: &replay_core::UsedContentKey,
    resources: &'a BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<&'a ProductionMapResource> {
    let mut matches = resources.values().filter(|resource| {
        resource.content == *content
            && resource
                .consumers
                .iter()
                .any(|consumer| consumer == expected_consumer)
    });
    let resource = matches.next().ok_or_else(|| {
        format!(
            "exact render-target initialization {} is not bound to sealed consumer {expected_consumer}",
            content.normalized_game_relative_path
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "exact render-target initialization {} has ambiguous sealed consumer bindings",
            content.normalized_game_relative_path
        )
        .into());
    }
    if u64::try_from(resource.bytes.len())? != content.payload_bytes
        || Sha256::hex(&resource.bytes) != content.payload_sha256
    {
        return Err(format!(
            "exact render-target initialization {} differs from its sealed consumer binding",
            content.normalized_game_relative_path
        )
        .into());
    }
    Ok(resource)
}

fn materialize_render_target_initial(
    resolver: &crate::GameFileResolver,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: Option<&str>,
    source: &RenderTargetInitialSource,
) -> Result<Vec<u8>> {
    match source {
        RenderTargetInitialSource::Inline { bytes, sha256 } => {
            if Sha256::hex(bytes) != *sha256 {
                return Err("exact render-target inline initialization SHA-256 mismatch".into());
            }
            Ok(bytes.clone())
        }
        RenderTargetInitialSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            let admitted_payload;
            let payload: &[u8] = match expected_consumer {
                Some(expected_consumer) => {
                    let binding =
                        require_exact_consumer_binding(content, resources, expected_consumer)?;
                    &binding.bytes
                }
                None => {
                    admitted_payload = resolver.read_admitted_used_content(content)?;
                    &admitted_payload
                }
            };
            let start = usize::try_from(*source_byte_offset)?;
            let count = usize::try_from(*byte_count)?;
            let end = start
                .checked_add(count)
                .ok_or("exact render-target content range overflows")?;
            let bytes = payload
                .get(start..end)
                .ok_or("exact render-target content range exceeds admitted payload")?;
            if Sha256::hex(bytes) != *range_sha256 {
                return Err("exact render-target content-range SHA-256 mismatch".into());
            }
            Ok(bytes.to_vec())
        }
    }
}

unsafe fn create_exact_graph_texture(
    device: &NativeD3d11Device,
    resolver: &crate::GameFileResolver,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: Option<&str>,
    exact: &RenderTargetExactResource,
) -> Result<NativeGraphTexture> {
    let resource = &exact.resource;
    let storage = exact
        .ordered_initial_subresources
        .iter()
        .map(|initial| {
            materialize_render_target_initial(
                resolver,
                resources,
                expected_consumer,
                &initial.source,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    if !exact.ordered_initial_subresources.is_empty() && resource.mip_levels == 0 {
        return Err("exact render-target initialization requires an explicit mip count".into());
    }
    for (initial, bytes) in exact.ordered_initial_subresources.iter().zip(&storage) {
        let mip_level = initial.subresource_ordinal % resource.mip_levels;
        super::native_draw::validate_texture_subresource_payload(
            resource.dimension,
            resource.dxgi_format_u32,
            resource.width,
            resource.height,
            resource.depth,
            mip_level,
            initial.row_pitch,
            initial.slice_pitch,
            bytes.len(),
        )?;
    }
    let subresource_count = resource
        .mip_levels
        .checked_mul(resource.array_size)
        .ok_or("exact render-target subresource count overflows")?;
    let complete_initialization = exact.ordered_initial_subresources.len()
        == usize::try_from(subresource_count)?
        && exact
            .ordered_initial_subresources
            .iter()
            .enumerate()
            .all(|(ordinal, initial)| initial.subresource_ordinal == ordinal as u32);
    let init = exact
        .ordered_initial_subresources
        .iter()
        .zip(&storage)
        .map(|(initial, bytes)| D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: initial.row_pitch,
            SysMemSlicePitch: initial.slice_pitch,
        })
        .collect::<Vec<_>>();
    if resource.usage_u32 == D3D11_USAGE_IMMUTABLE && !complete_initialization {
        return Err("immutable exact render target lacks complete ordered initialization".into());
    }
    let initial_data = if complete_initialization {
        init.as_ptr()
    } else {
        ptr::null()
    };

    let texture = match resource.dimension {
        TextureResourceDimension::Texture1d => {
            let native = D3D11_TEXTURE1D_DESC {
                Width: resource.width,
                MipLevels: resource.mip_levels,
                ArraySize: resource.array_size,
                Format: resource.dxgi_format_u32 as _,
                Usage: resource.usage_u32,
                BindFlags: resource.bind_flags_u32,
                CPUAccessFlags: resource.cpu_access_flags_u32,
                MiscFlags: resource.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr(
                (*device.device.as_raw()).CreateTexture1D(&native, initial_data, &mut texture),
                "CreateTexture1D(exact render graph)",
            )?;
            NativeGraphTexture::Texture1d(D3dPtr::from_raw(
                texture,
                "exact portable render-graph Texture1D",
            )?)
        }
        TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube => {
            let native = D3D11_TEXTURE2D_DESC {
                Width: resource.width,
                Height: resource.height,
                MipLevels: resource.mip_levels,
                ArraySize: resource.array_size,
                Format: resource.dxgi_format_u32 as _,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: resource.sample_count,
                    Quality: resource.sample_quality,
                },
                Usage: resource.usage_u32,
                BindFlags: resource.bind_flags_u32,
                CPUAccessFlags: resource.cpu_access_flags_u32,
                MiscFlags: resource.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr(
                (*device.device.as_raw()).CreateTexture2D(&native, initial_data, &mut texture),
                "CreateTexture2D(exact render graph)",
            )?;
            NativeGraphTexture::Texture2d(D3dPtr::from_raw(
                texture,
                "exact portable render-graph Texture2D",
            )?)
        }
        TextureResourceDimension::Texture3d => {
            let native = D3D11_TEXTURE3D_DESC {
                Width: resource.width,
                Height: resource.height,
                Depth: resource.depth,
                MipLevels: resource.mip_levels,
                Format: resource.dxgi_format_u32 as _,
                Usage: resource.usage_u32,
                BindFlags: resource.bind_flags_u32,
                CPUAccessFlags: resource.cpu_access_flags_u32,
                MiscFlags: resource.misc_flags_u32,
            };
            let mut texture = ptr::null_mut();
            hr(
                (*device.device.as_raw()).CreateTexture3D(&native, initial_data, &mut texture),
                "CreateTexture3D(exact render graph)",
            )?;
            NativeGraphTexture::Texture3d(D3dPtr::from_raw(
                texture,
                "exact portable render-graph Texture3D",
            )?)
        }
    };

    if !complete_initialization && !exact.ordered_initial_subresources.is_empty() {
        if resource.usage_u32 != D3D11_USAGE_DEFAULT {
            return Err(
                "sparse exact render-target initialization requires D3D11_USAGE_DEFAULT".into(),
            );
        }
        for (initial, bytes) in exact.ordered_initial_subresources.iter().zip(&storage) {
            (*device.context.as_raw()).UpdateSubresource(
                texture.as_resource(),
                initial.subresource_ordinal,
                ptr::null(),
                bytes.as_ptr() as *const _,
                initial.row_pitch,
                initial.slice_pitch,
            );
            native_d3d11_check_void_upload_device(
                &device.device,
                &format!(
                    "UpdateSubresource sparse exact render-graph {:?} subresource {}",
                    resource.dimension, initial.subresource_ordinal
                ),
            )?;
        }
    }
    Ok(texture)
}

#[cfg(test)]
mod exact_target_consumer_tests {
    use super::*;
    use replay_core::{ContentLayerKind, RequiredContentKind, UsedContentKey};

    fn fixture() -> (UsedContentKey, BTreeMap<String, ProductionMapResource>) {
        let bytes = (0..64).map(|value| value as u8).collect::<Vec<_>>();
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            stored_game_relative_path: "gfx/exact/native-initial.bin".into(),
            normalized_game_relative_path: "gfx/exact/native-initial.bin".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: Sha256::hex(&bytes),
            payload_sha256: Sha256::hex(&bytes),
            payload_bytes: bytes.len() as u64,
        };
        let resource = ProductionMapResource {
            content_key: "exact.native.initial".into(),
            kind: RequiredContentKind::Texture,
            content: content.clone(),
            bytes,
            consumers: vec!["production_map.static_program/sealed-program".into()],
        };
        (
            content,
            [(resource.content_key.clone(), resource)]
                .into_iter()
                .collect(),
        )
    }

    #[test]
    fn sparse_update_device_failure_is_typed_and_fail_closed() {
        let error = crate::d3d11_device::native_d3d11_validate_void_upload_result(
            0x887A_0005u32 as i32,
            "UpdateSubresource sparse exact render-graph Texture2d subresource 3",
        )
        .expect_err("device removal must reject sparse exact initialization");
        assert!(error
            .downcast_ref::<crate::d3d11_device::NativeD3d11UploadError>()
            .is_some());
        let message = error.to_string();
        assert!(message.contains("subresource 3"), "{message}");
        assert!(
            message.contains("uploaded state was not admitted"),
            "{message}"
        );
    }

    #[test]
    fn sealed_initialization_materializes_only_from_its_exact_consumer_binding() {
        let (content, resources) = fixture();
        let resolver = crate::GameFileResolver::vanilla(std::env::temp_dir()).unwrap();
        let expected = resources["exact.native.initial"].bytes[8..40].to_vec();
        let source = RenderTargetInitialSource::UsedContentRange {
            content,
            source_byte_offset: 8,
            byte_count: 32,
            range_sha256: Sha256::hex(&expected),
        };
        assert_eq!(
            materialize_render_target_initial(
                &resolver,
                &resources,
                Some("production_map.static_program/sealed-program"),
                &source,
            )
            .unwrap(),
            expected
        );
        let error = materialize_render_target_initial(
            &resolver,
            &resources,
            Some("production_map.static_program/other-program"),
            &source,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not bound to sealed consumer"), "{error}");
    }

    #[test]
    fn sealed_initialization_rejects_changed_or_ambiguous_binding_bytes() {
        let (content, mut resources) = fixture();
        resources.get_mut("exact.native.initial").unwrap().bytes[0] ^= 0xff;
        let error = require_exact_consumer_binding(
            &content,
            &resources,
            "production_map.static_program/sealed-program",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("differs from its sealed consumer binding"),
            "{error}"
        );

        let (_, mut resources) = fixture();
        let duplicate = resources["exact.native.initial"].clone();
        resources.insert("exact.native.initial.duplicate".into(), duplicate);
        let error = require_exact_consumer_binding(
            &content,
            &resources,
            "production_map.static_program/sealed-program",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("ambiguous sealed consumer bindings"),
            "{error}"
        );
    }
}

unsafe fn render_target_view_desc(
    format: u32,
    view: &RenderTargetAttachmentView,
) -> Result<D3D11_RENDER_TARGET_VIEW_DESC> {
    let mut desc: D3D11_RENDER_TARGET_VIEW_DESC = mem::zeroed();
    desc.Format = format as _;
    match view {
        RenderTargetAttachmentView::Texture1d { mip_slice } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE1D;
            *desc.u.Texture1D_mut() = D3D11_TEX1D_RTV {
                MipSlice: *mip_slice,
            };
        }
        RenderTargetAttachmentView::Texture1dArray {
            mip_slice,
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE1DARRAY;
            *desc.u.Texture1DArray_mut() = D3D11_TEX1D_ARRAY_RTV {
                MipSlice: *mip_slice,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        RenderTargetAttachmentView::Texture2d { mip_slice } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2D;
            *desc.u.Texture2D_mut() = D3D11_TEX2D_RTV {
                MipSlice: *mip_slice,
            };
        }
        RenderTargetAttachmentView::Texture2dArray {
            mip_slice,
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2DARRAY;
            *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_RTV {
                MipSlice: *mip_slice,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        RenderTargetAttachmentView::Texture2dMultisampled => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2DMS;
        }
        RenderTargetAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY;
            *desc.u.Texture2DMSArray_mut() = D3D11_TEX2DMS_ARRAY_RTV {
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        RenderTargetAttachmentView::Texture3d {
            mip_slice,
            first_w_slice,
            w_size,
        } => {
            desc.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE3D;
            *desc.u.Texture3D_mut() = D3D11_TEX3D_RTV {
                MipSlice: *mip_slice,
                FirstWSlice: *first_w_slice,
                WSize: *w_size,
            };
        }
    }
    Ok(desc)
}

unsafe fn depth_stencil_view_desc(
    format: u32,
    flags: u32,
    view: &DepthStencilAttachmentView,
) -> Result<D3D11_DEPTH_STENCIL_VIEW_DESC> {
    let mut desc: D3D11_DEPTH_STENCIL_VIEW_DESC = mem::zeroed();
    desc.Format = format as _;
    desc.Flags = flags;
    match view {
        DepthStencilAttachmentView::Texture1d { mip_slice } => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE1D;
            *desc.u.Texture1D_mut() = D3D11_TEX1D_DSV {
                MipSlice: *mip_slice,
            };
        }
        DepthStencilAttachmentView::Texture1dArray {
            mip_slice,
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE1DARRAY;
            *desc.u.Texture1DArray_mut() = D3D11_TEX1D_ARRAY_DSV {
                MipSlice: *mip_slice,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        DepthStencilAttachmentView::Texture2d { mip_slice } => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2D;
            *desc.u.Texture2D_mut() = D3D11_TEX2D_DSV {
                MipSlice: *mip_slice,
            };
        }
        DepthStencilAttachmentView::Texture2dArray {
            mip_slice,
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2DARRAY;
            *desc.u.Texture2DArray_mut() = D3D11_TEX2D_ARRAY_DSV {
                MipSlice: *mip_slice,
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
        DepthStencilAttachmentView::Texture2dMultisampled => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2DMS;
        }
        DepthStencilAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => {
            desc.ViewDimension = D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY;
            *desc.u.Texture2DMSArray_mut() = D3D11_TEX2DMS_ARRAY_DSV {
                FirstArraySlice: *first_array_slice,
                ArraySize: *array_size,
            };
        }
    }
    Ok(desc)
}

unsafe fn create_texture(
    device: &NativeD3d11Device,
    desc: &D3D11_TEXTURE2D_DESC,
    role: &str,
) -> Result<D3dPtr<ID3D11Texture2D>> {
    let mut texture = ptr::null_mut();
    hr(
        (*device.device.as_raw()).CreateTexture2D(desc, ptr::null(), &mut texture),
        &format!("CreateTexture2D({role})"),
    )?;
    D3dPtr::from_raw(texture, role)
}

fn texture_desc(
    width: u32,
    height: u32,
    format: u32,
    samples: u32,
    quality: u32,
    usage: D3D11_USAGE,
    bind_flags: u32,
    cpu_access_flags: u32,
) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: samples,
            Quality: quality,
        },
        Usage: usage,
        BindFlags: bind_flags,
        CPUAccessFlags: cpu_access_flags,
        MiscFlags: 0,
    }
}

unsafe fn readback_texture(
    device: &NativeD3d11Device,
    source: *mut ID3D11Texture2D,
    width: u32,
    height: u32,
    format: u32,
    row_bytes: u32,
) -> Result<ProductionD3dReadback> {
    let staging_desc = texture_desc(
        width,
        height,
        format,
        1,
        0,
        D3D11_USAGE_STAGING,
        0,
        D3D11_CPU_ACCESS_READ,
    );
    let staging = create_texture(device, &staging_desc, "MainScene staging")?;
    (*device.context.as_raw()).CopyResource(staging.as_raw() as *mut _, source as *mut _);
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    hr(
        (*device.context.as_raw()).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        ),
        "Map(MainScene staging)",
    )?;
    let result = (|| {
        if mapped.RowPitch < row_bytes {
            return Err("D3D11 staging row pitch is smaller than tight row size".into());
        }
        let total = usize::try_from(row_bytes)?
            .checked_mul(usize::try_from(height)?)
            .ok_or("MainScene readback byte size overflow")?;
        let mut bytes = Vec::with_capacity(total);
        for row in 0..height as usize {
            let start = (mapped.pData as *const u8).add(row * mapped.RowPitch as usize);
            bytes.extend_from_slice(slice::from_raw_parts(start, row_bytes as usize));
        }
        Ok::<_, Box<dyn std::error::Error>>(ProductionD3dReadback {
            width,
            height,
            dxgi_format_u32: format,
            tight_row_pitch: row_bytes,
            sha256: Sha256::hex(&bytes),
            bytes,
        })
    })();
    (*device.context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
    result
}

fn bytes_per_pixel(format: u32) -> Result<u32> {
    match format {
        2 => Ok(16),
        10 => Ok(8),
        28 | 29 | 87 | 88 | 91 => Ok(4),
        _ => Err(format!("MainScene readback does not support DXGI format {format}").into()),
    }
}

fn hr(value: i32, role: &str) -> Result<()> {
    if value < 0 {
        Err(format!("{role} failed with HRESULT 0x{:08x}", value as u32).into())
    } else {
        Ok(())
    }
}
