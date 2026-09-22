//! Localised focus names and authored goal sprites from verified recorded layers.
use base64::Engine;
use desktop_protocol::dashboard_data::{FocusArtwork, FocusReply, FocusRequest, ResearchReply};
use installed_content::{ResolveOptions, content::ResolvedContent};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct Context {
    options: ResolveOptions,
    research: bool,
    technology_equipment: BTreeMap<String, String>,
    content: Arc<ResolvedContent>,
    allow_unverified: bool,
    artwork_cache: BTreeMap<String, (FocusArtwork, Vec<String>)>,
    artwork_bytes: usize,
    definitions: BTreeMap<String, installed_content::focus::FocusDefinition>,
    point_days: Option<f64>,
    localisation: installed_content::localisation::Localisation,
    sprites: installed_content::gfx::SpriteSet,
    warnings: Vec<String>,
}

impl Context {
    pub(crate) fn matches(&self, options: &ResolveOptions, allow: bool) -> bool {
        self.allow_unverified == allow
            && self.options.game_dir == options.game_dir
            && self.options.user_dir == options.user_dir
            && self.options.cache_dir == options.cache_dir
    }
    pub(crate) fn load(
        replay: &replay::container::Replay,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow: bool,
    ) -> io::Result<Self> {
        Self::load_kind(replay, content, options, allow, false)
    }
    pub(crate) fn load_research(
        replay: &replay::container::Replay,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow: bool,
    ) -> io::Result<Self> {
        Self::load_kind(replay, content, options, allow, true)
    }
    fn load_kind(
        replay: &replay::container::Replay,
        content: Arc<ResolvedContent>,
        options: ResolveOptions,
        allow: bool,
        research: bool,
    ) -> io::Result<Self> {
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".into()];
        crate::content_verification::check_verification(
            "recording",
            &replay.identity.verification,
            allow,
            &mut warnings,
        )?;
        for session in &replay.sessions {
            crate::content_verification::check_verification(
                "session",
                &session.identity.verification,
                allow,
                &mut warnings,
            )?;
        }
        let (definitions, definition_warnings) = if research {
            (BTreeMap::new(), Vec::new())
        } else {
            installed_content::focus::read(&content.layers)?
        };
        let point_days = if research {
            None
        } else {
            installed_content::focus::point_days(&content.layers)?
        };
        warnings.extend(definition_warnings);
        let (technology_equipment, technology_warnings) = if research {
            installed_content::technology::read_equipment(&content.layers)?
        } else {
            (BTreeMap::new(), Vec::new())
        };
        warnings.extend(technology_warnings);
        let localisation = installed_content::localisation::read(&content.layers, "english")?;
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
            research,
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
    fn artwork(&self, token: &str) -> io::Result<FocusArtwork> {
        let definition = self.definitions.get(token);
        let name = self.name(token);
        let icon = (|| -> io::Result<Option<String>> {
            let Some(sprite) = definition
                .and_then(|d| d.icon.as_ref())
                .and_then(|id| self.sprites.sprites.get(id))
                // The installed technology interface declares GFX_<technology>_medium
                // (e.g. basic_machine_tools). Resolve only actual authored sprites.
                .or_else(|| {
                    self.research
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
                return Err(crate::invalid("Focus artwork exceeds input budget"));
            }
            let image = if path.to_ascii_lowercase().ends_with(".dds") {
                // Check dimensions before decoding the container's authored mip levels.
                if bytes.len() < 128
                    || u32::from_le_bytes(bytes[12..16].try_into().unwrap()) > 2048
                    || u32::from_le_bytes(bytes[16..20].try_into().unwrap()) > 2048
                {
                    return Err(crate::invalid("Focus artwork dimensions exceed budget"));
                }
                installed_content::image::rgba::dds_base(&installed_content::image::dds::decode(&bytes)?)?
            } else if path.to_ascii_lowercase().ends_with(".tga") {
                if bytes.len() < 18
                    || u16::from_le_bytes(bytes[12..14].try_into().unwrap()) > 2048
                    || u16::from_le_bytes(bytes[14..16].try_into().unwrap()) > 2048
                {
                    return Err(crate::invalid("Focus artwork dimensions exceed budget"));
                }
                installed_content::image::tga::decode(&bytes)?
            } else {
                return Err(crate::invalid("Unsupported focus artwork format"));
            };
            // The first authored frame is the normal state; never stretch a sprite sheet.
            if sprite.frames == 0 || image.width % sprite.frames != 0 {
                return Err(crate::invalid("Invalid focus sprite frames"));
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
        Ok(FocusArtwork {
            token: token.into(),
            name,
            description: self.description(token),
            base_duration_days: self.base_duration_days(token),
            icon,
        })
    }
    pub(crate) fn reply(
        &mut self,
        query: &FocusRequest,
        history: analytics::FocusHistory,
        cancelled: &AtomicBool,
    ) -> io::Result<FocusReply> {
        let tokens: BTreeSet<_> = history
            .segments
            .iter()
            .flat_map(|segment| {
                segment
                    .order
                    .completed
                    .iter()
                    .map(|r| r.token.as_str())
                    .chain(segment.order.current.as_deref())
                    .chain(segment.order.continuous.as_deref())
            })
            .collect();
        let (artwork, warnings) = self.catalogue(tokens, cancelled)?;
        Ok(FocusReply {
            source_id: query.source_id.clone(),
            revision: query.revision.clone(),
            branch_id: query.branch_id.clone(),
            tag: query.tag.clone(),
            history,
            artwork,
            warnings,
        })
    }
    pub(crate) fn research_reply(
        &mut self,
        query: &FocusRequest,
        mut history: analytics::ResearchHistory,
        cancelled: &AtomicBool,
    ) -> io::Result<ResearchReply> {
        let tokens = history
            .slots
            .iter()
            .flat_map(|s| {
                s.entries
                    .iter()
                    .map(|e| e.token.as_str())
                    .chain(s.states.iter().filter_map(|state| state.token.as_deref()))
            })
            .collect();
        let (artwork, mut warnings) = self.catalogue(tokens, cancelled)?;
        for slot in &mut history.slots {
            for state in &mut slot.states {
                if let Some(modifiers) = state.details.as_mut().and_then(|d| d.modifiers.as_mut()) {
                    for source in modifiers.components.iter_mut().flat_map(|c| &mut c.sources) {
                        if let Some(token) = &source.token {
                            source.name = localised(
                                self.localisation.get(token).unwrap_or(token.as_str()),
                                &self.localisation,
                            )
                            .filter(|name| name.len() <= 4096);
                        }
                    }
                }
                for bonus in state
                    .details
                    .as_mut()
                    .and_then(|d| d.bonuses.as_mut())
                    .into_iter()
                    .flatten()
                {
                    if let Some(token) = &bonus.token {
                        bonus.name = self
                            .localisation
                            .get(token)
                            .and_then(|text| localised(text, &self.localisation))
                            .filter(|name| name.len() <= 4096);
                    }
                }
            }
        }
        warnings.push("Research dates are observation bounds. Elapsed durations use observed assignments across slots; initial progress and gaps can leave the full duration unknown. Base duration is derived separately from recorded cost operands. Technologies without observed slot assignments are not assigned to a column.".into());
        Ok(ResearchReply {
            source_id: query.source_id.clone(),
            revision: query.revision.clone(),
            branch_id: query.branch_id.clone(),
            tag: query.tag.clone(),
            history,
            artwork,
            warnings,
        })
    }
    fn catalogue(
        &mut self,
        tokens: BTreeSet<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<FocusArtwork>, Vec<String>)> {
        let mut warnings = self.warnings.clone();
        if tokens.len() > 2048 {
            return Err(crate::invalid("Widget artwork catalogue exceeds its limit"));
        }
        let mut artwork = Vec::new();
        let mut bytes = 0;
        for token in tokens {
            if cancelled.load(Ordering::Acquire) {
                return Err(crate::invalid("Widget request cancelled"));
            }
            if !self.artwork_cache.contains_key(token) {
                let mut messages = Vec::new();
                let art = match self.artwork(token) {
                    Ok(art) => art,
                    Err(error) => {
                        messages.push(format!("{token}: {error}"));
                        FocusArtwork {
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
                return Err(crate::invalid("Widget artwork exceeds output budget"));
            }
            warnings.extend(messages.iter().cloned());
            artwork.push(art.clone());
        }
        Ok((artwork, warnings))
    }
}

fn artwork_bytes(art: &FocusArtwork) -> usize {
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
