//! River submission with resolved effects and retained render state.
use std::cell::RefCell;
use std::{collections::BTreeMap, io, ops::Range, sync::Arc};

use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::native::Decode,
    pdxshader::{DepthStencilState, RasterizerState, program::Stage},
};
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader rivers: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/river.shader", &defines)
        .map_err(refuse)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub indices: Range<u32>,
    pub direction: f32,
}

pub struct Inputs<'a> {
    pub retained: Option<&'a RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: &'a [[f32; 4]; 4],
    pub time: f32,
    /// Actual retained PS b2 allocation, including its initialized-byte ranges.
    pub retained_pixel_b2: super::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub struct Geometry<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub runs: &'a [Run],
}

pub struct River {
    retained: super::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
}

impl River {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        high: bool,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, samples, format, depth, high)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        high: bool,
    ) -> io::Result<Self> {
        // Select river quality from map-rendering options.
        let technique = if high { "river" } else { "river_low_gfx" };
        let selection = effect
            .select(technique)
            .map_err(refuse)?
            .ok_or_else(|| refuse(format!("{technique} absent; retained program unavailable")))?;
        let program = &effect.source;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(program, &program.techniques[selection.index]).map_err(refuse)?;
        state
            .samplers
            .retain(|slot, _| (0..16).any(|i| *slot == binding(i, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                let offset = match (input.semantic.as_str(), input.kind, input.components) {
                    ("POSITION" | "POSITION0", ScalarKind::Float, 1..=4) => 0,
                    ("TEXCOORD" | "TEXCOORD0", ScalarKind::Float, 1..=4) => 16,
                    _ => {
                        return Err(refuse(format!(
                            "native vertex stream cannot supply {input:?}"
                        )));
                    }
                };
                Ok(wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset,
                    shader_location: input.location,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        pass.vertices = vec![gpu::VertexLayout {
            stride: 32,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        // Restore raster state immediately before rivers and force the river depth state.
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
        super::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..76)))
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
        target: super::border_shader::Target<'_>,
        inputs: Inputs<'_>,
        geometry: Geometry<'_>,
    ) -> io::Result<()> {
        inputs
            .retained_pixel_b2
            .check(&self.pipeline)
            .map_err(refuse)?;
        let textures = inputs
            .textures
            .iter()
            .enumerate()
            .map(|(slot, view)| (binding(slot as u32, Stage::Pixel), (*view).clone()))
            .collect();
        let mut bound = Vec::new();
        for frame in inputs.frames {
            for run in geometry.runs {
                // The 80-byte constant allocation carries the shadow matrix, time, direction and zero season.
                // Bytes 76 through 79 remain unknown.
                let mut constants = bytemuck::bytes_of(inputs.shadow_matrix).to_vec();
                constants.extend_from_slice(bytemuck::cast_slice(&[
                    inputs.time,
                    run.direction,
                    0.0,
                    0.0,
                ]));
                let buffers = BTreeMap::from([
                    (binding(0, Stage::Vertex), frame.to_vec()),
                    (binding(0, Stage::Pixel), frame.to_vec()),
                    (binding(1, Stage::Vertex), constants.clone()),
                    (binding(1, Stage::Pixel), constants),
                    (
                        binding(2, Stage::Pixel),
                        inputs.retained_pixel_b2.bytes().to_vec(),
                    ),
                ]);
                if let Some(retained) = inputs.retained {
                    let mut tracked = retained.borrow_mut();
                    self.retained.bind(&mut tracked, &buffers, &textures);
                    inputs.retained_pixel_b2.retain(&mut tracked);
                    for stage in [Stage::Vertex, Stage::Pixel] {
                        tracked.stream.bind_partial_constants(
                            1,
                            stage,
                            Some(&buffers[&binding(1, stage)]),
                            std::slice::from_ref(&(0..76)),
                        );
                    }
                }
                bound.push((
                    run,
                    self.pipeline
                        .bindings(arena, &buffers, &textures)
                        .map_err(refuse)?,
                ));
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled rivers"),
            color_attachments: &[Some(target.color)],
            depth_stencil_attachment: Some(target.depth),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, geometry.vertices.slice(..));
        pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
        for (run, bindings) in &bound {
            bindings.bind(&mut pass);
            pass.draw_indexed(run.indices.clone(), 0, 0..1);
        }
        Ok(())
    }
}
