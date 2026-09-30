//! Native mesh stream/skin probes. Actor palettes and map submission remain separate.
use super::{extent, read};
use crate::shader_constants;
use crate::shader_sources;
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::binding;
use std::{collections::BTreeMap, fs, io, path::Path};
#[path = "../../src/map/mesh_shader.rs"]
pub(super) mod native;

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
    let (mut library, defines) = shader_sources::library(layers, map);
    let size = [64, 32];
    let mut rows = Vec::new();
    for uv1 in [false, true] {
        let mut defines = defines.clone();
        if uv1 {
            defines.push("PDX_MESH_UV1".into());
        }
        let effect = library
            .load(b"gfx/FX/pdxmesh.lua", &defines)
            .map_err(io::Error::other)?;
        for (case, joints, weights) in [
            ("ordinary", [0, 1, 2, 3], [0.125, 0.25, 0.125, 0.5]),
            (
                "fourth_weight_ignored",
                [0, 1, 2, 3],
                [0.125, 0.25, 0.125, 7.],
            ),
            ("negative_indices", [-1, 1, -4, 3], [0.75, 0.25, 0.625, 0.]),
            (
                "byte_truncation",
                [256, 257, 258, 259],
                [0.125, 0.25, 0.125, 0.],
            ),
            (
                "fourth_negative_index",
                [0, 1, 2, -1],
                [0.125, 0.25, 0.125, 0.],
            ),
            ("negative_weight", [0, 1, 2, 3], [-0.25, 0.5, 0.25, 0.]),
        ] {
            let skin = native::SkinVertex::from_authored(joints, weights);
            let w = [
                skin.weights[0],
                skin.weights[1],
                skin.weights[2],
                1. - skin.weights[0] - skin.weights[1] - skin.weights[2],
            ];
            // Exact binary fractions make the independent CPU position oracle exact.
            let shifts = [
                [0., 0., 0.],
                [0.125, 0., 0.],
                [0., 0.125, 0.],
                [-0.125, 0., 0.],
            ];
            let mut shift = [0.; 3];
            for (joint, weight) in skin.indices.into_iter().zip(w) {
                for (out, value) in shift.iter_mut().zip(shifts[usize::from(joint)]) {
                    *out += value * weight;
                }
            }
            let mut palette = [[0_f32; 16]; 50];
            for matrix in &mut palette {
                for i in [0, 5, 10, 15] {
                    matrix[i] = 1.;
                }
            }
            for (matrix, shift) in palette.iter_mut().zip(shifts) {
                matrix[12..15].copy_from_slice(&shift);
            }
            let mut results = Vec::new();
            for skinned in [false, true] {
                let name = if skinned {
                    "PdxMeshStandardSkinnedShadow"
                } else {
                    "PdxMeshStandardShadow"
                };
                let selection = effect
                    .select(name)
                    .map_err(io::Error::other)?
                    .ok_or_else(|| io::Error::other(format!("mesh fixture missing {name}")))?;
                let mut frame = vec![0_f32; 104];
                for i in [0, 5, 10, 15] {
                    frame[i] = 1.;
                }
                let mut object = vec![0_f32; 52];
                for i in [0, 5, 10, 15] {
                    object[i] = 1.;
                }
                object[0] = 0.5;
                object[5] = 0.75;
                object[12] = 0.125;
                let buffers = BTreeMap::from([
                    (
                        binding(0, Stage::Vertex),
                        bytemuck::cast_slice(&frame).to_vec(),
                    ),
                    (
                        binding(1, Stage::Vertex),
                        bytemuck::cast_slice(&object).to_vec(),
                    ),
                    (
                        binding(2, Stage::Vertex),
                        bytemuck::cast_slice(&palette).to_vec(),
                    ),
                ]);
                let points = [
                    [-0.75, -0.75, 0.5],
                    [-0.75, 0.75, 0.5],
                    [0.75, -0.75, 0.5],
                    [0.75, -0.75, 0.5],
                    [-0.75, 0.75, 0.5],
                    [0.75, 0.75, 0.5],
                ];
                let mut sub = installed_content::mesh::SubMesh::default();
                for mut point in points {
                    if !skinned {
                        for (value, shift) in point.iter_mut().zip(shift) {
                            *value += shift;
                        }
                    }
                    sub.positions.push(point);
                    sub.normals.push([0., 0., 1.]);
                    sub.tangents.push([1., 0., 0., 1.]);
                    sub.uv0.push([0., 0.]);
                }
                sub.uv1 = uv1.then(|| vec![[0.25, 0.75]; 6]);
                if skinned {
                    sub.skin = Some(installed_content::mesh::Skin {
                        joints: vec![joints; 6],
                        weights: vec![weights; 6],
                        ..Default::default()
                    });
                }
                let geometry = native::Geometry::from_mesh(&sub, 1.)?;
                let pipeline = if skinned {
                    native::Pass::caster(
                        &mut Default::default(),
                        device,
                        &selection,
                        &geometry,
                        FORMAT,
                        DEPTH,
                    )?
                } else {
                    native::Pass::new(
                        &mut Default::default(),
                        device,
                        &selection,
                        &geometry,
                        FORMAT,
                        DEPTH,
                        1,
                    )?
                };
                let bindings = pipeline
                    .pipeline
                    .bindings(&arena, &buffers, &BTreeMap::new())
                    .map_err(io::Error::other)?;
                let buffer = |bytes| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytes,
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                };
                let vertex = buffer(bytemuck::cast_slice(&geometry.vertices));
                let skin = skinned.then(|| buffer(bytemuck::cast_slice(&geometry.skin)));
                let texture = |format| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: None,
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
                let view = target.create_view(&Default::default());
                let depth = texture(DEPTH);
                let depth = depth.create_view(&Default::default());
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
                                    r: 1.,
                                    g: 0.,
                                    b: 1.,
                                    a: 1.,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0),
                                store: wgpu::StoreOp::Store,
                            }),
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pipeline.begin(&mut pass);
                    bindings.bind(&mut pass);
                    pass.set_vertex_buffer(0, vertex.slice(..));
                    if let Some(skin) = &skin {
                        pass.set_vertex_buffer(1, skin.slice(..));
                    }
                    pass.draw(0..6, 0..1);
                }
                results.push(read(&arena, queue, encoder, &target, size)?);
            }
            let different = results[0]
                .chunks_exact(8)
                .zip(results[1].chunks_exact(8))
                .filter(|(a, b)| a != b)
                .count();
            let active = results[1]
                .chunks_exact(8)
                .filter(|pixel| *pixel == [0, 0x38, 0, 0x38, 0, 0x38, 0, 0x3c])
                .count();
            if different != 0 || active == 0 {
                return Err(io::Error::other(format!(
                    "mesh native skin {case}/uv1={uv1}: {different} different, {active} active"
                )));
            }
            rows.push(json!({"case":case,"uv1":uv1,"pixels":size[0]*size[1],"active":active,"different_rgba16f_pixels":different}));
        }
    }
    let report = json!({"scope":"native48/56-byte mesh declaration and16-byte skin upload with content shadow shaders; independent CPU affine-skin oracle; actor palette/history/material/scene binding integration not proven", "results":rows});
    fs::write(
        output.join("mesh-input-fixture.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
