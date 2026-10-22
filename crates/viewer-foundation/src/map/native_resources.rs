//! The content stack's GUI resources as the game loads them, for [`super::native_gui`]: every
//! sprite, texture, font and effect uses [`installed_content_core::gui::native::resources`].
//!
//! - Sprites: [`Registry::create`] selects the latest byte-preserving declaration of a name.
//!   [`resources::stack_texture`] tries `.dds`, `.tga`, then the supplied name, with the
//!   texture loader's 16 MiB limit. Failed textures stay null ([`NO_TEXTURE`]), sampling black.
//! - Effects: [`Effect::load`] uses the shared shader-pipeline library, with the sprite's
//!   `MASKING` define when its `masking_texture` loads.
//! - Fonts: [`installed_content_core::gui::native::fonts::Fonts::get_or_fallback`] searches the registry,
//!   selects the language's family, and loads files in order through
//!   [`installed_content_core::gui::native::fonts::load_glyphs`]. Page names append `.tga` and use the
//!   texture resolver.
//!
//! - Visual mods: a texture or font file the mods break or hide is retried by its path from the
//!   recording, as the map draws; given the recording's own registry
//!   ([`GuiResources::with_recorded_registry`], game windows), a sprite or font is instead drawn
//!   as a whole from the recording's own declaration when the mods' cannot be drawn. Textures
//!   are kept per stack read, so the two never share a load.
//!
//! Callers acquire template assets ([`super::native_gui::assets_in`]) and bound text's inline
//! icons ([`super::native_gui::Assets::add_text`]) once, then lay out and draw through these
//! resources. This supplies [`super::native_gui::Sheets`] and the resource half of
//! [`super::native_gui::Resources`]; callers supply recording localisation and flags.

use crate::map::native_windows::NativeWindows;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

use installed_content_core::{
    ResolvedLayers, gui::native::fonts, gui::native::fonts::TextColors, gui::native::resources,
    gui::native::resources::Lookup, gui::native::resources::NoTexture,
    gui::native::resources::Registry, gui::native::sprites::Plain,
    gui::native::sprites::SpriteFacts, gui::native::sprites::SpriteKind as Declared,
};
use installed_content_image::{dds, native::Decode};

use super::{
    native_effects::Effect,
    native_gui::{
        AlphaMask, AnimationRecord, Assets, FALLBACK_SPRITE, Hit, RefusalKind, Sheets, SpriteInfo,
        SpriteKind, Texture, TextureFile, lossy,
    },
    native_text::{self, Page},
    sprites::{NO_TEXTURE, SpriteSheet},
};

/// The effect bitmap fonts draw with.
pub const TEXT_EFFECT: &str = "gfx/FX/text.lua";

/// The effect a circular progress bar draws with.
pub const CIRCULAR_EFFECT: &str = "gfx/FX/circularprogressbar.shader";

pub use installed_content_core::gui::native::resources::SHIELD_OVERLAY_EFFECT;

/// The default effect for sprite types with an empty `effectFile`, including text sprites.
pub const DEFAULT_SPRITE_EFFECT: &str = "gfx/FX/buttonstate.lua";

/// A texture as loaded: its id among the caller's images and its extent, or the null texture
/// with why it does not load.
#[derive(Clone, Debug, PartialEq)]
struct Loaded {
    texture: usize,
    size: [u32; 2],
    missing: Option<String>,
    /// A texture whose draw the file or the recording does not determine
    /// ([`resources::NoTexture::native_failure`]). It is the null texture here, and what draws
    /// with it is refused by this.
    refused: Option<(RefusalKind, String)>,
}

/// Which stack a texture is read from, which keeps its load apart from the same path's on
/// another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stack {
    /// The stack handed in, and, when it shows visual mods, a file they break or hide read from
    /// the recording by its path: what the map reads.
    Retried,
    /// The stack handed in alone: the visual mods' attempt at a sprite or font drawn as a whole
    /// from the recording's own declaration when it fails ([`GuiResources::with_recorded_registry`]).
    Shown,
    /// The recorded layers below the visual mods, read for the recording's own declaration.
    Recorded,
}

/// What chose a loaded font: the registered font a name resolves to, as the map keeps it, or,
/// with the recording's own registry, the asked name, whose fallback to the recording's own
/// declaration of it depends on the name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FontKey {
    Registered(usize),
    Named(Vec<u8>),
}

/// What loading one font noted, applied only for the attempt that is kept: files and pages no
/// stack gives, and files read outside the content coverage.
#[derive(Default)]
struct FontNotes {
    refused: Vec<String>,
    missing: Vec<(String, String)>,
    uncovered: Vec<Vec<u8>>,
}

/// One prepared sprite: what the layout needs, and the textures each slot draws.
#[derive(Clone, Debug)]
struct Prepared {
    info: SpriteInfo,
    /// The main texture (a plain kind's `textureFile`, a cornered tile's, a progress bar's
    /// `textureFile1`, a masked shield's overlay `textureFile1`).
    sheet: SpriteSheet,
    /// A progress bar's `textureFile2`, a masked shield's mask `textureFile2`.
    second: Option<SpriteSheet>,
    /// A plain kind's `masking_texture`, when it loads.
    masking: Option<SpriteSheet>,
    /// The animation records an eagerly loaded plain sprite or tile keeps.
    animations: Vec<AnimationRecord>,
}

