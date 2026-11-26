use super::{branch, factory_groups, game_score::holders_unresolved, rows};
use crate::products::group_members::GroupsAt;
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Clock, Owner, Read, ReadsAt},
        validate_country,
    },
};
use dashboard_query::{
    coverage::{Witness, witnessed},
    identity::Holder,
    series::ranked::GroupSelection,
};
use replay_data::rows::{CountryFactoriesRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactoryMix;

const GROUPS: [&str; 3] = ["military", "naval", "civilian"];
const MODES: [&str; 3] = ["free", "built", "topBar"];

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactoryMixParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    pub mode: String,
    #[serde(default)]
    pub types: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryMixPart {
    pub key: String,
    pub name: Option<String>,
    pub damaged: bool,
    pub group: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryMixRow {
    pub country: Country,
    pub tag: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "(number | null)[]"))]
    pub values: Vec<Option<i64>>,
    pub shares: Vec<Option<f64>>,
    pub missing: Option<String>,
    pub parts_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryMixReply {
    pub hour: i32,
    pub mode: String,
    pub parts: Vec<FactoryMixPart>,
    pub rows: Vec<FactoryMixRow>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactoryMixReply);

pub struct FactoryMixContext {
    groups: Arc<factory_groups::FactoryGroupsReply>,
    members: GroupsAt,
}

fn shown(raw: i64) -> Option<i64> {
    i32::try_from(raw / 100_000).ok().map(i64::from)
}

fn group_fields(row: &CountryFactoriesRow, group: usize) -> (i64, i64, i32, i32) {
    match group {
        0 => (row.military_total_raw, row.military_damaged_raw, row.military_to_overlord, row.military_to_others),
        1 => (row.naval_total_raw, row.naval_damaged_raw, row.naval_to_overlord, row.naval_to_others),
        _ => (row.civilian_total_raw, row.civilian_damaged_raw, row.civilian_to_overlord, row.civilian_to_others),
    }
}

pub(super) fn group_count(row: &CountryFactoriesRow, group: usize, mode: &str) -> Option<i64> {
    let (total_raw, damaged_raw, to_overlord, to_others) = group_fields(row, group);
    if mode == "damaged" {
        return shown(damaged_raw);
    }
    let total = shown(total_raw)?;
    if mode == "built" {
        return Some(total);
    }
    let given = i64::from(to_overlord) + i64::from(to_others);
    let left = total - shown(damaged_raw)? - given;
    if mode == "topBar" || group != 2 {
        return Some(left);
    }
    Some(
        left - i64::from(row.consumer_goods)
            - i64::from(row.contract_payments)
            - i64::from(row.special_projects)
            - i64::from(row.license_imports)
            - i64::from(row.trade_imports),
    )
}

pub fn factory_channel_value(row: &CountryFactoriesRow, measure: &str) -> Option<i64> {
    let (mode, mask_text) = measure.split_at(measure.len().saturating_sub(1));
    if !matches!(mode, "free" | "built" | "topBar" | "damaged") {
        return None;
    }
    let mask: usize = mask_text.parse().ok()?;
    if !(1..=7).contains(&mask) {
        return None;
    }
    (0..3)
        .filter(|group| mask & (1 << group) != 0)
        .try_fold(0i64, |sum, group| sum.checked_add(group_count(row, group, mode)?))
}

fn chosen_groups(types: &[String]) -> Vec<usize> {
    if types.is_empty() {
        return vec![0, 1, 2];
    }
    GROUPS
        .iter()
        .enumerate()
        .filter(|(_, name)| types.iter().any(|asked| asked == *name))
        .map(|(index, _)| index)
        .collect()
}

fn group_title(context: &factory_groups::FactoryGroupsReply, key: &str) -> Option<String> {
    let group = context.groups.iter().find(|held| held.key == key)?;
    if group.buildings.is_empty() {
        return None;
    }
    Some(
        group
            .buildings
            .iter()
            .map(|building| building.name.as_deref().unwrap_or("Building name unavailable"))
            .collect::<Vec<_>>()
            .join(" \u{00b7} "),
    )
}

