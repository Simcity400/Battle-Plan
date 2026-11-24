use super::*;
use crate::{native_gui as gui, sprites};
use eframe::wgpu;
use content_reader_image::{Rgba8Image, dds};
use std::sync::Arc;

fn images() -> Vec<dds::Texture> {
    let mut images: Vec<_> = (0..5)
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
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    images.push(dds::Texture::from_rgba8(Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![0; 4],
    }));
    images
}

fn render_part(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    part: &gui::Part,
    images: &[dds::Texture],
) -> Vec<u8> {
    let mut batch = sprites::Batch::default();
    draw::emit(part, [0, 0], &models::Sheets, &mut batch).unwrap();
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
    assert_eq!(actual.len(), expected.len(), "{label}: image sizes");
    let different = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if different != 0
        && let Some(dir) = std::env::var_os("BATTLEPLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT"))
    {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Path::new(&dir).join(format!("{label}-actual.rgba")), actual).unwrap();
        std::fs::write(
            Path::new(&dir).join(format!("{label}-reference.rgba")),
            expected,
        )
        .unwrap();
    }
    assert_eq!(different, 0, "{label}: pixels differ");
    println!("PASS {label}: 256/256 exact pixels");
}

fn sprite_inputs(
    effect: &Arc<library::Effect>,
    technique: &str,
    views: &[wgpu::TextureView],
    uv: [f32; 4],
    offsets: [f32; 4],
) -> sprites::oracle::Inputs {
    let mut constants = models::matrix([16, 16], [0., -16., -1.]);
    constants.extend([1.; 4]);
    constants.extend(offsets);
    constants.extend([0.125, 0.]);
    constants.resize(64, 0.);
    models::inputs(
        effect,
        technique,
        models::rectangle([0., 0., 16., 16.], uv, &[3, 0, 2, 1], 1., None),
        vertices::Declaration::Sprite,
        20,
        constants,
        104,
        true,
        &[(0, 0)],
        views,
        wgpu::PrimitiveTopology::TriangleStrip,
        wgpu::TextureFormat::Rgba8Unorm,
    )
}