fn plain_info(
    plain: &Plain,
    main: &Loaded,
    masked: bool,
    animated: bool,
    kind: SpriteKind,
    effect: Result<Effect, String>,
    refusals: Vec<(RefusalKind, String)>,
) -> SpriteInfo {
    SpriteInfo {
        texture: main.size.map(|value| value as i32),
        frames: plain.frames,
        vertical: plain.frame_direction_y(),
        masked,
        animated,
        kind,
        anchor: plain.anchor,
        effect,
        hit: if plain.always_transparent || main.missing.is_some() {
            Hit::Through
        } else if plain.transparence_check {
            Hit::Alpha
        } else {
            Hit::Frame
        },
        missing: main.missing.clone(),
        refusals,
    }
}

/// The GUI resources one content stack draws with.
pub struct GuiResources {
    registry: Registry,
    /// The game's language as the font families name it (`l_english`).
    language: Vec<u8>,
    /// Each texture by the stack it was read from and its path.
    textures: BTreeMap<(Stack, Vec<u8>), Loaded>,
    /// Every texture [`GuiResources::texture`] returned since the sprite or font being prepared
    /// began, loaded then or before, whose refusals that sprite or font takes.
    touched: Vec<(Stack, Vec<u8>)>,
    /// The client state every texture decodes under ([`Decode`]).
    decode: Decode,
    sprites: HashMap<Vec<u8>, Prepared>,
    /// Shared CPU masks, built only for textures used by transparency-checked sprites.
    alpha: BTreeMap<usize, AlphaMask>,
    /// Each loaded font, by what chose it ([`FontKey`]).
    fonts: BTreeMap<FontKey, native_text::Font>,
    /// The loaded font each asked name resolves to.
    font_names: BTreeMap<Vec<u8>, FontKey>,
    /// Each effect as loaded, with what its load drew from the recording instead of the visual
    /// mods, credited again on every use ([`GuiResources::effect_result`]).
    effects: [BTreeMap<Vec<u8>, (Result<Effect, String>, BTreeSet<String>)>; 2],
    compiler: Option<super::native_shader::library::Library>,
    /// The renderer's global defines every effect compiles with (`library::globals`).
    globals: Vec<String>,
    /// Every font or texture path read that the content coverage policy does not verify.
    uncovered: BTreeSet<Vec<u8>>,
    /// The recorded layers below the visual mods of the stack a texture failed on, taken once,
    /// which a texture the mods break is read from.
    recorded: Option<ResolvedLayers>,
    /// Each `.fnt` a loaded font names that no stack gives, and why: its glyphs draw nothing.
    missing_fonts: BTreeMap<String, String>,
    /// The recording's own registry, when a sprite or font the visual mods cannot draw is drawn
    /// as a whole from the recording's own declaration of it
    /// ([`GuiResources::with_recorded_registry`]); `None` retries each file by its path.
    recorded_registry: Option<Registry>,
}

/// Resources and viewport for one presentation; None refreshes without advancing a frame.
pub struct Context<'a, R> {
    pub resources: &'a R,
    pub windows: &'a NativeWindows,
    pub viewport: [i32; 2],
    pub time: Option<f32>,
}

impl GuiResources {
    /// Over `registry` in game load order, for `language` and `map_size`, under `decode`
    /// settings. GUI effects receive renderer globals whose power-of-two scales follow
    /// the same `force_pow2_textures` setting as texture decoding.
    pub fn new(registry: Registry, language: &[u8], map_size: [u32; 2], decode: Decode) -> Self {
        Self {
            registry,
            language: language.to_vec(),
            textures: BTreeMap::new(),
            touched: Vec::new(),
            decode,
            sprites: HashMap::new(),
            alpha: BTreeMap::new(),
            fonts: BTreeMap::new(),
            font_names: BTreeMap::new(),
            effects: std::array::from_fn(|_| BTreeMap::new()),
            compiler: None,
            globals: super::native_shader::library::globals(map_size, decode),
            uncovered: BTreeSet::new(),
            recorded: None,
            missing_fonts: BTreeMap::new(),
            recorded_registry: None,
        }
    }

