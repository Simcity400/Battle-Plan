//! Native sky, after map text and immediately before post.
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{
    ResolvedLayers,
    image::native::Decode,
    pdxshader::{DepthStencilState, RasterizerState, program::Stage},
};
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!("Map shader sky: 1 refused pass: {}", reason.into()))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    // Fixed sky effect and technique selection.
    library.load(b"gfx/FX/sky.shader", &defines).map_err(refuse)
}

pub struct Sky {
    pipeline: Arc<gpu::Pipeline>,
    vertices: wgpu::Buffer,
}

impl Sky {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, samples, format, depth)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let selection = effect
            .select("sky")
            .map_err(refuse)?
            .ok_or_else(|| refuse("sky technique absent; retained program unavailable"))?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        // updates only physical PS texture/sampler0. Other registers retain.
        state
            .samplers
            .retain(|slot, _| *slot == binding(0, Stage::Pixel));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                if !matches!(input.semantic.as_str(), "POSITION" | "POSITION0")
                    || input.kind != ScalarKind::Sint
                    || input.components > 2
                {
                    return Err(refuse(format!("native sky stream cannot supply {input:?}")));
                }
                Ok(wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Sint16x2,
                    offset: 0,
                    shader_location: input.location,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        pass.vertices = vec![gpu::VertexLayout {
            stride: 4,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        // Sky enables depth with LESS_EQUAL, disables depth writes and stencil, and restores clockwise
        // renderer raster state with back-face culling.
        let raster = RasterizerState::default();
        let depth_state = DepthStencilState {
            depth_write_mask: 0,
            ..Default::default()
        };
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil = Some(gpu::depth_stencil(&depth_state, &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::scene(&pipeline).map_err(refuse)?;
        // Upload three signed 16-bit coordinate pairs at stride 4.
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native sky triangle"),
            contents: bytemuck::cast_slice(&[[3_i16, -1], [-1, -1], [-1, 3]]),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Ok(Self { pipeline, vertices })
    }

    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        frame: &[u8],
        reflection: &wgpu::TextureView,
    ) -> io::Result<()> {
        // Build a fresh 416-byte scene b0 for both stages. Unknown scene padding and extra retained
        // registers refuse by name.
        let buffers = BTreeMap::from([
            (binding(0, Stage::Vertex), frame.to_vec()),
            (binding(0, Stage::Pixel), frame.to_vec()),
        ]);
        let textures = BTreeMap::from([(binding(0, Stage::Pixel), reflection.clone())]);
        let bindings = self
            .pipeline
            .bindings(arena, &buffers, &textures)
            .map_err(refuse)?;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled sky"),
            color_attachments: &[Some(target.color)],
            depth_stencil_attachment: Some(target.depth),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        bindings.bind(&mut pass);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
