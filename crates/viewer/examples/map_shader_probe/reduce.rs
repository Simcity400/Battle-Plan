//! Actual production reduction adapter versus the retired port, including retained texels.
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};

#[path = "../../src/map/post_gather.rs"]
mod compiled_gather;
#[path = "../../src/map/post_reduce.rs"]
mod compiled_reduce;
#[path = "../../src/map/post_seed.rs"]
mod compiled_seed;
#[path = "../../src/map/post_fullscreen.rs"]
mod fullscreen;

#[path = "arrow.rs"]
mod arrow;
#[path = "border.rs"]
mod border;
#[path = "lut.rs"]
mod lut;
#[path = "mesh_input.rs"]
mod mesh_input;
#[path = "name.rs"]
mod name;
#[path = "particle.rs"]
mod particle;
#[path = "rail.rs"]
mod rail;
#[path = "restore.rs"]
mod restore;
#[path = "sky.rs"]
mod sky;
#[path = "terrain.rs"]
mod terrain;
#[path = "tree_caster.rs"]
mod tree_caster;
#[path = "tree_lit.rs"]
mod tree_lit;
#[path = "tree_receiver.rs"]
mod tree_receiver;
#[path = "water.rs"]
mod water;

#[path = "river.rs"]
mod river;
#[path = "shadow.rs"]
mod shadow;
#[path = "strait.rs"]
mod strait;

const LUMINANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn fail(reason: impl ToString) -> io::Error {
    io::Error::other(reason.to_string())
}

pub(super) fn run(layers: &ResolvedLayers, map: [u32; 2], output: &Path) -> io::Result<Value> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).map_err(fail)?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER | wgpu::Features::FLOAT32_FILTERABLE),
        ..Default::default()
    }))
    .map_err(fail)?;
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let assets = compiled_reduce::load(layers, map)?;
    let compiled = compiled_reduce::Reductions::new(&device, &assets)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("retired luminance reduction oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("reduce_port.wgsl").into()),
    });
    let old = ["reduce_seed_main", "reduce_main"].map(|entry| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: LUMINANCE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for seed in [[512_u32, 288_u32], [511, 287], [17, 11]] {
        let backing = seed.map(|n| n.div_ceil(2));
        let texture = |size: [u32; 2], format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("reduction fixture"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let source = texture(seed, wgpu::TextureFormat::Rgba16Float);
        // Exact half values, distinct red/alpha and asymmetric high-frequency pattern.
        let palette = [
            0x0000_u16, 0x3000, 0x3800, 0x3c00, 0x4000, 0xb800, 0xbc00, 0xc000,
        ];
        let pixels: Vec<u16> = (0..seed[1])
            .flat_map(|y| {
                (0..seed[0]).flat_map(move |x| {
                    [
                        palette[((x * 7 + y * 11) % 8) as usize],
                        0,
                        0,
                        palette[((x * 13 + y * 3) % 8) as usize],
                    ]
                })
            })
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            bytemuck::cast_slice(&pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(seed[0] * 8),
                rows_per_image: Some(seed[1]),
            },
            extent(seed),
        );
        let targets: [[wgpu::Texture; 2]; 2] =
            std::array::from_fn(|_| std::array::from_fn(|_| texture(backing, LUMINANCE_FORMAT)));
        let mut active = seed;
        let mut source_size = seed;
        let mut index = 0;
        loop {
            let viewport = active.map(|n| n.div_ceil(2));
            if viewport[0] <= 3 && viewport[1] <= 3 {
                break;
            }
            let uv = std::array::from_fn(|i| {
                ((viewport[i] * 2) as f32 / active[i] as f32)
                    * (active[i] as f32 / source_size[i] as f32)
            });
            let mut results = Vec::new();
            for (mode, pair) in targets.iter().enumerate() {
                let view = if index == 0 {
                    &source
                } else {
                    &pair[(index - 1) % 2]
                }
                .create_view(&Default::default());
                let target = &pair[index % 2];
                let target_view = target.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                if mode == 0 {
                    compiled.draw(
                        &arena,
                        &mut encoder,
                        index,
                        viewport,
                        uv,
                        &view,
                        &target_view,
                    )?;
                } else {
                    let pipeline = &old[usize::from(index != 0)];
                    let scale = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&[uv[0], uv[1], 0.0, 0.0]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: scale.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&sampler),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.set_viewport(0.0, 0.0, viewport[0] as f32, viewport[1] as f32, 0.0, 1.0);
                    pass.draw(0..3, 0..1);
                }
                let bytes = read(&arena, &queue, encoder, target, backing)?;
                fs::write(
                    output.join(format!(
                        "reduction-{}x{}-{index}-{mode}.r16f",
                        seed[0], seed[1]
                    )),
                    &bytes,
                )?;
                results.push(bytes);
            }
            let different = results[0]
                .chunks_exact(2)
                .zip(results[1].chunks_exact(2))
                .filter(|(a, b)| a != b)
                .count();
            rows.push(json!({"seed":seed,"backing":backing,"viewport":viewport,"uv_scale":uv,"step":index,"samples":results[0].len()/2,"different_half_samples":different}));
            active = viewport;
            source_size = backing;
            index += 1;
        }
    }
    let seed = seed_cases(&device, &queue, layers, map, output)?;
    let gather = gather_cases(&device, &queue, layers, map, output)?;
    let restore = restore::run(&device, &queue, layers, map, output)?;
    let shadow = shadow::run(&device, &queue, layers, map, output)?;
    let lut = lut::run(&device, &queue, layers, map, output)?;
    let strait = strait::run(&device, &queue, layers, map, output)?;
    let border = border::run(&device, &queue, layers, map, output)?;
    let rail = rail::run(&device, &queue, layers, map, output)?;
    let terrain = terrain::run(&device, &queue, layers, map, output)?;
    let water = water::run(&device, &queue, layers, map, output)?;
    let tree_lit = tree_lit::run(&device, &queue, layers, map, output)?;
    let tree_receiver = tree_receiver::run(&device, &queue, layers, map, output)?;
    let tree_caster = tree_caster::run(&device, &queue, layers, map, output)?;
    let sky = sky::run(&device, &queue, layers, map, output)?;
    let mesh_inputs = mesh_input::run(&device, &queue, layers, map, output)?;
    let particles = particle::run(&device, &queue, layers, map, output)?;
    let names = name::run(&device, &queue, layers, map, output)?;
    let arrows = arrow::run(&device, &queue, layers, map, output)?;
    let river = river::run(&device, &queue, layers, map, output)?;
    Ok(
        json!({"scope":"production native post and world adapters; synthetic sources; not a game-frame claim", "results":rows, "seed":seed,"gather":gather,"restore":restore,"shadow":shadow,"lut":lut,"strait":strait,"border":border,"water":water,"terrain":terrain,"sky":sky,"names":names,"arrows":arrows,"mesh_inputs":mesh_inputs,"particles":particles,"tree_caster":tree_caster,"tree_lit":tree_lit,"tree_receiver":tree_receiver,"river":river,"rail":rail}),
    )
}

fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

fn seed_cases(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled_seed::load(layers, map)?;
    let compiled = compiled_seed::Seed::new(device, &effect)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seed port oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("seed_port.wgsl").into()),
    });
    let old = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("seed port oracle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("seed_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: BLOOM_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for size in [[256_u32, 128_u32], [255, 127]] {
        let target_size = size.map(|n| n / 2);
        let texture = |size| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("seed HDR fixture"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BLOOM_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let source = texture(size);
        let source_view = source.create_view(&Default::default());
        let palette = [
            0x0000_u16, 0x2000, 0x3000, 0x3800, 0x3c00, 0x4000, 0x4400, 0x4900,
        ];
        let pixels: Vec<u16> = (0..size[1])
            .flat_map(|y| {
                (0..size[0]).flat_map(move |x| {
                    [
                        palette[((x * 7 + y * 11) % 8) as usize],
                        palette[((x ^ y) % 8) as usize],
                        if x < 17 && y > 89 { 0x4900 } else { 0 },
                        palette[((x + y) % 5) as usize],
                    ]
                })
            })
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            bytemuck::cast_slice(&pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 8),
                rows_per_image: Some(size[1]),
            },
            extent(size),
        );
        for (scenario, [threshold, emissive]) in [[0.4_f32, 1.0], [1.5, 0.0], [0.0, 2.0]]
            .into_iter()
            .enumerate()
        {
            let constants = [
                -0.0_f32,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                emissive,
                1.0 / target_size[0] as f32,
                1.0 / target_size[1] as f32,
                1.0 / size[0] as f32,
                1.0 / size[1] as f32,
                1.0,
                1.0,
                threshold,
                0.5,
                1.0,
            ];
            let mut results = Vec::new();
            for mode in 0..2 {
                let target = texture(target_size);
                let view = target.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                if mode == 0 {
                    compiled.draw(&arena, &mut encoder, &constants, &source_view, &view)?;
                } else {
                    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&[threshold, emissive, 0.0, 0.0]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &old.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&source_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&sampler),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&old);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                let bytes = read(&arena, queue, encoder, &target, target_size)?;
                fs::write(
                    output.join(format!(
                        "seed-{}x{}-{scenario}-{mode}.rgba16f",
                        size[0], size[1]
                    )),
                    &bytes,
                )?;
                results.push(bytes);
            }
            let different = results[0]
                .chunks_exact(2)
                .zip(results[1].chunks_exact(2))
                .filter(|(a, b)| a != b)
                .count();
            rows.push(json!({"scene_size":size,"target_size":target_size,"threshold":threshold,"emissive_strength":emissive,"samples":results[0].len()/2,"different_half_samples":different}));
        }
    }
    Ok(
        json!({"scope":"production seed adapter with synthetic HDR input and post constants", "results":rows}),
    )
}

