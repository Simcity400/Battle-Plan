use super::*;
use crate::{native_gui as gui, sprites};

struct Sheets(sprites::SpriteSheet);
impl gui::Sheets for Sheets {
    fn sheet(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&self.0)
    }
    fn progress(&self, _: &[u8]) -> Option<[&sprites::SpriteSheet; 2]> {
        Some([&self.0, &self.0])
    }
    fn masking(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&self.0)
    }
    fn mask(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&self.0)
    }
}

fn effect(source: &str) -> Arc<shader_pipeline::library::Effect> {
    let program = content_reader_gui::pdxshader::program::read_with(b"gfx/FX/fixture.lua", &|path| {
        Some(match path {
            b"gfx/FX/fixture.shader" => source.as_bytes().to_vec(),
            _ => Vec::new(),
        })
    })
    .unwrap();
    shader_pipeline::library::Effect::from_compiled(Arc::new(program), vec![], vec![])
}

fn part(kind: PartKind, asset: Arc<shader_pipeline::library::Effect>) -> gui::Part {
    gui::Part {
        path: "fixture".into(),
        child: None,
        background_index: None,
        kind,
        clip: None,
        color: [1.; 4],
        native: None,
        shading: gui::Shading {
            compiled: Some(Effect {
                asset,
                technique: "Down".into(),
            }),
            fixture: None,
            since: 0.,
            animation: 0.,
            time: 0.,
            clocked: true,
        },
    }
}

use gui::PartKind;

#[test]
fn a_recordless_animated_technique_draws_the_plain_sprite() {
    content_reader_script::test_game::enter();
    let sheets = Sheets(sprites::SpriteSheet {
        name: "fixture".into(),
        texture: 7,
        frames: 1,
        size: [16, 16],
    });
    let geometry = gui::Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [0., 16.],
        rotation: 0.,
    };
    let asset = effect(
        "PixelShader = { Samplers = { T = { Index = 0 } } MainCode P [[ ]] }\nEffect Up { PixelShader = P }\n",
    );
    let animated = PartKind::Animated(Box::new(gui::Animated {
        sprite: b"GFX_a".to_vec(),
        geometry,
        uv: [0., 0., 1., 1.],
        offsets: [0.; 4],
        effect: "gfx/FX/fixture.lua".into(),
        technique: "Down",
        records: Vec::new(),
        masked: true,
    }));
    let mut batch = sprites::Batch::default();
    let error = emit(
        &part(animated.clone(), asset.clone()),
        [0, 0],
        &sheets,
        &mut batch,
    )
    .unwrap_err();
    assert_eq!(error.kind, gui::RefusalKind::NullRead);
    let PartKind::Animated(mut unmasked) = animated else {
        unreachable!()
    };
    unmasked.masked = false;
    emit(
        &part(PartKind::Animated(unmasked), asset),
        [0, 0],
        &sheets,
        &mut batch,
    )
    .unwrap();
    let draw = batch.quads[0]
        .exact
        .as_ref()
        .unwrap()
        .native
        .as_ref()
        .unwrap();
    assert_eq!(draw.textures, [(0, Some(7))]);
    assert!(draw.extra_constants.is_empty());
    assert!(!draw.effect.asset.defines.contains(&"ANIMATED".to_owned()));
}

