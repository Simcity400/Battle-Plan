use viewer_gpu::gpu_contract::Targets;
use std::cell::RefCell;
use std::{collections::BTreeMap, io, ops::Range, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader rivers: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
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
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: &'a [[f32; 4]; 4],
    pub time: f32,
    pub retained_pixel_b2: viewer_scene_base::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub struct Geometry<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub runs: &'a [Run],
}

pub const SHADOW_SLOT: u32 = 15;

pub struct River {
    retained: viewer_scene_base::shader_scene::Program,
    animated: bool,
    pipeline: Arc<gpu::Pipeline>,
    pub texel_reads: Option<String>,
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
        let technique = if high { "river" } else { "river_low_gfx" };
        let selection = effect
            .select(technique)
            .map_err(refuse)?
            .ok_or_else(|| refuse(format!("{technique} absent; retained program unavailable")))?;
        let program = &effect.source;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let texel_reads = viewer_scene_base::shadow::texel_reads(&compiled, SHADOW_SLOT, technique);
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
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        viewer_scene_base::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..76)))
            .map_err(refuse)?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &depth_state,
        );
        Ok(Self {
            retained,
            animated: viewer_scene_base::shader_constants::reads_time(&pipeline, 1, 64),
            pipeline,
            texel_reads,
        })
    }

    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
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
        if bound.is_empty() {
            return Ok(());
        }
        let (mut pass, sampled) = viewer_scene_base::shader_scene::surface_pass(encoder, targets, "compiled rivers", self.animated);
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, geometry.vertices.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
        let mut previous = None;
        for (run, bindings) in &bound {
            bindings.bind_after(pass, previous);
            previous = Some(bindings);
            shader_pipeline::gpu::stats::draw();
            pass.draw_indexed(run.indices.clone(), 0, 0..1);
        }
        if sampled {
            pass.end_occlusion_query();
        }
        Ok(())
    }
}