fn gather_cases(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let assets = compiled_gather::load(layers, map)?;
    let compiled = compiled_gather::Gather::new(device, &assets)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gather port oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("gather_port.wgsl").into()),
    });
    let old = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gather port oracle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("adapt_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: LUMINANCE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let texture = |size| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gather fixture"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LUMINANCE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let mut rows = Vec::new();
    for (backing, gather) in [([256_u32, 144_u32], [4_u32, 3_u32]), ([9, 6], [5, 3])] {
        let source = texture(backing);
        let source_view = source.create_view(&Default::default());
        let palette = [0xbc00_u16, 0xb800, 0x0000, 0x3800, 0x3c00, 0x4000];
        let pixels: Vec<u16> = (0..backing[1])
            .flat_map(|y| (0..backing[0]).map(move |x| palette[((x * 7 + y * 11) % 6) as usize]))
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            bytemuck::cast_slice(&pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(backing[0] * 2),
                rows_per_image: Some(backing[1]),
            },
            extent(backing),
        );
        for hdr in [[0.1_f32, 5.0], [0.8, 0.6], [2.0, 4.0]] {
            for previous_half in [0x3800_u16, 0x4400] {
                let previous = texture([1, 1]);
                let previous_view = previous.create_view(&Default::default());
                queue.write_texture(
                    previous.as_image_copy(),
                    bytemuck::bytes_of(&previous_half),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(2),
                        rows_per_image: Some(1),
                    },
                    extent([1, 1]),
                );
                for tau_delta in [0.0_f32, 0.003, 0.1, 1.0, 10.0, 100.0] {
                    let mut results = Vec::new();
                    for mode in 0..2 {
                        let target = texture([1, 1]);
                        let target_view = target.create_view(&Default::default());
                        let mut encoder = device.create_command_encoder(&Default::default());
                        if mode == 0 {
                            compiled.draw(
                                &arena,
                                &mut encoder,
                                gather,
                                backing,
                                tau_delta,
                                hdr,
                                &source_view,
                                &previous_view,
                                &target_view,
                            )?;
                        } else {
                            let uniform =
                                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: None,
                                    contents: bytemuck::cast_slice(&[
                                        0.5,
                                        hdr[0],
                                        hdr[1],
                                        1.0 - (-tau_delta).exp(),
                                        gather[0] as f32,
                                        gather[1] as f32,
                                        1.0 / backing[0] as f32,
                                        1.0 / backing[1] as f32,
                                    ]),
                                    usage: wgpu::BufferUsages::UNIFORM,
                                });
                            let scale =
                                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: None,
                                    contents: bytemuck::cast_slice(&[1.0_f32, 1.0, 0.0, 0.0]),
                                    usage: wgpu::BufferUsages::UNIFORM,
                                });
                            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: None,
                                layout: &old.get_bind_group_layout(0),
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: uniform.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&source_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::TextureView(
                                            &previous_view,
                                        ),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 5,
                                        resource: wgpu::BindingResource::Sampler(&sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 6,
                                        resource: scale.as_entire_binding(),
                                    },
                                ],
                            });
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: None,
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                            pass.set_pipeline(&old);
                            pass.set_bind_group(0, &bind, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        results.push(read(&arena, queue, encoder, &target, [1, 1])?);
                    }
                    fs::write(
                        output.join(format!("gather-{}.r16f", rows.len())),
                        [results[0].as_slice(), results[1].as_slice()].concat(),
                    )?;
                    rows.push(json!({"backing":backing,"gather":gather,"hdr":hdr,"previous_half":previous_half,
                        "tau_delta":tau_delta,"samples":1,"different_half_samples":usize::from(results[0]!=results[1]),
                        "compiled_half":u16::from_le_bytes([results[0][0],results[0][1]]),"port_half":u16::from_le_bytes([results[1][0],results[1][1]])}));
                }
            }
        }
    }
    Ok(
        json!({"scope":"production gather adapter; raw adaptation/reset, two prior values, odd backing and inverted HDR limits", "results":rows}),
    )
}

/// Submits `encoder`, whose uploads are in `arena`. The arena is not reset: fixtures reuse
/// bindings across submissions.
fn read(
    arena: &shader_pipeline::gpu::Arena,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    target: &wgpu::Texture,
    size: [u32; 2],
) -> io::Result<Vec<u8>> {
    let device = arena.device();
    let pixel_bytes = match target.format() {
        wgpu::TextureFormat::R16Float => 2,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
        wgpu::TextureFormat::Rgba16Float => 8,
        format => return Err(fail(format!("unsupported fixture readback {format:?}"))),
    };
    let stride = (size[0] * pixel_bytes).div_ceil(256) * 256;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * size[1]),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        extent(size),
    );
    arena.flush(queue);
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    read.slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(fail)?;
    receive.recv().map_err(fail)?.map_err(fail)?;
    let mapped = read.slice(..).get_mapped_range().map_err(fail)?;
    let bytes = mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| &row[..(size[0] * pixel_bytes) as usize])
        .copied()
        .collect();
    drop(mapped);
    read.unmap();
    Ok(bytes)
}
