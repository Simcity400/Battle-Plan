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

/// The sprite `CNationalFocus::GetIconName` (1402D0890) names for a focus
/// without an icon.
const UNKNOWN_FOCUS_ICON: &str = installed_content::focus::UNKNOWN_ICON;

/// Longest side of returned product artwork previews.
pub(crate) const ICON_EDGE: u32 = 96;

/// The retained compressed artwork budget shared by product content caches.
pub(crate) const MAX_RETAINED_ARTWORK_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Technology,
}

struct TechnologyName {
    presentation: Result<installed_content::technology::PresentationName, String>,
    children: std::sync::OnceLock<Result<Vec<String>, String>>,
}

#[cfg(test)]
fn technology_name(
    presentation: &installed_content::technology::PresentationName,
    world: Result<&crate::triggers::World<'_>, String>,
    database: Option<&Arc<installed_content::equipment::Database>>,
) -> Result<String, String> {
    let mut children = technology_children(presentation, world, database)?;
    Ok(if children.len() == 1 {
        children.remove(0)
    } else {
        presentation.template.clone()
    })
}

fn technology_children(
    presentation: &installed_content::technology::PresentationName,
    world: Result<&crate::triggers::World<'_>, String>,
    database: Option<&Arc<installed_content::equipment::Database>>,
) -> Result<Vec<String>, String> {
    presentation.children(
        |body| {
            world
                .as_ref()
                .map_err(Clone::clone)?
                .evaluate(body, crate::triggers::Scopes::empty())
                .decided()
        },
        |child| {
            let database =
                database.ok_or_else(|| "native equipment database is unavailable".to_owned())?;
            // 140EBDC40/140C87E70 test normalized category bit 9;
            // feature 0x21 tests DLC bit 7 (1401AE640). Native names/masks
            // 1401ADD20 share the recorded DLC roster used by the
            // content-only has_dlc evaluator, never a row-hour scope.
            let enabled = world
                .as_ref()
                .map(|world| {
                    engine::dlc_feature_state::NAME_MASKS
                        .iter()
                        .any(|(name, mask)| mask & (1 << 7) != 0 && world.has_dlc(name))
                })
                .map_err(Clone::clone);
            database
                .technology_child(child, enabled)?
                .map(|equipment| {
                    equipment
                        .name_str()
                        .map(str::to_owned)
                        .map_err(|e| e.to_string())
                })
                .transpose()
        },
    )
}

pub struct Context {
    kind: Kind,
    technology_names: BTreeMap<String, TechnologyName>,
    /// The source's shared localisation and sprites for these folders.
    catalogue: Arc<Catalogue>,
    artwork_cache: BTreeMap<String, (Artwork, Vec<String>)>,
    artwork_bytes: usize,
    icon_cache: BTreeMap<String, Result<Option<String>, String>>,
    definitions: BTreeMap<String, installed_content::focus::FocusDefinition>,
    focus_text_needs_frame: bool,
    point_days: Option<f64>,
    warnings: Vec<String>,
    inventory: Vec<String>,
    preparation_warnings: Vec<installed_content::Diagnostic>,
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

/// Resolved artwork and, when refused, its feed-local reason.
pub type ResolvedArt = (Artwork, Option<String>);

fn focus_needs_frame(
    catalogue: &Catalogue,
    definition: &installed_content::focus::FocusDefinition,
) -> bool {
    definition
        .icons
        .cases
        .iter()
        .chain(definition.alternate_icons.cases.iter())
        .any(|case| {
            case.trigger
                .as_ref()
                .is_some_and(|trigger| !trigger.is_empty())
        })
        || [&definition.localisation, &definition.description]
            .into_iter()
            .any(|key| {
                catalogue
                    .localisation
                    .localise(key.as_bytes())
                    .is_ok_and(|text| text.contains('['))
            })
}

impl Context {
    pub(crate) fn load(catalogue: Arc<Catalogue>, kind: Kind) -> io::Result<Self> {
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".into()];
        warnings.extend(catalogue.verification.iter().cloned());
        let layers = &catalogue.content.layers;
        let (definitions, point_days, technology_names, inventory, preparation_warnings) =
            match kind {
                Kind::Focus => {
                    let (definitions, definition_warnings) = installed_content::focus::read(layers)?;
                    warnings.extend(
                        definition_warnings
                            .iter()
                            .map(|(_, message)| message.clone()),
                    );
                    let inventory = definitions.keys().cloned().collect();
                    (
                        definitions,
                        installed_content::focus::point_days(layers)?,
                        BTreeMap::new(),
                        inventory,
                        definition_warnings,
                    )
                }
                Kind::Technology => {
                    let read = installed_content::technology::Database::read(layers)?;
                    warnings.extend(
                        read.equipment_warnings
                            .into_iter()
                            .map(|(_, message)| message),
                    );
                    if let Some(database) = layers.key_names()?.equipment_database() {
                        warnings.extend(
                            database
                                .misreads
                                .iter()
                                .map(|note| format!("Equipment database: {note}")),
                        );
                    }
                    let names = read
                        .names
                        .into_iter()
                        .map(|(token, presentation)| {
                            (
                                token,
                                TechnologyName {
                                    presentation,
                                    children: std::sync::OnceLock::new(),
                                },
                            )
                        })
                        .collect();
                    (
                        BTreeMap::new(),
                        None,
                        names,
                        read.tokens,
                        read.inventory_warnings,
                    )
                }
            };
        let focus_text_needs_frame = definitions
            .values()
            .any(|definition| focus_needs_frame(&catalogue, definition));
        warnings.extend(catalogue.warnings.iter().cloned());
        Ok(Self {
            kind,
            technology_names,
            catalogue,
            artwork_cache: BTreeMap::new(),
            artwork_bytes: 0,
            icon_cache: BTreeMap::new(),
            definitions,
            focus_text_needs_frame,
            point_days,
            warnings,
            inventory,
            preparation_warnings,
        })
    }

