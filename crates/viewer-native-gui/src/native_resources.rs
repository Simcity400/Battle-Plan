
use crate::native_windows::NativeWindows;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

use content_reader_layers::content::diagnostics::{self, Diagnostics};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::gui::native::{fonts::{self, TextColors}, resources::{self, Lookup, NoTexture, Registry}, sprites::{Plain, SpriteFacts, SpriteKind as Declared}};
use content_reader_image::{dds, native::Decode};

use super::{
    native_effects::Effect,
    native_gui::{
        AlphaMask, AnimationRecord, Assets, FALLBACK_SPRITE, Hit, RefusalKind, Sheets, SpriteInfo,
        SpriteKind, Texture, TextureFile, lossy,
    },
    native_text::{self, Page},
    sprites::{NO_TEXTURE, SpriteSheet},
};

pub const TEXT_EFFECT: &str = "gfx/FX/text.lua";

pub const CIRCULAR_EFFECT: &str = "gfx/FX/circularprogressbar.shader";

pub use content_reader_gui::gui::native::resources::SHIELD_OVERLAY_EFFECT;

pub const DEFAULT_SPRITE_EFFECT: &str = "gfx/FX/buttonstate.lua";

#[derive(Clone, Debug, PartialEq)]
struct Loaded {
    texture: usize,
    size: [u32; 2],
    missing: Option<String>,
    refused: Option<(RefusalKind, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stack {
    Retried,
    Shown,
    Recorded,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FontKey {
    Registered(usize),
    Named(Vec<u8>),
}

#[derive(Default)]
struct FontNotes {
    refused: Vec<String>,
    missing: Vec<(String, String)>,
    uncovered: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct Prepared {
    info: SpriteInfo,
    sheet: SpriteSheet,
    second: Option<SpriteSheet>,
    masking: Option<SpriteSheet>,
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

mod window_art;
use window_art::WindowArt;
use crate::lazy_icons::LazyTextures;

pub struct GuiResources {
    window: Option<Arc<WindowArt>>,
    uploads: Option<Arc<LazyTextures>>,
    registry: Arc<Registry>,
    language: Vec<u8>,
    textures: BTreeMap<(Stack, Vec<u8>), Loaded>,
    texture_diagnostics: BTreeMap<(Stack, Vec<u8>), Diagnostics>,
    touched: Vec<(Stack, Vec<u8>)>,
    decode: Decode,
    sprites: HashMap<Vec<u8>, Prepared>,
    sprite_diagnostics: HashMap<Vec<u8>, Diagnostics>,
    alpha: BTreeMap<usize, AlphaMask>,
    fonts: BTreeMap<FontKey, native_text::Font>,
    font_diagnostics: BTreeMap<FontKey, Diagnostics>,
    font_names: BTreeMap<Vec<u8>, FontKey>,
    effects: [BTreeMap<Vec<u8>, (Result<Effect, String>, Diagnostics)>; 2],
    compiler: Option<shader_pipeline::library::Library>,
    globals: Vec<String>,
    uncovered: BTreeSet<Vec<u8>>,
    recorded: Option<ResolvedLayers>,
    missing_fonts: BTreeMap<String, String>,
    recorded_registry: Option<Arc<Registry>>,
}

pub struct Context<'a, R> {
    pub resources: &'a R,
    pub windows: &'a NativeWindows,
    pub viewport: [i32; 2],
    pub time: Option<f32>,
}

impl GuiResources {
    pub fn new(
        registry: impl Into<Arc<Registry>>,
        language: &[u8],
        map_size: [u32; 2],
        decode: Decode,
    ) -> Self {
        Self::unloaded(registry.into(), language, shader_pipeline::library::globals(map_size, decode), decode)
    }

    fn unloaded(registry: Arc<Registry>, language: &[u8], globals: Vec<String>, decode: Decode) -> Self {
        Self {
            window: None,
            uploads: None,
            registry,
            language: language.to_vec(),
            textures: BTreeMap::new(),
            texture_diagnostics: BTreeMap::new(),
            touched: Vec::new(),
            decode,
            sprites: HashMap::new(),
            sprite_diagnostics: HashMap::new(),
            alpha: BTreeMap::new(),
            fonts: BTreeMap::new(),
            font_diagnostics: BTreeMap::new(),
            font_names: BTreeMap::new(),
            effects: std::array::from_fn(|_| BTreeMap::new()),
            compiler: None,
            globals,
            uncovered: BTreeSet::new(),
            recorded: None,
            missing_fonts: BTreeMap::new(),
            recorded_registry: None,
        }
    }

    pub fn with_recorded_registry(mut self, recorded: impl Into<Arc<Registry>>) -> Self {
        self.recorded_registry = Some(recorded.into());
        self
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn defer_window_art(&mut self, layers: &ResolvedLayers, assets: &Assets, textures: &Arc<LazyTextures>) {
        let mut reader = Self::unloaded(Arc::clone(&self.registry), &self.language, self.globals.clone(), self.decode);
        reader.recorded_registry.clone_from(&self.recorded_registry);
        reader.uploads = Some(Arc::clone(textures));
        self.window = Some(WindowArt::new(reader, layers.clone(), assets, textures));
    }

    pub fn with_ready_window_art<T>(&self, mut read: impl FnMut() -> T) -> T {
        match &self.window {
            Some(art) => art.resolve(read),
            None => read(),
        }
    }

    pub fn window_art_misses(&self) -> u64 {
        self.window.as_ref().map_or(0, |art| art.misses())
    }

    pub fn loading_window_art(&self) -> bool {
        self.window.as_ref().is_some_and(|art| art.loading())
    }

    pub fn load_lazy_plain(&mut self, layers: &ResolvedLayers, name: &[u8]) -> Result<(), String> {
        let (result, diagnostics) =
            diagnostics::capture(|| self.load_lazy_plain_inner(layers, name));
        diagnostics.credit();
        if result.is_ok() {
            self.sprite_diagnostics.insert(name.to_vec(), diagnostics);
        }
        result
    }

    fn load_lazy_plain_inner(
        &mut self,
        layers: &ResolvedLayers,
        name: &[u8],
    ) -> Result<(), String> {
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

    pub fn effects(&self) -> Vec<Arc<shader_pipeline::library::Effect>> {
        let loaded = self
            .effects
            .iter()
            .flat_map(BTreeMap::values)
            .filter_map(|(effect, _)| effect.as_ref().ok()?.compiled.clone());
        let animated = self.sprites.values().filter_map(|prepared| {
            if prepared.animations.is_empty() {
                return None;
            }
            let effect = prepared.info.effect.as_ref().ok()?.compiled.as_ref()?;
            let mut defines: Vec<String> = effect
                .caller()
                .iter()
                .filter(|d| {
                    *d != "MASKING" && *d != "ANIMATED" && !d.starts_with("NUM_ANIMATIONS_")
                })
                .cloned()
                .collect();
            defines.extend([
                "ANIMATED".into(),
                format!("NUM_ANIMATIONS_{}", prepared.animations.len()),
            ]);
            Some(effect.variant(defines))
        });
        let mut effects: Vec<_> = loaded.chain(animated).collect();
        effects.sort_by_key(|effect| Arc::as_ptr(effect));
        effects.dedup_by_key(|effect| Arc::as_ptr(effect));
        effects
    }

    pub fn load(
        &mut self,
        layers: &ResolvedLayers,
        assets: &Assets,
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        for effect in [TEXT_EFFECT, SHIELD_OVERLAY_EFFECT, DEFAULT_SPRITE_EFFECT] {
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
            if let Some(diagnostics) = self.sprite_diagnostics.get(name) {
                diagnostics.credit();
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

    fn effect_result(
        &mut self,
        layers: &ResolvedLayers,
        file: &[u8],
        masked: bool,
    ) -> Result<Effect, String> {
        let globals = &self.globals;
        let library = self.compiler.get_or_insert_with(|| {
            shader_pipeline::library::Library::new(Arc::new(layers.clone()))
                .with_globals(globals.clone())
        });
        let (effect, fallbacks) = self.effects[usize::from(masked)]
            .entry(file.to_vec())
            .or_insert_with(|| {
                let file = content_reader_parser::parser::c_string(file);
                diagnostics::observe(|| {
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
        fallbacks.credit();
        effect.clone()
    }

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
            if let Some(diagnostics) = self.texture_diagnostics.get(&key) {
                diagnostics.credit();
            }
            return Ok(loaded.clone());
        }
        let recorded = match stack {
            Stack::Retried => self.recorded_layers(layers),
            Stack::Shown | Stack::Recorded => None,
        };
        let (loaded, diagnostics) = diagnostics::capture(|| {
            resources::stack_texture_or_recorded(layers, recorded.as_ref(), name, self.decode)
        });
        diagnostics.credit();
        let loaded = match loaded? {
            Ok(texture) => {
                if !layers.covered_path(name) {
                    self.uncovered.insert(name.to_vec());
                }
                let id = if let Some(uploads) = &self.uploads {
                    uploads.insert(texture.image)
                } else {
                    images.push(texture.image);
                    images.len() - 1
                };
                Loaded {
                    texture: id,
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
        self.texture_diagnostics.insert(key.clone(), diagnostics);
        self.textures.insert(key, loaded.clone());
        Ok(loaded)
    }

    fn recorded_layers(&mut self, layers: &ResolvedLayers) -> Option<ResolvedLayers> {
        if !layers.shows_visual_mods() {
            return None;
        }
        if self.recorded.is_none() {
            self.recorded = layers.without_overlays();
        }
        self.recorded.clone()
    }

    fn touched_failure(&self) -> Option<String> {
        self.touched.iter().find_map(|key| {
            let why = self.textures[key].missing.as_ref()?;
            (!key.1.is_empty()).then(|| format!("texture {} ({why})", lossy(&key.1)))
        })
    }

    pub fn uncovered_note(&self, owner: &str) -> Option<String> {
        let diagnostics = diagnostics::current();
        let paths: Vec<_> = if diagnostics::is_active() {
            diagnostics
                .uncovered_paths()
                .filter(|path| self.uncovered.contains(*path))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            self.uncovered.iter().collect()
        };
        (!paths.is_empty()).then(|| {
            let paths: Vec<_> = paths.into_iter().map(|path| String::from_utf8_lossy(path)).collect();
            format!(
                "{owner}: {} font/texture file(s) are outside the content coverage policy and are not hash-verified: {}.",
                paths.len(),
                paths.join(", ")
            )
        })
    }

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

    fn prepare(
        &mut self,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        let (result, diagnostics) =
            diagnostics::capture(|| self.prepare_inner(layers, name, images));
        diagnostics.credit();
        if result.is_ok() {
            self.sprite_diagnostics.insert(name.to_vec(), diagnostics);
        }
        result
    }

    fn prepare_inner(
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
        let (shown, noted) =
            diagnostics::capture(|| self.prepare_from(Stack::Shown, layers, name, images));
        let why = match &shown {
            Ok(Some((_, broken))) => broken.clone(),
            Ok(None) => None,
            Err(error) => Some(error.to_string()),
        };
        if let Some(why) = why
            && let Some(recorded) = self.recorded_layers(layers)
        {
            let (own, own_noted) = diagnostics::capture(|| {
                self.prepare_from(Stack::Recorded, &recorded, name, images)
            });
            if let Ok(Some((own, None))) = own {
                own_noted.credit();
                layers.note_overlay_refusal(format!("sprite {} ({why})", lossy(name)));
                self.sprites.insert(name.to_vec(), own);
                return Ok(());
            }
        }
        noted.credit();
        if let Some((prepared, _)) = shown? {
            self.sprites.insert(name.to_vec(), prepared);
        }
        Ok(())
    }

    fn prepare_from(
        &mut self,
        stack: Stack,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<Option<(Prepared, Option<String>)>> {
        self.touched.clear();
        let registry: &Registry = match stack {
            Stack::Recorded => self
                .recorded_registry
                .as_deref()
                .expect("a recorded draw has the recording's registry"),
            Stack::Retried | Stack::Shown => &self.registry,
        };
        let facts: SpriteFacts = match registry.create(name) {
            Lookup::Defined(facts) => facts.clone(),
            Lookup::Fallback(_) | Lookup::Nothing => return Ok(None),
        };
        let refusals: Vec<(RefusalKind, String)> = facts
            .refusals
            .iter()
            .map(|refusal| (RefusalKind::Reader, refusal.to_string()))
            .collect();
        let label = lossy(name);
        let sheet = |loaded: &Loaded, frames: i32| SpriteSheet {
            name: label.clone(),
            texture: loaded.texture,
            frames: frames.max(1) as u32,
            size: loaded.size,
        };
        let extent = |loaded: &Loaded| loaded.size.map(|value| value as i32);
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
                    let uploaded = self.uploads.as_ref().and_then(|uploads| uploads.image(main.texture));
                    if let Some(image) = uploaded.as_deref().or_else(|| images.get(main.texture))
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
                info: {
                    let mut info = chart(SpriteKind::LineChart { size: line.size });
                    info.effect = self.effect_result(layers, &line.effect_file, false);
                    info
                },
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

    fn load_font(
        &mut self,
        layers: &ResolvedLayers,
        name: &[u8],
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<()> {
        let (result, diagnostics) =
            diagnostics::capture(|| self.load_font_inner(layers, name, images));
        diagnostics.credit();
        if result.is_ok()
            && let Some(key) = self.font_names.get(name)
        {
            self.font_diagnostics.insert(key.clone(), diagnostics);
        }
        result
    }

    fn load_font_inner(
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
            if let Some(diagnostics) = self.font_diagnostics.get(&key) {
                diagnostics.credit();
            }
            return Ok(());
        }
        let declared = (found.family(&self.language).clone(), found.facts.clone());
        if self.recorded_registry.is_none() {
            let (font, notes, _) = self.font_from(Stack::Retried, layers, declared, images)?;
            self.keep_font(layers, key, font, notes);
            return Ok(());
        }
        let (shown, noted) =
            diagnostics::capture(|| self.font_from(Stack::Shown, layers, declared, images));
        let why = match &shown {
            Ok((_, _, broken)) => broken.clone(),
            Err(error) => Some(error.to_string()),
        };
        if let Some(why) = why
            && let Some(own) = self
                .recorded_registry
                .as_ref()
                .and_then(|registry| registry.fonts.get(name))
                .map(|own| (own.family(&self.language).clone(), own.facts.clone()))
            && let Some(recorded) = self.recorded_layers(layers)
        {
            let (own, own_noted) =
                diagnostics::capture(|| self.font_from(Stack::Recorded, &recorded, own, images));
            if let Ok((own, own_notes, None)) = own {
                own_noted.credit();
                layers.note_overlay_refusal(format!("font {} ({why})", lossy(name)));
                self.keep_font(layers, key, own, own_notes);
                return Ok(());
            }
        }
        noted.credit();
        let (font, notes, _) = shown?;
        self.keep_font(layers, key, font, notes);
        Ok(())
    }

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

    fn font_from(
        &mut self,
        stack: Stack,
        layers: &ResolvedLayers,
        (family, facts): (fonts::FontFamily, fonts::FontFacts),
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<(native_text::Font, FontNotes, Option<String>)> {
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

    pub fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.sprite_info(name).cloned()
    }

    fn credit_sprite(&self, name: &[u8]) {
        if let Some(diagnostics) = self.sprite_diagnostics.get(name) {
            diagnostics.credit();
        }
    }

    fn prepared(&self, name: &[u8]) -> Option<&Prepared> {
        self.credit_sprite(name);
        self.sprites.get(name).or_else(|| self.window.as_ref()?.sprite(name).map(|(sprite, _)| sprite))
    }

    pub fn sprite_info(&self, name: &[u8]) -> Option<&SpriteInfo> {
        self.prepared(name).map(|prepared| &prepared.info)
    }

    pub fn sprite_alpha(&self, name: &[u8]) -> Option<AlphaMask> {
        self.credit_sprite(name);
        if let Some(sprite) = self.sprites.get(name) {
            return self.alpha.get(&sprite.sheet.texture).cloned();
        }
        self.window.as_ref()?.sprite(name)?.1.cloned()
    }

    pub fn sprite_animations(&self, name: &[u8]) -> Option<Vec<AnimationRecord>> {
        self.prepared(name)
            .map(|prepared| prepared.animations.clone())
    }

    pub fn texture_file(&self, path: &[u8]) -> Option<TextureFile> {
        let loaded = [Stack::Retried, Stack::Shown]
            .into_iter()
            .find_map(|stack| {
                let key = (stack, path.to_vec());
                if let Some(diagnostics) = self.texture_diagnostics.get(&key) {
                    diagnostics.credit();
                }
                self.textures.get(&key)
            });
        let Some(loaded) = loaded else {
            return self.window.as_ref()?.texture(path).cloned();
        };
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

    pub fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        if let Some(key) = self.font_names.get(name)
            && let Some(diagnostics) = self.font_diagnostics.get(key)
        {
            diagnostics.credit();
        }
        self.font_names.get(name).and_then(|key| self.fonts.get(key))
            .or_else(|| self.window.as_ref()?.font(name))
    }

    pub fn text_colors(&self) -> &TextColors {
        &self.registry.text_colors
    }

    pub fn effect(&self, file: &str) -> Result<Effect, String> {
        self.effects[0]
            .get(file.as_bytes())
            .map(|(effect, diagnostics)| {
                diagnostics.credit();
                effect.clone()
            })
            .unwrap_or_else(|| Err(format!("effect {file} was not loaded")))
    }

    pub fn text_effect(&self) -> Result<Effect, String> {
        self.effect(TEXT_EFFECT)
    }

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
        self.prepared(sprite).map(|prepared| &prepared.sheet)
    }

    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]> {
        let prepared = self.prepared(sprite)?;
        Some([&prepared.sheet, prepared.second.as_ref()?])
    }

    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.prepared(sprite)?.masking.as_ref()
    }

    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.prepared(sprite)?.second.as_ref()
    }
}

#[cfg(test)]
mod tests;
