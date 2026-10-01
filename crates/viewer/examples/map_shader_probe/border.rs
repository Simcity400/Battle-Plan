//! Actual border adapter with selection/fade, shadow/FOW, depth and winding controls.
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/border_shader.rs"]
pub(super) mod compiled;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map, crate::DECODE)?;
    let format = wgpu::TextureFormat::Rgba16Float;
    let depth_format = wgpu::TextureFormat::Depth24PlusStencil8;
    let native = compiled::Border::new(device, &effect, 1, format, depth_format)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("border port oracle"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}",
                include_str!("world_port.wgsl"),
                include_str!("border_port.wgsl")
            )
            .into(),
        ),
    });
    let old = std::array::from_fn::<_, 2, _>(|mode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("border cull control"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("border_vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Sint16x2],
                })],
            },
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: if mode == 0 {
                    None
                } else {
                    Some(wgpu::Face::Back)
                },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("border_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    });
    let diffuse = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: extent([16, 16]),
        mip_level_count: 5,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for mip in 0..5 {
        let n = 16 >> mip;
        let bytes: Vec<_> = (0..n * n)
            .flat_map(|i| {
                [
                    (i * 13 + mip * 7) as u8,
                    (i * 37 + 29) as u8,
                    (i * 7 + mip * 53) as u8,
                    (i * 11 + 31) as u8,
                ]
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &diffuse,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n * 4),
                rows_per_image: None,
            },
            extent([n, n]),
        );
    }
    let diffuse = diffuse.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        lod_max_clamp: 4.0,
        ..Default::default()
    });
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for (case, (size, alpha, selection, time, blocked, reversed)) in [
        ([64, 48], 1.0, [0.0; 4], 0.0, false, false),
        ([63, 47], 0.35, [0.2, 0.7, 0.4, 0.8], 0.3, false, false),
        ([64, 48], 1.0, [0.7, 0.1, 0.2, 1.0], 2.0, false, false),
        ([64, 48], 1.0, [0.0; 4], 0.0, true, false),
        ([64, 48], 1.0, [0.0; 4], 0.0, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut vertices = Vec::new();
        for (x, z, u, v) in [
            (-0.85_f32, 0.75_f32, 0_i16, 0_i16),
            (0.8, 0.75, 1, 0),
            (-0.85, -0.9, 0, 24),
            (0.8, -0.9, 1, 24),
        ] {
            vertices.extend_from_slice(bytemuck::cast_slice(&[
                x * if reversed { -1.0 } else { 1.0 },
                20.0,
                z,
            ]));
            vertices.extend_from_slice(bytemuck::cast_slice(&[u, v]));
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &vertices,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0_u32, 3, 1, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut globals = [0_f32; 140];
        globals[0] = 1.0;
        globals[9] = 1.0;
        globals[14] = 0.5;
        globals[15] = 1.0;
        globals[53] = 120.0;
        globals[54] = 120.0;
        globals[55] = 1.0;
        globals[71] = 1.0;
        globals[136] = map[0] as f32;
        globals[137] = map[1] as f32;
        let global = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let b2 = [
            selection[0],
            selection[1],
            selection[2],
            selection[3],
            time,
            alpha,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        let b2 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&b2),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let shadow_bytes: Vec<u8> = (0..size[0] * size[1])
            .flat_map(|i| {
                [
                    (i % 255) as u8,
                    ((i * 7) % 127) as u8,
                    ((i * 3) % 127) as u8,
                    255,
                ]
            })
            .collect();
        queue.write_texture(
            shadow.as_image_copy(),
            &shadow_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: None,
            },
            extent(size),
        );
        let shadow = shadow.create_view(&Default::default());
        let mut results = Vec::new();
        for (mode, oracle) in old.iter().map(Some).chain([None]).enumerate() {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
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
            let attachments = compiled::Target {
                color: wgpu::RenderPassColorAttachment {
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
                },
                depth: wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if blocked { 0.1 } else { 1.0 }),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0xA5),
                        store: wgpu::StoreOp::Store,
                    }),
                },
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            if let Some(oracle) = oracle {
                let frame = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &oracle.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: global.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&shadow),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                        },
                    ],
                });
                let material = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &oracle.get_bind_group_layout(1),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&diffuse),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: b2.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(attachments.color)],
                    depth_stencil_attachment: Some(attachments.depth),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(oracle);
                pass.set_bind_group(0, &frame, &[]);
                pass.set_bind_group(1, &material, &[]);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            } else {
                native.draw(
                    &arena,
                    &mut encoder,
                    attachments,
                    compiled::Inputs {
                        retained: None,
                        frames: &[bytemuck::cast_slice(&globals[..104])],
                        snow_mud: &diffuse,
                        intel: &diffuse,
                        shadow: &shadow,
                        time,
                        selection,
                    },
                    &[compiled::Batch {
                        texture: &diffuse,
                        alpha,
                        runs: vec![compiled::Run {
                            vertices: &vertices,
                            indices: &indices,
                            count: 6,
                        }],
                    }],
                )?;
            }
            let bytes = read(&arena, queue, encoder, &target, size)?;
            fs::write(output.join(format!("border-{case}-{mode}.rgba16f")), &bytes)?;
            results.push(bytes);
        }
        let differences = |mode: usize| {
            results[mode]
                .chunks_exact(8)
                .zip(results[2].chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count()
        };
        let non_clear = results[2]
            .chunks_exact(8)
            .filter(|p| *p != [0, 0x34, 0, 0x38, 0, 0x3a, 0, 0x39])
            .count();
        rows.push(json!({"size":size,"pixels":size[0]*size[1],"selection":selection,"time":time,"alpha":alpha,"blocked":blocked,"back_facing":reversed,
            "native_state_oracle_different_pixels":differences(1),"old_dmz_different_pixels":differences(0),"non_clear_pixels":non_clear}));
    }
    Ok(
        json!({"scope":"production border adapter vs original kernel; synthetic b0/b2, mipmapped diffuse, varying screen shadow/fow; OWB differences may be authored","results":rows}),
    )
}