    /// The localised text for a key as plain text, a key the content lacks
    /// being its own bytes (the game's localiser, [`Catalogue::localised`]);
    /// None when refused, which the catalogue notes.
    pub fn localise(&self, key: &str) -> Option<String> {
        if matches!(self.kind, Kind::Technology) {
            return self.catalogue.localised(key, self.view());
        }
        let (text, findings) = catalogue::localised_details(
            &self.catalogue.localisation,
            key,
            &[],
            self.view(),
            crate::content_check::Kind::Offline,
        );
        for finding in findings {
            self.catalogue.note(finding);
        }
        text
    }

    /// Focus names and descriptions go through 1412B9CA0 (1402D2250,
    /// 1402D21E0); full technology names are plain (140EC9A60).
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

    /// Content-wide notices once per product reply, rather than per row.
    pub fn standing_warnings(&self) -> Vec<String> {
        let mut warnings = self.warnings.clone();
        warnings.sort();
        warnings.dedup();
        warnings
    }

    /// Description 140EC51F0 uses the owner's own tag, never the dynamic
    /// original tag used by names. A single non-duplicate child selects the
    /// first admitted child, then its archetype if neither child key exists;
    /// otherwise 140AC1830 selects the template's nonempty parent/id.
    fn description_key(&self, token: &str, tag: Option<&str>) -> Result<String, String> {
        if self.kind == Kind::Focus {
            return Ok(self
                .definitions
                .get(token)
                .map(|d| d.description.clone())
                .unwrap_or_else(|| format!("{token}_desc")));
        }
        let children = self.technology_children(token)?;
        let presentation = self.technology_names[token]
            .presentation
            .as_ref()
            .map_err(Clone::clone)?;
        let key_for = |root: &str| {
            // LocalizeDescriptionForCountry 140BBA390, suffix 1427C7718.
            let key = format!("{root}_desc");
            let prefixed = tag.map(|tag| format!("{tag}_{key}"));
            prefixed
                .filter(|key| self.catalogue.localisation.get(key).is_some())
                .unwrap_or(key)
        };
        if !children.is_empty() {
            let registry = self
                .catalogue
                .content
                .layers
                .key_names()
                .map_err(|error| error.to_string())?;
            let database = registry
                .equipment_database()
                .ok_or_else(|| "native equipment database is unavailable".to_owned())?;
            let mut ordinary = 0;
            for name in children {
                let child = database
                    .named(name)
                    .ok_or_else(|| format!("native admitted equipment {name} is unavailable"))?;
                ordinary += usize::from(!child.duplicate);
            }
            if ordinary == 1 {
                let first = database
                    .named(&children[0])
                    .expect("admitted child resolved above");
                let key = key_for(first.name_str().map_err(|error| error.to_string())?);
                if self.catalogue.localisation.get(&key).is_some() {
                    return Ok(key);
                }
                let archetype = first.archetype.and_then(|index| database.equipment.get(index))
                    .ok_or_else(|| format!("technology {token}: first equipment description archetype is unavailable (140EC51F0)"))?;
                return Ok(key_for(
                    archetype.name_str().map_err(|error| error.to_string())?,
                ));
            }
        }
        Ok(key_for(&presentation.template))
    }

    fn description(&self, token: &str) -> Option<String> {
        self.localise(&self.description_key(token, None).ok()?)
    }

    pub(crate) fn description_is_defined(&self, token: &str) -> bool {
        self.description_key(token, None)
            .is_ok_and(|key| self.catalogue.localisation.get(&key).is_some())
    }

    fn base_duration_days(&self, token: &str) -> Option<f64> {
        let days = self.definitions.get(token)?.cost? * self.point_days?;
        days.is_finite().then_some(days)
    }

    fn technology_children(&self, token: &str) -> Result<&[String], String> {
        let presentation = self.technology_names.get(token).ok_or_else(|| {
            format!("technology {token}: definition unavailable in recorded content")
        })?;
        presentation
            .children
            .get_or_init(|| {
                let presentation = presentation.presentation.as_ref().map_err(Clone::clone)?;
                let registry = self
                    .catalogue
                    .content
                    .layers
                    .key_names()
                    .map_err(|error| error.to_string())?;
                let world = self
                    .catalogue
                    .scripted_content()
                    .and_then(|content| content.unrecorded_trigger_world());
                technology_children(
                    presentation,
                    world.as_ref().map_err(Clone::clone),
                    registry.equipment_database(),
                )
            })
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }

    fn technology_root(&self, token: &str) -> Result<String, String> {
        let children = self.technology_children(token)?;
        Ok(if children.len() == 1 {
            children[0].clone()
        } else {
            self.technology_names[token]
                .presentation
                .as_ref()
                .map_err(Clone::clone)?
                .template
                .clone()
        })
    }

    fn name(&self, token: &str) -> Option<String> {
        if self.kind == Kind::Technology {
            // Topic artwork has no country/hour parameter. It uses the same
            // native full-name root as the feed, without a country prefix.
            let key = self.technology_root(token).ok()?;
            return self.localise(&key);
        }
        let definition = self.definitions.get(token);
        let key = definition.map(|d| d.localisation.as_str()).unwrap_or(token);
        self.localise(key)
    }

