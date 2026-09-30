//! Production terrain adapters: active geometry, native winding and depth rejection.
//! This is a binding/raster fixture, not a pixel-equivalence oracle.
use super::{extent, read};
use crate::shader_constants;
use crate::shader_scene;
use crate::shader_sources;
use eframe::wgpu;
use installed_content::ResolvedLayers;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
#[path = "../../src/map/terrain_shader.rs"]
mod compiled;
// The fixture uses the production attachment formats from scene/shadow/water.
mod scene {
    pub const SCENE_FORMAT: eframe::wgpu::TextureFormat = eframe::wgpu::TextureFormat::Rgba16Float;
    pub const DEPTH_FORMAT: eframe::wgpu::TextureFormat =
        eframe::wgpu::TextureFormat::Depth24PlusStencil8;
}
mod shadow {
    pub const BUFFER_FORMAT: eframe::wgpu::TextureFormat = eframe::wgpu::TextureFormat::Bgra8Unorm;
}
mod shadow_blur {
    pub const DEPTH_FORMAT: eframe::wgpu::TextureFormat =
        eframe::wgpu::TextureFormat::Depth24PlusStencil8;
}
mod water {
    pub const REFRACTION_FORMAT: eframe::wgpu::TextureFormat =
        eframe::wgpu::TextureFormat::Bgra8Unorm;
}

