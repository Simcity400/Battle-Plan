
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::group_members::GroupsAt;
use super::production_spending::{self as spending, Content, Domain, Read, Span};
use dashboard_base::Source;
use dashboard_query::{
    Query, cost,
    identity::Holder,
    read::{self, CoveredValue},
    series::{ranked::GroupSelection, totals::Gap},
};
use replay_lanes::lanes::{self, Keys};
use replay_columnar::manifest::Status;
use replay_data::rows::{ProducedValueRow, ProductionCoverageRow, ProductionTypeTotalRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
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
    pub types_missing: Option<String>,
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
    #[doc = " A Range's first hour: a total counts the deliveries from it on."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
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

fn witnessed_production(value: CoveredValue) -> bool {
    !matches!(value, CoveredValue::Value(0) | CoveredValue::Uncovered)
}

fn types_reply(source: &Source, query: &Query<'_>, params: &ProducedTypesParams, context: &LoadContext) -> io::Result<ProducedTypesReply> {
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let types = branch_types(source, query, &content, &mut notes)?;
    let column = if params.refits { "units" } else { "units_without_refits" };
    let mut delivered: HashMap<String, i128> = HashMap::new();
    let mut produced: BTreeSet<String> = BTreeSet::new();
    let mut types_missing = None;
    let table = TableId::ProductionTypeTotals;
    let columns = [column, "units", "spent"].map(|name| dashboard_declare::field(table, name));
    let [units, delivered_units, spent] = columns;
    let columns = [units?, delivered_units?, spent?];
    let recording = query.replay.recording();
    for (run, (first, last)) in runs(query) {
        let statuses = read::statuses(recording, run, table, first, last)?;
        let unread = statuses.iter().find(|status| status.status != Status::Complete);
        if let Some(status) = unread.filter(|_| types_missing.is_none()) {
            let reasons = read::status_reasons(recording, run, table, std::slice::from_ref(status))?;
            let reason = match status.status {
                Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
                _ => None,
            };
            types_missing = dashboard_query::coverage::row_coverage(table, status.status, reason, false).into_missing();
        }
        let read = cost::storage(|| lanes::at_keys(recording, run, table, last, Some(&columns), Keys::Range(WORLD << 32, WORLD << 32 | u128::from(u32::MAX))))?;
        let view = read.view();
        let Some(run_types) = types.get(&run) else { continue };
        for &(key, slot) in &read.rows {
            let Some(typed) = run_types.get(&(key as u32)) else { continue };
            if [columns[1], columns[2]].into_iter().any(|column| witnessed_production(view.number(column, slot).map_or(CoveredValue::Unvalued, CoveredValue::Value))) {
                produced.insert(typed.kind.key.clone());
            }
            if let (Some(units), false) = (view.number(columns[0], slot), typed.kind.key == UNRECORDED_TYPE) {
                *delivered.entry(typed.kind.key.clone()).or_default() += i128::from(units);
            }
        }
        if unread.is_some() && !run_types.is_empty() {
            let keys: Vec<_> = run_types.keys().map(|&token| WORLD << 32 | u128::from(token)).collect();
            for column in [columns[1], columns[2]] {
                let source = replay_columnar::summary::Source::Cell(column);
                let (held, _) = read::covered_runs(recording, run, table, source, Some(&keys), first, last)?;
                for (key, rows) in held {
                    if rows.iter().any(|&(_, _, value)| witnessed_production(value)) {
                        produced.insert(run_types[&(key as u32)].kind.key.clone());
                    }
                }
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
        most_delivered: most.filter(|_| types_missing.is_none()).map(|(key, _)| key.to_owned()),
        types: types.into_iter().map(|typed| typed.kind).collect(),
        types_missing,
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
    spending::Design { type_key: format!("{}variant/{}", spending::UNRESOLVED_MARK, key.1), name: None, domain: None, names: Vec::new(), typed_since: None }
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
    fn read(query: &Query<'_>, run: u32, hour: i32, ids: &[u32]) -> io::Result<BTreeMap<u32, Self>> {
        let owners: Vec<_> = ids.iter().copied().map(u128::from).collect();
        let countries = snapshot(query, run, TableId::ProductionCountryTotals, hour, &COUNTRY_COLUMNS, Keys::These(&owners))?.unwrap_or_default();
        let coverage_ids: BTreeSet<_> = ids.iter().copied().flat_map(coverage_keys).collect();
        let cover = snapshot(query, run, TableId::ProductionCoverage, hour, &COVERAGE_COLUMNS, Keys::These(&coverage_ids.into_iter().collect::<Vec<_>>()))?.unwrap_or_default();
        let rocket_ranges: Vec<_> = std::iter::once((0, 0)).chain(ids.iter().copied().map(rocket_designs)).collect();
        let rockets = snapshot(query, run, TableId::RocketProductionOutput, hour, &ROCKET_COLUMNS, Keys::Ranges(&rocket_ranges))?.unwrap_or_default();
        let world = rockets.contains_key(&0);
        let mut out: BTreeMap<_, _> = ids.iter().map(|&id| {
            let [own, lines, rocket_run] = coverage_keys(id).map(|key| cover.get(&key).map(|cells| coverage(|at| cells[at])));
            (id, Self {
                hour, country: countries.contains_key(&u128::from(id)), lines, rocket: [own, rocket_run], world,
                observation: (lines.and_then(|row| row.observation), rocket_run.and_then(|row| row.observation)),
                designs: BTreeMap::new(), rockets: BTreeMap::new(), archetypes: BTreeMap::new(),
            })
        }).collect();
        for (key, cells) in rockets {
            if key != 0 && let Some(owner) = out.get_mut(&((key >> 64) as u32)) {
                owner.rockets.insert(key as u64, rocket_row(|at| cells[at]));
            }
        }
        let ranges: Vec<_> = owners.iter().map(|owner| (owner << 72, owner << 72 | ((1u128 << 72) - 1))).collect();
        let rows = snapshot(query, run, TableId::ProductionTotals, hour, &DESIGN_COLUMNS, Keys::Ranges(&ranges))?.unwrap_or_default();
        for (key, cells) in rows {
            let owner = out.get_mut(&((key >> 72) as u32)).expect("requested country");
            let variant = (key >> 8) as u64;
            owner.designs.entry((false, variant)).or_default().push(design_row(key as u8, |at| cells[at]));
            let token = cells[4].filter(|&token| token > 0).map_or(0, |token| token as u32);
            let held = owner.archetypes.entry(variant).or_insert(0);
            *held = (*held).max(token);
        }
        Ok(out)
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

#[derive(Clone, Copy)]
struct DetailWindow<'a> {
    branch_id: &'a str,
    hour: i32,
    measure: ProducedMeasure,
    refits: bool,
    from_hour: Option<i32>,
}

struct ModelSnapshot {
    run: u32,
    hour: i32,
    countries: BTreeMap<u32, DesignsAt>,
}

#[derive(Default)]
struct ModelSnapshots(Vec<Arc<ModelSnapshot>>);

pub(crate) fn forget(source: &Source) {
    source.state::<ModelSnapshots>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
    *source.state::<Option<Arc<DatedDesigns>>>().lock().unwrap_or_else(PoisonError::into_inner) = None;
}

fn model_snapshot(source: &Source, query: &Query<'_>, run: u32, hour: i32, ids: &BTreeSet<u32>) -> io::Result<Arc<ModelSnapshot>> {
    let cache = source.state::<ModelSnapshots>();
    {
        let mut held = cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = held.0.iter().position(|s| s.run == run && s.hour == hour && ids.iter().all(|id| s.countries.contains_key(id))) {
            held.0.swap(0, at);
            return Ok(Arc::clone(&held.0[0]));
        }
    }
    let countries = DesignsAt::read(query, run, hour, &ids.iter().copied().collect::<Vec<_>>())?;
    let made = Arc::new(ModelSnapshot { run, hour, countries });
    let mut held = cache.lock().unwrap_or_else(PoisonError::into_inner);
    held.0.retain(|s| (s.run, s.hour) != (run, hour));
    held.0.truncate(1);
    held.0.insert(0, Arc::clone(&made));
    Ok(made)
}

struct DatedDesigns {
    run: u32,
    hour: i32,
    catalogue: Option<Arc<dashboard_base::catalogue::Catalogue>>,
    variants: BTreeSet<(u64, u32)>,
    designs: HashMap<(u64, u32), spending::Design>,
    notes: BTreeSet<String>,
}

impl DatedDesigns {
    fn read(source: &Source, query: &Query<'_>, span: &Span, variants: BTreeSet<(u64, u32)>, content: &Content) -> io::Result<Arc<Self>> {
        let cache = source.state::<Option<Arc<Self>>>();
        if let Some(hit) = cache.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && (hit.run, hit.hour) == (span.run, span.last) && hit.variants == variants
            && match (&hit.catalogue, &content.catalogue) { (Some(a), Some(b)) => Arc::ptr_eq(a, b), (None, None) => true, _ => false }
        {
            return Ok(Arc::clone(hit));
        }
        let mut notes = BTreeSet::new();
        let mut designs = HashMap::new();
        let mut pending = variants.clone();
        while !pending.is_empty() {
            let batch: BTreeMap<_, _> = pending.iter().copied().collect();
            for (variant, design) in spending::designs(query, span, &batch, content, &mut notes)? {
                designs.insert((variant, batch[&variant]), design);
            }
            for entry in batch { pending.remove(&entry); }
        }
        let made = Arc::new(Self { run: span.run, hour: span.last, catalogue: content.catalogue.clone(), variants, designs, notes });
        *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&made));
        Ok(made)
    }
}

fn at_reply(source: &Source, query: &Query<'_>, params: &ProducedAtParams, context: &LoadContext) -> io::Result<ProducedAtReply> {
    let window = DetailWindow { branch_id: &params.branch_id, hour: params.hour, measure: params.measure, refits: params.refits, from_hour: params.from_hour };
    Ok(details_at(source, query, window, std::slice::from_ref(&params.country), context, &AtomicBool::new(false))?.pop().expect("one asked country"))
}

fn details_at(source: &Source, query: &Query<'_>, params: DetailWindow<'_>, countries: &[Country], context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Vec<ProducedAtReply>> {
    let hour = params.hour;
    let mut replies: Vec<_> = countries.iter().map(|country| ProducedAtReply {
        branch_id: params.branch_id.to_owned(), tag: query.holders.name(country, hour), asked: country.clone(), hour, measure: params.measure,
        recorded: false, models: Vec::new(), ic_total: amount_of(Amount::Unknown(ProducedUnknown::NotRecorded)),
        observation: None, read: None, warnings: Vec::new(),
    }).collect();
    let Some(stretch) = query.timeline.at(hour).copied() else { return Ok(replies) };
    let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
    let counted = series.measure(produced_measure(ProducedCount::Units, params.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
    let line = dashboard_query::series::Line::new(query, series, counted)?;
    let opening = tiles::range_baseline(&line, query, params.measure, params.from_hour)?;
    let restarted = match opening {
        tiles::Baseline::Opening(start) => line.clock_at(hour)?.since != Some(start),
        _ => false,
    };
    let whole = matches!(opening, tiles::Baseline::Unrestricted | tiles::Baseline::Opening(_)) && !restarted;
    let ranged = if matches!(opening, tiles::Baseline::Unrestricted) { None } else { params.from_hour };
    let mut needed: BTreeMap<i32, BTreeSet<u32>> = BTreeMap::new();
    let mut windows = BTreeMap::new();
    let mut by_start = HashMap::new();
    for (index, country) in countries.iter().enumerate() {
        let Some(run) = query.holders.at(country, hour) else { continue };
        let Holder::Found { id, .. } = run.holder else {
            if let Some(why) = query.holders.not_known(country, hour) { return Err(dashboard_core::refused(why)); }
            continue;
        };
        let first = stretch.first.max(run.first);
        let reads = match by_start.entry(first) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(reads_in(query, stretch.run, first, hour)?),
        };
        let Some(read) = measure::read_at(reads, hour) else { continue };
        let before = opening.hour().or_else(|| ranged.is_none().then(|| measure::back(params.measure, read)).flatten())
            .filter(|base| reads.binary_search(base).is_ok());
        needed.entry(read).or_default().insert(id);
        if let Some(base) = before { needed.entry(base).or_default().insert(id); }
        windows.entry(read).or_insert_with(Vec::new).push((index, id, before));
    }
    let snapshots = needed.into_iter().map(|(hour, ids)| {
        dashboard_text::metadata::check_cancel(cancelled)?;
        Ok((hour, model_snapshot(source, query, stretch.run, hour, &ids)?))
    }).collect::<io::Result<BTreeMap<_, _>>>()?;
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    for (read, rows) in windows {
        let mut named = BTreeSet::new();
        for &(_, id, before) in &rows {
            let now = &snapshots[&read].countries[&id];
            for (_, variant) in now.keys().chain(before.into_iter().flat_map(|base| snapshots[&base].countries[&id].keys())) {
                named.insert((variant, now.archetypes.get(&variant).copied().unwrap_or(0)));
            }
        }
        let span = Span { run: stretch.run, first: read, last: read, id: 0, breaks: false };
        dashboard_text::metadata::check_cancel(cancelled)?;
        let found = DatedDesigns::read(source, query, &span, named, &content)?;
        for (index, id, before) in rows {
            let reply = &mut replies[index];
            let now = &snapshots[&read].countries[&id];
            let before = before.map(|base| &snapshots[&base].countries[&id]);
            fill_models(reply, now, before, &found.designs, params, whole, restarted);
            reply.observation = ranged.or(now.observation.0);
            reply.read = Some(read);
            reply.warnings.extend(warnings.iter().chain(&found.notes).cloned());
        }
    }
    Ok(replies)
}

fn fill_models(reply: &mut ProducedAtReply, now: &DesignsAt, before: Option<&DesignsAt>, found: &HashMap<(u64, u32), spending::Design>, params: DetailWindow<'_>, whole: bool, restarted: bool) {
    let at = |held: &DesignsAt, key: DesignKey, choice: Choice| held.amount(key, held.hour, choice);
    let windowed = |key: DesignKey, choice: Choice| match (params.measure, whole) {
        _ if restarted => Amount::Unknown(ProducedUnknown::ObservationRestarted),
        (ProducedMeasure::Total, true) => at(now, key, choice).0,
        _ => measure::difference(at(now, key, choice), before.map(|before| at(before, key, choice))),
    };
    let units = Choice { count: ProducedCount::Units, refits: params.refits };
    let ic = Choice { count: ProducedCount::Ic, refits: params.refits };
    let keys: BTreeSet<DesignKey> = now.keys().chain(before.into_iter().flat_map(DesignsAt::keys)).collect();
    let mut total = Amount::Known(0);
    for key in keys {
        let (units, ic) = (windowed(key, units), windowed(key, ic));
        total = total.plus(ic);
        if units == Amount::Known(0) && ic == Amount::Known(0) {
            continue;
        }
        let fallback;
        let design = match found.get(&(key.1, now.archetypes.get(&key.1).copied().unwrap_or(0))) {
            Some(design) => design,
            None => { fallback = unnamed(key); &fallback }
        };
        reply.models.push(ProducedModelAt {
            variant: key.1.to_string(),
            rocket: key.0,
            type_recorded: type_recorded(design),
            name: design.names.first().and_then(|names| names.name.clone()),
            type_key: design.type_key.clone(),
            type_name: design.name.clone(),
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
    reply.ic_total = amount_of(country(now).plus(match (params.measure, whole) {
        _ if restarted => Amount::Unknown(ProducedUnknown::ObservationRestarted),
        (ProducedMeasure::Total, true) => Amount::Known(0),
        _ => before.map_or(Amount::Unknown(ProducedUnknown::WindowIncomplete), country),
    }).plus(total));
    reply.recorded = now.country;
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, measure, refits, fromHour: Start];
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub models: Option<Vec<Country>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProducedRaceRow {
    pub country: Country,
    #[doc = " The country the total was read for: the slot a ranking counted, or the country a selection asked for. A detail of the row asks for the same."]
    pub counted: Country,
    pub tag: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub value: Option<i64>,
    pub unknown: Option<ProducedUnknown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub breakdown: Option<ProducedBreakdown>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct ProducedModelAmount {
    name: String,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum ProducedBreakdown {
    Models {
        models: Vec<ProducedModelAmount>,
        more: usize,
        #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
        remainder: i64,
    },
    Missing { reason: ProducedUnknown },
    Unavailable { unavailable: String },
}

impl ProducedBreakdown {
    fn from_detail(detail: ProducedAtReply, count: ProducedCount, types: &[String], total: i64) -> Self {
        if !detail.recorded {
            return Self::Missing { reason: ProducedUnknown::NotRecorded };
        }
        if let Some(reason) = detail.ic_total.reason
            && (detail.models.is_empty() || matches!(reason, ProducedUnknown::WindowIncomplete | ProducedUnknown::ObservationRestarted))
        {
            return Self::Missing { reason };
        }
        let mut models = Vec::new();
        for model in detail.models {
            let key = if model.type_recorded { &model.type_key } else { UNRECORDED_TYPE };
            if !types.is_empty() && !types.iter().any(|asked| asked == key) {
                continue;
            }
            let amount = match count { ProducedCount::Units => model.units, ProducedCount::Ic => model.ic };
            let Some(value) = amount.value else {
                return Self::Missing { reason: amount.reason.unwrap_or(ProducedUnknown::NotRecorded) };
            };
            if value != 0 {
                models.push(ProducedModelAmount { name: model.name.unwrap_or_else(|| format!("Design {}", model.variant)), value });
            }
        }
        if models.iter().map(|model| i128::from(model.value)).sum::<i128>() != i128::from(total) {
            return Self::Unavailable { unavailable: "Model amounts do not add up to the recorded total.".into() };
        }
        const MODEL_ROWS: usize = 9;
        models.sort_by(|a, b| b.value.cmp(&a.value));
        let (mut more, mut remainder) = (0, 0);
        if models.len() > MODEL_ROWS {
            let rest = models.drain(MODEL_ROWS - 1..);
            more = rest.len();
            let sum = rest.map(|model| i128::from(model.value)).sum::<i128>();
            if sum.abs() > MAX_EXACT {
                return Self::Missing { reason: ProducedUnknown::OutOfRange };
            }
            remainder = sum as i64;
        }
        Self::Models { models, more, remainder }
    }
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
    let observation_hour = if baseline.hour().is_some() { params.from_hour } else { clock.since.filter(|&since| since <= hour) };
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
    let amounts = if tags.is_empty() {
        Vec::new()
    } else {
        line.totals_at(&tags, &parts_of, back, hour)?
    };
    let mut rows = Vec::with_capacity(tags.len());
    for (asked, amount) in tags.into_iter().zip(amounts) {
        let counted = asked.clone();
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
        let (value, unknown) = match amount_of_total(amount) {
            Amount::Known(val) if val.abs() <= MAX_EXACT => (Some(val as i64), None),
            Amount::Known(_) => (None, Some(ProducedUnknown::OutOfRange)),
            Amount::Unknown(reason) => (None, Some(reason)),
        };
        rows.push(EquipmentProducedRaceRow { country, counted, tag, value, unknown, breakdown: None });
    }
    if is_population {
        (first_hour, last_hour) = population_bounds(query, hour, first_hour, last_hour);
    }
    if !is_population && params.models.is_none() {
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, group, top, count, measure, types, refits, fromHour: Start, "countries[]", "models[]"];
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
        if params.models.is_some() && params.countries.is_empty() {
            return Err("Model breakdowns require the displayed countries");
        }
        if let Some(countries) = &params.models {
            for country in countries { validate_country(country)?; }
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
            ProducedModelAmount::decl(),
            ProducedBreakdown::decl(),
            EquipmentProducedRaceRow::decl(),
            EquipmentProducedRaceReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let mut reply = race_reply(source, &query, params, context, cancelled)?;
        if let Some(named) = &params.models {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let rows: Vec<_> = reply.rows.iter_mut().filter(|row| named.contains(&row.counted))
                .filter_map(|row| row.value.map(|total| (row, total))).collect();
            let countries: Vec<_> = rows.iter().map(|(row, _)| row.counted.clone()).collect();
            if !countries.is_empty() {
                let window = DetailWindow { branch_id: &params.branch_id, hour: params.hour, measure: params.measure, refits: params.refits, from_hour: params.from_hour };
                match details_at(source, &query, window, &countries, context, cancelled) {
                    Ok(details) => for ((row, total), detail) in rows.into_iter().zip(details) {
                        row.breakdown = Some(ProducedBreakdown::from_detail(detail, params.count, &params.types, total));
                    },
                    Err(error) if replay_recording::columnar_frames::Damage::of(&error).is_some() => for (row, _) in rows {
                        row.breakdown = Some(ProducedBreakdown::Unavailable { unavailable: error.to_string() });
                    },
                    Err(error) => return Err(error),
                }
            }
            dashboard_base::products::check_reply_budget(&reply)?;
        }
        Ok(reply)
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

    #[test]
    fn produced_type_list_distinguishes_recorded_empty_from_an_unread_run_end() {
        let ask = |read| {
            let mut frame = source(1, 1000, Some(1));
            *frame.tables[TableId::ProductionLineOutput.index()] = if read {
                TableView::Complete(TableRows::ProductionLineOutput(vec![]))
            } else {
                TableView::Unavailable { reason: "fixture production feed failed".into() }
            };
            let replay = dashboard_base::test_fixtures::replay(vec![frame]).into_timelines(&Default::default()).unwrap();
            let source = Source::from_replay(dashboard_base::SourceKey { source_id: "produced-types".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
            EquipmentProducedTypes::load(&source, &ProducedTypesParams { branch_id: "1".into(), refits: false }, &LoadContext { content: None }, &false.into()).unwrap()
        };
        let empty = ask(true);
        assert!(empty.types.is_empty());
        assert_eq!(serde_json::to_value(&empty).unwrap().get("typesMissing"), Some(&serde_json::Value::Null));
        let unread = ask(false);
        assert!(unread.types.is_empty());
        let unread = serde_json::to_value(unread).unwrap();
        assert!(unread["typesMissing"].as_str().is_some_and(|reason| reason.contains("fixture production feed failed")), "{unread}");

        let ask = |units, lose_tail| {
            let mut witnessed = source(1, 1000, Some(1));
            *witnessed.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(vec![
                ProductionLineOutputRow { line: 11, variant: 17, country: 9001, units, since_hour: 1000, observed_since_creation: true, exact: true, spent_raw: Recorded::Value(None), purpose: Recorded::Value(1), ..Default::default() }
            ]));
            let mut frames = vec![witnessed];
            if lose_tail {
                let mut lost = source(1, 1001, Some(1));
                *lost.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable { reason: "fixture lost its production feed".into() };
                frames.push(lost);
            }
            let replay = dashboard_base::test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
            let source = Source::from_replay(dashboard_base::SourceKey { source_id: "produced-types-partial".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
            EquipmentProducedTypes::load(&source, &ProducedTypesParams { branch_id: "1".into(), refits: false }, &LoadContext { content: None }, &false.into()).unwrap()
        };
        for units in [0, 91] {
            let witnessed = ask(units, false);
            assert_eq!(witnessed.types.len(), 1);
            let lost = ask(units, true);
            assert_eq!(lost.types, witnessed.types, "a run-end failure does not remove the witnessed type");
            assert!(lost.most_delivered.is_none());
            assert!(lost.types_missing.as_deref().is_some_and(|reason| reason.contains("fixture lost its production feed")));
        }

        let mut lost = dashboard_base::test_fixtures::source(1, 1000, Some(1));
        *lost.tables[TableId::ProductionLineOutput.index()] = TableView::Unavailable { reason: "fixture production gap".into() };
        let mut recovered = dashboard_base::test_fixtures::source(1, 1001, Some(1));
        *recovered.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(vec![]));
        let replay = dashboard_base::test_fixtures::replay(vec![lost, recovered]).into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "produced-types-recovered".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
        let reply = EquipmentProducedTypes::load(&source, &ProducedTypesParams { branch_id: "1".into(), refits: false }, &LoadContext { content: None }, &false.into()).unwrap();
        assert!(reply.types.is_empty());
        assert!(reply.types_missing.as_deref().is_some_and(|reason| reason.contains("fixture production gap")));
    }

    #[test]
    fn the_race_reads_each_date_once_for_all_its_countries() {
        let src = two_producers_read_from("race-dated-reads", 1000, 1300, 1000, true, Some(1200), None);
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let ask = |countries: Vec<Country>, top: Option<u32>, from_hour: i32| {
            let params = EquipmentProducedRaceParams {
                branch_id: "1".into(), hour: 1300, countries, group: None, top, count: ProducedCount::Units,
                measure: ProducedMeasure::Total, types: vec![], refits: false, from_hour: Some(from_hour), models: None,
            };
            let (reply, reads) = replay_recording::cells::count_reads(|| {
                race_reply(&src, &query, &params, &LoadContext { content: None }, &false.into()).unwrap()
            });
            (reply, reads.get(&TableId::ProducedValues).copied().unwrap_or_default())
        };
        let (one, one_read) = ask(vec![], Some(1), 1120);
        let (all, all_read) = ask(vec![], None, 1120);
        assert_eq!(one.rows, all.rows[..1]);
        let (selected, selected_read) = ask(vec![Country::Slot(7), Country::Slot(8)], None, 1120);
        assert_eq!(selected, all);
        let (with_tag, tag_read) = ask(vec![Country::Slot(7), Country::Slot(8), Country::from("DEU")], None, 1120);
        assert_eq!(with_tag.rows[..2], all.rows);
        assert_eq!(with_tag.rows[2].unknown, Some(ProducedUnknown::WindowIncomplete));
        assert_eq!(with_tag.rows[2].value, None);
        assert_eq!(all.rows.iter().map(|r| (r.tag.as_str(), r.value)).collect::<Vec<_>>(), [("ITA", Some(15)), ("DEU", Some(8))]);
        assert_eq!((all.first_hour, all.last_hour, all.read_hour, all.observation_hour), (1292, 1300, Some(1292), Some(1120)));
        let (refused, _) = ask(vec![], None, 999);
        assert!(refused.rows.is_empty());
        assert_eq!((refused.first_hour, refused.last_hour, refused.missing.as_deref()), (1300, 1300, Some(tiles::RANGE_BASELINE_UNRECORDED)));
        eprintln!("ranked production reads: {all_read:?}; selected production reads: {selected_read:?}");
        assert_eq!((one_read, selected_read), (all_read, tag_read), "selecting additional rows must share the same dated production reads");
    }

    #[test]
    fn hover_counts_the_same_country_and_window_as_the_line() {
        let src = two_producers_read_from("produced-hover", 1000, 1300, 1000, true, Some(1200), None);
        let ask = |hour: i32, models: Option<Vec<Country>>| {
            let params = serde_json::from_value(serde_json::json!({
                "branchId": "1", "countries": [7, 8, "DEU"], "hour": hour, "count": "units",
                "measure": "total", "fromHour": 1120, "types": [], "refits": false, "models": models,
            })).unwrap();
            EquipmentProducedRace::load(&src, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
        };
        let head = ask(1300, None);
        assert!(head.rows.iter().all(|row| !serde_json::to_value(row).unwrap().as_object().unwrap().contains_key("breakdown")));
        ask(1250, Some(vec![Country::Slot(7)]));
        let snapshots = src.state::<ModelSnapshots>();
        let opening = snapshots.lock().unwrap().0.iter().min_by_key(|s| s.hour).unwrap().clone();
        let hover = ask(1300, Some(vec![Country::Slot(7)]));
        let slot = hover.rows.iter().find(|row| row.counted == Country::Slot(7)).unwrap();
        assert_eq!(slot.value, Some(8));
        let Some(ProducedBreakdown::Models { models, more: 0, remainder: 0 }) = &slot.breakdown else { panic!("missing models: {slot:?}") };
        assert_eq!(models.iter().map(|model| model.value).collect::<Vec<_>>(), [8]);
        let tag = hover.rows.iter().find(|row| row.counted == Country::Tag("DEU".into())).unwrap();
        assert_eq!(tag.unknown, Some(ProducedUnknown::WindowIncomplete));
        assert!(tag.breakdown.is_none());
        let tail = hover.rows.iter().find(|row| row.counted == Country::Slot(8)).unwrap();
        assert!(tail.value.unwrap() > slot.value.unwrap() && tail.breakdown.is_none(), "a higher total at the hovered hour does not promote a tail");
        let names = src.state::<Option<Arc<DatedDesigns>>>();
        assert!(snapshots.lock().unwrap().0.iter().any(|s| Arc::ptr_eq(s, &opening)), "a Range keeps its opening snapshot when the inspected hour advances");
        let named = ask(1300, Some(vec![Country::Slot(8)]));
        assert_eq!(named.rows.iter().filter(|row| row.breakdown.is_some()).map(|row| &row.counted).collect::<Vec<_>>(), [&Country::Slot(8)]);
        assert!(names.lock().unwrap().is_some());
        crate::forget(&src);
        assert!(snapshots.lock().unwrap().0.is_empty());
        assert!(names.lock().unwrap().is_none());
    }

    #[test]
    fn hover_lists_the_largest_models_of_the_selected_type_and_accounts_for_the_rest() {
        let model = |variant: i64, kind: &str| ProducedModelAt {
            variant: variant.to_string(), type_key: kind.into(), name: None, type_name: None, type_recorded: true,
            domain: None, rocket: false, ic: amount_of(Amount::Unknown(ProducedUnknown::SpendingUnavailable)),
            units: amount_of(Amount::Known(i128::from(variant))),
        };
        let detail = ProducedAtReply {
            branch_id: "1".into(), tag: "GER".into(), asked: Country::Slot(7), hour: 1, measure: ProducedMeasure::Total,
            recorded: true, models: (1..=10).map(|i| model(i, "truck")).chain([model(20, "tank")]).collect(),
            ic_total: amount_of(Amount::Unknown(ProducedUnknown::SpendingUnavailable)), observation: Some(1), read: Some(1), warnings: vec![],
        };
        assert_eq!(ProducedBreakdown::from_detail(detail.clone(), ProducedCount::Ic, &[], 55),
            ProducedBreakdown::Missing { reason: ProducedUnknown::SpendingUnavailable });
        assert!(matches!(ProducedBreakdown::from_detail(detail.clone(), ProducedCount::Units, &["truck".into()], 54), ProducedBreakdown::Unavailable { .. }));
        let ProducedBreakdown::Models { models, more, remainder } = ProducedBreakdown::from_detail(detail, ProducedCount::Units, &["truck".into()], 55)
            else { panic!("the unit counts are recorded") };
        assert_eq!(models.iter().map(|model| model.value).collect::<Vec<_>>(), (3..=10).rev().collect::<Vec<_>>());
        assert_eq!((more, remainder), (2, 3));
    }

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
