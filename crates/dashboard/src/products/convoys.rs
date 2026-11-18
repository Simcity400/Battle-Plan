//! Convoy health (family `convoys`): one country's convoy efficiency in each sea region, the trade
//! efficiency of each resource deal it imports, and the lend-lease it receives by sea, as runs of
//! the recorded cells over the hours asked.
//!
//! - Regional efficiency is keyed by country above region: the holder's key range.
//! - Deals and convoy clients are keyed by handle with their country in a column. Each run's index
//!   ([`run_convoys`], read once per run) says which handles named which country over which
//!   hours; a reply reads only those keys' history, cut to those hours.
//! - A region or deal not read is not shown: never 100% and never zero. Lend-lease clients record
//!   their route's bad flag and convoys, but no numeric efficiency: that is never invented.

use super::{
    LoadContext, Product,
    country_ids::{Names, RunLinks},
};
use crate::{Source, catalogue, span::HourSpan};
use dashboard_base::products::{resources::ResourceNaming, validate_tag};
use dashboard_query::{Query, identity::Holder, read};
use replay::{
    columnar_frames::Recording,
    lanes::{self, Cell, History, Keys},
};
use replay_columnar::summary::{Cells, Source as Column};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const KEY: &str = "convoy-health";
/// Fixed-point denominator of every efficiency.
const SCALE: u32 = 100_000;
/// The convoy client category of a lend-lease shipment.
const LEND_LEASE: i64 = 4;

const REGION_EFFICIENCY: usize = 0;
const DEAL_EXPORTER: usize = 0;
const DEAL_IMPORTER: usize = 1;
const DEAL_RESOURCE: usize = 2;
const DEAL_ASSIGNED: usize = 5;
const DEAL_REQUIRED: usize = 6;
const DEAL_FACTOR_A: usize = 7;
const DEAL_FACTOR_B: usize = 8;
const CLIENT_CATEGORY: usize = 0;
const CLIENT_OWNER: usize = 1;
const CLIENT_OTHER: usize = 2;
const CLIENT_BAD: usize = 6;
const CLIENT_ASSIGNED: usize = 7;
const CLIENT_REQUIRED: usize = 8;

/// The rows one run names one country in: by country, each key with the hours it named it.
pub type Owned = HashMap<u32, Vec<(u128, i32, i32)>>;

/// One run's deals by importer and by exporter, and lend-lease convoy clients by receiver.
#[derive(Default)]
pub struct RunConvoys {
    pub(crate) deals: Owned,
    pub(crate) exports: Owned,
    pub(crate) leases: Owned,
}

