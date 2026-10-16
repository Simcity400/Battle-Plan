//! Country-specific idea naming, shared by the politics products and effect
//! descriptions: an idea's selected localisation key, or its generated advisor name.

use crate::catalogue::{self, View};
use installed_content::{
    ideas::{Idea, IdeaContent},
    localisation::Argument,
};

/// The country an idea is named for: its tag,
/// canonical slot (CountryLinks) and appearance, `None` when the hour does
/// not hold its CountryAppearance row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdeaCountry<'a> {
    pub tag: &'a str,
    pub slot: u32,
    pub look: Option<CountryLook<'a>>,
}

/// A country's CountryAppearance fields the naming reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryLook<'a> {
    /// The original tag of a dynamic country, empty for any other country.
    pub name_tag: &'a str,
    /// The positive original reference's
    /// bytes, else the own tag, or why the exact operand is unavailable.
    /// Generated advisors demand it only after the cosmetic names lookup misses.
    pub original_or_own_tag: Result<&'a str, &'a str>,
    /// The cosmetic tag.
    pub cosmetic_tag: &'a str,
    /// Whether a dynamic country is in civil war
    /// ([`crate::identity::civil_war`]), or why the hour cannot say; `Ok(false)` for
    /// any other country, whose name selection never needs this check.
    pub civil_war: Result<bool, &'a str>,
}

/// An idea's selected localisation key or generated name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdeaName {
    Key(String),
    Generated(String),
}

/// The localisation pattern for a generated advisor name.
const ADVISOR_NAME_PATTERN: &str = "POLITICS_ADVISOR_NAME_PATTERN";

/// An idea's name for a country: its selected localisation key, or a generated advisor name.
pub fn idea_name(
    ideas: &IdeaContent,
    names: &installed_content::names::NameLists,
    localisation: &installed_content::localisation::Localisation,
    token: &str,
    country: IdeaCountry<'_>,
) -> Result<IdeaName, String> {
    let Some(idea) = ideas.idea(token).map_err(|gap| gap.to_string())? else {
        // Without an idea definition, an advisor's idea is its character, and an unknown
        // token is copied.
        return Ok(IdeaName::Key(token.to_owned()));
    };
    // Use the selected definition's key, not the recorded case alias.
    let token = idea.token.as_str();
    // An empty override, including one beginning with a NUL terminator,
    // continues down the naming ladder.
    if let Some(name) = idea.name.as_deref().filter(|name| !name.is_empty()) {
        return Ok(IdeaName::Key(name.to_owned()));
    }
    let Some(look) = country.look else {
        return Err(format!(
            "whether {} is a dynamic country (CountryAppearance.name_tag) is not recorded for this hour",
            country.tag
        ));
    };
    // The recorded name tag distinguishes dynamic countries.
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
        // Resolve the category by its authored group name.
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

/// Draw a male first name and surname from the country's cosmetic names,
/// then its original-or-own tag's names, then the default entry. Seed the
/// draw with [`Idea::position`], the country's canonical slot and the idea's
/// `level`. Localise the pattern with `NAME` and `SURNAME` text arguments.
/// No draw gives empty text; unreadable drawn names retain their refusal.
fn generated_idea_name(
    names: &installed_content::names::NameLists,
    localisation: &installed_content::localisation::Localisation,
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
        installed_content::parser::text(bytes)
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
