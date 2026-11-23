use crate::{
    native_effects::{Effect, synthetic},
    native_gui::*,
    native_text,
    native_windows::{NAVAL_COMBAT_WINDOW, NAVY_WINDOW, NativeWindows},
    sprites::SpriteSheet,
};
use content_reader_gui::gui::native::{self, Element, fonts::{FontFacts, TextColors}};
use content_reader_localisation::localisation::Localisation;
use std::collections::HashMap;

pub struct Fixture {
    pub(crate) templates: Vec<native::Gui>,
    pub sprites: HashMap<&'static str, SpriteInfo>,
    pub(crate) sheets: HashMap<&'static str, SpriteSheet>,
    pub font: native_text::Font,
    pub(crate) localisation: Localisation,
    pub(crate) global: TextColors,
    pub(crate) animations: HashMap<&'static str, Vec<AnimationRecord>>,
    pub(crate) text: Option<Effect>,
    pub(crate) buttonstate: Option<Effect>,
    pub physical: Option<[i32; 2]>,
    pub alpha: HashMap<&'static str, AlphaMask>,
}

impl Resources for Fixture {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        self.physical
    }
    fn portrait_texture(&self, name: &[u8]) -> Option<Texture> {
        let sheet = self.sheet(name)?;
        Some(Texture {
            texture: sheet.texture,
            uv: [0.0, 0.0, 1.0, 1.0],
            size: sheet.size.map(|n| n as i32),
        })
    }

    fn sprite_alpha(&self, name: &[u8]) -> Option<AlphaMask> {
        self.alpha.get(std::str::from_utf8(name).ok()?).cloned()
    }

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        name == b"GFX_unpreloaded"
    }

    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.sprites.get(std::str::from_utf8(name).ok()?).cloned()
    }

    fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        (name == b"Main_14" || name.is_empty()).then_some(&self.font)
    }

    fn localisation(&self) -> &Localisation {
        &self.localisation
    }

    fn text_colors(&self) -> &TextColors {
        &self.global
    }

    fn text_effect(&self) -> Result<Effect, String> {
        Ok(self.text.clone().unwrap_or_else(synthetic::text))
    }

    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
        match tag {
            "GER" => Ok(Some(Texture {
                texture: 9,
                uv: [0.25, 0.5, 0.5, 0.75],
                size: [10, 7],
            })),
            "NUL" => Ok(None),
            _ => Err(format!("the recording holds no country table for {tag}")),
        }
    }

    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
        if key == b"GER_group" || key == b"GER-Western_group" {
            Ok(Some(Texture {
                texture: 10,
                uv: [0.0, 0.0, 0.25, 0.25],
                size: [10, 7],
            }))
        } else {
            Err(format!(
                "resolved atlas key {} has no fixture record",
                lossy(key)
            ))
        }
    }

    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
        ["GER_group", "GER-Western_group"]
            .into_iter()
            .map(|key| key.chars().collect::<Vec<_>>())
            .filter(|key| text.starts_with(key))
            .map(|key| key.len())
            .max()
    }

    fn effect(&self, file: &str) -> Result<Effect, String> {
        match file {
            "gfx/FX/buttonstate.shader" | "gfx/FX/buttonstate.lua" => Ok(self
                .buttonstate
                .clone()
                .unwrap_or_else(synthetic::buttonstate)),
            _ => Err(format!("custom effect {file}")),
        }
    }

    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.animations
            .get(std::str::from_utf8(sprite).ok()?)
            .cloned()
    }

    fn gui_type(&self, name: &[u8]) -> Option<&Element> {
        self.templates.iter().find_map(|gui| gui.template(name))
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.templates.iter().any(|gui| gui.text_box_named(name))
    }

    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        Ok((small && index == 0).then_some((
            Texture {
                texture: 11,
                uv: [0.0; 4],
                size: [0; 2],
            },
            [0.5, 0.25, 0.125, 0.25],
        )))
    }
}

