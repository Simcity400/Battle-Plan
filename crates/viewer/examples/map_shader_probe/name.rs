//! Production country-name ABI, blend, culling, disabled depth and masked stencil.
use super::{extent, mesh_input::native as mesh, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::{TextureKind, binding, gpu};
use std::{collections::BTreeMap, fs, io, path::Path};
#[allow(dead_code)]
#[path = "../../src/map/name_shader.rs"]
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
    let effect = compiled::load(layers, map, crate::DECODE)?;
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("name analytic atlas"),
        size: extent([1, 1]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        atlas.as_image_copy(),
        bytemuck::cast_slice(&[0x3c00_u16, 0x3800, 0, 0x3800]),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: None,
        },
        extent([1, 1]),
    );
    let atlas = atlas.create_view(&Default::default());
    let mut frame = [0_f32; 104];
    for i in [0, 5, 10, 15] {
        frame[i] = 1.;
    }
    frame[58] = 1.; // Native look direction changes Z only; depth is disabled.
    let mut rows = Vec::new();
    for samples in [1, 4] {
        let names = compiled::Names::new(device, &effect, samples, FORMAT, DEPTH)?;
        let writers = mesh_writers(&arena, queue, layers, map, samples, &frame)?;
        for size in [[64, 32], [62, 30]] {
            let texture = |format, count| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("name fixture target"),
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
            let msaa = (samples > 1).then(|| texture(FORMAT, samples));
            let msaa = msaa.as_ref().map(|t| t.create_view(&Default::default()));
            let depth = texture(DEPTH, samples);
            let depth = depth.create_view(&Default::default());
            for stencil in [0, 1, 4, 5, 8, 12] {
                for alpha in [0., 0.5, 1.] {
                    for offset in [0., 1.] {
                        for reversed in [false, true] {
                            let point = |x, y| compiled::Vertex {
                                position: [x, y, 0.75],
                                uv: [0.5, 0.5],
                            };
                            let mut vertices = [
                                point(-1., -1.),
                                point(1., -1.),
                                point(-1., 1.),
                                point(1., -1.),
                                point(1., 1.),
                                point(-1., 1.),
                            ];
                            if reversed {
                                for triangle in vertices.chunks_exact_mut(3) {
                                    triangle.swap(0, 1);
                                }
                            }
                            let buffer =
                                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("name native declaration"),
                                    contents: bytemuck::cast_slice(&vertices),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                            let bindings = names.bindings(
                                &arena,
                                bytemuck::cast_slice(&frame),
                                [alpha, offset],
                                &atlas,
                            )?;
                            let mut encoder = device.create_command_encoder(&Default::default());
                            if stencil & 12 != 0 {
                                let mut pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("compiled mesh before country names"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
                                                view: msaa.as_ref().unwrap_or(&view),
                                                depth_slice: None,
                                                resolve_target: msaa.as_ref().map(|_| &view),
                                                ops: wgpu::Operations {
                                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                                    store: wgpu::StoreOp::Store,
                                                },
                                            },
                                        )],
                                        depth_stencil_attachment: Some(
                                            wgpu::RenderPassDepthStencilAttachment {
                                                view: &depth,
                                                depth_ops: Some(wgpu::Operations {
                                                    load: wgpu::LoadOp::Clear(1.),
                                                    store: wgpu::StoreOp::Store,
                                                }),
                                                stencil_ops: Some(wgpu::Operations {
                                                    load: wgpu::LoadOp::Clear(stencil & !12),
                                                    store: wgpu::StoreOp::Store,
                                                }),
                                            },
                                        ),
                                        ..Default::default()
                                    });
                                for (bit, writer) in [4, 8].into_iter().zip(&writers) {
                                    if stencil & bit != 0 {
                                        writer.pipeline.begin(&mut pass);
                                        writer.bindings.bind(&mut pass);
                                        pass.set_vertex_buffer(0, writer.vertices.slice(..));
                                        pass.draw(0..6, 0..1);
                                    }
                                }
                            }
                            {
                                let mut pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("native name state fixture"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
                                                view: msaa.as_ref().unwrap_or(&view),
                                                depth_slice: None,
                                                resolve_target: msaa.as_ref().map(|_| &view),
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
                                        )],
                                        depth_stencil_attachment: Some(
                                            wgpu::RenderPassDepthStencilAttachment {
                                                view: &depth,
                                                depth_ops: Some(wgpu::Operations {
                                                    load: if stencil & 12 != 0 {
                                                        wgpu::LoadOp::Load
                                                    } else {
                                                        wgpu::LoadOp::Clear(0.)
                                                    },
                                                    store: wgpu::StoreOp::Store,
                                                }),
                                                stencil_ops: Some(wgpu::Operations {
                                                    load: if stencil & 12 != 0 {
                                                        wgpu::LoadOp::Load
                                                    } else {
                                                        wgpu::LoadOp::Clear(stencil)
                                                    },
                                                    store: wgpu::StoreOp::Store,
                                                }),
                                            },
                                        ),
                                        ..Default::default()
                                    });
                                names.begin(&mut pass);
                                bindings.bind(&mut pass);
                                pass.set_vertex_buffer(0, buffer.slice(..));
                                pass.draw(0..6, 0..1);
                            }
                            let bytes = read(&arena, queue, encoder, &target, size)?;
                            // All values are binary fractions, including native RGB-only alpha blend.
                            let blended = if alpha == 1. {
                                [0x3900_u16, 0x3800, 0x3600, 0x3900]
                            } else if alpha == 0.5 {
                                [0x3700, 0x3800, 0x3880, 0x3900]
                            } else {
                                [0x3400, 0x3800, 0x3a00, 0x3900]
                            };
                            let clear = [0x3400_u16, 0x3800, 0x3a00, 0x3900];
                            let mut expected_stencil = stencil;
                            if stencil & 12 != 0 {
                                expected_stencil &= !12;
                                for (bit, writer) in [4, 8].into_iter().zip(&writers) {
                                    if stencil & bit != 0 {
                                        let (mask, reference) = writer.stencil_write;
                                        expected_stencil =
                                            (expected_stencil & !mask) | (reference & mask);
                                    }
                                }
                            }
                            let different = bytes
                                .chunks_exact(8)
                                .enumerate()
                                .filter(|(i, pixel)| {
                                    let active = !reversed
                                        && expected_stencil & 4 == 0
                                        && (offset == 0.
                                            || i % size[0] as usize >= size[0] as usize / 2);
                                    *pixel
                                        != bytemuck::cast_slice::<u16, u8>(if active {
                                            &blended
                                        } else {
                                            &clear
                                        })
                                })
                                .count();
                            rows.push(json!({"size":size,"samples":samples,"stencil":stencil,"mesh_writers":stencil & 12,"expected_stencil":expected_stencil,"alpha":alpha,"offset":offset,"reversed":reversed,"pixels":size[0]*size[1],"different_rgba16f_pixels":different}));
                        }
                    }
                }
            }
        }
    }
    let report = json!({"scope":"production name adapter with native 20-byte vertices and stage buffers; analytic active RGB blend, alpha preservation, X offset, culling, disabled depth and stencil bit4 rejection after actual compiled PdxMeshStandard/PdxMeshAdvancedSnow writers; mesh colour cleared independently, synthetic material inputs, not game screenshot or geometry proof","results":rows});
    fs::write(
        output.join("names.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if rows.iter().any(|r| r["different_rgba16f_pixels"] != 0) {
        return Err(io::Error::other(
            "name native-state fixture differs; see names.json",
        ));
    }
    Ok(report)
}

