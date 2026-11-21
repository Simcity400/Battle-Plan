
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    map::ProvinceDefinition,
    parser::native::{self, Shape},
    states,
    terrain::DefaultMap,
};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

#[derive(Clone, Debug, Default)]
pub struct Content {
    continent_by_state: BTreeMap<u32, Option<String>>,
}

impl Content {
    pub fn load(
        layers: &ResolvedLayers,
        default_map: &DefaultMap,
        provinces: &BTreeMap<u32, ProvinceDefinition>,
        defines: &crate::defines::Defines,
    ) -> io::Result<Self> {
        let continents = continent_names(&layers.read(&default_map.file("continent")?)?)?;
        let continent = |province: &u32| {
            let index = provinces.get(province)?.continent;
            continents.get(usize::from(index.checked_sub(1)?)).cloned()
        };
        Ok(Self {
            continent_by_state: states::read(layers, defines, provinces)?
                .0
                .definitions()
                .iter()
                .map(|(id, state)| (*id, state.provinces.iter().find_map(continent)))
                .collect(),
        })
    }
}

pub fn continent_names(input: &[u8]) -> io::Result<Vec<String>> {
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), field| {
            if field.keyword_is("continents") {
                Shape::RawList
            } else {
                Shape::Skip
            }
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("the continent file: {cause}")));
    }
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in read
        .items
        .iter()
        .filter(|item| item.keyword_is("continents"))
        .flat_map(|item| item.value.items().unwrap_or_default())
    {
        let name = entry
            .value
            .c_str()
            .map_err(|error| invalid(format!("the continent file: {error}")))?
            .to_ascii_lowercase();
        if name.is_empty() || !seen.insert(name.clone()) {
            return Err(invalid(
                "the continent file has an empty or duplicate continent",
            ));
        }
        names.push(name);
    }
    if !read.items.iter().any(|item| item.keyword_is("continents")) {
        return Err(invalid("the continent file has no continents list"));
    }
    Ok(names)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CountryContext {
    pub country_tag: String,
    pub faction_tags: Vec<String>,
    pub capital_continent: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[derive(replay_spool::Spooled)]
pub struct Prepared {
    slots: BTreeMap<u32, u32>,
    contexts: BTreeMap<u32, CountryContext>,
}

impl Prepared {
    pub fn heap(&self) -> usize {
        let text = |text: &String| std::mem::size_of::<String>() + text.len();
        self.slots.len() * 2 * std::mem::size_of::<u32>()
            + self
                .contexts
                .values()
                .map(|context| {
                    std::mem::size_of::<(u32, CountryContext)>()
                        + context.country_tag.len()
                        + context.faction_tags.iter().map(text).sum::<usize>()
                        + context.capital_continent.as_ref().map_or(0, String::len)
                })
                .sum::<usize>()
    }

    pub fn read(frame: &DecodedFrame, content: &Content) -> Result<Self, String> {
        let error = |error: io::Error| error.to_string();
        let links = frame.complete::<tables::CountryLinks>().map_err(error)?;
        let countries = frame.complete::<tables::Countries>().map_err(error)?;
        let politics = frame.complete::<tables::Politics>().map_err(error)?;
        let factions = frame.complete::<tables::Factions>().map_err(error)?;
        let members = frame.complete::<tables::FactionMembers>().map_err(error)?;
        let states = frame.complete::<tables::States>().map_err(error)?;

        let slots: BTreeMap<u32, u32> = links
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let slot = |raw: u32, what: &str| {
            slots
                .get(&raw)
                .copied()
                .ok_or_else(|| format!("{what} country {raw} has no CountryLinks row"))
        };
        let mut tags = BTreeMap::new();
        for country in countries {
            let tag = std::str::from_utf8(&country.tag)
                .map_err(|_| format!("country {} tag is not UTF-8", country.id))?;
            if tags
                .insert(country.canonical_slot, tag.to_owned())
                .is_some()
            {
                return Err(format!("duplicate country slot {}", country.canonical_slot));
            }
        }
        let mut capital_by_slot = BTreeMap::new();
        for row in politics {
            let capital = if row.original_capital != 0 {
                row.original_capital
            } else {
                row.capital
            };
            if capital_by_slot
                .insert(slot(row.country, "Politics")?, capital)
                .is_some()
            {
                return Err(format!("duplicate Politics for country {}", row.country));
            }
        }
        let state_ids: BTreeSet<u32> = states.iter().map(|row| row.id).collect();
        let faction_ids: BTreeSet<u64> = factions.iter().map(|row| row.id).collect();
        let mut rosters: BTreeMap<u64, Vec<(u32, u32)>> = BTreeMap::new();
        let mut membership = BTreeMap::new();
        for member in members {
            if !faction_ids.contains(&member.faction) {
                return Err(format!(
                    "FactionMembers faction {} is absent",
                    member.faction
                ));
            }
            let member_slot = slot(member.country, "FactionMembers")?;
            if membership.insert(member_slot, member.faction).is_some() {
                return Err(format!("country slot {member_slot} has two factions"));
            }
            rosters
                .entry(member.faction)
                .or_default()
                .push((member.ordinal, member_slot));
        }
        let mut faction_tags = BTreeMap::new();
        for (faction, mut roster) in rosters {
            roster.sort_by_key(|(ordinal, _)| *ordinal);
            let mut ordered = Vec::with_capacity(roster.len());
            for (expected, &(ordinal, member)) in roster.iter().enumerate() {
                if ordinal != expected as u32 {
                    return Err(format!("faction {faction} has an incomplete member order"));
                }
                ordered.push(
                    tags.get(&member)
                        .ok_or_else(|| format!("faction {faction} member slot {member} is absent"))?
                        .clone(),
                );
            }
            faction_tags.insert(faction, ordered);
        }
        let mut contexts = BTreeMap::new();
        for (country_slot, tag) in tags {
            let capital = *capital_by_slot
                .get(&country_slot)
                .ok_or_else(|| format!("country slot {country_slot} has no Politics row"))?;
            let capital_continent = if capital <= 0 || !state_ids.contains(&(capital as u32)) {
                None
            } else {
                content
                    .continent_by_state
                    .get(&(capital as u32))
                    .ok_or_else(|| format!("capital state {capital} is absent from content"))?
                    .clone()
            };
            contexts.insert(
                country_slot,
                CountryContext {
                    country_tag: tag,
                    faction_tags: membership
                        .get(&country_slot)
                        .and_then(|faction| faction_tags.get(faction))
                        .cloned()
                        .unwrap_or_default(),
                    capital_continent,
                },
            );
        }
        Ok(Self { slots, contexts })
    }

    pub fn context(&self, country_ref: u32) -> Result<Option<&CountryContext>, String> {
        if (country_ref as i32) <= 0 {
            return Ok(None);
        }
        let slot = self
            .slots
            .get(&country_ref)
            .ok_or_else(|| format!("country reference {country_ref} has no CountryLinks row"))?;
        self.contexts.get(slot).map(Some).ok_or_else(|| {
            format!("country reference {country_ref} resolves to absent slot {slot}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continent_names_fold_and_refuse_duplicates() {
        assert_eq!(
            continent_names(b"continents = { europe North_America }").unwrap(),
            ["europe", "north_america"]
        );
        assert!(continent_names(b"continents = { europe EUROPE }").is_err());
        assert!(continent_names(b"other = { }").is_err());
        assert_eq!(
            continent_names(b"@c = asia other = { europe } continents = x africa @c }").unwrap(),
            ["africa", "asia"]
        );
    }

    #[test]
    fn a_nonpositive_reference_visits_only_the_default_pool() {
        let prepared = Prepared {
            slots: [(5, 1)].into(),
            contexts: [(
                1,
                CountryContext {
                    country_tag: "GER".into(),
                    faction_tags: vec![],
                    capital_continent: Some("europe".into()),
                },
            )]
            .into(),
        };
        assert_eq!(prepared.context(0), Ok(None));
        assert_eq!(prepared.context(u32::MAX), Ok(None));
        assert_eq!(prepared.context(5).unwrap().unwrap().country_tag, "GER");
        assert!(prepared.context(6).is_err());
    }
}
