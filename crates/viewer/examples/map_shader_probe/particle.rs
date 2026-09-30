//! Active analytic particle ABI, projection, native depth override and blend fixture.
use super::{extent, mesh_input::native as mesh_shader, read};
use crate::{shader_constants, shader_sources};
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use serde_json::{Value, json};
use shader_pipeline::binding;
use std::{collections::BTreeMap, fs, io, path::Path};
#[path = "../../src/map/particle_shader.rs"]
mod compiled;
#[path = "particle_pass.rs"]
mod fixture;
#[path = "particle_trail.rs"]
mod trail_fixture;

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
    let effect = compiled::load(layers, map)?;
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("particle analytic atlas"),
        size: extent([64, 64]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Four distinct tiles. A one-texel guard around tile (1,1) keeps its
    // analytic colour exact at linear-filter boundaries, including wrap.
    let texels: Vec<[u16; 4]> = (0..64)
        .flat_map(|y| {
            (0..64).map(move |x| {
                if (x == 0 || x >= 31) && (y == 0 || y >= 31) {
                    [0x3800, 0x3400, 0x3c00, 0x3800]
                } else if x < 32 && y < 32 {
                    [0x3c00, 0, 0, 0x3c00]
                } else if x < 32 {
                    [0, 0x3c00, 0, 0x3c00]
                } else {
                    [0, 0, 0x3c00, 0x3c00]
                }
            })
        })
        .collect();
    queue.write_texture(
        atlas.as_image_copy(),
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64 * 8),
            rows_per_image: None,
        },
        extent([64, 64]),
    );
    let textures = BTreeMap::from([(
        binding(0, Stage::Pixel),
        atlas.create_view(&Default::default()),
    )]);
    let buffer = |label: &str, contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    };
    let vertices = buffer(
        "native particle quad",
        bytemuck::cast_slice(&compiled::QUAD),
        wgpu::BufferUsages::VERTEX,
    );
    let indices = buffer(
        "native particle indices",
        bytemuck::cast_slice(&compiled::INDICES),
        wgpu::BufferUsages::INDEX,
    );
    let identity = [
        [1_f32, 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    let mut frame = [0_f32; 104];
    frame[..16].copy_from_slice(bytemuck::cast_slice(&identity));
    frame[53] = 1.; // Nonzero near camera vector; distance below authored fog begin.
    let size = [64, 32];
    let mut rows = Vec::new();
    for samples in [1, 4] {
        let texture = |format, count| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("particle fixture target"),
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
        for (technique, source_factor, dest_factor) in [
            ("ParticleAlphaBlend", 0.5, 0.5),
            ("ParticlePreAlphaBlend", 1., 0.5),
            ("ParticleAdditive", 0.5, 1.),
        ] {
            for billboard in [false, true] {
                let name = format!("{technique}{}", if billboard { "" } else { "NoBillboard" });
                let pipeline = fixture::Pass::new(device, &effect, &name, samples, FORMAT, DEPTH)?;
                for scale in [1., 2.] {
                    for depth_clear in [0., 1.] {
                        for stencil in [0, 5] {
                            let mut world = identity;
                            world[3][0] = -0.25;
                            let packet = compiled::Packet {
                                world,
                                scale: [
                                    compiled::world_scale([
                                        [scale, 0., 0.],
                                        [0., 1., 0.],
                                        [0., 0., 1.],
                                    ]),
                                    2.,
                                ],
                                tiles: [2, 2],
                                time: 7.,
                            };
                            let buffers = packet.buffers(bytemuck::cast_slice(&frame), &identity);
                            let bindings = pipeline.bindings(&arena, &buffers, &textures)?;
                            let instances = [0.25, 0.75].map(|z| compiled::Instance {
                                position_size: [0.125, 0., z / 2., 1.],
                                rotation: [0.; 3],
                                tile: [1, 1, 0, 0],
                                color: [255; 4],
                                tail: [0; 12],
                            });
                            let instance = buffer(
                                "native48 particle records",
                                bytemuck::cast_slice(&instances),
                                wgpu::BufferUsages::VERTEX,
                            );
                            let mut encoder = device.create_command_encoder(&Default::default());
                            {
                                let mut pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("particle native-state fixture"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
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
                                            },
                                        )],
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
                                pass.set_vertex_buffer(1, instance.slice(..));
                                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint16);
                                pass.draw_indexed(0..6, 0, 0..2);
                            }
                            let bytes = read(&arena, queue, encoder, &target, size)?;
                            let clear = [0.25_f32, 0.5, 0.75, 0.625];
                            let mut expected = clear;
                            for _ in 0..2 {
                                for i in 0..3 {
                                    expected[i] = [0.5, 0.25, 1.][i] * source_factor
                                        + expected[i] * dest_factor;
                                }
                            }
                            let mut active = 0;
                            let different = bytes
                                .chunks_exact(8)
                                .enumerate()
                                .filter(|(index, bytes)| {
                                    let x = *index % size[0] as usize;
                                    let y = *index / size[0] as usize;
                                    let inside = depth_clear == 1.
                                        && (scale == 2.
                                            || (16..48).contains(&x) && (8..24).contains(&y));
                                    if inside {
                                        active += 1;
                                    }
                                    let expected = if inside { expected } else { clear };
                                    bytes
                                        .chunks_exact(2)
                                        .zip(expected)
                                        .any(|(v, e)| decode(u16::from_le_bytes([v[0], v[1]])) != e)
                                })
                                .count();
                            rows.push(json!({"technique":name,"samples":samples,"scale":scale,"depth":depth_clear,"stencil":stencil,"atlas":[2,2],"tile":[1,1],"active":active,"pixels":size[0]*size[1],"different_rgba16f_pixels":different}));
                        }
                    }
                }
            }
        }
    }
    let trails = trail_fixture::run(device, queue)?;
    let report = json!({"scope":"production particle adapter, analytic active simple quads; nonzero 2x2 atlas tile, both blend families plus premultiplied, billboard/nonbillboard, scale/translation, native depth rejection/no-write and stencil disabled; synthetic authored trail technique exercises native ribbon vertices; retained missing-program and game histories not proven", "results":rows, "synthetic_trail":trails});
    fs::write(
        output.join("particles.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if rows.iter().any(|v| v["different_rgba16f_pixels"] != 0) {
        return Err(io::Error::other(
            "particle analytic fixture differs; see particles.json",
        ));
    }
    Ok(report)
}

fn decode(v: u16) -> f32 {
    if v & 0x7fff == 0 {
        return 0.;
    }
    f32::from_bits(
        (u32::from(v & 0x8000) << 16)
            | ((u32::from(v >> 10 & 31) + 112) << 23)
            | (u32::from(v & 1023) << 13),
    )
}
