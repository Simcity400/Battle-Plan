use super::counter_content;
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::{Argument as Arg, Localisation};
use installed_content_regions::regions::StrategicRegion;
use replay_data::rows::{FactionTheatreRow, Recorded, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_foundation::tooltip_text::{Tooltip, localise};

type Fact<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theatre<'a> {
    pub name: Recorded<&'a [u8]>,
    pub owner: Recorded<u32>,
    pub commander: Recorded<Option<u64>>,
    pub countries: Recorded<&'a [u32]>,
    pub regions: &'a [u32],
}

impl<'a> Theatre<'a> {
    pub fn of(row: &'a FactionTheatreRow) -> Self {
        Self {
            name: borrowed(&row.name),
            owner: row.owner.clone(),
            commander: row.commander.clone(),
            countries: borrowed(&row.countries),
            regions: &row.regions,
        }
    }
}

fn borrowed<T>(field: &Recorded<Vec<T>>) -> Recorded<&[T]> {
    match field {
        Recorded::Value(values) => Recorded::Value(values),
        Recorded::NotRecorded => Recorded::NotRecorded,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub tag: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leader {
    pub name: String,
    pub category: i32,
    pub skill: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub name: Fact<String>,
    pub owner: Fact<Country>,
    pub commander: Fact<Option<Leader>>,
    pub countries: Fact<Vec<Country>>,
    pub regions: Fact<Vec<String>>,
}

const LINE: &str = "faction theatre";

pub fn build(content: &counter_content::Content, frame: &DecodedFrame, faction: u64, ordinal: u32) -> Tooltip {
    let row = frame.complete::<tables::FactionTheatres>().map_err(|e| e.to_string()).and_then(|rows| {
        rows.binary_search_by_key(&(faction, ordinal), |row| (row.faction, row.ordinal))
            .map(|at| &rows[at])
            .map_err(|_| format!("FactionTheatres holds no theatre {ordinal} of faction {faction}"))
    });
    match row {
        Ok(row) => {
            let theatre = Theatre::of(row);
            let facts = facts(&content.localisation, &content.recorded_language, &content.strategic_regions, frame, &theatre);
            lines(&content.localisation, &facts)
        }
        Err(reason) => {
            let mut out = Tooltip::default();
            out.refuse(LINE, reason);
            out
        }
    }
}

pub fn facts(
    loc: &Localisation,
    recorded_language: &str,
    regions: &BTreeMap<u32, StrategicRegion>,
    frame: &DecodedFrame,
    theatre: &Theatre<'_>,
) -> Facts {
    let name = theatre
        .name
        .require("FactionTheatres.name")
        .and_then(|bytes| String::from_utf8(bytes.to_vec()).map_err(|_| "the stored theatre name is not UTF-8".to_owned()));
    let links = std::cell::OnceCell::new();
    let slot = |raw: u32| -> Fact<u32> {
        let links = links.get_or_init(|| frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())).as_ref().map_err(Clone::clone)?;
        links
            .binary_search_by_key(&raw, |row| row.id)
            .map(|at| links[at].canonical_slot)
            .map_err(|_| format!("country link {raw} has no CountryLinks row"))
    };
    let owner = theatre.owner.require("FactionTheatres.owner").and_then(|&raw| slot(raw));
    let countries = theatre
        .countries
        .require("FactionTheatres.countries")
        .and_then(|raws| raws.iter().map(|&raw| slot(raw)).collect::<Fact<Vec<_>>>());
    let wanted: Vec<u32> = owner.iter().copied().chain(countries.iter().flatten().copied()).collect();
    let names = (!wanted.is_empty()).then(|| CountryNames::read_selected(frame, loc, wanted));
    let country = |slot: u32| -> Fact<Country> {
        let names = names.as_ref().ok_or("no country names were read")?;
        let tag = names
            .operands(slot)
            .ok_or_else(|| format!("country {slot} has no recorded name operands"))?
            .as_ref()
            .map_err(Clone::clone)?
            .tag
            .clone();
        Ok(Country { tag, name: names.name(slot)? })
    };
    let commander = theatre.commander.require("FactionTheatres.commander").and_then(|commander| {
        commander.map(|handle| leader(loc, recorded_language, frame, handle)).transpose()
    });
    Facts {
        name,
        owner: owner.and_then(&country),
        commander,
        countries: countries.and_then(|slots| slots.into_iter().map(&country).collect()),
        regions: theatre
            .regions
            .iter()
            .map(|id| {
                let region = regions.get(id).ok_or_else(|| format!("strategic region {id} is absent from the content"))?;
                localise(loc, &region.name_key, &[])
            })
            .collect(),
    }
}

fn leader(loc: &Localisation, recorded_language: &str, frame: &DecodedFrame, handle: u64) -> Fact<Leader> {
    let leaders = frame.complete::<tables::UnitLeaders>().map_err(|e| e.to_string())?;
    let row = leaders
        .binary_search_by_key(&handle, |row| u64::from(row.id) << 32 | u64::from(row.reference_type))
        .map(|at| &leaders[at])
        .map_err(|_| format!("UnitLeaders leader {handle} is absent"))?;
    Ok(Leader {
        name: dashboard_text::scripted_text::stored_text(loc, recorded_language, &row.inherited_text_b, "unit leader name")?,
        category: row.category,
        skill: (row.skill_status != 0).then_some(row.skill_level),
    })
}

pub fn lines(loc: &Localisation, facts: &Facts) -> Tooltip {
    let mut out = Tooltip::default();
    let commander = facts.commander.clone().and_then(|leader| commander_text(loc, leader.as_ref()));
    let countries = facts.countries.clone().and_then(|countries| {
        joined(loc, &countries, "FACTION_THEATER_TOOLTIP_NO_COUNTRIES", |country| {
            localise(loc, "FACTION_THEATER_TOOLTIP_COUNTRY", &[("TAG", Arg::Text(&country.tag)), ("NAME", Arg::Text(&country.name))])
        })
    });
    let regions = facts.regions.clone().and_then(|regions| {
        joined(loc, &regions, "FACTION_THEATER_TOOLTIP_NO_REGIONS", |name| {
            localise(loc, "FACTION_THEATER_TOOLTIP_REGION", &[("NAME", Arg::Text(name))])
        })
    });
    let (Ok(name), Ok(owner), Ok(commander), Ok((num_countries, countries)), Ok((num_regions, regions))) =
        (&facts.name, &facts.owner, &commander, &countries, &regions)
    else {
        let missing = [
            ("name", facts.name.as_ref().err()),
            ("owner", facts.owner.as_ref().err()),
            ("commander", commander.as_ref().err()),
            ("countries", countries.as_ref().err()),
            ("regions", regions.as_ref().err()),
        ];
        for (part, reason) in missing {
            if let Some(reason) = reason {
                out.refuse(&format!("{LINE} {part}"), reason.clone());
            }
        }
        return out;
    };
    let text = localise(
        loc,
        "FACTION_THEATER_TOOLTIP",
        &[
            ("THEATER", Arg::Text(name)),
            ("COMMANDER", Arg::Text(commander)),
            ("OWNER_TAG", Arg::Text(&owner.tag)),
            ("OWNER_NAME", Arg::Text(&owner.name)),
            ("NUM_COUNTRIES", Arg::Int32(*num_countries)),
            ("NUM_REGIONS", Arg::Int32(*num_regions)),
            ("COUNTRIES", Arg::Text(countries)),
            ("REGIONS", Arg::Text(regions)),
        ],
    );
    match text {
        Ok(text) => out.title = text,
        Err(reason) => out.refuse(LINE, reason),
    }
    out
}

fn commander_text(loc: &Localisation, leader: Option<&Leader>) -> Fact<String> {
    let Some(leader) = leader else {
        return localise(loc, "FACTION_THEATER_TOOLTIP_NO_COMMANDER", &[]);
    };
    let title = role_title(loc, leader.category, leader.skill)?;
    localise(loc, "FACTION_THEATER_TOOLTIP_COMMANDER", &[("NAME", Arg::Text(&leader.name)), ("TITLE", Arg::Text(&title))])
}

fn role_title(loc: &Localisation, category: i32, skill: Option<i32>) -> Fact<String> {
    let key = match category {
        0 => "fm_leader".to_owned(),
        1 => "cc_leader".to_owned(),
        2 => "navy_leader".to_owned(),
        3 => {
            let skill = skill.ok_or("the operative has no skill definition, whose level picks its title")?;
            let key = format!("operative_skill_{skill}");
            if loc.get(&key).is_some() { key } else { "OPERATIVE".to_owned() }
        }
        _ => return Ok(String::new()),
    };
    localise(loc, &key, &[])
}

fn joined<T>(loc: &Localisation, items: &[T], empty: &str, item: impl Fn(&T) -> Fact<String>) -> Fact<(i32, String)> {
    let count = i32::try_from(items.len()).map_err(|_| "the list is longer than the game can count".to_owned())?;
    if items.is_empty() {
        return Ok((0, localise(loc, empty, &[])?));
    }
    let mut out = String::new();
    for value in items {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&item(value)?);
    }
    Ok((count, out))
}

#[cfg(test)]
mod tests;
