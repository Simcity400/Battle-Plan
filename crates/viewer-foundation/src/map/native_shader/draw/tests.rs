use super::*;
use crate::map::{native_gui as gui, sprites};

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

fn effect(source: &str) -> Arc<super::super::library::Effect> {
    let program = installed_content::pdxshader::program::read_with(b"gfx/FX/fixture.lua", &|path| {
        Some(match path {
            b"gfx/FX/fixture.shader" => source.as_bytes().to_vec(),
            _ => Vec::new(),
        })
    })
    .unwrap();
    super::super::library::Effect::from_compiled(Arc::new(program), vec![], vec![])
}

fn part(kind: PartKind, asset: Arc<super::super::library::Effect>) -> gui::Part {
    gui::Part {
        path: "fixture".into(),
        child: None,
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

/// An animated state whose technique has no sampler slots draws as the plain sprite,
/// masking included, and binds none of the records' slots 1-4 or b1.
#[test]
fn a_recordless_animated_technique_draws_the_plain_sprite() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Up is declared (with a sampler), Down is not.
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
    // The masked plain sprite then reads the null record: the game's null read, refused.
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

/// Each draw submits its native records in its native topology: four as a strip for a
/// sprite, a bound texture, a progress bar, a shield's flag and overlay (bottom-left,
/// top-left, bottom-right, top-right), a plain tile (bottom-left, top-left, top-right,
/// bottom-right) and an inline icon (top-left, top-right, bottom-left, bottom-right); six
/// per quad as a list for glyphs and cornered tiles.
#[test]
fn each_draw_submits_its_native_records_and_topology() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Each record's position (x, y) in the part's local frame, y up.
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
    // The overlay's buffer is ordered as a sprite's.
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

/// Each draw uploads its whole b0 allocation but writes only a prefix: sprites write
/// 104 of 112 bytes and expose a pooled 256-byte slot; cornered tiles and shield overlays
/// write 104 of 112 unpooled bytes; progress bars write 100 of 112; masked flags write all
/// 96, including a 16-byte zero tail; text writes all 80; circular progress writes all 64.
/// The payload carries both allocation and written prefix without claiming unwritten zeros.
/// Animation b1 holds two 80-byte records, each with 48 zeroed bytes and the active record's
/// writes through byte 68, including floats 12 and 14 only for types 1 to 4.
#[test]
fn each_draw_carries_its_native_allocation_and_the_bytes_it_writes() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // A masked shield's overlay: a tile whose constants carry no clock, unpooled.
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
