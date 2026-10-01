//! Actual river adapter with native flow directions, shadow/FOW and culling controls.
use super::border::compiled as border_shader;
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/river_shader.rs"]
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
    let format = wgpu::TextureFormat::Rgba16Float;
    let depth_format = wgpu::TextureFormat::Depth24PlusStencil8;
    let native = [
        compiled::River::new(device, &effect, 1, format, depth_format, true)?,
        compiled::River::new(device, &effect, 1, format, depth_format, false)?,
    ];
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("river port oracle"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}",
                include_str!("world_port.wgsl"),
                include_str!("river_port.wgsl")
            )
            .into(),
        ),
    });
    let old = std::array::from_fn::<_, 2, _>(|mode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("river cull control"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("river_vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4],
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
                entry_point: Some("river_fragment"),
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
    let texture = |label: &str, color: [u8; 4], cube: bool| {
        let depth = if cube { 6 } else { 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: depth,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &(0..16 * depth)
                .flat_map(|i| {
                    if label == "lean" {
                        [
                            110 + (i % 4 * 11) as u8,
                            117 + (i / 4 % 4 * 7) as u8,
                            192,
                            128,
                        ]
                    } else {
                        color
                    }
                })
                .collect::<Vec<_>>(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(4),
            },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(if cube {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        })
    };
    let diffuse = texture("diffuse", [71, 113, 167, 231], false);
    let normal = texture("normal", [128, 128, 255, 255], false);
    let lean = texture("lean", [128, 128, 192, 192], false);
    let masks = texture("river masks", [217, 0, 119, 255], false);
    let zero = texture("zero", [0; 4], false);
    let light_indices = texture("no point lights", [255; 4], false);
    let active_indices = texture("one point light", [0, 255, 255, 255], false);
    let light_data = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native point-light float32 data"),
        size: extent([128, 1]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut values = vec![0_f32; 128 * 4];
    values[..8].copy_from_slice(&[0.2, 25.0, -0.4, 50.0, 1.7, 0.4, 0.8, 35.0]);
    queue.write_texture(
        light_data.as_image_copy(),
        bytemuck::cast_slice(&values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(128 * 16),
            rows_per_image: None,
        },
        light_data.size(),
    );
    let light_data = light_data.create_view(&Default::default());
    let reflection = texture("reflection", [101, 179, 223, 255], true);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let strip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for (case, (size, time, direction, blocked, reversed, high, lights)) in [
        ([64, 48], 0.0_f32, -1.0_f32, false, false, true, false),
        ([63, 47], 1.7, 1.0, false, false, true, false),
        ([64, 48], 1.7, -1.0, false, false, true, false),
        ([64, 48], 0.0, -1.0, true, false, true, false),
        ([64, 48], 0.0, -1.0, false, true, true, false),
        ([64, 48], 0.0, -1.0, false, false, false, false),
        ([64, 48], 1.7, 1.0, false, false, false, false),
        ([64, 48], 1.7, -1.0, false, false, false, false),
        ([64, 48], 0.0, -1.0, false, false, true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let light_indices = if lights {
            &active_indices
        } else {
            &light_indices
        };
        let quality = usize::from(!high);
        let mut vertices = Vec::new();
        for (x, z, u, v) in [
            (-0.85_f32, 0.75_f32, 0.0_f32, 0.0_f32),
            (0.8, 0.75, 1.0, 0.0),
            (-0.85, -0.9, 0.0, 24.0),
            (0.8, -0.9, 1.0, 24.0),
        ] {
            vertices.extend_from_slice(bytemuck::cast_slice(&[
                x * if reversed { -1.0 } else { 1.0 },
                20.0,
                z,
                0.85,
            ]));
            vertices.extend_from_slice(bytemuck::cast_slice(&[u, v, 1.0, 0.0]));
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
        globals[36..39].copy_from_slice(&[0.3, 0.8, 0.5]);
        globals[40..43].copy_from_slice(&[-0.2, 0.9, 0.4]);
        globals[44..47].copy_from_slice(&[0.4, 0.7, -0.3]);
        globals[48..51].copy_from_slice(&[-0.3, 0.7, -0.4]);
        globals[60] = 1.0;
        for first in [68, 72, 76, 80, 84, 88] {
            globals[first..first + 3].fill(0.2);
        }
        globals[92..100].copy_from_slice(&[1.0, 0.9, 0.8, 1.0, 0.3, 0.4, 0.6, 0.7]);
        globals[100] = map[1] as f32;
        globals[101] = 1.0;
        globals[136] = map[0] as f32;
        globals[137] = map[1] as f32;
        let global = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let b2 = [0_f32, time * direction, f32::from(!high), 0.0];
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
            let attachments = border_shader::Target {
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
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(light_indices),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::TextureView(&light_data),
                        },
                    ],
                });
                let views = [
                    &diffuse,
                    &diffuse,
                    &normal,
                    &masks,
                    &lean,
                    &lean,
                    &zero,
                    &diffuse,
                    &zero,
                    &zero,
                    &reflection,
                ];
                let mut entries: Vec<_> = views
                    .iter()
                    .enumerate()
                    .map(|(slot, view)| wgpu::BindGroupEntry {
                        binding: slot as u32,
                        resource: wgpu::BindingResource::TextureView(view),
                    })
                    .collect();
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 13,
                        resource: wgpu::BindingResource::Sampler(&strip_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 14,
                        resource: b2.as_entire_binding(),
                    },
                ]);
                let material = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &oracle.get_bind_group_layout(1),
                    entries: &entries,
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
                native[quality].draw(
                    &arena,
                    &mut encoder,
                    attachments,
                    compiled::Inputs {
                        retained: None,
                        frames: &[bytemuck::cast_slice(&globals[..104])],
                        shadow_matrix: &[[0.0; 4]; 4],
                        time,
                        retained_pixel_b2: crate::shader_constants::WorldPixelConstants::LightGrid(
                            [0.; 4],
                        ),
                        textures: [
                            &diffuse,
                            &diffuse,
                            &normal,
                            &masks,
                            &lean,
                            &lean,
                            &zero,
                            &normal,
                            &zero,
                            &diffuse,
                            light_indices,
                            &light_data,
                            &zero,
                            &zero,
                            &reflection,
                            &shadow,
                        ],
                    },
                    compiled::Geometry {
                        vertices: &vertices,
                        indices: &indices,
                        runs: &[compiled::Run {
                            indices: 0..6,
                            direction,
                        }],
                    },
                )?;
            }
            let bytes = read(&arena, queue, encoder, &target, size)?;
            fs::write(output.join(format!("river-{case}-{mode}.rgba16f")), &bytes)?;
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
        rows.push(json!({"high_gfx":high,"point_light":lights,"size":size,"pixels":size[0]*size[1],"direction":direction,"time":time,"blocked":blocked,"back_facing":reversed,
            "native_state_oracle_different_pixels":differences(1),"old_no_cull_different_pixels":differences(0),"non_clear_pixels":non_clear}));
    }
    Ok(
        json!({"scope":"production river adapter vs original kernel with native direction/b2 and culling; patterned LEAN textures, RGBA32Float point lights and scene constants; OWB differences may be authored","results":rows}),
    )
}
