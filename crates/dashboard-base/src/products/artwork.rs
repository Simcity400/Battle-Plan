//! Localised names, descriptions and sprite thumbnails for focus and technology
//! tokens, read through the source's shared content catalogue.
//! [`sprites`] draws exact authored sprite keys with the same renderer.

pub mod sprites;

use super::ContentOptions;
use crate::{
    Artwork, Source,
    catalogue::{self, CachedResult, Catalogue},
    invalid,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// The fallback sprite for a focus without an icon.
const UNKNOWN_FOCUS_ICON: &str = installed_content_focus::focus::UNKNOWN_ICON;

/// Longest side of returned product artwork previews.
pub const ICON_EDGE: u32 = 96;

pub use crate::catalogue::MAX_RETAINED_ARTWORK_BYTES;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Technology,
}

struct TechnologyName {
    presentation: Result<installed_content_core::technology::PresentationName, String>,
    children: std::sync::OnceLock<Result<Vec<String>, String>>,
}

#[cfg(test)]
fn technology_name(
    presentation: &installed_content_core::technology::PresentationName,
    world: Result<&crate::triggers::World<'_>, String>,
    database: Option<&Arc<installed_content_core::equipment::Database>>,
) -> Result<String, String> {
    let mut children = technology_children(presentation, world, database)?;
    Ok(if children.len() == 1 {
        children.remove(0)
    } else {
        presentation.template.clone()
    })
}

