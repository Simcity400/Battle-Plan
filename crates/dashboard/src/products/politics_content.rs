//! The Politics tab's content, read once per catalogue: idea categories and
//! ideas, characters (advisor and leader roles, portraits), leader traits,
//! ideologies and scripted modifier definitions. Every widget that names an
//! idea, advisor, trait or ideology reads it here, so the files are parsed once.

use super::ContentOptions;
use crate::{
    Artwork, Source,
    catalogue::{self, Catalogue},
};
use installed_content::{
    characters::{Authored, Characters},
    ideas::{Idea, IdeaContent},
    ideologies::Ideologies,
    leader_traits::LeaderTrait,
    modifier_definitions::ModifierDefinition,
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};

/// Longest side of a returned idea icon or portrait.
const ICON_EDGE: u32 = 96;
/// Idea artwork kept per catalogue.
const MAX_CACHED_ARTWORK: usize = 2048;
/// Recorded portraits kept per catalogue.
const MAX_CACHED_PORTRAITS: usize = 512;

/// See [`PoliticsContent::recorded_name`].
pub fn recorded_name(
    localisation: &installed_content::localisation::Localisation,
    saved: &str,
    script: Option<&str>,
) -> RecordedName {
    if saved.is_empty() {
        return RecordedName::None;
    }
    // Only a key localisation lacks altogether is a literal name; a key whose text
    // cannot be expanded is a name that is not known.
    match localisation.get(saved) {
        Some(text) => match catalogue::resolve_text(text, localisation) {
            catalogue::Localised::Text(text) => RecordedName::Named(text),
            catalogue::Localised::NeedsScope | catalogue::Localised::Missing => {
                RecordedName::Unresolved
            }
        },
        None if Some(saved) == script => RecordedName::None,
        None => RecordedName::Named(catalogue::plain_text(saved)),
    }
}

/// A recorded character name, as [`PoliticsContent::recorded_name`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedName {
    /// The name to show.
    Named(String),
    /// A key whose text needs runtime scope: the name is not known.
    Unresolved,
    /// Nothing recorded beyond the script token: the authored name stands.
    None,
}

/// A recorded portrait entry: its kind and identity.
type PortraitKey = (u32, Vec<u8>);

pub struct PoliticsContent {
    pub catalogue: Arc<Catalogue>,
    pub ideas: IdeaContent,
    pub characters: Characters,
    pub traits: BTreeMap<String, LeaderTrait>,
    pub ideologies: Ideologies,
    pub modifier_definitions: BTreeMap<String, ModifierDefinition>,
    /// Standing notes about the content: files left out, closed or ambiguous.
    pub warnings: Vec<String>,
    /// Artwork already built, by token, with why its icon is missing if it is.
    artwork: Mutex<BTreeMap<String, (Artwork, Option<String>)>>,
    /// Recorded portraits already loaded, by kind and identity.
    portraits: Mutex<BTreeMap<PortraitKey, Result<String, String>>>,
}

/// The source's politics content for these folders, read on first use and
/// again only when the catalogue changes (other folders or language).
pub fn politics_content(
    source: &Source,
    options: &ContentOptions,
) -> io::Result<Arc<PoliticsContent>> {
    type Cached = Option<Arc<PoliticsContent>>;
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = state.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &catalogue)
    {
        return Ok(Arc::clone(cached));
    }
    let loaded = Arc::new(PoliticsContent::load(catalogue)?);
    *state = Some(Arc::clone(&loaded));
    Ok(loaded)
}

