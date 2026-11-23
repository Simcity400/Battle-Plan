
use super::BreakdownLine;
use dashboard_base::catalogue::View;
use dashboard_base::products::politics_content::{PoliticsContent, RecordedName};
use dashboard_text::idea_names::{IdeaCountry, IdeaName};
use content_reader_key_names::characters::BuiltTemplate;

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

pub fn line_name(line: &BreakdownLine, content: &PoliticsContent, character: Option<RecordedCharacter>, country: IdeaCountry<'_>) -> Option<String> {
    let localise = |key: &str| content.localised(key, View::Scripted);
    let expanded = |name: String| (!name.contains('[')).then_some(name);
    let character_name = |token: &str| content.authored_character_name(token);
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
