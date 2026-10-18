//! The Politics tab's content, read once per catalogue: idea categories and
//! ideas, characters (advisor and leader roles, portraits), leader traits,
//! ideologies and scripted modifier definitions. Every widget that names an
//! idea, advisor, trait or ideology reads it here, so the files are parsed once.

use super::ContentOptions;
use crate::findings::{Finding, Kind};
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue, View},
};
use installed_content_core::{
    characters::AdvisorRole, characters::Authored, characters::BuiltTemplate,
    characters::Characters, characters::PORTRAIT_ROLES, characters::PORTRAIT_SIZES, coverage::Gap,
    ideas::Idea, ideas::IdeaContent, ideologies::Ideologies, leader_traits::LeaderTraits,
    modifier_definitions::ModifierDefinitions,
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};

pub use crate::idea_names::{CountryLook, IdeaCountry, IdeaName, idea_name};

/// Longest side of a returned idea icon or portrait.
const ICON_EDGE: u32 = 96;
/// Idea artwork kept per catalogue.
const MAX_CACHED_ARTWORK: usize = 2048;
/// Recorded portraits kept per catalogue.
const MAX_CACHED_PORTRAITS: usize = 512;

/// See [`PoliticsContent::recorded_name`].
pub fn recorded_name(
    localisation: &installed_content_localisation::localisation::Localisation,
    saved: &str,
) -> RecordedName {
    if saved.is_empty() {
        return RecordedName::None;
    }
    // Localise the saved name without a probe: an undefined name keeps its
    // own bytes, even when it is the script token. Show character names plain.
    match catalogue::localised(localisation, saved, View::Plain) {
        Ok(name) => RecordedName::Named(name),
        Err(reason) => RecordedName::Unresolved(reason),
    }
}

/// A recorded character name, as [`PoliticsContent::recorded_name`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedName {
    /// The name to show.
    Named(String),
    /// Why the recorded name cannot be shown: no other name stands in.
    Unresolved(String),
    /// No saved name recorded: the authored name stands.
    None,
}

/// A recorded portrait entry: its kind, identity and requested thumbnail edge.
type PortraitKey = (u32, Vec<u8>, u32);

pub struct PoliticsContent {
    pub catalogue: Arc<Catalogue>,
    pub ideas: IdeaContent,
    pub characters: Characters,
    pub traits: LeaderTraits,
    pub ideologies: Ideologies,
    pub modifier_definitions: ModifierDefinitions,
    /// The names database a generated advisor name draws from.
    pub names: installed_content_core::names::NameLists,
    /// Standing notes about the content: files left out, closed or ambiguous.
    pub warnings: Vec<String>,
    /// Artwork already built, by token, with why its icon is missing if it is.
    artwork: Mutex<BTreeMap<String, (Artwork, Option<String>)>>,
    /// Recorded portraits already loaded, by kind, identity and requested edge.
    portraits: Mutex<BTreeMap<PortraitKey, Result<String, String>>>,
    portrait_thumbnails: Mutex<BTreeMap<PortraitKey, Result<String, String>>>,
    // World borrows these facts for the event scope's lifetime. The shared
    // outcome owns loading/refusal policy; this stable Arc supplies that borrow.
    power: OnceLock<Arc<PowerContent>>,
    power_result: catalogue::CachedResult<PowerContent>,
    power_requested: std::sync::atomic::AtomicBool,
    fixed_text: catalogue::KeyedResults<String, Option<String>>,
    idea_classes: catalogue::KeyedResults<String, Option<bool>>,
}

/// Presentation definitions needed only by the power ledger. Reading is lazy
/// and shared by every country/widget using this catalogue.
pub struct PowerContent {
    pub decisions: installed_content_core::decisions::Database,
    pub abilities: installed_content_abilities::abilities::AbilitySprites,
    pub focuses: installed_content_focus::focus::Focuses,
    pub warnings: Vec<String>,
    pub unit_traits: installed_content_core::unit_leader::Traits,
    pub triggers: installed_content_core::trigger_keys::TriggerNames,
    pub aliases: Arc<installed_content_core::tag_aliases::TagAliases>,
    pub dlcs: std::collections::BTreeSet<String>,
}

type CachedPolitics = Option<(
    Arc<Catalogue>,
    Arc<catalogue::CachedResult<PoliticsContent>>,
)>;

