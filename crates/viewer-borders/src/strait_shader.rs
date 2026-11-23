use viewer_gpu::gpu_contract::Targets;
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StraitVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader straits: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/strait.shader", &defines)
        .map_err(refuse)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 4],
    uv: [f32; 4],
}

struct Compiled {
    retained: viewer_scene_base::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    stencil_reference: u32,
}

impl Compiled {
    fn compile(
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
            .select("strait")
            .map_err(refuse)?
            .ok_or_else(|| refuse("strait technique absent; retained program unavailable"))?;
        let program = &effect.source;
        let technique = &program.techniques[selection.index];
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state = gpu::State::authored(program, technique).map_err(refuse)?;
        state
            .samplers
            .retain(|slot, _| (0..3).any(|i| *slot == binding(i, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let mut attributes = Vec::new();
        for input in &compiled.vertex.inputs {
            let offset = match input.semantic.as_str() {
                "POSITION" | "POSITION0" => 0,
                "TEXCOORD" | "TEXCOORD0" => 16,
                other => return Err(refuse(format!("native vertex stream has no {other}"))),
            };
            if input.kind != ScalarKind::Float || input.components > 4 {
                return Err(refuse(format!("unsupported native vertex input {input:?}")));
            }
            attributes.push(wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset,
                shader_location: input.location,
            });
        }
        pass.vertices = vec![gpu::VertexLayout {
            stride: 32,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        let rasterizer = program.rasterizer(technique);
        pass.primitive = gpu::primitive(&rasterizer, wgpu::PrimitiveTopology::TriangleList).0;
        let depth_state = program.depth_stencil(technique);
        if depth_state.stencil_enable && !depth.has_stencil_aspect() {
            return Err(refuse(
                "authored stencil needs the native scene stencil attachment",
            ));
        }
        let (depth_state, stencil_reference) = gpu::depth_stencil(&depth_state, &rasterizer, depth);
        pass.depth_stencil = Some(depth_state);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &rasterizer,
            &program.depth_stencil(technique),
        );
        Ok(Self {
            retained,
            pipeline,
            stencil_reference,
        })
    }
}

pub struct Straits {
    compiled: Result<Compiled, String>,
    texture: wgpu::TextureView,
    geometry: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
}

impl Straits {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        texture: wgpu::TextureView,
    ) -> Self {
        Self {
            compiled: Compiled::compile(device, effect, samples, format, depth)
                .map_err(|error| error.to_string()),
            texture,
            geometry: None,
        }
    }

    pub fn upload(&mut self, device: &wgpu::Device, source: &[StraitVertex]) -> io::Result<()> {
        if !source.len().is_multiple_of(6) {
            return Err(refuse("ribbon geometry is not complete native quads"));
        }
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for triangle in source.chunks_exact(6) {
            let base = vertices.len() as u16;
            for i in [0, 1, 5, 2] {
                let v = triangle[i];
                vertices.push(Vertex {
                    position: [v.position[0], v.position[1], v.position[2], 0.0],
                    uv: [v.uv[0], v.uv[1], 0.0, 0.0],
                });
            }
            indices.extend([0_u16, 1, 3, 0, 3, 2].map(|i| base.wrapping_add(i)));
        }
        let count = u32::try_from(indices.len())
            .map_err(|_| refuse("index count exceeds native u32 draw range"))?;
        let limit = device.limits().max_buffer_size;
        if std::mem::size_of_val(vertices.as_slice()) as u64 > limit
            || std::mem::size_of_val(indices.as_slice()) as u64 > limit
        {
            return Err(refuse(format!(
                "ribbon buffers exceed adapter limit {limit} bytes"
            )));
        }
        self.geometry = if count == 0 {
            None
        } else {
            Some((
                shader_pipeline::gpu::stats::create_buffer_init(
                    &device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("native strait vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                ),
                shader_pipeline::gpu::stats::create_buffer_init(
                    &device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("native strait indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                ),
                count,
            ))
        };
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        frames: &[&[u8]],
        snow_mud_and_intel: [&wgpu::TextureView; 2],
        retained: Option<&RefCell<viewer_scene_base::shader_scene::State>>,
        visible: bool,
    ) -> io::Result<()> {
        let compiled = self
            .compiled
            .as_ref()
            .map_err(|reason| io::Error::other(reason.clone()))?;
        let Some((vertices, indices, count)) = &self.geometry else {
            return Ok(());
        };
        let textures = [&self.texture, snow_mud_and_intel[0], snow_mud_and_intel[1]]
            .into_iter()
            .enumerate()
            .map(|(slot, view)| (binding(slot as u32, Stage::Pixel), view.clone()))
            .collect();
        let bindings = frames
            .iter()
            .map(|frame| {
                if let Some(retained) = retained {
                    compiled.retained.bind(
                        &mut retained.borrow_mut(),
                        &BTreeMap::from([
                            (binding(0, Stage::Vertex), frame.to_vec()),
                            (binding(0, Stage::Pixel), frame.to_vec()),
                        ]),
                        &textures,
                    );
                }
                compiled
                    .pipeline
                    .bindings(
                        arena,
                        &BTreeMap::from([
                            (binding(0, Stage::Vertex), frame.to_vec()),
                            (binding(0, Stage::Pixel), frame.to_vec()),
                        ]),
                        &textures,
                    )
                    .map_err(refuse)
            })
            .collect::<io::Result<Vec<_>>>()?;
        if !visible {
            return Ok(());
        }
        let mut pass = viewer_scene_base::shader_scene::world_pass(encoder, targets, "compiled straits");
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&compiled.pipeline.pipeline);
        pass.set_stencil_reference(compiled.stencil_reference);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, vertices.slice(..));
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint16);
        let mut previous = None;
        for binding in &bindings {
            binding.bind_after(pass, previous);
            previous = Some(binding);
            shader_pipeline::gpu::stats::draw();
            pass.draw_indexed(0..*count, 0, 0..1);
        }
        Ok(())
    }
}
