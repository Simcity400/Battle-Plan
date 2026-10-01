//! Native tree caster against the retired arithmetic with native declaration/state/inputs.
use super::{extent, read};
use crate::shader_constants;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/tree_caster.rs"]
pub(super) mod compiled;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map, crate::DECODE)?;
    let caster = compiled::Caster::new(device, &effect, FORMAT, DEPTH)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("retired tree caster oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("tree_caster_port.wgsl").into()),
    });
    let attrs = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 40,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 48,
            shader_location: 4,
        },
    ];
    let instances = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 0,
            shader_location: 5,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 16,
            shader_location: 6,
        },
    ];
    let oracle = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tree caster native-input oracle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: 56,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instances,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Cw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("pixel_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let source = |label, pixels: &[u8]| {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent([8, 8]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            tex.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(8),
            },
            extent([8, 8]),
        );
        tex.create_view(&Default::default())
    };
    let sampler = |address| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: address,
            address_mode_v: address,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        })
    };
    let wrap = sampler(wgpu::AddressMode::Repeat);
    let clamp = sampler(wgpu::AddressMode::ClampToEdge);
    let null = source("tree null", &[0; 256]);
    let center = [map[0] as f32 * 0.375, 4., map[1] as f32 * 0.5];
    let matrix = [
        [0.08, 0., 0., 0.],
        [0., 0., 0.03, 0.],
        [0., 0.08, 0., 0.],
        [-center[0] * 0.08, -center[2] * 0.08, 0.2, 1.],
    ];
    let mut constants = bytemuck::bytes_of(&matrix).to_vec();
    constants.extend_from_slice(bytemuck::cast_slice(&[
        map[0] as f32,
        map[1] as f32,
        0.,
        0.,
    ]));
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tree oracle constants"),
        contents: &constants,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let mut rows = Vec::new();
    for size in [[64, 40], [63, 39]] {
        for (name, alpha, mask_alpha, angle, depth_clear, reverse) in [
            ("opaque", 255, 0, 0., 1., false),
            ("cutout", 0, 0, 0.7, 1., false),
            ("mask_below", 255, 25, 0.7, 1., false),
            ("mask_above", 255, 26, 0., 1., false),
            ("depth_reject", 255, 0, 0., 0., false),
            ("backface", 255, 0, 0., 1., true),
        ] {
            let diffuse_pixels: Vec<u8> = (0..64)
                .flat_map(|i| {
                    [
                        255,
                        128,
                        64,
                        if alpha == 0 {
                            if i % 3 == 0 { 0 } else { 255 }
                        } else {
                            alpha
                        },
                    ]
                })
                .collect();
            let diffuse = source("tree diffuse", &diffuse_pixels);
            let mask = source("native A8 tree mask", &[0, 0, 0, mask_alpha].repeat(64));
            let bound = caster.bindings(&arena, matrix, &diffuse, &mask, &null)?;
            let old = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree caster oracle"),
                layout: &oracle.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&diffuse),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&mask),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&wrap),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&clamp),
                    },
                ],
            });
            let points = [[-9., 2., -8.], [-9., 4., 8.], [9., 6., -8.]];
            let mut vertices = Vec::<f32>::new();
            for i in if reverse { [0, 2, 1] } else { [0, 1, 2] } {
                vertices.extend_from_slice(&points[i]);
                vertices.extend_from_slice(&[0., 1., 0., 1., 0., 0., 1.]);
                vertices.extend_from_slice(&[[0., 0.], [0., 3.], [3., 0.]][i]);
                vertices.extend_from_slice(&[0.25, 0.75]);
            }
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree fixture vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("native tree instance"),
                contents: bytemuck::cast_slice(&[
                    center[0], center[1], center[2], angle, 0.04, -0.06,
                ]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mut results = Vec::new();
            for mode in 0..2 {
                let texture = |format| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("tree caster fixture target"),
                        size: extent(size),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                };
                let target = texture(FORMAT);
                let target_view = target.create_view(&Default::default());
                let depth = texture(DEPTH);
                let depth_view = depth.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("tree caster fixture"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(depth_clear),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if mode == 0 {
                        pass.set_pipeline(&caster.pipeline.pipeline);
                        bound.bind(&mut pass);
                    } else {
                        pass.set_pipeline(&oracle);
                        pass.set_bind_group(0, &old, &[]);
                    }
                    pass.set_blend_constant(wgpu::Color::WHITE);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                    pass.set_vertex_buffer(1, instances.slice(..));
                    pass.draw(0..3, 0..1);
                }
                results.push(read(&arena, queue, encoder, &target, size)?);
            }
            let different = results[0]
                .chunks_exact(2)
                .zip(results[1].chunks_exact(2))
                .filter(|(a, b)| a != b)
                .count();
            let active = results[0]
                .chunks_exact(2)
                .filter(|p| *p != [0, 0x3c])
                .count();
            if different != 0 {
                fs::write(
                    output.join(format!("tree-caster-{name}-{}-native.bin", size[0])),
                    &results[0],
                )?;
                fs::write(
                    output.join(format!("tree-caster-{name}-{}-port.bin", size[0])),
                    &results[1],
                )?;
            }
            rows.push(json!({"case":name,"size":size,"pixels":size[0]*size[1],"different":different,"active":active}));
        }
    }
    let result = json!({"scope":"native-state tree caster versus retired arithmetic; synthetic geometry, not historical game proof","cases":rows});
    fs::write(
        output.join("tree-caster.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