#[expect(clippy::too_many_arguments, reason = "each input of one native text draw the test builds")]
fn text_inputs(
    effect: &Arc<library::Effect>,
    views: &[wgpu::TextureView],
    texture: usize,
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    size: [u32; 2],
    z: f32,
    background: bool,
    inline: bool,
) -> sprites::oracle::Inputs {
    let [x, y, width, height] = rect;
    let mut constants = models::matrix(size, [0., -(size[1] as f32), 0.]);
    constants.extend([1.; 4]);
    let order: &[usize] = if inline {
        &[0, 1, 3, 2]
    } else {
        &[3, 0, 2, 1, 2, 0]
    };
    let mut inputs = models::inputs(
        effect,
        "Text",
        models::rectangle(
            [x, size[1] as f32 - y - height, width, height],
            uv,
            order,
            z,
            Some((color, 0)),
        ),
        vertices::Declaration::Text,
        28,
        constants,
        80,
        false,
        &[(0, texture)],
        views,
        if inline {
            wgpu::PrimitiveTopology::TriangleStrip
        } else {
            wgpu::PrimitiveTopology::TriangleList
        },
        wgpu::TextureFormat::Bgra8Unorm,
    );
    if background {
        inputs.state.blend = None;
        inputs.state.write_mask = wgpu::ColorWrites::ALL;
    }
    inputs
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn compiled_parts_fan_animation_and_text_bake() {
    content_reader_script::test_game::enter();
    let (_install, mut library) = models::effects();
    let circular = library
        .load(b"gfx/FX/circularprogressbar.shader", &[])
        .unwrap();
    let button = library.load(b"gfx/FX/buttonstate.shader", &[]).unwrap();
    let text = library.load(b"gfx/FX/text.shader", &[]).unwrap();
    let (_serial, device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let images = images();
    let views = models::views(&device, &queue, &images);
    for (n, (weights, rotation)) in [([100, 0], 0.), ([75, 25], 0.4), ([0, 100], -0.7)]
        .into_iter()
        .enumerate()
    {
        let p = models::part(
            gui::PartKind::Fan {
                sprite: b"fan".to_vec(),
                centre: [8., 8.],
                radius: [7., 6.],
                rotation,
                weights,
            },
            circular.clone(),
            "CircularProgressBar",
        );
        let actual = render_part(&device, &queue, &p, &images);
        let inputs = models::inputs(
            &circular,
            "CircularProgressBar",
            models::fan_vertices([7., 6.], rotation, weights),
            vertices::Declaration::Circular,
            24,
            models::matrix([16, 16], [8., -8., 0.]),
            64,
            false,
            &[(0, 0), (1, 1)],
            &views,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        arena.reset();
        let oracle = sprites::oracle::Oracle::prepare(
            &device,
            &arena,
            &circular,
            "CircularProgressBar",
            inputs,
        )
        .unwrap();
        arena.flush(&queue);
        let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
        equal(&format!("fan-{n}"), &actual, &expected);
    }

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
            let p = models::part(
                gui::PartKind::Animated(Box::new(gui::Animated {
                    sprite: b"sprite".to_vec(),
                    geometry: models::geometry(),
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
            let animated =
                button.variant(vec!["ANIMATED".into(), format!("NUM_ANIMATIONS_{count}")]);
            let active = models::has_sampler_slots(&animated, technique);
            let reference = if active { animated } else { button.clone() };
            let mut inputs =
                sprite_inputs(&reference, technique, &views, [0., 0., 1., 1.], [0.; 4]);
            if active {
                for slot in 1..=4 {
                    inputs
                        .textures
                        .insert(binding(slot, Stage::Pixel), views[5].clone());
                }
                for (i, record) in records.iter().enumerate() {
                    inputs.textures.insert(
                        binding(1 + i as u32 * 2, Stage::Pixel),
                        views[record.mask].clone(),
                    );
                    inputs.textures.insert(
                        binding(2 + i as u32 * 2, Stage::Pixel),
                        views[record.texture].clone(),
                    );
                }
                models::animation_inputs(
                    &mut inputs,
                    &records.iter().map(|r| r.constants).collect::<Vec<_>>(),
                );
            }
            arena.reset();
            let oracle =
                sprites::oracle::Oracle::prepare(&device, &arena, &reference, technique, inputs)
                    .unwrap();
            arena.flush(&queue);
            let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
            equal(
                &format!("animation-{count}-{technique}"),
                &actual,
                &expected,
            );
        }
    }

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
            shading: models::part(
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
        let p = models::part(
            gui::PartKind::TextSprite {
                sprite: b"baked".to_vec(),
                bake: Box::new(bake),
                geometry: models::geometry(),
                uv: [0., 0., 0.5, 1.],
                offsets: [frame as f32 * 0.5, 0., 0., 0.],
            },
            button.clone(),
            "Up",
        );
        let actual = render_part(&device, &queue, &p, &images);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("independent reference bake"),
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
        arena.reset();
        let mut reference_draws = Vec::new();
        for (rect, uv) in quads {
            let inputs = text_inputs(
                &text,
                &views,
                0,
                rect,
                uv,
                [1.; 4],
                [32, 16],
                0.5,
                true,
                false,
            );
            reference_draws.push(
                sprites::oracle::Oracle::prepare(&device, &arena, &text, "Text", inputs).unwrap(),
            );
        }
        let glyph = glyphs[0];
        let inputs = text_inputs(
            &text,
            &views,
            glyph.texture,
            [
                glyph.at[0],
                glyph.at[1],
                glyph.size[0] as f32,
                glyph.size[1] as f32,
            ],
            glyph.uv,
            glyph.color,
            [32, 16],
            1.,
            false,
            false,
        );
        reference_draws.push(
            sprites::oracle::Oracle::prepare(&device, &arena, &text, "Text", inputs).unwrap(),
        );
        for (source, rect, uv) in inline {
            let gui::InlineSource::Texture(texture) = source else {
                unreachable!()
            };
            let inputs = text_inputs(
                &text,
                &views,
                texture,
                rect,
                uv,
                [1.; 4],
                [32, 16],
                0.,
                false,
                true,
            );
            reference_draws.push(
                sprites::oracle::Oracle::prepare(&device, &arena, &text, "Text", inputs).unwrap(),
            );
        }
        let glyph = glyphs[1];
        let inputs = text_inputs(
            &text,
            &views,
            glyph.texture,
            [
                glyph.at[0],
                glyph.at[1],
                glyph.size[0] as f32,
                glyph.size[1] as f32,
            ],
            glyph.uv,
            glyph.color,
            [32, 16],
            1.,
            false,
            false,
        );
        reference_draws.push(
            sprites::oracle::Oracle::prepare(&device, &arena, &text, "Text", inputs).unwrap(),
        );
        arena.flush(&queue);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reference background, inline and glyph order"),
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
            for oracle in &reference_draws {
                oracle.draw(&mut pass);
            }
        }
        queue.submit([encoder.finish()]);
        arena.reset();
        let inputs = sprite_inputs(
            &button,
            "Up",
            &[target],
            [0., 0., 0.5, 1.],
            [frame as f32 * 0.5, 0., 0., 0.],
        );
        let oracle =
            sprites::oracle::Oracle::prepare(&device, &arena, &button, "Up", inputs).unwrap();
        arena.flush(&queue);
        let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
        equal(&format!("text-bake-frame-{frame}"), &actual, &expected);
    }

    let first = models::part(
        gui::PartKind::Sprite {
            sprite: b"retained".to_vec(),
            geometry: models::geometry(),
            uv: [0., 0., 1., 1.],
            offsets: [0.; 4],
            masked: false,
        },
        button.clone(),
        "Up",
    );
    let second = models::part(
        gui::PartKind::Cell {
            texture: 1,
            masking: None,
            geometry: models::geometry(),
            uv: [0., 0., 1., 1.],
        },
        button.clone(),
        "FixtureUndeclaredTechnique",
    );
    assert!(
        button
            .source
            .technique("FixtureUndeclaredTechnique")
            .is_none()
    );
    let mut batch = sprites::Batch::default();
    draw::emit(&first, [0, 0], &models::Sheets, &mut batch).unwrap();
    draw::emit(&second, [0, 0], &models::Sheets, &mut batch).unwrap();
    let (actual, refused, _) = sprites::oracle::render_images(&device, &queue, &batch, &images);
    assert!(refused.is_empty(), "{refused:?}");
    arena.reset();
    let first = sprites::oracle::Oracle::prepare(
        &device,
        &arena,
        &button,
        "Up",
        sprite_inputs(&button, "Up", &views, [0., 0., 1., 1.], [0.; 4]),
    )
    .unwrap();
    let mut inputs = sprite_inputs(&button, "Up", &views, [0., 0., 1., 1.], [0.; 4]);
    inputs
        .textures
        .insert(binding(0, Stage::Pixel), views[1].clone());
    inputs.state.blend = None;
    inputs.state.write_mask = wgpu::ColorWrites::ALL;
    let second = sprites::oracle::Oracle::prepare(&device, &arena, &button, "Up", inputs).unwrap();
    arena.flush(&queue);
    let expected = render_with(&device, &queue, |pass| {
        first.draw(pass);
        second.draw(pass);
    });
    equal("retained-program-new-bindings", &actual, &expected);
}
