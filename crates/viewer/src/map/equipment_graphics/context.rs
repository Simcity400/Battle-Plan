//! Country, faction and continent operands for authored equipment graphics.
//!
//! `1409F0BB0` visits the supplied country's tag, its faction members in the
//! faction's vector order, then the continent of its original (or current)
//! capital state. The variant owner's ideology and culture are separate inputs
//! to `equipment_graphics::Context`; they must not replace this country.
//!
//! `1406EA260` chooses CCountry+0x101C unless it is zero, then +0x1018.
//! `140AAE7A0` sets CStateTemplate+0x140 from the first authored province
//! whose continent entry is valid. `1409F0BB0` reads that entry's key.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{ResolvedLayers, map::ProvinceDefinition, parser, states, terrain::DefaultMap};
use replay::{frame::DecodedFrame, rows::tables};

/// Each authored state's continent: `None` is the native null-continent entry,
/// including a state with no province whose continent index is valid.
#[derive(Clone, Debug, Default)]
pub struct Content {
    continent_by_state: BTreeMap<u32, Option<String>>,
}

impl Content {
    pub fn load(
        layers: &ResolvedLayers,
        default_map: &DefaultMap,
        provinces: &BTreeMap<u32, ProvinceDefinition>,
    ) -> io::Result<Self> {
        let continents = continent_names(&layers.read(&default_map.file("continent")?)?)?;
        let continent = |province: &u32| {
            let index = provinces.get(province)?.continent;
            continents.get(usize::from(index.checked_sub(1)?)).cloned()
        };
        Ok(Self {
            // CStateTemplate::vf8 (140AAE7A0) walks the template's province
            // vector, not a sorted set or the province owning the capital VP.
            continent_by_state: states::read(layers)?
                .into_iter()
                .map(|(id, state)| (id, state.provinces.iter().find_map(continent)))
                .collect(),
        })
    }
}

/// `map/default.map`'s continent list, folded, in authored order.
pub fn continent_names(input: &[u8]) -> io::Result<Vec<String>> {
    let document = parser::parse(input)?;
    let entries = document
        .get(b"continents")
        .and_then(parser::Value::items)
        .ok_or_else(|| invalid("the continent file has no continents list"))?;
    let mut names = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for entry in entries {
        let name = entry
            .value
            .scalar_str()
            .filter(|_| entry.key.is_none())
            .ok_or_else(|| invalid("the continent file has a keyed or non-text continent"))?
            .to_ascii_lowercase();
        if name.is_empty() || !seen.insert(name.clone()) {
            return Err(invalid(
                "the continent file has an empty or duplicate continent",
            ));
        }
        names.push(name);
    }
    Ok(names)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryContext {
    pub country_tag: String,
    /// Native faction member order, including the leader at ordinal zero.
    pub faction_tags: Vec<String>,
    pub capital_continent: Option<String>,
}

/// One hour's strict join. Positive native country references resolve through
/// CountryLinks; no local viewing country or selected map object is involved.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    slots: BTreeMap<u32, u32>,
    contexts: BTreeMap<u32, CountryContext>,
}

impl Prepared {
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
            // Native compares exactly with zero, including negative values.
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

    /// A nonpositive CCountry reference visits only the default pool. A
    /// positive but unresolved reference is unavailable, never known absence.
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
