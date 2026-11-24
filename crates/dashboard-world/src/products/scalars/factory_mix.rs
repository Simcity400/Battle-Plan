use super::{Named, branch, factory_groups, game_score::holders_unresolved, rows};
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour},
        validate_country,
    },
};
use dashboard_query::identity::Holder;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactoryMixRow {
    pub country: Country,
    pub tag: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "(number | null)[]"))]
    pub values: Vec<Option<i64>>,
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

pub(super) fn row_mix(row: &CountryFactoriesRow, mode: &str, chosen: &[usize]) -> (Option<i64>, Vec<Option<i64>>, Option<String>, Option<String>) {
    let mut total = 0i64;
    let mut group_totals = Vec::with_capacity(chosen.len());
    for &group in chosen {
        let Some(value) = group_count(row, group, mode) else {
            return (None, Vec::new(), Some("A factory total does not fit a whole number the game shows".to_owned()), None);
        };
        let Some(next) = total.checked_add(value) else {
            return (None, Vec::new(), Some("Factory total overflowed at this hour".to_owned()), None);
        };
        total = next;
        group_totals.push(value);
    }
    if mode == "built" {
        let mut undamaged = Vec::with_capacity(chosen.len());
        let mut damaged = Vec::with_capacity(chosen.len());
        for (&group, &built) in chosen.iter().zip(&group_totals) {
            let Some(dmg) = group_count(row, group, "damaged") else {
                return (Some(total), Vec::new(), None, Some("A damaged factory count does not fit a whole number the game shows".to_owned()));
            };
            if dmg < 0 || built < 0 || dmg > built {
                return (Some(total), Vec::new(), None, Some("Damaged factories do not reconcile with built factories at this hour".to_owned()));
            }
            undamaged.push(Some(built - dmg));
            damaged.push(Some(dmg));
        }
        undamaged.extend(damaged);
        return (Some(total), undamaged, None, None);
    }
    if group_totals.iter().any(|&value| value < 0) {
        return (Some(total), Vec::new(), None, Some("A factory group count is negative at this hour; shares cannot be shown".to_owned()));
    }
    (Some(total), group_totals.into_iter().map(Some).collect(), None, None)
}

impl AtHour for FactoryMix {
    const KEY: &'static str = "factory-mix";
    const TITLE: &'static str = "Factory mix";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = FactoryMixParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, mode, types, "countries[]"];
    type Reply = FactoryMixReply;
    type Context = Arc<factory_groups::FactoryGroupsReply>;

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
        match (params.countries.is_empty(), params.top) {
            (true, Some(0)) => Err("top must be at least 1"),
            (false, Some(_)) => Err("Chosen countries or a top N, not both"),
            _ => Ok(()),
        }
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryFactories]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Arc::new(factory_groups::load_groups(source, load)))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let chosen = chosen_groups(&params.types);
        let parts = mix_parts(at.context, &params.mode, &chosen);
        let mut warnings = at.context.warnings.clone();
        if let Some(missing) = &at.context.missing {
            warnings.push(missing.clone());
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
        if !params.countries.is_empty() {
            let by_id: HashMap<u32, &CountryFactoriesRow> = held.iter().map(|row| (row.country, row)).collect();
            let mut out = Vec::with_capacity(params.countries.len());
            for country in &params.countries {
                let tag = at.holders.name(country, frame.hour);
                match at.holders.at(country, frame.hour).map(|run| run.holder) {
                    Some(Holder::Found { id, .. }) => match by_id.get(&id) {
                        Some(row) => {
                            let (total, values, missing, parts_missing) = row_mix(row, &params.mode, &chosen);
                            out.push(FactoryMixRow { country: country.clone(), tag, total, values, missing, parts_missing });
                        }
                        None => out.push(FactoryMixRow {
                            country: country.clone(),
                            tag,
                            total: None,
                            values: Vec::new(),
                            missing: Some(at.holders.say(country, frame.hour, |name| format!("country factories held no row for {name} at this hour"))),
                            parts_missing: None,
                        }),
                    },
                    _ => out.push(FactoryMixRow {
                        country: country.clone(),
                        tag,
                        total: None,
                        values: Vec::new(),
                        missing: Some("This country is not present at this hour".to_owned()),
                        parts_missing: None,
                    }),
                }
            }
            return Ok(FactoryMixReply { hour: frame.hour, mode: params.mode.clone(), parts, rows: out, missing: None, warnings });
        }

        let named = Named::new(at.holders, frame.hour);
        let unresolved = params.top.is_some() && holders_unresolved(at.holders, frame.hour);
        let mut out = Vec::new();
        let mut unread = 0usize;
        let mut unplaced = Vec::new();
        for row in held {
            let (total, values, missing, parts_missing) = row_mix(row, &params.mode, &chosen);
            match named.id(row.country) {
                Some(tag) => {
                    if total.is_none() {
                        unread += 1;
                    }
                    if params.top.is_some() && total.is_some_and(|value| value <= 0) {
                        continue;
                    }
                    out.push(FactoryMixRow {
                        country: Country::from(tag.to_owned()),
                        tag: tag.to_owned(),
                        total,
                        values,
                        missing,
                        parts_missing,
                    });
                }
                None if unresolved => unplaced.push(total),
                None => {}
            }
        }
        out.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag),
        });
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
