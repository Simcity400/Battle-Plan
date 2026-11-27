
use super::ContentOptions;
use crate::findings::{Finding, Kind};
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue, View},
};
use content_reader_key_names::{characters::{AdvisorRole, Authored, BuiltTemplate, Characters, PORTRAIT_ROLES, PORTRAIT_SIZES}, ideologies::Ideologies, modifier_definitions::ModifierDefinitions};
use content_reader_parser::coverage::Gap;
use content_reader_politics::ideas::{Idea, IdeaContent};
use content_reader_military::leader_traits::LeaderTraits;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

pub use crate::idea_names::{CountryLook, IdeaCountry, IdeaName, idea_name};

const ICON_EDGE: u32 = 96;

pub fn recorded_name(
    localisation: &content_reader_localisation::localisation::Localisation,
    saved: &str,
) -> RecordedName {
    if saved.is_empty() {
        return RecordedName::None;
    }
    match catalogue::localised(localisation, saved, View::Plain) {
        Ok(name) => RecordedName::Named(name),
        Err(reason) => RecordedName::Unresolved(reason),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedName {
    Named(String),
    Unresolved(String),
    None,
}

type PortraitKey = (u32, Vec<u8>, u32, content_reader_layers::content::ArtworkMode);

pub struct PoliticsContent {
    pub catalogue: Arc<Catalogue>,
    pub ideas: Arc<IdeaContent>,
    pub characters: Arc<Characters>,
    pub traits: LeaderTraits,
    pub ideologies: Ideologies,
    pub modifier_definitions: ModifierDefinitions,
    pub names: content_reader_politics::names::NameLists,
    pub warnings: Vec<String>,
    artwork: Mutex<BTreeMap<String, catalogue::Drawn<(Artwork, Option<String>)>>>,
    portraits: Mutex<BTreeMap<PortraitKey, super::artwork::sprites::DrawnArt>>,
    portrait_thumbnails: Mutex<BTreeMap<PortraitKey, super::artwork::sprites::DrawnArt>>,
    power: OnceLock<Arc<PowerContent>>,
    power_result: catalogue::CachedResult<PowerContent>,
    power_requested: std::sync::atomic::AtomicBool,
    fixed_text: catalogue::KeyedResults<String, Option<String>>,
    idea_classes: catalogue::KeyedResults<String, Option<bool>>,
}

pub struct PowerContent {
    pub decisions: content_reader_politics::decisions::Database,
    pub abilities: content_reader_abilities::abilities::AbilitySprites,
    focuses: content_reader_focus::focus::Focuses,
    pub warnings: Vec<String>,
    pub unit_traits: content_reader_key_names::unit_leader::Traits,
    pub triggers: content_reader_script::trigger_keys::TriggerNames,
    pub aliases: Arc<content_reader_key_names::tag_aliases::TagAliases>,
    pub dlcs: std::collections::BTreeSet<String>,
}

impl PowerContent {
    pub fn national_focus(
        &self,
        id: &str,
    ) -> Result<Option<&content_reader_focus::focus::FocusDefinition>, &content_reader_parser::coverage::Gap>
    {
        self.focuses
            .get_in(content_reader_focus::focus::Database::National, id)
    }
}

type CachedPolitics = Option<(
    Arc<Catalogue>,
    Arc<catalogue::CachedResult<PoliticsContent>>,
)>;

pub fn politics_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<PoliticsContent>> {
    let catalogue = catalogue::catalogue(source, options)?;
    kept(source, catalogue)
}

pub fn recorded_politics_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<PoliticsContent>> {
    kept(source, catalogue::recorded_catalogue(source, options)?)
}

fn kept(source: &Source, catalogue: Arc<Catalogue>) -> io::Result<Arc<PoliticsContent>> {
    let result = {
        let state = source.state::<catalogue::ByCatalogue<CachedPolitics>>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let slot = state.of(&catalogue);
        if !slot
            .as_ref()
            .is_some_and(|(bound, _)| Arc::ptr_eq(bound, &catalogue))
        {
            *slot = Some((
                Arc::clone(&catalogue),
                Arc::new(catalogue::CachedResult::default()),
            ));
        }
        Arc::clone(&slot.as_ref().expect("politics content binding retained").1)
    };
    result.get_or_init(|| PoliticsContent::load(catalogue))
}

pub(crate) fn warm(
    source: &Source,
    catalogue: Arc<Catalogue>,
) -> Option<io::Result<Arc<PoliticsContent>>> {
    let result = {
        let state = source.state::<catalogue::ByCatalogue<CachedPolitics>>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let slot = state.of(&catalogue);
        if let Some((bound, result)) = slot.as_ref() {
            if !Arc::ptr_eq(bound, &catalogue) {
                return None;
            }
            Arc::clone(result)
        } else {
            let result = Arc::new(catalogue::CachedResult::default());
            *slot = Some((Arc::clone(&catalogue), Arc::clone(&result)));
            result
        }
    };
    Some(result.get_or_init(|| PoliticsContent::load(catalogue)))
}

#[cfg(test)]
pub(crate) fn held(source: &Source, catalogue: &Arc<Catalogue>) -> Option<Arc<PoliticsContent>> {
    let state = source.state::<catalogue::ByCatalogue<CachedPolitics>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let (bound, result) = state.of(catalogue).as_ref()?;
    Arc::ptr_eq(bound, catalogue)
        .then(|| result.get()?.ok())
        .flatten()
}

#[cfg(any(test, feature = "test-support"))]
pub fn content_loads(source: &Source) -> usize {
    let state = source.state::<catalogue::ByCatalogue<CachedPolitics>>();
    let state = state.lock().unwrap();
    state
        .both()
        .into_iter()
        .flatten()
        .map(|(_, result)| {
            let loads = result.loads();
            match result.get() {
                Some(Ok(content)) => loads + content.power_result.loads(),
                _ => loads,
            }
        })
        .sum()
}

fn settled<T>(read: std::thread::Result<io::Result<T>>) -> io::Result<T> {
    read.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

impl PoliticsContent {
    pub fn idea_is_law(&self, token: &str) -> io::Result<Arc<Option<bool>>> {
        self.idea_classes.get_or_init(&token.to_owned(), |token| {
            self.ideas
                .idea(token)
                .map(|idea| idea.map(|idea| idea.law))
                .map_err(|gap| content_reader_parser::fields::invalid(gap.to_string()))
        })
    }
    pub fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        Self::read(catalogue, content_reader_key_names::ideologies::read)
    }

    pub fn load_with_ideologies(
        catalogue: Arc<Catalogue>,
        ideologies: Ideologies,
    ) -> io::Result<Self> {
        Self::read(catalogue, |_| Ok((ideologies, Vec::new())))
    }

    fn read(
        catalogue: Arc<Catalogue>,
        ideologies: impl FnOnce(
            &content_reader_layers::ResolvedLayers,
        ) -> io::Result<(Ideologies, Vec<content_reader_parser::Diagnostic>)>,
    ) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let (ideologies, ideology_warnings) = ideologies(layers)?;
        let officers = catalogue.read_defines()?
            .strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
        let (read, traits, definitions, names) = content_reader_layers::parallel::scope(|threads| {
            let traits = content_reader_layers::parallel::spawn(threads, || {
                content_reader_military::leader_traits::read(layers)
            });
            let definitions = content_reader_layers::parallel::spawn(threads, || {
                content_reader_key_names::modifier_definitions::read(layers)
            });
            let names =
                content_reader_layers::parallel::spawn(threads, || content_reader_politics::names::read(layers));
            let read = (|| {
                let ideas = content_reader_politics::ideas::catalogue(layers, &officers)?;
                let characters = content_reader_politics::ideas::characters(layers, &officers)?;
                io::Result::Ok((ideas, characters))
            })();
            (read, traits.join(), definitions.join(), names.join())
        });
        let (ideas, characters) = read?;
        let (traits, trait_warnings) = settled(traits)?;
        let (modifier_definitions, definition_warnings) = settled(definitions)?;
        let names = settled(names)?;
        let mut warnings = catalogue.verification.clone();
        for list in [
            &ideas.warnings,
            &characters.warnings,
            &trait_warnings,
            &ideology_warnings,
            &definition_warnings,
        ] {
            warnings.extend(list.iter().map(|(_, message)| message.clone()));
        }
        Ok(Self {
            catalogue,
            ideas: Arc::clone(&ideas.ideas),
            characters: Arc::clone(&characters.characters),
            traits,
            ideologies,
            modifier_definitions,
            names,
            warnings,
            artwork: Mutex::new(BTreeMap::new()),
            portraits: Mutex::new(BTreeMap::new()),
            power: OnceLock::new(),
            power_result: Default::default(),
            power_requested: Default::default(),
            fixed_text: Default::default(),

            idea_classes: Default::default(),
            portrait_thumbnails: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn power_content(&self) -> Result<&PowerContent, String> {
        self.power_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.prepare_power_content()
    }

    pub fn prepare_power_content(&self) -> Result<&PowerContent, String> {
        let power = self
            .power_result
            .get_or_init(|| {
                let layers = &self.catalogue.content.layers;
                let (
                    decisions,
                    abilities,
                    focuses,
                    unit_traits,
                    scripted_triggers,
                    buildings,
                    resources,
                    aliases,
                ) = content_reader_layers::parallel::scope(|threads| {
                    use content_reader_layers::parallel::spawn;
                    let abilities = spawn(threads, || content_reader_abilities::abilities::read(layers));
                    let focuses = spawn(threads, || content_reader_focus::focus::read_national(layers));
                    let unit_traits = spawn(threads, || {
                        content_reader_key_names::unit_leader::Traits::registered(layers)
                    });
                    let scripted_triggers =
                        spawn(threads, || content_reader_key_names::scripted_triggers::read(layers));
                    let buildings = spawn(threads, || content_reader_key_names::buildings::read(layers));
                    let resources = spawn(threads, || content_reader_key_names::resources::read(layers));
                    let aliases = spawn(threads, || content_reader_key_names::tag_aliases::read(layers));
                    (
                        content_reader_politics::decisions::read(layers),
                        abilities.join(),
                        focuses.join(),
                        unit_traits.join(),
                        scripted_triggers.join(),
                        buildings.join(),
                        resources.join(),
                        aliases.join(),
                    )
                });
                let decisions = decisions?;
                let abilities = settled(abilities)?;
                let (focuses, focus_warnings) = settled(focuses)?;
                let unit_traits = settled(unit_traits)?;
                let triggers = content_reader_script::trigger_keys::TriggerNames::new(
                    settled(scripted_triggers)?,
                    content_reader_key_names::key_name_products::count_trigger_names(
                        &settled(buildings)?,
                        &self.ideologies,
                        &settled(resources)?.0,
                    ),
                );
                let aliases = Arc::new(settled(aliases)?);
                let dlcs = layers.dlc_names()?;
                let warnings = decisions
                    .notes
                    .iter()
                    .cloned()
                    .chain(focus_warnings.into_iter().map(|(_, message)| message))
                    .chain(
                        abilities
                            .warnings
                            .iter()
                            .map(|(_, message)| message.clone()),
                    )
                    .collect();
                Ok(PowerContent {
                    decisions,
                    abilities,
                    focuses,
                    warnings,
                    unit_traits,
                    triggers,
                    aliases,
                    dlcs,
                })
            })
            .map_err(|error| error.to_string())?;
        Ok(self.power.get_or_init(|| power).as_ref())
    }

    pub fn power_requested(&self) -> bool {
        self.power_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn power_warnings(&self) -> &[String] {
        if !self
            .power_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return &[];
        }
        self.power
            .get()
            .map_or(&[], |power| power.warnings.as_slice())
    }

    pub fn defines(&self, key: &str) -> bool {
        self.catalogue.localisation.get(key).is_some()
    }

    pub fn localise_in(
        &self,
        key: &str,
        scripted: &crate::scripted_text::ScriptedText<'_>,
    ) -> Option<String> {
        self.catalogue.localise_in(key, scripted)
    }

    pub fn localised(&self, key: &str, view: View) -> Option<String> {
        if view == View::Plain {
            return self
                .fixed_text
                .get_or_init(&key.to_owned(), |key| {
                    Ok(self.catalogue.localised(key, view))
                })
                .expect("fixed localisation has an optional result")
                .as_ref()
                .clone();
        }
        self.catalogue.localised(key, view)
    }

    pub fn recorded_name(&self, saved: &str) -> RecordedName {
        let name = recorded_name(&self.catalogue.localisation, saved);
        if let RecordedName::Unresolved(reason) = &name {
            self.catalogue.refuse(reason, saved);
        }
        name
    }

    fn refused(&self, gap: &Gap, token: &str) -> String {
        let reason = gap.to_string();
        self.catalogue.refuse(&reason, token);
        reason
    }

    pub fn idea(&self, token: &str) -> Result<Option<&Idea>, String> {
        self.ideas
            .idea(token)
            .map_err(|gap| self.refused(gap, token))
    }

    pub fn advisor(&self, idea_token: &str) -> Result<Option<&AdvisorRole>, String> {
        self.characters
            .advisor(idea_token)
            .map_err(|gap| self.refused(gap, idea_token))
    }

    pub fn held_advisor(
        &self,
        idea_token: &str,
        built: Result<&BuiltTemplate, &str>,
    ) -> Result<Option<&AdvisorRole>, String> {
        self.characters
            .held_advisor(idea_token, built)
            .inspect_err(|reason| self.catalogue.refuse(reason, idea_token))
    }

    pub fn offline_advisor(&self, idea_token: &str) -> Result<Option<&AdvisorRole>, String> {
        self.characters
            .held_advisor(
                idea_token,
                Err("recorded advisor holder is unavailable offline"),
            )
            .inspect_err(|reason| {
                let finding =
                    advisor_finding(&self.characters, idea_token, idea_token, reason, true, true);
                self.catalogue.refuse_finding(finding);
            })
    }

    pub fn sprite_png(&self, name: &str) -> io::Result<Option<String>> {
        self.sprite_png_sized(name, ICON_EDGE)
    }

    pub fn sprite_png_sized(&self, name: &str, edge: u32) -> io::Result<Option<String>> {
        sprite_png_in(&self.catalogue, name, edge)
    }

    fn portrait(&self, character: &str) -> Result<Option<&str>, Box<Finding>> {
        let Some(portraits) = self.characters.portraits(character).map_err(|gap| {
            Box::new(Finding::new(
                Kind::Refused,
                character,
                self.refused(gap, character),
            ))
        })?
        else {
            return Ok(None);
        };
        for role in PORTRAIT_ROLES {
            match portraits.get(&(role.to_owned(), PORTRAIT_SIZES[0].to_owned())) {
                Some(Authored::Asset(name)) => return Ok(Some(name)),
                Some(Authored::Ambiguous(names)) => {
                    let reason = format!(
                        "{character}: first small portrait ({role}) is ambiguous: {names:?}"
                    );
                    let finding = Finding::new(Kind::Ambiguous, character, reason);
                    self.catalogue.refuse_finding(finding.clone());
                    return Err(Box::new(finding));
                }
                Some(Authored::Unknown(cause)) => {
                    let reason =
                        format!("{character}: first small portrait ({role}) is unknown: {cause}");
                    let finding = Finding::new(Kind::Refused, character, reason);
                    self.catalogue.refuse_finding(finding.clone());
                    return Err(Box::new(finding));
                }
                Some(Authored::Missing) | None => {}
            }
        }
        Ok(None)
    }

    pub fn character_name(&self, script: Option<&str>, saved: &str) -> Option<String> {
        self.character_name_for(script, saved, None)
    }

    pub fn character_name_with_refusals(
        &self,
        script: Option<&str>,
        saved: &str,
        refusals: &Mutex<catalogue::Refusals>,
    ) -> Option<String> {
        self.character_name_for(script, saved, Some(refusals))
    }

    fn character_name_for(
        &self,
        script: Option<&str>,
        saved: &str,
        refusals: Option<&Mutex<catalogue::Refusals>>,
    ) -> Option<String> {
        match self.recorded_name(saved) {
            RecordedName::Named(name) => Some(name),
            RecordedName::Unresolved(reason) => {
                Self::note_name_refusal(refusals, &reason, saved);
                None
            }
            RecordedName::None => self.authored_character_name_for(script?, false, refusals),
        }
    }

    fn note_name_refusal(refusals: Option<&Mutex<catalogue::Refusals>>, reason: &str, token: &str) {
        if let Some(refusals) = refusals {
            refusals
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .note(reason, token);
        }
    }

    pub fn authored_character_name(&self, character: &str) -> Option<String> {
        self.authored_character_name_for(character, false, None)
    }

    fn authored_character_name_for(
        &self,
        character: &str,
        offline_holder: bool,
        refusals: Option<&Mutex<catalogue::Refusals>>,
    ) -> Option<String> {
        let name = self
            .characters
            .name(character)
            .map_err(|gap| {
                let reason = self.refused(gap, character);
                Self::note_name_refusal(refusals, &reason, character);
                reason
            })
            .ok()??;
        match name {
            Authored::Asset(key) => match refusals {
                Some(refusals) => {
                    self.catalogue
                        .localised_with_refusals(key, &[], View::Plain, refusals)
                }
                None => self.localised(key, View::Plain),
            },
            Authored::Missing => None,
            Authored::Unknown(cause) => {
                self.catalogue.refuse(cause, character);
                Self::note_name_refusal(refusals, cause, character);
                None
            }
            Authored::Ambiguous(keys) => {
                let message = format!(
                    "the character's alternatives name different keys ({})",
                    keys.iter().cloned().collect::<Vec<_>>().join(", ")
                );
                let finding = if offline_holder {
                    Finding::grouped(
                        Kind::Offline,
                        character,
                        "character name needs the recorded holder name; unavailable offline",
                        message,
                    )
                } else {
                    Finding::new(Kind::Ambiguous, character, message)
                };
                Self::note_name_refusal(refusals, &finding.message, &finding.subject);
                self.catalogue.refuse_finding(finding);
                None
            }
        }
    }

    pub fn idea_name(&self, token: &str, country: IdeaCountry<'_>) -> Result<IdeaName, String> {
        idea_name(
            &self.ideas,
            &self.names,
            &self.catalogue.localisation,
            token,
            country,
        )
        .inspect_err(|reason| self.catalogue.refuse(reason, token))
    }

    pub fn idea_name_offline(&self, token: &str) -> Result<IdeaName, String> {
        let Some(idea) = self.idea(token)? else {
            return Ok(IdeaName::Key(token.to_owned()));
        };
        let token = idea.token.as_str();
        if let Some(name) = idea.name.as_deref().filter(|name| !name.is_empty()) {
            return Ok(IdeaName::Key(name.to_owned()));
        }
        let reason = "idea name can use recorded country localisation and civil-war appearance; unavailable offline";
        self.catalogue
            .note(Finding::grouped(Kind::Offline, token, reason, reason));
        if self.defines(token) {
            return Ok(IdeaName::Key(token.to_owned()));
        }
        if !idea.law && !idea.designer {
            let refused = match self.ideas.tags.resolve(&idea.group) {
                Ok(Some(category)) if category.kind.is_none() => {
                    let reason = "generated advisor name needs the recorded country tag and canonical slot; unavailable offline";
                    self.catalogue.refuse_as(Kind::Offline, reason, token);
                    return Err(reason.into());
                }
                Ok(Some(_)) => None,
                Ok(None) => Some(format!(
                    "idea {token}'s group {} is in no category content declares",
                    idea.group
                )),
                Err(gap) => Some(gap.to_string()),
            };
            if let Some(reason) = refused {
                self.catalogue.refuse(&reason, token);
                return Err(reason);
            }
        }
        Ok(IdeaName::Key(token.to_owned()))
    }

    pub fn idea_description_key(&self, token: &str) -> Option<String> {
        let idea = self.idea(token).ok()?;
        let token = idea.map_or(token, |idea| idea.token.as_str());
        let own = format!("{token}_desc");
        if self.defines(&own) {
            return Some(own);
        }
        let named = format!("{}_desc", idea?.name.as_deref()?);
        self.defines(&named).then_some(named)
    }

    pub fn recorded_portrait(&self, kind: u32, identity: &[u8]) -> Result<String, String> {
        self.recorded_portrait_sized(kind, identity, ICON_EDGE)
    }

    pub fn recorded_portrait_sized(
        &self,
        kind: u32,
        identity: &[u8],
        edge: u32,
    ) -> Result<String, String> {
        match self.recorded_portrait_art(kind, identity, edge) {
            super::artwork::ArtResolution::Ready { url, .. } => Ok(url),
            other => Err(other
                .reason()
                .map_or_else(|| "The recorded portrait has no texture.".to_owned(), str::to_owned)),
        }
    }

    pub fn recorded_portrait_art(
        &self,
        kind: u32,
        identity: &[u8],
        edge: u32,
    ) -> super::artwork::ArtResolution {
        let refused = |reason: &str, construct: &str| super::artwork::ArtResolution::Refused {
            reason: reason.to_owned(),
            construct: construct.to_owned(),
            origin: None,
            file: None,
        };
        let Ok(text) = std::str::from_utf8(identity) else {
            return refused("Recorded portrait identity is not UTF-8", "portrait-non-utf8");
        };
        if edge == 0 {
            return refused("Portrait thumbnail edge must be positive", "portrait-edge");
        }
        let key = (
            kind,
            identity.to_vec(),
            edge,
            content_reader_layers::content::artwork_mode(),
        );
        let store = if edge == ICON_EDGE {
            &self.portraits
        } else {
            &self.portrait_thumbnails
        };
        super::artwork::sprites::keep_settled(store, key, || {
            super::artwork::sprites::drawn_art(
                || format!("portrait {text}"),
                &self.catalogue,
                |catalogue| super::artwork::resolve_portrait_entry(catalogue, kind, text, edge),
            )
        })
    }

    pub fn idea_artwork(
        &self,
        token: &str,
        character: Option<&str>,
        country: IdeaCountry<'_>,
    ) -> (Artwork, Option<String>) {
        self.idea_artwork_for(token, character, Some(country), None)
    }

    pub fn held_idea_artwork(
        &self,
        token: &str,
        character: Option<&str>,
        saved: &str,
        country: IdeaCountry<'_>,
    ) -> (Artwork, Option<String>) {
        self.idea_artwork_for(token, character, Some(country), Some(saved))
    }

    pub fn idea_named(
        &self,
        token: &str,
        character: Option<&str>,
        country: IdeaCountry<'_>,
    ) -> (Option<String>, Option<String>) {
        match self.idea_and_role(token) {
            Ok((idea, role)) => (
                self.idea_display_name(token, character, Some(country), None, idea, role)
                    .ok(),
                None,
            ),
            Err(reason) => (None, Some(format!("{token}: {reason}"))),
        }
    }

    pub fn idea_name_missing(
        &self,
        token: &str,
        character: Option<&str>,
        country: IdeaCountry<'_>,
        saved: Option<&str>,
    ) -> Option<String> {
        match self.idea_and_role(token) {
            Ok((idea, role)) => self
                .idea_display_name(token, character, Some(country), saved, idea, role)
                .err(),
            Err(reason) => saved
                .and_then(|saved| self.character_name(None, saved))
                .is_none()
                .then(|| format!("{token}: {reason}")),
        }
    }

    fn idea_and_role(&self, token: &str) -> Result<(Option<&Idea>, Option<&AdvisorRole>), String> {
        match (self.idea(token), self.advisor(token)) {
            (Ok(idea), Ok(role)) => Ok((idea, role)),
            (Ok(Some(idea)), Err(_)) => Ok((Some(idea), None)),
            (Err(reason), _) | (_, Err(reason)) => Err(reason),
        }
    }

    pub fn idea_artwork_offline(&self, token: &str) -> (Artwork, Option<String>) {
        self.idea_artwork_for(token, None, None, None)
    }

    fn idea_artwork_for(
        &self,
        token: &str,
        character: Option<&str>,
        country: Option<IdeaCountry<'_>>,
        saved: Option<&str>,
    ) -> (Artwork, Option<String>) {
        let key = format!(
            "{token}\u{0}{}\u{0}{country:?}\u{0}{saved:?}\u{0}{:?}",
            character.unwrap_or(""),
            content_reader_layers::content::artwork_mode(),
        );
        if let Some(cached) = self
            .artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
        {
            return cached.get().clone();
        }
        let (idea, role) = match self.idea_and_role(token) {
            Ok(found) => found,
            Err(reason) => {
                let art = Artwork {
                    token: token.to_owned(),
                    name: saved.and_then(|saved| self.character_name(None, saved)),
                    description: None,
                    base_duration_days: None,
                    icon: None,
                    sprite: None,
                };
                return (art, Some(format!("{token}: {reason}")));
            }
        };
        let drawn = catalogue::Drawn::new(|| {
            self.draw_idea_artwork(token, character, country, saved, idea, role)
        });
        let result = drawn.get().clone();
        self.artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key, drawn);
        result
    }

    fn shown_idea_icon(
        &self,
        token: &str,
        icon: &IdeaIcon,
    ) -> Result<Option<(String, String)>, Box<Finding>> {
        let recorded = match self.catalogue.recorded() {
            Some(recorded) if !content_reader_layers::content::is_strict() => recorded,
            _ => return idea_icon(&self.catalogue, token, icon),
        };
        let drawn = catalogue::draw(
            || format!("idea {token} icon"),
            &self.catalogue,
            |catalogue| match idea_icon(catalogue, token, icon) {
                Ok(Some(drawn)) => Ok(drawn),
                Ok(None) => Err(format!("{token}: no artwork")),
                Err(finding) => Err(finding.message),
            },
        );
        match drawn.get() {
            Ok(drawn) => Ok(Some(drawn.clone())),
            Err(_) => idea_icon(recorded, token, icon),
        }
    }

    fn draw_idea_artwork(
        &self,
        token: &str,
        character: Option<&str>,
        country: Option<IdeaCountry<'_>>,
        saved: Option<&str>,
        idea: Option<&Idea>,
        role: Option<&AdvisorRole>,
    ) -> (Artwork, Option<String>) {
        let advisor = character.or_else(|| role.map(|role| role.character.as_str()));
        let name = self
            .idea_display_name(token, character, country, saved, idea, role)
            .ok();
        let description = self
            .idea_description_key(token)
            .and_then(|key| self.localised(&key, View::Scripted));
        let icon = match idea {
            Some(idea) => self.shown_idea_icon(
                token,
                &IdeaIcon::Candidates {
                    names: idea.sprite_candidates(country.map(|country| country.tag)),
                    recorded_tag: country.is_some(),
                },
            ),
            None => match advisor
                .map(|character| self.portrait(character))
                .transpose()
            {
                Ok(Some(Some(portrait))) => {
                    self.shown_idea_icon(token, &IdeaIcon::Sprite(portrait.to_owned()))
                }
                Err(finding) => Err(finding),
                Ok(_) => {
                    self.shown_idea_icon(token, &IdeaIcon::Sprite(format!("GFX_idea_{token}")))
                }
            },
        };
        let (drawn, problem) = match icon {
            Ok(Some(drawn)) => (Some(drawn), None),
            Ok(None) => {
                let reason =
                    format!("{token}: no artwork in the recorded content; a placeholder is shown");
                self.catalogue
                    .note(Finding::new(Kind::Fallback, token, &reason));
                (None, Some(reason))
            }
            Err(finding) => {
                let reason = if finding.kind == Kind::Fallback {
                    finding.message.clone()
                } else {
                    format!("{token}: icon unavailable: {}", finding.message)
                };
                self.catalogue.note(*finding);
                (None, Some(reason))
            }
        };
        let (sprite, icon) = drawn.unzip();
        let art = Artwork {
            token: token.to_owned(),
            name,
            description,
            base_duration_days: None,
            icon,
            sprite,
        };
        (art, problem)
    }

    fn idea_display_name(
        &self,
        token: &str,
        character: Option<&str>,
        country: Option<IdeaCountry<'_>>,
        saved: Option<&str>,
        idea: Option<&Idea>,
        role: Option<&AdvisorRole>,
    ) -> Result<String, String> {
        let advisor = character.or_else(|| role.map(|role| role.character.as_str()));
        match saved.map(|saved| self.recorded_name(saved)) {
            Some(RecordedName::Named(name)) => Ok(name),
            Some(RecordedName::Unresolved(reason)) => Err(reason),
            Some(RecordedName::None) | None => match (idea, advisor) {
                (Some(_), _) | (None, None) => match country.map_or_else(
                    || self.idea_name_offline(token),
                    |country| self.idea_name(token, country),
                )? {
                    IdeaName::Key(key) => self.catalogue.localised_or_why(&key, View::Scripted),
                    IdeaName::Generated(text) => Ok(text),
                },
                (None, Some(character)) => self
                    .authored_character_name_for(character, country.is_none(), None)
                    .ok_or_else(|| format!("the name of advisor {character} is unknown")),
            },
        }
    }
}

enum IdeaIcon {
    Candidates {
        names: Vec<String>,
        recorded_tag: bool,
    },
    Sprite(String),
}

fn idea_icon(
    catalogue: &Catalogue,
    token: &str,
    icon: &IdeaIcon,
) -> Result<Option<(String, String)>, Box<Finding>> {
    let refused = |message: String| Box::new(Finding::new(Kind::Refused, token, message));
    let (names, recorded_tag) = match icon {
        IdeaIcon::Sprite(name) => {
            return sprite_png_in(catalogue, name, ICON_EDGE)
                .map(|png| png.map(|png| (name.clone(), png)))
                .map_err(|why| refused(why.to_string()));
        }
        IdeaIcon::Candidates {
            names,
            recorded_tag,
        } => (names, *recorded_tag),
    };
    for name in names {
        if catalogue.sprites.find(name).map_err(&refused)?.is_some() {
            return sprite_png_in(catalogue, name, ICON_EDGE)
                .map_err(|why| refused(why.to_string()))?
                .map(|png| Some((name.clone(), png)))
                .ok_or_else(|| refused(format!("sprite {name} has no readable texture")));
        }
    }
    if !recorded_tag {
        let reason = "idea icon needs the recorded holder tag after its static candidates; unavailable offline";
        return Err(Box::new(Finding::grouped(
            Kind::Offline,
            token,
            reason,
            reason,
        )));
    }
    Err(Box::new(Finding::new(
        Kind::Fallback,
        token,
        format!(
            "{token}: icon unavailable: none of {names:?} is declared in the recorded content"
        ),
    )))
}

pub fn sprite_png_in(catalogue: &Catalogue, name: &str, edge: u32) -> io::Result<Option<String>> {
    match super::artwork::resolve_sprite_or_texture(catalogue, name, edge) {
        super::artwork::ArtResolution::NoArtExpected => Ok(None),
        super::artwork::ArtResolution::Ready { url, .. } => Ok(Some(url)),
        super::artwork::ArtResolution::ConfirmedMissing {
            origin: None,
            file: None,
            ..
        } => Ok(None),
        other => other.into_io_result(),
    }
}

pub(crate) fn advisor_finding(
    characters: &Characters,
    token: &str,
    subject: &str,
    reason: &str,
    offline_holder: bool,
    holder_missing: bool,
) -> Finding {
    let ambiguous = characters.ambiguous_advisor(token);
    if offline_holder && ambiguous && characters.advisor(token).is_ok() {
        return Finding::grouped(
            Kind::Offline,
            subject,
            "advisor role needs its recorded holder template; unavailable offline",
            reason,
        );
    }
    let kind = if ambiguous {
        if holder_missing {
            Kind::Unsupported
        } else {
            Kind::Ambiguous
        }
    } else {
        Kind::Refused
    };
    Finding::new(kind, subject, reason)
}

#[cfg(test)]
mod idea_name_tests;

#[cfg(test)]
mod presentation_tests;

#[cfg(test)]
mod recorded_name_tests;