/// Both outcomes are retained for this catalogue binding; changing folders or
/// language binds a new result. Cancellation remains retryable.
pub fn politics_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<PoliticsContent>> {
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<CachedPolitics>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state
        .as_ref()
        .is_some_and(|(bound, _)| Arc::ptr_eq(bound, &catalogue))
    {
        *state = Some((
            Arc::clone(&catalogue),
            Arc::new(catalogue::CachedResult::default()),
        ));
    }
    let result = Arc::clone(&state.as_ref().expect("politics content binding retained").1);
    drop(state);
    result.get_or_init(|| PoliticsContent::load(catalogue))
}

#[cfg(any(test, feature = "test-support"))]
pub fn content_loads(source: &Source) -> usize {
    source
        .state::<CachedPolitics>()
        .lock()
        .unwrap()
        .as_ref()
        .map_or(0, |(_, result)| {
            let loads = result.loads();
            match result.get_or_init(|| unreachable!("binding was loaded")) {
                Ok(content) => loads + content.power_result.loads(),
                Err(_) => loads,
            }
        })
}

impl PoliticsContent {
    pub fn idea_is_law(&self, token: &str) -> io::Result<Arc<Option<bool>>> {
        self.idea_classes.get_or_init(&token.to_owned(), |token| {
            self.ideas
                .idea(token)
                .map(|idea| idea.map(|idea| idea.law))
                .map_err(|gap| io::Error::other(gap.to_string()))
        })
    }
    #[cfg(test)]
    pub(crate) fn idea_class_loads(&self, token: &str) -> usize {
        self.idea_classes.loads(&token.to_owned())
    }
    pub fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        let (ideologies, ideology_warnings) =
            installed_content_core::ideologies::read(&catalogue.content.layers)?;
        Self::load_prepared(catalogue, ideologies, ideology_warnings)
    }

    /// Offline, the corpus ideologies reader owns the ideology notes.
    pub fn load_with_ideologies(
        catalogue: Arc<Catalogue>,
        ideologies: Ideologies,
    ) -> io::Result<Self> {
        Self::load_prepared(catalogue, ideologies, Vec::new())
    }

    fn load_prepared(
        catalogue: Arc<Catalogue>,
        ideologies: Ideologies,
        ideology_warnings: Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let mut warnings = catalogue.verification.clone();
        // The groups named by this define contain character advisor templates.
        // Read them as advisors rather than ideas.
        let officers = installed_content_core::defines::read(layers)?
            .strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
        let (ideas, advisors, idea_warnings) = installed_content_core::ideas::read(layers, &officers)?;
        let (characters, character_warnings) = installed_content_core::characters::read(layers, advisors)?;
        let (traits, trait_warnings) = installed_content_core::leader_traits::read(layers)?;
        let (modifier_definitions, definition_warnings) =
            installed_content_core::modifier_definitions::read(layers)?;
        let names = installed_content_core::names::read(layers)?;
        for list in [
            idea_warnings,
            character_warnings,
            trait_warnings,
            ideology_warnings,
            definition_warnings,
        ] {
            warnings.extend(list.into_iter().map(|(_, message)| message));
        }
        Ok(Self {
            catalogue,
            ideas,
            characters,
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

    /// Resolve fixed databases during preparation configuration without making
    /// their diagnostics reachable before a ledger item demands them.
    pub fn prepare_power_content(&self) -> Result<&PowerContent, String> {
        let power = self
            .power_result
            .get_or_init(|| {
                let layers = &self.catalogue.content.layers;
                let decisions = installed_content_core::decisions::read(layers)?;
                let abilities = installed_content_abilities::abilities::read(layers)?;
                let (focuses, focus_warnings) = installed_content_focus::focus::read(layers)?;
                let unit_traits = installed_content_core::unit_leader::Traits::registered(layers)?;
                let triggers = installed_content_core::trigger_keys::TriggerNames::new(
                    installed_content_core::scripted_triggers::read(layers)?,
                    installed_content_core::trigger_keys::CountTriggerNames::new(
                        &installed_content_core::buildings::read(layers)?,
                        &self.ideologies,
                        &installed_content_core::resources::read(layers)?.0,
                    ),
                );
                let aliases = Arc::new(installed_content_core::tag_aliases::read(layers)?);
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

    /// Reading diagnostics already requested by an item's presentation; merely
    /// reporting them never loads the optional power databases.
    /// Whether a ledger item demanded the power content
    /// ([`Self::power_content`]): only then are its diagnostics reachable
    /// ([`Self::power_warnings`]), and never unreachable again.
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

    /// Whether localisation defines a key: only for naming and description
    /// ladders, never to hide a selected key.
    pub fn defines(&self, key: &str) -> bool {
        self.catalogue.localisation.get(key).is_some()
    }

    /// [`Catalogue::localise_in`]: a Scripted view evaluated for a country.
    pub fn localise_in(
        &self,
        key: &str,
        scripted: &crate::scripted_text::ScriptedText<'_>,
    ) -> Option<String> {
        self.catalogue.localise_in(key, scripted)
    }

    /// [`Catalogue::localised`]: a key the content lacks is its own bytes.
    pub fn localised(&self, key: &str, view: View) -> Option<String> {
        if view == View::Plain {
            // Only fixed content keys enter this slot. Recorded names and
            // scoped/scripted evaluation keep their production path below.
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

    /// What a recorded character's saved name says: it is
    /// localised, a name localisation does not define being its own bytes;
    /// text that cannot be shown is unresolved, the reason noted in the
    /// catalogue.
    pub fn recorded_name(&self, saved: &str) -> RecordedName {
        let name = recorded_name(&self.catalogue.localisation, saved);
        if let RecordedName::Unresolved(reason) = &name {
            self.catalogue.refuse(reason, saved);
        }
        name
    }

    /// A content lookup's gap, noted in the catalogue by token.
    fn refused(&self, gap: &Gap, token: &str) -> String {
        let reason = gap.to_string();
        self.catalogue.refuse(&reason, token);
        reason
    }

    /// The idea a token names: `Ok(None)` when content defines none, the
    /// reason (noted by token) when a file that could define it stops short.
    pub fn idea(&self, token: &str) -> Result<Option<&Idea>, String> {
        self.ideas
            .idea(token)
            .map_err(|gap| self.refused(gap, token))
    }

    /// The advisor role an idea token appoints, as [`Self::idea`] answers.
    pub fn advisor(&self, idea_token: &str) -> Result<Option<&AdvisorRole>, String> {
        self.characters
            .advisor(idea_token)
            .map_err(|gap| self.refused(gap, idea_token))
    }

    /// The advisor role an idea token appoints for a recorded holder
    /// ([`Characters::held_advisor`]); a refusal is noted by token.
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

    /// A sprite's first frame as a PNG data URL, or None when the content has no
    /// such sprite or texture. A name that is not a sprite and not a `GFX_` name is
    /// a texture path, as authored portraits may give one.
    pub fn sprite_png(&self, name: &str) -> io::Result<Option<String>> {
        self.sprite_png_sized(name, ICON_EDGE)
    }

    pub fn sprite_png_sized(&self, name: &str, edge: u32) -> io::Result<Option<String>> {
        let Some(sprite) = self.catalogue.sprites.find(name).map_err(crate::invalid)? else {
            if name.starts_with("GFX_") || name.is_empty() {
                return Ok(None);
            }
            let path = name
                .split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            return catalogue::texture_png(
                &self.catalogue.content.layers,
                &path,
                *self
                    .catalogue
                    .gui_decode
                    .as_ref()
                    .map_err(|why| crate::invalid(why.clone()))?,
                1,
                edge,
            )
            .map(Some);
        };
        if sprite.texture.is_none() {
            return Ok(None);
        }
        catalogue::sprite_frame_png(
            &self.catalogue.content.layers,
            sprite,
            *self
                .catalogue
                .gui_decode
                .as_ref()
                .map_err(|why| crate::invalid(why.clone()))?,
            0,
            edge,
        )
        .map(Some)
    }

    /// The first small portrait in native role order. An
    /// unknown higher role stops the search; a lower one cannot stand in.
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

    /// The first declared sprite, with a refused declaration ending the search.
    fn first_sprite_png(
        &self,
        names: &[String],
        token: &str,
        recorded_tag: bool,
    ) -> Result<Option<String>, Box<Finding>> {
        let refused = |message: String| Box::new(Finding::new(Kind::Refused, token, message));
        for name in names {
            if self
                .catalogue
                .sprites
                .find(name)
                .map_err(&refused)?
                .is_some()
            {
                return self
                    .sprite_png(name)
                    .map_err(|why| refused(why.to_string()))?
                    .map(Some)
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
        // Use the shared fallback sprite after all authored candidates miss.
        const UNKNOWN: &str = "GFX_idea_unknown";
        self.sprite_png(UNKNOWN)
            .map_err(|why| refused(format!("idea fallback sprite {UNKNOWN} is unreadable: {why}")))?
            .map(Some)
            .ok_or_else(|| Box::new(Finding::new(Kind::Fallback, token, format!("{token}: icon unavailable: idea fallback sprite {UNKNOWN} is undeclared or has no texture; a placeholder is shown"))))
    }

    /// A recorded character's name: the saved name first (a rename shows its
    /// current name), then the authored name of its script token; None when the
    /// saved name cannot be shown as plain text or nothing names the character.
    pub fn character_name(&self, script: Option<&str>, saved: &str) -> Option<String> {
        self.character_name_for(script, saved, None)
    }

    /// The same native name selection, retaining this caller's refusals.
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

    /// A content character's authored name, localised plain as its text
    /// boxes show it; None when content names none,
    /// or when its alternatives name different keys, which is noted by
    /// character.
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

    /// The key selected for
    /// an idea token and a country, in its order:
    /// 1. the idea's `name` when set, without asking whether
    ///    localisation defines it;
    /// 2. unless the dynamic country is in civil war ([`super::identity::civil_war`]),
    ///    `TAG_token` when localisation defines it
    ///    else the token when defined;
    /// 3. for an idea whose group is neither a law nor a designer group
    ///    in a category of type 0, a
    ///    generated advisor name ([`Self::generated_name`]), for a country
    ///    in civil war only when its cosmetic tag is empty;
    /// 4. else the token, which a missing key copies.
    ///
    /// Only the selected key is localised, so its refusal is
    /// final. `Err` names what cannot be decided (noted by token): a file
    /// that could define the idea stops short, a dynamic country whose
    /// civil-war inputs the hour does not hold, or a country whose
    /// CountryAppearance row the hour does not hold.
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

    /// [`Self::idea_name`] as far as content alone decides it: the name
    /// override, then the token's own key. The country's rungs are offline.
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

    /// Select an idea's description key: `token_desc` when localisation defines it, else the
    /// `name` override's `_desc` when that is defined, else none, and the
    /// game shows no description.
    pub fn idea_description_key(&self, token: &str) -> Option<String> {
        // Append to the definition's key. Localisation
        // itself remains case-sensitive.
        let idea = self.idea(token).ok()?;
        let token = idea.map_or(token, |idea| idea.token.as_str());
        let own = format!("{token}_desc");
        if self.defines(&own) {
            return Some(own);
        }
        let named = format!("{}_desc", idea?.name.as_deref()?);
        self.defines(&named).then_some(named)
    }

    /// A recorded portrait entry as a PNG data URL (kind 0 a sprite, kind 1 a
    /// texture path), or why it cannot be shown: a sprite content lacks or a
    /// texture that does not load is one the game replaces with a generated
    /// portrait.
    pub fn recorded_portrait(&self, kind: u32, identity: &[u8]) -> Result<String, String> {
        self.recorded_portrait_sized(kind, identity, ICON_EDGE)
    }

    /// The same recorded asset and fallback rules, at the longest edge the caller draws.
    /// The size is part of the cache identity, so a thumbnail never replaces larger art.
    pub fn recorded_portrait_sized(
        &self,
        kind: u32,
        identity: &[u8],
        edge: u32,
    ) -> Result<String, String> {
        let text = std::str::from_utf8(identity)
            .map_err(|_| "Recorded portrait identity is not UTF-8".to_owned())?;
        if edge == 0 {
            return Err("Portrait thumbnail edge must be positive".to_owned());
        }
        let key = (kind, identity.to_vec(), edge);
        // Small mounted-row thumbnails cannot evict Politics' full-size portraits.
        let store = if edge == ICON_EDGE {
            &self.portraits
        } else {
            &self.portrait_thumbnails
        };
        if let Some(cached) = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
        {
            return cached.clone();
        }
        let loaded = self.load_recorded_portrait(kind, text, edge);
        let mut cached = store.lock().unwrap_or_else(|error| error.into_inner());
        // Start over rather than grow without bound, as idea artwork does.
        if cached.len() >= MAX_CACHED_PORTRAITS {
            cached.clear();
        }
        cached.insert(key, loaded.clone());
        loaded
    }

    fn load_recorded_portrait(&self, kind: u32, text: &str, edge: u32) -> Result<String, String> {
        let loaded = match kind {
            0 => match self.catalogue.sprites.find(text) {
                Ok(Some(_)) => self.sprite_png_sized(text, edge),
                Err(why) => return Err(why),
                Ok(None) => {
                    return Err(format!(
                        "Content has no sprite {text}; the game generates this portrait."
                    ));
                }
            },
            1 => {
                let path = text
                    .split(['/', '\\'])
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("/");
                catalogue::texture_png(
                    &self.catalogue.content.layers,
                    &path,
                    *self.catalogue.gui_decode.as_ref().map_err(Clone::clone)?,
                    1,
                    edge,
                )
                .map(Some)
            }
            other => return Err(format!("Portrait kind {other} is not supported.")),
        };
        match loaded {
            Ok(Some(png)) => Ok(png),
            Ok(None) => Err(format!("{text} has no texture.")),
            Err(error) => Err(format!("{text} did not load: {error}")),
        }
    }

    /// Name, description and icon for an idea token, with why the icon is missing
    /// if it could not load (the text is kept either way). Advisors (whose ideas
    /// come from characters, not `common/ideas`) show the character's name and portrait.
    pub fn idea_artwork(
        &self,
        token: &str,
        character: Option<&str>,
        country: IdeaCountry<'_>,
    ) -> (Artwork, Option<String>) {
        self.idea_artwork_for(token, character, Some(country), None)
    }

    /// A recorded holder's saved name wins before authored alternatives are
    /// examined. This also avoids
    /// emitting an authored-name ambiguity for a name the recording settles.
    pub fn held_idea_artwork(
        &self,
        token: &str,
        character: Option<&str>,
        saved: &str,
        country: IdeaCountry<'_>,
    ) -> (Artwork, Option<String>) {
        self.idea_artwork_for(token, character, Some(country), Some(saved))
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
            "{token}\u{0}{}\u{0}{country:?}\u{0}{saved:?}",
            character.unwrap_or(""),
        );
        if let Some(cached) = self
            .artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
        {
            return cached.clone();
        }
        // Content that cannot say what the token is (a file that could
        // define it stops short) leaves the idea's artwork unknown.
        let (idea, role) = match (self.idea(token), self.advisor(token)) {
            (Ok(idea), Ok(role)) => (idea, role),
            (Err(reason), _) | (_, Err(reason)) => {
                let art = Artwork {
                    token: token.to_owned(),
                    // A content refusal cannot erase the holder's recorded
                    // name; it is independent of authored role/idea lookup.
                    name: saved.and_then(|saved| self.character_name(None, saved)),
                    description: None,
                    base_duration_days: None,
                    icon: None,
                };
                return (art, Some(format!("{token}: {reason}")));
            }
        };
        let advisor = character.or_else(|| role.map(|role| role.character.as_str()));
        // One source is selected, then localised: a key content lacks or a
        // refused text leaves the name unknown, and no other text stands in.
        let name = match saved.map(|saved| self.recorded_name(saved)) {
            Some(RecordedName::Named(name)) => Some(name),
            Some(RecordedName::Unresolved(_)) => None,
            Some(RecordedName::None) | None => match (idea, advisor) {
                (Some(_), _) | (None, None) => match country.map_or_else(
                    || self.idea_name_offline(token),
                    |country| self.idea_name(token, country),
                ) {
                    Ok(IdeaName::Key(key)) => self.localised(&key, View::Scripted),
                    Ok(IdeaName::Generated(text)) => Some(text),
                    Err(_) => None,
                },
                // An advisor's idea is its character.
                (None, Some(character)) => {
                    self.authored_character_name_for(character, country.is_none(), None)
                }
            },
        };
        // No defined description key means the
        // game shows nothing. Authored absence is not a product finding.
        let description = self
            .idea_description_key(token)
            .and_then(|key| self.localised(&key, View::Scripted));
        let icon = match idea {
            Some(idea) => self.first_sprite_png(
                &idea.sprite_candidates(country.map(|country| country.tag)),
                token,
                country.is_some(),
            ),
            None => match advisor
                .map(|character| self.portrait(character))
                .transpose()
            {
                Ok(Some(Some(portrait))) => self
                    .sprite_png(portrait)
                    .map_err(|why| Box::new(Finding::new(Kind::Refused, token, why.to_string()))),
                Err(finding) => Err(finding),
                Ok(_) => self
                    .sprite_png(&format!("GFX_idea_{token}"))
                    .map_err(|why| Box::new(Finding::new(Kind::Refused, token, why.to_string()))),
            },
        };
        let (icon, problem) = match icon {
            Ok(Some(icon)) => (Some(icon), None),
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
                // Portrait findings have already reached Refusals; leave their
                // original identity intact so the offline set deduplicates them.
                self.catalogue.note(*finding);
                (None, Some(reason))
            }
        };
        let art = Artwork {
            token: token.to_owned(),
            name,
            description,
            base_duration_days: None,
            icon,
        };
        let mut cached = self
            .artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A long campaign names a few hundred ideas; start over rather than grow without bound.
        if cached.len() >= MAX_CACHED_ARTWORK {
            cached.clear();
        }
        cached.insert(key, (art.clone(), problem.clone()));
        (art, problem)
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
mod recorded_name_tests;