    /// These resources drawing a sprite or font the visual mods cannot draw (a texture, page or
    /// font file of theirs that does not load, or one they hide) from `recorded`, the recording's
    /// own registry: its declaration of that name, read from the recording, named, rather than
    /// the mods' declaration with the recording's file of the same path. One the recording does
    /// not declare, or cannot draw either, stays as the mods' draw it. Texture files named by
    /// path alone ([`Assets::textures`]) are still read from the recording by their path. Without
    /// it, as the map draws, every file is retried by its path.
    pub fn with_recorded_registry(mut self, recorded: Registry) -> Self {
        self.recorded_registry = Some(recorded);
        self
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Deferred plain textures keep the eager sprite's metadata and effect.
    pub fn load_lazy_plain(&mut self, layers: &ResolvedLayers, name: &[u8]) -> Result<(), String> {
        let Some(facts) = self.registry.sprite(name).cloned() else {
            return Err(format!("undeclared lazy sprite {}", lossy(name)));
        };
        let Declared::Plain(plain) = &facts.kind else {
            return Err(format!("lazy sprite {} is not plain", lossy(name)));
        };
        if !plain.masking_texture.is_empty()
            || (!plain.legacy_lazy_load && !plain.animations.is_empty())
            || !facts.refusals.is_empty()
        {
            return Err(format!(
                "lazy sprite {} requires eager resources",
                lossy(name)
            ));
        }
        let main = Loaded {
            texture: NO_TEXTURE,
            size: [0; 2],
            missing: None,
            refused: None,
        };
        let effect = self.effect_result(layers, plain.effect(), false);
        let info = plain_info(
            plain,
            &main,
            false,
            false,
            SpriteKind::Plain,
            effect,
            Vec::new(),
        );
        self.sprites.insert(
            name.to_vec(),
            Prepared {
                info,
                sheet: SpriteSheet {
                    name: lossy(name),
                    texture: NO_TEXTURE,
                    frames: plain.frames.max(1) as u32,
                    size: [0; 2],
                },
                second: None,
                masking: None,
                animations: Vec::new(),
            },
        );
        Ok(())
    }

    /// Loads every sprite and font `assets` names, the fallback sprite and the effects the
    /// game's code names itself, appending each texture that loads to `images` once. `Err` only
    /// for a read that fails other than by a file's absence.
    pub fn load(
        &mut self,
        layers: &ResolvedLayers,
        assets: &Assets,
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        for effect in [TEXT_EFFECT, SHIELD_OVERLAY_EFFECT, DEFAULT_SPRITE_EFFECT] {
            // Refused effects are kept and reported where they are drawn.
            let _ = self.effect_result(layers, effect.as_bytes(), false);
        }
        for name in assets
            .sprites
            .iter()
            .map(Vec::as_slice)
            .chain([FALLBACK_SPRITE])
        {
            if !self.sprites.contains_key(name) {
                self.prepare(layers, name, images)?;
            }
        }
        for name in &assets.fonts {
            self.load_font(layers, name, images)?;
        }
        for path in &assets.textures {
            self.texture(Stack::Retried, layers, path, images)?;
        }
        Ok(())
    }

    /// The effect `file` resolves to (read once), or why it is refused. What its load drew from
    /// the recording is credited to every use, so a sprite kept after another attempt loaded the
    /// effect still names it.
    fn effect_result(
        &mut self,
        layers: &ResolvedLayers,
        file: &[u8],
        masked: bool,
    ) -> Result<Effect, String> {
        let globals = &self.globals;
        let library = self.compiler.get_or_insert_with(|| {
            super::native_shader::library::Library::new(Arc::new(layers.clone()))
                .with_globals(globals.clone())
        });
        let (effect, fallbacks) = self.effects[usize::from(masked)]
            .entry(file.to_vec())
            .or_insert_with(|| {
                // The name to its C string's end, opened by its bytes.
                let file = installed_content_core::parser::c_string(file);
                installed_content_core::content::observe_overlay_refusals(|| {
                    Effect::load(
                        library,
                        file,
                        &if masked {
                            vec!["MASKING".into()]
                        } else {
                            Vec::new()
                        },
                    )
                })
            });
        installed_content_core::content::credit_overlay_refusals(fallbacks);
        effect.clone()
    }

    /// The texture `name` loads from `stack`, once. Every texture returned, loaded now or before,
    /// is noted in `touched`: the one path a sprite or font takes its textures' refusals by.
    fn texture(
        &mut self,
        stack: Stack,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<Loaded> {
        let key = (stack, name.to_vec());
        self.touched.push(key.clone());
        if let Some(loaded) = self.textures.get(&key) {
            return Ok(loaded.clone());
        }
        // A texture the visual mods break (or hide) is the recording's own, named, when it is
        // retried by its path; one the recording cannot give either fails as it would without
        // them.
        let recorded = match stack {
            Stack::Retried => self.recorded_layers(layers),
            Stack::Shown | Stack::Recorded => None,
        };
        let loaded =
            match resources::stack_texture_or_recorded(layers, recorded.as_ref(), name, self.decode)? {
                Ok(texture) => {
                    if !layers.covered_path(name) {
                        self.uncovered.insert(name.to_vec());
                    }
                    images.push(texture.image);
                    Loaded {
                        texture: images.len() - 1,
                        size: [texture.width, texture.height],
                        missing: None,
                        refused: None,
                    }
                }
                Err(why) => Loaded {
                    texture: NO_TEXTURE,
                    size: [0; 2],
                    missing: Some(why.to_string()),
                    refused: match &why {
                        _ if why.native_failure() => None,
                        NoTexture::Device { .. } | NoTexture::OverBudget { .. } => {
                            Some((RefusalKind::MissingInput, why.to_string()))
                        }
                        _ => Some((RefusalKind::UndefinedRead, why.to_string())),
                    },
                },
            };
        self.textures.insert(key, loaded.clone());
        Ok(loaded)
    }

    /// The recorded layers below the visual mods `layers` shows, taken once; `None` when it
    /// shows none.
    fn recorded_layers(&mut self, layers: &ResolvedLayers) -> Option<ResolvedLayers> {
        if !layers.shows_visual_mods() {
            return None;
        }
        if self.recorded.is_none() {
            self.recorded = layers.without_overlays();
        }
        self.recorded.clone()
    }

    /// Why the sprite or font being prepared cannot be drawn as declared: the first texture it
    /// asked for (by a path) that does not load.
    fn touched_failure(&self) -> Option<String> {
        self.touched.iter().find_map(|key| {
            let why = self.textures[key].missing.as_ref()?;
            (!key.1.is_empty()).then(|| format!("texture {} ({why})", lossy(&key.1)))
        })
    }

    /// One note naming every font or texture read outside the content coverage policy, whose
    /// bytes are not hash-verified; `None` when every read was covered.
    pub fn uncovered_note(&self, owner: &str) -> Option<String> {
        (!self.uncovered.is_empty()).then(|| {
            let paths: Vec<_> = self
                .uncovered
                .iter()
                .map(|path| String::from_utf8_lossy(path))
                .collect();
            format!(
                "{owner}: {} font/texture file(s) are outside the content coverage policy and are not hash-verified: {}.",
                paths.len(),
                paths.join(", ")
            )
        })
    }

    /// The refusals of every texture returned since `touched` was last taken, each once.
    fn touched_refusals(&mut self) -> Vec<(RefusalKind, String)> {
        let mut refusals = Vec::new();
        for texture in std::mem::take(&mut self.touched) {
            if let Some(refused) = self.textures[&texture].refused.clone()
                && !refusals.contains(&refused)
            {
                refusals.push(refused);
            }
        }
        refusals
    }

    /// Prepares the sprite `name` shows ([`Registry::create`]: the name, else the fallback). With
    /// the recording's own registry ([`Self::with_recorded_registry`]), a sprite the visual mods
    /// cannot draw as they declare it is the recording's own declaration of it, drawn from the
    /// recording and named, when that draws.
    fn prepare(
        &mut self,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        if self.recorded_registry.is_none() {
            if let Some((prepared, _)) = self.prepare_from(Stack::Retried, layers, name, images)? {
                self.sprites.insert(name.to_vec(), prepared);
            }
            return Ok(());
        }
        // Each attempt keeps what it notes to itself; only the kept one's is published. A read
        // that fails is the attempt's failure too, which the recording's declaration may draw.
        let (shown, noted) = installed_content_core::content::observe_overlay_refusals_isolated(|| {
            self.prepare_from(Stack::Shown, layers, name, images)
        });
        let why = match &shown {
            Ok(Some((_, broken))) => broken.clone(),
            Ok(None) => None,
            Err(error) => Some(error.to_string()),
        };
        if let Some(why) = why
            && let Some(recorded) = self.recorded_layers(layers)
        {
            let (own, own_noted) = installed_content_core::content::observe_overlay_refusals_isolated(|| {
                self.prepare_from(Stack::Recorded, &recorded, name, images)
            });
            // A recording that cannot draw it either leaves the selection's outcome as it is.
            if let Ok(Some((own, None))) = own {
                installed_content_core::content::credit_overlay_refusals(&own_noted);
                layers.note_overlay_refusal(format!("sprite {} ({why})", lossy(name)));
                self.sprites.insert(name.to_vec(), own);
                return Ok(());
            }
        }
        installed_content_core::content::credit_overlay_refusals(&noted);
        if let Some((prepared, _)) = shown? {
            self.sprites.insert(name.to_vec(), prepared);
        }
        Ok(())
    }

    /// The sprite `name` as `stack`'s registry declares it (the recording's own for
    /// [`Stack::Recorded`]), its textures read from `layers`, with why it cannot be drawn as
    /// declared; `None` when the registry does not declare it.
    fn prepare_from(
        &mut self,
        stack: Stack,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<Option<(Prepared, Option<String>)>> {
        self.touched.clear();
        let registry = match stack {
            Stack::Recorded => self
                .recorded_registry
                .as_ref()
                .expect("a recorded draw has the recording's registry"),
            Stack::Retried | Stack::Shown => &self.registry,
        };
        let facts: SpriteFacts = match registry.create(name) {
            Lookup::Defined(facts) => facts.clone(),
            // The module retries the fallback itself (`FALLBACK_SPRITE` is prepared by `load`).
            Lookup::Fallback(_) | Lookup::Nothing => return Ok(None),
        };
        // The reader's refusals of the declaration, which the layout refuses a use by; the
        // refusals of the textures it loads join them below.
        let refusals: Vec<(RefusalKind, String)> = facts
            .refusals
            .iter()
            .map(|refusal| (RefusalKind::Reader, refusal.to_string()))
            .collect();
        // The legacy sheet's `name` is a display label; the native path keys every sheet by the
        // sprite's bytes ([`Sheets`]).
        let label = lossy(name);
        let sheet = |loaded: &Loaded, frames: i32| SpriteSheet {
            name: label.clone(),
            texture: loaded.texture,
            frames: frames.max(1) as u32,
            size: loaded.size,
        };
        let extent = |loaded: &Loaded| loaded.size.map(|value| value as i32);
        // A chart: no texture, and its effect the code's own (`gfx/FX/color.lua`,
        // `gfx/FX/linechart.lua`), which it draws nothing with until the code sets its data.
        let chart = |kind: SpriteKind| SpriteInfo {
            texture: [0; 2],
            frames: 1,
            vertical: false,
            masked: false,
            animated: false,
            kind,
            anchor: [0.0; 2],
            effect: Ok(Effect {
                compiled: None,
                file: String::new(),
                techniques: Vec::new(),
                samplers: [None; 6],
            }),
            hit: Hit::Frame,
            missing: None,
            refusals: refusals.clone(),
        };
        let prepared = match &facts.kind {
            Declared::Plain(plain)
            | Declared::FrameAnimated { plain, .. }
            | Declared::Tile { plain, .. }
            | Declared::MaskedShield { plain, .. } => {
                let (main, second) = match &facts.kind {
                    Declared::MaskedShield { shield, .. } => (
                        self.texture(stack, layers, &shield.texture1, images)?,
                        Some(self.texture(stack, layers, &shield.texture2, images)?),
                    ),
                    _ => (self.texture(stack, layers, &plain.texture, images)?, None),
                };
                if plain.transparence_check && main.missing.is_none() {
                    // The game creates a mask only for decoded BGRA8 surfaces. Other
                    // native formats keep the no-mask rectangular-hit fallback.
                    if let Some(image) = images.get(main.texture)
                        && image.format == dds::TextureFormat::Rgba8
                    {
                        self.alpha.entry(main.texture).or_insert_with(|| AlphaMask {
                            width: image.width as i32,
                            transparent: image
                                .base()
                                .data
                                .chunks_exact(4)
                                .map(|texel| u8::from(texel[3] == 0))
                                .collect(),
                        });
                    }
                }
                let masking = if plain.masking_texture.is_empty() {
                    None
                } else {
                    Some(self.texture(stack, layers, &plain.masking_texture, images)?)
                        .filter(|loaded| loaded.missing.is_none())
                };
                let masked = masking.is_some();
                let effect = match &facts.kind {
                    Declared::MaskedShield { shield, .. } => {
                        self.effect_result(layers, &shield.effect_file, false)
                    }
                    _ => self.effect_result(layers, plain.effect(), masked),
                };
                // Eager plain sprites and tiles draw animation records only when their
                // animated variant declares Up and at least five sampler slots. Otherwise
                // they draw plain. Frame-animated sprites and masked shields never draw
                // these records. Missing record textures stay null.
                let animates = matches!(facts.kind, Declared::Plain(_) | Declared::Tile { .. })
                    && !plain.legacy_lazy_load
                    && !plain.animations.is_empty()
                    && self
                        .effect_result(layers, plain.effect(), false)
                        .is_ok_and(|effect| {
                            effect.compiled.as_ref().is_some_and(|asset| {
                                asset.source.technique("Up").is_some()
                                    && asset
                                        .source
                                        .common
                                        .samplers
                                        .iter()
                                        .chain(&asset.source.pixel.samplers)
                                        .any(|sampler| sampler.index >= 4)
                            })
                        });
                let mut animations = Vec::new();
                if animates {
                    for record in &plain.animations {
                        let mask = self.texture(stack, layers, &record.mask_file, images)?;
                        let texture = self.texture(stack, layers, &record.texture_file, images)?;
                        animations.push(AnimationRecord {
                            data: record.clone(),
                            mask: mask.texture,
                            texture: texture.texture,
                            texture_size: extent(&texture),
                        });
                    }
                }
                let kind = match &facts.kind {
                    Declared::FrameAnimated { animation, .. } => SpriteKind::FrameAnimated {
                        seconds_per_frame: animation.seconds_per_frame,
                        looping: animation.looping,
                        play_on_show: animation.play_on_show,
                        pause: animation.pause_on_loop,
                    },
                    Declared::Tile { size, .. } => SpriteKind::Tile { size: *size },
                    Declared::MaskedShield { shield, .. } => SpriteKind::MaskedShield {
                        mask: second.as_ref().map_or([0; 2], extent),
                        small: shield.small,
                    },
                    _ => SpriteKind::Plain,
                };
                Prepared {
                    info: plain_info(
                        plain,
                        &main,
                        masked,
                        animates,
                        kind,
                        effect,
                        refusals.clone(),
                    ),
                    sheet: sheet(&main, plain.frames),
                    second: second.as_ref().map(|loaded| sheet(loaded, 1)),
                    masking: masking.as_ref().map(|loaded| sheet(loaded, 1)),
                    animations,
                }
            }
            Declared::CorneredTile(tile) => {
                let main = self.texture(stack, layers, &tile.texture, images)?;
                let effect = self.effect_result(layers, tile.effect(), false);
                Prepared {
                    info: SpriteInfo {
                        texture: extent(&main),
                        frames: tile.frames as i32,
                        vertical: false,
                        masked: false,
                        animated: false,
                        kind: SpriteKind::CorneredTile {
                            size: tile.size,
                            border: tile.border,
                            tiling_center: tile.tiling_center,
                            looping: tile.looping,
                            rate: tile.animation_rate,
                        },
                        anchor: [0.0; 2],
                        effect,
                        hit: if tile.always_transparent {
                            Hit::Through
                        } else {
                            Hit::Frame
                        },
                        // Missing tile artwork draws with the null texture.
                        missing: None,
                        refusals: refusals.clone(),
                    },
                    sheet: sheet(&main, tile.frames as i32),
                    second: None,
                    masking: None,
                    animations: Vec::new(),
                }
            }
            Declared::Progress(progress) => {
                let one = self.texture(stack, layers, &progress.texture1, images)?;
                let two = self.texture(stack, layers, &progress.texture2, images)?;
                let effect = self.effect_result(layers, &progress.effect_file, false);
                Prepared {
                    info: SpriteInfo {
                        texture: extent(&one),
                        frames: 1,
                        vertical: false,
                        masked: false,
                        animated: false,
                        kind: SpriteKind::Progress {
                            size: progress.size,
                            steps: progress.steps,
                            colors: [progress.color, progress.color_two],
                            horizontal: progress.horizontal,
                            textured: !progress.texture1.is_empty()
                                && !progress.texture2.is_empty(),
                        },
                        anchor: [0.0; 2],
                        effect,
                        hit: if progress.always_transparent {
                            Hit::Through
                        } else {
                            Hit::Frame
                        },
                        missing: None,
                        refusals: refusals.clone(),
                    },
                    sheet: sheet(&one, 1),
                    second: Some(sheet(&two, 1)),
                    masking: None,
                    animations: Vec::new(),
                }
            }
            Declared::Resizeable(resizeable) => {
                let main = self.texture(stack, layers, &resizeable.texture, images)?;
                Prepared {
                    info: SpriteInfo {
                        texture: extent(&main),
                        frames: resizeable.frames as i32,
                        vertical: false,
                        masked: false,
                        animated: false,
                        kind: SpriteKind::Resizeable,
                        anchor: [0.0; 2],
                        effect: Ok(Effect {
                            file: String::new(),
                            techniques: Vec::new(),
                            samplers: [None; 6],
                            compiled: None,
                        }),
                        hit: Hit::Frame,
                        missing: main.missing.clone(),
                        refusals: refusals.clone(),
                    },
                    sheet: sheet(&main, resizeable.frames as i32),
                    second: None,
                    masking: None,
                    animations: Vec::new(),
                }
            }
            Declared::CircularProgress(circular) => {
                let one = self.texture(stack, layers, &circular.texture1, images)?;
                let two = self.texture(stack, layers, &circular.texture2, images)?;
                let effect = self.effect_result(layers, CIRCULAR_EFFECT.as_bytes(), false);
                Prepared {
                    info: SpriteInfo {
                        effect,
                        ..chart(SpriteKind::CircularProgress {
                            size: circular.size,
                            rotation: circular.rotation,
                            amount: circular.amount,
                        })
                    },
                    sheet: sheet(&one, 1),
                    second: Some(sheet(&two, 1)),
                    masking: None,
                    animations: Vec::new(),
                }
            }
            Declared::PieChart(pie) => Prepared {
                info: chart(SpriteKind::PieChart { size: pie.size }),
                sheet: sheet(&self.texture(stack, layers, b"", images)?, 1),
                second: None,
                masking: None,
                animations: Vec::new(),
            },
            Declared::LineChart(line) => Prepared {
                info: chart(SpriteKind::LineChart { size: line.size }),
                sheet: sheet(&self.texture(stack, layers, b"", images)?, 1),
                second: None,
                masking: None,
                animations: Vec::new(),
            },
            Declared::Text(text) => {
                let main = self.texture(stack, layers, &text.texture, images)?;
                let file: &[u8] = if text.effect_file.is_empty() {
                    DEFAULT_SPRITE_EFFECT.as_bytes()
                } else {
                    &text.effect_file
                };
                let effect = self.effect_result(layers, file, false);
                Prepared {
                    info: SpriteInfo {
                        texture: extent(&main),
                        // `noOfFrames` (`%u`) as the draw's `IDIV` reads it.
                        frames: text.frames as i32,
                        vertical: false,
                        masked: false,
                        animated: false,
                        kind: SpriteKind::Text {
                            textured: !text.texture.is_empty(),
                        },
                        anchor: [0.0; 2],
                        effect,
                        hit: Hit::Frame,
                        missing: main.missing.clone(),
                        refusals: refusals.clone(),
                    },
                    sheet: sheet(&main, text.frames as i32),
                    second: None,
                    masking: None,
                    animations: Vec::new(),
                }
            }
        };
        let mut prepared = prepared;
        let broken = self.touched_failure();
        for refused in self.touched_refusals() {
            if !prepared.info.refusals.contains(&refused) {
                prepared.info.refusals.push(refused);
            }
        }
        Ok(Some((prepared, broken)))
    }

    /// Loads the font resolved for `name`, including its fallback. With the recording's own
    /// registry ([`Self::with_recorded_registry`]), a font the visual mods cannot draw as they
    /// declare it (a font file or page of theirs that does not load) is the recording's own
    /// declaration of it, read from the recording and named, when that draws.
    fn load_font(
        &mut self,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        let Some(found) = self.registry.fonts.get_or_fallback(name) else {
            return Ok(());
        };
        let index = self
            .registry
            .fonts
            .fonts()
            .iter()
            .position(|font| std::ptr::eq(font, found))
            .expect("a registered font");
        let key = if self.recorded_registry.is_some() {
            FontKey::Named(name.to_vec())
        } else {
            FontKey::Registered(index)
        };
        self.font_names.insert(name.to_vec(), key.clone());
        if self.fonts.contains_key(&key) {
            return Ok(());
        }
        let declared = (found.family(&self.language).clone(), found.facts.clone());
        if self.recorded_registry.is_none() {
            let (font, notes, _) = self.font_from(Stack::Retried, layers, declared, images)?;
            self.keep_font(layers, key, font, notes);
            return Ok(());
        }
        // Each attempt keeps what it notes to itself; only the kept one's is published. A read
        // that fails is the attempt's failure too.
        let (shown, noted) = installed_content_core::content::observe_overlay_refusals_isolated(|| {
            self.font_from(Stack::Shown, layers, declared, images)
        });
        let why = match &shown {
            Ok((_, _, broken)) => broken.clone(),
            Err(error) => Some(error.to_string()),
        };
        // Only the recording's own declaration of the name stands in: a font the recording does
        // not declare is not replaced by another.
        if let Some(why) = why
            && let Some(own) = self
                .recorded_registry
                .as_ref()
                .and_then(|registry| registry.fonts.get(name))
                .map(|own| (own.family(&self.language).clone(), own.facts.clone()))
            && let Some(recorded) = self.recorded_layers(layers)
        {
            let (own, own_noted) = installed_content_core::content::observe_overlay_refusals_isolated(|| {
                self.font_from(Stack::Recorded, &recorded, own, images)
            });
            // A recording that cannot draw it either leaves the selection's outcome as it is.
            if let Ok((own, own_notes, None)) = own {
                installed_content_core::content::credit_overlay_refusals(&own_noted);
                layers.note_overlay_refusal(format!("font {} ({why})", lossy(name)));
                self.keep_font(layers, key, own, own_notes);
                return Ok(());
            }
        }
        installed_content_core::content::credit_overlay_refusals(&noted);
        let (font, notes, _) = shown?;
        self.keep_font(layers, key, font, notes);
        Ok(())
    }

    /// Keeps `font` under `key`, applying what its load noted.
    fn keep_font(
        &mut self,
        layers: &ResolvedLayers,
        key: FontKey,
        font: native_text::Font,
        notes: FontNotes,
    ) {
        for refused in notes.refused {
            layers.note_refused(refused);
        }
        self.missing_fonts.extend(notes.missing);
        self.uncovered.extend(notes.uncovered);
        self.fonts.insert(key, font);
    }

    /// The font `declared` (its language's family and its facts) as read from `stack` (`layers`),
    /// with what its load noted and why it cannot be drawn as declared.
    fn font_from(
        &mut self,
        stack: Stack,
        layers: &ResolvedLayers,
        (family, facts): (fonts::FontFamily, fonts::FontFacts),
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<(native_text::Font, FontNotes, Option<String>)> {
        // A `.fnt` the visual mods break or hide is the recording's own, named, when it is
        // retried by its path. One no stack gives is named for the map's fallback and reported;
        // one that fails to read, rather than being absent, also refuses the font's texts.
        let recorded = match stack {
            Stack::Retried => self.recorded_layers(layers),
            Stack::Shown | Stack::Recorded => None,
        };
        let unreadable = std::cell::RefCell::new(Vec::new());
        let glyphs = fonts::load_glyphs(&family, &|path| match layers.read_uncovered_path(path) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                if let Some(bytes) = recorded
                    .as_ref()
                    .and_then(|recorded| recorded.read_uncovered_path(path).ok())
                {
                    layers.note_overlay_refusal(format!("font file {} ({error})", lossy(path)));
                    return Some(bytes);
                }
                if error.kind() != io::ErrorKind::NotFound {
                    unreadable
                        .borrow_mut()
                        .push(format!("{}: {error}", lossy(path)));
                }
                None
            }
        });
        let unreadable = unreadable.into_inner();
        let mut notes = FontNotes::default();
        let mut broken = glyphs
            .missing
            .first()
            .map(|file| format!("font file {file} (no stack gives it)"))
            .or_else(|| unreadable.first().map(|why| format!("font file {why}")));
        for file in &glyphs.missing {
            notes.refused.push(format!("font file {file}"));
            notes.missing.push((
                file.clone(),
                "no font file of that path reads; its glyphs draw nothing".into(),
            ));
        }
        notes.uncovered.extend(
            family
                .files
                .iter()
                .map(|file| [file.as_slice(), b".fnt"].concat())
                .filter(|file| {
                    !layers.covered_path(file)
                        && !glyphs
                            .missing
                            .contains(&String::from_utf8_lossy(file).into_owned())
                }),
        );
        // The reader's refusals of the declaration, and those of every page texture returned.
        let mut refusals: Vec<(RefusalKind, String)> = facts
            .refusals
            .iter()
            .map(|refusal| (RefusalKind::Reader, refusal.to_string()))
            .chain(
                unreadable
                    .into_iter()
                    .map(|why| (RefusalKind::MissingResource, why)),
            )
            .collect();
        let mut pages = Vec::with_capacity(glyphs.pages.len());
        self.touched.clear();
        for page in &glyphs.pages {
            // A page the visual mods break is the recording's when retried by its path
            // ([`Self::texture`]); one no stack supplies leaves its glyphs blank, named for the
            // map's fallback.
            let loaded = self.texture(stack, layers, page, images)?;
            if loaded.missing.is_some() {
                notes.refused.push(format!("font page {}", lossy(page)));
            }
            pages.push(loaded.missing.is_none().then_some(Page {
                texture: loaded.texture,
                size: loaded.size.map(|value| value as i32),
            }));
        }
        broken = broken.or_else(|| self.touched_failure());
        for refused in self.touched_refusals() {
            if !refusals.contains(&refused) {
                refusals.push(refused);
            }
        }
        Ok((
            native_text::Font {
                facts,
                glyphs,
                pages,
                refusals,
            },
            notes,
            broken,
        ))
    }

    /// The sprite the content defines under `name`, as loaded.
    pub fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.sprites.get(name).map(|prepared| prepared.info.clone())
    }

    /// Immutable loaded metadata shared by every drawable of this sprite.
    pub fn sprite_info(&self, name: &[u8]) -> Option<&SpriteInfo> {
        self.sprites.get(name).map(|prepared| &prepared.info)
    }

    /// The texture's mip-0 CPU mask shared by all drawables using it, if it has one.
    pub fn sprite_alpha(&self, name: &[u8]) -> Option<AlphaMask> {
        self.alpha
            .get(&self.sprites.get(name)?.sheet.texture)
            .cloned()
    }

    /// The animation records the sprite `name` draws ([`SpriteInfo::animated`]).
    pub fn sprite_animations(&self, name: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.sprites
            .get(name)
            .map(|prepared| prepared.animations.clone())
    }

    /// A texture file [`GuiResources::load`] loaded ([`Assets::textures`], or a sprite's): its
    /// whole rectangle, or why it does not load; `None` when it was not asked for.
    pub fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        let loaded = [Stack::Retried, Stack::Shown]
            .into_iter()
            .find_map(|stack| self.textures.get(&(stack, path.to_vec())))?;
        Some(match (&loaded.refused, &loaded.missing) {
            (Some((kind, why)), _) => TextureFile::Refused(*kind, why.clone()),
            (None, Some(why)) => TextureFile::Failed(why.clone()),
            (None, None) => TextureFile::Loaded(Texture {
                texture: loaded.texture,
                uv: [0.0, 0.0, 1.0, 1.0],
                size: loaded.size.map(|value| value as i32),
            }),
        })
    }

