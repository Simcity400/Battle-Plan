//! Authored trail technique fixture; installed vanilla has no such technique.
use super::{DEPTH, FORMAT, compiled, decode, extent, read};
use crate::shader_scene as retained_scene;
use eframe::wgpu::{self, util::DeviceExt};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::Arc};

pub(super) fn run(device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let source = br#"
VertexStruct I { float3 p : POSITION;
float2 uv : TEXCOORD0;
uint4 tile : TEXCOORD1;
float4 color : COLOR;
};
VertexStruct O { float4 p : SV_POSITION;
float4 color : COLOR;
};
VertexShader = { MainCode V [[ O main(I v) { O o; o.p=float4(v.p.x,v.p.z,0.5,1); o.color=float4(float(v.tile.x)/255.0,v.color.g,v.uv.x,1); return o; } ]] }
PixelShader = { MainCode P [[ float4 main(O v) : SV_TARGET { return v.color; } ]] }
RasterizerState Raster { CullMode = "none" }
Effect AuthoredTrail { VertexShader = V PixelShader = P RasterizerState = Raster }
"#;
    let program = installed_content::pdxshader::program::read_with(b"fixture.shader", &|p| {
        Some(if p == b"fixture.shader" {
            source.to_vec()
        } else {
            Vec::new()
        })
    })
    .map_err(io::Error::other)?;
    let compiled_shader = shader_pipeline::Compiler::discover()
        .map_err(io::Error::other)?
        .compile(&program, &program.techniques[0], &[])
        .map_err(io::Error::other)?;
    let effect = shader_pipeline::library::Effect::from_compiled(
        Arc::new(program),
        vec![],
        vec![Ok(Arc::new(compiled_shader))],
    );
    let pipeline =
        super::fixture::Pass::for_packet(device, &effect, "AuthoredTrail", 1, FORMAT, DEPTH, true)?;
    let bindings = pipeline.bindings(&arena, &BTreeMap::new(), &BTreeMap::new())?;
    let records = [-0.5, 0.5].map(|x| compiled::Instance {
        position_size: [x, 0., 0., 0.5],
        rotation: [0.; 3],
        tile: [255, 2, 3, 4],
        color: [0, 128, 0, 255],
        tail: [0; 12],
    });
    let vertices = compiled::trail::vertices(&records, [0.; 3], 2., false)?;
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("native trail28 fixture"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let selection = effect
        .select("AuthoredTrail")
        .map_err(io::Error::other)?
        .unwrap();
    let shader = effect.compiled(selection.index).map_err(io::Error::other)?;
    let mut layouts = compiled::layouts(&shader.vertex.inputs, true)?;
    let color = shader
        .vertex
        .inputs
        .iter()
        .find(|input| input.semantic == "COLOR")
        .unwrap()
        .location;
    layouts[0]
        .attributes
        .retain(|attribute| attribute.shader_location != color);
    layouts.push(shader_pipeline::gpu::VertexLayout {
        stride: 48,
        step: wgpu::VertexStepMode::Instance,
        attributes: vec![wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 32,
            shader_location: color,
        }],
    });
    let mut preceding = retained_scene::State::default();
    let native_depth = installed_content::pdxshader::DepthStencilState {
        depth_enable: true,
        depth_write_mask: 0,
        depth_function: 3,
        stencil_enable: false,
        ..Default::default()
    };
    let state = shader_pipeline::gpu::State::authored(
        &effect.source,
        &effect.source.techniques[selection.index],
    )
    .map_err(io::Error::other)?;
    let producer = retained_scene::Program::new(
        &selection,
        &layouts,
        &state,
        &effect
            .source
            .rasterizer(&effect.source.techniques[selection.index]),
        &native_depth,
    );
    producer.bind(
        &mut preceding,
        &BTreeMap::<u32, Vec<u8>>::new(),
        &BTreeMap::new(),
    );
    preceding.bind_vertex(1, 48, bytemuck::bytes_of(&records[0]).into());
    let mut inherited = retained_scene::State::default();
    inherited.inherit(&preceding);
    inherited.stream.bind_program(
        effect.select("MissingTrail").map_err(io::Error::other)?,
        Vec::new(),
    );
    inherited.bind_vertex(0, 28, bytemuck::cast_slice(&vertices).into());
    let state = shader_pipeline::gpu::State {
        samplers: BTreeMap::new(),
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    };
    let mut template = shader_pipeline::gpu::Pass::packed(&shader, &state, FORMAT, 1);
    template.primitive.topology = wgpu::PrimitiveTopology::TriangleStrip;
    template.depth_stencil =
        Some(shader_pipeline::gpu::depth_stencil(&native_depth, &Default::default(), DEPTH).0);
    let retained = inherited.prepare(
        &arena,
        "retained particle program fixture",
        &template,
        vertices.len() as u32,
        1,
    )?;
    inherited = Default::default();
    assert!(
        inherited
            .prepare(&arena, "unknown previous program", &template, 4, 1)
            .is_err()
    );
    let size = [32, 16];
    let texture = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("native trail fixture target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if format == FORMAT {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        })
    };
    let target = texture(FORMAT);
    let view = target.create_view(&Default::default());
    let depth = texture(DEPTH).create_view(&Default::default());
    let mut rows = Vec::new();
    for (depth_clear, retained_program) in [(0., false), (1., false), (0., true), (1., true)] {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native trail fixture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth_clear),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(255),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                ..Default::default()
            });
            if retained_program {
                pass.set_pipeline(&retained.pass.pipeline.pipeline);
                retained.pass.bindings.bind(&mut pass);
                pass.set_stencil_reference(retained.pass.stencil_reference.unwrap_or(0));
                for (slot, buffer) in retained.vertices.iter().enumerate() {
                    pass.set_vertex_buffer(slot as u32, buffer.slice());
                }
            } else {
                pipeline.begin(&mut pass);
                bindings.bind(&mut pass);
                pass.set_vertex_buffer(0, buffer.slice(..));
            }
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        let bytes = read(&arena, queue, encoder, &target, size)?;
        let different = bytes
            .chunks_exact(8)
            .enumerate()
            .filter(|(i, bytes)| {
                let expected = if depth_clear == 0. {
                    [0.; 4]
                } else {
                    [
                        1.,
                        128. / 255.,
                        ((*i as u32 % size[0]) as f32 + 0.5) / size[0] as f32,
                        1.,
                    ]
                };
                bytes
                    .chunks_exact(2)
                    .zip(expected)
                    .any(|(v, e)| (decode(u16::from_le_bytes([v[0], v[1]])) - e).abs() > 0.00025)
            })
            .count();
        rows.push(json!({"depth":depth_clear,"retained_program":retained_program,"vertices":vertices.len(),"pixels":size[0]*size[1],"different":different}));
    }
    if rows.iter().any(|r| r["different"] != 0) {
        return Err(io::Error::other(format!(
            "native trail fixture differs: {rows:?}"
        )));
    }
    Ok(
        json!({"scope":"synthetic declared trail and retained prior scene program, production CPU expansion; retained 48-byte instance slot while slot0 changes to28; depth rejection; not historical scene-order proof","cases":rows}),
    )
}
