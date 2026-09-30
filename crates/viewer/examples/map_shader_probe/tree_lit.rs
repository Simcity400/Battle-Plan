//! Lit-tree oracle with native sampler, quality, season and projective inputs.
use super::tree_caster::compiled as tree_caster;
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::binding;
use std::{collections::BTreeMap, fs, io, path::Path};
#[allow(dead_code)]
#[path = "../../src/map/tree_shader.rs"]
mod compiled;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = tree_caster::load(layers, map)?;
    let common = include_str!("world_port.wgsl");
    let kernel = include_str!("tree_receiver_port.wgsl")
        .replace("@location(8) shadow_projection: vec4<f32>,", "@location(8) shadow_projection: vec4<f32>,\n    @location(9) screen: vec4<f32>,")
        .replace("out.shadow_projection = shadow_proj(p);", "out.shadow_projection = shadow_proj(p);\n    out.screen = screen_coord(out.position);")
        .replace("get_shadow_scaled(SHADOW_WEIGHT_TREE, input.position)", "get_shadow_scaled_projective(SHADOW_WEIGHT_TREE, input.screen)")
        .replace("apply_fow_buffer(out, input.position)", "apply_fow_projective(out, input.screen)")
        .replace("    let snow = apply_snow_tree(color, mud_snow);", "    var snow = TreeSnow(color, 0.0);\n    if globals.counts.z == 0u { snow = apply_snow_tree(color, mud_snow); }")
        .replace("    light = calculate_point_lights(properties, light, false, vec4<f32>(0.0));", "    if globals.counts.z == 0u { light = calculate_point_lights(properties, light, false, vec4<f32>(0.0)); }")
        .replace("    diffuse_light += reflective_color * TREE_SPECULAR;", "    if globals.counts.z == 0u { diffuse_light += reflective_color * TREE_SPECULAR; }")
        .replace("    out = apply_distance_fog(out, input.world);", "    if globals.counts.z == 0u { out = apply_distance_fog(out, input.world); }");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tree lit native-input oracle"),
        source: wgpu::ShaderSource::Wgsl(
            format!("diagnostic(off, derivative_uniformity);\n{common}\n{kernel}").into(),
        ),
    });
    let vertices_layout =
        wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4,3=>Float32x2,4=>Float32x2];
    let instances_layout = wgpu::vertex_attr_array![5=>Float32x3,6=>Float32,7=>Float32x2];
    let oracle = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tree lit oracle"),
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
            entry_point: Some("tree_fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
    let colormap = source("colormap", &[100, 140, 110, 255].repeat(64));
    let normal = source("normal", &[128, 255, 128, 255].repeat(64));
    let light_indices = source("no point lights", &[255; 256]);
    let active_indices = source("one point light", &[0, 255, 255, 255].repeat(64));
    let cube = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tree reflection cube"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        cube.as_image_copy(),
        &[64, 96, 128, 255].repeat(6),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        cube.size(),
    );
    let cube = cube.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let light_data = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tree native light data"),
        size: extent([128, 1]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut light_values = vec![0_f32; 512];
    light_values[..8].copy_from_slice(&[
        map[0] as f32 * 0.375 + 5.,
        10.,
        map[1] as f32 * 0.5 + 50.,
        150.,
        1.7,
        0.4,
        0.8,
        150.,
    ]);
    queue.write_texture(
        light_data.as_image_copy(),
        bytemuck::cast_slice(&light_values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(2048),
            rows_per_image: Some(1),
        },
        extent([128, 1]),
    );
    let light_data = light_data.create_view(&Default::default());
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
    let mut rows = Vec::new();
    for (size, high) in [
        ([64, 40], true),
        ([63, 39], true),
        ([64, 40], false),
        ([63, 39], false),
    ] {
        let receiver = compiled::Lit::new(device, &effect, FORMAT, DEPTH, 1, high)?;
        let mut ordinary = Vec::new();
        for (name, height, mask_alpha, angle, depth_clear, reverse) in [
            ("near_cutout", 40., 0, 0., 1., false),
            ("high_no_cutout", 120., 0, 0.7, 1., false),
            ("mask_below", 40., 43, 0.7, 1., false),
            ("snow", 40., 0, 0., 1., false),
            ("point_light", 40., 0, 0., 1., false),
            ("mask_above", 40., 44, 0., 1., false),
            ("depth_reject", 40., 0, 0., 0., false),
            ("backface", 40., 0, 0., 1., true),
        ] {
            let light_indices = if name == "point_light" {
                &active_indices
            } else {
                &light_indices
            };
            let snow = source(
                "snow/mud",
                &if name == "snow" {
                    [0, 223, 223, 0]
                } else {
                    [0; 4]
                }
                .repeat(64),
            );
            let diffuse = source(
                "tree cutout",
                &(0..64)
                    .flat_map(|i| [255, 128, 64, if i % 3 == 0 { 0 } else { 255 }])
                    .collect::<Vec<_>>(),
            );
            let mask = source("A8 tree mask", &[0, 0, 0, mask_alpha].repeat(64));
            let mut globals = [0_f32; 140];
            globals[..16].copy_from_slice(bytemuck::cast_slice(&matrix));
            globals[52..56].copy_from_slice(&[center[0], height, center[2], 1.]);
            globals[71] = 1.;
            globals[83] = 1.;
            globals[60] = 1.;
            for first in [36, 40, 44, 48] {
                globals[first..first + 4].copy_from_slice(&[0.3, 0.8, 0.5, 1200.]);
            }
            for first in [68, 72, 76, 80, 84, 88] {
                globals[first..first + 3].fill(0.2);
            }
            globals[92..100].copy_from_slice(&[1., 0.9, 0.8, 1., 0.3, 0.4, 0.6, 0.7]);
            globals[100] = map[1] as f32;
            globals[101] = 1.;
            globals[136] = map[0] as f32;
            globals[137] = map[1] as f32;
            globals[110] = f32::from_bits(u32::from(!high));
            globals[113] = 0.5 / size[0] as f32;
            globals[114] = 0.5 / size[1] as f32;
            let scene = bytemuck::cast_slice(&globals[..104]);
            let global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree oracle scene"),
                contents: bytemuck::cast_slice(&globals),
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
                (4, &colormap),
                (6, &snow),
                (1, &normal),
                (9, &cube),
                (10, light_indices),
                (11, &light_data),
                (8, &shadow),
                (14, &mask),
            ] {
                textures.insert(binding(slot, Stage::Pixel), view.clone());
            }
            let bound = receiver.bindings(
                &arena,
                scene,
                0.375,
                crate::shader_constants::WorldPixelConstants::LightGrid([0.; 4]),
                &textures,
            )?;
            let old0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree lit oracle scene"),
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
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(light_indices),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&light_data),
                    },
                ],
            });
            let old1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree lit oracle textures"),
                layout: &oracle.get_bind_group_layout(1),
                entries: &[
                    (0, &diffuse),
                    (1, &normal),
                    (2, &noise),
                    (3, &extra),
                    (4, &colormap),
                    (5, &null),
                    (6, &snow),
                    (7, &null),
                    (8, &null),
                    (9, &null),
                    (10, &cube),
                    (14, &mask),
                ]
                .into_iter()
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                })
                .chain([
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
                ])
                .collect::<Vec<_>>(),
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
                label: Some("tree lit geometry"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tree lit instances"),
                contents: bytemuck::cast_slice(&[
                    center[0], center[1], center[2], angle, 0.04, -0.06,
                ]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mut results = Vec::new();
            for mode in 0..2 {
                let texture = |format| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("tree lit fixture target"),
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
                        label: Some("tree lit fixture"),
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
                .chunks_exact(8)
                .zip(results[1].chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count();
            if results[0]
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) & 0x7c00 == 0x7c00)
            {
                return Err(io::Error::other(format!(
                    "nonfinite tree lit fixture {name}/{high}/{size:?}"
                )));
            }
            if name == "near_cutout" {
                ordinary = results[0].clone();
            }
            let input_effect = results[0]
                .chunks_exact(8)
                .zip(ordinary.chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count();
            let quality_input = matches!(name, "snow" | "point_light");
            if quality_input && high && input_effect == 0 {
                return Err(io::Error::other(format!(
                    "tree lit fixture {name}/{high}/{size:?}: input has no effect"
                )));
            }
            if quality_input && !high && input_effect != 0 {
                return Err(io::Error::other(format!(
                    "low quality tree unexpectedly uses {name}/{size:?}"
                )));
            }
            let active = results[0]
                .chunks_exact(8)
                .filter(|p| *p != [0, 60, 0, 0, 0, 60, 0, 60])
                .count();
            let rejected = matches!(name, "mask_above" | "depth_reject" | "backface");
            if (rejected && active != 0) || (!rejected && active == 0) {
                return Err(io::Error::other(format!(
                    "tree lit fixture {name}/{size:?}: unexpected active pixel count {active}"
                )));
            }
            if different != 0 {
                fs::write(
                    output.join(format!("tree-lit-{name}-{high}-{}-native.bin", size[0])),
                    &results[0],
                )?;
                fs::write(
                    output.join(format!("tree-lit-{name}-{high}-{}-port.bin", size[0])),
                    &results[1],
                )?;
            }
            rows.push(json!({"case":name,"high":high,"size":size,"pixels":size[0]*size[1],"different":different,"active":active,"changed_from_ordinary":input_effect}));
        }
    }
    let result = json!({"scope":"production tree lit versus native-input old arithmetic; synthetic tint mips, reflection, mask, shadow, high/low quality and geometry; OWB differences may be authored","cases":rows});
    fs::write(
        output.join("tree-lit.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