#[test]
fn each_draw_submits_its_native_records_and_topology() {
    content_reader_script::test_game::enter();
    use eframe::wgpu::PrimitiveTopology::{TriangleList, TriangleStrip};
    let sheets = Sheets(sprites::SpriteSheet {
        name: "fixture".into(),
        texture: 7,
        frames: 1,
        size: [16, 16],
    });
    let geometry = gui::Geometry {
        rect: [0., 0., 16., 8.],
        local: [0., 0., 16., 8.],
        pivot: [0., 8.],
        rotation: 0.,
    };
    let asset = effect("PixelShader = { MainCode P [[ ]] }\nEffect Down { PixelShader = P }\n");
    let tile = |cornered| PartKind::Tiles {
        sprite: b"GFX_a".to_vec(),
        quads: vec![([0., 0., 16., 8.], [0., 0., 1., 1.])],
        native_origin: [0., 8.],
        cornered,
    };
    let records = |kind: PartKind, clocked: bool| {
        let mut part = part(kind, asset.clone());
        part.shading.clocked = clocked;
        let mut batch = sprites::Batch::default();
        emit(&part, [0, 0], &sheets, &mut batch).unwrap();
        let draw = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        let corners: Vec<[f32; 2]> = draw
            .vertices
            .chunks_exact(draw.stride)
            .map(|record| {
                let value = |at: usize| f32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                [value(0), value(4)]
            })
            .collect();
        (draw.topology, corners)
    };
    let sprite = PartKind::Sprite {
        sprite: b"GFX_a".to_vec(),
        geometry,
        uv: [0., 0., 1., 1.],
        offsets: [0.; 4],
        masked: false,
    };
    let (topology, corners) = records(sprite, true);
    assert_eq!(topology, TriangleStrip);
    assert_eq!(corners, [[0., 0.], [0., 8.], [16., 0.], [16., 8.]]);
    let (topology, corners) = records(tile(false), true);
    assert_eq!(topology, TriangleStrip);
    assert_eq!(corners, [[0., 0.], [0., 8.], [16., 8.], [16., 0.]]);
    let (topology, corners) = records(tile(false), false);
    assert_eq!(topology, TriangleStrip);
    assert_eq!(corners, [[0., 0.], [0., 8.], [16., 0.], [16., 8.]]);
    let (topology, corners) = records(tile(true), true);
    assert_eq!((topology, corners.len()), (TriangleList, 6));
    let inline = PartKind::Inline {
        source: gui::InlineSource::Texture(7),
        rect: [0., 0., 16., 8.],
        uv: [0., 0., 1., 1.],
        modulate: [1.; 4],
        projection: [1.; 2],
        font_viewport: None,
    };
    let (topology, corners) = records(inline, true);
    assert_eq!(topology, TriangleStrip);
    assert_eq!(corners, [[0., 0.], [16., 0.], [0., -8.], [16., -8.]]);
    let text = PartKind::Text {
        font: b"Main_14".to_vec(),
        glyphs: vec![gui::DrawnGlyph {
            ch: 'A',
            at: [0., 0.],
            color: [1.; 4],
            texture: 7,
            size: [7, 14],
            uv: [0., 0., 1., 1.],
        }],
        scale: 1.,
        modulate: [1.; 4],
        border_color: 0,
        projection: [1.; 2],
        font_viewport: None,
    };
    let (topology, corners) = records(text, true);
    assert_eq!((topology, corners.len()), (TriangleList, 6));
}

