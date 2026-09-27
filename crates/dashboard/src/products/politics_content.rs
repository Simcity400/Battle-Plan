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
        })
    }

    /// Localised text for a key, cleaned of colour codes; None when missing or
    /// when it needs runtime scope.
    pub fn localise(&self, key: &str) -> Option<String> {
        let localisation = &self.catalogue.localisation;
        catalogue::resolve_text(localisation.get(key)?, localisation).text()
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
