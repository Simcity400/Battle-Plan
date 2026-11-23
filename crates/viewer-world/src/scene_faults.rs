use shader_pipeline::gpu::stats::CountedPasses as _;
use super::scene::passes;
use eframe::wgpu;
use std::{
    cell::{Cell, RefCell},
    io,
    sync::Arc,
};
use viewer_gpu::gpu_contract::{DEPTH_FORMAT, OUTPUT_FORMAT, Targets, read_texture};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Point {
    Targets,
    Shadow,
    Trees,
    Arrows,
    Models,
    Weather,
    Post,
}

#[derive(Default)]
pub struct Faults {
    pub isolated: Option<(passes::Pass, String)>,
    pub fatal: Option<Point>,
    pub submissions: Cell<u64>,
    pub attempted: RefCell<Vec<passes::Pass>>,
    pub reached: RefCell<Vec<Point>>,
    pub sentinel: bool,
}

impl Faults {
    pub fn begin_frame(&self) {
        self.attempted.borrow_mut().clear();
        self.reached.borrow_mut().clear();
    }

    pub fn preflight(&self, pass: passes::Pass) -> io::Result<()> {
        self.attempted.borrow_mut().push(pass);
        if let Some((failed, reason)) = &self.isolated
            && *failed == pass
        {
            return Err(io::Error::other(reason.clone()));
        }
        Ok(())
    }

    pub fn after(&self, point: Point) -> io::Result<()> {
        self.reached.borrow_mut().push(point);
        if self.fatal == Some(point) {
            return Err(io::Error::other(format!(
                "test Map {point:?}: controlled frame failure"
            )));
        }
        Ok(())
    }

    pub fn draw_sentinel(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
    ) {
        if !self.sentinel {
            return;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene failure sentinel"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
                    let p = array<vec2f, 3>(vec2f(-1, -1), vec2f(3, -1), vec2f(-1, 3));
                    return vec4f(p[i], 0, 1);
                }
                @fragment fn pixel() -> @location(0) vec4f { return vec4f(0, 1, 0, 1); }
            "#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene failure sentinel"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("pixel"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let mut render = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Scene failure sentinel"),
            color_attachments: &[Some(targets.output_attachment(wgpu::LoadOp::Load))],
            ..Default::default()
        });
        render.set_pipeline(&pipeline);
        render.set_scissor_rect(0, 0, 8, 8);
        render.draw(0..3, 0..1);
    }
}

pub struct Probe {
    effect: Arc<shader_pipeline::library::Effect>,
}

