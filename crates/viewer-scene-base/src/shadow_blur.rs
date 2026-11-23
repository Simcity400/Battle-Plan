use std::{collections::BTreeMap, io, sync::Arc};

use super::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
use shader_pipeline::{binding, gpu, library::Effect};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader shadow blur: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/shadowblur.shader", &defines)
        .map_err(refuse)
}

pub fn constants(extent: u32, height: f32) -> [f32; 8] {
    let offsets = [0.0, f32::from_bits(0x3fb13b14), f32::from_bits(0x404ec4ec)];
    let fade = (height / 700.0).min(1.0);
    let offsets = offsets.map(|offset| (offset - offset * fade) / extent as f32);
    [
        offsets[0],
        offsets[1],
        offsets[2],
        f32::from_bits(0x3e6879c6),
        f32::from_bits(0x3ea1e718),
        f32::from_bits(0x3d8fe9dc),
        1.0,
        1.0,
    ]
}

pub const SOURCE_SLOT: u32 = 0;

pub struct Blur {
    pipelines: [Arc<gpu::Pipeline>; 2],
    vertices: wgpu::Buffer,
    texel_reads: [Option<String>; 2],
}

impl Blur {
    pub fn new(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| Self::build(device, effect))
    }

    fn build(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        let mut cache = gpu::Cache::default();
        let mut texel_reads: [Option<String>; 2] = Default::default();
        let mut pipeline = |axis: usize, name| -> Result<Arc<gpu::Pipeline>, String> {
            let selection = effect
                .select(name)?
                .ok_or_else(|| format!("missing {name} technique"))?;
            let compiled = selection.effect.compiled(selection.index)?;
            texel_reads[axis] = super::shadow::texel_reads(&compiled, SOURCE_SLOT, name);
            let mut state =
                gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])?;
            state
                .samplers
                .retain(|slot, _| *slot == binding(SOURCE_SLOT, Stage::Pixel));
            let mut pass = gpu::Pass::packed(&compiled, &state, COLOR_FORMAT, 1);
            let attributes = compiled
                .vertex
                .inputs
                .iter()
                .map(|input| {
                    let (format, offset) = match input.semantic.to_ascii_uppercase().as_str() {
                        "POSITION" | "POSITION0" => (wgpu::VertexFormat::Float32x4, 0),
                        "TEXCOORD" | "TEXCOORD0" => (wgpu::VertexFormat::Float32x2, 16),
                        _ => {
                            return Err(format!(
                                "native shadow blur declaration has no {}",
                                input.semantic
                            ));
                        }
                    };
                    Ok(wgpu::VertexAttribute {
                        format,
                        offset,
                        shader_location: input.location,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            pass.vertices = vec![gpu::VertexLayout {
                stride: 32,
                step: wgpu::VertexStepMode::Vertex,
                attributes,
            }];
            pass.primitive.topology = wgpu::PrimitiveTopology::TriangleStrip;
            pass.primitive.front_face = wgpu::FrontFace::Cw;
            pass.primitive.cull_mode = Some(wgpu::Face::Back);
            pass.depth_stencil = (axis == 1).then_some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            });
            cache.get_pass(device, &selection, &state, &pass)
        };
        let pipelines = [
            pipeline(0, "ShadowBlurHorizontal"),
            pipeline(1, "ShadowBlurVertical"),
        ];
        let [horizontal, vertical] = pipelines;
        let vertices: [[f32; 8]; 4] = [
            [-1.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            [1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0],
        ];
        Ok(Self {
            pipelines: [horizontal.map_err(refuse)?, vertical.map_err(refuse)?],
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("native shadow blur strip"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            texel_reads,
        })
    }

    pub fn texel_reads(&self) -> impl Iterator<Item = &str> {
        self.texel_reads.iter().flatten().map(String::as_str)
    }

    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        axis: usize,
        bytes: &[f32; 8],
        source: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        self.pipelines[axis]
            .bindings(
                arena,
                &BTreeMap::from([
                    (
                        binding(0, Stage::Vertex),
                        bytemuck::cast_slice(bytes).to_vec(),
                    ),
                    (
                        binding(0, Stage::Pixel),
                        bytemuck::cast_slice(bytes).to_vec(),
                    ),
                ]),
                &BTreeMap::from([(binding(SOURCE_SLOT, Stage::Pixel), source.clone())]),
            )
            .map_err(refuse)
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>, axis: usize, bindings: &gpu::Bindings) {
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipelines[axis].pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        bindings.bind(pass);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        shader_pipeline::gpu::stats::draw();
        pass.draw(0..4, 0..1);
    }
}
