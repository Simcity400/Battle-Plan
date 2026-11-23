use viewer_gpu::gpu_contract::Targets;
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader railways: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/models/supply/railroad.shader", &defines)
        .map_err(refuse)
}

pub struct Inputs<'a> {
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: &'a [[f32; 4]; 4],
    pub time: f32,
    pub alpha: f32,
    pub retained_pixel_b2: viewer_scene_base::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub struct Geometry<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub strips: Option<(&'a wgpu::Buffer, u32)>,
}

pub const SHADOW_SLOT: u32 = 15;

pub struct Rail {
    retained: viewer_scene_base::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    stencil_reference: u32,
    pub texel_reads: Option<String>,
}

impl Rail {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
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
        let technique = "railroad";
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
                    ("TEXCOORD" | "TEXCOORD0", ScalarKind::Float, 1..=2) => 16,
                    _ => {
                        return Err(refuse(format!(
                            "native vertex stream cannot supply {input:?}"
                        )));
                    }
                };
                Ok(wgpu::VertexAttribute {
                    format: if offset == 0 {
                        wgpu::VertexFormat::Float32x4
                    } else {
                        wgpu::VertexFormat::Float32x2
                    },
                    offset,
                    shader_location: input.location,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        pass.vertices = vec![gpu::VertexLayout {
            stride: 24,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        let technique = &program.techniques[selection.index];
        let raster = program.rasterizer(technique);
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleStrip).0;
        pass.primitive.strip_index_format = Some(wgpu::IndexFormat::Uint32);
        let (depth, stencil_reference) =
            gpu::depth_stencil(&program.depth_stencil(technique), &raster, depth);
        pass.depth_stencil = Some(depth);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        viewer_scene_base::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..72)))
            .map_err(refuse)?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &program.depth_stencil(technique),
        );
        Ok(Self {
            retained,
            pipeline,
            stencil_reference,
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
            let mut constants = bytemuck::bytes_of(inputs.shadow_matrix).to_vec();
            constants.extend_from_slice(bytemuck::cast_slice(&[
                inputs.time,
                inputs.alpha,
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
                        std::slice::from_ref(&(0..72)),
                    );
                }
            }
            bound.push(
                self.pipeline
                    .bindings(arena, &buffers, &textures)
                    .map_err(refuse)?,
            );
        }
        let mut pass = viewer_scene_base::shader_scene::world_pass(encoder, targets, "compiled railways");
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(self.stencil_reference);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, geometry.vertices.slice(..));
        if let Some((indices, _)) = geometry.strips {
            shader_pipeline::gpu::stats::state();
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        }
        let mut previous = None;
        for bindings in &bound {
            bindings.bind_after(pass, previous);
            previous = Some(bindings);
            if let Some((_, count)) = geometry.strips {
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(0..count, 0, 0..1);
            }
        }
        Ok(())
    }
}