struct MeshWriter {
    stencil_write: (u32, u32),
    pipeline: mesh::Pass,
    bindings: gpu::Bindings,
    vertices: wgpu::Buffer,
}

fn mesh_writers(
    arena: &shader_pipeline::gpu::Arena,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    samples: u32,
    frame: &[f32; 104],
) -> io::Result<Vec<MeshWriter>> {
    let device = arena.device();
    let (mut library, defines) = shader_sources::library(layers, map, crate::DECODE);
    let effect = library
        .load(b"gfx/FX/pdxmesh.lua", &defines)
        .map_err(io::Error::other)?;
    let mut sub = installed_content::mesh::SubMesh::default();
    for [x, y] in [
        [-1., -1.],
        [-1., 1.],
        [1., -1.],
        [1., -1.],
        [-1., 1.],
        [1., 1.],
    ] {
        sub.positions.push([x, y, 0.5]);
        sub.normals.push([0., 0., 1.]);
        sub.tangents.push([1., 0., 0., 1.]);
        sub.uv0.push([0.5, 0.5]);
    }
    let geometry = mesh::Geometry::from_mesh(&sub, 1.)?;
    let packed = geometry.packed(geometry.uv1)?;
    let mut out = Vec::new();
    for name in ["PdxMeshStandard", "PdxMeshAdvancedSnow"] {
        let selection = effect
            .select(name)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other(format!("mesh/name fixture missing {name}")))?;
        let compiled = effect.compiled(selection.index).map_err(io::Error::other)?;
        // Vanilla writes4/8; OWB omits both authored stencil states. Check
        // source semantics independently of wgpu lowering, not a vanilla mask.
        let state = effect
            .source
            .depth_stencil(&effect.source.techniques[selection.index]);
        let stencil_write = if !state.stencil_enable {
            (0, 0)
        } else if state.front_function == 7 && state.front_pass == 2 {
            (state.stencil_write_mask, state.stencil_ref)
        } else {
            return Err(io::Error::other(format!(
                "mesh/name fixture requires disabled or ALWAYS/REPLACE stencil: {name}"
            )));
        };
        let mut buffers: BTreeMap<_, _> = compiled
            .vertex
            .buffers
            .iter()
            .chain(&compiled.pixel.buffers)
            .map(|b| (b.binding, vec![0; b.size as usize]))
            .collect();
        for stage in [Stage::Vertex, Stage::Pixel] {
            buffers.insert(binding(0, stage), bytemuck::cast_slice(frame).to_vec());
        }
        let mut object = [0_f32; 52];
        for i in [0, 5, 10, 15] {
            object[i] = 1.;
        }
        buffers.insert(
            binding(1, Stage::Vertex),
            bytemuck::cast_slice(&object).to_vec(),
        );
        let textures = compiled
            .vertex
            .textures
            .iter()
            .chain(&compiled.pixel.textures)
            .map(|&(slot, kind)| {
                let depth = if kind == TextureKind::Cube { 6 } else { 1 };
                let size = wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: depth,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("mesh/name synthetic material"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: if kind == TextureKind::D3 {
                        wgpu::TextureDimension::D3
                    } else {
                        wgpu::TextureDimension::D2
                    },
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    texture.as_image_copy(),
                    &vec![255; 4 * depth as usize],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    size,
                );
                (
                    slot,
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(kind.view()),
                        ..Default::default()
                    }),
                )
            })
            .collect();
        let pipeline = mesh::Pass::new(
            &mut Default::default(),
            device,
            &selection,
            &geometry,
            FORMAT,
            DEPTH,
            samples,
        )?;
        let bindings = pipeline
            .pipeline
            .bindings(arena, &buffers, &textures)
            .map_err(io::Error::other)?;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh/name native declaration"),
            contents: bytemuck::cast_slice(&packed),
            usage: wgpu::BufferUsages::VERTEX,
        });
        out.push(MeshWriter {
            stencil_write,
            pipeline,
            bindings,
            vertices,
        });
    }
    Ok(out)
}
