use dashboard_base::products::Country;
use dashboard_base::products::LoadContext;
use super::army::ArmyIdentityRun;
use dashboard_world::products::army_windows::ArmySpan;
use dashboard_base::Source;
use dashboard_base::catalogue::{self, Catalogue, View};
use dashboard_query::{Query, identity::Holder, series::Part};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const UNIDENTIFIED: &str = ":not-identified";
const UNRESOLVED: &str = ":";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirForceMeasure {
    Planes,
    Wings,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirForceOwnership {
    Roster,
    Effective,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyMeasure {
    Ships,
    Strength,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyOwnership {
    Roster,
    Logical,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavyGrouping {
    Class,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForcePart {
    pub key: String,
    pub name: Option<String>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    #[doc = " Null where the total is not recorded."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    #[doc = " Parallel to the reply's parts; every one null where the hour's rows do not split the total."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number | null>"))]
    pub values: Vec<Option<i64>>,
    #[doc = " The navy's maximum hull HP (fixed5) for the strength measure, null for ships or where not"]
    #[doc = " recorded; absent for the air force."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, type = "number | null"))]
    pub reference_total: Option<Option<i64>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceSeries {
    pub country: Country,
    pub identities: Vec<ArmyIdentityRun>,
    pub runs: Vec<ForceRun>,
    pub segments: Vec<ArmySpan>,
    pub membership: Option<Vec<ArmySpan>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ForceSizeReply {
    pub branch_id: String,
    pub windows: Vec<ArmySpan>,
    pub window: Option<usize>,
    pub interval: Option<ArmySpan>,
    pub parts: Vec<ForcePart>,
    pub series: Vec<ForceSeries>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ForceSizeReply);

pub(crate) fn validate_selection(branch_id: &str, tags: &[Country], top: Option<usize>) -> Result<(), &'static str> {
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if top.is_some_and(|top| top == 0 || !tags.is_empty()) {
        return Err("Invalid country selection");
    }
    for tag in tags {
        dashboard_base::products::validate_country(tag)?;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Named {
    pub(crate) parts: Vec<ForcePart>,
    pub(crate) of: HashMap<u128, usize>,
    pub(crate) order: BTreeMap<String, u32>,
}

impl Named {
    pub(crate) fn group(resolved: BTreeMap<u128, ForcePart>) -> Self {
        let mut keys: BTreeMap<String, ForcePart> = BTreeMap::new();
        for part in resolved.values() {
            keys.entry(part.key.clone()).or_insert_with(|| part.clone());
        }
        let parts: Vec<ForcePart> = keys.into_values().collect();
        let at: HashMap<&str, usize> = parts.iter().enumerate().map(|(index, part)| (part.key.as_str(), index)).collect();
        let of = resolved.iter().map(|(part, named)| (*part, at[named.key.as_str()])).collect();
        Self { parts, of, order: BTreeMap::new() }
    }

    pub(crate) fn in_content_order(mut self, keys: impl IntoIterator<Item = String>) -> Self {
        for key in keys {
            let next = self.order.len() as u32;
            self.order.entry(key).or_insert(next);
        }
        self
    }

    pub(crate) fn ordinals(&self) -> Vec<Option<u32>> {
        self.parts.iter().map(|part| self.order.get(&part.key).copied()).collect()
    }
}

fn split(total: Option<i64>, held: &[(usize, Option<i64>)], groups: usize) -> (Vec<Option<i64>>, bool) {
    let mut out = vec![Some(0i64); groups];
    for &(group, value) in held {
        out[group] = match (out[group], value) {
            (Some(sum), Some(value)) => sum.checked_add(value),
            _ => None,
        };
    }
    let known: Option<i128> = out.iter().map(|value| value.map(i128::from)).sum();
    let reconciled = total.is_some_and(|total| known == Some(i128::from(total)));
    (out, reconciled)
}

pub fn segments(query: &Query<'_>, tag: &Country, first: i32, last: i32) -> Vec<ArmySpan> {
    query
        .holders
        .runs(tag)
        .iter()
        .filter(|run| matches!(run.holder, Holder::Found { .. }) && run.first <= last && run.last >= first)
        .map(|run| ArmySpan { first_hour: run.first, last_hour: run.last.min(last) })
        .collect()
}

pub fn identities(parts: &[Part]) -> Vec<ArmyIdentityRun> {
    parts.iter().fold(Vec::<ArmyIdentityRun>::new(), |mut held, part| {
        match held.last_mut() {
            Some(last) if last.slot == part.slot && last.country_id == part.id && last.last_hour + 1 == part.first => last.last_hour = part.last,
            _ => held.push(ArmyIdentityRun { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id }),
        }
        held
    })
}

struct Content {
    catalogue: Option<Arc<Catalogue>>,
}

impl Content {
    fn read(source: &Source, context: &LoadContext, what: &str, warnings: &mut Vec<String>) -> Self {
        let catalogue = match context.content().and_then(|options| catalogue::catalogue(source, options)) {
            Ok(catalogue) => {
                warnings.extend(catalogue.refusal_lines());
                Some(catalogue)
            }
            Err(error) => {
                warnings.push(format!("{what} are unavailable: {error}"));
                None
            }
        };
        Self { catalogue }
    }

    fn localise(&self, key: &str) -> Option<String> {
        let catalogue = self.catalogue.as_ref()?;
        catalogue.localisation.get(key)?;
        catalogue.localised(key, View::Plain)
    }
}

fn variant_part(variant: u64, token: Option<&str>, units: Option<&content_reader_key_names::equipment::Database>, content: &Content, warnings: &mut BTreeSet<String>) -> ForcePart {
    if let (Some(token), Some(units)) = (token, units) {
        let archetype = units
            .named(token)
            .ok_or_else(|| format!("{token} is not authored equipment"))
            .and_then(|kind| kind.archetype.ok_or_else(|| format!("equipment {token} has no known archetype")))
            .and_then(|ordinal| units.equipment[ordinal].name_str().map(str::to_owned).map_err(|error| error.to_string()));
        match archetype {
            Ok(key) if !key.starts_with(UNRESOLVED) => return ForcePart { name: content.localise(&key), key, unresolved: false },
            Ok(key) => {
                warnings.insert(format!("{token}: archetype {key} uses the reserved unresolved mark; its variants remain separate."));
            }
            Err(reason) => {
                warnings.insert(format!("{reason}; its variants remain separate."));
            }
        }
    }
    let key = match token {
        Some(token) => format!("{UNRESOLVED}variant/{variant}/{token}"),
        None => format!("{UNRESOLVED}variant/{variant}"),
    };
    ForcePart { key, name: Some(format!("Variant {variant}")), unresolved: true }
}

fn unidentified() -> ForcePart {
    ForcePart { key: UNIDENTIFIED.into(), name: Some("Other / not identified".into()), unresolved: true }
}

fn equipment(content: &Content, warnings: &mut Vec<String>) -> Option<Arc<content_reader_key_names::equipment::Database>> {
    let catalogue = content.catalogue.as_ref()?;
    match catalogue::equipment_units(catalogue) {
        Ok(units) => {
            warnings.extend(units.misreads.iter().cloned());
            Some(units)
        }
        Err(error) => {
            warnings.push(format!("Air equipment content is unavailable: {error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests;
pub(crate) mod at_hour;
pub use at_hour::{AirForceAt, AirForceAtParams, ForceAtReply, ForceAtRow, NavyAt, NavyAtParams};
