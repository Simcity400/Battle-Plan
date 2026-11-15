//! A stability line's name and group in the viewer's language, from the recorded content.

use super::BreakdownLine;
use dashboard_base::catalogue::View;
use dashboard_base::products::politics_content::{PoliticsContent, RecordedName};
use dashboard_text::idea_names::{IdeaCountry, IdeaName};
use installed_content_core::characters::BuiltTemplate;

/// A line's character as recorded: its script token, saved name and template.
pub struct RecordedCharacter<'a> {
    pub script: Option<&'a str>,
    pub saved: &'a str,
    pub template: Result<BuiltTemplate, String>,
}

fn built<'a>(character: Option<&'a RecordedCharacter>) -> Result<&'a BuiltTemplate, &'a str> {
    match character {
        Some(character) => character.template.as_ref().map_err(String::as_str),
        None => Err("the line's character is not recorded"),
    }
}

/// Which slot or kind of category a paired idea or advisor line belongs to: an advisor's role is
/// its recorded character's.
pub fn line_group(line: &BreakdownLine, content: &PoliticsContent, character: Option<&RecordedCharacter>) -> (Option<String>, Option<String>) {
    match line.kind.as_str() {
        "advisor" => (
            content
                .held_advisor(&line.identity, built(character))
                .ok()
                .flatten()
                .and_then(|role| content.localised(&role.slot, View::Plain)),
            None,
        ),
        "idea" => {
            let Ok(Some(idea)) = content.idea(&line.identity) else {
                return (None, None);
            };
            match content.ideas.tags.resolve(&idea.group) {
                Ok(Some(category)) if category.name == idea.group => (None, category.kind.clone()),
                Ok(_) => (content.localised(&idea.group, View::Plain), None),
                Err(gap) => {
                    content.catalogue.refuse(&gap.to_string(), &idea.group);
                    (None, None)
                }
            }
        }
        _ => (None, None),
    }
}

/// A line's name in the viewer's language: a character's saved name first, else the content name
/// of its script token. No country scope is evaluated, so a name holding a closed `[...]` is none.
pub fn line_name(line: &BreakdownLine, content: &PoliticsContent, character: Option<RecordedCharacter>, country: IdeaCountry<'_>) -> Option<String> {
    let localise = |key: &str| content.localised(key, View::Scripted);
    let expanded = |name: String| (!name.contains('[')).then_some(name);
    let character_name = |token: &str| content.authored_character_name(token);
    // A recorded name that cannot be resolved stays unknown (Some(None)): no authored name stands in.
    let recorded = || {
        let character = character.as_ref()?;
        match content.recorded_name(character.saved) {
            RecordedName::Named(name) => Some(Some(name)),
            RecordedName::Unresolved(_) => Some(None),
            RecordedName::None => character.script.and_then(character_name).map(Some),
        }
    };
    match line.kind.as_str() {
        "idea" => match content.idea_name(&line.identity, country) {
            Ok(IdeaName::Key(key)) => localise(&key),
            Ok(IdeaName::Generated(text)) => expanded(text),
            Err(_) => None,
        },
        "advisor" => recorded()
            .unwrap_or_else(|| {
                let role = content.held_advisor(&line.identity, built(character.as_ref())).ok()??;
                character_name(&role.character)
            })
            .and_then(expanded),
        "countryLeader" => recorded().flatten().and_then(expanded),
        "ideology" | "static" | "staticScaled" | "dynamicModifier" | "dynamicModifierValues" => localise(&line.identity),
        _ => None,
    }
}
