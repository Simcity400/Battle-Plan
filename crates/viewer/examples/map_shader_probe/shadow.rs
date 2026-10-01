//! Shadow blur intermediate orientation, native inputs and depth/stencil retention.
use std::{fs, io, path::Path};

use eframe::wgpu::{self, util::DeviceExt};
use installed_content::ResolvedLayers;
use serde_json::{Value, json};

use super::{extent, read};
use crate::shader_sources;
#[path = "../../src/map/shadow_blur.rs"]
mod compiled;

const FORMAT: wgpu::TextureFormat = compiled::COLOR_FORMAT;

fn pipeline(
    device: &wgpu::Device,
    source: &str,
    entry: &str,
    quad: bool,
    depth: bool,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow blur fixture oracle"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: 32,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: if quad { &buffers } else { &[] },
        },
        primitive: wgpu::PrimitiveState {
            topology: if quad {
                wgpu::PrimitiveTopology::TriangleStrip
            } else {
                wgpu::PrimitiveTopology::TriangleList
            },
            ..Default::default()
        },
        depth_stencil: depth.then_some(wgpu::DepthStencilState {
            format: compiled::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Equal),
            stencil: wgpu::StencilState {
                front: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::Equal,
                    ..Default::default()
                },
                back: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::Equal,
                    ..Default::default()
                },
                read_mask: 0xff,
                write_mask: 0,
            },
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map, crate::DECODE)?;
    let native = compiled::Blur::new(device, &effect)?;
    let port = include_str!("shadow_port.wgsl");
    let begin = port.find("@vertex").expect("oracle vertex");
    let end = port.find("/// The body").expect("oracle fragment");
    let quad_port = format!(
        "{}{}{}",
        &port[..begin],
        r#"
@vertex fn vertex_main(@location(0) pos:vec4<f32>, @location(1) uv:vec2<f32>) -> FullScreen {
    var out:FullScreen; out.position=pos; out.uv=uv; return out;
}
"#,
        &port[end..]
    );
    let ports = [port, quad_port.as_str()].map(|source| {
        ["horizontal_main", "vertical_main"]
            .map(|entry| pipeline(device, source, entry, source == quad_port, false))
    });
    let depth_source = format!(
        "{}{}",
        &port[..end],
        "@fragment fn depth_main() -> @location(0) vec4<f32> {return vec4<f32>(1.0);}"
    );
    let depth_probe = pipeline(device, &depth_source, "depth_main", false, true);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let quad: [[f32; 8]; 4] = [
        [-1., -1., 0., 1., 0., 0., 0., 0.],
        [-1., 1., 0., 1., 0., 1., 0., 0.],
        [1., -1., 0., 1., 1., 0., 0., 0.],
        [1., 1., 0., 1., 1., 1., 0., 0.],
    ];
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&quad),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mut rows = Vec::new();
    for size in [[128_u32, 64_u32], [127, 63], [2048, 1152]] {
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow blur fixture"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let source = texture(FORMAT);
        let pixels: Vec<u8> = (0..size[1])
            .flat_map(|y| {
                (0..size[0]).flat_map(move |x| {
                    [
                        ((x * 7 + y * 11) % 256) as u8,
                        ((x * 3 + y * 17) % 256) as u8,
                        ((x ^ y) % 256) as u8,
                        255,
                    ]
                })
            })
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            extent(size),
        );
        let heights: &[f32] = if size[0] == 2048 {
            &[125.630_7, 53.883_33]
        } else {
            &[0.0, 131.682, 699.0]
        };
        for &height in heights {
            let mut results = Vec::new();
            let mut depth_pixels = 0;
            for mode in 0..4 {
                let targets = [texture(FORMAT), texture(FORMAT)];
                let depth = texture(compiled::DEPTH_FORMAT);
                let depth_view = depth.create_view(&Default::default());
                let mut steps = Vec::new();
                for axis in 0..2 {
                    let input = if axis == 0 { &source } else { &targets[0] };
                    let input = input.create_view(&Default::default());
                    let target = targets[axis].create_view(&Default::default());
                    let mut constants = compiled::constants(size[axis], height);
                    if mode == 1 || mode == 3 {
                        // Preserve original port CPU arithmetic and literals.
                        let scale = 1.0 - (height / 700.0).min(1.0);
                        constants = [
                            0.0,
                            1.384_615_4 * scale / size[axis] as f32,
                            3.230_769_2 * scale / size[axis] as f32,
                            0.227_027_03,
                            0.316_216_2,
                            0.070_270_27,
                            0.0,
                            0.0,
                        ];
                    }
                    let binding = native.bindings(&arena, axis, &constants, &input)?;
                    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(&constants),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let oracle = &ports[if mode >= 2 { 1 } else { 0 }][axis];
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &oracle.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&input),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&sampler),
                            },
                        ],
                    });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    if mode == 0 && axis == 1 {
                        // Nonzero initial depth/stencil distinguishes native write/retention.
                        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(0.375),
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(0xa5),
                                        store: wgpu::StoreOp::Store,
                                    }),
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        }));
                    }
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &target,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: (mode == 0 && axis == 1).then_some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        if mode == 0 {
                            native.render(&mut pass, axis, &binding);
                        } else {
                            pass.set_pipeline(oracle);
                            pass.set_bind_group(0, &group, &[]);
                            if mode >= 2 {
                                pass.set_vertex_buffer(0, vertices.slice(..));
                            }
                            pass.draw(0..if mode >= 2 { 4 } else { 3 }, 0..1);
                        }
                    }
                    steps.push(read(&arena, queue, encoder, &targets[axis], size)?);
                }
                if mode == 0 {
                    let probe = texture(FORMAT);
                    let view = probe.create_view(&Default::default());
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
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        pass.set_pipeline(&depth_probe);
                        pass.set_stencil_reference(0xa5);
                        pass.draw(0..3, 0..1);
                    }
                    depth_pixels = read(&arena, queue, encoder, &probe, size)?
                        .chunks_exact(4)
                        .filter(|p| *p == [255, 255, 255, 255])
                        .count();
                }
                for (axis, bytes) in steps.iter().enumerate() {
                    fs::write(
                        output.join(format!("shadow-{}-{mode}-{axis}.bgra8", rows.len())),
                        bytes,
                    )?;
                }
                results.push(steps);
            }
            let diff = |a: &[u8], b: &[u8]| json!({"different_pixels":a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a!=b).count(),"max_byte_delta":a.iter().zip(b).map(|(a,b)|a.abs_diff(*b)).max().unwrap_or(0)});
            let flipped: Vec<u8> = results[1][0]
                .chunks_exact(size[0] as usize * 4)
                .rev()
                .flatten()
                .copied()
                .collect();
            rows.push(json!({"size":size,"height":height,"pixels":size[0]*size[1],"depth_zero_stencil_preserved_pixels":depth_pixels,
                "native_vs_port_horizontal_flipped":diff(&results[0][0],&flipped),"native_vs_port_final":diff(&results[0][1],&results[1][1]),
                "native_vs_quad_control_horizontal":diff(&results[0][0],&results[2][0]),"native_vs_quad_control_final":diff(&results[0][1],&results[2][1]),
                "native_vs_quad_old_constants_final":diff(&results[0][1],&results[3][1]),"quad_old_vs_quad_native_constants_final":diff(&results[2][1],&results[3][1])}));
        }
    }
    Ok(
        json!({"scope":"production adapter with synthetic asymmetric sources; original port and native-quad controls; depth zero/stencil retention probe", "results":rows}),
    )
}
