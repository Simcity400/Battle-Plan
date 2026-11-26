
use dashboard_base::products::{Country, regions::{ActiveRegions, NoRegions, automatic, no_regions}, validate_country};
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::{Names, RunLinks}};
use dashboard_base::{Source, catalogue, span::HourSpan};
use dashboard_base::products::resources::ResourceNaming;
use dashboard_query::{Query, coverage::{self, Witness}, identity::Holder, read};
use dashboard_world::products::styles::LookResolver;
use replay_recording::columnar_frames::{Recording, complete as status_complete};
use replay_lanes::{at::Seeker, lanes::{self, Cell, History, Keys}};
use replay_columnar::summary::{Cells, Source as Column};
use replay_data::rows::{TableId, TradeRouteRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const KEY: &str = "convoy-health";
const SCALE: u32 = 100_000;
const SUPPLY_TRANSFER: u8 = 1;
const IMPORT: u8 = 2;
const LEND_LEASE: i64 = 4;
const LEND_LEASE_U8: u8 = 4;
const MARKET: u8 = 6;
const NO_ROUTE: u8 = 0;
const NAVAL_ROUTE: u8 = 2;
pub const TRAFFIC_NOT_RECORDED: &str = "regional convoy traffic was not recorded";
const GAP_NOT_RECORDED: &str = "The recording holds no hour here";
const UNREAD_PROBE: u128 = u128::MAX;

const REGION_EFFICIENCY: usize = 0;
const LINK_SLOT: usize = 0;
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

fn lease_receiver(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
    replay_columnar::summary::Computed::of((cells.number(CLIENT_CATEGORY, slot) == Some(LEND_LEASE)).then(|| cells.number(CLIENT_OTHER, slot)).flatten())
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
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<usize>,
    #[doc = " In the trade view, also lists every deal and lease seen from this hour to the day before `from`, from a per-source index extended by the days added since."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub list_from: Option<i32>,
    #[doc = " Every region with a convoy efficiency in the hours, unranked, and `absent`: the cells of a region without one."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub every: Option<bool>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConvoyCellState {
    Efficiency { value: i64 },
    NoTraffic,
    NotRecorded { reason: String },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyCellRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub state: ConvoyCellState,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyCountry {
    pub first_hour: i32,
    pub last_hour: i32,
    pub tag: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConvoyRegion {
    pub region: u32,
    #[doc = " The region's in-game name; null with why where the content does not name it."]
    pub name: Option<String>,
    pub missing: Option<String>,
    pub pinned: bool,
    pub cells: Vec<ConvoyCellRun>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub readout: Option<ConvoyCellState>,
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
    #[doc = " The last hour through which nothing this reply shows changes: read beside the reply, never among its rows."]
    pub valid_through: i32,
    #[doc = " Fixed-point denominator of every efficiency."]
    pub scale: u32,
    #[doc = " Hours each table the view reads was read whole (regional efficiency; resource deals,"]
    #[doc = " convoy clients), in the hours asked: outside them a row not shown is not recorded, not"]
    #[doc = " absent, though a row shown there was witnessed."]
    pub known: Vec<Vec<HourSpan>>,
    #[doc = " Hours no country held the tag: nothing is known of it there. Recording gaps are `gaps`."]
    pub unheld: Vec<HourSpan>,
    pub gaps: Vec<HourSpan>,
    pub regions: Vec<ConvoyRegion>,
    #[doc = " Why the most strained regions are not a proven rank at the asked hour: the regions' convoy state there is not recorded. A top then lists only the pinned regions."]
    pub rank_missing: Option<String>,
    #[doc = " Why no region is listed in the ranked regions view; set only when `regions` is empty."]
    pub empty: Option<NoRegions>,
    pub active_unlisted: ActiveRegions,
    #[doc = " With `every`: the cells of a region without a convoy efficiency."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub absent: Option<Vec<ConvoyCellRun>>,
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
                _ => out.push(ConvoyCountry { first_hour: a, last_hour: b, tag, name: None, name_missing: None }),
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrafficProof {
    pub(crate) crossed: BTreeSet<u32>,
    pub(crate) unknown: Option<String>,
}

impl TrafficProof {
    pub(crate) fn unknown(reason: impl Into<String>) -> Self {
        Self { crossed: BTreeSet::new(), unknown: Some(reason.into()) }
    }
}

pub(crate) const TRAFFIC_TABLES: [(TableId, &str); 4] = [
    (TableId::CountryLinks, "Country links were not recorded at this hour"),
    (TableId::ConvoyClients, "Convoy clients were not recorded at this hour"),
    (TableId::ResourceOrigins, "Resource origins were not recorded at this hour"),
    (TableId::TradeRoutes, "Trade routes were not recorded at this hour"),
];
const ROUTE_NOT_RECORDED: &str = "A convoy route was not recorded at this hour";
const PORTS_NOT_RECORDED: &str = "A naval convoy route's ports were not recorded at this hour";
const NO_PATH: &str = "A convoy route with assigned convoys has no recorded path";

pub(crate) fn traffic_in_frame(frame: &DecodedFrame, slot: u32) -> TrafficProof {
    let (Ok(links_rows), Ok(clients), Ok(origins), Ok(route_rows)) =
        (frame.complete::<tables::CountryLinks>(), frame.complete::<tables::ConvoyClients>(), frame.complete::<tables::ResourceOrigins>(), frame.complete::<tables::TradeRoutes>())
    else {
        let missing = [
            frame.complete::<tables::CountryLinks>().is_err(),
            frame.complete::<tables::ConvoyClients>().is_err(),
            frame.complete::<tables::ResourceOrigins>().is_err(),
            frame.complete::<tables::TradeRoutes>().is_err(),
        ];
        let at = missing.iter().position(|missing| *missing).unwrap_or(0);
        return TrafficProof::unknown(TRAFFIC_TABLES[at].1);
    };
    let links: HashMap<u32, u32> = links_rows.iter().map(|row| (row.id, row.canonical_slot)).collect();
    let routes: HashMap<(u8, u32, u32), &TradeRouteRow> = route_rows.iter().map(|row| ((row.class, row.country, row.other), row)).collect();
    let ours = |raw: u32| links.get(&raw) == Some(&slot);
    let client_routes = clients.iter().filter(|row| match row.category {
        IMPORT | SUPPLY_TRANSFER => ours(row.owner),
        LEND_LEASE_U8 | MARKET => ours(row.other),
        _ => false,
    }).map(|row| (row.convoys_assigned, (row.route_class, row.route_country, row.route_other)));
    let origin_routes = origins.iter().filter(|row| row.list == 0 && ours(row.country)).map(|row| (row.convoys_assigned, (0u8, row.country, row.state)));
    let mut proof = TrafficProof::default();
    for (convoys, key) in client_routes.chain(origin_routes) {
        if convoys == 0 {
            continue;
        }
        let Some(route) = routes.get(&key).copied() else {
            proof.unknown.get_or_insert_with(|| ROUTE_NOT_RECORDED.to_owned());
            continue;
        };
        match route.kind {
            NAVAL_ROUTE if route.path.is_empty() && (route.from == 0 || route.to == 0) => {
                proof.unknown.get_or_insert_with(|| PORTS_NOT_RECORDED.to_owned());
            }
            NAVAL_ROUTE => proof.crossed.extend(route.path.iter().copied()),
            NO_ROUTE => {
                proof.unknown.get_or_insert_with(|| NO_PATH.to_owned());
            }
            _ => {}
        }
    }
    proof
}

fn intersect_spans(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let from = a[i].0.max(b[j].0);
        let to = a[i].1.min(b[j].1);
        if from <= to {
            out.push((from, to));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

pub(crate) fn missing_traffic(complete: &[Vec<(i32, i32)>], spans: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32, TrafficProof)> {
    let mut out: Vec<(i32, i32, TrafficProof)> = Vec::new();
    let inside = |held: &[(i32, i32)], hour: i32| held.iter().any(|&(a, b)| a <= hour && hour <= b);
    let mut cuts = BTreeSet::from([first]);
    for &(a, b) in complete.iter().flatten().chain(spans) {
        for edge in [a, b.saturating_add(1)] {
            if first <= edge && edge <= last {
                cuts.insert(edge);
            }
        }
    }
    let points: Vec<i32> = cuts.into_iter().collect();
    for (i, &start) in points.iter().enumerate() {
        let end = points.get(i + 1).map_or(last, |&next| next - 1);
        if inside(spans, start) {
            continue;
        }
        let at = complete.iter().position(|held| !inside(held, start)).unwrap_or(0);
        let proof = TrafficProof::unknown(TRAFFIC_TABLES[at].1);
        match out.last_mut() {
            Some(prev) if prev.2 == proof && i64::from(prev.1) + 1 == i64::from(start) => prev.1 = end,
            _ => out.push((start, end, proof)),
        }
    }
    out
}

fn complete_spans(recording: &Recording, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    let statuses = dashboard_query::cost::storage(|| lanes::statuses(recording, run, table, first, last))?;
    let mut out: Vec<(i32, i32)> = Vec::new();
    for status in statuses {
        if !status_complete(status.status) {
            continue;
        }
        let (a, b) = (status.first.max(first), status.last.min(last));
        if a > b {
            continue;
        }
        match out.last_mut() {
            Some(prev) if i64::from(prev.1) + 1 >= i64::from(a) => prev.1 = prev.1.max(b),
            _ => out.push((a, b)),
        }
    }
    Ok(out)
}

pub(crate) fn build_region_cells(
    region: u32,
    efficiency: &[(i32, i32, i64)],
    traffic: &[(i32, i32, TrafficProof)],
    known: &[HourSpan],
    unread: &[(i32, i32, String)],
    unheld: &[HourSpan],
    gaps: &[(HourSpan, &str)],
    from: i32,
    to: i32,
) -> Vec<ConvoyCellRun> {
    if from > to {
        return Vec::new();
    }
    let mut cuts = BTreeSet::from([from]);
    let mut mark = |a: i32, b: i32| {
        let (a, b) = (a.max(from), b.min(to));
        if a <= b {
            cuts.insert(a);
            if let Some(next) = b.checked_add(1).filter(|&n| n <= to) {
                cuts.insert(next);
            }
        }
    };
    for &(a, b, _) in efficiency {
        mark(a, b);
    }
    for (a, b, _) in traffic {
        mark(*a, *b);
    }
    for span in known {
        mark(span.first_hour, span.last_hour);
    }
    for &(a, b, _) in unread {
        mark(a, b);
    }
    for span in unheld {
        mark(span.first_hour, span.last_hour);
    }
    for (span, _) in gaps {
        mark(span.first_hour, span.last_hour);
    }
    let points: Vec<i32> = cuts.into_iter().collect();
    let in_spans = |spans: &[HourSpan], h: i32| {
        let idx = spans.partition_point(|s| s.last_hour < h);
        idx < spans.len() && spans[idx].first_hour <= h
    };
    let eff_at = |h: i32| {
        let idx = efficiency.partition_point(|r| r.1 < h);
        (idx < efficiency.len() && efficiency[idx].0 <= h).then(|| efficiency[idx].2)
    };
    let unread_at = |h: i32| {
        let idx = unread.partition_point(|r| r.1 < h);
        unread.get(idx).filter(|r| r.0 <= h).map(|r| r.2.clone())
    };
    let traffic_at = |h: i32| {
        let idx = traffic.partition_point(|r| r.1 < h);
        (idx < traffic.len() && traffic[idx].0 <= h).then(|| &traffic[idx].2)
    };
    let mut out: Vec<ConvoyCellRun> = Vec::new();
    for (i, &start) in points.iter().enumerate() {
        let end = points.get(i + 1).map_or(to, |&next| next - 1);
        let gap = gaps.iter().find(|(span, _)| span.first_hour <= start && start <= span.last_hour);
        let state = if let Some((_, reason)) = gap {
            ConvoyCellState::NotRecorded { reason: if reason.is_empty() { GAP_NOT_RECORDED.to_owned() } else { (*reason).to_owned() } }
        } else if in_spans(unheld, start) {
            ConvoyCellState::NotRecorded { reason: "No country holds this tag at this hour".into() }
        } else if let Some(value) = eff_at(start) {
            ConvoyCellState::Efficiency { value }
        } else if !in_spans(known, start) {
            ConvoyCellState::NotRecorded { reason: unread_at(start).unwrap_or_else(|| "Regional convoy efficiency was not recorded".into()) }
        } else {
            match traffic_at(start) {
                Some(proof) if proof.crossed.contains(&region) => ConvoyCellState::Efficiency { value: i64::from(SCALE) },
                Some(TrafficProof { unknown: Some(reason), .. }) => ConvoyCellState::NotRecorded { reason: reason.clone() },
                Some(TrafficProof { unknown: None, .. }) => ConvoyCellState::NoTraffic,
                None => ConvoyCellState::NotRecorded { reason: TRAFFIC_NOT_RECORDED.to_owned() },
            }
        };
        match out.last_mut() {
            Some(prev) if prev.state == state && i64::from(prev.last_hour) + 1 == i64::from(start) => prev.last_hour = end,
            _ => out.push(ConvoyCellRun { first_hour: start, last_hour: end, state }),
        }
    }
    out
}

#[derive(Default)]
struct Gathered {
    regions: BTreeMap<u32, Vec<(i32, i32, i64)>>,
    traffic: Vec<(i32, i32, TrafficProof)>,
    deals: BTreeMap<u128, ConvoyDeal>,
    leases: BTreeMap<u128, ConvoyLease>,
    known: [Vec<HourSpan>; 3],
    unread: Vec<(i32, i32, String)>,
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
        row.resource.extend(runs(&|cells| number(cells, resource)));
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

pub(crate) fn without(spans: &[HourSpan], gaps: &[(HourSpan, &str)]) -> Vec<HourSpan> {
    let mut out = Vec::new();
    for span in spans {
        let mut at = i64::from(span.first_hour);
        let last = i64::from(span.last_hour);
        for (gap, _) in gaps {
            let (a, b) = (i64::from(gap.first_hour), i64::from(gap.last_hour));
            if b < at || a > last {
                continue;
            }
            if a > at {
                out.push(HourSpan { first_hour: at as i32, last_hour: (a - 1) as i32 });
            }
            at = at.max(b + 1);
        }
        if at <= last {
            out.push(HourSpan { first_hour: at as i32, last_hour: last as i32 });
        }
    }
    out
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
            let read = match params.view {
                ConvoyView::Regions => complete_spans(recording, run, table, stretch.first, stretch.last)?,
                ConvoyView::Trade => complete_spans(recording, run, table, stretch.first, stretch.last)?,
            };
            gathered.known[at].extend(spans(read));
        }
        let (links, index) = match params.view {
            ConvoyView::Regions => (None, None),
            ConvoyView::Trade => (Some(dashboard_base::queries::links(source, run)?), Some(crate::products::convoys::cached_convoys(source, run)?)),
        };
        for holder in query.holders.within(&params.country, stretch.first, stretch.last) {
            let Holder::Found { slot, id } = holder.holder else { continue };
            let (first, last) = (holder.first, holder.last);
            held.push((first, last));
            match (params.view, &links, &index) {
                (ConvoyView::Regions, ..) => {
                    let owner = u128::from(id) << 32;
                    let read = history(recording, run, TableId::RegionalConvoyEfficiency, &[REGION_EFFICIENCY], Keys::Range(owner, owner | u128::from(u32::MAX)), first, last)?;
                    for held in read.statuses.iter().filter(|held| !status_complete(held.status)) {
                        if let Witness::NotRecorded(reason) = coverage::reading_witness(TableId::RegionalConvoyEfficiency, read.reading(UNREAD_PROBE, held.first)) {
                            gathered.unread.push((held.first.max(first), held.last.min(last), reason));
                        }
                    }
                    for (key, runs) in read.runs(REGION_EFFICIENCY).unwrap_or_default() {
                        let region = gathered.regions.entry(key as u32).or_default();
                        for run in numbers(&runs, &[(first, last)]) {
                            push(region, run);
                        }
                    }
                    let complete: Vec<Vec<(i32, i32)>> = TRAFFIC_TABLES.iter().map(|&(table, _)| complete_spans(recording, run, table, first, last)).collect::<io::Result<_>>()?;
                    let spans = complete.iter().skip(1).fold(complete[0].clone(), |held, other| intersect_spans(&held, other));
                    let mut traffic: Vec<(i32, i32, TrafficProof)> = missing_traffic(&complete, &spans, first, last);
                    if !spans.is_empty() {
                        let mut eval_hours = BTreeSet::new();
                        for &(a, b) in &spans {
                            eval_hours.insert(a);
                            eval_hours.insert(b);
                        }
                        let links_h = history(recording, run, TableId::CountryLinks, &[LINK_SLOT], Keys::All, first, last)?;
                        let slot_i = at(&links_h, LINK_SLOT);
                        let mut raw_ids = HashSet::from([id]);
                        for (key, steps) in &links_h.keys {
                            let ours = steps.iter().any(|step| step.cells.as_deref().is_some_and(|cells| number(cells, slot_i) == Some(i64::from(slot))));
                            if ours && let Ok(raw) = u32::try_from(*key) {
                                raw_ids.insert(raw);
                                eval_hours.extend(steps.iter().map(|step| step.hour));
                            }
                        }
                        let clients_h = history(recording, run, TableId::ConvoyClients, &[CLIENT_CATEGORY, CLIENT_OWNER, CLIENT_OTHER, 3, 4, 5, CLIENT_ASSIGNED], Keys::All, first, last)?;
                        let mut route_keys: Vec<u128> = Vec::new();
                        let cat_i = at(&clients_h, CLIENT_CATEGORY);
                        let own_i = at(&clients_h, CLIENT_OWNER);
                        let oth_i = at(&clients_h, CLIENT_OTHER);
                        let rc_i = at(&clients_h, 3);
                        let rcty_i = at(&clients_h, 4);
                        let roth_i = at(&clients_h, 5);
                        let held_raw = |value: Option<i64>| value.and_then(|v| u32::try_from(v).ok()).is_some_and(|raw| raw_ids.contains(&raw));
                        for (_, steps) in &clients_h.keys {
                            let relevant = steps.iter().any(|step| {
                                step.cells.as_deref().is_some_and(|cells| {
                                    let cat = number(cells, cat_i);
                                    matches!(cat, Some(1 | 2)) && held_raw(number(cells, own_i)) || matches!(cat, Some(4 | 6)) && held_raw(number(cells, oth_i))
                                })
                            });
                            if relevant {
                                for step in steps {
                                    eval_hours.insert(step.hour);
                                    if let Some(cells) = step.cells.as_deref()
                                        && let (Some(rc), Some(rcty), Some(roth)) = (
                                            number(cells, rc_i).and_then(|v| u8::try_from(v).ok()),
                                            number(cells, rcty_i).and_then(|v| u32::try_from(v).ok()),
                                            number(cells, roth_i).and_then(|v| u32::try_from(v).ok()),
                                        )
                                    {
                                        route_keys.push((u128::from(rcty) << 40) | (u128::from(rc) << 32) | u128::from(roth));
                                    }
                                }
                            }
                        }
                        let origins_h = history(recording, run, TableId::ResourceOrigins, &[0, 5], Keys::All, first, last)?;
                        for (key, steps) in &origins_h.keys {
                            let raw = *key as u32;
                            if raw_ids.contains(&raw) {
                                let state = (*key >> 32) as u32;
                                route_keys.push((u128::from(raw) << 40) | u128::from(state));
                                for step in steps {
                                    eval_hours.insert(step.hour);
                                }
                            }
                        }
                        route_keys.sort_unstable();
                        route_keys.dedup();
                        if !route_keys.is_empty() {
                            let routes_h = history(recording, run, TableId::TradeRoutes, &[0, 1, 2, 3], Keys::These(&route_keys), first, last)?;
                            for (_, steps) in &routes_h.keys {
                                for step in steps {
                                    eval_hours.insert(step.hour);
                                }
                            }
                        }
                        let mut seeker = Seeker::new(recording)?;
                        let traffic_tables = TRAFFIC_TABLES.map(|(table, _)| replay_lanes::at::Read::all(table));
                        for &(span_a, span_b) in &spans {
                            let hours: Vec<i32> = eval_hours.range(span_a..=span_b).copied().collect();
                            for (idx, &hour) in hours.iter().enumerate() {
                                dashboard_text::metadata::check_cancel(cancelled)?;
                                let seg_end = hours.get(idx + 1).map_or(span_b, |&next| next - 1);
                                let frame = dashboard_query::cost::storage(|| seeker.frame(run, hour, &traffic_tables))?;
                                let proof = traffic_in_frame(&frame, slot);
                                for &region in &proof.crossed {
                                    gathered.regions.entry(region).or_default();
                                }
                                traffic.push((hour, seg_end, proof));
                            }
                        }
                    }
                    traffic.sort_by_key(|run| run.0);
                    for (a, b, proof) in traffic {
                        match gathered.traffic.last_mut() {
                            Some(prev) if prev.2 == proof && i64::from(prev.1) + 1 == i64::from(a) => prev.1 = b,
                            _ => gathered.traffic.push((a, b, proof)),
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

const DAY: i32 = 24;
const ABSENT: u32 = u32::MAX;

trait Run {
    fn span(&self) -> (i32, i32);
    fn clip(&mut self, last: i32);
}

impl Run for (i32, i32, i64) {
    fn span(&self) -> (i32, i32) {
        (self.0, self.1)
    }
    fn clip(&mut self, last: i32) {
        self.1 = self.1.min(last);
    }
}

impl Run for HourSpan {
    fn span(&self) -> (i32, i32) {
        (self.first_hour, self.last_hour)
    }
    fn clip(&mut self, last: i32) {
        self.last_hour = self.last_hour.min(last);
    }
}

impl Run for ConvoyCellRun {
    fn span(&self) -> (i32, i32) {
        (self.first_hour, self.last_hour)
    }
    fn clip(&mut self, last: i32) {
        self.last_hour = self.last_hour.min(last);
    }
}

impl Run for ConvoyCountry {
    fn span(&self) -> (i32, i32) {
        (self.first_hour, self.last_hour)
    }
    fn clip(&mut self, last: i32) {
        self.last_hour = self.last_hour.min(last);
    }
}

fn unchanged_through<R: Run>(runs: &[R], at: i32) -> i32 {
    let index = runs.partition_point(|run| run.span().1 < at);
    match runs.get(index).map(Run::span) {
        Some((first, last)) if first <= at => last,
        Some((first, _)) => first - 1,
        None => i32::MAX,
    }
}

fn cut<R: Run>(runs: &mut Vec<R>, at: i32) {
    runs.retain(|run| run.span().0 <= at);
    for run in runs.iter_mut() {
        run.clip(at);
    }
}

struct Window<'q> {
    gathered: Gathered,
    known: Vec<Vec<HourSpan>>,
    unheld: Vec<HourSpan>,
    gaps: Vec<(HourSpan, &'q str)>,
}

impl Window<'_> {
    fn cells(&self, region: u32, efficiency: &[(i32, i32, i64)], from: i32, to: i32) -> Vec<ConvoyCellRun> {
        build_region_cells(region, efficiency, &self.gathered.traffic, &self.known[0], &self.gathered.unread, &self.unheld, &self.gaps, from, to)
    }
}

fn window<'q>(source: &Source, query: &'q Query<'_>, params: &ConvoyHealthParams, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Window<'q>> {
    let (mut gathered, mut held) = if from <= to { gather(source, query, params, (from, to), cancelled)? } else { (Gathered::default(), Vec::new()) };
    held.sort_unstable();
    let gaps: Vec<(HourSpan, &str)> = query
        .timeline
        .unheld
        .iter()
        .filter_map(|gap| {
            let (a, b) = (gap.first.max(from), gap.last.min(to));
            (a <= b).then_some((HourSpan { first_hour: a, last_hour: b }, gap.reason.as_str()))
        })
        .collect();
    let unheld = without(&outside(&held, from, to), &gaps);
    let known = std::mem::take(&mut gathered.known).map(dashboard_world::products::subjects::settle).to_vec();
    Ok(Window { gathered, known, unheld, gaps })
}

#[derive(Clone, Default)]
struct RegionHistory {
    reach: i32,
    regions: BTreeMap<u32, (Vec<ConvoyCellRun>, Vec<(i32, i32, i64)>)>,
    absent: Vec<ConvoyCellRun>,
    known: Vec<HourSpan>,
    unheld: Vec<HourSpan>,
    used: u64,
}

fn join_cells(held: &mut Vec<ConvoyCellRun>, next: Vec<ConvoyCellRun>) {
    let mut next = next.into_iter();
    if let (Some(last), Some(first)) = (held.last_mut(), next.as_slice().first())
        && last.state == first.state
        && i64::from(last.last_hour) + 1 == i64::from(first.first_hour)
    {
        last.last_hour = first.last_hour;
        next.next();
    }
    held.extend(next);
}

impl RegionHistory {
    fn read(source: &Source, query: &Query<'_>, params: &ConvoyHealthParams, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Self> {
        let window = window(source, query, params, from, to, cancelled)?;
        let absent = window.cells(ABSENT, &[], from, to);
        let regions = window.gathered.regions.iter().map(|(&region, efficiency)| (region, (window.cells(region, efficiency, from, to), efficiency.clone()))).collect();
        let known = window.known.into_iter().next().unwrap_or_default();
        Ok(Self { reach: to, regions, absent, known, unheld: window.unheld, used: 0 })
    }

    fn join(&mut self, next: Self) {
        for (region, (cells, _)) in &mut self.regions {
            if !next.regions.contains_key(region) {
                join_cells(cells, next.absent.clone());
            }
        }
        for (region, (cells, efficiency)) in next.regions {
            let (held_cells, held_efficiency) = self.regions.entry(region).or_insert_with(|| (self.absent.clone(), Vec::new()));
            join_cells(held_cells, cells);
            for run in efficiency {
                push(held_efficiency, run);
            }
        }
        join_cells(&mut self.absent, next.absent);
        self.known = dashboard_world::products::subjects::settle(std::mem::take(&mut self.known).into_iter().chain(next.known).collect());
        self.unheld = dashboard_world::products::subjects::settle(std::mem::take(&mut self.unheld).into_iter().chain(next.unheld).collect());
        self.reach = next.reach;
    }
}

#[derive(Default)]
struct HeldRegions {
    histories: HashMap<String, RegionHistory>,
    uses: u64,
}

const HELD_REGIONS: usize = 16;

fn region_history(source: &Source, query: &Query<'_>, params: &ConvoyHealthParams, from: i32, reach: i32, cancelled: &AtomicBool) -> io::Result<RegionHistory> {
    let key = format!("{}|{:?}|{from}", params.branch_id, params.country);
    let state = source.state::<HeldRegions>();
    let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).histories.get(&key).filter(|held| held.reach <= reach).cloned();
    let history = match held {
        Some(held) if held.reach == reach => held,
        Some(mut held) => {
            held.join(RegionHistory::read(source, query, params, held.reach + 1, reach, cancelled)?);
            held
        }
        None => RegionHistory::read(source, query, params, from, reach, cancelled)?,
    };
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.uses += 1;
    let used = state.uses;
    if state.histories.len() >= HELD_REGIONS && !state.histories.contains_key(&key)
        && let Some(oldest) = state.histories.iter().min_by_key(|(_, held)| held.used).map(|(key, _)| key.clone())
    {
        state.histories.remove(&oldest);
    }
    state.histories.insert(key, RegionHistory { used, ..history.clone() });
    Ok(history)
}

fn has_efficiency(cells: &[ConvoyCellRun]) -> bool {
    cells.iter().any(|run| matches!(run.state, ConvoyCellState::Efficiency { .. }))
}

#[derive(Clone, Default)]
struct Listed {
    through: i32,
    deals: BTreeSet<u128>,
    leases: BTreeSet<u128>,
}

#[derive(Default)]
struct HeldListed(HashMap<String, Listed>);

const HELD_LISTED: usize = 32;

fn listed_since(source: &Source, query: &Query<'_>, params: &ConvoyHealthParams, from: i32, through: i32, cancelled: &AtomicBool) -> io::Result<Listed> {
    let key = format!("{}|{:?}|{from}", params.branch_id, params.country);
    let state = source.state::<HeldListed>();
    let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.get(&key).filter(|held| held.through <= through).cloned();
    let (start, mut listed) = held.map_or_else(|| (from, Listed::default()), |held| (held.through + 1, held));
    if start > through {
        return Ok(listed);
    }
    for stretch in query.timeline.within(start, through) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let index = cached_convoys(source, stretch.run)?;
        for holder in query.holders.within(&params.country, stretch.first, stretch.last) {
            let Holder::Found { id, .. } = holder.holder else { continue };
            listed.deals.extend(rows_of(&index.deals, id, holder.first, holder.last).into_keys());
            listed.leases.extend(rows_of(&index.leases, id, holder.first, holder.last).into_keys());
        }
    }
    listed.through = through;
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.0.len() >= HELD_LISTED && !state.0.contains_key(&key) {
        state.0.clear();
    }
    state.0.insert(key, listed.clone());
    Ok(listed)
}

fn health(source: &Source, params: &ConvoyHealthParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ConvoyHealthReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let from = params.from.unwrap_or(query.timeline.first).max(query.timeline.first);
    let to = params.to.unwrap_or(query.timeline.last).min(query.timeline.last);
    let reach = if from <= to { to.div_euclid(DAY).saturating_mul(DAY).saturating_add(DAY - 1).min(query.timeline.last).max(to) } else { to };
    let every = params.every == Some(true);
    let mut warnings = Vec::new();
    let gap_spans: Vec<HourSpan> = query.timeline.unheld.iter().filter_map(|gap| {
        let (a, b) = (gap.first.max(from), gap.last.min(reach));
        (a <= b).then_some(HourSpan { first_hour: a, last_hour: b })
    }).collect();
    let (mut history, mut gathered, mut known, mut unheld) = match params.view {
        ConvoyView::Regions => {
            let history = if from <= to { region_history(source, &query, params, from, reach, cancelled)? } else { RegionHistory::read(source, &query, params, from, reach, cancelled)? };
            let (known, unheld) = (vec![history.known.clone(), Vec::new(), Vec::new()], history.unheld.clone());
            (history, Gathered::default(), known, unheld)
        }
        ConvoyView::Trade => {
            let held = window(source, &query, params, from, reach, cancelled)?;
            (RegionHistory::default(), held.gathered, held.known, held.unheld)
        }
    };
    let mut valid = reach;
    for spans in known.iter().chain([&unheld, &gap_spans]) {
        valid = valid.min(unchanged_through(spans, to));
    }
    let done = params.list_from.filter(|&list_from| params.view == ConvoyView::Trade && list_from < from).map(|list_from| listed_since(source, &query, params, list_from, from - 1, cancelled)).transpose()?;

    let mut region_ids: BTreeSet<u32> = BTreeSet::new();
    if params.view == ConvoyView::Regions {
        region_ids.extend(history.regions.keys().copied());
        region_ids.extend(params.regions.iter().flatten().copied());
        region_ids.extend(params.pinned_regions.iter().flatten().copied());
    }
    let named = region_names(source, context, &region_ids, &mut warnings);
    let absent = (params.view == ConvoyView::Regions).then(|| {
        let mut cells = history.absent.clone();
        valid = valid.min(unchanged_through(&cells, to));
        cut(&mut cells, to);
        cells
    });
    let mut rank_missing = None;
    let mut ranked_view = false;
    let mut hidden_active = 0u32;
    let mut active_unlisted = ActiveRegions::default();
    let mut all_regions: BTreeMap<u32, ConvoyRegion> = BTreeMap::new();
    for region in region_ids {
        let (mut cells, mut efficiency) = history.regions.remove(&region).unwrap_or_else(|| (history.absent.clone(), Vec::new()));
        valid = valid.min(unchanged_through(&cells, to));
        cut(&mut cells, to);
        cut(&mut efficiency, to);
        let readout = cells.last().map(|run| run.state.clone());
        let (name, missing) = match named.get(&region) {
            Some(Ok(name)) => (Some(name.clone()), None),
            Some(Err(why)) => (None, Some(why.clone())),
            None => (None, Some(format!("Region {region} absent from the recorded stack"))),
        };
        all_regions.insert(region, ConvoyRegion { region, name, missing, pinned: false, cells, readout, efficiency });
    }
    let regions: Vec<ConvoyRegion> = if let Some(explicit) = &params.regions {
        explicit
            .iter()
            .filter_map(|id| {
                let mut entry = all_regions.remove(id)?;
                entry.pinned = true;
                Some(entry)
            })
            .collect()
    } else if !every && (params.top.is_some() || params.pinned_regions.is_some() || params.hidden_regions.is_some()) {
        let mut seen = HashSet::new();
        let pinned_list: Vec<u32> = params.pinned_regions.iter().flatten().copied().filter(|id| seen.insert(*id)).collect();
        let hidden: HashSet<u32> = params.hidden_regions.iter().flatten().copied().collect();
        ranked_view = true;
        let mut auto: Vec<(u32, i64)> = Vec::new();
        match absent.as_ref().and_then(|cells| cells.last()).map(|run| &run.state) {
            Some(ConvoyCellState::NotRecorded { reason }) => rank_missing = Some(reason.clone()),
            _ => {
                auto = all_regions
                    .iter()
                    .filter(|(id, _)| !seen.contains(id))
                    .filter_map(|(&id, entry)| match entry.cells.last().map(|run| &run.state) {
                        Some(ConvoyCellState::Efficiency { value }) => Some((id, i64::from(SCALE) - value)),
                        _ => None,
                    })
                    .collect();
            }
        }
        auto.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let chosen = automatic(auto.into_iter().map(|(id, _)| id), &hidden, params.top.unwrap_or(usize::MAX), rank_missing.clone());
        hidden_active = chosen.hidden;
        active_unlisted = chosen.active_unlisted;
        let listed = chosen.listed;
        let mut ordered = Vec::with_capacity(pinned_list.len() + listed.len());
        for id in pinned_list {
            if let Some(mut entry) = all_regions.remove(&id) {
                entry.pinned = true;
                ordered.push(entry);
            }
        }
        for id in listed {
            if let Some(entry) = all_regions.remove(&id) {
                ordered.push(entry);
            }
        }
        ordered
    } else {
        all_regions.into_values().filter(|entry| has_efficiency(&entry.cells)).collect()
    };

    let listed = |key: &u128, set: fn(&Listed) -> &BTreeSet<u128>| done.as_ref().is_some_and(|done| set(done).contains(key));
    let mut deals: BTreeMap<u128, ConvoyDeal> = std::mem::take(&mut gathered.deals);
    let mut leases: BTreeMap<u128, ConvoyLease> = std::mem::take(&mut gathered.leases);
    if let Some(done) = &done {
        for &key in &done.deals {
            deals.entry(key).or_insert_with(|| deal(key));
        }
        for &key in &done.leases {
            leases.entry(key).or_insert_with(|| lease(key));
        }
    }
    for row in deals.values_mut() {
        valid = valid.min(unchanged_through(&row.exporter, to)).min(unchanged_through(&row.resource, to)).min(unchanged_through(&row.efficiency, to));
        cut(&mut row.exporter, to);
        cut(&mut row.resource, to);
        cut(&mut row.efficiency, to);
        row.factors.iter_mut().chain(row.convoys.iter_mut()).for_each(|runs| cut(runs, to));
    }
    for row in leases.values_mut() {
        valid = valid.min(unchanged_through(&row.giver, to)).min(unchanged_through(&row.bad, to));
        cut(&mut row.giver, to);
        cut(&mut row.bad, to);
        row.convoys.iter_mut().for_each(|runs| cut(runs, to));
    }
    let mut deals: Vec<ConvoyDeal> = deals
        .into_iter()
        .filter(|(key, deal)| listed(key, |done| &done.deals) || !deal.resource.is_empty() || !deal.efficiency.is_empty() || !deal.exporter.is_empty())
        .map(|(_, deal)| deal)
        .collect();
    let mut leases: Vec<ConvoyLease> = leases.into_iter().filter(|(key, lease)| listed(key, |done| &done.leases) || !lease.bad.is_empty()).map(|(_, lease)| lease).collect();
    let resources = if params.view == ConvoyView::Trade {
        let shown: BTreeSet<u32> = deals.iter().flat_map(|deal| deal.resource.iter().filter_map(|run| u32::try_from(run.2).ok())).collect();
        let tokens = resource_tokens(&query, Some(&shown), &mut warnings)?;
        let (infos, notes) = ResourceNaming::stored(source, context).infos(&tokens, cancelled)?;
        warnings.extend(notes);
        infos.into_iter().map(|info| ConvoyResource { id: info.id, token: info.token, name: info.name }).collect()
    } else {
        Vec::new()
    };
    if params.view == ConvoyView::Trade && (!deals.is_empty() || !leases.is_empty()) {
        let mut looks = LookResolver::new(source, &query.timeline, context, &mut warnings)?;
        for country in deals.iter_mut().flat_map(|d| d.exporter.iter_mut()).chain(leases.iter_mut().flat_map(|l| l.giver.iter_mut())) {
            if let Some(tag) = &country.tag {
                let look = looks.at(tag, country.last_hour)?;
                country.name = look.name;
                country.name_missing = look.name_missing;
            }
        }
    }
    known.iter_mut().for_each(|spans| cut(spans, to));
    cut(&mut unheld, to);
    let mut gaps = gap_spans;
    cut(&mut gaps, to);
    Ok(ConvoyHealthReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        tag: query.holders.name(&params.country, to),
        first_hour: from,
        last_hour: to,
        valid_through: valid.max(to),
        scale: SCALE,
        known,
        unheld,
        gaps,
        empty: no_regions(!ranked_view || !regions.is_empty(), rank_missing.as_deref(), hidden_active, 0),
        regions,
        rank_missing,
        active_unlisted,
        absent: every.then_some(absent).flatten(),
        deals,
        leases,
        resources,
        warnings,
    })
}

impl Product for ConvoyHealth {
    const KEY: &'static str = KEY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = ConvoyHealthParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, view, country, from: Start, to: End, regions, pinnedRegions, hiddenRegions, top, listFrom: Start, every, "regions[]", "pinnedRegions[]", "hiddenRegions[]"];
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
            ConvoyCellState::decl(),
            ConvoyCellRun::decl(),
            ConvoyCountry::decl(),
            ConvoyRegion::decl(),
            ConvoyDeal::decl(),
            ConvoyLease::decl(),
            ConvoyResource::decl(),
            NoRegions::decl(),
            ActiveRegions::decl(),
            HourSpan::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        health(source, params, context, cancelled)
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
