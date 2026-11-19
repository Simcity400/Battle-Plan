//! Railway submission with resolved effects and retained render state.
use viewer_foundation::map::gpu_contract::Targets;
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

use eframe::wgpu;
use installed_content_core::{ResolvedLayers, pdxshader::program::Stage};
use installed_content_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader railways: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/models/supply/railroad.shader", &defines)
        .map_err(refuse)
}

pub struct Inputs<'a> {
    pub retained: Option<&'a RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub shadow_matrix: &'a [[f32; 4]; 4],
    pub time: f32,
    pub alpha: f32,
    /// Actual retained PS b2 allocation, including its initialized-byte ranges.
    pub retained_pixel_b2: super::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub struct Geometry<'a> {
    pub vertices: &'a wgpu::Buffer,
    /// Every run's strip in one index list, each closed by the restart index, and its length
    /// ([`super::railways::strip_indices`]): drawn with one call.
    pub strips: Option<(&'a wgpu::Buffer, u32)>,
}

pub struct Rail {
    retained: super::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    stencil_reference: u32,
}

impl Rail {
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
        let technique = "railroad";
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
        // Apply every authored state handle and triangle-strip topology.
        let technique = &program.techniques[selection.index];
        let raster = program.rasterizer(technique);
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleStrip).0;
        // The restart index closes each strip of the joined list.
        pass.primitive.strip_index_format = Some(wgpu::IndexFormat::Uint32);
        let (depth, stencil_reference) =
            gpu::depth_stencil(&program.depth_stencil(technique), &raster, depth);
        pass.depth_stencil = Some(depth);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::scene(&pipeline).map_err(refuse)?;
        super::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..72)))
            .map_err(refuse)?;
        let retained = super::shader_scene::Program::new(
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
            // An 80-byte allocation initializes 72 matrix/time/alpha bytes; bytes 72 through 79 remain unknown.
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
        let mut pass = super::scene::world_pass(encoder, targets, "compiled railways");
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
