
use dashboard_base::products::{Country, validate_country};
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::{Names, RunLinks}};
use dashboard_base::{Source, catalogue, span::HourSpan};
use dashboard_base::products::{resources::ResourceNaming};
use dashboard_query::{Query, identity::Holder, read};
use replay_recording::columnar_frames::Recording;
use replay_lanes::lanes::{self, Cell, History, Keys};
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
const SCALE: u32 = 100_000;
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

pub type Owned = HashMap<u32, Vec<(u128, i32, i32)>>;

#[derive(Default)]
pub struct RunConvoys {
    pub(crate) deals: Owned,
    pub(crate) exports: Owned,
    pub(crate) leases: Owned,
}

impl RunConvoys {
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

fn lease_receiver(cells: &dyn Cells, slot: u32) -> Option<i64> {
    (cells.number(CLIENT_CATEGORY, slot)? == LEND_LEASE).then(|| cells.number(CLIENT_OTHER, slot)).flatten()
}

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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvoyHealthParams {
    pub branch_id: String,
    pub view: ConvoyView,
    #[doc = " The one country shown."]
    pub country: Country,
    #[doc = " The hours read; the whole timeline when not given."]
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
}

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
    #[doc = " The region's in-game name; null with why where the content does not name it."]
    pub name: Option<String>,
    pub missing: Option<String>,
    #[doc = " Fixed5 efficiency: first hour, last hour, value."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub efficiency: Vec<(i32, i32, i64)>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyDeal {
    #[doc = " The exchange's native handle, hexadecimal."]
    pub id: String,
    pub exporter: Vec<ConvoyCountry>,
    #[doc = " The resource's native index."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub resource: Vec<(i32, i32, i64)>,
    #[doc = " The fixed5 efficiency product, truncated toward zero as the game compares it."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub efficiency: Vec<(i32, i32, i64)>,
    #[doc = " Its two fixed5 factors."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, number]>>"))]
    pub factors: Vec<Vec<(i32, i32, i64)>>,
    #[doc = " Convoys assigned and required."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, number]>>"))]
    pub convoys: Vec<Vec<(i32, i32, i64)>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyLease {
    #[doc = " The client's native handle."]
    pub handle: String,
    pub giver: Vec<ConvoyCountry>,
    #[doc = " 1 while its route draws as bad."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<[number, number, number]>"))]
    pub bad: Vec<(i32, i32, i64)>,
    #[doc = " Convoys assigned and required."]
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyHealthReply {
    pub branch_id: String,
    pub view: ConvoyView,
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Fixed-point denominator of every efficiency."]
    pub scale: u32,
    #[doc = " Hours each table the view reads was read and available (regional efficiency; resource"]
    #[doc = " deals, convoy clients), in the hours asked: outside them a row not shown is not recorded,"]
    #[doc = " not absent; an hour read but refused holds no rows and is not known."]
    pub known: Vec<Vec<HourSpan>>,
    #[doc = " Hours the tag had no country, or the recording holds none: nothing is known of it there."]
    pub unheld: Vec<HourSpan>,
    pub regions: Vec<ConvoyRegion>,
    pub deals: Vec<ConvoyDeal>,
    pub leases: Vec<ConvoyLease>,
    pub resources: Vec<ConvoyResource>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ConvoyHealthReply);

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

pub(crate) fn product(a: i64, b: i64) -> Option<i64> {
    i64::try_from(i128::from(a) * i128::from(b) / i128::from(SCALE)).ok()
}

fn at(history: &History, column: usize) -> Option<usize> {
    history.columns.binary_search(&column).ok()
}

fn number(cells: &[Cell], at: Option<usize>) -> Option<i64> {
    match cells.get(at?) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

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

fn gather(source: &Source, query: &Query<'_>, params: &ConvoyHealthParams, (from, to): (i32, i32), cancelled: &AtomicBool) -> io::Result<(Gathered, Vec<(i32, i32)>)> {
    let recording = query.replay.recording();
    let names = Names::new(&query.holders);
    let mut gathered = Gathered::default();
    let mut held = Vec::new();
    for stretch in query.timeline.within(from, to) {
        dashboard_text::metadata::check_cancel(cancelled)?;
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
            ConvoyView::Trade => (Some(dashboard_base::queries::links(source, run)?), Some(crate::products::convoys::cached_convoys(source, run)?)),
        };
        for holder in query.holders.within(&params.country, stretch.first, stretch.last) {
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

fn bounds(rows: &BTreeMap<u128, Vec<(i32, i32)>>) -> (i32, i32) {
    rows.values().flatten().fold((i32::MAX, i32::MIN), |(a, b), &(first, last)| (a.min(first), b.max(last)))
}

fn region_names(source: &Source, context: &LoadContext, regions: &BTreeSet<u32>, warnings: &mut Vec<String>) -> BTreeMap<u32, Result<String, String>> {
    let named = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| format!("Sea region names unavailable: {error}")).and_then(|cat| {
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| format!("Sea region names unavailable: {error}"))?;
        warnings.extend(content.notes.iter().cloned());
        Ok(regions.iter().map(|&id| (id, dashboard_world::products::region_names::region_name(&cat, &content, id))).collect::<BTreeMap<_, _>>())
    });
    match named {
        Ok(named) => named,
        Err(why) => {
            warnings.push(why.clone());
            regions.iter().map(|&id| (id, Err(why.clone()))).collect()
        }
    }
}

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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, view, country, from: Start, to: End];
    type Reply = ConvoyHealthReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if let (Some(from), Some(to)) = (params.from, params.to)
            && from > to
        {
            return Err("Invalid convoy health hours");
        }
        validate_country(&params.country)
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
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
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
        let known = [regional, deals, clients].map(dashboard_world::products::subjects::settle).to_vec();
        let deals = gathered.deals.into_values().filter(|deal| !deal.resource.is_empty() || !deal.efficiency.is_empty() || !deal.exporter.is_empty()).collect();
        let leases = gathered.leases.into_values().filter(|lease| !lease.bad.is_empty()).collect();
        Ok(ConvoyHealthReply {
            branch_id: params.branch_id.clone(),
            view: params.view,
            tag: query.holders.name(&params.country, query.timeline.last),
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

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, RunConvoys>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

#[derive(Clone)]
pub(crate) struct ConvoyIndex {
    replay: std::sync::Arc<replay::campaign::Replay>,
    state: std::sync::Arc<std::sync::Mutex<CachedRuns>>,
}

impl ConvoyIndex {
    pub(crate) fn of(source: &dashboard_base::Source) -> Self {
        Self { replay: source.replay(), state: source.state::<CachedRuns>() }
    }

    pub(crate) fn run(&self, run: u32) -> io::Result<std::sync::Arc<RunConvoys>> {
        CachedRuns::held(&self.state, run, || run_convoys(self.replay.recording(), run))
    }
}

pub fn cached_convoys(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunConvoys>> {
    ConvoyIndex::of(source).run(run)
}