impl Probe {
    pub fn new() -> io::Result<Self> {
        let source = br#"
            VertexStruct I { float2 p : POSITION; };
            VertexStruct O { float4 p : SV_POSITION; };
            ConstantBuffer(2,0) { float4 colour; float4 unknown; };
            VertexShader = { MainCode V [[ O main(I i) { O o; o.p=float4(i.p,0,1); return o; } ]] }
            PixelShader = {
                Samplers = { T = { Index = 0 MagFilter = Point MinFilter = Point MipFilter = None AddressU = Clamp AddressV = Clamp } }
                MainCode P [[ float4 main(O o) : SV_TARGET { return colour * T_Texture.Sample(T_Sampler,float2(0.5,0.5)); } ]]
                MainCode U [[ float4 main(O o) : SV_TARGET { return unknown; } ]]
            }
            RasterizerState Visible { CullMode = none }
            Effect Known { VertexShader = V PixelShader = P RasterizerState = Visible }
            Effect Unknown { VertexShader = V PixelShader = U RasterizerState = Visible }
        "#;
        let program =
            content_reader_gui::pdxshader::program::read_with(b"scene-test.shader", &|path| {
                Some(if path == b"scene-test.shader" {
                    source.to_vec()
                } else {
                    Vec::new()
                })
            })
            .map_err(io::Error::other)?;
        let compiler = shader_pipeline::Compiler::discover().map_err(io::Error::other)?;
        let compiled = program
            .techniques
            .iter()
            .map(|technique| compiler.compile(&program, technique, &[]).map(Arc::new))
            .collect();
        Ok(Self {
            effect: shader_pipeline::library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                compiled,
            ),
        })
    }

    pub fn template(
        &self,
        name: &str,
    ) -> io::Result<(
        shader_pipeline::library::Selection,
        shader_pipeline::gpu::Pass,
    )> {
        let selection = self.effect.select(name).map_err(io::Error::other)?.unwrap();
        let compiled = self
            .effect
            .compiled(selection.index)
            .map_err(io::Error::other)?;
        let state = shader_pipeline::gpu::State::authored(
            &self.effect.source,
            &self.effect.source.techniques[selection.index],
        )
        .map_err(io::Error::other)?;
        let mut pass = shader_pipeline::gpu::Pass::packed(&compiled, &state, OUTPUT_FORMAT, 1);
        let raster = content_reader_gui::pdxshader::RasterizerState {
            cull_mode: 0,
            ..Default::default()
        };
        let depth = content_reader_gui::pdxshader::DepthStencilState {
            depth_enable: false,
            depth_write_mask: 0,
            ..Default::default()
        };
        pass.depth_stencil =
            Some(shader_pipeline::gpu::depth_stencil(&depth, &raster, DEPTH_FORMAT).0);
        Ok((selection, pass))
    }

    pub fn state(
        &self,
        name: &str,
    ) -> io::Result<(viewer_scene_base::shader_scene::State, shader_pipeline::gpu::Pass)> {
        let (selection, pass) = self.template(name)?;
        let mut state = viewer_scene_base::shader_scene::State::default();
        state
            .stream
            .bind_program(Some(selection), pass.vertices.clone());
        state
            .stream
            .bind_rasterizer(content_reader_gui::pdxshader::RasterizerState {
                cull_mode: 0,
                ..Default::default()
            });
        state
            .stream
            .bind_depth_stencil(content_reader_gui::pdxshader::DepthStencilState {
                depth_enable: false,
                depth_write_mask: 0,
                ..Default::default()
            });
        state.bind_vertex(
            0,
            8,
            Arc::from(bytemuck::cast_slice(&[[-1_f32, -1.], [3., -1.], [-1., 3.]])),
        );
        Ok((state, pass))
    }

    pub fn bind(
        &self,
        state: &mut viewer_scene_base::shader_scene::State,
        texture: &wgpu::TextureView,
    ) -> io::Result<()> {
        use content_reader_gui::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let selection = self
            .effect
            .select("Known")
            .map_err(io::Error::other)?
            .unwrap();
        let authored = shader_pipeline::gpu::State::authored(
            &self.effect.source,
            &self.effect.source.techniques[selection.index],
        )
        .map_err(io::Error::other)?;
        let mut bytes = vec![0; 32];
        bytes[..16].copy_from_slice(bytemuck::cast_slice(&[1_f32; 4]));
        state.stream.bind_partial_constants(
            2,
            Stage::Pixel,
            Some(&bytes),
            std::slice::from_ref(&(0..16)),
        );
        state.stream.bind_texture(
            0,
            Stage::Pixel,
            Some(texture.clone()),
            authored.samplers.get(&binding(0, Stage::Pixel)),
        );
        Ok(())
    }

    pub fn pixel(
        &self,
        state: &mut viewer_scene_base::shader_scene::State,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> io::Result<Vec<u8>> {
        let arena = shader_pipeline::gpu::Arena::new(device);
        let (_, template) = self.template("Known")?;
        let submission = state.prepare(&arena, "retained Scene fixture", &template, 3, 1)?;
        let targets = Targets::new(device, [8, 8], [8, 8], 1, viewer_gpu::gpu_contract::SeaBed::Half);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut render = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(
                    targets.output_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                )],
                depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.))),
                ..Default::default()
            });
            render.set_pipeline(&submission.pass.pipeline.pipeline);
            submission.pass.bindings.bind(&mut render);
            for (slot, vertices) in submission.vertices.iter().enumerate() {
                render.set_vertex_buffer(slot as u32, vertices.slice());
            }
            render.draw(0..3, 0..1);
        }
        arena.flush(queue);
        queue.submit([encoder.finish()]);
        read_texture(device, queue, targets.output(), 4)
    }

    pub fn missing_binding(&self, device: &wgpu::Device) -> io::Result<String> {
        let arena = shader_pipeline::gpu::Arena::new(device);
        let (mut state, pass) = self.state("Unknown")?;
        let error = state
            .prepare(&arena, "Scene binding fault", &pass, 3, 1)
            .err()
            .ok_or_else(|| io::Error::other("missing native b2 was accepted"))?;
        assert!(error.to_string().contains("constant buffer 2"), "{error}");
        Ok(error.to_string())
    }
}
