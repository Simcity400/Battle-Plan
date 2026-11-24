
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::identities::CountryIdentityRun;
use super::production_spending::{self as spending, Content, Domain, Read, Span, StockVariantName, runs::{Piece, pieces}};
use dashboard_base::Source;
use dashboard_query::{
    Query, cost,
    identity::Holder,
    series::totals::{Clock, Gap, TotalRun},
};
use replay_lanes::lanes::{self, Cell, Keys, Step};
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
#[cfg(test)]
mod reference;
mod value;

use value::{Amount, Choice, Counted, Coverage, DesignRow, RocketRow, Totals};

pub struct EquipmentProduced;
pub struct EquipmentProducedTypes;
pub struct EquipmentProducedCountry;
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedGap {
    pub run: u32,
    pub reason: ProducedUnknown,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedSeries {
    pub first_hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    #[doc = " Raw values (fixed5 for IC); null is not known, never a partial sum."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number | null>"))]
    pub values: Vec<Option<i64>>,
    #[doc = " Why each run without a value is not known."]
    pub unknown: Vec<ProducedGap>,
    #[doc = " Runs starting another holder's or stretch's totals: nothing is measured across them."]
    pub breaks: Vec<u32>,
}

const MAX_EXACT: i128 = (1 << 53) - 1;

impl ProducedSeries {
    fn append(&mut self, pieces: &[Piece<Amount>], breaks: bool) {
        for (index, &(first, last, amount)) in pieces.iter().enumerate() {
            let (value, reason) = match amount {
                Amount::Known(value) if value.abs() <= MAX_EXACT => (Some(value as i64), None),
                Amount::Known(_) => (None, Some(ProducedUnknown::OutOfRange)),
                Amount::Unknown(reason) => (None, Some(reason)),
            };
            let starts = index == 0 && breaks;
            let open = self.first_hours.len();
            let joined = !starts
                && open > 0
                && self.last_hours[open - 1].checked_add(1) == Some(first)
                && self.values[open - 1] == value
                && self.unknown.last().filter(|gap| gap.run as usize == open - 1).map(|gap| gap.reason) == reason;
            if joined {
                self.last_hours[open - 1] = last;
                continue;
            }
            self.first_hours.push(first);
            self.last_hours.push(last);
            self.values.push(value);
            if let Some(reason) = reason {
                self.unknown.push(ProducedGap { run: open as u32, reason });
            }
            if starts {
                self.breaks.push(open as u32);
            }
        }
    }
}

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
pub struct EquipmentProducedParams {
    pub branch_id: String,
    #[doc = " Chosen countries; none with a top N."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The hour a top N is ranked at: the view's last hour, or the cursor."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub rank_hour: Option<i32>,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[doc = " Type keys counted together; none counts every type."]
    #[serde(default)]
    pub types: Vec<String>,
    #[doc = " Count what ship refit lines deliver."]
    pub refits: bool,
    #[doc = " The window of hours asked ([`super::windows`])."]
    pub window: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedLine {
    #[doc = " The country asked, or a ranked country's tag."]
    pub country: Country,
    #[doc = " The tag it goes by at the window's last hour."]
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub series: ProducedSeries,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProducedReply {
    pub branch_id: String,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[doc = " Fixed-point decimals of every value: 0 for units, 5 for IC."]
    pub decimals: u8,
    pub types: Vec<String>,
    #[doc = " For a top N, the hour it was ranked at; its lines are then in rank order, greatest first,"]
    #[doc = " only countries that delivered something."]
    pub rank_hour: Option<i32>,
    pub window: u32,
    #[doc = " The hours the window holds of the timeline; first past last where it holds none."]
    pub first_hour: i32,
    pub last_hour: i32,
    pub lines: Vec<ProducedLine>,
    #[doc = " Hours production lines' output was read in the window, ascending, whichever lines are shown: a value is"]
    #[doc = " as of the latest read."]
    pub reads: Vec<i32>,
    #[doc = " The hour each observation counting over the window began, the first maybe before it."]
    pub observations: Vec<i32>,
    pub warnings: Vec<String>,
}

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
pub struct ProducedCountryParams {
    pub branch_id: String,
    pub country: Country,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    pub refits: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedModel {
    pub variant: String,
    pub rocket: bool,
    pub type_key: String,
    pub type_name: Option<String>,
    pub domain: Option<Domain>,
    #[doc = " False when the design's type was not recorded: it could be of any type."]
    pub type_recorded: bool,
    pub names: Vec<StockVariantName>,
    pub series: ProducedSeries,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProducedCountryReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    pub decimals: u8,
    pub identities: Vec<CountryIdentityRun>,
    pub models: Vec<ProducedModel>,
    #[doc = " Every design together, lines and Rocket output: exact however it splits."]
    pub total: ProducedSeries,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
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

dashboard_base::reply_warnings!(EquipmentProducedReply, ProducedTypesReply, ProducedCountryReply, ProducedAtReply);

fn identities(query: &Query<'_>, tag: &Country) -> Vec<CountryIdentityRun> {
    query.holders.runs(tag).iter().filter_map(|run| match run.holder {
        Holder::Found { slot, id } => Some(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
        Holder::Unresolved => None,
    }).collect()
}

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

fn totals(number: impl Fn(usize) -> Option<i64>) -> Totals {
    let (units, units_without_refits) = (number(0), number(3));
    Totals {
        units: units.unwrap_or(0),
        units_inexact: number(1).unwrap_or(1) + i64::from(units.is_none()),
        spent: number(2),
        units_without_refits: units_without_refits.unwrap_or(0),
        units_without_refits_inexact: number(4).unwrap_or(1) + i64::from(units_without_refits.is_none()),
        spent_without_refits: number(5),
    }
}

fn coverage(number: impl Fn(usize) -> Option<i64>) -> Coverage {
    let hour = |at| number(at).and_then(|hour| i32::try_from(hour).ok());
    Coverage { observation: hour(0), units_unknown_from: hour(1), spending_unknown_from: hour(2), reason: number(3).map_or(0, |code| code as u8) }
}

fn rocket_row(number: impl Fn(usize) -> Option<i64>) -> RocketRow {
    let units = number(0);
    RocketRow { units: units.unwrap_or(0), units_exact: units.is_some() && number(1) == Some(1), spent: number(2).unwrap_or(0) }
}

fn sweep<'r>(tracks: &[&'r [Step]], first: i32, last: i32, mut each: impl FnMut(i32, &[Option<&'r [Cell]>])) {
    let mut next = vec![0usize; tracks.len()];
    let mut cells: Vec<Option<&'r [Cell]>> = vec![None; tracks.len()];
    let mut hour = first;
    loop {
        for (at, steps) in tracks.iter().enumerate() {
            while let Some(step) = steps.get(next[at]).filter(|step| step.hour <= hour) {
                cells[at] = step.cells.as_deref();
                next[at] += 1;
            }
        }
        each(hour, &cells);
        let upcoming = tracks.iter().enumerate().filter_map(|(at, steps)| steps.get(next[at]).map(|step| step.hour)).min();
        match upcoming {
            Some(after) if after <= last => hour = after.max(hour + 1),
            _ => break,
        }
    }
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

#[cfg(test)]
fn spellings(query: &Query<'_>, run: u32, hour: i32, tokens: &[u128]) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let frame = cost::storage(|| replay_lanes::at::frame(query.replay.recording(), run, hour, &[replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: Keys::These(tokens) }]))?;
    let rows = frame.complete::<replay_data::rows::tables::LexicalTokens>().unwrap_or(&[]);
    Ok(rows
        .iter()
        .filter_map(|row| {
            let text = std::str::from_utf8(content_reader_parser::parser::c_string(&row.literal)).ok().filter(|text| !text.is_empty())?;
            Some((row.token, text.to_owned()))
        })
        .collect())
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

fn number<'c>(read: &'c Read, cells: Option<&'c [Cell]>) -> impl Fn(usize) -> Option<i64> + 'c {
    move |column| cells.and_then(|cells| read.number(cells, column))
}

#[derive(Default)]
struct Dated {
    reads: BTreeSet<i32>,
    observations: BTreeSet<i32>,
    stretches: HashMap<(u32, i32, i32), Vec<i32>>,
}

impl Dated {
    fn reads(&mut self, query: &Query<'_>, span: &Span) -> io::Result<Vec<i32>> {
        let Some(stretch) = query.timeline.at(span.first).copied() else {
            return Ok(Vec::new());
        };
        let key = (stretch.run, stretch.first, stretch.last);
        if let std::collections::hash_map::Entry::Vacant(slot) = self.stretches.entry(key) {
            slot.insert(reads_in(query, stretch.run, stretch.first, stretch.last)?);
        }
        let reads: Vec<i32> = self.stretches[&key].iter().copied().filter(|&read| span.first <= read && read <= span.last).collect();
        self.reads.extend(&reads);
        Ok(reads)
    }

    fn observed(&mut self, totals: &[Piece<measure::Counted>]) {
        self.observations.extend(totals.iter().filter_map(|piece| (piece.2).1.0));
    }
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

fn series_of(runs: &[TotalRun]) -> ProducedSeries {
    let mut series = ProducedSeries::default();
    let mut at = 0;
    while at < runs.len() {
        let end = runs[at + 1..].iter().position(|run| run.opens).map_or(runs.len(), |offset| at + 1 + offset);
        let pieces: Vec<Piece<Amount>> = runs[at..end].iter().map(|run| (run.first, run.last, amount_of_total(run.value))).collect();
        series.append(&pieces, runs[at].opens);
        at = end;
    }
    series
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

fn reply(source: &Source, query: &Query<'_>, params: &EquipmentProducedParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<EquipmentProducedReply> {
    window_reply(source, query, params, dashboard_world::products::windows::within(query, params.window)?, context, cancelled)
}

fn window_reply(source: &Source, query: &Query<'_>, params: &EquipmentProducedParams, (first, last): (i32, i32), context: &LoadContext, cancelled: &AtomicBool) -> io::Result<EquipmentProducedReply> {
    let mut warnings = Vec::new();
    let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
    let measure = series.measure(produced_measure(params.count, params.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
    let line = dashboard_query::series::Line::new(query, series, measure)?;
    let back = back_of(params.measure);
    let back: Option<&dyn Fn(i32) -> Option<i32>> = back.as_ref().map(|back| back as &dyn Fn(i32) -> Option<i32>);
    let mut met: BTreeSet<u32> = query.timeline.within(first, last).map(|stretch| stretch.run).collect();
    if let (Some(_), Some(hour)) = (params.top, params.rank_hour) {
        met.extend(query.timeline.at(hour).map(|stretch| stretch.run));
    }
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
    let tags = match (params.top, params.rank_hour) {
        (Some(n), Some(hour)) => line.totals_ranked(hour, n as usize, &parts_of, back, &|value| value > 0)?.into_iter().map(Country::from).collect(),
        _ => params.countries.clone(),
    };
    let shown = first <= last;
    let runs = if shown { line.totals(&tags, &parts_of, back, first, last)? } else { vec![Vec::new(); tags.len()] };
    let lines: Vec<ProducedLine> = tags.into_iter().zip(runs).map(|(country, runs)| ProducedLine { identities: identities(query, &country), tag: query.holders.name(&country, last), country, series: series_of(&runs) }).collect();
    let clock = if shown { line.clock(first, last)? } else { Clock::default() };
    warnings.extend(notes);
    let reply = EquipmentProducedReply {
        branch_id: params.branch_id.clone(),
        count: params.count,
        measure: params.measure,
        decimals: decimals(params.count),
        types: params.types.clone(),
        rank_hour: params.top.and(params.rank_hour),
        window: params.window,
        first_hour: first,
        last_hour: last,
        lines,
        reads: clock.reads,
        observations: clock.since,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
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

struct DesignRows {
    span: Span,
    country: Read,
    coverage: Read,
    designs: Read,
    world: Read,
    rocket: Read,
}

impl DesignRows {
    fn read(query: &Query<'_>, span: Span) -> io::Result<Self> {
        let id = u128::from(span.id);
        let (low, high) = rocket_designs(span.id);
        Ok(Self {
            country: Read::of(query, &span, TableId::ProductionCountryTotals, &COUNTRY_COLUMNS, Keys::These(&[id]))?,
            coverage: Read::of(query, &span, TableId::ProductionCoverage, &COVERAGE_COLUMNS, Keys::These(&coverage_keys(span.id)))?,
            designs: Read::of(query, &span, TableId::ProductionTotals, &DESIGN_COLUMNS, Keys::Range(id << 72, id << 72 | ((1u128 << 72) - 1)))?,
            world: Read::of(query, &span, TableId::RocketProductionOutput, &["spent_raw"], Keys::These(&[0]))?,
            rocket: Read::of(query, &span, TableId::RocketProductionOutput, &ROCKET_COLUMNS, Keys::Range(low, high))?,
            span,
        })
    }

    fn named(&self) -> BTreeMap<u64, u32> {
        let mut out = BTreeMap::new();
        for (key, steps) in &self.designs.history.keys {
            let token = steps.iter().rev().filter_map(|step| step.cells.as_deref()).filter_map(|cells| self.designs.number(cells, 4)).find(|&token| token > 0);
            let held = out.entry((key >> 8) as u64).or_insert(0);
            if let Some(token) = token {
                *held = token as u32;
            }
        }
        for variant in self.rocket_variants() {
            out.entry(variant).or_insert(0);
        }
        out
    }

    fn rocket_variants(&self) -> Vec<u64> {
        self.rocket.history.keys.iter().map(|(key, _)| *key as u64).collect()
    }
}

type DesignKey = (bool, u64);

struct DesignValues {
    designs: BTreeMap<DesignKey, Vec<Piece<measure::Counted>>>,
    total: Vec<Piece<measure::Counted>>,
}

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

fn design_values(rows: &DesignRows, designs: &BTreeSet<DesignKey>, choice: Choice) -> DesignValues {
    let span = &rows.span;
    let id = u128::from(span.id);
    let mut tracks = vec![rows.country.steps(id)];
    tracks.extend(coverage_keys(span.id).map(|key| rows.coverage.steps(key)));
    tracks.push(rows.world.steps(0));
    let mut purpose_of: Vec<u8> = vec![0; tracks.len()];
    let mut owned: Vec<(DesignKey, std::ops::Range<usize>)> = Vec::with_capacity(designs.len());
    for &(rocket, variant) in designs {
        let start = tracks.len();
        if rocket {
            tracks.push(rows.rocket.steps(id << 64 | u128::from(variant)));
            purpose_of.push(0);
        } else {
            let low = id << 72 | u128::from(variant) << 8;
            let from = rows.designs.history.keys.partition_point(|(key, _)| *key < low);
            for (key, steps) in rows.designs.history.keys[from..].iter().take_while(|(key, _)| *key <= low | 0xff) {
                tracks.push(steps.as_slice());
                purpose_of.push(*key as u8);
            }
        }
        owned.push(((rocket, variant), start..tracks.len()));
    }
    let mut steps_of: BTreeMap<DesignKey, Vec<(i32, measure::Counted)>> = BTreeMap::new();
    let mut steps_total: Vec<(i32, measure::Counted)> = Vec::new();
    let mut held_rocket: Vec<(Counted, Option<RocketRow>)> = Vec::new();
    let mut purpose_rows: Vec<DesignRow> = Vec::new();
    sweep(&tracks, span.first, span.last, |hour, cells| {
        let country = cells[0].map(|at| totals(number(&rows.country, Some(at))));
        let [own, lines, rocket_run] = [1, 2, 3].map(|at| cells[at].map(|held| coverage(number(&rows.coverage, Some(held)))));
        let world = cells[4].is_some();
        let observation = (lines.and_then(|row| row.observation), rocket_run.and_then(|row| row.observation));
        let push = |steps: &mut Vec<(i32, measure::Counted)>, amount: Amount| {
            if steps.last().is_none_or(|last| last.1 != (amount, observation)) {
                steps.push((hour, (amount, observation)));
            }
        };
        held_rocket.clear();
        for (key, range) in &owned {
            let amount = if key.0 {
                let row = cells[range.start].map(|held| rocket_row(number(&rows.rocket, Some(held))));
                held_rocket.push((Counted::Yes, row));
                value::rocket_design(world, [own, rocket_run], row, hour, choice.count)
            } else {
                purpose_rows.clear();
                for at in range.clone() {
                    if let Some(held) = cells[at] {
                        purpose_rows.push(design_row(purpose_of[at], number(&rows.designs, Some(held))));
                    }
                }
                value::design(country.is_some(), lines, &purpose_rows, hour, choice)
            };
            push(steps_of.entry(*key).or_default(), amount);
        }
        let cells = value::Cells { country, lines, rocket_run, rocket_own: own, types: None, rocket_world: world, rocket: &held_rocket };
        push(&mut steps_total, value::value(&cells, hour, choice));
    });
    let absent = (Amount::Unknown(ProducedUnknown::NotRecorded), (None, None));
    DesignValues {
        designs: steps_of.into_iter().map(|(key, steps)| (key, pieces(steps, span.first, span.last, absent))).collect(),
        total: pieces(steps_total, span.first, span.last, absent),
    }
}

fn unnamed(key: DesignKey) -> spending::Design {
    spending::Design { type_key: format!("{}variant/{}", spending::UNRESOLVED_MARK, key.1), name: None, domain: None, names: Vec::new() }
}

fn type_recorded(design: &spending::Design) -> bool {
    !design.type_key.starts_with(&format!("{}variant/", spending::UNRESOLVED_MARK))
}

fn country_reply(source: &Source, query: &Query<'_>, params: &ProducedCountryParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ProducedCountryReply> {
    let mut read = Vec::new();
    for span in spending::spans(query, &params.country) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        read.push(DesignRows::read(query, span)?);
    }
    let mut designs: BTreeSet<DesignKey> = BTreeSet::new();
    for rows in &read {
        designs.extend(rows.designs.history.keys.iter().map(|(key, _)| (false, (key >> 8) as u64)));
        designs.extend(rows.rocket_variants().into_iter().map(|variant| (true, variant)));
    }
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let mut dated = Dated::default();
    let mut models: BTreeMap<DesignKey, ProducedModel> = BTreeMap::new();
    let mut total = ProducedSeries::default();
    let choice = Choice { count: params.count, refits: params.refits };
    for rows in &read {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let span = &rows.span;
        let values = design_values(rows, &designs, choice);
        let reads = dated.reads(query, span)?;
        dated.observed(&values.total);
        total.append(&measure::measured(&values.total, &reads, params.measure), span.breaks);
        let found = spending::designs(query, span, &rows.named(), &content, &mut notes)?;
        for key in &designs {
            let model = models.entry(*key).or_insert_with(|| {
                let design = unnamed(*key);
                ProducedModel { variant: key.1.to_string(), rocket: key.0, type_key: design.type_key, type_name: None, domain: None, type_recorded: false, names: Vec::new(), series: ProducedSeries::default() }
            });
            if let Some(design) = found.get(&key.1) {
                model.type_recorded = type_recorded(design);
                model.type_key.clone_from(&design.type_key);
                model.type_name.clone_from(&design.name);
                model.domain = design.domain;
                for name in &design.names {
                    if model.names.last().is_none_or(|last| (&last.name, last.known) != (&name.name, name.known)) {
                        model.names.push(name.clone());
                    }
                }
            }
            if let Some(pieces) = values.designs.get(key) {
                model.series.append(&measure::measured(pieces, &reads, params.measure), span.breaks);
            }
        }
    }
    warnings.extend(notes);
    let reply = ProducedCountryReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.nearest_name(&params.country, query.timeline.last),
        asked: params.country.clone(),
        count: params.count,
        measure: params.measure,
        decimals: decimals(params.count),
        identities: identities(query, &params.country),
        models: models.into_values().collect(),
        total,
        reads: dated.reads.into_iter().collect(),
        observations: dated.observations.into_iter().collect(),
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
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

impl Product for EquipmentProduced {
    const KEY: &'static str = "equipment-produced";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EquipmentProducedParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, top, rankHour: Read, count, measure, types, refits, window, "countries[]"];
    type Reply = EquipmentProducedReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for country in &params.countries {
            validate_country(country)?;
        }
        match (params.countries.is_empty(), params.top, params.rank_hour) {
            (true, Some(top), Some(_)) if top > 0 => Ok(()),
            (true, Some(_), _) => Err("A top N names a size above zero and the hour it is ranked at"),
            (false, None, _) => Ok(()),
            (false, Some(_), _) => Err("Chosen countries or a top N, not both"),
            (true, None, _) => Err("The request names no country and no top N"),
        }
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            EquipmentProducedParams::decl(),
            EquipmentProducedReply::decl(),
            ProducedCount::decl(),
            ProducedMeasure::decl(),
            ProducedUnknown::decl(),
            ProducedGap::decl(),
            ProducedSeries::decl(),
            ProducedLine::decl(),
            CountryIdentityRun::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        reply(source, &query, params, context, cancelled)
    }
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

impl Product for EquipmentProducedCountry {
    const KEY: &'static str = "equipment-produced-country";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProducedCountryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, count, measure, refits];
    type Reply = ProducedCountryReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProducedCountryParams::decl(), ProducedCountryReply::decl(), ProducedModel::decl(), StockVariantName::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        country_reply(source, &query, params, context, cancelled)
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
    pub top: Option<u32>,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[serde(default)]
    pub types: Vec<String>,
    pub refits: bool,
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
    let back = back_of(params.measure);
    let back: Option<&dyn Fn(i32) -> Option<i32>> = back.as_ref().map(|back| back as &dyn Fn(i32) -> Option<i32>);
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

    let clock = line.clock(stretch.first, stretch.last)?;
    let read_hour = measure::read_at(&clock.reads, hour);
    let observation_hour = measure::read_at(&clock.since, hour);
    let next_read = clock.reads.iter().copied().find(|&h| h > hour);
    let mut first_hour = read_hour.unwrap_or(stretch.first).max(stretch.first);
    let mut last_hour = next_read.map_or(stretch.last, |h| h - 1).min(stretch.last);

    let tags: Vec<Country> = if params.countries.is_empty() {
        let limit = params.top.map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
        line.totals_ranked(hour, limit, &parts_of, back, &|value| value > 0)?.into_iter().map(Country::from).collect()
    } else {
        params.countries.clone()
    };
    let runs = if tags.is_empty() { Vec::new() } else { line.totals(&tags, &parts_of, back, hour, hour)? };
    let mut rows = Vec::with_capacity(tags.len());
    for (country, country_runs) in tags.into_iter().zip(runs) {
        let tag = query.holders.name(&country, hour);
        if let Some(holder_run) = query.holders.at(&country, hour) {
            first_hour = first_hour.max(holder_run.first);
            last_hour = last_hour.min(holder_run.last);
        }
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
    if params.countries.is_empty() {
        for holder_runs in query.holders.tags.values() {
            for run in holder_runs {
                if run.first > first_hour && run.first <= hour {
                    first_hour = run.first;
                }
                if run.last >= hour && run.last < last_hour {
                    last_hour = run.last;
                }
                if run.first > hour && run.first - 1 < last_hour {
                    last_hour = run.first - 1;
                }
            }
        }
    }
    if !params.countries.is_empty() {
        rows.sort_unstable_by(|a, b| match (a.value, b.value) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.tag.cmp(&b.tag)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.tag.cmp(&b.tag),
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
        missing: None,
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, top, count, measure, types, refits, "countries[]"];
    type Reply = EquipmentProducedRaceReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for country in &params.countries {
            validate_country(country)?;
        }
        match (params.countries.is_empty(), params.top) {
            (true, Some(0)) => Err("top must be at least 1"),
            (false, Some(_)) => Err("Chosen countries or a top N, not both"),
            _ => Ok(()),
        }
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EquipmentProducedRaceParams::decl(), EquipmentProducedRaceRow::decl(), EquipmentProducedRaceReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        race_reply(source, &query, params, context, cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryLinkRow, CountryRow, ProductionLineOutputRow, Recorded, RocketProductionOutputRow, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;

    #[test]
    fn a_series_joins_equal_neighbours_and_names_its_gaps_and_breaks() {
        let mut series = ProducedSeries::default();
        series.append(&[(0, 4, Amount::Known(3)), (5, 9, Amount::Unknown(ProducedUnknown::LostOutput)), (10, 12, Amount::Known(1 << 60))], false);
        series.append(&[(13, 20, Amount::Known(1 << 60)), (21, 22, Amount::Known(4))], false);
        series.append(&[(23, 30, Amount::Known(4))], true);
        assert_eq!(series.first_hours, vec![0, 5, 10, 21, 23]);
        assert_eq!(series.last_hours, vec![4, 9, 20, 22, 30]);
        assert_eq!(series.values, vec![Some(3), None, None, Some(4), Some(4)]);
        assert_eq!(series.unknown, vec![
            ProducedGap { run: 1, reason: ProducedUnknown::LostOutput },
            ProducedGap { run: 2, reason: ProducedUnknown::OutOfRange },
        ]);
        assert_eq!(series.breaks, vec![4]);
    }

    #[test]
    fn a_part_that_is_not_the_tags_first_starts_afresh_and_its_runs_join_otherwise() {
        let run = |first: i32, last: i32, value: Result<i64, Gap>, opens: bool| TotalRun { first, last, value, opens };
        let series = series_of(&[
            run(0, 9, Ok(2), false),
            run(10, 19, Ok(2), false),
            run(20, 29, Err(Gap::Cell { part: 7, reason: ProducedValueRow::TYPE_NOT_RECORDED }), false),
            run(40, 49, Ok(2), true),
            run(50, 59, Err(Gap::Restarted), false),
        ]);
        assert_eq!(series.first_hours, vec![0, 20, 40, 50]);
        assert_eq!(series.values, vec![Some(2), None, Some(2), None]);
        assert_eq!(series.unknown, vec![
            ProducedGap { run: 1, reason: ProducedUnknown::TypeNotRecorded },
            ProducedGap { run: 3, reason: ProducedUnknown::ObservationRestarted },
        ]);
        assert_eq!(series.breaks, vec![2]);
    }

    fn two_producers(name: &str, first: i32, last: i32) -> Source {
        let frames = (first..=last)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!("the fixture holds its countries") };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..countries[0].clone() });
                *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 11, canonical_slot: 7 },
                    CountryLinkRow { id: 9001, canonical_slot: 7 },
                    CountryLinkRow { id: 9002, canonical_slot: 8 },
                ]));
                let output = |line: u64, country: u32, units: i64| ProductionLineOutputRow { line, ordinal: 0, country, variant: 100, mio: 0, units, since_hour: first, observed_since_creation: true, ended: false, exact: true, spent_raw: Recorded::Value(None), purpose: Recorded::Value(1), ..Default::default() };
                let mut rows = vec![output(1, 9001, i64::from((hour - first) / 24))];
                if hour >= first + 100 {
                    rows.push(output(2, 9002, i64::from((hour - first - 100) / 12)));
                }
                *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(rows));
                let world = RocketProductionOutputRow { since_hour: first, exact: true, units_exact: true, ..Default::default() };
                *frame.tables[TableId::RocketProductionOutput.index()] = TableView::Complete(TableRows::RocketProductionOutput(vec![world]));
                frame.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: name.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    fn params(tags: &[&str], top: Option<u32>, rank_hour: Option<i32>, count: ProducedCount, measure: ProducedMeasure, types: &[&str], window: u32) -> EquipmentProducedParams {
        EquipmentProducedParams {
            branch_id: "1".into(),
            countries: tags.iter().map(|tag| Country::from(*tag)).collect(),
            top,
            rank_hour,
            count,
            measure,
            types: types.iter().map(|key| (*key).to_owned()).collect(),
            refits: false,
            window,
        }
    }

    fn join(series: &mut ProducedSeries, next: &ProducedSeries) {
        for run in 0..next.first_hours.len() {
            let amount = match next.values[run] {
                Some(value) => Amount::Known(i128::from(value)),
                None => Amount::Unknown(next.unknown.iter().find(|gap| gap.run as usize == run).map_or(ProducedUnknown::NotRecorded, |gap| gap.reason)),
            };
            series.append(&[(next.first_hours[run], next.last_hours[run], amount)], next.breaks.contains(&(run as u32)));
        }
    }

    #[test]
    fn a_top_n_with_nothing_ranked_at_the_first_hour_still_names_the_hours_production_was_read() {
        const FIRST: i32 = 1_000;
        const LAST: i32 = 1_100;
        let frames = (FIRST..=LAST)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let output = ProductionLineOutputRow { line: 1, ordinal: 0, country: 9001, variant: 100, mio: 0, units: i64::from((hour - FIRST) / 24), since_hour: FIRST, observed_since_creation: true, ended: false, exact: true, spent_raw: Recorded::Value(None), purpose: Recorded::Value(1), ..Default::default() };
                *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(vec![output]));
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "produced".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let asked = params(&[], Some(10), Some(FIRST), ProducedCount::Units, ProducedMeasure::Day, &[], (FIRST / dashboard_world::products::windows::HOURS) as u32);
        let reply = reply(&source, &query, &asked, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        assert!(reply.lines.is_empty(), "no day of production has passed at the first hour: nothing ranks");
        assert_eq!((reply.first_hour, reply.last_hour), (FIRST, LAST), "the window is cut to the timeline");
        assert_eq!(reply.reads, (FIRST..=LAST).step_by(24).collect::<Vec<_>>(), "production was read: the widget says a day is not known yet, never that production is not in the recording");
    }

    #[test]
    fn countries_asked_together_have_the_lines_each_has_alone() {
        const FIRST: i32 = 1_000;
        let source = two_producers("produced-together", FIRST, 1_300);
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let ask = |tags: &[&str], measure: ProducedMeasure| reply(&source, &query, &params(tags, None, None, ProducedCount::Units, measure, &[], 0), &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        for measure in [ProducedMeasure::Total, ProducedMeasure::Day] {
            let together = ask(&["GER", "ITA"], measure);
            let (germany, italy) = (ask(&["GER"], measure), ask(&["ITA"], measure));
            assert_eq!(together.lines.iter().map(|line| line.tag.as_str()).collect::<Vec<_>>(), ["GER", "ITA"]);
            assert_eq!(together.lines[0].series, germany.lines[0].series, "{measure:?}: Germany's line is its own, whoever is asked with it");
            assert_eq!(together.lines[1].series, italy.lines[0].series, "{measure:?}: and Italy's");
            assert_ne!(together.lines[0].series, together.lines[1].series, "{measure:?}: the two lines differ, so neither is the other's");
            assert!(together.lines[1].series.values.iter().any(|value| value.is_some_and(|value| value > 0)), "{measure:?}: Italy's deliveries are counted");
            for line in &together.lines {
                assert!(line.series.unknown.iter().all(|gap| gap.reason != ProducedUnknown::RocketTableUnavailable), "{measure:?}: {}: the Rocket read is known, never a gap", line.tag);
            }
            assert_eq!(together.reads, germany.reads);
            assert_eq!(together.observations, vec![FIRST]);
        }
    }

    #[test]
    fn windows_joined_equal_the_whole_branch_reply() {
        const FIRST: i32 = 1_000;
        const LAST: i32 = 2_400;
        let source = two_producers("produced-windows", FIRST, LAST);
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        let measures = [ProducedMeasure::Total, ProducedMeasure::Day, ProducedMeasure::Week, ProducedMeasure::Month];
        for measure in measures {
            for count in [ProducedCount::Units, ProducedCount::Ic] {
                for (tags, top) in [(&["GER", "ITA"][..], None), (&[][..], Some(5))] {
                    for types in [&[][..], &[UNRECORDED_TYPE][..]] {
                        let asked = params(tags, top, top.map(|_| LAST), count, measure, types, 0);
                        let whole = reference::whole(&source, &query, &asked, &context, &running).unwrap();
                        let named = whole.lines.iter().map(|line| line.tag.clone()).collect::<Vec<_>>();
                        let mut joined = vec![ProducedSeries::default(); whole.lines.len()];
                        let (mut reads, mut observations) = (BTreeSet::new(), BTreeSet::new());
                        let mut hour = query.timeline.first;
                        while hour <= query.timeline.last {
                            let last = (hour + 69).min(query.timeline.last);
                            let part = window_reply(&source, &query, &asked, (hour, last), &context, &running).unwrap();
                            assert_eq!(part.lines.iter().map(|line| line.tag.clone()).collect::<Vec<_>>(), named, "{measure:?} {count:?} {types:?}: each window names the whole reply's lines");
                            for (series, line) in joined.iter_mut().zip(&part.lines) {
                                join(series, &line.series);
                            }
                            reads.extend(part.reads);
                            observations.extend(part.observations);
                            hour = last + 1;
                        }
                        for (line, series) in whole.lines.iter().zip(&joined) {
                            assert_eq!(series, &line.series, "{measure:?} {count:?} {types:?} {}: the windows joined are the whole reply's line", line.tag);
                        }
                        assert_eq!(reads.into_iter().collect::<Vec<_>>(), whole.reads, "{measure:?} {count:?} {types:?}: the reads");
                        if !whole.lines.is_empty() {
                            assert_eq!(observations.into_iter().collect::<Vec<_>>(), whole.observations, "{measure:?} {count:?} {types:?}: the observations");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_sweep_visits_each_change_once_with_every_tracks_cells() {
        let step = |hour: i32, value: Option<i64>| Step { hour, cells: value.map(|value| vec![Cell::Number(value)].into_boxed_slice()) };
        let (one, two) = (vec![step(0, Some(1)), step(10, Some(2))], vec![step(5, Some(7)), step(10, None), step(30, Some(1))]);
        let tracks: [&[Step]; 3] = [&one, &two, &[]];
        let mut seen = Vec::new();
        let number = |cells: Option<&[Cell]>| match cells.and_then(|cells| cells.first()) {
            Some(Cell::Number(value)) => Some(*value),
            _ => None,
        };
        sweep(&tracks, 0, 20, |hour, cells| seen.push((hour, cells.iter().map(|cells| number(*cells)).collect::<Vec<_>>())));
        assert_eq!(seen, vec![(0, vec![Some(1), None, None]), (5, vec![Some(1), Some(7), None]), (10, vec![Some(2), None, None])]);
    }
}