    fn declared_icon(
        &self,
        token: &str,
    ) -> io::Result<(
        Option<&installed_content::gfx::SpriteType>,
        Vec<crate::content_check::Finding>,
    )> {
        use crate::content_check::{Finding, Kind as FindingKind};
        let selected = match self.kind {
            // Technology badge 141BC1A00 uses _medium (142936B70).
            Kind::Technology => Some(format!("GFX_{token}_medium")),
            Kind::Focus => match self.definitions.get(token) {
                Some(definition) => match definition.icon(None, |trigger| {
                    if trigger.is_empty() {
                        Ok(true)
                    } else {
                        Err("focus icon requires recorded country/hour trigger operands".into())
                    }
                }) {
                    Ok(name) => Some(name.unwrap_or(UNKNOWN_FOCUS_ICON).to_owned()),
                    Err(reason) => {
                        return Ok((
                            None,
                            vec![Finding::grouped(
                                FindingKind::Offline,
                                token,
                                "focus icon requires recorded context",
                                reason,
                            )],
                        ));
                    }
                },
                None => None,
            },
        };
        let declared = selected
            .as_deref()
            .map(|name| self.catalogue.sprites.find(name).map_err(invalid))
            .transpose()?
            .flatten();
        let mut findings = Vec::new();
        if declared.is_none_or(|sprite| sprite.texture.is_none()) {
            let message = match selected {
                Some(name) => {
                    format!("sprite {name} is undeclared or has no texture; a placeholder is shown")
                }
                None => {
                    "focus definition is absent from the recorded content; a placeholder is shown"
                        .into()
                }
            };
            findings.push(Finding::grouped(
                FindingKind::Fallback,
                token,
                "icon is unavailable; a placeholder is shown",
                message,
            ));
        }
        Ok((declared, findings))
    }

    fn artwork(&self, token: &str) -> io::Result<(Artwork, Vec<crate::content_check::Finding>)> {
        let name = self.name(token);
        let (declared, findings) = self.declared_icon(token)?;
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
        Ok((
            Artwork {
                token: token.into(),
                name,
                description: self.description(token),
                base_duration_days: self.base_duration_days(token),
                icon,
            },
            findings,
        ))
    }

    pub(crate) fn tokens(&self) -> (&[String], &[installed_content::Diagnostic]) {
        (&self.inventory, &self.preparation_warnings)
    }

    /// The same per-token preparation for the reply and the offline check.
    pub(crate) fn prepare(&self, token: &str) -> (Artwork, Vec<crate::content_check::Finding>) {
        use crate::content_check::{Finding, Kind};
        let mut findings = Vec::new();
        let art = match self.artwork(token) {
            Ok((art, problems)) => {
                findings.extend(problems);
                art
            }
            Err(error) => {
                // Focus/research widgets draw their own placeholder for None,
                // so even a native no-texture outcome changes the shown art.
                findings.push(Finding::grouped(
                    Kind::Refused,
                    token,
                    "icon is unavailable; a placeholder is shown",
                    format!("icon unavailable: {error}"),
                ));
                Artwork {
                    token: token.into(),
                    name: self.name(token),
                    description: self.description(token),
                    base_duration_days: self.base_duration_days(token),
                    icon: None,
                }
            }
        };
        if let Some(Err(reason)) = self
            .technology_names
            .get(token)
            .and_then(|name| name.children.get())
        {
            findings.push(Finding::grouped(
                Kind::Refused,
                token,
                "technology name is unavailable",
                reason.clone(),
            ));
        }
        if let Err(reason) = self.description_key(token, None) {
            findings.push(Finding::grouped(
                Kind::Refused,
                token,
                "description is unavailable",
                reason,
            ));
        }
        (art, findings)
    }

    pub fn requested_focus_needs_frame(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .filter_map(|token| self.definitions.get(token))
            .any(|definition| focus_needs_frame(&self.catalogue, definition))
    }

    /// Whether focus text or icon cases need recorded country/hour operands.
    /// Dollar references have already expanded through the content localiser.
    pub fn focus_text_needs_frame(&self) -> bool {
        self.focus_text_needs_frame
    }

    /// Event focus presentation resolves names and first-true icon cases at the row hour.
    pub fn focuses_at(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay::frame::DecodedFrame,
    ) -> Vec<ResolvedArt> {
        self.focuses(requests, frame, false, |items| items.collect())
    }