impl RunConvoys {
    /// The deals naming raw country `country` as importer or exporter at `hour`, ascending, each once.
    pub(crate) fn deals_at(&self, country: u32, hour: i32) -> Vec<u128> {
        let mut out: Vec<u128> = [&self.deals, &self.exports]
            .into_iter()
            .flat_map(|owned| owned.get(&country).map(Vec::as_slice).unwrap_or_default())
            .filter(|&&(_, first, last)| first <= hour && hour <= last)
            .map(|&(key, ..)| key)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A lend-lease client's receiver; none for any other category.
fn lease_receiver(cells: &dyn Cells, slot: u32) -> Option<i64> {
    (cells.number(CLIENT_CATEGORY, slot)? == LEND_LEASE).then(|| cells.number(CLIENT_OTHER, slot)).flatten()
}

/// Runs of a country column by key, grouped by the country they name.
pub(crate) fn owned(runs: impl IntoIterator<Item = (u128, read::Runs)>) -> Owned {
    let mut out = Owned::new();
    for (key, runs) in runs {
        for (first, last, country) in runs {
            if let Some(country) = country.and_then(|country| u32::try_from(country).ok()) {
                out.entry(country).or_default().push((key, first, last));
            }
        }
    }
    for rows in out.values_mut() {
        rows.sort_unstable_by_key(|&(key, first, _)| (key, first));
    }
    out
}

/// Run `run`'s deals by importer and lend-lease clients by receiver, from one walk of each
/// table's country column over the whole run.
pub fn run_convoys(recording: &Recording, run: u32) -> io::Result<RunConvoys> {
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
    let deals = read::runs(recording, run, TableId::ResourceExchanges, Column::Cell(DEAL_IMPORTER), None, first, last)?;
    let exports = read::runs(recording, run, TableId::ResourceExchanges, Column::Cell(DEAL_EXPORTER), None, first, last)?;
    let receiver = Column::Computed { id: 0, inputs: 1 << CLIENT_CATEGORY | 1 << CLIENT_OTHER, value: lease_receiver };
    let leases = read::runs(recording, run, TableId::ConvoyClients, receiver, None, first, last)?;
    Ok(RunConvoys { deals: owned(deals), exports: owned(exports), leases: owned(leases) })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ConvoyView {
    Regions,
    Trade,
}

pub struct ConvoyHealth;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvoyHealthParams {
    pub branch_id: String,
    pub view: ConvoyView,
    /// The one country shown.
    pub tag: String,
    /// The hours read; the whole timeline when not given.
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
}

/// A country named in a row over consecutive hours; none where no tag holds it then.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyCountry {
    pub first_hour: i32,
    pub last_hour: i32,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyRegion {
    pub region: u32,
    /// The region's in-game name; null with why where the content does not name it.
    pub name: Option<String>,
    pub missing: Option<String>,
    /// Fixed5 efficiency: first hour, last hour, value.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub efficiency: Vec<(i32, i32, i64)>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyDeal {
    /// The exchange's native handle, hexadecimal.
    pub id: String,
    pub exporter: Vec<ConvoyCountry>,
    /// The resource's native index.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub resource: Vec<(i32, i32, i64)>,
    /// The fixed5 efficiency product, truncated toward zero as the game compares it.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub efficiency: Vec<(i32, i32, i64)>,
    /// Its two fixed5 factors.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, number]>>"))]
    pub factors: Vec<Vec<(i32, i32, i64)>>,
    /// Convoys assigned and required.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, number]>>"))]
    pub convoys: Vec<Vec<(i32, i32, i64)>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyLease {
    /// The client's native handle.
    pub handle: String,
    pub giver: Vec<ConvoyCountry>,
    /// 1 while its route draws as bad.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub bad: Vec<(i32, i32, i64)>,
    /// Convoys assigned and required.
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, number]>>"))]
    pub convoys: Vec<Vec<(i32, i32, i64)>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyResource {
    pub id: u32,
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyHealthReply {
    pub branch_id: String,
    pub view: ConvoyView,
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Fixed-point denominator of every efficiency.
    pub scale: u32,
    /// Hours each table the view reads was read and available (regional efficiency; resource
    /// deals, convoy clients), in the hours asked: outside them a row not shown is not recorded,
    /// not absent; an hour read but refused holds no rows and is not known.
    pub known: Vec<Vec<HourSpan>>,
    /// Hours the tag had no country, or the recording holds none: nothing is known of it there.
    pub unheld: Vec<HourSpan>,
    pub regions: Vec<ConvoyRegion>,
    pub deals: Vec<ConvoyDeal>,
    pub leases: Vec<ConvoyLease>,
    pub resources: Vec<ConvoyResource>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ConvoyHealthReply);

/// A number cell's runs from a history column, cut to `spans` (sorted, not overlapping).
pub(crate) fn numbers(runs: &[(i32, i32, Option<Cell>)], spans: &[(i32, i32)]) -> Vec<(i32, i32, i64)> {
    let mut out = Vec::new();
    for &(first, last, cell) in runs {
        let Some(Cell::Number(value)) = cell else { continue };
        for &(a, b) in spans {
            let (from, to) = (first.max(a), last.min(b));
            if from <= to {
                push(&mut out, (from, to, value));
            }
        }
    }
    out
}

fn push(out: &mut Vec<(i32, i32, i64)>, run: (i32, i32, i64)) {
    match out.last_mut() {
        Some(held) if held.2 == run.2 && i64::from(held.1) + 1 == i64::from(run.0) => held.1 = run.1,
        _ => out.push(run),
    }
}

/// One key's steps as runs of one value made from its cells; none where the row is not shown or
/// `value` gives none.
pub(crate) fn steps_runs(steps: &[lanes::Step], to: i32, value: impl Fn(&[Cell]) -> Option<i64>) -> Vec<(i32, i32, Option<Cell>)> {
    let mut runs: Vec<(i32, i32, Option<Cell>)> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let cell = step.cells.as_deref().and_then(&value).map(Cell::Number);
        let last = steps.get(index + 1).map_or(to, |next| next.hour - 1);
        match runs.last_mut() {
            Some(run) if run.2 == cell => run.1 = last,
            _ => runs.push((step.hour, last, cell)),
        }
    }
    runs
}

/// The efficiency product of two fixed5 factors, widened and truncated toward zero.
pub(crate) fn product(a: i64, b: i64) -> Option<i64> {
    i64::try_from(i128::from(a) * i128::from(b) / i128::from(SCALE)).ok()
}

/// What a history's column `column` holds at a step's cells.
fn at(history: &History, column: usize) -> Option<usize> {
    history.columns.binary_search(&column).ok()
}

fn number(cells: &[Cell], at: Option<usize>) -> Option<i64> {
    match cells.get(at?) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

/// Country cells' runs named by tag at each hour.
fn countries(runs: &[(i32, i32, i64)], names: &Names, links: &RunLinks, out: &mut Vec<ConvoyCountry>) {
    for &(first, last, raw) in runs {
        let Ok(raw) = u32::try_from(raw) else { continue };
        for (tag, a, b) in names.tags_over(links, raw, first, last) {
            let tag = tag.map(str::to_owned);
            match out.last_mut() {
                Some(held) if held.tag == tag && i64::from(held.last_hour) + 1 == i64::from(a) => held.last_hour = b,
                _ => out.push(ConvoyCountry { first_hour: a, last_hour: b, tag }),
            }
        }
    }
}

/// Rows of `owned`'s `country` meeting `first..=last`: each key with the hours it named the
/// country there, ascending by key.
pub(crate) fn rows_of(owned: &Owned, country: u32, first: i32, last: i32) -> BTreeMap<u128, Vec<(i32, i32)>> {
    let mut out: BTreeMap<u128, Vec<(i32, i32)>> = BTreeMap::new();
    for &(key, a, b) in owned.get(&country).map(Vec::as_slice).unwrap_or_default() {
        let (a, b) = (a.max(first), b.min(last));
        if a <= b {
            out.entry(key).or_default().push((a, b));
        }
    }
    out
}

/// What the reply gathers per row over every stretch.
#[derive(Default)]
struct Gathered {
    regions: BTreeMap<u32, Vec<(i32, i32, i64)>>,
    deals: BTreeMap<u128, ConvoyDeal>,
    leases: BTreeMap<u128, ConvoyLease>,
    known: [Vec<HourSpan>; 3],
    resources: BTreeSet<u32>,
}

fn deal(key: u128) -> ConvoyDeal {
    ConvoyDeal {
        id: format!("{:016x}", key as u64),
        exporter: Vec::new(),
        resource: Vec::new(),
        efficiency: Vec::new(),
        factors: vec![Vec::new(), Vec::new()],
        convoys: vec![Vec::new(), Vec::new()],
    }
}

fn lease(key: u128) -> ConvoyLease {
    ConvoyLease { handle: (key as u32).to_string(), giver: Vec::new(), bad: Vec::new(), convoys: vec![Vec::new(), Vec::new()] }
}

/// Adds the deals of `history` (keys of `rows`, each cut to its hours).
fn gather_deals(gathered: &mut Gathered, history: &History, rows: &BTreeMap<u128, Vec<(i32, i32)>>, names: &Names, links: &RunLinks) {
    let (exporter, resource, a, b, assigned, required) =
        (at(history, DEAL_EXPORTER), at(history, DEAL_RESOURCE), at(history, DEAL_FACTOR_A), at(history, DEAL_FACTOR_B), at(history, DEAL_ASSIGNED), at(history, DEAL_REQUIRED));
    for (key, steps) in &history.keys {
        let Some(spans) = rows.get(key) else { continue };
        let runs = |value: &dyn Fn(&[Cell]) -> Option<i64>| numbers(&steps_runs(steps, history.to, value), spans);
        let row = gathered.deals.entry(*key).or_insert_with(|| deal(*key));
        countries(&runs(&|cells| number(cells, exporter)), names, links, &mut row.exporter);
        let resources = runs(&|cells| number(cells, resource));
        gathered.resources.extend(resources.iter().filter_map(|run| u32::try_from(run.2).ok()));
        row.resource.extend(resources);
        row.efficiency.extend(runs(&|cells| product(number(cells, a)?, number(cells, b)?)));
        row.factors[0].extend(runs(&|cells| number(cells, a)));
        row.factors[1].extend(runs(&|cells| number(cells, b)));
        row.convoys[0].extend(runs(&|cells| number(cells, assigned)));
        row.convoys[1].extend(runs(&|cells| number(cells, required)));
    }
}

/// Adds the lend-lease clients of `history` (keys of `rows`, each cut to its hours).
fn gather_leases(gathered: &mut Gathered, history: &History, rows: &BTreeMap<u128, Vec<(i32, i32)>>, names: &Names, links: &RunLinks) {
    let (owner, bad, assigned, required) = (at(history, CLIENT_OWNER), at(history, CLIENT_BAD), at(history, CLIENT_ASSIGNED), at(history, CLIENT_REQUIRED));
    for (key, steps) in &history.keys {
        let Some(spans) = rows.get(key) else { continue };
        let runs = |value: &dyn Fn(&[Cell]) -> Option<i64>| numbers(&steps_runs(steps, history.to, value), spans);
        let row = gathered.leases.entry(*key).or_insert_with(|| lease(*key));
        countries(&runs(&|cells| number(cells, owner)), names, links, &mut row.giver);
        row.bad.extend(runs(&|cells| number(cells, bad)));
        row.convoys[0].extend(runs(&|cells| number(cells, assigned)));
        row.convoys[1].extend(runs(&|cells| number(cells, required)));
    }
}

fn spans(runs: Vec<(i32, i32)>) -> impl Iterator<Item = HourSpan> {
    runs.into_iter().map(|(first_hour, last_hour)| HourSpan { first_hour, last_hour })
}

/// The hours of `first..=last` not in `held` (sorted, not overlapping).
pub(crate) fn outside(held: &[(i32, i32)], first: i32, last: i32) -> Vec<HourSpan> {
    let mut out = Vec::new();
    let mut at = i64::from(first);
    for &(a, b) in held {
        if i64::from(a) > at {
            out.push(HourSpan { first_hour: at as i32, last_hour: a - 1 });
        }
        at = at.max(i64::from(b) + 1);
    }
    if at <= i64::from(last) {
        out.push(HourSpan { first_hour: at as i32, last_hour: last });
    }
    out
}

fn reads(recording: &Recording, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    dashboard_query::cost::storage(|| lanes::available_reads(recording, run, table, first, last))
}

fn history(recording: &Recording, run: u32, table: TableId, columns: &[usize], keys: Keys<'_>, first: i32, last: i32) -> io::Result<History> {
    dashboard_query::cost::storage(|| lanes::history(recording, run, table, columns, keys, first, last))
}

/// Every row of `params`' view over the hours asked, each stretch of the timeline read from its
/// run, the tag's holder resolved per stretch by the one identity rule.
fn gather(source: &Source, query: &Query<'_>, params: &ConvoyHealthParams, (from, to): (i32, i32), cancelled: &AtomicBool) -> io::Result<(Gathered, Vec<(i32, i32)>)> {
    let recording = query.replay.recording();
    let names = Names::new(&query.holders);
    let mut gathered = Gathered::default();
    let mut held = Vec::new();
    for stretch in query.timeline.within(from, to) {
        crate::metadata::check_cancel(cancelled)?;
        let run = stretch.run;
        let tables: &[(usize, TableId)] = match params.view {
            ConvoyView::Regions => &[(0, TableId::RegionalConvoyEfficiency)],
            ConvoyView::Trade => &[(1, TableId::ResourceExchanges), (2, TableId::ConvoyClients)],
        };
        for &(at, table) in tables {
            gathered.known[at].extend(spans(reads(recording, run, table, stretch.first, stretch.last)?));
        }
        let (links, index) = match params.view {
            ConvoyView::Regions => (None, None),
            ConvoyView::Trade => (Some(crate::answer::links(source, run)?), Some(crate::answer::convoys(source, run)?)),
        };
        for holder in query.holders.within(&params.tag, stretch.first, stretch.last) {
            let Holder::Found { id, .. } = holder.holder else { continue };
            let (first, last) = (holder.first, holder.last);
            held.push((first, last));
            match (params.view, &links, &index) {
                (ConvoyView::Regions, ..) => {
                    let owner = u128::from(id) << 32;
                    let read = history(recording, run, TableId::RegionalConvoyEfficiency, &[REGION_EFFICIENCY], Keys::Range(owner, owner | u128::from(u32::MAX)), first, last)?;
                    for (key, runs) in read.runs(REGION_EFFICIENCY).unwrap_or_default() {
                        let region = gathered.regions.entry(key as u32).or_default();
                        for run in numbers(&runs, &[(first, last)]) {
                            push(region, run);
                        }
                    }
                }
                (ConvoyView::Trade, Some(links), Some(index)) => {
                    let deals = rows_of(&index.deals, id, first, last);
                    if !deals.is_empty() {
                        let keys: Vec<u128> = deals.keys().copied().collect();
                        let columns = [DEAL_EXPORTER, DEAL_RESOURCE, DEAL_ASSIGNED, DEAL_REQUIRED, DEAL_FACTOR_A, DEAL_FACTOR_B];
                        let (a, b) = bounds(&deals);
                        let read = history(recording, run, TableId::ResourceExchanges, &columns, Keys::These(&keys), a, b)?;
                        gather_deals(&mut gathered, &read, &deals, &names, links);
                    }
                    let leases = rows_of(&index.leases, id, first, last);
                    if !leases.is_empty() {
                        let keys: Vec<u128> = leases.keys().copied().collect();
                        let columns = [CLIENT_OWNER, CLIENT_BAD, CLIENT_ASSIGNED, CLIENT_REQUIRED];
                        let (a, b) = bounds(&leases);
                        let read = history(recording, run, TableId::ConvoyClients, &columns, Keys::These(&keys), a, b)?;
                        gather_leases(&mut gathered, &read, &leases, &names, links);
                    }
                }
                _ => {}
            }
        }
    }
    Ok((gathered, held))
}

/// The first and last hour any of `rows` names.
fn bounds(rows: &BTreeMap<u128, Vec<(i32, i32)>>) -> (i32, i32) {
    rows.values().flatten().fold((i32::MAX, i32::MIN), |(a, b), &(first, last)| (a.min(first), b.max(last)))
}

/// Strategic regions' in-game names through the recorded content, or why each is unnamed.
fn region_names(source: &Source, context: &LoadContext, regions: &BTreeSet<u32>, warnings: &mut Vec<String>) -> BTreeMap<u32, Result<String, String>> {
    let named = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| format!("Sea region names unavailable: {error}")).and_then(|cat| {
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| format!("Sea region names unavailable: {error}"))?;
        warnings.extend(content.notes.iter().cloned());
        Ok(regions.iter().map(|&id| (id, super::air_ledger::region_name(&cat, &content, id))).collect::<BTreeMap<_, _>>())
    });
    match named {
        Ok(named) => named,
        Err(why) => {
            warnings.push(why.clone());
            regions.iter().map(|&id| (id, Err(why.clone()))).collect()
        }
    }
}

/// The recorded resources `ids` names (every one when none), with their tokens, at the timeline's
/// last hour, by native index.
pub(crate) fn resource_tokens(query: &Query<'_>, ids: Option<&BTreeSet<u32>>, warnings: &mut Vec<String>) -> io::Result<Vec<(u32, String)>> {
    let Some(stretch) = query.timeline.stretches.last() else { return Ok(Vec::new()) };
    let recording = query.replay.recording();
    let wanted: Vec<u128> = ids.into_iter().flatten().map(|&id| u128::from(id)).collect();
    let keys = if ids.is_some() { read::Keys::These(&wanted) } else { read::Keys::All };
    let definitions = read::at(recording, stretch.run, &[read::Read { table: TableId::ResourceDefinitions, columns: None, keys }], stretch.last)?;
    let token = dashboard_declare::field(TableId::ResourceDefinitions, "token")?;
    let view = definitions[0].view();
    let tokens: Vec<(u32, u32)> = definitions[0].rows.iter().filter_map(|&(key, slot)| Some((key as u32, u32::try_from(view.number(token, slot)?).ok()?))).collect();
    let mut keys: Vec<u128> = tokens.iter().map(|&(_, token)| u128::from(token)).collect();
    keys.sort_unstable();
    keys.dedup();
    let literal = dashboard_declare::field(TableId::LexicalTokens, "literal")?;
    let lexical = read::at(recording, stretch.run, &[read::Read { table: TableId::LexicalTokens, columns: None, keys: read::Keys::These(&keys) }], stretch.last)?;
    let view = lexical[0].view();
    let spelled: HashMap<u32, String> = lexical[0]
        .rows
        .iter()
        .filter_map(|&(key, slot)| Some((key as u32, std::str::from_utf8(read::text(&view, literal, slot)?).ok().filter(|text| !text.is_empty())?.to_owned())))
        .collect();
    let listed: Vec<u32> = match ids {
        Some(ids) => ids.iter().copied().collect(),
        None => tokens.iter().map(|&(id, _)| id).collect(),
    };
    let mut out = Vec::new();
    let mut unnamed = 0usize;
    for id in listed {
        match tokens.iter().find(|(held, _)| *held == id).and_then(|(_, token)| spelled.get(token)) {
            Some(token) => out.push((id, token.clone())),
            None => unnamed += 1,
        }
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} recorded resources have no recorded definition or token; they are not named"));
    }
    Ok(out)
}

impl Product for ConvoyHealth {
    const KEY: &'static str = KEY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = ConvoyHealthParams;
    type Reply = ConvoyHealthReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if let (Some(from), Some(to)) = (params.from, params.to)
            && from > to
        {
            return Err("Invalid convoy health hours");
        }
        validate_tag(&params.tag)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ConvoyHealthParams::decl(),
            ConvoyHealthReply::decl(),
            ConvoyView::decl(),
            ConvoyCountry::decl(),
            ConvoyRegion::decl(),
            ConvoyDeal::decl(),
            ConvoyLease::decl(),
            ConvoyResource::decl(),
            HourSpan::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| crate::invalid("Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        let from = params.from.unwrap_or(query.timeline.first).max(query.timeline.first);
        let to = params.to.unwrap_or(query.timeline.last).min(query.timeline.last);
        let mut warnings = Vec::new();
        let (gathered, mut held) = if from <= to { gather(source, &query, params, (from, to), cancelled)? } else { (Gathered::default(), Vec::new()) };
        held.sort_unstable();
        let unheld = outside(&held, from, to);
        let named = region_names(source, context, &gathered.regions.keys().copied().collect(), &mut warnings);
        let regions = gathered
            .regions
            .into_iter()
            .map(|(region, efficiency)| {
                let (name, missing) = match named.get(&region) {
                    Some(Ok(name)) => (Some(name.clone()), None),
                    Some(Err(why)) => (None, Some(why.clone())),
                    None => (None, Some(format!("Region {region} absent from the recorded stack"))),
                };
                ConvoyRegion { region, name, missing, efficiency }
            })
            .collect();
        let resources = if params.view == ConvoyView::Trade {
            let tokens = resource_tokens(&query, Some(&gathered.resources), &mut warnings)?;
            let (infos, notes) = ResourceNaming::stored(source, context).infos(&tokens, cancelled)?;
            warnings.extend(notes);
            infos.into_iter().map(|info| ConvoyResource { id: info.id, token: info.token, name: info.name }).collect()
        } else {
            Vec::new()
        };
        let [regional, deals, clients] = gathered.known;
        let known = [regional, deals, clients].map(super::subjects::settle).to_vec();
        let deals = gathered.deals.into_values().filter(|deal| !deal.resource.is_empty() || !deal.efficiency.is_empty() || !deal.exporter.is_empty()).collect();
        let leases = gathered.leases.into_values().filter(|lease| !lease.bad.is_empty()).collect();
        Ok(ConvoyHealthReply {
            branch_id: params.branch_id.clone(),
            view: params.view,
            tag: params.tag.clone(),
            first_hour: from,
            last_hour: to,
            scale: SCALE,
            known,
            unheld,
            regions,
            deals,
            leases,
            resources,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
