//! Production LUT draws versus the port, with native retained alpha.
use std::{fs, io, path::Path};

use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};

use super::{extent, fullscreen, read};
#[path = "../../src/map/post_lut_shader.rs"]
mod compiled;

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map)?;
    let native = compiled::Blends::new(device, &effect)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("LUT port oracle"),
        source: wgpu::ShaderSource::Wgsl(include_str!("lut_port.wgsl").into()),
    });
    let old = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("LUT port oracle"),
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
            entry_point: Some("blend_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: compiled::FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let texture = |label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(compiled::SIZE),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: compiled::FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let sources: [_; 4] = std::array::from_fn(|index| {
        let texture = texture("asymmetric LUT source");
        let pixels: Vec<u8> = (0..compiled::SIZE[1])
            .flat_map(|y| {
                (0..compiled::SIZE[0]).flat_map(move |x| {
                    [
                        (x * 17 + y * 7 + index as u32 * 31) as u8,
                        (x * 3 + y * 29 + index as u32 * 71) as u8,
                        (x * 13 + y * 11 + index as u32 * 53) as u8,
                        (x + y) as u8,
                    ]
                })
            })
            .collect();
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(compiled::SIZE[0] * 4),
                rows_per_image: Some(compiled::SIZE[1]),
            },
            extent(compiled::SIZE),
        );
        texture.create_view(&Default::default())
    });
    let mut rows = Vec::new();
    for double in [false, true] {
        for (index, factors) in [
            [0.0, 0.0],
            [0.25, 0.5],
            [0.9, 0.1],
            [0.33333334, 0.6666667],
            [1.0, 1.0],
            [-0.5, 1.5],
        ]
        .into_iter()
        .enumerate()
        {
            let factors = if double { factors } else { [0.0, factors[1]] };
            let factor_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&[factors[0], factors[1], 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let entries: Vec<_> = std::iter::once(wgpu::BindGroupEntry {
                binding: 0,
                resource: factor_buffer.as_entire_binding(),
            })
            .chain(
                sources
                    .iter()
                    .enumerate()
                    .map(|(slot, view)| wgpu::BindGroupEntry {
                        binding: slot as u32 + 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    }),
            )
            .collect();
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &old.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut results = Vec::new();
            for port in [true, false] {
                let target = texture("LUT fixture target");
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
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.1,
                                    g: 0.2,
                                    b: 0.3,
                                    a: 165.0 / 255.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if port {
                        pass.set_pipeline(&old);
                        pass.set_bind_group(0, &bind, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
                if !port {
                    native.draw(
                        &arena,
                        &mut encoder,
                        &view,
                        std::array::from_fn(|i| &sources[i]),
                        factors,
                        double,
                    )?;
                }
                let pixels = read(&arena, queue, encoder, &target, compiled::SIZE)?;
                fs::write(
                    output.join(format!(
                        "lut-{}-{index}-{}.bgra8",
                        usize::from(double),
                        if port { "port" } else { "compiled" }
                    )),
                    &pixels,
                )?;
                results.push(pixels);
            }
            let different_rgb = results[0]
                .chunks_exact(4)
                .zip(results[1].chunks_exact(4))
                .filter(|(a, b)| a[..3] != b[..3])
                .count();
            let max_rgb_delta = results[0]
                .chunks_exact(4)
                .zip(results[1].chunks_exact(4))
                .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
                .max()
                .unwrap_or(0);
            let retained_alpha = results[1].chunks_exact(4).filter(|p| p[3] == 165).count();
            rows.push(json!({"double":double,"factors":factors,"pixels":results[1].len()/4,"different_rgb":different_rgb,"max_rgb_delta":max_rgb_delta,"retained_alpha":retained_alpha}));
        }
    }
    Ok(
        json!({"scope":"native b0, two/four physical textures, authored raster/samplers/RGB mask; alpha preinitialized to0xA5, not a native initial-alpha claim", "results":rows}),
    )
}
