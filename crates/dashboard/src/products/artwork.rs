//! Localised names, descriptions and sprite thumbnails for focus and technology
//! tokens, read from the recording's verified content layers.

use super::ContentOptions;
use crate::{Artwork, Source, content, invalid};
use base64::Engine;
use installed_content::{ResolveOptions, content::ResolvedContent};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Technology,
}

pub struct Context {
    options: ResolveOptions,
    kind: Kind,
    technology_equipment: BTreeMap<String, String>,
    content: Arc<ResolvedContent>,
    allow_unverified: bool,
    artwork_cache: BTreeMap<String, (Artwork, Vec<String>)>,
    artwork_bytes: usize,
    definitions: BTreeMap<String, installed_content::focus::FocusDefinition>,
    point_days: Option<f64>,
    localisation: installed_content::localisation::Localisation,
    sprites: installed_content::gfx::SpriteSet,
    warnings: Vec<String>,
}

/// The source's artwork context for the configured folders, loaded on first use.
pub fn ensure<'a>(
    slot: &'a mut Option<Context>,
    source: &Source,
    content: &ContentOptions,
    kind: Kind,
) -> io::Result<&'a mut Context> {
    if !slot
        .as_ref()
        .is_some_and(|cached| cached.matches(&content.options, content.allow_unverified))
    {
        *slot = Some(Context::load(
            &source.replay(),
            source.content(&content.options)?,
            content.options.clone(),
            content.allow_unverified,
            kind,
        )?);
    }
    Ok(slot.as_mut().expect("artwork context loaded"))
}

impl Context {
    fn matches(&self, options: &ResolveOptions, allow: bool) -> bool {
        self.allow_unverified == allow
            && self.options.game_dir == options.game_dir
            && self.options.user_dir == options.user_dir
            && self.options.cache_dir == options.cache_dir
    }

    fn load(
        replay: &replay::container::Replay,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow: bool,
        kind: Kind,
    ) -> io::Result<Self> {
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".into()];
        content::check_replay(replay, allow, &mut warnings)?;
        let (definitions, point_days, technology_equipment) = match kind {
            Kind::Focus => {
                let (definitions, definition_warnings) = installed_content::focus::read(&content.layers)?;
                warnings.extend(definition_warnings);
                (
                    definitions,
                    installed_content::focus::point_days(&content.layers)?,
                    BTreeMap::new(),
                )
            }
            Kind::Technology => {
                let (equipment, technology_warnings) =
                    installed_content::technology::read_equipment(&content.layers)?;
                warnings.extend(technology_warnings);
                (BTreeMap::new(), None, equipment)
            }
        };
        let localisation =
            installed_content::localisation::read(&content.layers, &content::language(&options)?)?;
        let sprites = installed_content::gfx::read(&content.layers)?;
        for (label, count) in [
            ("unparsed localisation lines", localisation.unparsed.len()),
            ("duplicate localisation keys", localisation.duplicates.len()),
            ("recovered sprite files", sprites.recovered.len()),
        ] {
            if count > 0 {
                warnings.push(format!("Recorded content contains {count} {label}."));
            }
        }
        Ok(Self {
            options,
            kind,
            technology_equipment,
            content,
            allow_unverified: allow,
            artwork_cache: BTreeMap::new(),
            artwork_bytes: 0,
            definitions,
            point_days,
            localisation,
            sprites,
            warnings,
        })
    }

    /// The localised text for a key, cleaned of colour codes; None when the
    /// text needs runtime scope evaluation or is missing.
    pub fn localise(&self, key: &str) -> Option<String> {
        localised(
            self.localisation.get(key).unwrap_or(key),
            &self.localisation,
        )
        .filter(|name| name.len() <= 4096)
    }

    fn description(&self, token: &str) -> Option<String> {
        let key = self
            .definitions
            .get(token)
            .map(|d| d.description.clone())
            .unwrap_or_else(|| format!("{token}_desc"));
        self.localisation
            .get(&key)
            .or_else(|| {
                self.technology_equipment
                    .get(token)
                    .and_then(|equipment| self.localisation.get(&format!("{equipment}_desc")))
            })
            .and_then(|text| localised(text, &self.localisation))
    }

    fn base_duration_days(&self, token: &str) -> Option<f64> {
        let days = self.definitions.get(token)?.cost? * self.point_days?;
        days.is_finite().then_some(days)
    }

