//! Native receiver versus the old arithmetic with native mip selection and projective input.
use super::tree_caster::compiled as tree_caster;
use super::{extent, read};
use crate::shader_constants;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::binding;
use std::{collections::BTreeMap, fs, io, path::Path};
#[path = "../../src/map/tree_receiver.rs"]
mod compiled;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = tree_caster::load(layers, map, crate::DECODE)?;
    let receiver = compiled::Receiver::new(device, &effect, FORMAT, DEPTH)?;
    let common = include_str!("world_port.wgsl")
        .replace(
            "textureSampleLevel(fow_noise, fow_sampler, uv, 0.0)",
            "textureSample(fow_noise, fow_sampler, uv)",
        )
        .replace(
            "textureSampleLevel(fow_extra, fow_sampler, uv, 0.0)",
            "textureSample(fow_extra, fow_sampler, uv)",
        )
        .replace(
            "textureSampleLevel(intel_map, fow_sampler, map_uv_bottom_up(world), 0.0)",
            "textureSample(intel_map, fow_sampler, map_uv_bottom_up(world))",
        );
    let kernel = include_str!("tree_receiver_port.wgsl").replace(
        "calculate_shadow(shadow_proj(input.world))",
        "calculate_shadow(input.shadow_projection)",
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tree receiver native-input oracle"),
        source: wgpu::ShaderSource::Wgsl(
            format!("diagnostic(off, derivative_uniformity);\n{common}\n{kernel}").into(),
        ),
    });
    let vertices_layout =
        wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4,3=>Float32x2,4=>Float32x2];
    let instances_layout = wgpu::vertex_attr_array![5=>Float32x3,6=>Float32,7=>Float32x2];
    let oracle = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tree receiver oracle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("tree_vertex"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: 56,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertices_layout,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instances_layout,
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
            entry_point: Some("tree_unlit_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::COLOR,
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
            format: wgpu::TextureFormat::Rgba8Unorm,
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
    let noise = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("distinct native fog mip levels"),
        size: extent([512, 512]),
        mip_level_count: 4,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, color) in [
        [0, 64, 0, 220],
        [0, 220, 0, 64],
        [0, 96, 0, 160],
        [0, 180, 0, 180],
    ]
    .iter()
    .enumerate()
    {
        let side = 512 >> mip;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &noise,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &color.repeat(side * side),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side as u32 * 4),
                rows_per_image: Some(side as u32),
            },
            extent([side as u32; 2]),
        );
    }
    let noise = noise.create_view(&Default::default());
    let null = source("tree null", &[0; 256]);
    let intel = source("A8 intel", &[0, 0, 0, 255].repeat(64));
    let extra = source("extra fog", &[0, 170, 0, 0].repeat(64));
    let shadow = source(
        "shadow depth test pattern",
        &(0..64)
            .flat_map(|i| [if i % 3 == 0 { 255 } else { 64 }, 0, 0, 255])
            .collect::<Vec<_>>(),
    );
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
    let center = [map[0] as f32 * 0.375, 4., map[1] as f32 * 0.5];
    let matrix = [
        [0.08, 0., 0., 0.],
        [0., 0., 0.03, 0.],
        [0., 0.08, 0., 0.],
        [-center[0] * 0.08, -center[2] * 0.08, 0.2, 1.],
    ];
    let shadow_matrix = [
        [0.04, 0., 0., 0.],
        [0., 0., 0.03, 0.],
        [0., -0.04, 0., 0.],
        [0.5 - center[0] * 0.04, 0.5 + center[2] * 0.04, 0.2, 1.],
    ];
    let mut rows = Vec::new();
    for size in [[64, 40], [63, 39]] {
        for (name, height, mask_alpha, angle, depth_clear, reverse) in [
            ("near_cutout", 40., 0, 0., 1., false),
            ("high_no_cutout", 120., 0, 0.7, 1., false),
            ("mask_below", 40., 43, 0.7, 1., false),
            ("mask_above", 40., 44, 0., 1., false),
            ("depth_reject", 40., 0, 0., 0., false),
            ("backface", 40., 0, 0., 1., true),
        ] {
            let diffuse = source(
                "tree cutout",
                &(0..64)
                    .flat_map(|i| [255, 128, 64, if i % 3 == 0 { 0 } else { 255 }])
                    .collect::<Vec<_>>(),
            );
            let mask = source("A8 tree mask", &[0, 0, 0, mask_alpha].repeat(64));
            let mut scene = tree_caster::constants(matrix);
            for (offset, value) in [
                (208, center[0]),
                (212, height),
                (216, center[2]),
                (220, 1.),
                (244, 2.5),
                (300, 1.),
            ] {
                scene[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            let mut globals = scene.clone();
            globals.resize(560, 0);
            globals[464..528].copy_from_slice(bytemuck::bytes_of(&shadow_matrix));
            globals[544..552]
                .copy_from_slice(bytemuck::cast_slice(&[map[0] as f32, map[1] as f32]));
            let global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree oracle scene"),
                contents: &globals,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let tree_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree oracle b1"),
                contents: bytemuck::cast_slice(&[0_f32, 0.375, 0., 0.]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let mut textures: BTreeMap<_, _> = (0..16)
                .map(|slot| (binding(slot, Stage::Pixel), null.clone()))
                .collect();
            for (slot, view) in [
                (0, &diffuse),
                (2, &noise),
                (3, &extra),
                (4, &intel),
                (8, &shadow),
                (14, &mask),
            ] {
                textures.insert(binding(slot, Stage::Pixel), view.clone());
            }
            let bound = receiver.bindings(&arena, &scene, shadow_matrix, 0.375, &textures)?;
            let old0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree receiver oracle scene"),
                layout: &oracle.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: global_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&shadow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&wrap),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&intel),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&noise),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&extra),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&wrap),
                    },
                ],
            });
            let old1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree receiver oracle textures"),
                layout: &oracle.get_bind_group_layout(1),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&diffuse),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&wrap),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: wgpu::BindingResource::Sampler(&clamp),
                    },
                    wgpu::BindGroupEntry {
                        binding: 13,
                        resource: tree_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 14,
                        resource: wgpu::BindingResource::TextureView(&mask),
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
                label: Some("tree receiver geometry"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree receiver instances"),
                contents: bytemuck::cast_slice(&[
                    center[0], center[1], center[2], angle, 0.04, -0.06,
                ]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mut results = Vec::new();
            for mode in 0..2 {
                let texture = |format| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("tree receiver fixture target"),
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
                        label: Some("tree receiver fixture"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0,
                                    g: 0.0,
                                    b: 1.0,
                                    a: 1.0,
                                }),
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
                        pass.set_pipeline(&receiver.pipeline.pipeline);
                        bound.bind(&mut pass);
                    } else {
                        pass.set_pipeline(&oracle);
                        pass.set_bind_group(0, &old0, &[]);
                        pass.set_bind_group(1, &old1, &[]);
                    }
                    pass.set_blend_constant(wgpu::Color::WHITE);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                    pass.set_vertex_buffer(1, instances.slice(..));
                    pass.draw(0..3, 0..1);
                }
                results.push(read(&arena, queue, encoder, &target, size)?);
            }
            let different = results[0]
                .chunks_exact(4)
                .zip(results[1].chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let active = results[0]
                .chunks_exact(4)
                .filter(|p| *p != [255, 0, 255, 255])
                .count();
            let rejected = matches!(name, "mask_above" | "depth_reject" | "backface");
            if (rejected && active != 0) || (!rejected && active == 0) {
                return Err(io::Error::other(format!(
                    "tree receiver fixture {name}/{size:?}: unexpected active pixel count {active}"
                )));
            }
            if different != 0 {
                fs::write(
                    output.join(format!("tree-receiver-{name}-{}-native.bin", size[0])),
                    &results[0],
                )?;
                fs::write(
                    output.join(format!("tree-receiver-{name}-{}-port.bin", size[0])),
                    &results[1],
                )?;
            }
            rows.push(json!({"case":name,"size":size,"pixels":size[0]*size[1],"different":different,"active":active}));
        }
    }
    let result = json!({"scope":"production tree receiver versus native-input old arithmetic; synthetic mipped fog, mask, shadow and geometry; OWB differences may be authored","cases":rows});
    fs::write(
        output.join("tree-receiver.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
