//! Water adapter versus the old kernel, with separate native-state and native-vertex controls.
use super::border::compiled as border_shader;
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/water_shader.rs"]
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
        compiled::technique(true, true),
        compiled::technique(true, false),
        compiled::technique(false, true),
    ]
    .map(|technique| compiled::Water::new(device, &effect, map, 1, format, depth_format, technique))
    .into_iter()
    .collect::<io::Result<Vec<_>>>()?;
    let mut kernel = include_str!("water_port.wgsl").replace("\r\n", "\n");
    let start = kernel.find("    // The plane is flat").unwrap();
    let end = kernel[start..].find("    var out: WaterVertex;").unwrap() + start;
    kernel.replace_range(start..end, "    let corners = array<vec2<f32>,4>(vec2<f32>(1.0,0.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,1.0));\n    let x = corners[index].x * globals.map_size.x;\n    let z = corners[index].y * globals.map_size.y;\n");
    kernel = kernel.replace("@location(3) uv_ice: vec2<f32>,", "@location(3) uv_ice: vec2<f32>,\n    @location(4) screen: vec4<f32>,")
        .replace("out.screen_pos = out.position;", "out.screen_pos = out.position;\n    out.screen = screen_coord(out.position);")
        .replace("screen_coord(input.screen_pos)", "input.screen")
        .replace("    let offset_x =", "    if globals.counts.z == 2u { return textureSample(tex,samp,uv).x; }\n    let offset_x =")
        .replace("let sun_dir_water = calculate_sun_direction_water(input.pos);", "var sun_dir_water = calculate_sun_direction_water(input.pos); if globals.counts.z == 2u { sun_dir_water = vec3<f32>(0.0,-1.0,0.0); }")
        .replace("    let fresnel_bias", "    if globals.counts.z != 0u { refractive_color = vec3<f32>(0.0,0.1,0.2); }\n    let fresnel_bias")
        .replace("    let mud_snow =", "    if globals.counts.z != 2u {\n    let mud_snow =")
        .replace("    glossiness += ice.fade * 20.0;", "    glossiness += ice.fade * 20.0;\n    }")
        .replace("    var light = calculate_sun_light_phong(", "    var light = LightOut(vec3<f32>(1.0),vec3<f32>(0.002));\n    if globals.counts.z != 2u { light = calculate_sun_light_phong(")
        .replace("    var out = compose_light", "    }\n    var out = compose_light")
        .replace("    out = apply_fow_projective", "    if globals.counts.z != 2u {\n    out = apply_fow_projective")
        .replace("    out = apply_distance_fog(out, input.pos);", "    out = apply_distance_fog(out, input.pos);\n    }");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("water native-input oracle"),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{}", include_str!("world_port.wgsl"), kernel).into(),
        ),
    });
    let selected = effect.select("water").map_err(io::Error::other)?.unwrap();
    let compiled_vertex = effect.compiled(selected.index).map_err(io::Error::other)?;
    let native_vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("content water vertex oracle control"),
        source: wgpu::ShaderSource::Wgsl(compiled_vertex.vertex.wgsl.clone().into()),
    });
    let native_pixel = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("old water pixel with native vertex outputs"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}",
                include_str!("world_port.wgsl").replace("@group(0)", "@group(2)"),
                kernel
                    .replace("@location(3) uv_ice", "@location(6) uv_ice")
                    .replace("@location(4) screen:", "@location(5) screen:")
            )
            .into(),
        ),
    });
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("native water oracle strip"),
        contents: bytemuck::cast_slice(&[
            [map[0] as i16, 0],
            [0, 0],
            [map[0] as i16, map[1] as i16],
            [0, map[1] as i16],
        ]),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let make_oracle = |native: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water native strip control"),
            layout: None,
            vertex: wgpu::VertexState {
                module: if native { &native_vertex } else { &module },
                entry_point: Some(if native { "main" } else { "vertex_main" }),
                compilation_options: Default::default(),
                buffers: if native {
                    &[Some(wgpu::VertexBufferLayout {
                        array_stride: 4,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Sint16x2],
                    })]
                } else {
                    &[]
                },
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
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
                module: if native { &native_pixel } else { &module },
                entry_point: Some("water_main"),
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
    };
    let old = make_oracle(false);
    let native_inputs = make_oracle(true);
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
                    } else if label == "ice diffuse" {
                        [
                            113 + (i % 4 * 17) as u8,
                            117 + (i / 4 % 4 * 13) as u8,
                            211,
                            93 + (i % 4 * 37) as u8,
                        ]
                    } else if label == "ice noise" {
                        [51 + (i % 4 * 43) as u8, 0, 0, 0]
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
    let diffuse = texture("refraction", [71, 113, 167, 231], false);
    let lean = texture("lean", [128, 128, 192, 192], false);
    let specular = texture("specular", [0, 0, 0, 128], false);
    let ice = texture("ice diffuse", [133, 131, 211, 179], false);
    let ice_noise = texture("ice noise", [217, 0, 0, 0], false);
    let snow = texture("snow mud", [0, 223, 223, 0], false);
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
    values[..8].copy_from_slice(&[
        map[0] as f32 * 0.5,
        25.0,
        map[1] as f32 * 0.5,
        map[0] as f32,
        1.7,
        0.4,
        0.8,
        map[0] as f32,
    ]);
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
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    // Odd backing, clock, all quality variants, ice/shore, active light, depth and winding.
    for (case, (size, time, quality, ice_on, shoreline, blocked, reversed, lights)) in [
        (
            [64, 48],
            0.0_f32,
            0_usize,
            false,
            false,
            false,
            false,
            false,
        ),
        ([63, 47], 27.5, 0, false, true, false, false, false),
        ([64, 48], 27.5, 0, true, false, false, false, false),
        ([63, 47], 0.0, 0, true, false, false, false, false),
        ([64, 48], 1.7, 1, false, false, false, false, false),
        ([64, 48], 1.7, 1, true, true, false, false, false),
        ([64, 48], 1.7, 2, false, false, false, false, false),
        ([64, 48], 27.5, 2, true, true, false, false, false),
        ([64, 48], 0.0, 0, false, false, true, false, false),
        ([64, 48], 0.0, 0, false, false, false, true, false),
        ([64, 48], 0.0, 0, false, false, false, false, true),
        ([127, 95], 1.7, 0, true, false, false, false, false),
        ([127, 95], 1.7, 0, true, false, false, false, false),
    ]
    .into_iter()
    .enumerate()
    {
        let light_indices = if lights {
            &active_indices
        } else {
            &light_indices
        };
        let height = texture("height", [if shoreline { 94 } else { 51 }, 0, 0, 0], false);
        let mut globals = [0_f32; 140];
        globals[0] = 2.0 / map[0] as f32 * if reversed { -1.0 } else { 1.0 };
        globals[9] = 2.0 / map[1] as f32;
        globals[12] = if reversed { 1.0 } else { -1.0 };
        globals[13] = -1.0;
        globals[14] = 0.5;
        globals[15] = 1.0;
        globals[52] = map[0] as f32 * 0.5;
        globals[53] = 120.0;
        globals[54] = map[1] as f32 * 0.5;
        globals[55] = 1.0;
        globals[71] = 1.0;
        globals[83] = 1.0;
        globals[36..39].copy_from_slice(&[0.3, 0.8, 0.5]);
        globals[40..43].copy_from_slice(&[-0.2, 0.9, 0.4]);
        globals[44..47].copy_from_slice(&[0.4, 0.7, -0.3]);
        globals[48..51].copy_from_slice(&[-0.3, 0.7, -0.4]);
        if case >= 11 {
            // Synthetic sun above the plane exercises the ice specular branch.
            // Earlier full-map fixtures placed the water's sun at height zero.
            for first in [36, 40, 44, 48] {
                globals[first] = map[0] as f32 * 0.5;
                globals[first + 2] = map[1] as f32 * 0.5;
                globals[first + 3] = 1200.0;
            }
            if case == 12 {
                // Vary clip.w across the plane to exercise perspective interpolation.
                globals[11] = 0.35 / map[1] as f32;
            }
        }
        globals[60] = 1.0;
        for first in [68, 72, 76, 80, 84, 88] {
            globals[first..first + 3].fill(0.2);
        }
        globals[92..100].copy_from_slice(&[1.0, 0.9, 0.8, 1.0, 0.3, 0.4, 0.6, 0.7]);
        globals[100] = map[1] as f32;
        globals[101] = 1.0;
        globals[136] = map[0] as f32;
        globals[137] = map[1] as f32;
        globals[110] = f32::from_bits(quality as u32);
        globals[112..115].copy_from_slice(&[time, 0.5 / size[0] as f32, 0.5 / size[1] as f32]);
        let global = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&globals),
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
        for (mode, oracle) in [Some(&old), None, Some(&native_inputs)]
            .into_iter()
            .enumerate()
        {
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
                    layout: &oracle.get_bind_group_layout(if mode == 2 { 2 } else { 0 }),
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
                            resource: wgpu::BindingResource::Sampler(&sampler),
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
                    &height,
                    &lean,
                    &lean,
                    &zero,
                    &specular,
                    &diffuse,
                    &ice,
                    &ice_noise,
                    &reflection,
                    if ice_on { &snow } else { &zero },
                    &zero,
                    &zero,
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
                        binding: 12,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 13,
                        resource: wgpu::BindingResource::Sampler(&shadow_sampler),
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
                if mode == 2 {
                    let b0 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&globals[..104]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let b1 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: &[0; 112],
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let b3 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&globals[112..116]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let native_frame = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &oracle.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: b0.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: b1.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: b3.as_entire_binding(),
                            },
                        ],
                    });
                    pass.set_bind_group(0, &native_frame, &[]);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                }
                pass.set_bind_group(if mode == 2 { 2 } else { 0 }, &frame, &[]);
                pass.set_bind_group(1, &material, &[]);
                pass.draw(0..4, 0..1);
            } else {
                native[quality].draw(
                    &arena,
                    &mut encoder,
                    attachments,
                    compiled::Inputs {
                        retained: None,
                        frames: &[bytemuck::cast_slice(&globals[..104])],
                        time,
                        size,
                        retained_pixel_b2: crate::shader_constants::WorldPixelConstants::LightGrid(
                            [0.; 4],
                        ),
                        textures: [
                            &height,
                            &lean,
                            &lean,
                            &zero,
                            &specular,
                            &diffuse,
                            &ice,
                            &ice_noise,
                            &reflection,
                            if ice_on { &snow } else { &zero },
                            light_indices,
                            &light_data,
                            &zero,
                            &zero,
                            &diffuse,
                            &shadow,
                        ],
                    },
                )?;
            }
            let bytes = read(&arena, queue, encoder, &target, size)?;
            fs::write(output.join(format!("water-{case}-{mode}.rgba16f")), &bytes)?;
            results.push(bytes);
        }
        let differences = |mode: usize| {
            results[mode]
                .chunks_exact(8)
                .zip(results[1].chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count()
        };
        let non_clear = results[1]
            .chunks_exact(8)
            .filter(|p| *p != [0, 0x34, 0, 0x38, 0, 0x3a, 0, 0x39])
            .count();
        rows.push(json!({"technique":(["water","water_no_refractions","water_low_gfx"][quality]),"point_light":lights,"ice":ice_on,"sun_above_plane":case>=11,"varying_clip_w":case==12,"shore":shoreline,"size":size,"pixels":size[0]*size[1],"time":time,"blocked":blocked,"back_facing":reversed,
            "native_state_oracle_different_pixels":differences(0),"native_vertex_oracle_different_pixels":differences(2),"non_clear_pixels":non_clear}));
    }
    Ok(
        json!({"scope":"production water adapter versus old pixel kernel; native-state control retains old vertex arithmetic, native-vertex control uses the content vertex stage; synthetic fixtures, three quality variants; OWB differences may be authored", "results":rows}),
    )
}
