//! Localised names, descriptions and sprite thumbnails for focus and technology
//! tokens, read through the source's shared content catalogue.

use super::ContentOptions;
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue},
    invalid,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Longest side of returned product artwork previews.
pub(crate) const ICON_EDGE: u32 = 96;

/// The retained compressed artwork budget shared by product content caches.
pub(crate) const MAX_RETAINED_ARTWORK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Technology,
}

pub struct Context {
    kind: Kind,
    technology_equipment: BTreeMap<String, String>,
    /// The source's shared localisation and sprites for these folders.
    catalogue: Arc<Catalogue>,
    artwork_cache: BTreeMap<String, (Artwork, Vec<String>)>,
    artwork_bytes: usize,
    definitions: BTreeMap<String, installed_content::focus::FocusDefinition>,
    point_days: Option<f64>,
    warnings: Vec<String>,
}

/// The source's artwork context for the configured folders, loaded on first use.
pub fn ensure<'a>(
    slot: &'a mut Option<Context>,
    source: &Source,
    content: &ContentOptions,
    kind: Kind,
) -> io::Result<&'a mut Context> {
    let catalogue = catalogue::catalogue(source, content)?;
    if !slot
        .as_ref()
        .is_some_and(|cached| Arc::ptr_eq(&cached.catalogue, &catalogue))
    {
        *slot = Some(Context::load(catalogue, kind)?);
    }
    Ok(slot.as_mut().expect("artwork context loaded"))
}

impl Context {
    fn load(catalogue: Arc<Catalogue>, kind: Kind) -> io::Result<Self> {
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".into()];
        warnings.extend(catalogue.verification.iter().cloned());
        let layers = &catalogue.content.layers;
        let (definitions, point_days, technology_equipment) = match kind {
            Kind::Focus => {
                let (definitions, definition_warnings) = installed_content::focus::read(layers)?;
                warnings.extend(definition_warnings.into_iter().map(|(_, message)| message));
                (
                    definitions,
                    installed_content::focus::point_days(layers)?,
                    BTreeMap::new(),
                )
            }
            Kind::Technology => {
                let (equipment, technology_warnings) =
                    installed_content::technology::read_equipment(layers)?;
                warnings.extend(technology_warnings.into_iter().map(|(_, message)| message));
                (BTreeMap::new(), None, equipment)
            }
        };
        warnings.extend(catalogue.warnings.iter().cloned());
        Ok(Self {
            kind,
            technology_equipment,
            catalogue,
            artwork_cache: BTreeMap::new(),
            artwork_bytes: 0,
            definitions,
            point_days,
            warnings,
        })
    }

    /// The localised text for a key as plain text, a key the content lacks
    /// being its own bytes (the game's localiser, [`Catalogue::localised`]);
    /// None when refused, which the catalogue notes.
    pub fn localise(&self, key: &str) -> Option<String> {
        self.catalogue.localised(key, self.view())
    }

    /// Focus names and descriptions go through 1412B9CA0 (1402D2250,
    /// 1402D21E0); technology names are plain (140EC9B40 -> 140BBAA50).
    fn view(&self) -> catalogue::View {
        match self.kind {
            Kind::Focus => catalogue::View::Scripted,
            Kind::Technology => catalogue::View::Plain,
        }
    }

    /// The texts refused so far, one warning per reason.
    pub fn refusal_lines(&self) -> Vec<String> {
        self.catalogue.refusal_lines()
    }

    /// The first of `keys` the content defines, else the first key, localised:
    /// a focus names one key, which 1402D2250 and 1402D21E0 localise without
    /// a probe, so a key the content lacks is its own bytes (142230940). A
    /// technology's equipment rung is not traced (140BBAA50 probes only
    /// `TAG_key_short`, `key_short` and `TAG_key`).
    fn first_defined(&self, keys: impl IntoIterator<Item = String>) -> Option<String> {
        let localisation = &self.catalogue.localisation;
        let mut keys = keys.into_iter().peekable();
        let first = keys.peek()?.clone();
        let key = keys
            .find(|key| localisation.get(key).is_some())
            .unwrap_or(first);
        self.catalogue.localised(&key, self.view())
    }

    /// The `_desc` suffix is the executable's (`1427C7718`), appended by
    /// for technologies. That function first tries a `<TAG>_` prefixed key for
    /// the viewing country; artwork is per token, so that variant is not read.
    fn description(&self, token: &str) -> Option<String> {
        let key = self
            .definitions
            .get(token)
            .map(|d| d.description.clone())
            .unwrap_or_else(|| format!("{token}_desc"));
        let equipment = self
            .technology_equipment
            .get(token)
            .map(|equipment| format!("{equipment}_desc"));
        self.first_defined(std::iter::once(key).chain(equipment))
    }