    /// The loaded font resolved for `name`, including its fallback.
    pub fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        self.fonts.get(self.font_names.get(name)?)
    }

    /// The global `textcolors` table.
    pub fn text_colors(&self) -> &TextColors {
        &self.registry.text_colors
    }

    /// An effect the game's code names itself, loaded by [`GuiResources::load`]
    /// ([`TEXT_EFFECT`], [`SHIELD_OVERLAY_EFFECT`]).
    pub fn effect(&self, file: &str) -> Result<Effect, String> {
        self.effects[0]
            .get(file.as_bytes())
            .map(|(effect, _)| effect.clone())
            .unwrap_or_else(|| Err(format!("effect {file} was not loaded")))
    }

    /// The bitmap fonts' effect.
    pub fn text_effect(&self) -> Result<Effect, String> {
        self.effect(TEXT_EFFECT)
    }

    /// Every texture that did not load, by name, for startup diagnostics.
    pub fn missing_textures(&self) -> Vec<(String, String)> {
        self.textures
            .iter()
            .filter_map(|((_, name), loaded)| Some((lossy(name), loaded.missing.clone()?)))
            .chain(
                self.missing_fonts
                    .iter()
                    .map(|(file, why)| (file.clone(), why.clone())),
            )
            .collect()
    }
}

impl Sheets for GuiResources {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.sprites.get(sprite).map(|prepared| &prepared.sheet)
    }

    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]> {
        let prepared = self.sprites.get(sprite)?;
        Some([&prepared.sheet, prepared.second.as_ref()?])
    }

    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.sprites.get(sprite)?.masking.as_ref()
    }

    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.sprites.get(sprite)?.second.as_ref()
    }
}

#[cfg(test)]
mod tests;
