//! Active arrow blending, native strip declaration, depth and stencil modes.
use super::{extent, read, tree_caster::compiled as tree_caster};
use crate::shader_scene;
use crate::{shader_constants, shader_sources};
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::binding;
use std::{collections::BTreeMap, fs, io, path::Path};
#[allow(dead_code)]
#[path = "../../src/map/arrow_shader.rs"]
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
    let effect = compiled::load(layers, map, crate::DECODE, "map arrows")?;
    let mut frame = [0_f32; 104];
    for base in [0, 16] {
        for i in [0, 5, 10, 15] {
            frame[base + i] = 1.;
        }
    }
    for i in [0, 5, 10] {
        frame[i] = 4.;
        frame[16 + i] = 0.25;
    }
    for base in [36, 40, 44, 48] {
        frame[base..base + 3].copy_from_slice(&[10., 20., 30.]);
    }
    frame[55] = 1.;
    frame[66] = 1.;
    frame[83] = 1.;
    // Black source, nonzero alpha: analytic RGB blending independent of lighting.
    // World positions stay inside FOG_BEGIN=1 on both source snapshots; a camera
    // at [0,5,10] would add distance-fog RGB to this supposedly black source.
    // Valid normals/directions and the light-index sentinel avoid invalid lights.
    let mut textures = BTreeMap::new();
    for slot in 0..11 {
        let rgba: [u16; 4] = match slot {
            0 => [0x3c00; 4],
            1 => [0, 0, 0, 0x3800],
            2 | 4 | 10 => [0x3c00; 4],
            5 | 6 => [0x3800, 0x3800, 0x3c00, 0x3c00],
            7 | 8 => [0x3800, 0x3800, 0x3c00, 0],
            _ => [0; 4],
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("arrow analytic input"),
            size: extent([1, 1]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::bytes_of(&rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: None,
            },
            extent([1, 1]),
        );
        textures.insert(
            binding(slot, Stage::Pixel),
            texture.create_view(&Default::default()),
        );
    }
    let vertices = [[-1., -1.], [-1., 1.], [1., -1.], [1., 1.]].map(|[x, y]| compiled::Vertex {
        position: [x * 0.25, y * 0.25, 0.125],
        padding: 0,
        uv: [0.25, 0.5, 0., 0.],
    });
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("native arrow strip32"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let size = [32, 16];
    let mut rows = Vec::new();
    for samples in [1, 4] {
        let texture = |format, count| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("arrow state fixture"),
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
        let view = target.create_view(&Default::default());
        let msaa = (samples > 1).then(|| texture(FORMAT, samples).create_view(&Default::default()));
        let depth = texture(DEPTH, samples).create_view(&Default::default());
        for name in [
            "MapArrowDefault",
            "MapArrowDefaultWithDepth",
            "MapArrowDefaultWithDepthTestOnly",
            "MapArrowNoHead",
            "MapArrowNoHeadWidthDepth",
            "MapArrowNoHeadSeethrough",
            "MapArrowNoHeadWidthDepthSeethrough",
        ] {
            let seethrough = name.ends_with("Seethrough");
            let stencil_test = seethrough
                || matches!(
                    name,
                    "MapArrowDefault" | "MapArrowDefaultWithDepthTestOnly" | "MapArrowNoHead"
                );
            let depth_test = seethrough || !matches!(name, "MapArrowDefault" | "MapArrowNoHead");
            for mode in [
                compiled::Mode::Main,
                compiled::Mode::Seethrough,
                compiled::Mode::Late,
                compiled::Mode::Shadow,
            ] {
                let draw_last = matches!(mode, compiled::Mode::Late | compiled::Mode::Shadow);
                if !mode.admits(draw_last) {
                    continue;
                }
                let pipeline =
                    compiled::Arrow::new(device, &effect, name, samples, FORMAT, DEPTH, mode)?;
                let mut arrow = [0_f32; 32];
                arrow[0] = 1.;
                arrow[7] = 1.;
                arrow[11] = 1.;
                arrow[20..23].fill(1.);
                arrow[24] = mode.lighting();
                arrow[25] = mode.lighting();
                arrow[26] = 0.5;
                arrow[27] = 1.;
                arrow[28] = 1.;
                let scene = if mode == compiled::Mode::Shadow {
                    let matrix: [[f32; 4]; 4] =
                        bytemuck::pod_read_unaligned(bytemuck::cast_slice(&frame[..16]));
                    compiled::shadow_constants(bytemuck::cast_slice(&frame), matrix)
                } else {
                    bytemuck::cast_slice(&frame).to_vec()
                };
                let bindings = pipeline.bindings(
                    &arena,
                    &scene,
                    bytemuck::bytes_of(&[0_f32; 4]),
                    bytemuck::cast_slice(&arrow),
                    &textures,
                )?;
                for stencil in [0, 1, 2, 3, 4, 8, 10, 12] {
                    for depth_clear in [0., 1.] {
                        let mut encoder = device.create_command_encoder(&Default::default());
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("two overlapping native arrow strips"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: msaa.as_ref().unwrap_or(&view),
                                    resolve_target: msaa.as_ref().map(|_| &view),
                                    depth_slice: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: 0.25,
                                            g: 0.5,
                                            b: 0.75,
                                            a: 0.625,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: Some(
                                    wgpu::RenderPassDepthStencilAttachment {
                                        view: &depth,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(depth_clear),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                        stencil_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(stencil),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                    },
                                ),
                                ..Default::default()
                            });
                            pipeline.begin(&mut pass);
                            bindings.bind(&mut pass);
                            pass.set_vertex_buffer(0, vertices.slice(..));
                            pass.draw(0..4, 0..1);
                            pass.draw(0..4, 0..1);
                        }
                        let bytes = read(&arena, queue, encoder, &target, size)?;
                        let mask = if mode == compiled::Mode::Late {
                            2
                        } else if seethrough {
                            8
                        } else {
                            1
                        };
                        let stencil_pass = !stencil_test
                            || if seethrough {
                                stencil & mask == mask
                            } else {
                                stencil & mask != mask
                            };
                        let depth_pass = !depth_test
                            || if seethrough {
                                depth_clear < 0.5
                            } else {
                                depth_clear >= 0.5
                            };
                        let count = if !stencil_pass || !depth_pass {
                            0
                        } else if stencil_test && !seethrough {
                            1
                        } else {
                            2
                        };
                        let factor = if seethrough {
                            0.75_f32.powi(count)
                        } else {
                            0.5_f32.powi(count)
                        };
                        let expected =
                            [0.25 * factor, 0.5 * factor, 0.75 * factor, 0.625].map(half);
                        let different = bytes
                            .chunks_exact(8)
                            .filter(|pixel| *pixel != bytemuck::bytes_of(&expected))
                            .count();
                        let first_difference = bytes.chunks_exact(8).enumerate().find_map(|(i, pixel)| {
                            (pixel != bytemuck::bytes_of(&expected)).then(|| {
                                let actual: [u16; 4] = bytemuck::pod_read_unaligned(pixel);
                                json!({"xy":[i % size[0] as usize, i / size[0] as usize], "actual_bits":actual, "expected_bits":expected})
                            })
                        });
                        rows.push(json!({"technique":name,"mode":format!("{mode:?}"),"samples":samples,"stencil":stencil,"depth_clear":depth_clear,"expected_draws":count,"pixels":size[0]*size[1],"different_rgba16f_pixels":different,"first_difference":first_difference}));
                    }
                }
            }
        }
    }
    let report = json!({"scope":"production arrow adapter; analytic black-source blending after two overlapping native strips, depth/stencil state and late clones, 1/4 samples; not game geometry, lighting or screenshot proof","results":rows});
    fs::write(
        output.join("arrows.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if rows.iter().any(|r| r["different_rgba16f_pixels"] != 0) {
        return Err(io::Error::other(
            "arrow native-state fixture differs; see arrows.json",
        ));
    }
    Ok(report)
}

// All expected values above are exactly representable normal binary fractions.
fn half(v: f32) -> u16 {
    if v == 0. {
        0
    } else {
        let bits = v.to_bits();
        (((bits >> 23) - (127 - 15)) << 10 | ((bits & 0x7fffff) >> 13)) as u16
    }
}

#[test]
fn analytic_fractions_encode_as_binary16() {
    assert_eq!(
        [0., 0.0625, 0.25, 0.5, 0.625, 1.].map(half),
        [0, 0x2c00, 0x3400, 0x3800, 0x3900, 0x3c00]
    );
}