    fn base_duration_days(&self, token: &str) -> Option<f64> {
        let days = self.definitions.get(token)?.cost? * self.point_days?;
        days.is_finite().then_some(days)
    }

    fn name(&self, token: &str) -> Option<String> {
        let definition = self.definitions.get(token);
        let key = definition.map(|d| d.localisation.as_str()).unwrap_or(token);
        let equipment = self.technology_equipment.get(token).cloned();
        self.first_defined(std::iter::once(key.to_owned()).chain(equipment))
    }

    fn artwork(&self, token: &str) -> io::Result<Artwork> {
        let definition = self.definitions.get(token);
        let name = self.name(token);
        let sprites = &self.catalogue.sprites;
        let find = |name: &str| sprites.find(name).map_err(invalid);
        let declared = match definition.and_then(|d| d.icon.as_ref()) {
            Some(id) => find(id)?,
            None => None,
        };
        // _medium: technology_badge_controller::update 141BC1A00, 142936B70.
        let declared = match declared {
            None if self.kind == Kind::Technology => find(&format!("GFX_{token}_medium"))?,
            declared => declared,
        };
        let icon = match declared {
            Some(sprite) if sprite.texture.is_some() => Some(catalogue::sprite_frame_png(
                &self.catalogue.content.layers,
                sprite,
                *self
                    .catalogue
                    .gui_decode
                    .as_ref()
                    .map_err(|why| invalid(why.clone()))?,
                0,
                ICON_EDGE,
            )?),
            _ => None,
        };
        Ok(Artwork {
            token: token.into(),
            name,
            description: self.description(token),
            base_duration_days: self.base_duration_days(token),
            icon,
        })
    }

    /// Artwork for every token, with the context's standing warnings first.
    pub fn catalogue(
        &mut self,
        tokens: BTreeSet<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<Artwork>, Vec<String>)> {
        let mut warnings = self.warnings.clone();
        if tokens.len() > 2048 {
            return Err(invalid("Widget artwork catalogue exceeds its limit"));
        }
        let mut artwork = Vec::new();
        let mut bytes = 0;
        for token in tokens {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Widget request cancelled"));
            }
            if !self.artwork_cache.contains_key(token) {
                let mut messages = Vec::new();
                let art = match self.artwork(token) {
                    Ok(art) => art,
                    Err(error) => {
                        messages.push(format!("{token}: {error}"));
                        Artwork {
                            token: token.into(),
                            name: self.name(token),
                            description: self.description(token),
                            base_duration_days: self.base_duration_days(token),
                            icon: None,
                        }
                    }
                };
                if art.name.is_none() || art.icon.is_none() || art.description.is_none() {
                    messages.push(format!("{token}: some localisation or artwork is unavailable in the recorded content."));
                }
                let size =
                    artwork_bytes(&art) + messages.iter().map(|s| s.len() + 32).sum::<usize>();
                // Retain a multi-country dashboard's working set of compressed
                // icons so switching countries does not repeatedly decode them.
                if self.artwork_bytes + size > MAX_RETAINED_ARTWORK_BYTES {
                    self.artwork_cache.clear();
                    self.artwork_bytes = 0;
                }
                self.artwork_bytes += size;
                self.artwork_cache.insert(token.into(), (art, messages));
            }
            let (art, messages) = &self.artwork_cache[token];
            bytes += artwork_bytes(art);
            if bytes > 8 * 1024 * 1024 {
                return Err(invalid("Widget artwork exceeds output budget"));
            }
            warnings.extend(messages.iter().cloned());
            artwork.push(art.clone());
        }
        Ok((artwork, warnings))
    }
}

