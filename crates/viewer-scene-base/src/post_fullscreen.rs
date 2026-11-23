use shader_pipeline::gpu::stats::CountedPasses as _;
use std::{collections::BTreeMap, sync::Arc};

use eframe::wgpu::{self, util::DeviceExt};
use content_reader_gui::pdxshader::program::Stage;
use shader_pipeline::{
    binding, gpu,
    library::{Effect, Selection},
};

pub(super) use super::super::shader_sources::library;

pub(super) fn select(effect: &Arc<Effect>, name: &str) -> Result<Selection, String> {
    effect
        .select(name)?
        .ok_or_else(|| format!("missing {name} technique"))
}

pub(super) struct Fullscreen {
    pipeline: Arc<gpu::Pipeline>,
    vertices: wgpu::Buffer,
    blend_constant: bool,
}

impl Fullscreen {
    pub(super) fn new(
        device: &wgpu::Device,
        selection: &Selection,
        state: &gpu::State,
        format: wgpu::TextureFormat,
    ) -> Result<Self, String> {
        let compiled = selection.effect.compiled(selection.index)?;
        let mut pass = gpu::Pass::packed(&compiled, state, format, 1);
        pass.primitive.front_face = wgpu::FrontFace::Cw;
        pass.primitive.cull_mode = Some(wgpu::Face::Back);
        Self::from_pass(device, selection, state, pass)
    }

    pub(super) fn from_pass(
        device: &wgpu::Device,
        selection: &Selection,
        state: &gpu::State,
        mut pass: gpu::Pass,
    ) -> Result<Self, String> {
        let compiled = selection.effect.compiled(selection.index)?;
        let mut attributes = Vec::new();
        for input in &compiled.vertex.inputs {
            if !matches!(
                input.semantic.to_ascii_uppercase().as_str(),
                "POSITION" | "POSITION0"
            ) {
                return Err(format!(
                    "native fullscreen declaration has no {}",
                    input.semantic
                ));
            }
            attributes.push(wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Sint16x4,
                offset: 0,
                shader_location: input.location,
            });
        }
        pass.vertices = vec![gpu::VertexLayout {
            stride: 8,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        let blend_constant = pass.targets.iter().flatten().any(|target| {
            target.blend.is_some_and(|blend| {
                [blend.color, blend.alpha].iter().any(|component| {
                    [component.src_factor, component.dst_factor].iter().any(|factor| {
                        matches!(
                            factor,
                            wgpu::BlendFactor::Constant | wgpu::BlendFactor::OneMinusConstant
                        )
                    })
                })
            })
        });
        let pipeline = gpu::Cache::default().get_pass(device, selection, state, &pass)?;
        let vertices: [[i16; 4]; 3] = [[3, -1, 0, 0], [-1, -1, 1, 0], [-1, 3, 2, 0]];
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native post triangle"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Ok(Self {
            pipeline,
            vertices,
            blend_constant,
        })
    }

    pub(super) fn kept(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        slot: u32,
        offset: u32,
        bytes: &[u8],
        sources: &[&wgpu::TextureView],
    ) -> Result<gpu::Bindings, String> {
        self.pipeline.kept_bindings(
            device,
            buffer,
            |register| {
                [Stage::Vertex, Stage::Pixel]
                    .iter()
                    .any(|stage| binding(slot, *stage) == register)
                    .then_some((offset, bytes))
            },
            |register| {
                sources
                    .iter()
                    .enumerate()
                    .find(|(slot, _)| binding(*slot as u32, Stage::Pixel) == register)
                    .map(|(_, source)| *source)
            },
        )
    }

    pub(super) fn check_initialized(
        &self,
        slot: u32,
        physical: u32,
        known: &[std::ops::Range<u32>],
    ) -> Result<(), String> {
        for stage in [Stage::Vertex, Stage::Pixel] {
            self.pipeline
                .check_initialized(binding(slot, stage), physical, known)?;
        }
        Ok(())
    }

    pub(super) fn bind_textures(
        &self,
        arena: &gpu::Arena,
        slot: u32,
        bytes: &[u8],
        sources: &[&wgpu::TextureView],
    ) -> Result<gpu::Bindings, String> {
        self.pipeline.bindings(
            arena,
            &BTreeMap::from([
                (binding(slot, Stage::Vertex), bytes.to_vec()),
                (binding(slot, Stage::Pixel), bytes.to_vec()),
            ]),
            &sources
                .iter()
                .enumerate()
                .map(|(slot, source)| (binding(slot as u32, Stage::Pixel), (*source).clone()))
                .collect(),
        )
    }

    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        bindings: &gpu::Bindings,
        viewport: Option<[u32; 2]>,
    ) {
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled post triangle"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some([width, height]) = viewport {
            pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        }
        self.render(&mut pass, bindings);
    }

    pub(super) fn render(&self, pass: &mut wgpu::RenderPass<'_>, bindings: &gpu::Bindings) {
        if self.blend_constant {
            shader_pipeline::gpu::stats::state();
            pass.set_blend_constant(wgpu::Color::WHITE);
        }
        bindings.bind(pass);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        shader_pipeline::gpu::stats::draw();
        pass.draw(0..3, 0..1);
    }
}