fn technology_children(
    presentation: &installed_content_core::technology::PresentationName,
    world: Result<&crate::triggers::World<'_>, String>,
    database: Option<&Arc<installed_content_core::equipment::Database>>,
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
            // Normalized equipment category bit 9 and feature 0x21's DLC bit 7
            // use the recorded DLC roster shared with the content-only has_dlc
            // evaluator, never a row-hour scope.
            let enabled = world
                .as_ref()
                .map(|world| {
                    engine_dlc::dlc_feature_state::NAME_MASKS
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

#[derive(Default)]
struct PreparedContent {
    focus: CachedResult<Preparation>,
    technology: CachedResult<Preparation>,
}

struct Preparation {
    definitions: Arc<BTreeMap<String, installed_content_focus::focus::FocusDefinition>>,
    point_days: Option<f64>,
    technology_names: Arc<BTreeMap<String, TechnologyName>>,
    inventory: Vec<String>,
    preparation_warnings: Vec<installed_content_core::Diagnostic>,
    warnings: Vec<String>,
    /// Each technology's research folders in authored order.
    technology_folders: Arc<BTreeMap<String, Vec<String>>>,
    /// The research folders `common/technology_tags` defines, in content order.
    folder_definitions: Arc<Vec<installed_content_core::technology::FolderDefinition>>,
    /// Notes from reading research folders; only research replies carry them.
    folder_warnings: Vec<String>,
}

impl Preparation {
    fn read(layers: &installed_content_core::ResolvedLayers, kind: Kind) -> io::Result<Self> {
        let mut warnings = Vec::new();
        let mut technology_folders = BTreeMap::new();
        let mut folder_definitions = Vec::new();
        let mut folder_warnings = Vec::new();
        let (definitions, point_days, technology_names, inventory, preparation_warnings) =
            match kind {
                Kind::Focus => {
                    let (definitions, definition_warnings) = installed_content_focus::focus::read(layers)?;
                    warnings.extend(
                        definition_warnings
                            .iter()
                            .map(|(_, message)| message.clone()),
                    );
                    let inventory = definitions.keys().cloned().collect();
                    (
                        definitions,
                        installed_content_focus::focus::point_days(layers)?,
                        BTreeMap::new(),
                        inventory,
                        definition_warnings,
                    )
                }
                Kind::Technology => {
                    let read = installed_content_core::technology::Database::read(layers)?;
                    technology_folders = read.folders;
                    // Folders only colour and group research; a failure to read
                    // them never withholds technology text or artwork.
                    match installed_content_core::technology::read_folders(layers) {
                        Ok(folders) => {
                            folder_definitions = folders.folders;
                            folder_warnings
                                .extend(folders.warnings.into_iter().map(|(_, message)| message));
                        }
                        Err(error) => {
                            folder_warnings.push(format!("Research folders unavailable: {error}"));
                        }
                    }
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
        Ok(Self {
            technology_folders: Arc::new(technology_folders),
            folder_definitions: Arc::new(folder_definitions),
            folder_warnings,
            definitions: Arc::new(definitions),
            point_days,
            technology_names: Arc::new(technology_names),
            inventory,
            preparation_warnings,
            warnings,
        })
    }
}

pub struct Context {
    kind: Kind,
    technology_names: Arc<BTreeMap<String, TechnologyName>>,
    /// The source's shared localisation and sprites for these folders.
    catalogue: Arc<Catalogue>,
    artwork_cache: BTreeMap<String, catalogue::Drawn<(Artwork, Vec<String>)>>,
    artwork_bytes: usize,
    icon_cache: BTreeMap<String, catalogue::Drawn<Result<Option<String>, String>>>,
    definitions: Arc<BTreeMap<String, installed_content_focus::focus::FocusDefinition>>,
    focus_text_needs_frame: OnceLock<bool>,
    point_days: Option<f64>,
    warnings: Vec<String>,
    inventory: Vec<String>,
    preparation_warnings: Vec<installed_content_core::Diagnostic>,
    technology_folders: Arc<BTreeMap<String, Vec<String>>>,
    folder_definitions: Arc<Vec<installed_content_core::technology::FolderDefinition>>,
    folder_warnings: Vec<String>,
    /// Each folder's tab availability, decided once from content.
    folder_availability: OnceLock<Vec<Option<bool>>>,
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

/// A focus country's canonical slot at `frame`'s hour, None when absent.
fn focus_slot(frame: &replay_frame::frame::DecodedFrame, tag: &str) -> Result<Option<u32>, String> {
    let countries = frame
        .complete::<replay_data::rows::tables::Countries>()
        .map_err(|error| error.to_string())?;
    let links = frame
        .complete::<replay_data::rows::tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    match crate::metadata::tagged_country(countries, links, tag) {
        crate::metadata::TaggedCountry::Found(_, slot) => Ok(Some(slot)),
        crate::metadata::TaggedCountry::Absent => Ok(None),
        crate::metadata::TaggedCountry::Unresolved => {
            Err("focus country identity is unresolved".into())
        }
    }
}

/// Resolved artwork and, when refused, its feed-local reason.
pub type ResolvedArt = (Artwork, Option<String>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FocusProjection {
    Full,
    Text,
    Titles,
    Display,
    Icons,
}

/// Immutable scoped inputs for one accepted hour. Each expansion still owns
/// its fresh caller scopes, random counters and variable session.
pub struct FocusFrame<'a> {
    pub frame: &'a replay_frame::frame::DecodedFrame,
    catalogue: &'a Catalogue,
    text: OnceLock<Result<crate::scripted_text::ScriptedText<'a>, String>>,
    /// Each recorded input reread this hour, read once however many
    /// countries' evaluations read it.
    rereads:
        std::cell::RefCell<std::collections::HashMap<crate::triggers::inputs::Key, Option<u64>>>,
    #[cfg(any(test, feature = "test-support"))]
    pub text_builds: std::cell::Cell<usize>,
}

impl<'a> FocusFrame<'a> {
    pub fn new(frame: &'a replay_frame::frame::DecodedFrame, catalogue: &'a Catalogue) -> Self {
        Self {
            frame,
            catalogue,
            text: OnceLock::new(),
            rereads: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            text_builds: std::cell::Cell::new(0),
        }
    }

    /// The digest a recorded query reads this hour ([`crate::triggers::inputs`]).
    pub fn reread(&self, key: &crate::triggers::inputs::Key) -> Option<u64> {
        if let Some(read) = self.rereads.borrow().get(key) {
            return *read;
        }
        let read = self.read_now(key);
        self.rereads.borrow_mut().insert(key.clone(), read);
        read
    }

    fn read_now(&self, key: &crate::triggers::inputs::Key) -> Option<u64> {
        match key {
            crate::triggers::inputs::Key::FocusTag(tag) => Some(crate::triggers::inputs::digest(
                &focus_slot(self.frame, tag),
            )),
            _ => self.text().ok()?.reread(key),
        }
    }

    fn text(&self) -> Result<&crate::scripted_text::ScriptedText<'a>, String> {
        let catalogue = self.catalogue;
        let frame = self.frame;
        self.text
            .get_or_init(|| {
                #[cfg(any(test, feature = "test-support"))]
                self.text_builds.set(self.text_builds.get() + 1);
                catalogue.scripted_content().map(|content| {
                    crate::triggers::inputs::unrecorded(|| {
                        crate::scripted_text::ScriptedText::new(
                            frame,
                            &catalogue.localisation,
                            content,
                            0,
                        )
                    })
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl FocusProjection {
    fn icons(self) -> bool {
        matches!(self, Self::Full | Self::Display | Self::Icons)
    }

    fn descriptions(self) -> bool {
        matches!(self, Self::Full | Self::Text)
    }
}

fn focus_icons_need_frame(definition: &installed_content_focus::focus::FocusDefinition) -> bool {
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
}

fn focus_needs_frame(
    catalogue: &Catalogue,
    definition: &installed_content_focus::focus::FocusDefinition,
    include_descriptions: bool,
) -> bool {
    focus_icons_need_frame(definition)
        || [&definition.localisation, &definition.description]
            .into_iter()
            .take(if include_descriptions { 2 } else { 1 })
            .any(|key| {
                catalogue
                    .localisation
                    .localise(key.as_bytes())
                    .is_ok_and(|text| text.contains('['))
            })
}

impl Context {
    pub fn load(catalogue: Arc<Catalogue>, kind: Kind) -> io::Result<Self> {
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".into()];
        warnings.extend(catalogue.verification.iter().cloned());
        let layers = &catalogue.content.layers;
        let prepared = layers.derived::<PreparedContent>();
        let slot = match kind {
            Kind::Focus => &prepared.focus,
            Kind::Technology => &prepared.technology,
        };
        let prepared = slot
            .get_or_init(|| Preparation::read(layers, kind))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::Interrupted {
                    error
                } else {
                    invalid(error.to_string())
                }
            })?;
        let definitions = Arc::clone(&prepared.definitions);
        let technology_names = Arc::clone(&prepared.technology_names);
        let point_days = prepared.point_days;
        let inventory = prepared.inventory.clone();
        let preparation_warnings = prepared.preparation_warnings.clone();
        let technology_folders = Arc::clone(&prepared.technology_folders);
        let folder_definitions = Arc::clone(&prepared.folder_definitions);
        let folder_warnings = prepared.folder_warnings.clone();
        warnings.extend(prepared.warnings.iter().cloned());
        warnings.extend(catalogue.warnings.iter().cloned());
        Ok(Self {
            kind,
            technology_names,
            catalogue,
            artwork_cache: BTreeMap::new(),
            artwork_bytes: 0,
            icon_cache: BTreeMap::new(),
            definitions,
            focus_text_needs_frame: OnceLock::new(),
            point_days,
            warnings,
            inventory,
            preparation_warnings,
            technology_folders,
            folder_definitions,
            folder_warnings,
            folder_availability: OnceLock::new(),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn content_loads(catalogue: &Catalogue) -> usize {
        let prepared = catalogue.content.layers.derived::<PreparedContent>();
        prepared.focus.loads() + prepared.technology.loads() + catalogue.content_loads()
    }

    /// The research folders in content order with their localized names, the
    /// folder each token is shown in ([`super::research::shown_folders`]) and
    /// named notes for what could not be resolved. A folder without an
    /// `available` trigger is always available; a trigger is decided from the
    /// recorded content and DLC alone, else its availability stays unknown.
    pub fn research_folders(
        &self,
        tokens: &BTreeSet<&str>,
    ) -> (
        Vec<crate::ResearchFolder>,
        Vec<crate::ResearchTechnologyFolder>,
        Vec<String>,
    ) {
        let availability = self.folder_availability.get_or_init(|| {
            let world = self
                .catalogue
                .scripted_content()
                .and_then(|content| content.unrecorded_trigger_world());
            self.folder_definitions
                .iter()
                .map(|folder| match &folder.available {
                    None => Some(true),
                    Some(trigger) => world.as_ref().ok().and_then(|world| {
                        world
                            .evaluate(trigger, crate::triggers::Scopes::empty())
                            .decided()
                            .ok()
                    }),
                })
                .collect()
        });
        let definitions: Vec<(&str, Option<bool>)> = self
            .folder_definitions
            .iter()
            .zip(availability)
            .map(|(folder, available)| (folder.key.as_str(), *available))
            .collect();
        let (shown, notes) = super::research::shown_folders(
            &definitions,
            &self.technology_folders,
            tokens.iter().copied(),
        );
        let folders = self
            .folder_definitions
            .iter()
            .map(|folder| crate::ResearchFolder {
                key: folder.key.clone(),
                name: self.localise(&folder.key),
            })
            .collect();
        let mut warnings = self.folder_warnings.clone();
        warnings.extend(notes);
        (folders, shown, warnings)
    }

    /// Localized presentation text; absent keys stay unavailable. Defined empty
    /// strings and authored text identical to its key remain valid.
    pub fn localise(&self, key: &str) -> Option<String> {
        if !self.catalogue.has_presentation_key(key) {
            return None;
        }
        if matches!(self.kind, Kind::Technology) {
            return self.catalogue.localised(key, self.view());
        }
        let (text, findings) = catalogue::localised_details(
            &self.catalogue.localisation,
            key,
            &[],
            self.view(),
            dashboard_core::findings::Kind::Offline,
        );
        for finding in findings {
            self.catalogue.note(finding);
        }
        text
    }

    /// Only country-independent focus text belongs in a history reply. Scoped
    /// strings wait for their recorded hour without producing an offline refusal.
    fn static_focus_text(&self, key: &str) -> Option<String> {
        if !self.catalogue.has_presentation_key(key) {
            return None;
        }
        let raw = self.catalogue.localisation.localise(key.as_bytes()).ok()?;
        if raw.contains('[') {
            return None;
        }
        catalogue::plain_text(&raw).ok()
    }

    /// Focus names and descriptions expand scoped text; full technology names
    /// are plain.
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

    /// Descriptions use the owner's own tag, never the dynamic original tag
    /// used by names. A single non-duplicate child selects the first admitted
    /// child, then its archetype if neither child key exists; otherwise the
    /// template's nonempty parent/id supplies the key.
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
            // Descriptions use the country-specific localisation suffix.
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
                    .ok_or_else(|| format!("technology {token}: first equipment description archetype is unavailable"))?;
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

    pub fn description_is_defined(&self, token: &str) -> bool {
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
        Option<&installed_content_gfx::gfx::SpriteType>,
        Vec<dashboard_core::findings::Finding>,
    )> {
        use dashboard_core::findings::{Finding, Kind as FindingKind};
        let selected = match self.kind {
            // Technology badges use the _medium suffix.
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

    fn artwork(
        &self,
        token: &str,
    ) -> io::Result<(Artwork, Vec<dashboard_core::findings::Finding>)> {
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

    pub fn tokens(&self) -> (&[String], &[installed_content_core::Diagnostic]) {
        (&self.inventory, &self.preparation_warnings)
    }

    /// The same per-token preparation for the reply and the offline check.
    pub fn prepare(&self, token: &str) -> (Artwork, Vec<dashboard_core::findings::Finding>) {
        use dashboard_core::findings::{Finding, Kind};
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
            .any(|definition| focus_needs_frame(&self.catalogue, definition, true))
    }

    /// A country-qualified sprite makes otherwise static display country-specific.
    pub fn focus_country_icon_override(&self, token: &str, tag: &str) -> bool {
        self.definitions.get(token).is_some_and(|definition| {
            !matches!(
                self.catalogue
                    .sprites
                    .find(&definition.sprite_overrides(token, Some(tag))[0]),
                Ok(None)
            )
        })
    }

    /// Titles and icons do not depend on unrequested description operands.
    pub fn requested_focus_display_needs_frame(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .filter_map(|token| self.definitions.get(token))
            .any(|definition| focus_needs_frame(&self.catalogue, definition, false))
    }

    /// Icon requests do not depend on any title or description operands.
    pub fn requested_focus_icons_need_frame(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .filter_map(|token| self.definitions.get(token))
            .any(focus_icons_need_frame)
    }

    pub fn requested_focus_title_needs_frame(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .filter_map(|token| self.definitions.get(token))
            .any(|definition| {
                self.catalogue
                    .localisation
                    .localise(definition.localisation.as_bytes())
                    .is_ok_and(|text| text.contains('['))
            })
    }

    pub fn requested_focus_text_needs_frame(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .filter_map(|token| self.definitions.get(token))
            .any(|definition| {
                [&definition.localisation, &definition.description]
                    .into_iter()
                    .any(|key| {
                        self.catalogue
                            .localisation
                            .localise(key.as_bytes())
                            .is_ok_and(|text| text.contains('['))
                    })
            })
    }

    /// Tree buttons stop at the first declared/refused ID override. Conditional
    /// definition icons cannot read hourly operands on that selection path.
    pub fn requested_tree_focus_needs_frame(
        &self,
        token: &str,
        tag: &str,
        projection: FocusProjection,
    ) -> bool {
        let tokens = [token.to_owned()];
        let icons = projection.icons()
            && self.definitions.get(token).is_some_and(|definition| {
                !definition
                    .sprite_overrides(token, Some(tag))
                    .iter()
                    .any(|name| !matches!(self.catalogue.sprites.find(name), Ok(None)))
                    && focus_icons_need_frame(definition)
            });
        icons
            || match projection {
                FocusProjection::Icons => false,
                FocusProjection::Full | FocusProjection::Text => {
                    self.requested_focus_text_needs_frame(&tokens)
                }
                FocusProjection::Display | FocusProjection::Titles => {
                    self.requested_focus_title_needs_frame(&tokens)
                }
            }
    }

    /// Whether focus text or icon cases need recorded country/hour operands.
    /// Dollar references have already expanded through the content localiser.
    pub fn focus_text_needs_frame(&self) -> bool {
        *self.focus_text_needs_frame.get_or_init(|| {
            self.definitions
                .values()
                .any(|definition| focus_needs_frame(&self.catalogue, definition, true))
        })
    }

    /// Event focus presentation resolves names and first-true icon cases at the row hour.
    pub fn focuses_at(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
    ) -> Vec<ResolvedArt> {
        self.focuses(requests, frame, false, FocusProjection::Full, |items| {
            items.collect()
        })
    }

    /// The caller admits each lazy item while the batch shares its text context.
    pub fn focus_tree_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, FocusProjection::Full, collect)
    }

    /// Resolve dated text without selecting, decoding or rendering sprites.
    pub fn focus_text_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, FocusProjection::Text, collect)
    }

    /// Row titles use their recorded scope without expanding hidden descriptions.
    pub fn focus_titles_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, FocusProjection::Titles, collect)
    }

    /// Visible row artwork selects icons and titles independently of details.
    pub fn focus_display_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, FocusProjection::Display, collect)
    }

    /// Select and render dated icons without reaching unrequested text.
    pub fn focus_icons_at<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        self.focuses(requests, frame, true, FocusProjection::Icons, collect)
    }

    fn focuses<T>(
        &mut self,
        requests: &[(&str, &str)],
        frame: &replay_frame::frame::DecodedFrame,
        tree: bool,
        projection: FocusProjection,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        let catalogue = Arc::clone(&self.catalogue);
        let inputs = FocusFrame::new(frame, &catalogue);
        self.focuses_in(requests, &inputs, tree, projection, collect)
    }

    pub fn focuses_in<T>(
        &mut self,
        requests: &[(&str, &str)],
        inputs: &FocusFrame<'_>,
        tree: bool,
        projection: FocusProjection,
        collect: impl FnOnce(&mut dyn Iterator<Item = ResolvedArt>) -> T,
    ) -> T {
        let frame = inputs.frame;
        if projection.icons() && requests.len() <= 8 {
            let sprites = requests
                .iter()
                .filter_map(|&(token, tag)| {
                    let definition = self.definitions.get(token)?;
                    let selected = if tree {
                        let mut selected = None;
                        for name in definition.sprite_overrides(token, Some(tag)) {
                            match self.catalogue.sprites.find(&name) {
                                Ok(Some(_)) => {
                                    selected = Some(name);
                                    break;
                                }
                                Ok(None) => {}
                                Err(_) => return None,
                            }
                        }
                        selected
                    } else {
                        None
                    };
                    let selected = selected.or_else(|| {
                        definition
                            .icon(None, |trigger| {
                                if trigger.is_empty() {
                                    Ok(true)
                                } else {
                                    Err("recorded context required".into())
                                }
                            })
                            .ok()
                            .map(|name| name.unwrap_or(UNKNOWN_FOCUS_ICON).to_owned())
                    })?;
                    self.catalogue
                        .sprites
                        .find(&selected)
                        .ok()
                        .flatten()
                        .filter(|sprite| sprite.texture.is_some())
                })
                .collect();
            self.prefetch_icons(sprites);
        }
        let catalogue = Arc::clone(&self.catalogue);
        let mut country_scopes = BTreeMap::new();
        let mut items = requests.iter().map(|&(token, tag)| {
            let country = country_scopes
                .entry(tag)
                .or_insert_with(|| {
                    crate::triggers::inputs::query(
                        || crate::triggers::inputs::Key::FocusTag(tag.to_owned()),
                        || focus_slot(frame, tag),
                    )
                })
                .clone();
            let context = || inputs.text();
            let selected = (|| -> Result<String, String> {
                if !projection.icons() {
                    return Ok(String::new());
                }
                let definition = self.definitions.get(token).ok_or_else(|| {
                    format!("focus {token}: definition absent from recorded content")
                })?;
                // Focus-tree buttons probe authored country/id and id overrides before
                // the definition's icon. Events use the definition.
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
                            .evaluate(trigger, evaluator.event_scopes(0).map_err(|why| why.to_string())?)
                            .decided()
                    })
                    .map(|name| name.unwrap_or(UNKNOWN_FOCUS_ICON).to_owned())
            })();
            let (icon, mut refusal) = if !projection.icons() {
                (None, None)
            } else { match selected {
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
            } };
            let mut art = Artwork {
                token: token.into(),
                name: None,
                description: None,
                base_duration_days: self.base_duration_days(token),
                icon,
            };
            if matches!(projection, FocusProjection::Icons) {
                return (art, refusal);
            }
            let localise = |key: &str| -> Result<String, String> {
                if !catalogue.has_presentation_key(key) {
                    return Err(format!("localisation key {key} is absent from the recorded language"));
                }
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
                // Focus names and descriptions seed a fresh scope with Clock seed
                // plus the byte-exact focus ID's FNV-1a hash, then the shared hash lanes.
                // The seed is an input only when the text draws from it.
                let seed = crate::triggers::inputs::unrecorded(|| crate::scripted_text::clock_seed(frame));
                scope.clock_seed = Some(seed);
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
                        "focus random_list: Clock seed is not recorded",
                    )
                }?;
                catalogue::plain_text(&expanded)
            };
            if let Some(definition) = self.definitions.get(token) {
                let description = projection.descriptions()
                    .then_some((&definition.description, &mut art.description));
                for (key, target) in std::iter::once((&definition.localisation, &mut art.name))
                    .chain(description)
                {
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
            } else if refusal.is_none() {
                // Text alone names the missing definition as an icon request does.
                refusal = Some(format!(
                    "focus {token}: definition absent from recorded content"
                ));
            }
            (art, refusal)
        });
        collect(&mut items)
    }

    fn prefetch_icons(&self, sprites: Vec<&installed_content_gfx::gfx::SpriteType>) {
        let Ok(decode) = self.catalogue.gui_decode.as_ref() else {
            return;
        };
        let layers = &self.catalogue.content.layers;
        let cache = layers.derived::<catalogue::SpriteImages>();
        let mut seen = BTreeSet::new();
        let sprites = sprites
            .into_iter()
            .filter(|sprite| {
                let key = catalogue::sprite_image_key(sprite, *decode, 0, ICON_EDGE);
                !cache.contains(&key) && seen.insert(key)
            })
            .map(|sprite| (sprite, 0))
            .collect::<Vec<_>>();
        if sprites.len() < 2 {
            return;
        }
        // A refused batch is retried by the ordinary reader so each requested
        // item retains its output and named refusal.
        let _ = catalogue::sprite_frames_png(layers, &sprites, *decode, ICON_EDGE);
    }

    fn cached_icon(&mut self, name: &str) -> io::Result<Option<String>> {
        if let Some(result) = self.icon_cache.get(name) {
            return result.get().clone().map_err(invalid);
        }
        let render = || -> io::Result<Option<String>> {
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
        };
        let drawn = catalogue::Drawn::new(|| render().map_err(|error| error.to_string()));
        let result = drawn.get().clone();
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
        self.icon_cache.insert(name.into(), drawn);
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

    /// Independent text for history replies; artwork is requested separately.
    pub fn text_catalogue(
        &self,
        tokens: BTreeSet<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<Artwork>, Vec<String>)> {
        let mut items = Vec::with_capacity(tokens.len());
        for token in tokens {
            crate::metadata::check_cancel(cancelled)?;
            let (name, description) = if self.kind == Kind::Focus {
                let definition = self.definitions.get(token);
                let name_key = definition.map_or(token, |definition| &definition.localisation);
                let description_key = self.description_key(token, None).ok();
                (
                    self.static_focus_text(name_key),
                    description_key
                        .as_deref()
                        .and_then(|key| self.static_focus_text(key)),
                )
            } else {
                (self.name(token), self.description(token))
            };
            items.push(Artwork {
                token: token.into(),
                name,
                description,
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
        if tokens.len() <= 8 {
            let sprites = tokens
                .iter()
                .filter(|token| !self.artwork_cache.contains_key(**token))
                .filter_map(|token| {
                    self.declared_icon(token)
                        .ok()
                        .and_then(|(sprite, _)| sprite)
                        .filter(|sprite| sprite.texture.is_some())
                })
                .collect();
            self.prefetch_icons(sprites);
        }
        let mut warnings = self.warnings.clone();
        let mut artwork = Vec::new();
        let mut bytes = 0;
        for token in tokens {
            if cancelled.load(Ordering::Acquire) {
                return Err(invalid("Widget request cancelled"));
            }
            if !self.artwork_cache.contains_key(token) {
                let drawn = catalogue::Drawn::new(|| {
                    let (art, findings) = self.prepare(token);
                    let messages: Vec<String> = findings
                        .into_iter()
                        .map(|finding| format!("{token}: {}", finding.message))
                        .collect();
                    (art, messages)
                });
                let (art, messages) = drawn.get();
                let size =
                    artwork_bytes(art) + messages.iter().map(|s| s.len() + 32).sum::<usize>();
                // Retain a multi-country dashboard's working set of compressed
                // icons so switching countries does not repeatedly decode them.
                if self.artwork_bytes + size > MAX_RETAINED_ARTWORK_BYTES {
                    self.artwork_cache.clear();
                    self.icon_cache.clear();
                    self.artwork_bytes = 0;
                }
                self.artwork_bytes += size;
                self.artwork_cache.insert(token.into(), drawn);
            }
            let (art, messages) = self.artwork_cache[token].get();
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

/// The stockpile indicator's authored image uses the topbar template's nukes
/// container. Its sole icon supplies the artwork through the authored sprite;
/// ambiguous content refuses by authored paths.
pub fn nuke_icon(catalogue: &Catalogue) -> io::Result<Option<String>> {
    use installed_content_core::{gui::native::Fields, gui::native::Kind};
    let (_, root) = installed_content_core::gui::native::kept_container_window(
        catalogue.guis()?,
        "topbar",
        &mut Vec::new(),
    )?;
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
    // A positive authored frame stores frame-1 before calculating the
    // sprite's frame offset.
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
    use installed_content_image::{Rgba8Image, native::Decode};

    #[test]
    fn focus_text_keeps_authored_titles_without_reaching_conditional_icons() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::{frame::DecodedFrame, frame::TableView};
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/national_focus/mod.txt",
                    "focus_tree={focus={id=mod_focus text=mod_title icon={GFX_unavailable={has_country_flag=mod_flag}}} focus={id=mod_empty text=empty_title}}",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n mod_title:0 \"Authored title\"\n mod_title_desc:0 \"Authored description\"\n empty_title:0 \"\"\n empty_title_desc:0 \"\"\n",
                ),
            ],
            &[],
        );
        let catalogue = Arc::new(catalogue);
        let mut text = Context::load(Arc::clone(&catalogue), Kind::Focus).unwrap();
        let images = Context::load(catalogue, Kind::Focus).unwrap();
        assert!(Arc::ptr_eq(&text.definitions, &images.definitions));
        assert!(!text.requested_focus_text_needs_frame(&["mod_focus".into()]));
        assert!(text.requested_focus_needs_frame(&["mod_focus".into()]));
        let frame = DecodedFrame {
            branch: 1,
            hour: 10,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Default::default(),
        };
        let resolved: Vec<_> = text.focus_text_at(
            &[("mod_focus", "synthetic"), ("mod_empty", "synthetic")],
            &frame,
            |items| items.collect(),
        );
        assert_eq!(resolved[0].0.name.as_deref(), Some("Authored title"));
        assert_eq!(
            resolved[0].0.description.as_deref(),
            Some("Authored description")
        );
        assert_eq!(resolved[1].0.name.as_deref(), Some(""));
        assert!(
            resolved
                .iter()
                .all(|(art, reason)| art.icon.is_none() && reason.is_none())
        );
        assert!(text.icon_cache.is_empty());
        let (static_text, _) = text
            .text_catalogue(
                BTreeSet::from(["mod_focus", "mod_empty"]),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(
            static_text
                .iter()
                .find(|item| item.token == "mod_focus")
                .unwrap()
                .name
                .as_deref(),
            Some("Authored title")
        );
        assert_eq!(
            static_text
                .iter()
                .find(|item| item.token == "mod_empty")
                .unwrap()
                .name
                .as_deref(),
            Some("")
        );
    }

    #[test]
    fn focus_display_projections_do_not_reach_unrequested_scoped_descriptions() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
        use replay_frame::{frame::DecodedFrame, frame::TableView};
        let (catalogue, _folders) = texture_catalogue(
            &[
                (
                    "common/national_focus/mod.txt",
                    "focus_tree={focus={id=plain text=plain_title icon=GFX_mod} focus={id=empty text=empty_title icon=GFX_mod} focus={id=equal text=equal_title icon=GFX_mod} focus={id=scoped text=scoped_title icon=GFX_mod}}",
                ),
                (
                    "interface/art.gfx",
                    "spriteTypes={spriteType={name=GFX_mod textureFile=gfx/mod/icon.png}}",
                ),
                (
                    "common/defines/mod.lua",
                    "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
                ),
                (
                    "common/ideologies/mod.txt",
                    "ideologies={mod_group={types={mod_type={}}}}",
                ),
                (
                    "common/scripted_localisation/mod.txt",
                    "defined_text={name=Broken text={trigger={has_government=mod_group} localization_key=desc_known}}",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n plain_title:0 \"Authored literal title\"\n plain_title_desc:0 \"[Broken]\"\n empty_title:0 \"\"\n empty_title_desc:0 \"[Broken]\"\n equal_title:0 \"equal_title\"\n equal_title_desc:0 \"[Broken]\"\n scoped_title:0 \"[ROOT.GetTag] title\"\n scoped_title_desc:0 \"[Broken]\"\n desc_known:0 \"Reached description\"\n",
                ),
            ],
            &[(
                "gfx/mod/icon.png",
                png_bytes(&Rgba8Image {
                    width: 1,
                    height: 1,
                    pixels: vec![23, 0, 0, 255],
                }),
            )],
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Focus).unwrap();
        let plain = ["plain".into(), "empty".into(), "equal".into()];
        assert!(!context.requested_focus_title_needs_frame(&plain));
        assert!(!context.requested_focus_display_needs_frame(&plain));
        assert!(context.requested_focus_text_needs_frame(&plain));
        assert!(context.requested_focus_needs_frame(&plain));
        assert!(context.requested_focus_title_needs_frame(&["scoped".into()]));
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 2,
                tag: b"M0".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 7,
                canonical_slot: 2,
            }]));
        *frame.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
            reason: "synthetic party read failed".into(),
        };
        let requests = [
            ("plain", "M0"),
            ("empty", "M0"),
            ("equal", "M0"),
            ("scoped", "M0"),
        ];
        let titles: Vec<_> = context.focus_titles_at(&requests, &frame, |items| items.collect());
        let expected = ["Authored literal title", "", "equal_title", "M0 title"];
        for ((art, reason), expected) in titles.iter().zip(expected) {
            assert_eq!(art.name.as_deref(), Some(expected));
            assert!(art.description.is_none());
            assert!(art.icon.is_none());
            assert!(reason.is_none(), "{reason:?}");
        }
        assert!(context.icon_cache.is_empty());
        let displayed: Vec<_> =
            context.focus_display_at(&requests, &frame, |items| items.collect());
        for ((art, reason), expected) in displayed.iter().zip(expected) {
            assert_eq!(art.name.as_deref(), Some(expected));
            assert!(art.description.is_none());
            assert_eq!(from_url(art.icon.as_deref().unwrap()).pixels[0], 23);
            assert!(reason.is_none(), "{reason:?}");
        }
        let detailed: Vec<_> = context.focus_text_at(&requests, &frame, |items| items.collect());
        for ((art, reason), expected) in detailed.iter().zip(expected) {
            assert_eq!(art.name.as_deref(), Some(expected));
            assert!(art.description.is_none());
            let reason = reason.as_deref().unwrap();
            assert!(reason.contains("_title_desc"), "{reason}");
            assert!(reason.contains("PoliticalParties"), "{reason}");
        }
    }

    #[test]
    fn focus_icons_evaluate_native_cases_and_preserve_caller_overrides() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use replay_data::{rows::CountryRow, rows::TableId, rows::TableRows, rows::tables};
        use replay_frame::{frame::DecodedFrame, frame::TableView};
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
                (
                    "localisation/mod_l_english.yml",
                    "\u{feff}l_english:\n scalar:0 \"Scalar\"\n scalar_desc:0 \"Scalar description\"\n constant:0 \"Constant\"\n constant_desc:0 \"Constant description\"\n iconless:0 \"Iconless\"\n iconless_desc:0 \"Iconless description\"\n",
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
        let icons: Vec<_> = context.focus_icons_at(
            &[
                ("scalar", "Z9"),
                ("alternate", "Z9"),
                ("conditional", "Z9"),
                ("iconless", "Z9"),
                ("absent", "Z9"),
            ],
            &frame,
            |items| items.collect(),
        );
        assert_eq!(icons[0].0.icon, tree.0.icon);
        assert!(icons[0].1.is_none());
        assert!(icons[1].0.icon.is_none());
        assert!(icons[1].1.as_deref().unwrap().contains("focus-window mode"));
        assert!(icons[2].0.icon.is_none());
        assert!(icons[2].1.is_some());
        assert_eq!(from_url(icons[3].0.icon.as_deref().unwrap()).pixels[0], 22);
        assert!(icons[3].1.is_none());
        assert!(icons[4].0.icon.is_none());
        assert!(icons[4].1.as_deref().unwrap().contains("definition absent"));
        for (art, reason) in icons {
            assert!(art.name.is_none());
            assert!(art.description.is_none());
            assert!(!reason.is_some_and(|reason| reason.contains("focus localisation")));
        }
    }

    #[test]
    fn focus_art_uses_the_row_country_and_only_refuses_a_missing_random_seed() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use replay_data::{
            rows::ClockRow, rows::CountryLinkRow, rows::CountryRow, rows::PoliticalPartyRow,
            rows::TableId, rows::TableRows, rows::tables,
        };
        use replay_frame::{frame::DecodedFrame, frame::TableView};
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
                    "\u{feff}l_english:\n scoped:0 \"[ROOT.GetTag] focus\"\n scoped_desc:0 \"For [FROM.GetTag]\"\n random:0 \"[Choice]\"\n random_desc:0 \"Plain description\"\n promoted:0 \"[ROOT.Choice]\"\n promoted_desc:0 \"\"\n A:0 \"A\"\n B:0 \"B\"\n",
                ),
            ],
            &[],
        );
        let mut context = Context::load(Arc::new(catalogue), Kind::Focus).unwrap();
        assert!(context.focus_text_needs_frame());
        let (static_text, _) = context
            .text_catalogue(
                BTreeSet::from(["scoped", "random", "promoted"]),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(
            static_text
                .iter()
                .all(|item| item.name.is_none() && item.icon.is_none())
        );
        assert_eq!(
            static_text
                .iter()
                .find(|item| item.token == "random")
                .unwrap()
                .description
                .as_deref(),
            Some("Plain description")
        );
        assert!(context.refusal_lines().is_empty());
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
                replay_data::rows::CountryCounterInputRow {
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
        assert!(
            result[1]
                .1
                .as_deref()
                .unwrap()
                .contains("Clock seed is not recorded")
        );
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
        let titles: Vec<_> =
            context.focus_titles_at(&[("random", "M0"), ("promoted", "M0")], &frame, |items| {
                items.collect()
            });
        assert_eq!(titles[0].0.name, seeded.0.name);
        assert_eq!(titles[1].0.name, result[2].0.name);
        assert!(
            titles
                .iter()
                .all(|(art, reason)| art.description.is_none() && reason.is_none())
        );
        // ROOT selects the country context; its entry fallback constructs a
        // fresh known 424242 scope.
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
                .any(|reason| reason.contains("Clock seed is not recorded"))
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
            rows.push(replay_data::rows::CountryCounterInputRow {
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // The selected DLC name maps to bit 7, which feature 0x21 tests.
        // No content key is hard-coded here.
        let feature_dlc = engine_dlc::dlc_feature_state::NAME_MASKS
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
        let presentation = installed_content_core::technology::Database::read(layers).unwrap();
        let registry = layers.key_names().unwrap();
        let again = layers.key_names().unwrap();
        assert!(Arc::ptr_eq(
            registry.equipment_database().unwrap(),
            again.equipment_database().unwrap()
        ));
        let triggers = installed_content_core::trigger_keys::TriggerNames::default();
        let aliases = Arc::new(installed_content_core::tag_aliases::TagAliases::default());
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
        // The name comes from the feature-mask mapping rather than content
        // importance. Feature 0x21 tests the same bit 7.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert_eq!(context.description("duplicates"), None);
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // Geometry clips its fractional width at the queried integer extent
        // instead of squeezing the complete pitch into it.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

// Focus artwork text and resource titles over a catalogue whose interface reads refuse.
#[cfg(test)]
mod catalogue_reads {
    use crate::catalogue::{Catalogue, tests::folder_catalogue};
    use installed_content_core::ResolvedLayers;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    #[test]
    fn refused_gui_reads_keep_focus_and_resource_titles_and_retry_on_a_fresh_stack() {
        use crate::products::{artwork, resources};
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use std::sync::atomic::AtomicBool;

        let gui_path = "interface/synthetic.gui";
        let (_, folder) = folder_catalogue(&[
            (
                gui_path,
                "guiTypes={containerWindowType={name=synthetic_window}}",
            ),
            (
                "common/national_focus/synthetic.txt",
                "focus_tree={focus={id=synthetic_focus text=synthetic_title}}",
            ),
            (
                "localisation/synthetic_l_english.yml",
                "\u{feff}l_english:\n synthetic_title:0 \"Localized focus\"\n synthetic_title_desc:0 \"Localized description\"\n PRODUCTION_MATERIALS_SYNTHETIC:0 \"Localized resource\"\n",
            ),
        ]);
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        std::fs::remove_file(folder.0.join(gui_path)).unwrap();
        // A reached GUI read will fail, but constructing a text catalogue must not read it.
        let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
        assert!(!catalogue.guis_loaded());
        let focus = artwork::Context::load(Arc::clone(&catalogue), artwork::Kind::Focus).unwrap();
        let (titles, _) = focus
            .text_catalogue(BTreeSet::from(["synthetic_focus"]), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(titles[0].name.as_deref(), Some("Localized focus"));
        assert_eq!(
            titles[0].description.as_deref(),
            Some("Localized description")
        );
        assert!(!catalogue.guis_loaded());

        let refused = catalogue.guis().unwrap_err().to_string();
        assert!(refused.contains(gui_path), "{refused}");
        assert_eq!(
            artwork::nuke_icon(&catalogue).unwrap_err().to_string(),
            refused
        );
        let mut cached = BTreeMap::new();
        let mut warnings = Vec::new();
        resources::resource_content(
            &resources::ContentView::of(&catalogue),
            &[("synthetic".into(), 1)],
            &mut cached,
            &mut warnings,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            cached[&("synthetic".into(), 1)].get().name.as_deref(),
            Some("Localized resource")
        );
        assert!(cached[&("synthetic".into(), 1)].get().icon.is_none());
        assert!(
            warnings.iter().any(|warning| warning.contains(gui_path)),
            "{warnings:?}"
        );

        std::fs::write(
            folder.0.join(gui_path),
            "guiTypes={containerWindowType={name=replacement_window}}",
        )
        .unwrap();
        assert_eq!(catalogue.guis().unwrap_err().to_string(), refused);
        let fresh_layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        let fresh = Catalogue::of_layers(fresh_layers, "english").unwrap();
        let first = fresh.guis().unwrap();
        assert!(first.template("replacement_window").is_some());
        assert!(first.template("synthetic_window").is_none());
        assert!(std::ptr::eq(first, fresh.guis().unwrap()));
    }
}

// A focus text reply over interface files with refusals keeps their summary.
#[cfg(test)]
mod interface_refusals {
    use crate::catalogue::{Catalogue, tests::folder_catalogue};
    use crate::products::artwork;
    use installed_content_core::ResolvedLayers;
    use std::collections::BTreeSet;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn d16_fresh_text_only_reply_keeps_interface_refusal_summary() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (_, folder) = folder_catalogue(&[
            (
                "interface/synthetic.gfx",
                "spriteTypes={spriteType={name=GFX_refused noOfFrames=-1}}",
            ),
            (
                "common/national_focus/synthetic.txt",
                "focus_tree={focus={id=synthetic}}",
            ),
            (
                "localisation/synthetic_l_english.yml",
                "\u{feff}l_english:\n synthetic:0 \"Synthetic focus\"\n synthetic_desc:0 \"Synthetic description\"\n",
            ),
        ]);
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
        let context = artwork::Context::load(catalogue, artwork::Kind::Focus).unwrap();
        let (items, warnings) = context
            .text_catalogue(BTreeSet::from(["synthetic"]), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(items[0].name.as_deref(), Some("Synthetic focus"));
        assert_eq!(
            items[0].description.as_deref(),
            Some("Synthetic description")
        );
        let summary = warnings
            .iter()
            .filter(|line| line.contains("refusals in its interface files"))
            .collect::<Vec<_>>();
        assert_eq!(summary.len(), 1, "{warnings:?}");
        assert!(summary[0].contains("1 refusals"), "{summary:?}");
        assert!(summary[0].contains("GFX_refused"), "{summary:?}");
        assert!(summary[0].contains("synthetic.gfx"), "{summary:?}");
    }
}