pub(crate) fn artwork_bytes(art: &Artwork) -> usize {
    128 + art.token.len()
        + art.name.as_ref().map_or(0, String::len)
        + art.description.as_ref().map_or(0, String::len)
        + art.icon.as_ref().map_or(0, String::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::texture_tests::{from_url, png_bytes, texture_catalogue};
    use installed_content::image::{Rgba8Image, native::Decode};

    #[test]
    fn mod_png_artwork_and_native_refusals_reach_the_catalogue_reply() {
        let image = Rgba8Image {
            width: 6,
            height: 3,
            pixels: (0..18).flat_map(|x| [x, 20, 30, 200]).collect(),
        };
        let (mut catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/art.gfx",
                    concat!(
                        "spriteTypes = { ",
                        "spriteType = { name = GFX_mod_tech_medium textureFile = \"gfx/mod/sheet.png\" noOfFrames = 3 } ",
                        "spriteType = { name = GFX_bad_tech_medium textureFile = \"gfx/mod/noise.png\" } ",
                        "spriteType = { name = GFX_unknown_tech_medium textureFile = \"gfx/mod/tiny.png\" } ",
                        "spriteType = { name = GFX_absent_tech_medium textureFile = \"gfx/mod/absent.png\" } }",
                    ),
                ),
                (
                    "common/terrain/mod.txt",
                    "categories = { mod_ground = { movement_cost = 1 } }",
                ),
            ],
            &[
                ("gfx/mod/sheet.png", png_bytes(&image)),
                ("gfx/mod/noise.png", vec![0x13; 20]),
                ("gfx/mod/tiny.png", vec![0, 0, 1]),
            ],
        );
        catalogue.gui_decode = Ok(Decode {
            force_pow2_textures: true,
        });
        let mut context = Context::load(Arc::new(catalogue), Kind::Technology).unwrap();
        let tokens = BTreeSet::from(["mod_tech", "bad_tech", "unknown_tech", "absent_tech"]);
        let cancelled = AtomicBool::new(false);
        let (artwork, warnings) = context.catalogue(tokens.clone(), &cancelled).unwrap();
        let icon = artwork
            .iter()
            .find(|art| art.token == "mod_tech")
            .unwrap()
            .icon
            .as_deref()
            .unwrap();
        let frame = from_url(icon);
        assert_eq!((frame.width, frame.height), (2, 4));
        assert_eq!(&frame.pixels[..8], [0, 20, 30, 200, 2, 20, 30, 200]);
        assert_eq!(&frame.pixels[24..], [0; 8]);
        for (token, reason) in [
            ("bad_tech", "the game draws no texture either"),
            ("unknown_tech", "does not determine what the game draws"),
            ("absent_tech", "tried"),
        ] {
            assert!(
                artwork
                    .iter()
                    .find(|art| art.token == token)
                    .unwrap()
                    .icon
                    .is_none()
            );
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains(token) && warning.contains(reason)),
                "{warnings:?}"
            );
        }
        let (_, cached_warnings) = context.catalogue(tokens, &cancelled).unwrap();
        assert_eq!(cached_warnings, warnings);
    }

    #[test]
    fn every_texture_refusal_reaches_artwork_warnings() {
        let refusals = crate::catalogue::texture_tests::refusal_textures();
        let sprites = format!(
            "spriteTypes = {{ {} }}",
            refusals
                .iter()
                .enumerate()
                .map(|(i, (path, _, _))| format!(
                    "spriteType = {{ name = GFX_mod_refusal_{i}_medium textureFile = \"{path}\" }}"
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let textures: Vec<_> = refusals
            .iter()
            .filter(|(_, bytes, _)| !bytes.is_empty())
            .map(|(path, bytes, _)| (*path, bytes.clone()))
            .collect();
        let (catalogue, _folders) = texture_catalogue(
            &[
                ("interface/art.gfx", &sprites),
                (
                    "common/terrain/mod.txt",
                    "categories = { mod_ground = { movement_cost = 1 } }",
                ),
            ],
            &textures,
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Technology).unwrap();
        let tokens: Vec<_> = (0..refusals.len())
            .map(|i| format!("mod_refusal_{i}"))
            .collect();
        let cancelled = AtomicBool::new(false);
        let (artwork, warnings) = context
            .catalogue(tokens.iter().map(String::as_str).collect(), &cancelled)
            .unwrap();
        assert!(artwork.iter().all(|art| art.icon.is_none()));
        for (token, (_, _, reason)) in tokens.iter().zip(&refusals) {
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains(token) && warning.contains(reason)),
                "{warnings:?}"
            );
        }
        let (_, cached) = context
            .catalogue(tokens.iter().map(String::as_str).collect(), &cancelled)
            .unwrap();
        assert_eq!(cached, warnings);
    }

    #[test]
    fn unknown_gui_decode_is_the_items_problem_and_keeps_its_text() {
        let (mut catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/art.gfx",
                    "spriteTypes = { spriteType = { name = GFX_mod_tech_medium textureFile = gfx/mod/icon.png } }",
                ),
                (
                    "common/terrain/mod.txt",
                    "categories = { mod_ground = { movement_cost = 1 } }",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n mod_tech:0 \"Mod technology\"\n mod_tech_desc:0 \"Mod description\"\n",
                ),
            ],
            &[],
        );
        let reason = "settings.txt is unavailable: test read failure";
        catalogue.gui_decode = Err(reason.into());
        let mut context = Context::load(Arc::new(catalogue), Kind::Technology).unwrap();
        let (artwork, warnings) = context
            .catalogue(BTreeSet::from(["mod_tech"]), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(artwork[0].name.as_deref(), Some("Mod technology"));
        assert_eq!(artwork[0].description.as_deref(), Some("Mod description"));
        assert!(artwork[0].icon.is_none());
        assert!(
            warnings
                .iter()
                .any(|warning| warning == &format!("mod_tech: {reason}")),
            "{warnings:?}"
        );
    }
}
