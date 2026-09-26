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

/// Longest side of a returned focus or technology icon.
const ICON_EDGE: u32 = 96;

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
                warnings.extend(definition_warnings);
                (
                    definitions,
                    installed_content::focus::point_days(layers)?,
                    BTreeMap::new(),
                )
            }
            Kind::Technology => {
                let (equipment, technology_warnings) =
                    installed_content::technology::read_equipment(layers)?;
                warnings.extend(technology_warnings);
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

    /// The localised text for a key, cleaned of colour codes; None when the
    /// text needs runtime scope evaluation or is missing.
    pub fn localise(&self, key: &str) -> Option<String> {
        let localisation = &self.catalogue.localisation;
        catalogue::resolve_text(localisation.get(key).unwrap_or(key), localisation)
            .text()
            .filter(|name| name.len() <= 4096)
    }

    fn description(&self, token: &str) -> Option<String> {
        let key = self
            .definitions
            .get(token)
            .map(|d| d.description.clone())
            .unwrap_or_else(|| format!("{token}_desc"));
        let localisation = &self.catalogue.localisation;
        localisation
            .get(&key)
            .or_else(|| {
                self.technology_equipment
                    .get(token)
                    .and_then(|equipment| localisation.get(&format!("{equipment}_desc")))
            })
            .and_then(|text| catalogue::resolve_text(text, localisation).text())
    }

    fn base_duration_days(&self, token: &str) -> Option<f64> {
        let days = self.definitions.get(token)?.cost? * self.point_days?;
        days.is_finite().then_some(days)
    }

    fn name(&self, token: &str) -> Option<String> {
        let definition = self.definitions.get(token);
        let key = definition.map(|d| d.localisation.as_str()).unwrap_or(token);
        let localisation = &self.catalogue.localisation;
        localisation
            .get(key)
            .or_else(|| {
                self.technology_equipment
                    .get(token)
                    .and_then(|equipment| localisation.get(equipment))
            })
            .and_then(|s| catalogue::resolve_text(s, localisation).text())
    }

    fn artwork(&self, token: &str) -> io::Result<Artwork> {
        let definition = self.definitions.get(token);
        let name = self.name(token);
        let sprites = &self.catalogue.sprites.sprites;
        let icon = match definition
            .and_then(|d| d.icon.as_ref())
            .and_then(|id| sprites.get(id))
            // The installed technology interface declares GFX_<technology>_medium
            // (e.g. basic_machine_tools). Resolve only actual authored sprites.
            .or_else(|| {
                (self.kind == Kind::Technology)
                    .then(|| sprites.get(&format!("GFX_{token}_medium")))
                    .flatten()
            }) {
            Some(sprite) => match &sprite.texture {
                Some(path) => Some(catalogue::texture_png(
                    &self.catalogue.content.layers,
                    path,
                    sprite.frames,
                    ICON_EDGE,
                )?),
                None => None,
            },
            None => None,
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
                if self.artwork_bytes + size > 64 * 1024 * 1024 {
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

fn artwork_bytes(art: &Artwork) -> usize {
    128 + art.token.len()
        + art.name.as_ref().map_or(0, String::len)
        + art.description.as_ref().map_or(0, String::len)
        + art.icon.as_ref().map_or(0, String::len)
}