pub(super) fn mix_parts(context: &factory_groups::FactoryGroupsReply, mode: &str, chosen: &[usize]) -> Vec<FactoryMixPart> {
    let mut parts = Vec::with_capacity(chosen.len() * if mode == "built" { 2 } else { 1 });
    for &group in chosen {
        let key = GROUPS[group];
        parts.push(FactoryMixPart {
            key: key.to_owned(),
            name: group_title(context, key),
            damaged: false,
            group: key.to_owned(),
        });
    }
    if mode == "built" {
        for &group in chosen {
            let key = GROUPS[group];
            let base = group_title(context, key);
            parts.push(FactoryMixPart {
                key: format!("damaged:{key}"),
                name: base.map(|name| format!("Damaged {name}")),
                damaged: true,
                group: key.to_owned(),
            });
        }
    }
    parts
}

pub(super) fn row_mix(
    row: &CountryFactoriesRow,
    mode: &str,
    chosen: &[usize],
) -> (Option<i64>, Vec<Option<i64>>, Vec<Option<f64>>, Option<String>, Option<String>) {
    let group_totals: Vec<Option<i64>> = chosen.iter().map(|&group| group_count(row, group, mode)).collect();
    let mut total_missing: Option<String> = None;
    let mut total_acc = 0i64;
    for value in &group_totals {
        match value {
            Some(v) => match total_acc.checked_add(*v) {
                Some(next) => total_acc = next,
                None if total_missing.is_none() => total_missing = Some("Factory total overflowed at this hour".to_owned()),
                None => {}
            },
            None if total_missing.is_none() => {
                total_missing = Some("A factory total does not fit a whole number the game shows".to_owned());
            }
            None => {}
        }
    }
    let total = total_missing.is_none().then_some(total_acc);

    if mode == "built" {
        let mut undamaged = Vec::with_capacity(chosen.len());
        let mut damaged = Vec::with_capacity(chosen.len());
        let mut parts_missing: Option<String> = None;
        for (&group, &built_opt) in chosen.iter().zip(&group_totals) {
            let dmg_opt = group_count(row, group, "damaged");
            match (built_opt, dmg_opt) {
                (Some(built), Some(dmg)) if dmg >= 0 && built >= 0 && dmg <= built => {
                    undamaged.push(Some(built - dmg));
                    damaged.push(Some(dmg));
                }
                (Some(_), None) => {
                    undamaged.push(None);
                    damaged.push(None);
                    if parts_missing.is_none() {
                        parts_missing = Some("A damaged factory count does not fit a whole number the game shows".to_owned());
                    }
                }
                (Some(_), Some(_)) => {
                    undamaged.push(None);
                    damaged.push(None);
                    if parts_missing.is_none() {
                        parts_missing = Some("Damaged factories do not reconcile with built factories at this hour".to_owned());
                    }
                }
                (None, Some(dmg)) if dmg >= 0 => {
                    undamaged.push(None);
                    damaged.push(Some(dmg));
                }
                _ => {
                    undamaged.push(None);
                    damaged.push(None);
                }
            }
        }
        undamaged.extend(damaged);
        let values = undamaged;
        let shares = compute_shares(total, &values, parts_missing.is_some());
        return (total, values, shares, total_missing, parts_missing);
    }

    let mut parts_missing = None;
    if group_totals.iter().any(|v| v.is_some_and(|n| n < 0)) {
        parts_missing = Some("A factory group count is negative at this hour; shares cannot be shown".to_owned());
    }
    let shares = compute_shares(total, &group_totals, parts_missing.is_some());
    (total, group_totals, shares, total_missing, parts_missing)
}

fn compute_shares(total: Option<i64>, values: &[Option<i64>], has_parts_issue: bool) -> Vec<Option<f64>> {
    let Some(total) = total.filter(|&t| t > 0) else {
        return vec![None; values.len()];
    };
    if values.iter().any(|v| v.is_some_and(|n| n < 0)) {
        return vec![None; values.len()];
    }
    let known_sum: i64 = values.iter().filter_map(|v| *v).sum();
    if known_sum > total || (!has_parts_issue && values.iter().all(Option::is_some) && known_sum != total) {
        return vec![None; values.len()];
    }
    values.iter().map(|v| v.map(|n| n as f64 / total as f64)).collect()
}

fn sort_rows(rows: &mut [FactoryMixRow]) {
    rows.sort_unstable_by(|a, b| match (a.total, b.total) {
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)).then_with(|| a.country.cmp(&b.country)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.tag.cmp(&b.tag).then_with(|| a.country.cmp(&b.country)),
    });
}

