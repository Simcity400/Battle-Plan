use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

fn refuse(pass: &str, reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader {pass}: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/mapname.shader", &defines)
        .map_err(|error| refuse("country names", error))
}

pub struct Names {
    retained: viewer_scene_base::shader_scene::Program,
    pass_name: &'static str,
    pipeline: Arc<gpu::Pipeline>,
    stencil_reference: u32,
}

impl Names {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        Self::with_technique(device, effect, samples, format, depth_format, None)
    }

    pub fn with_technique(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        arrow_technique: Option<&str>,
    ) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(
                device,
                effect,
                samples,
                format,
                depth_format,
                arrow_technique,
            )
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        arrow_technique: Option<&str>,
    ) -> io::Result<Self> {
        let pass_name = if arrow_technique.is_some() {
            "order labels"
        } else {
            "country names"
        };
        let refuse = |reason: String| refuse(pass_name, reason);
        let name = arrow_technique.unwrap_or("mapname");
        let selection = effect
            .select(name)
            .map_err(|error| refuse(error.to_string()))?
            .ok_or_else(|| refuse(format!("{name} absent; retained program unavailable")))?;
        let compiled = effect
            .compiled(selection.index)
            .map_err(|error| refuse(error.to_string()))?;
        let technique = &effect.source.techniques[selection.index];
        let mut state = gpu::State::authored(&effect.source, technique)
            .map_err(|error| refuse(error.to_string()))?;
        state
            .samplers
            .retain(|slot, _| *slot == binding(0, Stage::Pixel));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let mut attributes = Vec::new();
        let mut has_color = false;
        let mut next_location = 0u32;
        for input in &compiled.vertex.inputs {
            next_location = next_location.max(input.location + 1);
            let (offset, format, width) = match input.semantic.as_str() {
                "POSITION" | "POSITION0" => (0, wgpu::VertexFormat::Float32x3, 3),
                "TEXCOORD" | "TEXCOORD0" => (12, wgpu::VertexFormat::Float32x2, 2),
                "COLOR" | "COLOR0" => {
                    has_color = true;
                    (20, wgpu::VertexFormat::Float32x4, 4)
                }
                _ => return Err(refuse(format!("native glyph cannot supply {input:?}"))),
            };
            if input.kind != ScalarKind::Float || input.components > width {
                return Err(refuse(format!("native glyph cannot supply {input:?}")));
            }
            attributes.push(wgpu::VertexAttribute {
                format,
                offset,
                shader_location: input.location,
            });
        }
        let retained_attributes = attributes.clone();
        if !has_color {
            attributes.push(wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 20,
                shader_location: next_location,
            });
            pass.vertex_color_tint = Some(next_location);
        }
        pass.vertices = vec![gpu::VertexLayout {
            stride: size_of::<Vertex>() as u64,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        let retained_vertices = vec![gpu::VertexLayout {
            stride: size_of::<Vertex>() as u64,
            step: wgpu::VertexStepMode::Vertex,
            attributes: retained_attributes,
        }];
        let raster = RasterizerState {
            cull_mode: if arrow_technique.is_some() {
                0
            } else {
                RasterizerState::default().cull_mode
            },
            front_ccw: arrow_technique.is_none(),
            ..Default::default()
        };
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        let (depth, stencil_reference) = gpu::depth_stencil(
            &effect.source.depth_stencil(technique),
            &raster,
            depth_format,
        );
        pass.depth_stencil = Some(depth);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(|error| refuse(error.to_string()))?;
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(|error| refuse(error.to_string()))?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &retained_vertices,
            &state,
            &raster,
            &effect.source.depth_stencil(technique),
        );
        Ok(Self {
            retained,
            pass_name,
            pipeline,
            stencil_reference,
        })
    }

    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        frame: &[u8],
        alpha_offset: [f32; 2],
        page: &gpu::TextureBinding,
    ) -> io::Result<gpu::Bindings> {
        let b1 = [alpha_offset[0], alpha_offset[1], 0., 0.];
        let buffers = BTreeMap::from([
            (binding(0, Stage::Vertex), frame.to_vec()),
            (binding(0, Stage::Pixel), frame.to_vec()),
            (binding(1, Stage::Vertex), bytemuck::bytes_of(&b1).to_vec()),
            (binding(1, Stage::Pixel), bytemuck::bytes_of(&b1).to_vec()),
        ]);
        let textures = BTreeMap::from([(binding(0, Stage::Pixel), page.clone())]);
        self.pipeline
            .bind_with(
                arena,
                |key| buffers.get(&key).map(Vec::as_slice),
                |key| {
                    textures
                        .get(&key)
                        .map(|texture| self.pipeline.sampled_texture(key, texture))
                },
            )
            .map_err(|error| refuse(self.pass_name, error.to_string()))
    }

    pub fn retain(
        &self,
        retained: &mut viewer_scene_base::shader_scene::State,
        frame: &[u8],
        alpha_offset: [f32; 2],
        page: &gpu::TextureBinding,
    ) {
        let b1 = [alpha_offset[0], alpha_offset[1], 0., 0.];
        self.retained.bind_formatted(
            retained,
            &BTreeMap::from([
                (binding(0, Stage::Vertex), frame.to_vec()),
                (binding(0, Stage::Pixel), frame.to_vec()),
                (binding(1, Stage::Vertex), bytemuck::bytes_of(&b1).to_vec()),
                (binding(1, Stage::Pixel), bytemuck::bytes_of(&b1).to_vec()),
            ]),
            &BTreeMap::from([(binding(0, Stage::Pixel), page.clone())]),
        );
        retained
            .stream
            .bind_vertex_stride(0, size_of::<Vertex>() as u64);
    }

    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(self.stencil_reference);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}
