//! Native strait geometry/state versus the old kernel with controlled alpha writes.
use crate::shader_scene;
use std::{fs, io, path::Path};

use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};

use super::{extent, read};
use crate::shader_constants;
use crate::shader_sources;
#[path = "../../src/map/strait_shader.rs"]
mod compiled;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map, crate::DECODE)?;
    let selection = effect
        .select("strait")
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("fixture strait technique missing"))?;
    let native_state = shader_pipeline::gpu::State::authored(
        &effect.source,
        &effect.source.techniques[selection.index],
    )
    .map_err(io::Error::other)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("strait port oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("strait_port.wgsl").into()),
    });
    let format = wgpu::TextureFormat::Rgba16Float;
    let depth_format = wgpu::TextureFormat::Depth24PlusStencil8;
    let old = std::array::from_fn::<_, 2, _>(|mode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("strait alpha control"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2],
                })],
            },
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: native_state.blend,
                    write_mask: if mode == 0 {
                        wgpu::ColorWrites::COLOR
                    } else {
                        native_state.write_mask
                    },
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    });
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("strait fixture diffuse"),
        size: extent([16, 16]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<u8> = (0..256_u32)
        .flat_map(|i| {
            [
                (i * 13) as u8,
                (i * 7 + 29) as u8,
                (i * 37 + 3) as u8,
                (i * 11) as u8,
            ]
        })
        .collect();
    queue.write_texture(
        source.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: None,
        },
        extent([16, 16]),
    );
    let source = source.create_view(&Default::default());
    let mut native =
        compiled::Straits::new(device, &effect, 1, format, depth_format, source.clone())?;
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for (case, (size, height, blocked, reversed)) in [
        ([64, 48], 2.0, false, false),
        ([63, 47], 40.0, false, false),
        ([64, 48], 400.0, false, false),
        ([64, 48], 2.0, true, false),
        ([64, 48], 2.0, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let corners = [
            [-0.85, 0.75, 0.4],
            [0.8, 0.75, 0.4],
            [-0.85, -0.9, 0.4],
            [0.8, -0.9, 0.4],
        ];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 3.0], [1.0, 3.0]];
        let vertices: Vec<_> = [0, 1, 3, 0, 3, 2]
            .into_iter()
            .map(|i| compiled::StraitVertex {
                position: [
                    corners[i][0] * if reversed { -1.0 } else { 1.0 },
                    corners[i][1],
                    corners[i][2],
                ],
                uv: uvs[i],
            })
            .collect();
        native.upload(device, &vertices)?;
        let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut constants = [0.0_f32; 104];
        for i in 0..4 {
            constants[i * 5] = 1.0;
            constants[16 + i * 5] = 1.0;
        }
        constants[53] = height;
        constants[54] = height;
        constants[58] = 1.0;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&constants),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut results = Vec::new();
        for (mode, oracle) in old.iter().map(Some).chain([None]).enumerate() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth = depth.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.25,
                                g: 0.5,
                                b: 0.75,
                                a: 0.625,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(if blocked { 0.1 } else { 1.0 }),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0xA5),
                            store: wgpu::StoreOp::Store,
                        }),
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            let target = compiled::Target {
                color: wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                },
                depth: wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                },
            };
            if let Some(oracle) = oracle {
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &oracle.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(target.color)],
                    depth_stencil_attachment: Some(target.depth),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(oracle);
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, vertex.slice(..));
                pass.draw(0..6, 0..1);
            } else {
                native.draw(
                    &arena,
                    &mut encoder,
                    target,
                    &[bytemuck::cast_slice(&constants)],
                    [&source, &source],
                    None,
                )?;
            }
            let bytes = read(&arena, queue, encoder, &texture, size)?;
            fs::write(output.join(format!("strait-{case}-{mode}.rgba16f")), &bytes)?;
            results.push(bytes);
        }
        let delta = |control: usize, channels: std::ops::Range<usize>| {
            results[control]
                .chunks_exact(8)
                .zip(results[2].chunks_exact(8))
                .filter(|(a, b)| {
                    channels
                        .clone()
                        .any(|i| a[i * 2..i * 2 + 2] != b[i * 2..i * 2 + 2])
                })
                .count()
        };
        let non_clear = results[2]
            .chunks_exact(8)
            .filter(|p| *p != [0x00, 0x34, 0x00, 0x38, 0x00, 0x3A, 0x00, 0x39])
            .count();
        rows.push(json!({"size":size,"camera_height":height,"blocked_by_depth":blocked,"back_facing":reversed,
            "pixels":size[0]*size[1],"native_state_oracle_different_rgb":delta(1,0..3),"native_state_oracle_different_alpha":delta(1,3..4),
            "original_port_different_alpha":delta(0,3..4),"non_clear_pixels":non_clear}));
    }
    Ok(
        json!({"scope":"production strait adapter; synthetic scene b0, float4/float4 native geometry, three physical texture slots, authored state on D24S8; no native replay claim", "results":rows}),
    )
}
