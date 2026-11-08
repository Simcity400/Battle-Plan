//! Border uses authored program/blend/samplers and forced map depth/raster.
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

use eframe::wgpu;
use installed_content_core::{
    ResolvedLayers, pdxshader::DepthStencilState, pdxshader::RasterizerState,
    pdxshader::program::Stage,
};
use installed_content_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader borders: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/border.shader", &defines)
        .map_err(refuse)
}

pub struct Target<'a> {
    pub color: wgpu::RenderPassColorAttachment<'a>,
    pub depth: wgpu::RenderPassDepthStencilAttachment<'a>,
}

pub struct Inputs<'a> {
    pub retained: Option<&'a RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub snow_mud: &'a wgpu::TextureView,
    pub intel: &'a wgpu::TextureView,
    pub shadow: &'a wgpu::TextureView,
    pub time: f32,
}

pub struct Run<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub count: u32,
}

pub struct Batch<'a> {
    pub texture: &'a wgpu::TextureView,
    pub alpha: f32,
    /// Palette color for this native reason group within a material bucket.
    pub selection: [f32; 4],
    pub runs: Vec<Run<'a>>,
}

pub struct Border {
    retained: super::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
}

impl Border {
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
            .select("border")
            .map_err(refuse)?
            .ok_or_else(|| refuse("border technique absent; retained program unavailable"))?;
        let program = &effect.source;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(program, &program.techniques[selection.index]).map_err(refuse)?;
        state
            .samplers
            .retain(|slot, _| (0..4).any(|i| *slot == binding(i, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                let (offset, format) = match (input.semantic.as_str(), input.kind, input.components)
                {
                    ("POSITION" | "POSITION0", ScalarKind::Float, 1..=4) => {
                        (0, wgpu::VertexFormat::Float32x3)
                    }
                    ("TEXCOORD" | "TEXCOORD0", ScalarKind::Sint, 1..=4) => {
                        (12, wgpu::VertexFormat::Sint16x2)
                    }
                    _ => {
                        return Err(refuse(format!(
                            "native vertex stream cannot supply {input:?}"
                        )));
                    }
                };
                Ok(wgpu::VertexAttribute {
                    format,
                    offset,
                    shader_location: input.location,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        pass.vertices = vec![gpu::VertexLayout {
            stride: 16,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        // Use counterclockwise fronts and enabled depth testing without depth writes.
        let raster = RasterizerState {
            front_ccw: true,
            ..Default::default()
        };
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
        pipeline
            .check_initialized(binding(2, Stage::Pixel), 32, std::slice::from_ref(&(0..24)))
            .map_err(refuse)?;
        let retained = super::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &depth_state,
        );
        Ok(Self { retained, pipeline })
    }

    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: Target<'_>,
        inputs: Inputs<'_>,
        batches: &[Batch<'_>],
        visible: bool,
    ) -> io::Result<super::shader_constants::WorldPixelConstants> {
        // Uploads before the strip-count check, even for no draws. A hidden bank keeps every
        // retained input it would have left for the passes after it and submits nothing.
        let mut retained =
            super::shader_constants::WorldPixelConstants::border(inputs.time, [0.; 4], 1.);
        if let Some(state) = inputs.retained {
            let mut state = state.borrow_mut();
            self.retained.select(&mut state);
            retained.retain(&mut state);
        }
        let mut bound = Vec::new();
        for frame in inputs.frames {
            for batch in batches {
                let textures = [batch.texture, inputs.snow_mud, inputs.intel, inputs.shadow]
                    .into_iter()
                    .enumerate()
                    .map(|(slot, view)| (binding(slot as u32, Stage::Pixel), view.clone()))
                    .collect();
                if let Some(state) = inputs.retained {
                    self.retained
                        .bind_textures(&mut state.borrow_mut(), &textures);
                }
                if batch.runs.is_empty() {
                    continue;
                }
                retained = super::shader_constants::WorldPixelConstants::border(
                    inputs.time,
                    batch.selection,
                    batch.alpha,
                );
                let buffers = BTreeMap::from([
                    (binding(0, Stage::Vertex), frame.to_vec()),
                    (binding(0, Stage::Pixel), frame.to_vec()),
                    (binding(2, Stage::Pixel), retained.bytes().to_vec()),
                ]);
                if let Some(retained) = inputs.retained {
                    let mut tracked = retained.borrow_mut();
                    self.retained.bind(&mut tracked, &buffers, &textures);
                    super::shader_constants::WorldPixelConstants::border(
                        inputs.time,
                        batch.selection,
                        batch.alpha,
                    )
                    .retain(&mut tracked);
                }
                if !visible {
                    continue;
                }
                bound.push((
                    batch,
                    self.pipeline
                        .bindings(arena, &buffers, &textures)
                        .map_err(refuse)?,
                ));
            }
        }
        if !visible {
            return Ok(retained);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled borders"),
            color_attachments: &[Some(target.color)],
            depth_stencil_attachment: Some(target.depth),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        let mut previous = None;
        for (batch, binding) in &bound {
            binding.bind_after(&mut pass, previous);
            previous = Some(binding);
            for run in &batch.runs {
                shader_pipeline::gpu::stats::state();
                pass.set_vertex_buffer(0, run.vertices.slice(..));
                shader_pipeline::gpu::stats::state();
                pass.set_index_buffer(run.indices.slice(..), wgpu::IndexFormat::Uint32);
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(0..run.count, 0, 0..1);
            }
        }
        Ok(retained)
    }
}
