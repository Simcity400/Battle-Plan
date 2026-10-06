//! Actual GUI submission compared with independent native model draws.
use super::*;
use crate::map::{native_effects as model, native_gui as gui, sprites};
use eframe::wgpu;
use installed_content::image::{Rgba8Image, dds};
use std::sync::Arc;

struct Sheets([sprites::SpriteSheet; 2]);
impl gui::Sheets for Sheets {
    fn sheet(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&self.0[0])
    }
    fn progress(&self, _: &[u8]) -> Option<[&sprites::SpriteSheet; 2]> {
        Some([&self.0[0], &self.0[1]])
    }
    fn masking(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        None
    }
    fn mask(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        None
    }
}
fn sheets() -> Sheets {
    Sheets(std::array::from_fn(|i| sprites::SpriteSheet {
        name: format!("fixture{i}"),
        texture: i,
        frames: 1,
        size: [16, 16],
    }))
}
fn effect(file: &str) -> Arc<library::Effect> {
    let root = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let p = installed_content::pdxshader::program::read_with(
        format!("gfx/FX/{file}.shader").as_bytes(),
        &|path| std::fs::read(root.join(String::from_utf8_lossy(path).as_ref())).ok(),
    )
    .unwrap();
    library::Effect::new(Arc::new(p), vec![], Arc::new(cache::Cache::default()))
}
fn part(kind: gui::PartKind, asset: Arc<library::Effect>, technique: &str) -> gui::Part {
    gui::Part {
        path: "offline GUI fixture".into(),
        child: None,
        kind,
        clip: None,
        color: [1.; 4],
        native: None,
        shading: gui::Shading {
            compiled: Some(draw::Effect {
                asset,
                technique: technique.into(),
            }),
            program: None,
            since: 0.125,
            animation: 0.,
            time: 0.125,
            clocked: true,
        },
    }
}
fn geometry() -> gui::Geometry {
    gui::Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [0., 16.],
        rotation: 0.,
    }
}
fn images() -> Vec<dds::Texture> {
    (0..5)
        .map(|seed| {
            dds::Texture::from_rgba8(Rgba8Image {
                width: 16,
                height: 16,
                pixels: (0..256)
                    .flat_map(|p| {
                        [
                            ((p % 16 * 16 + seed * 32) % 256) as u8,
                            ((p / 16 * 16 + seed * 16) % 256) as u8,
                            (seed * 32) as u8,
                            if seed == 0 { 128 } else { 192 },
                        ]
                    })
                    .collect(),
            })
        })
        .collect()
}
fn views(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[dds::Texture],
) -> Vec<wgpu::TextureView> {
    images
        .iter()
        .map(|image| {
            crate::map::textures::upload(
                device,
                queue,
                "fixture",
                image,
                crate::map::textures::ColorSpace::Linear,
            )
            .unwrap()
            .texture
            .create_view(&Default::default())
        })
        .collect()
}
fn render_part(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    part: &gui::Part,
    images: &[dds::Texture],
) -> Vec<u8> {
    let mut batch = sprites::Batch::default();
    draw::emit(part, [0, 0], &sheets(), &mut batch).unwrap();
    if let gui::PartKind::TextSprite { bake, .. } = &part.kind {
        let baked = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .as_ref()
            .unwrap()
            .bake
            .as_ref()
            .unwrap();
        for draw in &baked.draws {
            assert_eq!(draw.projected_constants(baked.size)[13], -1.);
        }
        let background = &baked.draws[0];
        assert_eq!(
            f32::from_le_bytes(background.vertices[4..8].try_into().unwrap()),
            bake.size[1] as f32 - 2.
        );
        assert_eq!(
            f32::from_le_bytes(background.vertices[8..12].try_into().unwrap()),
            0.5
        );
        let glyph = baked.draws.last().unwrap();
        assert_eq!(
            f32::from_le_bytes(glyph.vertices[8..12].try_into().unwrap()),
            1.
        );
    }
    let (pixels, refused, _) = sprites::oracle::render_images(device, queue, &batch, images);
    assert!(refused.is_empty(), "{refused:?}");
    pixels
}
fn equal(label: &str, actual: &[u8], expected: &[u8]) {
    let different = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if different != 0
        && let Some(dir) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT"))
    {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Path::new(&dir).join(format!("{label}-actual.rgba")), actual).unwrap();
        std::fs::write(
            Path::new(&dir).join(format!("{label}-oracle.rgba")),
            expected,
        )
        .unwrap();
    }
    assert_eq!(different, 0, "{label}: pixels differ");
    println!("PASS {label}: 256/256 exact pixels");
}
fn clip(mut q: sprites::Quad, size: [f32; 2]) -> sprites::Quad {
    for p in &mut q.exact.as_mut().unwrap().corners {
        *p = [p[0] * 2. / size[0] - 1., 1. - p[1] * 2. / size[1]];
    }
    q
}
/// `part` drawn by the model of `effect`'s `technique` instead of its installed program, with the
/// samplers that program declares.
fn model_part(mut part: gui::Part, effect: model::Effect, technique: &str) -> gui::Part {
    let model::Technique::Run { pixel, blend } = effect.technique(technique) else {
        panic!("model technique")
    };
    let installed = part.shading.compiled.take().expect("an installed program");
    let samplers = std::array::from_fn(|slot| {
        installed
            .asset
            .source
            .sampler(slot as u32)
            .map(models::sampler)
    });
    part.shading.program = Some((pixel, blend, samplers));
    part
}

