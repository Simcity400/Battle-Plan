
use super::{Named, rows};
use crate::products::group_members::GroupsAt;
use dashboard_base::{Source, products::{Country, LoadContext, at_hour::{At, AtHour, Read}, validate_country}};
use dashboard_query::{identity::{Holder, Holders}, series::ranked::GroupSelection};
use engine_native_air::raid_tooltip_operands::{
    NUCLEAR_BOMB_TOKEN,
    THERMONUCLEAR_BOMB_TOKEN,
};
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::{NukeRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct NuclearArsenal;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NuclearArsenalParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The countries whose stockpiles are the rows; never with a top or a group."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[doc = " The group whose members at the hour are the rows; never with countries or a top."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NukeType {
    pub ordinal: u32,
    #[doc = " Its lexical token's spelling; null when the game names no token for it."]
    pub token: Option<String>,
    #[doc = " Its name in the reader's language; null when the content has none."]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NukeCount {
    pub ordinal: u32,
    #[doc = " Whole bombs, as the top bar shows them."]
    pub value: i32,
    #[doc = " Queued drops whose delay elapsed."]
    pub ready: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NukeCountry {
    pub tag: String,
    pub country_id: u32,
    #[doc = " The recorded slot holding the country at the hour."]
    pub slot: Option<u32>,
    #[doc = " The top bar's stockpile: every type's count added with signed 32-bit wrapping."]
    pub total: i32,
    pub counts: Vec<NukeCount>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NuclearArsenalReply {
    #[doc = " Every type the game names and every other type a country holds at the hour, by ordinal."]
    pub types: Vec<NukeType>,
    #[doc = " Every country a tag holds with a stockpile row (or each chosen country, a recorded zero where it holds none), greatest total first, ties by tag."]
    pub countries: Vec<NukeCountry>,
    #[doc = " Why no stockpile is known at the hour."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NuclearArsenalReply);

const NAMED_TYPES: [u32; 2] = [0, 1];

pub fn type_token(ordinal: u32) -> Option<u32> {
    match ordinal {
        0 => Some(NUCLEAR_BOMB_TOKEN),
        1 => Some(THERMONUCLEAR_BOMB_TOKEN),
        _ => None,
    }
}

pub(super) fn countries(rows: &[NukeRow], tag_of: impl Fn(u32) -> Option<String>) -> (Vec<NukeCountry>, usize, Vec<i32>) {
    let mut by_country: BTreeMap<u32, Vec<NukeCount>> = BTreeMap::new();
    for row in rows {
        by_country.entry(row.country).or_default().push(NukeCount { ordinal: row.nuke_type, value: NukeRow::shown(row.amount_raw), ready: row.ready });
    }
    let mut unheld = 0;
    let mut unplaced_totals = Vec::new();
    let mut out: Vec<NukeCountry> = Vec::new();
    for (country, counts) in by_country {
        let total = counts.iter().fold(0i32, |sum, count| sum.wrapping_add(count.value));
        let Some(tag) = tag_of(country) else {
            unheld += 1;
            unplaced_totals.push(total);
            continue;
        };
        out.push(NukeCountry { tag, country_id: country, slot: None, total, counts });
    }
    out.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
    (out, unheld, unplaced_totals)
}

pub(super) fn top_countries(mut countries: Vec<NukeCountry>, unplaced_totals: &[i32], holders_unresolved: bool, limit: u32) -> Result<Vec<NukeCountry>, String> {
    countries.retain(|country| country.total > 0);
    let count = usize::try_from(limit).unwrap_or(usize::MAX);
    countries.truncate(count);
    if holders_unresolved && !unplaced_totals.is_empty() {
        let full = countries.len() >= count;
        let floor = countries.last().map(|country| country.total);
        let contending = unplaced_totals.iter().filter(|&&total| total > 0 && (!full || floor.is_none_or(|floor| total >= floor))).count();
        if contending > 0 {
            return Err(format!(
                "The top {limit} nuclear stockpiles are not known: {contending} countries with a stockpile could not be matched to a tag at this hour and could be among them"
            ));
        }
    }
    Ok(countries)
}

pub(super) fn held_at(holders: &Holders, hour: i32) -> Vec<(u32, u32, String)> {
    let mut held: Vec<(u32, u32, String)> = holders
        .tags
        .iter()
        .filter_map(|(tag, runs)| match runs.iter().find(|run| run.first <= hour && hour <= run.last).map(|run| run.holder) {
            Some(Holder::Found { slot, id }) => Some((slot, id, tag.clone())),
            _ => None,
        })
        .collect();
    held.sort_unstable();
    held
}

pub(super) fn chosen_countries(all: Vec<NukeCountry>, chosen: &[(u32, u32, String)]) -> Vec<NukeCountry> {
    let mut by_id: BTreeMap<u32, NukeCountry> = all.into_iter().map(|country| (country.country_id, country)).collect();
    let mut out: Vec<NukeCountry> = chosen
        .iter()
        .map(|(slot, id, tag)| match by_id.remove(id) {
            Some(country) => NukeCountry { slot: Some(*slot), ..country },
            None => NukeCountry { tag: tag.clone(), country_id: *id, slot: Some(*slot), total: 0, counts: Vec::new() },
        })
        .collect();
    out.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
    out
}

pub(super) fn rerank(rank: &mut serde_json::Value, rows: &[NukeRow], country: u32, tag: &str, slot: Option<u32>) -> io::Result<()> {
    let rank = rank.as_array_mut().expect("an arsenal ranking");
    if let Some(old) = rank.iter().position(|c| c["countryId"].as_u64() == Some(country.into())) { rank.remove(old); }
    let first = rows.partition_point(|r| r.country < country);
    let rows = &rows[first..first + rows[first..].partition_point(|r| r.country == country)];
    if rows.is_empty() { return Ok(()); }
    let counts: Vec<_> = rows.iter().map(|r| NukeCount { ordinal: r.nuke_type, value: NukeRow::shown(r.amount_raw), ready: r.ready }).collect();
    let total = counts.iter().fold(0i32, |sum, c| sum.wrapping_add(c.value));
    let index = rank.partition_point(|c| {
        let held = c["total"].as_i64().expect("a stockpile total");
        held > i64::from(total) || held == i64::from(total) && c["tag"].as_str().expect("a ranked tag") < tag
    });
    rank.insert(index, serde_json::to_value(NukeCountry { country_id: country, tag: tag.into(), slot, counts, total })?);
    Ok(())
}

pub struct NuclearContext {
    names: Result<Arc<Localisation>, String>,
    groups: GroupsAt,
}

impl AtHour for NuclearArsenal {
    const KEY: &'static str = "nuclear-arsenal";
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    const TITLE: &'static str = "Nuclear arsenal";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NuclearArsenalParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, top, countries, group, "countries[]"];
    type Reply = NuclearArsenalReply;
    type Context = NuclearContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NuclearArsenalParams::decl(), NukeType::decl(), NukeCount::decl(), NukeCountry::decl(), NuclearArsenalReply::decl()]
    }


    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)?;
        if params.top == Some(0) {
            return Err("top must be at least 1");
        }
        if [params.top.is_some(), !params.countries.is_empty(), params.group.is_some()].into_iter().filter(|&asked| asked).count() > 1 {
            return Err("A selection names a group, a top or countries, one of them");
        }
        params.countries.iter().try_for_each(validate_country)
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.groups.valid_through(params.group.as_ref(), at.hour)
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        if params.group.is_some() || !params.countries.is_empty() || params.top.is_some() && (edits.tables.contains_key(&TableId::Nukes) || edits.replaced.contains(&TableId::Nukes)) {
            return Ok(false);
        }
        if !edits.only_columns(TableId::Nukes, u64::MAX) { return Ok(false); }
        let rows = frame.complete::<tables::Nukes>()?;
        let mut last = None;
        for (&key, &mask) in edits.tables.get(&TableId::Nukes).into_iter().flatten() {
            if mask == u64::MAX && !NAMED_TYPES.contains(&(key as u32)) {
                let present = rows.iter().any(|r| r.nuke_type == key as u32);
                let named = data["types"].as_array().unwrap().iter().any(|t| t["ordinal"].as_u64() == Some(u64::from(key as u32)));
                if present != named { return Ok(false); }
            }
            let id = (key >> 32) as u32;
            if last == Some(id) { continue; }
            last = Some(id);
            let held = held_at(at.holders, at.hour).into_iter().find(|&(_, held, _)| held == id);
            if let Some((slot, _, tag)) = held { rerank(&mut data["countries"], rows, id, &tag, Some(slot))?; }
            else if mask == u64::MAX { return Ok(false); }
        }
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Nukes, TableId::LexicalTokens]
    }

    fn reads(_params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Nukes), Read::these(TableId::LexicalTokens, [NUCLEAR_BOMB_TOKEN, THERMONUCLEAR_BOMB_TOKEN].map(u128::from))],
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(NuclearContext {
            names: match load.content.as_ref() {
                Some(content) => dashboard_base::content::localisation(source, content).map_err(|error| format!("Nuclear type names are unavailable: {error}")),
                None => Err("Nuclear type names are unavailable until the game folders are chosen".into()),
            },
            groups: GroupsAt::default(),
        })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        Ok(NuclearContext { groups: GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?, ..Self::context(source, load)? })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = NuclearArsenalReply { types: Vec::new(), countries: Vec::new(), missing: None, warnings: Vec::new() };
        let nukes = match dashboard_query::coverage::population::<tables::Nukes>(frame, &Keys::All) {
            Ok(nukes) => nukes,
            Err(reason) => {
                reply.missing = Some(format!("Nuclear stockpiles are not known at this hour: {reason}"));
                return Ok(reply);
            }
        };
        let named = Named::new(at.holders, frame.hour);
        let (countries, unheld, unplaced_totals) = countries(nukes, |country| named.id(country).map(str::to_owned));
        let held = held_at(at.holders, frame.hour);
        let slots: BTreeMap<u32, u32> = held.iter().map(|&(slot, id, _)| (id, slot)).collect();
        let countries: Vec<NukeCountry> = countries.into_iter().map(|country| NukeCountry { slot: slots.get(&country.country_id).copied(), ..country }).collect();
        let chosen = match at.context.groups.chosen_slots(params.group.as_ref(), &params.countries, at.holders, frame.hour)? {
            Ok(chosen) => chosen,
            Err(refused) => {
                reply.missing = Some(refused);
                return Ok(reply);
            }
        };
        if let Some(chosen) = chosen {
            let picked: Vec<(u32, u32, String)> = held.into_iter().filter(|(slot, ..)| chosen.contains(slot)).collect();
            reply.countries = chosen_countries(countries, &picked);
        } else if let Some(limit) = params.top {
            let unresolved = super::game_score::holders_unresolved(at.holders, frame.hour);
            match top_countries(countries, &unplaced_totals, unresolved, limit) {
                Ok(top) => reply.countries = top,
                Err(reason) => {
                    reply.missing = Some(reason);
                    return Ok(reply);
                }
            }
        } else {
            reply.countries = countries;
        }
        if unheld > 0 {
            reply.warnings.push(format!("{unheld} countries with a stockpile are held by no tag at this hour and are not shown"));
        }
        let spellings: BTreeMap<u32, &[u8]> = rows::<tables::LexicalTokens>(frame).map(|held| held.iter().map(|row| (row.token, row.literal.as_slice())).collect()).unwrap_or_default();
        let mut ordinals: Vec<u32> = nukes.iter().map(|row| row.nuke_type).chain(NAMED_TYPES).collect();
        ordinals.sort_unstable();
        ordinals.dedup();
        if let Err(reason) = &at.context.names {
            reply.warnings.push(reason.clone());
        }
        for ordinal in ordinals {
            let Some(token) = type_token(ordinal) else {
                reply.warnings.push(format!("Nuclear type {ordinal} is not one the game names: it is counted in totals and shown unnamed"));
                reply.types.push(NukeType { ordinal, token: None, name: None });
                continue;
            };
            let spelling = spellings.get(&token).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned);
            if spelling.is_none() {
                reply.warnings.push(format!("Nuclear type {ordinal}'s lexical token {token} has no recorded spelling"));
            }
            let name = match (&spelling, &at.context.names) {
                (Some(spelling), Ok(localisation)) => dashboard_text::catalogue::localised(localisation, spelling, dashboard_text::catalogue::View::Plain)
                    .map_err(|reason| reply.warnings.push(format!("{spelling}: {reason}")))
                    .ok(),
                _ => None,
            };
            reply.types.push(NukeType { ordinal, token: spelling, name });
        }
        Ok(reply)
    }
}