    fn name(&self, token: &str) -> Option<String> {
        let definition = self.definitions.get(token);
        let key = definition.map(|d| d.localisation.as_str()).unwrap_or(token);
        self.localisation
            .get(key)
            .or_else(|| {
                self.technology_equipment
                    .get(token)
                    .and_then(|equipment| self.localisation.get(equipment))
            })
            .and_then(|s| localised(s, &self.localisation))
    }

    fn artwork(&self, token: &str) -> io::Result<Artwork> {
        let definition = self.definitions.get(token);
        let name = self.name(token);
        let icon = (|| -> io::Result<Option<String>> {
            let Some(sprite) = definition
                .and_then(|d| d.icon.as_ref())
                .and_then(|id| self.sprites.sprites.get(id))
                // The installed technology interface declares GFX_<technology>_medium
                // (e.g. basic_machine_tools). Resolve only actual authored sprites.
                .or_else(|| {
                    (self.kind == Kind::Technology)
                        .then(|| self.sprites.sprites.get(&format!("GFX_{token}_medium")))
                        .flatten()
                })
            else {
                return Ok(None);
            };
            let Some(path) = &sprite.texture else {
                return Ok(None);
            };
            let bytes = self.content.layers.read_uncovered(path)?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(invalid("Artwork exceeds input budget"));
            }
            let image = if path.to_ascii_lowercase().ends_with(".dds") {
                // Check dimensions before decoding the container's authored mip levels.
                if bytes.len() < 128
                    || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) > 2048
                    || u32::from_le_bytes(bytes[16..20].try_into().unwrap()) > 2048
                {
                    return Err(invalid("Artwork dimensions exceed budget"));
                }
                installed_content::image::rgba::dds_base(&installed_content::image::dds::decode(&bytes)?)?
            } else if path.to_ascii_lowercase().ends_with(".tga") {
                if bytes.len() < 18
                    || u16::from_le_bytes(bytes[12..14].try_into().unwrap()) > 2048
                    || u16::from_le_bytes(bytes[14..16].try_into().unwrap()) > 2048
                {
                    return Err(invalid("Artwork dimensions exceed budget"));
                }
                installed_content::image::tga::decode(&bytes)?
            } else {
                return Err(invalid("Unsupported artwork format"));
            };
            // The first authored frame is the normal state; never stretch a sprite sheet.
            if sprite.frames == 0 || image.width % sprite.frames != 0 {
                return Err(invalid("Invalid sprite frames"));
            }
            let frame_width = image.width / sprite.frames;
            let scale = (frame_width.max(image.height) as f64 / 96.0).max(1.0);
            let width = (frame_width as f64 / scale).round().max(1.0) as u32;
            let height = (image.height as f64 / scale).round().max(1.0) as u32;
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let at = ((y * image.height / height * image.width + x * frame_width / width)
                        * 4) as usize;
                    pixels.extend_from_slice(&image.pixels[at..at + 4]);
                }
            }
            let mut encoded = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut encoded, width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_compression(png::Compression::Fast);
                let mut writer = encoder.write_header()?;
                writer.write_image_data(&pixels)?;
            }
            Ok(Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(encoded)
            )))
        })()?;
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

fn localised(text: &str, localisation: &installed_content::localisation::Localisation) -> Option<String> {
    let mut result = text.to_owned();
    for _ in 0..8 {
        let mut changed = false;
        let parts = result
            .split('$')
            .enumerate()
            .map(|(i, part)| {
                if i % 2 == 1 {
                    if let Some(value) = localisation.get(part) {
                        changed = true;
                        value.to_owned()
                    } else {
                        format!("${part}$")
                    }
                } else {
                    part.to_owned()
                }
            })
            .collect::<String>();
        result = parts;
        if !changed || result.len() > 8192 {
            break;
        }
    }
    // Scripted scope functions require runtime operands; do not pass their source
    // expression off as a resolved game name.
    if result.contains(['[', ']', '$']) || result.len() > 8192 {
        return None;
    }
    let mut chars = result.chars();
    let mut clean = String::new();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            clean.push(c);
        }
    }
    Some(clean.replace("\\n", "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn localised_names_remove_colour_and_refuse_unresolved_scripts() {
        let loc = installed_content::localisation::Localisation::default();
        assert_eq!(
            localised("§YReal name§!\\nSecond line", &loc).as_deref(),
            Some("Real name\nSecond line")
        );
        assert_eq!(localised("[ROOT.GetName]", &loc), None);
        assert_eq!(localised("$MISSING$", &loc), None);
        assert_eq!(localised(&"x".repeat(8193), &loc), None);
    }
}