    /// The caller admits each lazy item while the batch shares its text context.
    pub fn focus_tree_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, collect)
    }

    fn focuses<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay::frame::DecodedFrame,
        tree: bool,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        let catalogue = Arc::clone(&self.catalogue);
        let text = std::sync::OnceLock::new();
        let countries = frame
            .complete::<replay::rows::tables::Countries>()
            .map_err(|error| error.to_string());
        let links = frame
            .complete::<replay::rows::tables::CountryLinks>()
            .map_err(|error| error.to_string());
        let mut country_scopes = BTreeMap::new();
        let mut items = requests.iter().map(|&(token, tag)| {
            let country = country_scopes
                .entry(tag)
                .or_insert_with(|| -> Result<Option<u32>, String> {
                    let countries = countries.as_ref().map_err(Clone::clone)?;
                    let links = links.as_ref().map_err(Clone::clone)?;
                    match crate::metadata::tagged_country(countries, links, tag) {
                        crate::metadata::TaggedCountry::Found(_, slot) => Ok(Some(slot)),
                        crate::metadata::TaggedCountry::Absent => Ok(None),
                        crate::metadata::TaggedCountry::Unresolved => {
                            Err("focus country identity is unresolved".into())
                        }
                    }
                })
                .clone();
            let context = || {
                text.get_or_init(|| {
                    catalogue.scripted_content().map(|content| {
                        crate::scripted_text::ScriptedText::new(
                            frame,
                            &catalogue.localisation,
                            content,
                            0,
                        )
                    })
                })
                .as_ref()
                .map_err(Clone::clone)
            };
            let selected = (|| -> Result<String, String> {
                let definition = self.definitions.get(token).ok_or_else(|| {
                    format!("focus {token}: definition absent from recorded content")
                })?;
                // Focus-tree button 140B32B40 probes authored country/id and
                // id overrides before GetIconName. Event uses the definition.
                if tree {
                    for name in definition.sprite_overrides(token, Some(tag)) {
                        if catalogue.sprites.find(&name)?.is_some() {
                            return Ok(name);
                        }
                    }
                }
                definition
                    .icon(None, |trigger| {
                        if trigger.is_empty() {
                            return Ok(true);
                        }
                        let slot = country.clone()?.ok_or_else(|| {
                            format!("focus {token}: country scope was not recorded")
                        })?;
                        let graph = std::rc::Rc::new(std::cell::RefCell::new(
                            crate::scripted_text::EventScopes::new(vec![
                                crate::scripted_text::EventScope::of(
                                    crate::scripted_text::EventObject::Country(slot),
                                ),
                            ]),
                        ));
                        let evaluator =
                            crate::triggers::Evaluator::new(context()?.trigger_world()?, None)
                                .with_event_scopes(graph);
                        evaluator
                            .evaluate(trigger, evaluator.event_scopes(0)?)
                            .decided()
                    })
                    .map(|name| name.unwrap_or(UNKNOWN_FOCUS_ICON).to_owned())
            })();
            let (icon, mut refusal) = match selected {
                Ok(name) => match self.cached_icon(&name) {
                    Ok(Some(icon)) => (Some(icon), None),
                    Ok(None) => (
                        None,
                        Some(format!(
                            "focus {token}: selected sprite {name} is undeclared or has no texture"
                        )),
                    ),
                    Err(reason) => (None, Some(format!("focus {token}: {reason}"))),
                },
                Err(reason) => (None, Some(reason)),
            };
            let mut art = Artwork {
                token: token.into(),
                name: None,
                description: None,
                base_duration_days: self.base_duration_days(token),
                icon,
            };
            let localise = |key: &str| -> Result<String, String> {
                let raw = catalogue
                    .localisation
                    .localise(key.as_bytes())
                    .map_err(|error| error.to_string())?;
                if !raw.contains('[') {
                    return catalogue::plain_text(&raw);
                }
                let slot = country.clone()?.ok_or_else(|| {
                    format!("focus {token}: Countries has no row for the requested country")
                })?;
                let context = context()?;
                let mut scope = crate::scripted_text::EventScope::of(
                    crate::scripted_text::EventObject::Country(slot),
                );
                // GetLocalizedName/GetDescription 1402D2250/1402D21E0
                // seed a fresh scope with Clock seed + byte-exact focus ID
                // FNV-1a (1402DADC0/1402D70E0), then the shared hash lanes.
                let seed = frame
                    .complete::<replay::rows::tables::Clock>()
                    .ok()
                    .and_then(|rows| rows.first())
                    .map(|clock| clock.seed as u32);
                let scopes = match seed {
                    Some(seed) => {
                        let hash = token.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
                            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
                        });
                        let seed = seed.wrapping_add(hash);
                        scope.random_seed = crate::scripted_text::random::scope_seed(seed);
                        scope.random_counter = crate::scripted_text::random::scope_counter(seed);
                        crate::scripted_text::EventScopes::new(vec![scope])
                    }
                    None => crate::scripted_text::EventScopes::new(vec![scope]),
                };
                let expanded = if seed.is_some() {
                    context.expand_in(&raw, &scopes)
                } else {
                    context.expand_in_without_random_seed(
                        &raw,
                        &scopes,
                        "focus random_list: Clock seed is not recorded (1402DADC0)",
                    )
                }?;
                catalogue::plain_text(&expanded)
            };
            if let Some(definition) = self.definitions.get(token) {
                for (key, target) in [
                    (&definition.localisation, &mut art.name),
                    (&definition.description, &mut art.description),
                ] {
                    match localise(key) {
                        Ok(value) => *target = Some(value),
                        Err(reason) => {
                            *target = None;
                            let reason = format!("focus localisation {key}: {reason}");
                            match &mut refusal {
                                Some(previous) => {
                                    if !previous.contains(&reason) {
                                        previous.push_str("; ");
                                        previous.push_str(&reason);
                                    }
                                }
                                None => refusal = Some(reason),
                            };
                        }
                    }
                }
            }
            (art, refusal)
        });
        collect(&mut items)
    }

    fn cached_icon(&mut self, name: &str) -> io::Result<Option<String>> {
        if let Some(result) = self.icon_cache.get(name) {
            return result.clone().map_err(invalid);
        }
        let result = (|| -> io::Result<Option<String>> {
            let Some(sprite) = self.catalogue.sprites.find(name).map_err(invalid)? else {
                return Ok(None);
            };
            if sprite.texture.is_none() {
                return Ok(None);
            }
            Ok(Some(catalogue::sprite_frame_png(
                &self.catalogue.content.layers,
                sprite,
                *self
                    .catalogue
                    .gui_decode
                    .as_ref()
                    .map_err(|why| invalid(why.clone()))?,
                0,
                ICON_EDGE,
            )?))
        })()
        .map_err(|error| error.to_string());
        let size = name.len()
            + result.as_ref().map_or_else(
                |error| error.len(),
                |icon| icon.as_ref().map_or(0, String::len),
            );
        if self.artwork_bytes.saturating_add(size) > MAX_RETAINED_ARTWORK_BYTES {
            self.artwork_cache.clear();
            self.icon_cache.clear();
            self.artwork_bytes = 0;
        }
        self.artwork_bytes = self.artwork_bytes.saturating_add(size);
        self.icon_cache.insert(name.into(), result.clone());
        result.map_err(invalid)
    }

    /// History supplies authored duration only; names and icons are dated reads.
    pub fn focus_metadata(
        &self,
        tokens: BTreeSet<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<Artwork>, Vec<String>)> {
        let mut items = Vec::with_capacity(tokens.len());
        for token in tokens {
            crate::metadata::check_cancel(cancelled)?;
            items.push(Artwork {
                token: token.into(),
                name: None,
                description: None,
                base_duration_days: self.base_duration_days(token),
                icon: None,
            });
        }
        Ok((items, self.standing_warnings()))
    }

    /// Artwork for every token, with the context's standing warnings first.
    pub fn catalogue(
        &mut self,
        tokens: BTreeSet<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<Artwork>, Vec<String>)> {
        let mut warnings = self.warnings.clone();
        let mut artwork = Vec::new();
        let mut bytes = 0;
        for token in tokens {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Widget request cancelled"));
            }
            if !self.artwork_cache.contains_key(token) {
                let (art, findings) = self.prepare(token);
                let messages: Vec<String> = findings
                    .into_iter()
                    .map(|finding| format!("{token}: {}", finding.message))
                    .collect();
                let size =
                    artwork_bytes(&art) + messages.iter().map(|s| s.len() + 32).sum::<usize>();
                // Retain a multi-country dashboard's working set of compressed
                // icons so switching countries does not repeatedly decode them.
                if self.artwork_bytes + size > MAX_RETAINED_ARTWORK_BYTES {
                    self.artwork_cache.clear();
                    self.icon_cache.clear();
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

/// The stockpile indicator's authored image. The executable selects the
/// "topbar" template (140B52390, literal 142934CD8) and "nukes" container
/// (14188BA30, literal 142712608). Container creation 1422FAF10/14229F050
/// runs 1422A9950 over authored children; 1422FA550 dispatches their registered
/// factories. CIconType 142358760 constructs CIcon 1422E5D40, whose drawable
/// builder 1422E6B90 resolves the authored sprite through 142222AD0. Its sole
/// icon supplies the artwork; ambiguous content refuses by authored paths.
pub fn nuke_icon(catalogue: &Catalogue) -> io::Result<Option<String>> {
    use installed_content::gui::native::{Fields, Kind};
    let (_, root) =
        installed_content::gui::native::kept_container_window(&catalogue.guis, "topbar", &mut Vec::new())?;
    let container = root
        .find_recursive(Kind::Container, "nukes")
        .ok_or_else(|| invalid("nukes artwork: topbar/nukes container unavailable"))?;
    if !root.refusals().is_empty() {
        return Err(invalid(format!(
            "nukes artwork: topbar has {} unsupported GUI fields: {:?}",
            root.refusals().len(),
            root.refusals()
        )));
    }
    let mut stack = vec![container];
    let mut icons = Vec::new();
    while let Some(element) = stack.pop() {
        if element.hide {
            return Err(invalid(format!(
                "nukes artwork: {} is hidden; its client visibility state is not recorded",
                element.path
            )));
        }
        if !element.refusals().is_empty() {
            return Err(invalid(format!(
                "nukes artwork: {} has {} unsupported GUI fields: {:?}",
                element.path,
                element.refusals().len(),
                element.refusals()
            )));
        }
        if matches!(element.fields, Fields::Icon(_)) {
            icons.push(element);
        }
        stack.extend(element.nested());
    }
    if icons.len() != 1 {
        return Err(invalid(format!(
            "nukes artwork: topbar/nukes has {} icons; their semantic role is not recorded: {}",
            icons.len(),
            icons
                .iter()
                .map(|icon| icon.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let Fields::Icon(icon) = &icons[0].fields else {
        unreachable!()
    };
    let name = std::str::from_utf8(&icon.sprite).map_err(|error| invalid(error.to_string()))?;
    let sprite = catalogue::created_gui_sprite(&catalogue.sprites, name)?;
    // CIconType 142358760 -> SetFrame 1422E6A90 -> CSprite 1422E95C0 sends
    // the authored frame to 14222D0D0/14222D5A0: a positive frame stores
    // frame-1 at sprite+0xFC before 14222A990 computes its offset.
    let frame = u32::try_from(
        icon.frame
            .checked_sub(1)
            .ok_or_else(|| invalid("nukes artwork: frame overflow"))?,
    )
    .map_err(|_| {
        invalid(format!(
            "nukes artwork: {} has unsupported frame {}",
            icons[0].path, icon.frame
        ))
    })?;
    Ok(Some(catalogue::sprite_frame_png(
        &catalogue.content.layers,
        sprite,
        *catalogue
            .gui_decode
            .as_ref()
            .map_err(|why| invalid(why.clone()))?,
        frame,
        ICON_EDGE,
    )?))
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
    fn focus_icons_evaluate_native_cases_and_preserve_caller_overrides() {
        use replay::{
            frame::{DecodedFrame, TableView},
            rows::{CountryRow, TableId, TableRows, tables},
        };
        let icon = |value| Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![value, 0, 0, 255],
        };
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/art.gfx",
                    "spriteTypes={spriteType={name=GFX_mod_scalar textureFile=gfx/mod/scalar.png} spriteType={name=GFX_mod_other textureFile=gfx/mod/other.png} spriteType={name=GFX_Z9_scalar textureFile=gfx/mod/other.png} spriteType={name=GFX_goal_unknown textureFile=gfx/mod/other.png}}",
                ),
                (
                    "common/national_focus/mod.txt",
                    "focus_tree={focus={id=scalar icon=GFX_mod_scalar} focus={id=alternate icon=GFX_mod_scalar alternate_icon=GFX_mod_other} focus={id=constant icon={GFX_mod_other=yes}} focus={id=conditional icon={GFX_mod_other={has_country_flag=mod_mark}}} focus={id=iconless}}",
                ),
            ],
            &[
                ("gfx/mod/scalar.png", png_bytes(&icon(11))),
                ("gfx/mod/other.png", png_bytes(&icon(22))),
            ],
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Focus).unwrap();
        assert!(context.focus_text_needs_frame());
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::from_rows::<tables::Countries>(vec![
                CountryRow {
                    id: 7,
                    canonical_slot: 2,
                    tag: b"Z9".to_vec(),
                    ..Default::default()
                },
            ]));
        for token in ["scalar", "constant", "iconless"] {
            let (topic, _) = context.artwork(token).unwrap();
            let feed = context.focuses_at(&[(token, "Z9")], &frame).remove(0);
            assert_eq!(feed.0.icon, topic.icon, "{token}");
            assert_eq!(
                from_url(feed.0.icon.as_deref().unwrap()).pixels[0],
                if token == "scalar" { 11 } else { 22 }
            );
            assert!(feed.1.is_none());
        }
        let alternate = context.focuses_at(&[("alternate", "Z9")], &frame).remove(0);
        assert!(alternate.0.icon.is_none());
        assert!(alternate.1.unwrap().contains("focus-window mode"));
        let conditional = context
            .focuses_at(&[("conditional", "Z9")], &frame)
            .remove(0);
        assert!(conditional.0.icon.is_none());
        assert!(conditional.1.is_some()); // This synthetic stack intentionally lacks trigger defines.
        let tree =
            context.focus_tree_at(&[("scalar", "Z9")], &frame, |items| items.next().unwrap());
        assert_eq!(from_url(tree.0.icon.as_deref().unwrap()).pixels[0], 22);
        assert!(tree.1.is_none());
    }

    #[test]
    fn focus_art_uses_the_row_country_and_only_refuses_a_missing_random_seed() {
        use replay::{
            frame::{DecodedFrame, TableView},
            rows::{
                ClockRow, CountryLinkRow, CountryRow, PoliticalPartyRow, TableId, TableRows, tables,
            },
        };
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/national_focus/mod.txt",
                    "focus_tree={focus={id=scoped icon=GFX_scoped} focus={id=random icon=GFX_scoped} focus={id=promoted icon=GFX_scoped} focus={id=conditional icon={GFX_M0={tag=M0} GFX_other=yes}} focus={id=government icon={GFX_party={has_government=mod_party} GFX_other=yes}}}",
                ),
                ("common/ideologies/mod.txt", "ideologies={mod_party={}}"),
                (
                    "common/defines/mod.lua",
                    "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
                ),
                (
                    "common/scripted_localisation/mod.txt",
                    "a={name=Choice text={random_list={1={localization_key=A} 1={localization_key=B}}}}",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n scoped:0 \"[ROOT.GetTag] focus\"\n scoped_desc:0 \"For [FROM.GetTag]\"\n random:0 \"[Choice]\"\n random_desc:0 \"Plain description\"\n promoted:0 \"[ROOT.Choice]\"\n A:0 \"A\"\n B:0 \"B\"\n",
                ),
            ],
            &[],
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Focus).unwrap();
        assert!(context.focus_text_needs_frame());
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::from_rows::<tables::Countries>(vec![
                CountryRow {
                    id: 7,
                    canonical_slot: 2,
                    tag: b"M0".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::from_rows::<tables::CountryLinks>(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 2,
                },
            ]));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                replay::rows::CountryCounterInputRow {
                    country: 7,
                    owned_state_count: 1,
                    ..Default::default()
                },
            ]));
        let result = context.focuses_at(
            &[("scoped", "M0"), ("random", "M0"), ("promoted", "M0")],
            &frame,
        );
        assert_eq!(result[0].0.name.as_deref(), Some("M0 focus"));
        assert_eq!(result[0].0.description.as_deref(), Some("For M0"));
        assert!(
            !result[0]
                .1
                .as_deref()
                .is_some_and(|reason| reason.contains("localisation"))
        );
        assert!(result[1].0.name.is_none());
        assert!(result[1].1.as_deref().unwrap().contains("1402DADC0"));
        let conditional = context
            .focuses_at(&[("conditional", "M0")], &frame)
            .remove(0);
        assert!(
            conditional
                .1
                .as_deref()
                .unwrap()
                .contains("selected sprite GFX_M0 ")
        );
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::from_rows::<tables::Clock>(vec![ClockRow {
                seed: 987,
                ..Default::default()
            }]));
        let seeded = context.focuses_at(&[("random", "M0")], &frame).remove(0);
        assert!(matches!(seeded.0.name.as_deref(), Some("A" | "B")));
        assert!(
            !seeded
                .1
                .as_deref()
                .is_some_and(|reason| reason.contains("localisation"))
        );
        assert_eq!(
            context.focuses_at(&[("random", "M0")], &frame)[0].0.name,
            seeded.0.name
        );
        // ROOT selects the country context 141A81250; its entry fallback
        // constructs a fresh known 424242 scope 1410CD900/1410CDDF0.
        assert!(matches!(result[2].0.name.as_deref(), Some("A" | "B")));
        assert!(
            !result[2]
                .1
                .as_deref()
                .is_some_and(|reason| reason.contains("localisation"))
        );
        assert_eq!(
            context
                .standing_warnings()
                .iter()
                .filter(
                    |warning| warning.contains("visual assets are outside content hash coverage")
                )
                .count(),
            1
        );
        assert!(
            !context
                .refusal_lines()
                .iter()
                .any(|reason| reason.contains("1402DADC0"))
        );
        frame.hour = 11;
        // Keep M0 a native tag operand while the row's country renames to M1.
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::from_rows::<tables::Countries>(vec![
                CountryRow {
                    id: 7,
                    canonical_slot: 2,
                    tag: b"M1".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 8,
                    canonical_slot: 3,
                    tag: b"M0".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 8,
                    canonical_slot: 3,
                },
            ]));
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        {
            rows.push(replay::rows::CountryCounterInputRow {
                country: 8,
                owned_state_count: 1,
                ..Default::default()
            });
        }
        *frame.tables[TableId::States.index()] = TableView::Unavailable {
            reason: "synthetic state read failed".into(),
        };
        *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "synthetic province read failed".into(),
        };
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 7,
                id: 1,
                is_ruling: 1,
                token: b"mod_party".to_vec(),
                ..Default::default()
            }]));
        let government = context
            .focuses_at(&[("government", "M1")], &frame)
            .remove(0);
        assert!(
            government
                .1
                .as_deref()
                .unwrap()
                .contains("selected sprite GFX_party ")
        );
        *frame.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
            reason: "synthetic party read failed".into(),
        };
        let government = context
            .focuses_at(&[("government", "M1")], &frame)
            .remove(0);
        let reason = government.1.as_deref().unwrap();
        assert!(reason.contains("PoliticalParties"), "{reason}");
        assert!(
            !reason.contains("selected sprite GFX_other "),
            "unknown first case must not be skipped"
        );
        let conditional = context
            .focuses_at(&[("conditional", "M1")], &frame)
            .remove(0);
        assert!(
            conditional
                .1
                .as_deref()
                .unwrap()
                .contains("selected sprite GFX_other ")
        );
        assert_eq!(
            context.focuses_at(&[("scoped", "M1")], &frame)[0]
                .0
                .name
                .as_deref(),
            Some("M1 focus")
        );
    }

    #[test]
    fn technology_limits_use_load_time_dlcs_empty_scope_and_unknown_globals() {
        // The executable maps this DLC name to bit7 (1401ADD20), which
        // feature0x21 tests 1401AE640. No content key is hard-coded here.
        let feature_dlc = engine::dlc_feature_state::NAME_MASKS
            .iter()
            .find(|(_, mask)| *mask == 1 << 7)
            .unwrap()
            .0;
        let technologies = format!(
            "technologies={{dlc={{enable_equipments={{limit={{has_dlc=\"{feature_dlc}\"}} mod_child}}}} empty={{enable_equipments={{limit={{has_government=mod_party}} mod_child}}}} global={{enable_equipments={{limit={{has_global_flag=mod_flag}} mod_child}}}} invalid={{enable_equipments={{missing_child}}}} harbor={{enable_equipments={{mod_harbor}}}}}}"
        );
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/units/equipment/mod.txt",
                    "equipments={mod_child={} mod_harbor={type=floating_harbor}}",
                ),
                ("common/technologies/mod.txt", &technologies),
                (
                    "common/terrain/mod.txt",
                    "categories = { mod_ground = { movement_cost = 1 } }",
                ),
            ],
            &[],
        );
        let layers = &catalogue.content.layers;
        let presentation = installed_content::technology::Database::read(layers).unwrap();
        let registry = layers.key_names().unwrap();
        let again = layers.key_names().unwrap();
        assert!(Arc::ptr_eq(
            registry.equipment_database().unwrap(),
            again.equipment_database().unwrap()
        ));
        let triggers = installed_content::trigger_keys::TriggerNames::default();
        let aliases = Arc::new(installed_content::tag_aliases::TagAliases::default());
        let empty = BTreeSet::new();
        let world = crate::triggers::World::unrecorded(&empty, &triggers, &aliases);
        let select = |key: &str| {
            technology_name(
                presentation.names[key].as_ref().unwrap(),
                Ok(&world),
                registry.equipment_database(),
            )
        };
        assert_eq!(select("dlc").unwrap(), "dlc");
        // A country predicate is false in the native empty content-load scope.
        assert_eq!(select("empty").unwrap(), "empty");
        let reason = select("global").unwrap_err();
        assert!(
            reason.contains("LexicalTokens") && reason.contains("content load"),
            "{reason}"
        );
        assert_eq!(select("invalid").unwrap(), "invalid");
        assert_eq!(select("harbor").unwrap(), "harbor");
        let enabled = BTreeSet::from([feature_dlc.to_owned()]);
        let world = crate::triggers::World::unrecorded(&enabled, &triggers, &aliases);
        assert_eq!(
            technology_name(
                presentation.names["dlc"].as_ref().unwrap(),
                Ok(&world),
                registry.equipment_database()
            )
            .unwrap(),
            "mod_child"
        );
        // The name is the executable's feature mask mapping 1401ADD20, not a
        // content importance list. Feature0x21 tests the same bit7.
        let enabled = BTreeSet::from([feature_dlc.to_owned()]);
        let world = crate::triggers::World::unrecorded(&enabled, &triggers, &aliases);
        assert_eq!(
            technology_name(
                presentation.names["harbor"].as_ref().unwrap(),
                Ok(&world),
                registry.equipment_database()
            )
            .unwrap(),
            "mod_harbor"
        );
        assert!(
            technology_name(
                presentation.names["harbor"].as_ref().unwrap(),
                Err("DLC load state unavailable".into()),
                registry.equipment_database()
            )
            .unwrap_err()
            .contains("DLC load state")
        );
    }

    #[test]
    fn technology_catalogue_keeps_parent_text_and_skips_unrequested_children() {
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/technologies/mod.txt",
                    "technologies={mod_tech={parent=mod_parent} mod_unrequested={enable_equipments={limit={has_global_flag=mod_flag} missing_child}}}",
                ),
                (
                    "common/terrain/mod.txt",
                    "categories = { mod_ground = { movement_cost = 1 } }",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n mod_tech:0 \"Wrong id\"\n mod_parent:0 \"Parent\"\n mod_parent_short:0 \"Short parent\"\n M0_mod_parent:0 \"Country full\"\n M0_mod_parent_short:0 \"Wrong short\"\n M0_mod_parent_desc:0 \"Country description\"\n mod_parent_desc:0 \"Parent description\"\n mod_tech_desc:0 \"Token description\"\n",
                ),
            ],
            &[],
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Technology).unwrap();
        assert!(
            context
                .technology_names
                .values()
                .all(|name| name.children.get().is_none())
        );
        let (topic, _) = context
            .catalogue(BTreeSet::from(["mod_tech"]), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(topic[0].name.as_deref(), Some("Parent"));
        assert_eq!(topic[0].description.as_deref(), Some("Parent description"));
        assert!(
            context.technology_names["mod_unrequested"]
                .children
                .get()
                .is_none()
        );
    }

    #[test]
    fn technology_descriptions_use_distinct_child_gate_and_archetype_fallback() {
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/units/equipment/mod.txt",
                    "equipments={mod_base={is_archetype=yes} mod_child={archetype=mod_base} mod_fallback={archetype=mod_base}} duplicate_archetypes={mod_copy={archetype=mod_base}}",
                ),
                (
                    "common/terrain/mod.txt",
                    "categories={mod_ground={movement_cost=1}}",
                ),
                (
                    "common/technologies/mod.txt",
                    "technologies={direct={parent=mod_parent enable_equipments={mod_child}} fallback={enable_equipments={mod_fallback}} duplicates={parent=mod_parent enable_equipments={mod_copy mod_child}} forced={parent=mod_parent enable_equipments={mod_child} force_use_small_tech_layout=yes}}",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n direct_desc:0 \"Wrong token description\"\n mod_child_desc:0 \"Child description\"\n M0_mod_child_desc:0 \"Country child description\"\n mod_base_desc:0 \"Archetype description\"\n M0_mod_base_desc:0 \"Country archetype description\"\n mod_parent_desc:0 \"Parent description\"\n",
                ),
            ],
            &[],
        );
        let context = Context::load(Arc::new(catalogue), Kind::Technology).unwrap();
        assert_eq!(
            context.description("direct").as_deref(),
            Some("Child description")
        );
        assert_eq!(
            context.description_key("direct", Some("M0")).unwrap(),
            "M0_mod_child_desc"
        );
        assert_eq!(
            context.description("forced").as_deref(),
            Some("Parent description")
        );
        // Two total children use the parent name. The one non-duplicate
        // description gate still selects the FIRST child, the generated copy.
        assert_eq!(context.technology_root("duplicates").unwrap(), "mod_parent");
        assert_eq!(
            context.description("duplicates").as_deref(),
            Some("mod_copy_desc")
        );
        assert_eq!(
            context.description("fallback").as_deref(),
            Some("Archetype description")
        );
        assert_eq!(
            context.description_key("fallback", Some("M0")).unwrap(),
            "M0_mod_base_desc"
        );
        assert_eq!(
            context.description_key("duplicates", Some("M0")).unwrap(),
            "mod_copy_desc"
        );
    }

    #[test]
    fn nuke_artwork_uses_the_recorded_gui_sprite_and_authored_frame() {
        let sheet = Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![11, 0, 0, 255, 22, 0, 0, 255],
        };
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/mod.gui",
                    "guiTypes={containerWindowType={name=topbar containerWindowType={name=nukes iconType={name=authored_stockpile spriteType=GFX_mod_stockpile frame=2}}}}",
                ),
                (
                    "interface/mod.gfx",
                    "spriteTypes={spriteType={name=GFX_mod_stockpile textureFile=gfx/mod/sheet.png noOfFrames=2}}",
                ),
            ],
            &[("gfx/mod/sheet.png", png_bytes(&sheet))],
        );
        assert_eq!(
            from_url(nuke_icon(&catalogue).unwrap().as_deref().unwrap()).pixels[0],
            22
        );
        let (catalogue, _folders) = texture_catalogue(
            &[(
                "interface/mod.gui",
                "guiTypes={containerWindowType={name=topbar containerWindowType={name=nukes iconType={name=first} iconType={name=second}}}}",
            )],
            &[],
        );
        assert!(
            nuke_icon(&catalogue)
                .unwrap_err()
                .to_string()
                .contains("2 icons")
        );
    }

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
        // build_geometry@14222CB90 clips its fractional width at the queried
        // integer extent instead of squeezing the complete pitch into it.
        assert_eq!(&frame.pixels[..8], [0, 20, 30, 200, 1, 20, 30, 200]);
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
                ("common/technologies/mod.txt", "technologies={mod_tech={}}"),
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
        assert_eq!(
            warnings
                .iter()
                .filter(|warning| **warning == format!("mod_tech: icon unavailable: {reason}"))
                .count(),
            1,
            "{warnings:?}"
        );
    }
}
