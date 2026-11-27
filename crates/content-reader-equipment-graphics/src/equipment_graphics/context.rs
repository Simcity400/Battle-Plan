use content_reader_script::native::{self, Shape};

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    map::ProvinceDefinition,

    states,
    terrain::DefaultMap,
};
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;

#[derive(Clone, Debug, Default)]
pub struct Content {
    continent_by_state: BTreeMap<u32, Option<String>>,
}

pub fn state_continent(
    provinces: &[u32],
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    coastal: &BTreeSet<u32>,
    continents: &[String],
) -> Option<String> {
    let mut chosen = None;
    for province in provinces {
        if chosen.is_none()
            && let Some(index) = definitions
                .get(province)
                .and_then(|definition| definition.continent.checked_sub(1))
            && let Some(name) = continents.get(usize::from(index))
        {
            chosen = Some(name.clone());
        }
        if coastal.contains(province) {
            break;
        }
    }
    chosen
}

impl Content {
    pub fn load(
        layers: &ResolvedLayers,
        default_map: &DefaultMap,
        provinces: &BTreeMap<u32, ProvinceDefinition>,
        defines: &crate::defines::Defines,
    ) -> io::Result<Self> {
        let continents = continent_names(&layers.read(&default_map.file("continent")?)?)?;
        let coastal: BTreeSet<u32> =
            crate::map::definition_rows(&layers.read(&default_map.file("definitions")?)?)?
                .into_iter()
                .filter(|row| row.coastal && row.kind() != Some(crate::map::ProvinceKind::Sea))
                .map(|row| u32::from(row.id))
                .collect();
        Ok(Self {
            continent_by_state: states::read(layers, defines, provinces)?
                .0
                .definitions()
                .iter()
                .map(|(id, state)| {
                    (
                        *id,
                        state_continent(&state.provinces, provinces, &coastal, &continents),
                    )
                })
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
    content_reader_parser::fields::invalid(message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CountryContext {
    pub country_tag: String,
    pub faction_tags: Vec<String>,
    pub capital_continent: Result<Option<String>, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[derive(replay_spool::Spooled)]
pub struct Prepared {
    slots: BTreeMap<u32, u32>,
    contexts: BTreeMap<u32, CountryContext>,
    capital_states: BTreeMap<u32, u32>,
    capital_continent_tokens: BTreeMap<u32, u32>,
}

impl Prepared {
    pub fn heap(&self) -> usize {
        let text = |text: &String| std::mem::size_of::<String>() + text.len();
        self.slots.len() * 2 * std::mem::size_of::<u32>()
            + self.capital_states.len() * 2 * std::mem::size_of::<u32>()
            + self.capital_continent_tokens.len() * 2 * std::mem::size_of::<u32>()
            + self
                .contexts
                .values()
                .map(|context| {
                    std::mem::size_of::<(u32, CountryContext)>()
                        + context.country_tag.len()
                        + context.faction_tags.iter().map(text).sum::<usize>()
                        + match &context.capital_continent {
                            Ok(continent) => continent.as_ref().map_or(0, String::len),
                            Err(why) => why.len(),
                        }
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
        let lexical: Result<BTreeMap<u32, &[u8]>, String> = frame
            .complete::<tables::LexicalTokens>()
            .map(|rows| {
                rows.iter()
                    .map(|row| (row.token, row.literal.as_slice()))
                    .collect()
            })
            .map_err(|why| why.to_string());

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
        let states_by_id: BTreeMap<u32, _> = states.iter().map(|row| (row.id, row)).collect();
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
        let mut capital_states = BTreeMap::new();
        let mut capital_continent_tokens = BTreeMap::new();
        for (country_slot, tag) in tags {
            let capital = *capital_by_slot
                .get(&country_slot)
                .ok_or_else(|| format!("country slot {country_slot} has no Politics row"))?;
            let capital_continent = if capital <= 0 {
                Ok(None)
            } else if let Some(state) = states_by_id.get(&(capital as u32)) {
                capital_states.insert(country_slot, capital as u32);
                match state.continent {
                    Recorded::Value(Some(0)) => Ok(None),
                    Recorded::Value(Some(token)) => {
                        capital_continent_tokens.insert(country_slot, token);
                        match &lexical {
                            Ok(tokens) => match tokens.get(&token) {
                                Some(literal) => std::str::from_utf8(literal)
                                    .map(|text| Some(text.to_ascii_lowercase()))
                                    .map_err(|_| {
                                        format!(
                                            "capital state {capital} continent token {token} is not UTF-8"
                                        )
                                    }),
                                None => Err(format!(
                                    "capital state {capital} continent token {token} has no recorded spelling"
                                )),
                            },
                            Err(why) => Err(format!(
                                "capital state {capital} continent spelling is unavailable: {why}"
                            )),
                        }
                    }
                    Recorded::Value(None) => Err(format!(
                        "capital state {capital} continent capture unavailable"
                    )),
                    Recorded::NotRecorded => content
                        .continent_by_state
                        .get(&(capital as u32))
                        .cloned()
                        .ok_or_else(|| format!("capital state {capital} is absent from content")),
                }
            } else {
                Ok(None)
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
        Ok(Self {
            slots,
            contexts,
            capital_states,
            capital_continent_tokens,
        })
    }

    pub fn slot(&self, country_ref: u32) -> Option<u32> {
        if (country_ref as i32) <= 0 {
            return None;
        }
        self.slots.get(&country_ref).copied()
    }

    pub fn capital_state(&self, country_ref: u32) -> Option<u32> {
        let slot = self.slot(country_ref)?;
        self.capital_states.get(&slot).copied()
    }

    pub fn capital_continent_token(&self, country_ref: u32) -> Option<u32> {
        let slot = self.slot(country_ref)?;
        self.capital_continent_tokens.get(&slot).copied()
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
                    capital_continent: Ok(Some("europe".into())),
                },
            )]
            .into(),
            capital_states: BTreeMap::new(),
            capital_continent_tokens: BTreeMap::new(),
        };
        assert_eq!(prepared.context(0), Ok(None));
        assert_eq!(prepared.context(u32::MAX), Ok(None));
        assert_eq!(prepared.context(5).unwrap().unwrap().country_tag, "GER");
        assert!(prepared.context(6).is_err());
    }

    #[test]
    fn mixed_continent_and_coastal_states_follow_native_continent_scan() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_context_mixed_continent_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in [
            (
                "map/default.map",
                "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt\n",
            ),
            ("map/continent.txt", "continents = { europe asia }\n"),
            (
                "map/definition.csv",
                "0;0;0;0;land;false;unknown;0\n\
                 1;1;0;0;land;false;plains;1\n\
                 2;2;0;0;land;false;plains;2\n\
                 3;3;0;0;sea;true;ocean;0\n\
                 4;4;0;0;land;false;plains;2\n\
                 5;5;0;0;sea;false;ocean;0\n\
                 6;6;0;0;land;true;plains;0\n",
            ),
            (
                "common/defines/00_defines.lua",
                "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 10 } }\n",
            ),
            (
                "history/states/1-mixed.txt",
                "state = { id = 1 name = \"STATE_1\" provinces = { 1 2 } }\n",
            ),
            (
                "history/states/2-sea-coastal-first.txt",
                "state = { id = 2 name = \"STATE_2\" provinces = { 3 4 } }\n",
            ),
            (
                "history/states/3-inland-zero-first.txt",
                "state = { id = 3 name = \"STATE_3\" provinces = { 5 4 } }\n",
            ),
            (
                "history/states/4-land-coastal-first.txt",
                "state = { id = 4 name = \"STATE_4\" provinces = { 6 4 } }\n",
            ),
        ] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let default_map = crate::terrain::read_default_map(&layers).unwrap();
        let provinces = crate::map::parse_definitions(
            &layers.read(&default_map.file("definitions").unwrap()).unwrap(),
        )
        .unwrap();
        let defines = crate::defines::read(&layers).unwrap();
        let loaded = Content::load(&layers, &default_map, &provinces, &defines).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            loaded.continent_by_state.get(&1),
            Some(&Some("europe".to_owned()))
        );
        assert_eq!(
            loaded.continent_by_state.get(&2),
            Some(&Some("asia".to_owned()))
        );
        assert_eq!(
            loaded.continent_by_state.get(&3),
            Some(&Some("asia".to_owned()))
        );
        assert_eq!(loaded.continent_by_state.get(&4), Some(&None));
    }
}
