
use crate::catalogue::{self, View};
use content_reader_politics::ideas::{Idea, IdeaContent};
use content_reader_localisation::localisation::Argument;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdeaCountry<'a> {
    pub tag: &'a str,
    pub slot: u32,
    pub look: Option<CountryLook<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryLook<'a> {
    pub name_tag: &'a str,
    pub original_or_own_tag: Result<&'a str, &'a str>,
    pub cosmetic_tag: &'a str,
    pub civil_war: Result<bool, &'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdeaName {
    Key(String),
    Generated(String),
}

const ADVISOR_NAME_PATTERN: &str = "POLITICS_ADVISOR_NAME_PATTERN";

pub fn idea_name(
    ideas: &IdeaContent,
    names: &content_reader_politics::names::NameLists,
    localisation: &content_reader_localisation::localisation::Localisation,
    token: &str,
    country: IdeaCountry<'_>,
) -> Result<IdeaName, String> {
    let Some(idea) = ideas.idea(token).map_err(|gap| gap.to_string())? else {
        return Ok(IdeaName::Key(token.to_owned()));
    };
    let token = idea.token.as_str();
    if let Some(name) = idea.name.as_deref().filter(|name| !name.is_empty()) {
        return Ok(IdeaName::Key(name.to_owned()));
    }
    let Some(look) = country.look else {
        return Err(format!(
            "whether {} is a dynamic country (name tag of country appearance) is not recorded for this hour",
            country.tag
        ));
    };
    let civil_war = match (look.name_tag.is_empty(), look.civil_war) {
        (true, _) => false,
        (false, Ok(civil_war)) => civil_war,
        (false, Err(reason)) => {
            return Err(format!(
                "{} is a dynamic country, and whether it is in civil war is not known: {reason}",
                country.tag
            ));
        }
    };
    if !civil_war {
        let own = format!("{}_{token}", country.tag);
        if localisation.get(&own).is_some() {
            return Ok(IdeaName::Key(own));
        }
        if localisation.get(token).is_some() {
            return Ok(IdeaName::Key(token.to_owned()));
        }
    }
    if !idea.law && !idea.designer {
        let kind = match ideas.tags.resolve(&idea.group) {
            Ok(Some(category)) => category.kind.as_deref(),
            Ok(None) => {
                return Err(format!(
                    "idea {token}'s group {} is in no category content declares",
                    idea.group
                ));
            }
            Err(gap) => return Err(gap.to_string()),
        };
        if kind.is_none() && (!civil_war || look.cosmetic_tag.is_empty()) {
            return generated_idea_name(names, localisation, idea, country, look)
                .map(IdeaName::Generated);
        }
    }
    Ok(IdeaName::Key(token.to_owned()))
}

fn generated_idea_name(
    names: &content_reader_politics::names::NameLists,
    localisation: &content_reader_localisation::localisation::Localisation,
    idea: &Idea,
    country: IdeaCountry<'_>,
    look: CountryLook<'_>,
) -> Result<String, String> {
    let entry = match names.for_cosmetic(look.cosmetic_tag) {
        Some(entry) => entry,
        None => names.for_country("", look.original_or_own_tag.map_err(str::to_owned)?),
    };
    if let Some(unknown) = names.unknown.first() {
        return Err(format!(
            "the names a generated advisor name draws from are unknown: {unknown}"
        ));
    }
    let seed = idea
        .position
        .wrapping_add(country.slot as i32)
        .wrapping_add(idea.level);
    let Some((first, surname)) = names.draw(entry, seed) else {
        return Ok(String::new());
    };
    let text = |part: &str, bytes| {
        content_reader_parser::parser::text(bytes)
            .map_err(|refused| format!("the drawn {part} of a generated advisor name is {refused}"))
    };
    let (first, surname) = (text("name", first)?, text("surname", surname)?);
    catalogue::localised_with(
        localisation,
        ADVISOR_NAME_PATTERN,
        &[
            ("NAME", Argument::Text(first)),
            ("SURNAME", Argument::Text(surname)),
        ],
        View::Plain,
    )
}
