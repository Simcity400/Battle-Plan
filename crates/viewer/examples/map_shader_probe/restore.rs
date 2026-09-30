//! Production scene restore against the retained port with identical HDR/LUT inputs.
use std::{fs, io, path::Path};

use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};

use super::{extent, fullscreen, read};
#[path = "../../src/map/post_restore.rs"]
mod compiled_restore;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled_restore::load(layers, map)?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let compiled = compiled_restore::Restore::new(device, &effect, format, 1)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("restore port oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("restore_port.wgsl").into()),
    });
    let old = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("restore port oracle"),
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
            entry_point: Some("grade_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let texture = |size, format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("restore fixture"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let write = |texture: &wgpu::Texture, bytes: &[u8], size: [u32; 2], pixel_bytes| {
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * pixel_bytes),
                rows_per_image: Some(size[1]),
            },
            extent(size),
        );
    };
    // Synthetic identity cube laid out as the native source's SampleColorCube reads it.
    let cube = texture([1024, 32], format);
    let pixels: Vec<u8> = (0..32)
        .flat_map(|g| {
            (0..1024).flat_map(move |x| {
                [
                    (x % 32 * 255 / 31) as u8,
                    (g * 255 / 31) as u8,
                    (x / 32 * 255 / 31) as u8,
                    255,
                ]
            })
        })
        .collect();
    write(&cube, &pixels, [1024, 32], 4);
    let cube_view = cube.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let mut rows = Vec::new();
    for size in [[64_u32, 32_u32], [65, 33]] {
        let scene = texture(size, wgpu::TextureFormat::Rgba16Float);
        let palette = [
            0x0000_u16, 0x2800, 0x3400, 0x3800, 0x3c00, 0x4000, 0x4400, 0x4800,
        ];
        let pixels: Vec<u16> = (0..size[1])
            .flat_map(|y| {
                (0..size[0]).flat_map(move |x| {
                    [
                        palette[((x * 7 + y * 11) % 8) as usize],
                        palette[((x + y) % 8) as usize],
                        palette[((x ^ y) % 8) as usize],
                        0x3c00,
                    ]
                })
            })
            .collect();
        write(&scene, bytemuck::cast_slice(&pixels), size, 8);
        let scene_view = scene.create_view(&Default::default());
        let bloom_size = size.map(|n| n / 2);
        let bloom = texture(bloom_size, wgpu::TextureFormat::Rgba16Float);
        let pixels: Vec<u16> = (0..bloom_size[1])
            .flat_map(|y| {
                (0..bloom_size[0])
                    .flat_map(move |x| [palette[((x + y) % 4) as usize], 0, 0x2800, 0])
            })
            .collect();
        write(&bloom, bytemuck::cast_slice(&pixels), bloom_size, 8);
        let bloom_view = bloom.create_view(&Default::default());
        for (middle_grey, luminance_half) in [(0.5_f32, 0x3c00_u16), (0.2, 0x3800), (1.1, 0x4000)] {
            let luminance = texture([1, 1], wgpu::TextureFormat::R16Float);
            write(&luminance, bytemuck::bytes_of(&luminance_half), [1, 1], 2);
            let luminance_view = luminance.create_view(&Default::default());
            let constants = [
                -0.0_f32,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0 / bloom_size[0] as f32,
                1.0 / bloom_size[1] as f32,
                1.0 / size[0] as f32,
                1.0 / size[1] as f32,
                1.0,
                1.0,
                0.4,
                middle_grey,
                1.0,
            ];
            let native = compiled.bindings(
                &arena,
                &constants,
                [&scene_view, &bloom_view, &cube_view, &luminance_view],
            )?;
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[middle_grey, 0.0, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let uv = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[1.0_f32, 1.0, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let entries = [
                (0, uniform.as_entire_binding()),
                (1, wgpu::BindingResource::TextureView(&scene_view)),
                (3, wgpu::BindingResource::TextureView(&luminance_view)),
                (4, wgpu::BindingResource::TextureView(&cube_view)),
                (5, wgpu::BindingResource::Sampler(&sampler)),
                (6, uv.as_entire_binding()),
                (7, wgpu::BindingResource::TextureView(&bloom_view)),
            ]
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &old.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut results = Vec::new();
            for mode in 0..2 {
                let target = texture(size, format);
                let view = target.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if mode == 0 {
                        compiled.draw(&mut pass, &native);
                    } else {
                        pass.set_pipeline(&old);
                        pass.set_bind_group(0, &bind, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
                results.push(read(&arena, queue, encoder, &target, size)?);
            }
            let different = results[0]
                .chunks_exact(4)
                .zip(results[1].chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let maximum = results[0]
                .iter()
                .zip(&results[1])
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            for (mode, bytes) in results.iter().enumerate() {
                fs::write(
                    output.join(format!("restore-{}-{mode}.rgba8", rows.len())),
                    bytes,
                )?;
            }
            rows.push(json!({"scene_size":size,"middle_grey":middle_grey,"luminance_half":luminance_half,
                "pixels":size[0]*size[1],"different_pixels":different,"maximum_byte_delta":maximum}));
        }
    }
    Ok(
        json!({"scope":"production restore adapter, synthetic HDR/bloom/identity LUT inputs", "results":rows}),
    )
}
