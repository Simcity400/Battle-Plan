//! Active sky pixels against analytic cube faces, with depth and stencil probes.
use super::border::compiled as border_shader;
use super::{extent, read};
use crate::shader_constants;
use crate::shader_sources;
use eframe::wgpu;
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/sky_shader.rs"]
mod compiled;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.25,
    g: 0.5,
    b: 0.75,
    a: 0.625,
};

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map, crate::DECODE)?;
    // Six distinct exact sRGB endpoints: +X,-X,+Y,-Y,+Z,-Z.
    let colors = [
        [1_u8, 0, 0],
        [0, 1, 0],
        [0, 0, 1],
        [0, 1, 1],
        [1, 0, 1],
        [1, 1, 0],
    ];
    let cube = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("analytic sky cube"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<u8> = colors
        .into_iter()
        .flat_map(|[r, g, b]| (0..16).flat_map(move |_| [r * 255, g * 255, b * 255, 255]))
        .collect();
    queue.write_texture(
        cube.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        cube.size(),
    );
    let reflection = cube.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky depth/stencil probe"),
        source: wgpu::ShaderSource::Wgsl(r#"
fn position(i:u32,z:f32)->vec4<f32> {
    let v=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));
    return vec4<f32>(v[i],z,1.);
}
@vertex fn fill(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> { return position(i,.5); }
@vertex fn probe(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> { return position(i,1.); }
@fragment fn pixel()->@location(0) vec4<f32> { return vec4<f32>(1.); }
"#.into()),
    });
    let mut rows = Vec::new();
    for samples in [1, 4] {
        let sky = compiled::Sky::new(device, &effect, samples, FORMAT, DEPTH)?;
        let pipeline = |probe| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky attachment check"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(if probe { "probe" } else { "fill" }),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(!probe),
                    depth_compare: Some(if probe {
                        wgpu::CompareFunction::Equal
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: if probe {
                        wgpu::StencilState {
                            front: wgpu::StencilFaceState {
                                compare: wgpu::CompareFunction::Equal,
                                ..Default::default()
                            },
                            back: wgpu::StencilFaceState {
                                compare: wgpu::CompareFunction::Equal,
                                ..Default::default()
                            },
                            read_mask: 255,
                            write_mask: 0,
                        }
                    } else {
                        Default::default()
                    },
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("pixel"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: if probe {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::empty()
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let fill = pipeline(false);
        let probe = pipeline(true);
        for size in [[64_u32, 32_u32], [63, 35]] {
            let texture = |format, count| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("sky fixture target"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | if count == 1 {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                })
            };
            let target = texture(FORMAT, 1);
            let target_view = target.create_view(&Default::default());
            let multisampled = (samples != 1).then(|| texture(FORMAT, samples));
            let msaa_view = multisampled
                .as_ref()
                .map(|t| t.create_view(&Default::default()));
            let depth = texture(DEPTH, samples);
            let depth_view = depth.create_view(&Default::default());
            let color = |load| wgpu::RenderPassColorAttachment {
                view: msaa_view.as_ref().unwrap_or(&target_view),
                depth_slice: None,
                resolve_target: msaa_view.as_ref().map(|_| &target_view),
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            };
            let depth_attachment = |clear| wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(1.)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(37)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
            };
            for (face, rgb) in colors.iter().enumerate() {
                // At every vertex the intended cube axis dominates. Translation
                // is larger than that axis, so omitting vCamPos changes the result.
                let mut frame = [0_f32; 104];
                frame[31] = 1.;
                let axis = face / 2;
                frame[16 + (axis + 1) % 3] = 0.375;
                frame[20 + (axis + 2) % 3] = 0.25;
                frame[24 + axis] = if face % 2 == 0 { 4. } else { -4. };
                for (i, v) in [17., -23., 31.].into_iter().enumerate() {
                    frame[28 + i] = v;
                    frame[52 + i] = v;
                }
                frame[83] = 1.; // NegFogMultiplier, InitializeSceneCB0@140223170.
                for blocked in [0, size[0] / 2, size[0]] {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("sky fixture initialize"),
                            color_attachments: &[Some(color(wgpu::LoadOp::Clear(CLEAR)))],
                            depth_stencil_attachment: Some(depth_attachment(true)),
                            ..Default::default()
                        });
                        if blocked > 0 {
                            pass.set_pipeline(&fill);
                            pass.set_scissor_rect(0, 0, blocked, size[1]);
                            pass.draw(0..3, 0..1);
                        }
                    }
                    sky.draw(
                        &arena,
                        &mut encoder,
                        border_shader::Target {
                            color: color(wgpu::LoadOp::Load),
                            depth: depth_attachment(false),
                        },
                        bytemuck::cast_slice(&frame),
                        &reflection,
                    )?;
                    let bytes = read(&arena, queue, encoder, &target, size)?;
                    let mut expected: Vec<u8> = Vec::new();
                    for _ in 0..size[1] {
                        for x in 0..size[0] {
                            let pixel = if x < blocked {
                                [0x3400_u16, 0x3800, 0x3a00, 0x3900]
                            } else {
                                [
                                    u16::from(rgb[0]) * 0x3c00,
                                    u16::from(rgb[1]) * 0x3c00,
                                    u16::from(rgb[2]) * 0x3c00,
                                    0x3c00,
                                ]
                            };
                            expected.extend_from_slice(bytemuck::cast_slice(&pixel));
                        }
                    }
                    let different = bytes
                        .chunks_exact(8)
                        .zip(expected.chunks_exact(8))
                        .filter(|(a, b)| a != b)
                        .count();
                    fs::write(
                        output.join(format!(
                            "sky-{samples}-{}-{face}-{blocked}.rgba16f",
                            size[0]
                        )),
                        bytes,
                    )?;
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("sky depth/stencil retained"),
                            color_attachments: &[Some(color(wgpu::LoadOp::Clear(
                                wgpu::Color::BLACK,
                            )))],
                            depth_stencil_attachment: Some(depth_attachment(false)),
                            ..Default::default()
                        });
                        pass.set_pipeline(&probe);
                        pass.set_stencil_reference(37);
                        pass.draw(0..3, 0..1);
                    }
                    let retained = read(&arena, queue, encoder, &target, size)?;
                    let bad_retention = retained
                        .chunks_exact(8)
                        .enumerate()
                        .filter(|(i, p)| {
                            let expected = if *i % size[0] as usize >= blocked as usize {
                                [0, 0x3c, 0, 0x3c, 0, 0x3c, 0, 0x3c]
                            } else {
                                [0, 0, 0, 0, 0, 0, 0, 0x3c]
                            };
                            **p != expected
                        })
                        .count();
                    rows.push(json!({"size":size,"samples":samples,"cube_face":face,"blocked_columns":blocked,"pixels":size[0]*size[1],"different_rgba16f_pixels":different,"failed_depth_stencil_probes":bad_retention}));
                }
            }
        }
    }
    Ok(
        json!({"scope":"production sky adapter; analytic constant sRGB cube faces, translated inverse projections, native NegFogMultiplier1, depth rejection and stencil retention at1/4samples; not a matched game screenshot or a retired-port comparison", "results":rows}),
    )
}