pub(super) fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &ResolvedLayers,
    map: [u32; 2],
    output: &Path,
) -> io::Result<Value> {
    let arena = shader_pipeline::gpu::Arena::new(device);
    let effect = compiled::load(layers, map)?;
    let mut terrain = compiled::Terrain::new(
        device,
        &effect,
        &vec![89; (map[0] * map[1]) as usize],
        map,
        1,
    )?;
    let texture = |label, pixel: &[u8]| {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent([1, 1]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            t.as_image_copy(),
            pixel,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            t.size(),
        );
        t.create_view(&Default::default())
    };
    let null = texture("null", &[0; 4]);
    let diffuse = texture("diffuse", &[112, 157, 82, 128]);
    let normal = texture("normal", &[128, 128, 255, 128]);
    let white = texture("clear shadow", &[255; 4]);
    let retained = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preceding shadow frame"),
        size: extent([1, 1]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    queue.write_texture(
        retained.as_image_copy(),
        &[0, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: None,
        },
        retained.size(),
    );
    let retained_view = retained.create_view(&Default::default());
    let light_indices = texture("no lights", &[255; 4]);
    let data = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("native light table"),
            size: extent([128, 1]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let mut rows = Vec::new();
    for mode in [
        compiled::Mode::Receiver,
        compiled::Mode::Underwater,
        compiled::Mode::Lit(true),
        compiled::Mode::Lit(false),
    ] {
        let format = match mode {
            compiled::Mode::Lit(_) => scene::SCENE_FORMAT,
            _ => shadow::BUFFER_FORMAT,
        };
        let mut results = Vec::new();
        let mut cases = vec![
            ("active", 1., false),
            ("depth_reject", 0., false),
            ("backface", 1., true),
        ];
        if mode == compiled::Mode::Underwater {
            cases.extend([
                ("retained_dark", 1., false),
                ("retained_refreshed", 1., false),
            ]);
        }
        for (name, depth_clear, flip) in cases {
            let mut scene = [0_f32; 104];
            let sx = if flip { -1. } else { 1. };
            let matrix: [[f32; 4]; 4] = [
                [sx / 32., 0., 0., 0.],
                [0., 0., 0., 0.],
                [0., 1. / 32., 0., 0.],
                [-sx, -1., 0.5, 1.],
            ];
            scene[..16].copy_from_slice(bytemuck::cast_slice(&matrix));
            scene[52..56].copy_from_slice(&[32., 100., 32., 1.]);
            scene[71] = 1.;
            scene[83] = 1.;
            scene[60] = 1.;
            scene[100] = map[1] as f32;
            scene[101] = 1.;
            for first in [36, 40, 44, 48] {
                scene[first..first + 4].copy_from_slice(&[32., 1200., 32., 1200.]);
            }
            for first in [68, 72, 76, 80, 84, 88] {
                scene[first..first + 3].fill(0.2);
            }
            scene[92..100].copy_from_slice(&[1., 0.9, 0.8, 1., 0.3, 0.4, 0.6, 0.7]);
            let mut textures = [&null; 16];
            match mode {
                compiled::Mode::Receiver => {
                    textures[0] = &null;
                    textures[1] = &null;
                    textures[2] = &null;
                    textures[12] = &white;
                }
                compiled::Mode::Underwater => {
                    textures[0] = &diffuse;
                    textures[1] = &normal;
                    textures[2] = &diffuse;
                    textures[10] = &light_indices;
                    textures[11] = &data;
                    textures[12] = &white;
                }
                compiled::Mode::Lit(_) => {
                    textures[0] = &diffuse;
                    textures[1] = &normal;
                    textures[2] = &diffuse;
                    textures[4] = &normal;
                    textures[9] = &normal;
                    textures[10] = &light_indices;
                    textures[11] = &data;
                    textures[12] = &white;
                    textures[13] = &diffuse;
                }
            }
            if name.starts_with("retained_") {
                textures[12] = &retained_view;
            }
            let prepared = terrain.prepare(
                &arena,
                mode,
                compiled::Inputs {
                    retained: None,
                    frames: &[bytemuck::cast_slice(&scene)],
                    offsets: &[0.],
                    grid: compiled::Grid {
                        origin: [0., 0.],
                        step: 1.,
                        counts: [65, 65],
                    },
                    matrix: [[0.; 4]; 4],
                    light_grid: [1.; 4],
                    textures,
                },
            )?;
            let size = [63, 39];
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("terrain fixture target"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("terrain fixture depth"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: scene::DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let view = target.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("terrain fixture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
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
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(depth_clear),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                terrain.draw(&mut pass, &prepared);
            }
            if name == "retained_dark" {
                // Native pre-frame reads the previous target before the scene
                // refresh. This later write must only affect the next draw.
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("current shadow frame refresh"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &retained_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }));
            }
            let bytes = read(&arena, queue, encoder, &target, size)?;
            fs::write(
                output.join(format!("terrain-{}-{name}.bin", mode.technique())),
                &bytes,
            )?;
            results.push(bytes);
        }
        if results[1] != results[2] {
            return Err(io::Error::other(format!(
                "{} backface/depth rejection differs",
                mode.technique()
            )));
        }
        let stride = if format == scene::SCENE_FORMAT { 8 } else { 4 };
        let active = results[0]
            .chunks_exact(stride)
            .zip(results[1].chunks_exact(stride))
            .filter(|(a, b)| a != b)
            .count();
        if active != 63 * 39 {
            return Err(io::Error::other(format!(
                "{} has {active} active pixels, expected full coverage",
                mode.technique()
            )));
        }
        let temporal = if mode == compiled::Mode::Underwater {
            let changed = results[3]
                .chunks_exact(stride)
                .zip(results[0].chunks_exact(stride))
                .filter(|(a, b)| a != b)
                .count();
            if changed == 0 || results[4] != results[0] {
                return Err(io::Error::other(
                    "underwater did not retain the previous shadow frame until refresh",
                ));
            }
            json!({"previous_shadow_changed_pixels":changed,"next_frame_equals_refreshed_reference":true})
        } else {
            Value::Null
        };
        rows.push(json!({"technique":mode.technique(),"active_pixels":active,"depth_rejected_pixels":63*39,"backface_rejected_pixels":63*39,"temporal":temporal}));
    }
    let report = json!({"scope":"production terrain binding/raster fixture; no pixel-equivalence claim","cases":rows});
    fs::write(
        output.join("terrain.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
