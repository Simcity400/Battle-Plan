//! Production railway adapter versus the old pixel kernel and native strip/state controls.
use super::border::compiled as border_shader;
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/rail_shader.rs"]
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
    let native = compiled::Rail::new(device, &effect, 1, format, depth_format)?;
    let modules = std::array::from_fn::<_, 2, _>(|mode| {
        let source = include_str!("rail_port.wgsl").replace("\r\n", "\n");
        let source = if mode == 1 {
            source
                .replace(
                    "@location(1) world: vec3<f32>,",
                    "@location(1) world: vec3<f32>,\n    @location(2) screen: vec4<f32>,",
                )
                .replace(
                    "    out.uv = uv;",
                    "    out.uv = uv;\n    out.screen = screen_coord(out.clip);",
                )
                .replace(
                    "apply_fow_buffer(color, input.clip)",
                    "apply_fow_projective(color, input.screen)",
                )
                // Native wraps translate the camera; local map UV is not remapped.
                .replace("let map_uv = wrap_u(vec2<f32>(", "let map_uv = vec2<f32>(")
                .replace("    ));\n    let border", "    );\n    let border")
        } else {
            source.to_owned()
        };
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("railway port oracle"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", include_str!("world_port.wgsl"), source).into(),
            ),
        })
    });
    let old = std::array::from_fn::<_, 2, _>(|mode| {
        let module = &modules[mode];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("railway native strip control"),
            layout: None,
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("rail_vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 16,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: if mode == 0 {
                    wgpu::PrimitiveTopology::TriangleList
                } else {
                    wgpu::PrimitiveTopology::TriangleStrip
                },
                front_face: wgpu::FrontFace::Cw,
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
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("rail_fragment"),
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
    let texture = |label, color: [u8; 4], cube| {
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
            &color.repeat((16 * depth) as usize),
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
    let diffuse = texture("railway diffuse", [71, 113, 167, 231], false);
    let snow = texture("snow mud", [31, 191, 113, 51], false);
    let gradient1 = texture("gradient color", [99, 167, 51, 255], false);
    let gradient2 = texture("gradient coverage", [109, 117, 13, 0], false);
    let secondary = texture("occupation", [193, 31, 77, 99], false);
    let zero = texture("explicit null", [0; 4], false);
    let reflection = texture("retained reflection", [0; 4], true);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for (case, (size, alpha, blocked, reversed)) in [
        ([64, 48], 1_f32, false, false),
        ([63, 47], 0.35, false, false),
        ([64, 48], 0.0, false, false),
        ([64, 48], 1.0, true, false),
        ([64, 48], 1.0, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let vertices: Vec<[f32; 6]> = [0, 0, 0, 1, 2, 3, 3, 3]
            .iter()
            .map(|&i| {
                let (x, z, u, v) = [
                    (-0.85_f32, 0.75_f32, 0_f32, -0.1_f32),
                    (0.8, 0.75, 0.0, 1.1),
                    (-0.85, -0.9, 3.0, -0.1),
                    (0.8, -0.9, 3.0, 1.1),
                ][i];
                [x * if reversed { -1.0 } else { 1.0 }, 20.0, z, 1.0, u, v]
            })
            .collect();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[2_u32, 3, 4, 4, 3, 5]),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut globals = [0_f32; 140];
        globals[0] = 1.0;
        globals[9] = 1.0;
        globals[14] = 0.5;
        globals[15] = 1.0;
        globals[32..36].copy_from_slice(&[0.0, 0.0, 0.5, 0.5]);
        globals[53] = 120.0;
        globals[54] = 120.0;
        globals[55] = 1.0;
        globals[62] = 0.6;
        globals[100] = map[1] as f32;
        globals[136] = map[0] as f32;
        globals[137] = map[1] as f32;
        let global = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let b1 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0_f32, alpha, 0.0, 0.0]),
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
        let data: Vec<u8> = (0..size[0] * size[1])
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
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: None,
            },
            shadow.size(),
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
                            resource: wgpu::BindingResource::Sampler(&clamp),
                        },
                    ],
                });
                let views = [&diffuse, &snow, &gradient1, &gradient2, &secondary];
                let mut entries: Vec<_> = views
                    .iter()
                    .enumerate()
                    .map(|(i, v)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: wgpu::BindingResource::TextureView(v),
                    })
                    .collect();
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&clamp),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: b1.as_entire_binding(),
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
                if mode == 0 {
                    pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..6, 0, 0..1);
                } else {
                    pass.draw(0..8, 0..1);
                }
            } else {
                native.draw(
                    &arena,
                    &mut encoder,
                    attachments,
                    compiled::Inputs {
                        retained: None,
                        frames: &[bytemuck::cast_slice(&globals[..104])],
                        shadow_matrix: &[[0.0; 4]; 4],
                        time: 1.7,
                        alpha,
                        retained_pixel_b2: crate::shader_constants::WorldPixelConstants::LightGrid(
                            [0.; 4],
                        ),
                        textures: [
                            &diffuse,
                            &zero,
                            &zero,
                            &zero,
                            &zero,
                            &zero,
                            &secondary,
                            &zero,
                            &snow,
                            &zero,
                            &zero,
                            &zero,
                            &gradient1,
                            &gradient2,
                            &reflection,
                            &shadow,
                        ],
                    },
                    compiled::Geometry {
                        vertices: &vertices,
                        runs: std::slice::from_ref(&(0..8)),
                    },
                )?;
            }
            let bytes = read(&arena, queue, encoder, &target, size)?;
            fs::write(output.join(format!("rail-{case}-{mode}.rgba16f")), &bytes)?;
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
        rows.push(json!({"size":size,"pixels":size[0]*size[1],"alpha":alpha,"depth_in_front":blocked,"back_facing":reversed,
            "native_state_oracle_different_pixels":differences(1),"old_state_different_pixels":differences(0),"non_clear_pixels":non_clear}));
    }
    Ok(
        json!({"scope":"production railway adapter; native strip/winding, snow, occupation/gradient and screen FOW; synthetic textures; OWB include changes may differ","results":rows}),
    )
}