impl PoliticsContent {
    fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let mut warnings = catalogue.verification.clone();
        let (ideas, idea_warnings) = installed_content::ideas::read(layers)?;
        let (characters, character_warnings) = installed_content::characters::read(layers)?;
        let (traits, trait_warnings) = installed_content::leader_traits::read(layers)?;
        let (ideologies, ideology_warnings) = installed_content::ideologies::read(layers)?;
        let (modifier_definitions, definition_warnings) =
            installed_content::modifier_definitions::read(layers)?;
        for list in [
            idea_warnings,
            character_warnings,
            trait_warnings,
            ideology_warnings,
            definition_warnings,
        ] {
            warnings.extend(list);
        }
        Ok(Self {
            catalogue,
            ideas,
            characters,
            traits,
            ideologies,
            modifier_definitions,
            warnings,
            artwork: Mutex::new(BTreeMap::new()),
            portraits: Mutex::new(BTreeMap::new()),
        })
    }

    /// Whether localisation defines a key, as the game's existence check asks.
    pub fn defines(&self, key: &str) -> bool {
        self.catalogue.localisation.get(key).is_some()
    }

    /// Localised text for a key, cleaned of colour codes; None when missing or
    /// when it needs runtime scope.
    pub fn localise(&self, key: &str) -> Option<String> {
        let localisation = &self.catalogue.localisation;
        catalogue::resolve_text(localisation.get(key)?, localisation).text()
    }

    /// What a recorded character's saved name (CCharacter +0x48) says. A key is
    /// localised even when it equals the script token; text that needs runtime
    /// scope or does not expand is unresolved rather than shown as its key; a
    /// name localisation does not define is a literal, unless it is only the
    /// script token itself.
    pub fn recorded_name(&self, saved: &str, script: Option<&str>) -> RecordedName {
        recorded_name(&self.catalogue.localisation, saved, script)
    }

    pub fn idea(&self, token: &str) -> Option<&Idea> {
        self.ideas.ideas.get(token)
    }

    /// A sprite's first frame as a PNG data URL, or None when the content has no
    /// such sprite or texture. A name that is not a sprite and not a `GFX_` name is
    /// a texture path, as authored portraits may give one.
    fn sprite_png(&self, name: &str) -> io::Result<Option<String>> {
        let Some(sprite) = self.catalogue.sprites.sprites.get(name) else {
            if name.starts_with("GFX_") || name.is_empty() {
                return Ok(None);
            }
            let path = name
                .split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            return catalogue::texture_png(&self.catalogue.content.layers, &path, 1, ICON_EDGE)
                .map(Some);
        };
        let Some(path) = &sprite.texture else {
            return Ok(None);
        };
        catalogue::texture_png(
            &self.catalogue.content.layers,
            path,
            sprite.frames,
            ICON_EDGE,
        )
        .map(Some)
    }

    /// A character's portrait sprite as the political screen shows advisors:
    /// the small portrait of any category every alternative agrees on.
    fn portrait(&self, character: &str) -> Option<&str> {
        let portraits = self.characters.portraits.get(character)?;
        ["civilian", "army", "navy"].iter().find_map(|category| {
            match portraits.get(&((*category).to_owned(), "small".to_owned()))? {
                Authored::Asset(name) => Some(name.as_str()),
                _ => None,
            }
        })
    }

    /// A recorded character's name: the saved name first (a rename shows its
    /// current name), then the authored name of its script token; None when the
    /// saved name needs runtime scope or nothing names the character.
    pub fn character_name(&self, script: Option<&str>, saved: &str) -> Option<String> {
        match self.recorded_name(saved, script) {
            RecordedName::Named(name) => Some(name),
            RecordedName::Unresolved => None,
            RecordedName::None => match self.characters.names.get(script?)? {
                Authored::Asset(key) => self.localise(key),
                _ => None,
            },
        }
    }

    /// A recorded portrait entry as a PNG data URL (kind 0 a sprite, kind 1 a
    /// texture path), or why it cannot be shown: a sprite content lacks or a
    /// texture that does not load is one the game replaces with a generated
    /// portrait (`IsPortraitDescriptorAvailable@141388180`).
    pub fn recorded_portrait(&self, kind: u32, identity: &[u8]) -> Result<String, String> {
        let key = (kind, identity.to_vec());
        if let Some(cached) = self
            .portraits
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
        {
            return cached.clone();
        }
        let loaded = self.load_recorded_portrait(kind, identity);
        let mut cached = self
            .portraits
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Start over rather than grow without bound, as idea artwork does.
        if cached.len() >= MAX_CACHED_PORTRAITS {
            cached.clear();
        }
        cached.insert(key, loaded.clone());
        loaded
    }

    fn load_recorded_portrait(&self, kind: u32, identity: &[u8]) -> Result<String, String> {
        let text = String::from_utf8_lossy(identity);
        let loaded = match kind {
            0 => match self.catalogue.sprites.sprites.get(text.as_ref()) {
                Some(_) => self.sprite_png(&text),
                None => {
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
                catalogue::texture_png(&self.catalogue.content.layers, &path, 1, ICON_EDGE)
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
    pub fn idea_artwork(&self, token: &str, character: Option<&str>) -> (Artwork, Option<String>) {
        let key = format!("{token}\u{0}{}", character.unwrap_or(""));
        if let Some(cached) = self
            .artwork
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
        {
            return cached.clone();
        }
        let idea = self.idea(token);
        let advisor = character.or_else(|| {
            self.characters
                .advisors
                .get(token)
                .map(|role| role.character.as_str())
        });
        let character_name =
            advisor.and_then(|character| match self.characters.names.get(character) {
                Some(Authored::Asset(name)) => self.localise(name),
                _ => None,
            });
        let name = idea
            .and_then(|idea| self.localise(idea.name_key()))
            .or(character_name)
            .or_else(|| self.localise(token));
        // The description follows a name override; the token's own key is the fallback.
        let description = idea
            .and_then(|idea| self.localise(&idea.description_key()))
            .or_else(|| self.localise(&format!("{token}_desc")));
        let icon = match (idea, advisor.and_then(|character| self.portrait(character))) {
            (Some(idea), _) => self.sprite_png(&idea.sprite()),
            (None, Some(portrait)) => self.sprite_png(portrait),
            (None, None) => self.sprite_png(&format!("GFX_idea_{token}")),
        };
        let (icon, problem) = match icon {
            Ok(icon) => (icon, None),
            Err(error) => (None, Some(format!("{token}: icon unavailable: {error}"))),
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

#[cfg(test)]
mod recorded_name_tests {
    use super::{RecordedName, recorded_name};

    #[test]
    fn a_saved_name_is_localised_scoped_or_literal_as_recorded() {
        let localisation: installed_content::localisation::Localisation = [
            ("MOD_advisor".to_owned(), "Current Name".to_owned()),
            ("SCOPED_NAME".to_owned(), "[ROOT.GetLeader]".to_owned()),
            ("BROKEN_NAME".to_owned(), "$MISSING_REFERENCE$".to_owned()),
        ]
        .into_iter()
        .collect();
        let name = |saved: &str, script: Option<&str>| recorded_name(&localisation, saved, script);
        // A key is localised even when it is the script token.
        assert_eq!(
            name("MOD_advisor", Some("MOD_advisor")),
            RecordedName::Named("Current Name".into())
        );
        assert_eq!(
            name("SCOPED_NAME", Some("MOD_advisor")),
            RecordedName::Unresolved
        );
        // A key that does not expand is not a literal name either.
        assert_eq!(
            name("BROKEN_NAME", Some("MOD_advisor")),
            RecordedName::Unresolved
        );
        assert_eq!(
            name("Alex Reed", Some("MOD_advisor")),
            RecordedName::Named("Alex Reed".into())
        );
        assert_eq!(name("MOD_other", Some("MOD_other")), RecordedName::None);
        assert_eq!(name("", None), RecordedName::None);
    }
}
