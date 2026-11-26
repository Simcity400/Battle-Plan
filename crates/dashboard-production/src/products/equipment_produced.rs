
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::group_members::GroupsAt;
use super::production_spending::{self as spending, Content, Domain, Read, Span};
use dashboard_base::Source;
use dashboard_query::{
    Query, cost,
    identity::Holder,
    series::{ranked::GroupSelection, totals::Gap},
};
use replay_lanes::lanes::{self, Keys};
use replay_columnar::manifest::Status;
use replay_data::rows::{ProducedValueRow, ProductionCoverageRow, ProductionTypeTotalRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod measure;
pub mod tiles;
mod value;

use value::{Amount, Choice, Coverage, DesignRow, RocketRow};

pub struct EquipmentProducedTypes;
pub struct EquipmentProducedAt;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProducedCount {
    #[default]
    Units,
    Ic,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProducedMeasure {
    Total,
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProducedUnknown {
    NotRecorded,
    UnitsInexact,
    SpendingUnavailable,
    LostOutput,
    MissedPublication,
    TypeNotRecorded,
    WindowIncomplete,
    ObservationRestarted,
    RocketTableUnavailable,
    RocketInputInexact,
    RocketOwnerUnreadable,
    OutOfRange,
}

const MAX_EXACT: i128 = (1 << 53) - 1;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedType {
    pub key: String,
    pub name: Option<String>,
    pub domain: Option<Domain>,
}

pub const UNRECORDED_TYPE: &str = ":unrecorded";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProducedTypesParams {
    pub branch_id: String,
    pub refits: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedTypesReply {
    pub branch_id: String,
    #[doc = " Every type any country produced in the branch, in the equipment database's order, the"]
    #[doc = " ones content does not resolve last."]
    pub types: Vec<ProducedType>,
    #[doc = " The type with the most units delivered, every country together, at each run's end: the"]
    #[doc = " widget's type until one is chosen. Null when nothing was."]
    pub most_delivered: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProducedAtParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    pub measure: ProducedMeasure,
    pub refits: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedAmount {
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub value: Option<i64>,
    pub reason: Option<ProducedUnknown>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedModelAt {
    pub variant: String,
    pub rocket: bool,
    pub type_key: String,
    pub type_name: Option<String>,
    pub domain: Option<Domain>,
    pub type_recorded: bool,
    #[doc = " The design's name as recorded at the hour; null where not known."]
    pub name: Option<String>,
    pub units: ProducedAmount,
    pub ic: ProducedAmount,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedAtReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub hour: i32,
    pub measure: ProducedMeasure,
    #[doc = " The country's production totals are recorded at the hour: a design it has no row of is zero."]
    pub recorded: bool,
    pub models: Vec<ProducedModelAt>,
    #[doc = " Every design's IC together."]
    pub ic_total: ProducedAmount,
    #[doc = " When the running totals began counting, as of the hour."]
    pub observation: Option<i32>,
    #[doc = " The latest read of production lines' output at or before the hour."]
    pub read: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProducedTypesReply, ProducedAtReply);

const COUNTRY_COLUMNS: [&str; 6] = ["units", "units_inexact", "spent", "units_without_refits", "units_without_refits_inexact", "spent_without_refits"];
const COVERAGE_COLUMNS: [&str; 4] = ["observation_since", "units_unknown_from", "spending_unknown_from", "spending_reason"];
const ROCKET_COLUMNS: [&str; 3] = ["units", "units_exact", "spent_raw"];
const DESIGN_COLUMNS: [&str; 5] = ["units", "inexact_rows", "spent", "spent_reason", "archetype"];

const RUN: u128 = ProductionCoverageRow::RUN as u128;
const WORLD: u128 = ProductionTypeTotalRow::WORLD as u128;

fn coverage_keys(id: u32) -> [u128; 3] {
    let id = u128::from(id);
    [id << 8 | u128::from(ProductionCoverageRow::ROCKET), RUN << 8 | u128::from(ProductionCoverageRow::LINES), RUN << 8 | u128::from(ProductionCoverageRow::ROCKET)]
}

fn rocket_designs(id: u32) -> (u128, u128) {
    (u128::from(id) << 64 | 1, u128::from(id) << 64 | u128::from(u64::MAX))
}

fn coverage(number: impl Fn(usize) -> Option<i64>) -> Coverage {
    let hour = |at| number(at).and_then(|hour| i32::try_from(hour).ok());
    Coverage { observation: hour(0), units_unknown_from: hour(1), spending_unknown_from: hour(2), reason: number(3).map_or(0, |code| code as u8) }
}

fn rocket_row(number: impl Fn(usize) -> Option<i64>) -> RocketRow {
    let units = number(0);
    RocketRow { units: units.unwrap_or(0), units_exact: units.is_some() && number(1) == Some(1), spent: number(2).unwrap_or(0) }
}

#[derive(Clone, Debug)]
struct Typed {
    kind: ProducedType,
    order: Option<usize>,
}

fn typed(content: &Content, spelling: Option<&str>, notes: &mut BTreeSet<String>) -> Typed {
    let Some(spelling) = spelling else {
        notes.insert("An equipment type's spelling was not recorded; its designs stay apart.".to_owned());
        return Typed { kind: ProducedType { key: UNRECORDED_TYPE.to_owned(), name: None, domain: None }, order: None };
    };
    match content.archetype(spelling) {
        Some((key, name, domain)) => {
            let order = content.units.as_ref().and_then(|units| units.get(&key));
            Typed { kind: ProducedType { key, name, domain: Some(domain) }, order }
        }
        None => {
            if content.units.is_some() {
                notes.insert(format!("Equipment type {spelling} is not in the recorded content; its designs stay apart."));
            }
            Typed { kind: ProducedType { key: format!("{}{spelling}", spending::UNRESOLVED_MARK), name: None, domain: None }, order: None }
        }
    }
}

fn runs(query: &Query<'_>) -> BTreeMap<u32, (i32, i32)> {
    let mut out: BTreeMap<u32, (i32, i32)> = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        let held = out.entry(stretch.run).or_insert((stretch.first, stretch.last));
        held.0 = held.0.min(stretch.first);
        held.1 = held.1.max(stretch.last);
    }
    out
}

fn branch_types(source: &Source, query: &Query<'_>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<BTreeMap<u32, BTreeMap<u32, Typed>>> {
    let every: BTreeSet<u32> = runs(query).into_keys().collect();
    run_types(source, query, content, &every, notes)
}

fn run_types(source: &Source, query: &Query<'_>, content: &Content, wanted: &BTreeSet<u32>, notes: &mut BTreeSet<String>) -> io::Result<BTreeMap<u32, BTreeMap<u32, Typed>>> {
    let mut out = BTreeMap::new();
    for (run, (first, last)) in runs(query).into_iter().filter(|(run, _)| wanted.contains(run)) {
        let lives = dashboard_base::queries::lifetimes(source, run, TableId::ProductionTypeTotals)?;
        let world = &lives[lives.partition_point(|&(key, ..)| key >> 32 < WORLD)..];
        let tokens: BTreeSet<u32> = world.iter().filter(|&&(_, born, ends)| born <= i64::from(last) && ends >= i64::from(first)).map(|(key, ..)| *key as u32).collect();
        let asked: BTreeSet<u32> = tokens.iter().copied().filter(|&token| token != ProductionTypeTotalRow::NO_ARCHETYPE).collect();
        let spelled = dashboard_base::queries::spellings(source, run, last, &asked)?;
        let types = tokens
            .into_iter()
            .map(|token| {
                let kind = if token == ProductionTypeTotalRow::NO_ARCHETYPE {
                    Typed { kind: ProducedType { key: UNRECORDED_TYPE.to_owned(), name: None, domain: None }, order: None }
                } else {
                    typed(content, spelled.get(&token).map(String::as_str), notes)
                };
                (token, kind)
            })
            .collect();
        out.insert(run, types);
    }
    Ok(out)
}

fn ordered(types: &BTreeMap<u32, BTreeMap<u32, Typed>>) -> Vec<Typed> {
    let mut by_key: BTreeMap<String, Typed> = BTreeMap::new();
    for typed in types.values().flat_map(BTreeMap::values) {
        by_key.entry(typed.kind.key.clone()).or_insert_with(|| typed.clone());
    }
    let mut out: Vec<Typed> = by_key.into_values().collect();
    out.sort_by(|a, b| (a.order.is_none(), a.order, &a.kind.key).cmp(&(b.order.is_none(), b.order, &b.kind.key)));
    out
}

fn reads_in(query: &Query<'_>, run: u32, first: i32, last: i32) -> io::Result<Vec<i32>> {
    let span = Span { run, first, last, id: 0, breaks: false };
    let reads = spending::reads_of(query, &span, TableId::ProductionLineOutput)?;
    if !reads.is_empty() {
        return Ok(reads);
    }
    let world = Read::of(query, &span, TableId::ProductionTypeTotals, &["units", "spent"], Keys::Range(WORLD << 32, WORLD << 32 | u128::from(u32::MAX)))?;
    let hours: BTreeSet<i32> = world.history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)).filter(|&hour| first <= hour && hour <= last).collect();
    Ok(hours.into_iter().collect())
}

fn snapshot(query: &Query<'_>, run: u32, table: TableId, hour: i32, names: &[&str], keys: Keys<'_>) -> io::Result<Option<HashMap<u128, Vec<Option<i64>>>>> {
    let columns = names.iter().map(|name| dashboard_declare::field(table, name)).collect::<io::Result<Vec<_>>>()?;
    let read = cost::storage(|| lanes::at_keys(query.replay.recording(), run, table, hour, Some(&columns), keys))?;
    if !matches!(read.status, Status::Complete) {
        return Ok(None);
    }
    let view = read.view();
    Ok(Some(read.rows.iter().map(|&(key, slot)| (key, columns.iter().map(|&column| view.number(column, slot)).collect())).collect()))
}

fn decimals(count: ProducedCount) -> u8 {
    match count {
        ProducedCount::Units => 0,
        ProducedCount::Ic => 5,
    }
}

fn back_of(measure: ProducedMeasure) -> Option<fn(i32) -> Option<i32>> {
    match measure {
        ProducedMeasure::Total => None,
        ProducedMeasure::Day => Some(|hour: i32| hour.checked_sub(24)),
        ProducedMeasure::Week => Some(|hour: i32| hour.checked_sub(7 * 24)),
        ProducedMeasure::Month => Some(measure::month_before),
    }
}

fn produced_measure(count: ProducedCount, refits: bool) -> &'static str {
    match (count, refits) {
        (ProducedCount::Units, true) => "units",
        (ProducedCount::Units, false) => "unitsWithoutRefits",
        (ProducedCount::Ic, true) => "ic",
        (ProducedCount::Ic, false) => "icWithoutRefits",
    }
}

fn unknown_of(code: u8) -> ProducedUnknown {
    match code {
        ProducedValueRow::UNITS_INEXACT => ProducedUnknown::UnitsInexact,
        ProducedValueRow::SPENDING_UNAVAILABLE => ProducedUnknown::SpendingUnavailable,
        ProducedValueRow::LOST_OUTPUT => ProducedUnknown::LostOutput,
        ProducedValueRow::MISSED_PUBLICATION => ProducedUnknown::MissedPublication,
        ProducedValueRow::TYPE_NOT_RECORDED => ProducedUnknown::TypeNotRecorded,
        ProducedValueRow::ROCKET_TABLE_UNAVAILABLE => ProducedUnknown::RocketTableUnavailable,
        ProducedValueRow::ROCKET_INPUT_INEXACT => ProducedUnknown::RocketInputInexact,
        ProducedValueRow::ROCKET_OWNER_UNREADABLE => ProducedUnknown::RocketOwnerUnreadable,
        ProducedValueRow::OUT_OF_RANGE => ProducedUnknown::OutOfRange,
        _ => ProducedUnknown::NotRecorded,
    }
}

fn amount_of_total(value: Result<i64, Gap>) -> Amount {
    match value {
        Ok(value) => Amount::Known(i128::from(value)),
        Err(Gap::NotRecorded) => Amount::Unknown(ProducedUnknown::NotRecorded),
        Err(Gap::Cell { reason, .. }) => Amount::Unknown(unknown_of(reason)),
        Err(Gap::WindowIncomplete) => Amount::Unknown(ProducedUnknown::WindowIncomplete),
        Err(Gap::Restarted) => Amount::Unknown(ProducedUnknown::ObservationRestarted),
        Err(Gap::OutOfRange) => Amount::Unknown(ProducedUnknown::OutOfRange),
    }
}

fn asked_parts(types: &BTreeMap<u32, BTreeMap<u32, Typed>>, asked: &[String]) -> BTreeMap<u32, Vec<u128>> {
    types
        .iter()
        .map(|(&run, held)| {
            let parts = match asked.is_empty() {
                true => vec![u128::from(ProducedValueRow::WHOLE)],
                false => held.iter().filter(|(_, typed)| asked.contains(&typed.kind.key)).map(|(&token, _)| u128::from(token)).collect(),
            };
            (run, parts)
        })
        .collect()
}

fn types_reply(source: &Source, query: &Query<'_>, params: &ProducedTypesParams, context: &LoadContext) -> io::Result<ProducedTypesReply> {
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let types = branch_types(source, query, &content, &mut notes)?;
    let column = if params.refits { "units" } else { "units_without_refits" };
    let mut delivered: HashMap<String, i128> = HashMap::new();
    let mut produced: BTreeSet<String> = BTreeSet::new();
    for (run, (_, last)) in runs(query) {
        let Some(rows) = snapshot(query, run, TableId::ProductionTypeTotals, last, &[column, "units", "spent"], Keys::Range(WORLD << 32, WORLD << 32 | u128::from(u32::MAX)))? else { continue };
        for (key, cells) in rows {
            let Some(typed) = types.get(&run).and_then(|types| types.get(&(key as u32))) else { continue };
            if cells[1] != Some(0) || cells[2] != Some(0) {
                produced.insert(typed.kind.key.clone());
            }
            if let (Some(units), false) = (cells[0], typed.kind.key == UNRECORDED_TYPE) {
                *delivered.entry(typed.kind.key.clone()).or_default() += i128::from(units);
            }
        }
    }
    let mut types = ordered(&types);
    types.retain(|typed| produced.contains(&typed.kind.key));
    let mut most: Option<(&str, i128)> = None;
    for typed in &types {
        let units = delivered.get(&typed.kind.key).copied().unwrap_or(0);
        if units > 0 && most.is_none_or(|(_, best)| units > best) {
            most = Some((&typed.kind.key, units));
        }
    }
    warnings.extend(notes);
    Ok(ProducedTypesReply {
        branch_id: params.branch_id.clone(),
        most_delivered: most.map(|(key, _)| key.to_owned()),
        types: types.into_iter().map(|typed| typed.kind).collect(),
        warnings,
    })
}

type DesignKey = (bool, u64);

fn design_row(purpose: u8, read: impl Fn(usize) -> Option<i64>) -> DesignRow {
    let units = read(0);
    DesignRow {
        purpose,
        units: units.unwrap_or(0),
        inexact_rows: read(1).unwrap_or(1) + i64::from(units.is_none()),
        spent: read(2),
        spent_reason: read(3).map_or(0, |code| code as u8),
    }
}

fn unnamed(key: DesignKey) -> spending::Design {
    spending::Design { type_key: format!("{}variant/{}", spending::UNRESOLVED_MARK, key.1), name: None, domain: None, names: Vec::new() }
}

fn type_recorded(design: &spending::Design) -> bool {
    !design.type_key.starts_with(&format!("{}variant/", spending::UNRESOLVED_MARK))
}

struct DesignsAt {
    hour: i32,
    country: bool,
    lines: Option<Coverage>,
    rocket: [Option<Coverage>; 2],
    world: bool,
    observation: (Option<i32>, Option<i32>),
    designs: BTreeMap<DesignKey, Vec<DesignRow>>,
    rockets: BTreeMap<u64, RocketRow>,
    archetypes: BTreeMap<u64, u32>,
}

impl DesignsAt {
    fn read(query: &Query<'_>, run: u32, hour: i32, id: u32) -> io::Result<Self> {
        let owner = u128::from(id);
        let country = snapshot(query, run, TableId::ProductionCountryTotals, hour, &COUNTRY_COLUMNS, Keys::These(&[owner]))?.is_some_and(|rows| rows.contains_key(&owner));
        let keys = coverage_keys(id);
        let cover = snapshot(query, run, TableId::ProductionCoverage, hour, &COVERAGE_COLUMNS, Keys::These(&keys))?.unwrap_or_default();
        let [own, lines, rocket_run] = keys.map(|key| cover.get(&key).map(|cells| coverage(|at| cells[at])));
        let mut designs: BTreeMap<DesignKey, Vec<DesignRow>> = BTreeMap::new();
        let mut archetypes = BTreeMap::new();
        let rows = snapshot(query, run, TableId::ProductionTotals, hour, &DESIGN_COLUMNS, Keys::Range(owner << 72, owner << 72 | ((1u128 << 72) - 1)))?.unwrap_or_default();
        for (key, cells) in &rows {
            let variant = (key >> 8) as u64;
            designs.entry((false, variant)).or_default().push(design_row(*key as u8, |at| cells[at]));
            let token = cells[4].filter(|&token| token > 0).map_or(0, |token| token as u32);
            let held = archetypes.entry(variant).or_insert(0);
            *held = (*held).max(token);
        }
        let world = snapshot(query, run, TableId::RocketProductionOutput, hour, &["spent_raw"], Keys::These(&[0]))?.is_some_and(|rows| rows.contains_key(&0));
        let (low, high) = rocket_designs(id);
        let rockets: BTreeMap<u64, RocketRow> = snapshot(query, run, TableId::RocketProductionOutput, hour, &ROCKET_COLUMNS, Keys::Range(low, high))?
            .unwrap_or_default()
            .into_iter()
            .map(|(key, cells)| (key as u64, rocket_row(|at| cells[at])))
            .collect();
        Ok(Self {
            hour,
            country,
            lines,
            rocket: [own, rocket_run],
            world,
            observation: (lines.and_then(|row| row.observation), rocket_run.and_then(|row| row.observation)),
            designs,
            rockets,
            archetypes,
        })
    }

    fn amount(&self, key: DesignKey, hour: i32, choice: Choice) -> measure::Counted {
        let amount = if key.0 {
            value::rocket_design(self.world, self.rocket, self.rockets.get(&key.1).copied(), hour, choice.count)
        } else {
            value::design(self.country, self.lines, self.designs.get(&key).map_or(&[][..], Vec::as_slice), hour, choice)
        };
        (amount, self.observation)
    }

    fn keys(&self) -> impl Iterator<Item = DesignKey> + '_ {
        self.designs.keys().copied().chain(self.rockets.keys().map(|&variant| (true, variant)))
    }
}

fn amount_of(amount: Amount) -> ProducedAmount {
    match amount {
        Amount::Known(value) if value.abs() <= MAX_EXACT => ProducedAmount { value: Some(value as i64), reason: None },
        Amount::Known(_) => ProducedAmount { value: None, reason: Some(ProducedUnknown::OutOfRange) },
        Amount::Unknown(reason) => ProducedAmount { value: None, reason: Some(reason) },
    }
}

fn at_reply(source: &Source, query: &Query<'_>, params: &ProducedAtParams, context: &LoadContext) -> io::Result<ProducedAtReply> {
    let mut warnings = Vec::new();
    let mut reply = ProducedAtReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.name(&params.country, params.hour),
        asked: params.country.clone(),
        hour: params.hour,
        measure: params.measure,
        recorded: false,
        models: Vec::new(),
        ic_total: ProducedAmount { value: None, reason: Some(ProducedUnknown::NotRecorded) },
        observation: None,
        read: None,
        warnings: Vec::new(),
    };
    let hour = params.hour;
    let (Some(stretch), Some(run)) = (query.timeline.at(hour).copied(), query.holders.at(&params.country, hour).copied()) else {
        return Ok(reply);
    };
    let Holder::Found { id, .. } = run.holder else {
        return Ok(reply);
    };
    let first = stretch.first.max(run.first);
    let reads = reads_in(query, stretch.run, first, hour)?;
    let Some(read) = measure::read_at(&reads, hour) else {
        reply.recorded = false;
        return Ok(reply);
    };
    let now = DesignsAt::read(query, stretch.run, read, id)?;
    let before = match measure::back(params.measure, read) {
        Some(start) if start >= first && reads.binary_search(&start).is_ok() => Some(DesignsAt::read(query, stretch.run, start, id)?),
        _ => None,
    };
    let at = |held: &DesignsAt, key: DesignKey, choice: Choice| held.amount(key, held.hour, choice);
    let windowed = |key: DesignKey, choice: Choice| match params.measure {
        ProducedMeasure::Total => at(&now, key, choice).0,
        _ => measure::difference(at(&now, key, choice), before.as_ref().map(|before| at(before, key, choice))),
    };
    let units = Choice { count: ProducedCount::Units, refits: params.refits };
    let ic = Choice { count: ProducedCount::Ic, refits: params.refits };
    let keys: BTreeSet<DesignKey> = now.keys().chain(before.iter().flat_map(DesignsAt::keys)).collect();
    let span = Span { run: stretch.run, first: read, last: read, id, breaks: false };
    let mut named: BTreeMap<u64, u32> = now.archetypes.clone();
    for key in &keys {
        named.entry(key.1).or_insert(0);
    }
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let found = spending::designs(query, &span, &named, &content, &mut notes)?;
    let mut total = Amount::Known(0);
    for key in keys {
        let (units, ic) = (windowed(key, units), windowed(key, ic));
        total = total.plus(ic);
        if units == Amount::Known(0) && ic == Amount::Known(0) {
            continue;
        }
        let design = found.get(&key.1).cloned().unwrap_or_else(|| unnamed(key));
        reply.models.push(ProducedModelAt {
            variant: key.1.to_string(),
            rocket: key.0,
            type_recorded: type_recorded(&design),
            name: design.names.first().and_then(|names| names.name.clone()),
            type_key: design.type_key,
            type_name: design.name,
            domain: design.domain,
            units: amount_of(units),
            ic: amount_of(ic),
        });
    }
    let country = |held: &DesignsAt| -> Amount {
        if !held.country {
            return Amount::Unknown(ProducedUnknown::NotRecorded);
        }
        Amount::Known(0)
    };
    reply.ic_total = amount_of(country(&now).plus(match params.measure {
        ProducedMeasure::Total => Amount::Known(0),
        _ => before.as_ref().map_or(Amount::Unknown(ProducedUnknown::WindowIncomplete), country),
    }).plus(total));
    reply.recorded = now.country;
    reply.observation = now.observation.0;
    reply.read = Some(read);
    warnings.extend(notes);
    reply.warnings = warnings;
    Ok(reply)
}