#[test]
fn each_draw_carries_its_native_allocation_and_the_bytes_it_writes() {
    content_reader_script::test_game::enter();
    let sheets = Sheets(sprites::SpriteSheet {
        name: "fixture".into(),
        texture: 7,
        frames: 1,
        size: [16, 16],
    });
    let geometry = gui::Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [0., 16.],
        rotation: 0.,
    };
    let asset = effect(
        "PixelShader = { Samplers = { A = { Index = 0 } B = { Index = 1 } } MainCode P [[ ]] }\nEffect Down { PixelShader = P }\n",
    );
    let glyph = gui::DrawnGlyph {
        ch: 'A',
        at: [0., 0.],
        color: [1.; 4],
        texture: 7,
        size: [7, 14],
        uv: [0., 0., 1., 1.],
    };
    let cases = [
        (
            PartKind::Sprite {
                sprite: b"GFX_a".to_vec(),
                geometry,
                uv: [0., 0., 1., 1.],
                offsets: [0.; 4],
                masked: false,
            },
            0x100,
            104,
        ),
        (
            PartKind::Cell {
                texture: 7,
                masking: None,
                uv: [0., 0., 1., 1.],
                geometry,
            },
            0x100,
            104,
        ),
        (
            PartKind::Tiles {
                sprite: b"GFX_a".to_vec(),
                quads: vec![([0., 0., 16., 16.], [0., 0., 1., 1.])],
                native_origin: [0., 16.],
                cornered: false,
            },
            0x100,
            104,
        ),
        (
            PartKind::Tiles {
                sprite: b"GFX_a".to_vec(),
                quads: vec![([0., 0., 16., 16.], [0., 0., 1., 1.])],
                native_origin: [0., 16.],
                cornered: true,
            },
            0x70,
            104,
        ),
        (
            PartKind::Progress {
                sprite: b"GFX_a".to_vec(),
                geometry,
                uv: [[0.; 2]; 4],
                state: 0.5,
                colors: [[1.; 4]; 2],
                textured: true,
            },
            0x70,
            100,
        ),
        (
            PartKind::MaskedFlag {
                sprite: b"GFX_a".to_vec(),
                flag: 7,
                flag_coords: [0., 0., 1., 1.],
                rect: [0., 0., 16., 16.],
                native_origin: [0., 16.],
            },
            0x60,
            96,
        ),
        (
            PartKind::Text {
                font: b"Main_14".to_vec(),
                glyphs: vec![glyph],
                scale: 1.,
                modulate: [1.; 4],
                border_color: 0,
                projection: [1.; 2],
                font_viewport: None,
            },
            0x50,
            80,
        ),
        (
            PartKind::Fan {
                sprite: b"GFX_a".to_vec(),
                centre: [8., 8.],
                radius: [8., 8.],
                rotation: 0.,
                weights: [50, 50],
            },
            0x40,
            64,
        ),
    ];
    for (kind, allocation, known) in cases {
        let label = format!("{kind:?}");
        let mut batch = sprites::Batch::default();
        emit(&part(kind, asset.clone()), [0, 0], &sheets, &mut batch).unwrap();
        let draw = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        assert_eq!(draw.constants.len() * 4, allocation, "{label}");
        assert_eq!(draw.known, known, "{label}");
    }
    let mut overlay = part(
        PartKind::Tiles {
            sprite: b"GFX_a".to_vec(),
            quads: vec![([0., 0., 16., 16.], [0., 0., 1., 1.])],
            native_origin: [0., 16.],
            cornered: false,
        },
        asset.clone(),
    );
    overlay.shading.clocked = false;
    let mut batch = sprites::Batch::default();
    emit(&overlay, [0, 0], &sheets, &mut batch).unwrap();
    let draw = batch.quads[0]
        .exact
        .as_ref()
        .unwrap()
        .native
        .clone()
        .unwrap();
    assert_eq!((draw.constants.len() * 4, draw.known), (0x70, 104));
    let animated = |typed: bool| {
        PartKind::Animated(Box::new(gui::Animated {
            sprite: b"GFX_a".to_vec(),
            geometry,
            uv: [0., 0., 1., 1.],
            offsets: [0.; 4],
            effect: "gfx/FX/fixture.lua".into(),
            technique: "Down",
            records: vec![gui::animation::AnimationDraw {
                mask: 7,
                texture: 7,
                constants: [[1.; 4]; 5],
                typed,
            }],
            masked: false,
        }))
    };
    for (typed, known) in [
        (true, vec![0..48, 80..128, 0..48, 48..68]),
        (false, vec![0..48, 80..128, 0..48, 52..56, 60..68]),
    ] {
        let mut batch = sprites::Batch::default();
        emit(
            &part(animated(typed), asset.clone()),
            [0, 0],
            &sheets,
            &mut batch,
        )
        .unwrap();
        let draw = batch.quads[0]
            .exact
            .as_ref()
            .unwrap()
            .native
            .clone()
            .unwrap();
        let (slot, constants, ranges) = &draw.extra_constants[0];
        assert_eq!((*slot, constants.len() * 4), (1, 0x100));
        assert_eq!(*ranges, known, "typed {typed}");
        assert_eq!((draw.constants.len() * 4, draw.known), (0x100, 104));
    }
}

#[test]
fn bound_portrait_texture_keeps_the_icons_mask_sampler() {
    content_reader_script::test_game::enter();
    let sheets = Sheets(sprites::SpriteSheet {
        name: "mask".into(),
        texture: 7,
        frames: 1,
        size: [16, 16],
    });
    let asset = effect(
        "PixelShader = { Samplers = { T = { Index = 0 } M = { Index = 5 } } MainCode P [[ ]] }\nEffect Down { PixelShader = P }\n",
    );
    let geometry = gui::Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [0., 16.],
        rotation: 0.,
    };
    let mut batch = sprites::Batch::default();
    emit(
        &part(
            PartKind::Cell {
                texture: 42,
                masking: Some(b"circle".to_vec()),
                geometry,
                uv: [0., 0., 1., 1.],
            },
            asset,
        ),
        [0, 0],
        &sheets,
        &mut batch,
    )
    .unwrap();
    let draw = batch.quads[0]
        .exact
        .as_ref()
        .unwrap()
        .native
        .as_ref()
        .unwrap();
    assert_eq!(draw.textures, [(0, Some(42)), (5, Some(7))]);
}
