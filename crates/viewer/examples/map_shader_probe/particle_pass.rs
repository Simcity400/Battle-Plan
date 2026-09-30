//! Independent direct-binding particle fixture; runtime uses Stream.
use super::{compiled::layouts, mesh_shader, shader_constants};
use eframe::wgpu;
use installed_content::pdxshader::{DepthStencilState, program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};
fn fail(reason: impl ToString) -> io::Error {
    io::Error::other(reason.to_string())
}
pub struct Pass {
    pipeline: Arc<gpu::Pipeline>,
}

impl Pass {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        name: &str,
        samples: u32,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        Self::for_packet(device, effect, name, samples, format, depth_format, false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn for_packet(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        name: &str,
        samples: u32,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        trail: bool,
    ) -> io::Result<Self> {
        let selection = effect
            .select(name)
            .map_err(fail)?
            .ok_or_else(|| fail(format!("{name}: retained missing technique unavailable")))?;
        let technique = &effect.source.techniques[selection.index];
        let compiled = effect.compiled(selection.index).map_err(fail)?;
        let mut state = gpu::State::authored(&effect.source, technique).map_err(fail)?;
        //14227FD35 replaces only PS0;140B47790 supplied map PS4..13 earlier.
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        state.samplers.extend(mesh_shader::map_samplers());
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        pass.vertices = layouts(&compiled.vertex.inputs, trail)?;
        let raster = effect.source.rasterizer(technique);
        let topology = if trail {
            wgpu::PrimitiveTopology::TriangleStrip
        } else {
            wgpu::PrimitiveTopology::TriangleList
        };
        pass.primitive = gpu::primitive(&raster, topology).0;
        //14227EEB0 constructs LEQUAL, no write/stencil;14227F420 binds it
        //independently of the authored program/blend/raster.
        let depth = DepthStencilState {
            depth_enable: true,
            depth_write_mask: 0,
            depth_function: 3,
            stencil_enable: false,
            ..Default::default()
        };
        pass.depth_stencil = Some(gpu::depth_stencil(&depth, &raster, depth_format).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(fail)?;
        shader_constants::scene(&pipeline).map_err(fail)?;
        shader_constants::both(&pipeline, 1, 64, std::slice::from_ref(&(0..64))).map_err(fail)?;
        shader_constants::both(&pipeline, 2, 16, std::slice::from_ref(&(0..8))).map_err(fail)?;
        pipeline
            .check_initialized(
                binding(3, Stage::Vertex),
                96,
                std::slice::from_ref(&(0..84)),
            )
            .map_err(fail)?;
        Ok(Self { pipeline })
    }

    pub fn bindings(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        buffers: &BTreeMap<u32, Vec<u8>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> io::Result<gpu::Bindings> {
        self.pipeline
            .bindings(arena, buffers, textures)
            .map_err(fail)
    }

    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(0);
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}