fn branch(text: &str) -> io::Result<u64> {
    text.parse().map_err(|_| invalid("Invalid branch"))
}

impl Product for EquipmentProducedTypes {
    const KEY: &'static str = "equipment-produced-types";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProducedTypesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, refits];
    type Reply = ProducedTypesReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProducedTypesParams::decl(), ProducedTypesReply::decl(), ProducedType::decl(), Domain::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        types_reply(source, &query, params, context)
    }
}

impl Product for EquipmentProducedAt {
    const KEY: &'static str = "equipment-produced-at";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ProducedAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, measure, refits];
    type Reply = ProducedAtReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProducedAtParams::decl(), ProducedAtReply::decl(), ProducedAmount::decl(), ProducedModelAt::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        at_reply(source, &query, params, context)
    }
}

pub struct EquipmentProducedRace;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquipmentProducedRaceParams {
    pub branch_id: String,
    pub hour: i32,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[serde(default)]
    pub types: Vec<String>,
    pub refits: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProducedRaceRow {
    pub country: Country,
    pub tag: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub value: Option<i64>,
    pub unknown: Option<ProducedUnknown>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProducedRaceReply {
    pub branch_id: String,
    pub hour: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    pub decimals: u8,
    pub types: Vec<String>,
    pub rows: Vec<EquipmentProducedRaceRow>,
    pub read_hour: Option<i32>,
    pub observation_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EquipmentProducedRaceReply);

fn population_bounds(query: &Query<'_>, hour: i32, first_hour: i32, last_hour: i32) -> (i32, i32) {
    let (lo, hi) = query.holders.interval(hour);
    (first_hour.max(lo), last_hour.min(hi))
}

fn race_reply(
    source: &Source,
    query: &Query<'_>,
    params: &EquipmentProducedRaceParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<EquipmentProducedRaceReply> {
    let mut warnings = Vec::new();
    let hour = params.hour;
    let Some(stretch) = query.timeline.at(hour).copied() else {
        return Ok(EquipmentProducedRaceReply {
            branch_id: params.branch_id.clone(),
            hour,
            first_hour: hour,
            last_hour: hour,
            count: params.count,
            measure: params.measure,
            decimals: decimals(params.count),
            types: params.types.clone(),
            rows: Vec::new(),
            read_hour: None,
            observation_hour: None,
            missing: Some("This hour is not recorded on this branch".to_owned()),
            warnings,
        });
    };
    let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
    let measure = series.measure(produced_measure(params.count, params.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
    let line = dashboard_query::series::Line::new(query, series, measure)?;
    let baseline = tiles::range_baseline(&line, query, params.measure, params.from_hour)?;
    if let tiles::Baseline::Unavailable(reason) = baseline {
        return Ok(EquipmentProducedRaceReply {
            branch_id: params.branch_id.clone(),
            hour,
            first_hour: hour,
            last_hour: hour,
            count: params.count,
            measure: params.measure,
            decimals: decimals(params.count),
            types: params.types.clone(),
            rows: Vec::new(),
            read_hour: None,
            observation_hour: None,
            missing: Some(reason.to_owned()),
            warnings,
        });
    }
    let fixed_back = back_of(params.measure);
    let range_fn = |base: i32| move |read: i32| (read >= base).then_some(base);
    let range_closure = baseline.hour().map(range_fn);
    let back: Option<&dyn Fn(i32) -> Option<i32>> = match (&range_closure, &fixed_back) {
        (Some(func), _) => Some(func as &dyn Fn(i32) -> Option<i32>),
        (None, Some(func)) => Some(func as &dyn Fn(i32) -> Option<i32>),
        (None, None) => None,
    };
    let mut met = BTreeSet::new();
    met.insert(stretch.run);
    let mut notes = BTreeSet::new();
    let types: BTreeMap<u32, BTreeMap<u32, Typed>> = match params.types.is_empty() {
        true => met.iter().map(|&run| (run, BTreeMap::new())).collect(),
        false => {
            let content = Content::read(source, context, &mut warnings);
            run_types(source, query, &content, &met, &mut notes)?
        }
    };
    let parts = asked_parts(&types, &params.types);
    let parts_of = |run: u32| parts.get(&run).cloned().unwrap_or_default();
    dashboard_text::metadata::check_cancel(cancelled)?;

    let clock = line.clock_at(hour)?;
    let read_hour = clock.read;
    let observation_hour = clock.since.filter(|&since| since <= hour);
    if let tiles::Baseline::Opening(start) = baseline
        && clock.since != Some(start)
    {
        return Ok(EquipmentProducedRaceReply {
            branch_id: params.branch_id.clone(),
            hour,
            first_hour: clock.since_from.unwrap_or(hour).min(hour),
            last_hour: clock.next_change.map_or(stretch.last, |h| h - 1).min(stretch.last).max(hour),
            count: params.count,
            measure: params.measure,
            decimals: decimals(params.count),
            types: params.types.clone(),
            rows: Vec::new(),
            read_hour,
            observation_hour,
            missing: Some(tiles::unknown_reason(dashboard_query::series::totals::Gap::Restarted).to_owned()),
            warnings,
        });
    }
    let next_read = clock.next_change;
    let mut first_hour = read_hour.unwrap_or(stretch.first).max(stretch.first).max(clock.since_from.unwrap_or(i32::MIN));
    let mut last_hour = next_read.map_or(stretch.last, |h| h - 1).min(stretch.last);

    let mut missing = None;
    let (tags, is_population): (Vec<Country>, bool) = if let Some(group) = &params.group {
        let groups = GroupsAt::load(source, context, &params.branch_id, Some(group))?;
        let members = groups.members(group, hour)?;
        if let Some(refused) = members.refused {
            return Ok(EquipmentProducedRaceReply {
                branch_id: params.branch_id.clone(),
                hour,
                first_hour: members.from.max(first_hour).min(hour),
                last_hour: members.until.min(last_hour).max(hour),
                count: params.count,
                measure: params.measure,
                decimals: decimals(params.count),
                types: params.types.clone(),
                rows: Vec::new(),
                read_hour,
                observation_hour,
                missing: Some(refused),
                warnings,
            });
        }
        first_hour = first_hour.max(members.from);
        last_hour = last_hour.min(members.until);
        (members.tags.into_iter().map(Country::Tag).collect(), false)
    } else if !params.countries.is_empty() {
        (params.countries.clone(), false)
    } else {
        let limit = params.top.map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
        match line.totals_ranked(hour, limit, &parts_of, back, &|value| value > 0)? {
            Ok(ranked) => (
                ranked
                    .into_iter()
                    .map(|tag| {
                        let country = Country::from(tag);
                        match query.holders.at(&country, hour).map(|run| run.holder) {
                            Some(held @ Holder::Found { slot, .. }) if query.holders.at(&Country::Slot(slot), hour).is_some_and(|run| run.holder == held) => Country::Slot(slot),
                            _ => country,
                        }
                    })
                    .collect(),
                true,
            ),
            Err(reason) => {
                missing = Some(reason);
                (Vec::new(), true)
            }
        }
    };
    let runs = if tags.is_empty() { Vec::new() } else { line.totals(&tags, &parts_of, back, hour, hour)? };
    let mut rows = Vec::with_capacity(tags.len());
    for (asked, country_runs) in tags.into_iter().zip(runs) {
        let tag = query.holders.name(&asked, hour);
        let country = match query.holders.at(&asked, hour) {
            Some(holder_run) => {
                first_hour = first_hour.max(holder_run.first);
                last_hour = last_hour.min(holder_run.last);
                match holder_run.holder {
                    Holder::Found { slot, .. } if !is_population => Country::Slot(slot),
                    _ => asked,
                }
            }
            None => asked,
        };
        let amount = country_runs
            .into_iter()
            .find(|run| run.first <= hour && hour <= run.last)
            .map_or(Amount::Unknown(ProducedUnknown::NotRecorded), |run| amount_of_total(run.value));
        let (value, unknown) = match amount {
            Amount::Known(val) if val.abs() <= MAX_EXACT => (Some(val as i64), None),
            Amount::Known(_) => (None, Some(ProducedUnknown::OutOfRange)),
            Amount::Unknown(reason) => (None, Some(reason)),
        };
        rows.push(EquipmentProducedRaceRow { country, tag, value, unknown });
    }
    if is_population {
        (first_hour, last_hour) = population_bounds(query, hour, first_hour, last_hour);
    }
    if !is_population {
        rows.sort_unstable_by(|a, b| match (a.value, b.value) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)).then_with(|| a.country.cmp(&b.country)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag).then_with(|| a.country.cmp(&b.country)),
        });
    }
    let first_hour = first_hour.min(hour);
    let last_hour = last_hour.max(hour);
    warnings.extend(notes);
    let reply = EquipmentProducedRaceReply {
        branch_id: params.branch_id.clone(),
        hour,
        first_hour,
        last_hour,
        count: params.count,
        measure: params.measure,
        decimals: decimals(params.count),
        types: params.types.clone(),
        rows,
        read_hour,
        observation_hour,
        missing,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

impl Product for EquipmentProducedRace {
    const KEY: &'static str = "equipment-produced-race";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = EquipmentProducedRaceParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, group, top, count, measure, types, refits, fromHour: Start, "countries[]"];
    type Reply = EquipmentProducedRaceReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for country in &params.countries {
            validate_country(country)?;
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

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ProducedCount::decl(),
            ProducedMeasure::decl(),
            ProducedUnknown::decl(),
            EquipmentProducedRaceParams::decl(),
            EquipmentProducedRaceRow::decl(),
            EquipmentProducedRaceReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        race_reply(source, &query, params, context, cancelled)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryLinkRow, CountryRow, ProductionLineOutputRow, Recorded, RocketProductionOutputRow, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;

    pub(super) fn two_producers(name: &str, first: i32, last: i32) -> Source {
        two_producers_read_from(name, first, last, first, true, None, None)
    }

    pub(super) fn two_producers_read_from(name: &str, first: i32, last: i32, read_from: i32, second_exact: bool, rename_at: Option<i32>, restart_at: Option<i32>) -> Source {
        let frames = (first..=last)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!("the fixture holds its countries") };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..countries[0].clone() });
                if rename_at.is_some_and(|at| hour >= at) {
                    countries[0].tag = b"DEU".to_vec();
                }
                *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 11, canonical_slot: 7 },
                    CountryLinkRow { id: 9001, canonical_slot: 7 },
                    CountryLinkRow { id: 9002, canonical_slot: 8 },
                ]));
                let output = |line: u64, country: u32, units: i64| ProductionLineOutputRow { line, ordinal: 0, country, variant: 100, mio: 0, units, since_hour: first, observed_since_creation: true, ended: false, exact: true, spent_raw: Recorded::Value(None), purpose: Recorded::Value(1), ..Default::default() };
                let mut rows = vec![output(1, 9001, i64::from((hour - first) / 24))];
                if hour >= first + 100 {
                    rows.push(ProductionLineOutputRow { exact: second_exact, ..output(2, 9002, i64::from((hour - first - 100) / 12)) });
                }
                let since_hour = restart_at.filter(|&at| hour >= at).unwrap_or(read_from);
                let world = RocketProductionOutputRow { since_hour, exact: true, units_exact: true, ..Default::default() };
                if hour < read_from {
                    *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable { reason: "production not read yet".into() };
                    *frame.tables[TableId::RocketProductionOutput.index()] = TableView::Unavailable { reason: "production not read yet".into() };
                } else {
                    *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(rows));
                    *frame.tables[TableId::RocketProductionOutput.index()] = TableView::Complete(TableRows::RocketProductionOutput(vec![world]));
                }
                frame.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: name.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }
}
