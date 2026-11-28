use viewer_gpu::gpu_contract::Targets;
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader borders: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/border.shader", &defines)
        .map_err(refuse)
}

pub struct Inputs<'a> {
    pub fog: bool,
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub snow_mud: &'a wgpu::TextureView,
    pub intel: &'a wgpu::TextureView,
    pub shadow: &'a wgpu::TextureView,
    pub time: f32,
}

pub struct Run<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub index_format: wgpu::IndexFormat,
    pub range: std::ops::Range<u32>,
}

pub struct Batch<'a> {
    pub texture: &'a wgpu::TextureView,
    pub alpha: f32,
    pub selection: [f32; 4],
    pub runs: Vec<Run<'a>>,
}

pub const SHADOW_SLOT: u32 = 3;

pub struct Border {
    retained: viewer_scene_base::shader_scene::Program,
    default_clock: viewer_scene_base::shader_constants::SurfaceClock,
    selected_clock: viewer_scene_base::shader_constants::SurfaceClock,
    pipeline: Arc<gpu::Pipeline>,
    pub texel_reads: Option<String>,
}

impl Border {
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
        let selection = effect
            .select("border")
            .map_err(refuse)?
            .ok_or_else(|| refuse("border technique absent; retained program unavailable"))?;
        let program = &effect.source;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let texel_reads = viewer_scene_base::shadow::texel_reads(&compiled, SHADOW_SLOT, "border");
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
        pipeline
            .check_initialized(binding(2, Stage::Pixel), 32, std::slice::from_ref(&(0..24)))
            .map_err(refuse)?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &depth_state,
        );
        use viewer_scene_base::shader_constants::SurfaceClock;
        let selection_color = [0, 4, 8, 12].map(|byte| (binding(2, Stage::Pixel), byte));
        Ok(Self {
            retained,
            default_clock: SurfaceClock::new(&compiled, 2, 16, &selection_color),
            selected_clock: SurfaceClock::new(&compiled, 2, 16, &[]),
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
        batches: &[Batch<'_>],
        visible: bool,
    ) -> io::Result<viewer_scene_base::shader_constants::WorldPixelConstants> {
        let mut retained =
            viewer_scene_base::shader_constants::WorldPixelConstants::border(inputs.time, [0.; 4], 1.);
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
                retained = viewer_scene_base::shader_constants::WorldPixelConstants::border(
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
                    viewer_scene_base::shader_constants::WorldPixelConstants::border(
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
                    targets.surface_motion.query(if batch.selection == [0.; 4] {
                        self.default_clock.live(inputs.fog)
                    } else {
                        self.selected_clock.live(inputs.fog)
                    }),
                ));
            }
        }
        if bound.is_empty() {
            return Ok(retained);
        }
        let query_set = bound
            .iter()
            .find_map(|(_, _, query)| query.map(|(set, _)| set));
        let mut pass = viewer_scene_base::shader_scene::world_pass_with_query(
            encoder,
            targets,
            "compiled borders",
            query_set,
        );
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        let mut previous = None;
        for (batch, binding, query) in &bound {
            binding.bind_after(pass, previous);
            previous = Some(binding);
            if let Some((_, index)) = query {
                pass.begin_occlusion_query(*index);
            }
            for run in &batch.runs {
                shader_pipeline::gpu::stats::state();
                pass.set_vertex_buffer(0, run.vertices.slice(..));
                shader_pipeline::gpu::stats::state();
                pass.set_index_buffer(run.indices.slice(..), run.index_format);
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(run.range.clone(), 0, 0..1);
            }
            if query.is_some() {
                pass.end_occlusion_query();
            }
        }
        Ok(retained)
    }
}