pub(crate) fn plain(frame: [i32; 2], frames: i32) -> SpriteInfo {
    SpriteInfo {
        texture: [frame[0] * frames, frame[1]],
        frames,
        vertical: false,
        masked: false,
        animated: false,
        kind: SpriteKind::Plain,
        anchor: [0.0; 2],
        effect: Ok(synthetic::buttonstate()),
        hit: Hit::Frame,
        missing: None,
        refusals: Vec::new(),
    }
}

pub fn fixture() -> Fixture {
    let mut fixture = Fixture {
        templates: Vec::new(),
        sheets: HashMap::new(),
        sprites: HashMap::from([
            ("GFX_odd", plain([15, 9], 3)),
            ("GFX_even", plain([20, 10], 1)),
            (
                "GFX_anchored",
                SpriteInfo {
                    anchor: [4.7, 2.2],
                    ..plain([20, 10], 1)
                },
            ),
            (
                "GFX_bar",
                SpriteInfo {
                    texture: [154, 14],
                    frames: 1,
                    vertical: false,
                    masked: false,
                    animated: false,
                    kind: SpriteKind::Progress {
                        size: [154, 14],
                        steps: 100,
                        colors: [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.5]],
                        horizontal: true,
                        textured: false,
                    },
                    anchor: [9.0, 9.0],
                    effect: Ok(synthetic::progress()),
                    hit: Hit::Frame,
                    missing: None,
                    refusals: Vec::new(),
                },
            ),
            (
                "GFX_upright_bar",
                SpriteInfo {
                    texture: [8, 20],
                    frames: 1,
                    vertical: false,
                    masked: false,
                    animated: false,
                    kind: SpriteKind::Progress {
                        size: [8, 20],
                        steps: 0,
                        colors: [[1.0; 4]; 2],
                        horizontal: false,
                        textured: true,
                    },
                    anchor: [0.0; 2],
                    effect: Ok(synthetic::progress()),
                    hit: Hit::Frame,
                    missing: None,
                    refusals: Vec::new(),
                },
            ),
            ("GFX_default_fallback_texture", plain([8, 8], 1)),
            ("GFX_bar3x17", plain([3, 17], 1)),
            (
                "GFX_thirds",
                SpriteInfo {
                    texture: [10, 4],
                    ..plain([10, 4], 3)
                },
            ),
            (
                "GFX_column",
                SpriteInfo {
                    texture: [6, 12],
                    vertical: true,
                    ..plain([6, 4], 3)
                },
            ),
            ("GFX_four", plain([10, 10], 4)),
            (
                "GFX_through",
                SpriteInfo {
                    hit: Hit::Through,
                    ..plain([10, 10], 4)
                },
            ),
            (
                "GFX_alpha",
                SpriteInfo {
                    hit: Hit::Alpha,
                    ..plain([10, 10], 4)
                },
            ),
            (
                "GFX_tile",
                SpriteInfo {
                    kind: SpriteKind::Text { textured: true },
                    ..plain([3, 3], 1)
                },
            ),
            (
                "GFX_cornered",
                SpriteInfo {
                    texture: [12, 12],
                    kind: SpriteKind::CorneredTile {
                        size: [30, 20],
                        border: [4, 4],
                        tiling_center: false,
                        looping: false,
                        rate: None,
                    },
                    ..plain([12, 12], 1)
                },
            ),
            (
                "GFX_text",
                SpriteInfo {
                    texture: [120, 20],
                    kind: SpriteKind::Text { textured: true },
                    ..plain([60, 20], 2)
                },
            ),
            (
                "GFX_repeat",
                SpriteInfo {
                    kind: SpriteKind::Tile { size: [25, 10] },
                    ..plain([10, 4], 1)
                },
            ),
            (
                "GFX_resizeable",
                SpriteInfo {
                    kind: SpriteKind::Resizeable,
                    ..plain([10, 4], 1)
                },
            ),
            (
                "GFX_animated",
                SpriteInfo {
                    kind: SpriteKind::FrameAnimated {
                        seconds_per_frame: 0.5,
                        looping: true,
                        play_on_show: true,
                        pause: 0.0,
                    },
                    ..plain([10, 4], 4)
                },
            ),
            (
                "GFX_shield",
                SpriteInfo {
                    texture: [24, 30],
                    kind: SpriteKind::MaskedShield {
                        mask: [20, 24],
                        small: true,
                    },
                    ..plain([24, 30], 1)
                },
            ),
            (
                "GFX_custom",
                SpriteInfo {
                    effect: Err("custom effect gfx/FX/mod.lua".into()),
                    ..plain([3, 3], 1)
                },
            ),
            (
                "GFX_no_down",
                SpriteInfo {
                    effect: Ok(Effect {
                        techniques: synthetic::buttonstate()
                            .techniques
                            .into_iter()
                            .filter(|(name, _, _)| *name != "Down")
                            .collect(),
                        ..synthetic::buttonstate()
                    }),
                    ..plain([10, 10], 4)
                },
            ),
        ]),
        font: native_text::Font {
            facts: FontFacts {
                color: 0xFF80_4020,
                text_colors: TextColors::from([(b'R', [1.0, 0.0, 0.0, 1.0])]),
                ..native_text::monospace("Main_14", 7, 14).facts
            },
            ..native_text::monospace("Main_14", 7, 14)
        },
        localisation: [
            ("MOD_KEY", "ab $MOD_INNER$"),
            ("MOD_INNER", "c"),
            ("MOD_SCRIPTED", "[GetDateText]"),
            ("MOD_COLOURED", "a\u{a7}Rb\u{a7}!c \u{a3}odd\u{a3}d"),
            ("MOD_LONG", "aaaa bbbb cccc"),
            ("MOD_LINES", "a\nb\nc\nd\ne\nf\ng"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect(),
        global: TextColors::from([(b'b', [0.0, 0.0, 1.0, 1.0])]),
        animations: HashMap::new(),
        text: None,
        buttonstate: None,
        physical: None,
        alpha: HashMap::new(),
    };
    for (&name, info) in &fixture.sprites {
        fixture.sheets.insert(
            name,
            SpriteSheet {
                name: name.into(),
                texture: 0,
                frames: info.frames as u32,
                size: info.texture.map(|v| v as u32),
            },
        );
    }
    fixture
}

pub fn windows(source: &[u8]) -> NativeWindows {
    let mut bytes = format!("guiTypes = {{ containerWindowType = {{ name = {} }} containerWindowType = {{ name = {} }} }}",
        NAVY_WINDOW, NAVAL_COMBAT_WINDOW).into_bytes();
    bytes.extend_from_slice(source);
    NativeWindows::read(&bytes, [2048, 1152]).unwrap()
}

pub fn fixture_with(document: native::Gui) -> Fixture {
    Fixture {
        templates: vec![document],
        ..fixture()
    }
}

impl Fixture {
    pub fn add_sprite(&mut self, name: &'static str, frame: [i32; 2], frames: i32) {
        let sprite = plain(frame, frames);
        let size = sprite.texture.map(|v| v as u32);
        self.sprites.insert(name, sprite);
        self.sheets.insert(
            name,
            SpriteSheet {
                name: name.into(),
                texture: self.sheets.len(),
                frames: frames as u32,
                size,
            },
        );
    }
}

impl Sheets for Fixture {
    fn sheet(&self, name: &[u8]) -> Option<&SpriteSheet> {
        self.sheets.get(std::str::from_utf8(name).ok()?)
    }
    fn progress(&self, _: &[u8]) -> Option<[&SpriteSheet; 2]> {
        None
    }
    fn masking(&self, _: &[u8]) -> Option<&SpriteSheet> {
        None
    }
    fn mask(&self, _: &[u8]) -> Option<&SpriteSheet> {
        None
    }
}