fn country_id_by_slot(holders: &dashboard_query::identity::Holders, hour: i32) -> HashMap<u32, (u32, String)> {
    let mut out = HashMap::new();
    for (tag, runs) in &holders.tags {
        let Some(run) = runs.iter().find(|run| run.first <= hour && hour <= run.last) else { continue };
        if let Holder::Found { slot, id } = run.holder {
            out.entry(id).or_insert_with(|| (slot, tag.clone()));
        }
    }
    out
}

impl AtHour for FactoryMix {
    const KEY: &'static str = "factory-mix";
    const TITLE: &'static str = "Factory mix";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: Clock = Clock::Boundaries;
    type Params = FactoryMixParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, group, top, mode, types, "countries[]"];
    type Reply = FactoryMixReply;
    type Context = FactoryMixContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactoryMixParams::decl(), FactoryMixPart::decl(), FactoryMixRow::decl(), FactoryMixReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        for country in &params.countries {
            validate_country(country)?;
        }
        if !MODES.contains(&params.mode.as_str()) {
            return Err("Factory mix mode must be free, built or topBar");
        }
        if params.types.iter().any(|name| !GROUPS.contains(&name.as_str())) {
            return Err("Factory mix types must name military, naval or civilian");
        }
        let selectors = usize::from(!params.countries.is_empty()) + usize::from(params.group.is_some()) + usize::from(params.top.is_some());
        if selectors > 1 {
            return Err("Chosen countries, a group or a top N, not more than one");
        }
        if params.top == Some(0) {
            return Err("top must be at least 1");
        }
        Ok(())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryFactories]
    }

    fn retained_reads(params: &Self::Params, _owner: Option<Owner>, stage: usize, _held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        if stage > 0 {
            return (Vec::new(), i32::MAX);
        }
        let valid = at.context.members.valid_through(params.group.as_ref(), at.hour);
        if let Some(group) = &params.group {
            let Ok(members) = at.context.members.members(group, at.hour) else {
                return (Vec::new(), valid);
            };
            if members.refused.is_some() || members.tags.is_empty() {
                return (Vec::new(), valid);
            }
            let keys: Vec<u128> = members
                .tags
                .iter()
                .filter_map(|tag| match at.holders.at(&Country::Tag(tag.clone()), at.hour).map(|run| run.holder) {
                    Some(Holder::Found { id, .. }) => Some(u128::from(id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(TableId::CountryFactories, keys)], valid);
        }
        if !params.countries.is_empty() {
            let keys: Vec<u128> = params
                .countries
                .iter()
                .filter_map(|country| match at.holders.at(country, at.hour).map(|run| run.holder) {
                    Some(Holder::Found { id, .. }) => Some(u128::from(id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(TableId::CountryFactories, keys)], i32::MAX);
        }
        (vec![Read::all(TableId::CountryFactories)], i32::MAX)
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.members.valid_through(params.group.as_ref(), at.hour)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(FactoryMixContext {
            groups: Arc::new(factory_groups::load_groups(source, load)),
            members: GroupsAt::default(),
        })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        Ok(FactoryMixContext {
            groups: Arc::new(factory_groups::load_groups(source, load)),
            members: GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?,
        })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let chosen = chosen_groups(&params.types);
        let parts = mix_parts(&at.context.groups, &params.mode, &chosen);
        let mut warnings = at.context.groups.warnings.clone();
        if let Some(missing) = &at.context.groups.missing {
            warnings.push(missing.clone());
        }

        let explicit: Option<Vec<Country>> = if let Some(group) = &params.group {
            let members = at.context.members.members(group, frame.hour)?;
            if let Some(refused) = members.refused {
                return Ok(FactoryMixReply {
                    hour: frame.hour,
                    mode: params.mode.clone(),
                    parts,
                    rows: Vec::new(),
                    missing: Some(refused),
                    warnings,
                });
            }
            Some(members.tags.into_iter().map(Country::Tag).collect())
        } else if !params.countries.is_empty() {
            Some(params.countries.clone())
        } else {
            None
        };

        if let Some(asked) = explicit {
            let mut out = Vec::with_capacity(asked.len());
            for country in &asked {
                let tag = at.holders.name(country, frame.hour);
                match at.holders.at(country, frame.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => {
                        let resolved_country = Country::Slot(slot);
                        match witnessed::<tables::CountryFactories>(frame, u128::from(id)) {
                            Witness::Row(row) => {
                                let (total, values, shares, missing, parts_missing) = row_mix(row, &params.mode, &chosen);
                                out.push(FactoryMixRow {
                                    country: resolved_country,
                                    tag,
                                    total,
                                    values,
                                    shares,
                                    missing,
                                    parts_missing,
                                });
                            }
                            Witness::Absent => out.push(FactoryMixRow {
                                country: resolved_country,
                                tag,
                                total: None,
                                values: vec![None; parts.len()],
                                shares: vec![None; parts.len()],
                                missing: Some(at.holders.say(country, frame.hour, |name| format!("country factories held no row for {name} at this hour"))),
                                parts_missing: None,
                            }),
                            Witness::NotRecorded(reason) => out.push(FactoryMixRow {
                                country: resolved_country,
                                tag,
                                total: None,
                                values: vec![None; parts.len()],
                                shares: vec![None; parts.len()],
                                missing: Some(reason),
                                parts_missing: None,
                            }),
                        }
                    }
                    _ => out.push(FactoryMixRow {
                        country: country.clone(),
                        tag,
                        total: None,
                        values: vec![None; parts.len()],
                        shares: vec![None; parts.len()],
                        missing: Some(at.holders.not_known(country, frame.hour).unwrap_or_else(|| "This country is not present at this hour".to_owned())),
                        parts_missing: None,
                    }),
                }
            }
            sort_rows(&mut out);
            return Ok(FactoryMixReply { hour: frame.hour, mode: params.mode.clone(), parts, rows: out, missing: None, warnings });
        }

        let held = match rows::<tables::CountryFactories>(frame) {
            Ok(held) => held,
            Err(reason) => {
                return Ok(FactoryMixReply {
                    hour: frame.hour,
                    mode: params.mode.clone(),
                    parts,
                    rows: Vec::new(),
                    missing: Some(format!("Factories are not known at this hour: {reason}")),
                    warnings,
                });
            }
        };
        let by_id = country_id_by_slot(at.holders, frame.hour);
        let unresolved = params.top.is_some() && holders_unresolved(at.holders, frame.hour);
        let mut out = Vec::new();
        let mut unread = 0usize;
        let mut unplaced = Vec::new();
        for row in held {
            let (total, values, shares, missing, parts_missing) = row_mix(row, &params.mode, &chosen);
            match by_id.get(&row.country) {
                Some((slot, tag)) => {
                    if total.is_none() {
                        unread += 1;
                    }
                    if params.top.is_some() && total.is_some_and(|value| value <= 0) {
                        continue;
                    }
                    out.push(FactoryMixRow {
                        country: Country::Slot(*slot),
                        tag: tag.clone(),
                        total,
                        values,
                        shares,
                        missing,
                        parts_missing,
                    });
                }
                None if unresolved => unplaced.push(total),
                None => {}
            }
        }
        sort_rows(&mut out);
        if let Some(limit) = params.top {
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            if unread > 0 {
                return Ok(FactoryMixReply {
                    hour: frame.hour,
                    mode: params.mode.clone(),
                    parts,
                    rows: Vec::new(),
                    missing: Some(format!("The top {limit} countries by factories cannot be proved: {unread} countries had unreadable factory totals at this hour")),
                    warnings,
                });
            }
            out.truncate(limit);
            let full = out.len() >= limit;
            let floor = out.last().and_then(|row| row.total);
            let contending = unplaced
                .iter()
                .filter(|value| match value {
                    Some(v) => *v > 0 && (!full || floor.is_none_or(|floor| *v >= floor)),
                    None => true,
                })
                .count();
            if contending > 0 {
                return Ok(FactoryMixReply {
                    hour: frame.hour,
                    mode: params.mode.clone(),
                    parts,
                    rows: Vec::new(),
                    missing: Some(format!("The top {limit} countries by factories cannot be proved: {contending} countries could not be matched to a tag at this hour")),
                    warnings,
                });
            }
        }
        Ok(FactoryMixReply { hour: frame.hour, mode: params.mode.clone(), parts, rows: out, missing: None, warnings })
    }
}