#[test]
#[ignore = "requires installed effects, DXC and GPU"]
fn compiled_parts_fan_animation_and_text_bake() {
    let (_serial, device, queue) = device();
    let images = images();
    let views = views(&device, &queue, &images);
    let circular = effect("circularprogressbar");
    let blend = gpu::blend(&circular.source, &circular.source.techniques[0])
        .unwrap()
        .0;
    for (n, (weights, rotation)) in [([100, 0], 0.), ([75, 25], 0.4), ([0, 100], -0.7)]
        .into_iter()
        .enumerate()
    {
        let p = part(
            gui::PartKind::Fan {
                sprite: "fan".into(),
                centre: [8., 8.],
                radius: [7., 6.],
                rotation,
                weights,
            },
            circular.clone(),
            "CircularProgressBar",
        );
        let actual = render_part(&device, &queue, &p, &images);
        let batch = gui::oracle_part(
            &model_part(p, model::synthetic::circular(), "CircularProgressBar"),
            &sheets(),
        );
        let oracles: Vec<_> = batch
            .quads
            .into_iter()
            .map(|q| {
                sprites::oracle::Oracle::with_source(
                    &device,
                    clip(q, [16., 16.]),
                    &[&views[0], &views[1], &views[0]],
                    blend,
                    wgpu::ColorWrites::ALL,
                    &sprites::oracle::hardware_source(sprites::EXACT_SHADER),
                )
            })
            .collect();
        let expected = render_with(&device, &queue, |pass| {
            for oracle in &oracles {
                oracle.draw(pass)
            }
        });
        equal(&format!("fan-{n}"), &actual, &expected);
    }
    let button = effect("buttonstate");
    // The model census covers all animation techniques/types/modes; these draws verify
    // variant selection, b1 and slot1..4 through the actual sorted upload path.
    for count in 1..=2 {
        for technique in ["Up", "Down", "Over", "Disable"] {
            let records: Vec<_> = (0..count)
                .map(|i| gui::animation::AnimationDraw {
                    mask: 1 + i * 2,
                    texture: 2 + i * 2,
                    constants: [
                        [1., 0., 0., 1.],
                        [1.; 4],
                        [0., 0., 1., 1.],
                        [0.25, 0.75, (i + 1) as f32, 3.],
                        [1., 0., 0., 0.],
                    ],
                    typed: true,
                })
                .collect();
            let p = part(
                gui::PartKind::Animated(Box::new(gui::Animated {
                    sprite: "sprite".into(),
                    geometry: geometry(),
                    uv: [0., 0., 1., 1.],
                    offsets: [0.; 4],
                    effect: button.source.effect.clone(),
                    technique,
                    records: records.clone(),
                    masked: false,
                })),
                button.clone(),
                technique,
            );
            let actual = render_part(&device, &queue, &p, &images);
            let ordinary = part(
                gui::PartKind::Sprite {
                    sprite: "sprite".into(),
                    geometry: geometry(),
                    uv: [0., 0., 1., 1.],
                    offsets: [0.; 4],
                    masked: false,
                },
                button.clone(),
                technique,
            );
            let model = model::synthetic::buttonstate();
            let q = gui::oracle_part(&model_part(ordinary, model, technique), &sheets())
                .quads
                .remove(0);
            let code = sprites::oracle::animated_source(
                &records.iter().map(|r| r.constants).collect::<Vec<_>>(),
                true,
            );
            let all = [
                &views[0], &views[1], &views[0], &views[2], &views[3], &views[4],
            ];
            let blend = gpu::blend(&button.source, button.source.technique(technique).unwrap())
                .unwrap()
                .0;
            let oracle = sprites::oracle::Oracle::with_source(
                &device,
                clip(q, [16., 16.]),
                &all[..if count == 1 { 4 } else { 6 }],
                blend,
                wgpu::ColorWrites::ALL,
                &code,
            );
            let oracle = oracle.animation(
                &device,
                &records.iter().map(|r| r.constants).collect::<Vec<_>>(),
            );
            let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
            equal(
                &format!("animation-{count}-{technique}"),
                &actual,
                &expected,
            );
        }
    }
    let text = effect("text");
    let mut textblend = gpu::blend(&text.source, text.source.technique("Text").unwrap())
        .unwrap()
        .0
        .unwrap();
    textblend.alpha = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    for frame in 0..2 {
        let quads = (0..3)
            .flat_map(|y| {
                (0..3).map(move |x| {
                    let xs = [0., 2., 30., 32.];
                    let ys = [0., 2., 14., 16.];
                    let us = [0., 0.125, 0.875, 1.];
                    (
                        [xs[x], ys[y], xs[x + 1] - xs[x], ys[y + 1] - ys[y]],
                        [us[x], us[y], us[x + 1], us[y + 1]],
                    )
                })
            })
            .collect::<Vec<_>>();
        let glyphs = vec![
            gui::DrawnGlyph {
                ch: 'A',
                at: [3., 1.],
                color: [1.; 4],
                texture: 1,
                size: [10, 12],
                uv: [0., 0., 1., 1.],
            },
            gui::DrawnGlyph {
                ch: 'B',
                at: [18., 3.],
                color: [1.; 4],
                texture: 2,
                size: [12, 10],
                uv: [0., 0., 1., 1.],
            },
        ];
        let inline = vec![(
            gui::InlineSource::Texture(3),
            [24., 8., 7., 8.],
            [0., 0., 1., 1.],
        )];
        let bake = gui::text_sprites::TextBake {
            size: [32, 16],
            background: Some(gui::text_sprites::Background {
                source: gui::InlineSource::Texture(0),
                quads: quads.clone(),
            }),
            glyphs: glyphs.clone(),
            border_color: 0,
            inline: inline.clone(),
            frame_ends: vec![[1, 0], [2, 1]],
            shading: part(
                gui::PartKind::Inline {
                    source: gui::InlineSource::Texture(0),
                    rect: [0.; 4],
                    uv: [0.; 4],
                    modulate: [1.; 4],
                    projection: [1.; 2],
                    font_viewport: None,
                },
                text.clone(),
                "Text",
            )
            .shading,
        };
        let p = part(
            gui::PartKind::TextSprite {
                sprite: "baked".into(),
                bake: Box::new(bake),
                geometry: geometry(),
                uv: [0., 0., 0.5, 1.],
                offsets: [frame as f32 * 0.5, 0., 0., 0.],
            },
            button.clone(),
            "Up",
        );
        let actual = render_part(&device, &queue, &p, &images);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("independent model bake"),
            size: wgpu::Extent3d {
                width: 32,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let target = target.create_view(&Default::default());
        let mut oracles = Vec::new();
        let mut add = |kind, blend| {
            let p = part(kind, text.clone(), "Text");
            for q in
                gui::oracle_part(&model_part(p, model::synthetic::text(), "Text"), &sheets()).quads
            {
                let view = &views[q.texture];
                oracles.push(sprites::oracle::Oracle::with_format(
                    &device,
                    clip(q, [32., 16.]),
                    &[view, view, view],
                    blend,
                    wgpu::ColorWrites::ALL,
                    &sprites::oracle::hardware_source(sprites::EXACT_SHADER),
                    wgpu::TextureFormat::Bgra8Unorm,
                ));
            }
        };
        for (rect, uv) in quads {
            add(
                gui::PartKind::Inline {
                    source: gui::InlineSource::Texture(0),
                    rect,
                    uv,
                    modulate: [1.; 4],
                    projection: [1.; 2],
                    font_viewport: None,
                },
                None,
            );
        }
        add(
            gui::PartKind::Text {
                font: "fixture".into(),
                glyphs: vec![glyphs[0]],
                scale: 1.,
                modulate: [1.; 4],
                border_color: 0,
                projection: [1.; 2],
                font_viewport: None,
            },
            Some(textblend),
        );
        for (source, rect, uv) in inline {
            add(
                gui::PartKind::Inline {
                    source,
                    rect,
                    uv,
                    modulate: [1.; 4],
                    projection: [1.; 2],
                    font_viewport: None,
                },
                Some(textblend),
            );
        }
        add(
            gui::PartKind::Text {
                font: "fixture".into(),
                glyphs: vec![glyphs[1]],
                scale: 1.,
                modulate: [1.; 4],
                border_color: 0,
                projection: [1.; 2],
                font_viewport: None,
            },
            Some(textblend),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for oracle in &oracles {
                oracle.draw(&mut pass);
            }
        }
        queue.submit([encoder.finish()]);
        let p = part(
            gui::PartKind::Sprite {
                sprite: "baked".into(),
                geometry: geometry(),
                uv: [0., 0., 0.5, 1.],
                offsets: [frame as f32 * 0.5, 0., 0., 0.],
                masked: false,
            },
            button.clone(),
            "Up",
        );
        let q = gui::oracle_part(
            &model_part(p, model::synthetic::buttonstate(), "Up"),
            &sheets(),
        )
        .quads
        .remove(0);
        let blend = gpu::blend(&button.source, button.source.technique("Up").unwrap())
            .unwrap()
            .0;
        let oracle = sprites::oracle::Oracle::with_source(
            &device,
            clip(q, [16., 16.]),
            &[&target, &target, &target],
            blend,
            wgpu::ColorWrites::ALL,
            &sprites::oracle::hardware_source(sprites::EXACT_SHADER),
        );
        let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
        equal(&format!("text-bake-frame-{frame}"), &actual, &expected);
    }
}
