use std::{
    io,
    sync::{Arc, OnceLock},
};

use eframe::wgpu;
use content_reader_gui::pdxshader::{DepthStencilState, program::Stage};
use shader_pipeline::{
    ScalarKind, VertexInput, binding, gpu,
    library::{Effect, Selection},
    stream::NativeSubmission,
};

use viewer_scene_base::shader_scene;

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Movement arrows: 1 refused draw: {}",
        reason.into()
    ))
}

fn layouts(inputs: &[VertexInput]) -> Result<Vec<gpu::VertexLayout>, String> {
    let attributes = inputs
        .iter()
        .map(|input| {
            let (offset, format) = match (input.semantic.as_str(), input.kind, input.components) {
                ("POSITION" | "POSITION0", ScalarKind::Float, 1..=3) => {
                    (0, wgpu::VertexFormat::Float32x3)
                }
                ("TEXCOORD" | "TEXCOORD0", ScalarKind::Float, 1..=2) => {
                    (12, wgpu::VertexFormat::Float32x2)
                }
                ("TEXCOORD1", ScalarKind::Float, 1..=3) => (20, wgpu::VertexFormat::Float32x3),
                _ => {
                    return Err(format!(
                        "native 32-byte arrow vertex cannot supply {input:?}"
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
    Ok(vec![gpu::VertexLayout {
        stride: 32,
        step: wgpu::VertexStepMode::Vertex,
        attributes,
    }])
}

struct Compiled {
    selection: Selection,
    layouts: Vec<gpu::VertexLayout>,
}

pub type Texture = Result<Option<wgpu::TextureView>, String>;

pub struct Arrow {
    effect: Arc<Effect>,
    state: gpu::State,
    depth: DepthStencilState,
    samplers: u32,
    template: gpu::Pass,
    compiled: OnceLock<Result<Option<Compiled>, String>>,
}

impl Arrow {
    pub fn new(
        effect: &Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let source = &effect.source;
        let (state, depth_state, samplers) =
            if let Some(technique) = source.technique("ArrowEffect") {
                let samplers = source
                    .common
                    .samplers
                    .iter()
                    .chain(&source.pixel.samplers)
                    .map(|sampler| sampler.index.saturating_add(1))
                    .max()
                    .unwrap_or(0)
                    .min(4);
                (
                    gpu::State::authored(source, technique).map_err(refuse)?,
                    source.depth_stencil(technique),
                    samplers,
                )
            } else {
                (
                    gpu::State {
                        samplers: Default::default(),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    },
                    DepthStencilState {
                        depth_function: 7,
                        ..Default::default()
                    },
                    0,
                )
            };
        let template = gpu::Pass {
            vertices: Vec::new(),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(gpu::depth_stencil(&depth_state, &Default::default(), depth).0),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format,
                blend: state.blend,
                write_mask: state.write_mask,
            })],
            instanced: 0,
            stream: None,
            vertex_color_tint: None,
        };
        Ok(Self {
            effect: effect.clone(),
            state,
            depth: depth_state,
            samplers,
            template,
            compiled: OnceLock::new(),
        })
    }

    fn compiled(&self) -> &Result<Option<Compiled>, String> {
        self.compiled.get_or_init(|| {
            shader_pipeline::library::with_recorded(&self.effect, |effect| {
                let build = || -> Result<_, String> {
                    let Some(selection) = effect.select("ArrowEffect")? else {
                        return Ok(None);
                    };
                    let compiled = effect.compiled(selection.index)?;
                    Ok(Some(Compiled {
                        layouts: layouts(&compiled.vertex.inputs)?,
                        selection,
                    }))
                };
                build().map_err(std::io::Error::other)
            })
            .map_err(|error| error.to_string())
        })
    }

    pub fn bind_style(&self, retained: &mut shader_scene::State, textures: &[Texture; 4]) {
        let compiled = self.compiled();
        let stream = &mut retained.stream;
        stream.bind_depth_stencil(self.depth.clone());
        stream.bind_blend(self.state.blend, self.state.write_mask);
        match compiled {
            Ok(Some(compiled)) => {
                stream.bind_program(Some(compiled.selection.clone()), compiled.layouts.clone())
            }
            Ok(None) => stream.bind_program(None, Vec::new()),
            Err(_) => stream.forget_program(),
        }
        for slot in 0..self.samplers {
            let sampler = self.state.samplers.get(&binding(slot, Stage::Pixel));
            match &textures[slot as usize] {
                Ok(texture) => stream.bind_texture(slot, Stage::Pixel, texture.clone(), sampler),
                Err(_) => stream.forget_texture(slot, Stage::Pixel, sampler),
            }
        }
    }

    pub fn prepare(
        &self,
        arena: &gpu::Arena,
        retained: &mut shader_scene::State,
        vertices: Arc<[u8]>,
        count: u32,
        constants: &[u8; 112],
        textures: &[Texture; 4],
    ) -> io::Result<NativeSubmission> {
        for stage in [Stage::Vertex, Stage::Pixel] {
            retained.stream.bind_partial_constants(
                1,
                stage,
                Some(constants),
                std::slice::from_ref(&(0..100)),
            );
        }
        retained.bind_vertex(0, 32, vertices);
        if let Err(reason) = self.compiled() {
            return Err(refuse(reason.clone()));
        }
        retained
            .prepare(arena, "selected unit arrow", &self.template, count, 1)
            .map_err(|error| {
                let mut reason = error.to_string();
                if let Some((slot, error)) = textures
                    .iter()
                    .take(self.samplers as usize)
                    .enumerate()
                    .find_map(|(slot, texture)| texture.as_ref().err().map(|reason| (slot, reason)))
                {
                    reason.push_str(&format!("; texture slot {slot}: {error}"));
                }
                refuse(reason)
            })
    }
}

pub fn restore_depth(retained: &mut shader_scene::State) {
    retained
        .stream
        .bind_depth_stencil(DepthStencilState::default());
}

pub fn draw(pass: &mut wgpu::RenderPass<'_>, submission: &NativeSubmission, count: u32) {
    shader_pipeline::gpu::stats::pipeline();
    pass.set_pipeline(&submission.pass.pipeline.pipeline);
    pass.set_stencil_reference(submission.pass.stencil_reference.unwrap_or(0));
    shader_pipeline::gpu::stats::state();
    pass.set_blend_constant(wgpu::Color::WHITE);
    submission.pass.bindings.bind(pass);
    for (slot, vertices) in submission.vertices.iter().enumerate() {
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(slot as u32, vertices.slice());
    }
    shader_pipeline::gpu::stats::draw();
    pass.draw(0..count, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_bucket_applies_program_transition_without_geometry_upload() {
        use content_reader_gui::pdxshader::program::{Program, Technique};

        let missing = Effect::from_compiled(Arc::new(Program::default()), vec![], vec![]);
        let previous = Selection {
            effect: missing.clone(),
            index: 0,
        };
        let declaration = vec![gpu::VertexLayout {
            stride: 48,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![],
        }];
        let mut retained = shader_scene::State::default();
        retained
            .stream
            .bind_program(Some(previous), declaration.clone());
        let textures = std::array::from_fn(|_| Ok(None));
        let arrow = Arrow::new(
            &missing,
            1,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth24PlusStencil8,
        )
        .unwrap();
        arrow.bind_style(&mut retained, &textures);
        assert_eq!(retained.stream.vertex_layouts().unwrap(), declaration);

        let source = Program {
            techniques: vec![Technique {
                name: "ArrowEffect".into(),
                vertex: "VS".into(),
                pixel: "PS".into(),
                defines: vec![],
                blend: String::new(),
                depth_stencil: String::new(),
                rasterizer: String::new(),
            }],
            ..Default::default()
        };
        let failed = Effect::from_compiled(
            Arc::new(source),
            vec![],
            vec![Err("fixture declared compilation failure".into())],
        );
        let arrow = Arrow::new(
            &failed,
            1,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth24PlusStencil8,
        )
        .unwrap();
        arrow.bind_style(&mut retained, &textures);
        assert!(retained.stream.vertex_layouts().is_err());
        assert!(
            matches!(arrow.compiled(), Err(reason) if reason == "fixture declared compilation failure")
        );
    }

    #[test]
    fn reflected_input_order_keeps_native_offsets() {
        let input = |semantic: &str, location, components| VertexInput {
            semantic: semantic.into(),
            location,
            components,
            kind: ScalarKind::Float,
        };
        let layouts = layouts(&[
            input("TEXCOORD1", 0, 3),
            input("POSITION0", 1, 3),
            input("TEXCOORD0", 2, 2),
        ])
        .unwrap();
        assert_eq!(layouts[0].stride, 32);
        assert_eq!(
            layouts[0]
                .attributes
                .iter()
                .map(|a| (a.shader_location, a.offset))
                .collect::<Vec<_>>(),
            [(0, 20), (1, 0), (2, 12)]
        );
        assert!(super::layouts(&[input("TANGENT0", 0, 3)]).is_err());
        assert!(super::layouts(&[input("POSITION", 0, 4)]).is_err());
        let mut integer = input("POSITION", 0, 3);
        integer.kind = ScalarKind::Uint;
        assert!(super::layouts(&[integer]).is_err());
    }
}
