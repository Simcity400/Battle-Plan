
use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::identities::CountryIdentityRun;
use dashboard_base::{Source, span::HourSpan};
use dashboard_base::products::{
    resources::{RESOURCE_DECIMALS, ResourceInfo, ResourceNaming},
};
use dashboard_query::{Query, identity::Holder};
use replay_lanes::lanes::{self, Cell, Keys, Step};
use replay_data::rows::{TableId, TagHolderRow, field_of_key};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const KEY: &str = "trade-flows";
const COLUMNS: [usize; 5] = [0, 1, 2, 3, 4];
const DEALS: usize = 4;

pub struct TradeFlows;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeFlowParams {
    pub branch_id: String,
    #[doc = " Every country's trade when none; else the flows any holder of these tags sold or bought."]
    pub countries: Option<Vec<Country>>,
    #[doc = " The window of hours read ([`super::windows`]): never the whole timeline in one reply."]
    pub window: u32,
    #[doc = " The hours of the window read; all of it when not given."]
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TradeFlow {
    #[doc = " Indexes into the reply's countries: the sides' tags at the flow's first run."]
    pub seller: u32,
    pub buyer: u32,
    #[doc = " The sides' holders, `[slot,id]`."]
    pub seller_identity: String,
    pub buyer_identity: String,
    #[doc = " Each run's sides' tags."]
    pub seller_tags: Vec<u32>,
    pub buyer_tags: Vec<u32>,
    pub resource: u32,
    pub first_hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    #[doc = " Fixed-point daily amount delivered, exact."]
    pub delivered_raw: Vec<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number>"))]
    pub factories: Vec<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number>"))]
    pub convoys_assigned: Vec<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number>"))]
    pub convoys_required: Vec<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<number>"))]
    pub deals: Vec<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TradeCountryIdentity {
    pub tag: String,
    pub runs: Vec<CountryIdentityRun>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PartialSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub deals: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TradeFlowsReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Fixed-point decimals of every amount."]
    pub decimals: u8,
    #[doc = " Every recorded resource, by native index."]
    pub resources: Vec<ResourceInfo>,
    #[doc = " Every tag a flow names, in byte order."]
    pub countries: Vec<String>,
    #[doc = " Each listed tag's holders over the span."]
    pub country_identities: Vec<TradeCountryIdentity>,
    #[doc = " Hours the deals were read: outside them nothing is known, not no trade."]
    pub known: Vec<HourSpan>,
    #[doc = " Hours some deals were left out of the flows."]
    pub partial: Vec<PartialSpan>,
    pub flows: Vec<TradeFlow>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TradeFlowsReply);

type FlowKey = ((u32, u32), (u32, u32), u32);

type Terms = [i64; 5];

fn terms(cells: Option<&[Cell]>) -> Option<Terms> {
    let cells = cells?;
    let mut out = [0; 5];
    for (at, value) in out.iter_mut().enumerate() {
        let Some(Cell::Number(number)) = cells.get(at) else { return None };
        *value = *number;
    }
    Some(out)
}

fn deals(cells: Option<&[Cell]>) -> i64 {
    match cells.and_then(|cells| cells.get(DEALS)) {
        Some(Cell::Number(deals)) => (*deals).max(0),
        _ => 0,
    }
}

pub(crate) fn step_runs(steps: &[Step], to: i32) -> Vec<(i32, i32, Result<Terms, i64>)> {
    let mut out: Vec<(i32, i32, Result<Terms, i64>)> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let last = steps.get(index + 1).map_or(to, |next| next.hour - 1);
        if step.hour > last {
            continue;
        }
        let value = match step.cells.as_deref() {
            None => continue,
            cells => terms(cells).ok_or_else(|| deals(cells)),
        };
        match out.last_mut() {
            Some(held) if held.2 == value && i64::from(held.1) + 1 == i64::from(step.hour) => held.1 = last,
            _ => out.push((step.hour, last, value)),
        }
    }
    out
}

pub(crate) fn identity_tags(query: &Query<'_>) -> HashMap<(u32, u32), Vec<(String, i32, i32)>> {
    let mut out: HashMap<(u32, u32), Vec<(String, i32, i32)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                out.entry((slot, id)).or_default().push((tag.clone(), run.first, run.last));
            }
        }
    }
    for runs in out.values_mut() {
        runs.sort_unstable_by_key(|run| run.1);
    }
    out
}

pub(crate) fn tagged<'a>(seller: &'a [(String, i32, i32)], buyer: &'a [(String, i32, i32)], first: i32, last: i32) -> Vec<(i32, i32, Option<(&'a str, &'a str)>)> {
    let mut edges: Vec<i32> = vec![first];
    for &(_, a, b) in seller.iter().chain(buyer) {
        for edge in [a, b.saturating_add(1)] {
            if edge > first && edge <= last {
                edges.push(edge);
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let at = |runs: &'a [(String, i32, i32)], hour: i32| runs.iter().find(|run| run.1 <= hour && hour <= run.2).map(|run| run.0.as_str());
    let mut out: Vec<(i32, i32, Option<(&str, &str)>)> = Vec::new();
    for (index, &start) in edges.iter().enumerate() {
        let end = edges.get(index + 1).map_or(last, |next| next - 1);
        let tags = at(seller, start).zip(at(buyer, start));
        match out.last_mut() {
            Some(held) if held.2 == tags => held.1 = end,
            _ => out.push((start, end, tags)),
        }
    }
    out
}

pub(crate) fn partial(left: &[(i32, i32, i64)]) -> Vec<PartialSpan> {
    let mut edges: Vec<(i64, i64)> = left.iter().flat_map(|&(a, b, deals)| [(i64::from(a), deals), (i64::from(b) + 1, -deals)]).collect();
    edges.sort_unstable();
    let mut out: Vec<PartialSpan> = Vec::new();
    let mut sum = 0i64;
    for (index, &(hour, change)) in edges.iter().enumerate() {
        sum += change;
        let Some(&(next, _)) = edges.get(index + 1) else { break };
        if next == hour || sum <= 0 {
            continue;
        }
        let (first_hour, last_hour) = (hour as i32, (next - 1) as i32);
        match out.last_mut() {
            Some(held) if held.deals == sum && i64::from(held.last_hour) + 1 == hour => held.last_hour = last_hour,
            _ => out.push(PartialSpan { first_hour, last_hour, deals: sum }),
        }
    }
    out
}

#[derive(Default)]
struct Gathered {
    flows: BTreeMap<FlowKey, Vec<(i32, i32, String, String, Terms)>>,
    left: Vec<(i32, i32, i64)>,
    known: Vec<HourSpan>,
}

pub(crate) fn both(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            out.push((first, last));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn holder(key: u128, shift: u32) -> Option<(u32, u32)> {
    let (slot, id) = (field_of_key(key, shift + 24), field_of_key(key, shift));
    (slot != TagHolderRow::NONE && id != TagHolderRow::NONE).then_some((slot, id))
}

fn gather(
    query: &Query<'_>,
    run: u32,
    (first, last): (i32, i32),
    tags: &HashMap<(u32, u32), Vec<(String, i32, i32)>>,
    chosen: Option<&HashSet<(u32, u32)>>,
    defined: &BTreeSet<u32>,
    gathered: &mut Gathered,
) -> io::Result<()> {
    let recording = query.replay.recording();
    let read = dashboard_query::cost::storage(|| lanes::available_reads(recording, run, TableId::ResourceExchanges, first, last))?;
    let written: Vec<(i32, i32)> = dashboard_query::cost::storage(|| lanes::statuses(recording, run, TableId::TradeFlows, first, last))?
        .into_iter()
        .filter(|held| replay_recording::columnar_frames::available(held.status))
        .map(|held| (held.first, held.last))
        .collect();
    gathered.known.extend(both(&read, &written).into_iter().map(|(first_hour, last_hour)| HourSpan { first_hour, last_hour }));
    let history = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::TradeFlows, &COLUMNS, Keys::All, first, last))?;
    let none: &[(String, i32, i32)] = &[];
    for (key, steps) in &history.keys {
        let (seller, buyer, resource) = (holder(*key, 80), holder(*key, 32), *key as u32);
        if let (Some(chosen), Some(seller), Some(buyer)) = (chosen, seller, buyer)
            && !chosen.contains(&seller)
            && !chosen.contains(&buyer)
        {
            continue;
        }
        for (a, b, value) in step_runs(steps, history.to) {
            let terms = match value {
                Ok(terms) => terms,
                Err(deals) => {
                    gathered.left.push((a, b, deals));
                    continue;
                }
            };
            let (Some(seller), Some(buyer)) = (seller, buyer) else {
                gathered.left.push((a, b, terms[DEALS].max(0)));
                continue;
            };
            if !defined.contains(&resource) {
                gathered.left.push((a, b, terms[DEALS].max(0)));
                continue;
            }
            let (seller_tags, buyer_tags) = (tags.get(&seller).map_or(none, Vec::as_slice), tags.get(&buyer).map_or(none, Vec::as_slice));
            for (from, to, named) in tagged(seller_tags, buyer_tags, a, b) {
                match named {
                    Some((s, b)) => gathered.flows.entry((seller, buyer, resource)).or_default().push((from, to, s.to_owned(), b.to_owned(), terms)),
                    None => gathered.left.push((from, to, terms[DEALS].max(0))),
                }
            }
        }
    }
    Ok(())
}

fn flows(gathered: BTreeMap<FlowKey, Vec<(i32, i32, String, String, Terms)>>) -> (Vec<String>, Vec<TradeFlow>) {
    let countries: Vec<String> = gathered.values().flatten().flat_map(|run| [run.2.clone(), run.3.clone()]).collect::<BTreeSet<_>>().into_iter().collect();
    let index = |tag: &str| countries.binary_search_by(|held| held.as_str().cmp(tag)).map_or(0, |at| at as u32);
    let identity = |(slot, id): (u32, u32)| format!("[{slot},{id}]");
    let mut out: Vec<TradeFlow> = gathered
        .into_iter()
        .filter(|(_, runs)| !runs.is_empty())
        .map(|((seller, buyer, resource), mut runs)| {
            runs.sort_unstable_by_key(|run| run.0);
            let mut flow = TradeFlow { seller: index(&runs[0].2), buyer: index(&runs[0].3), seller_identity: identity(seller), buyer_identity: identity(buyer), resource, ..TradeFlow::default() };
            for (first, last, s, b, terms) in runs {
                flow.first_hours.push(first);
                flow.last_hours.push(last);
                flow.seller_tags.push(index(&s));
                flow.buyer_tags.push(index(&b));
                flow.delivered_raw.push(terms[0].to_string());
                flow.factories.push(terms[1]);
                flow.convoys_assigned.push(terms[2]);
                flow.convoys_required.push(terms[3]);
                flow.deals.push(terms[4]);
            }
            flow
        })
        .collect();
    out.sort_by(|a, b| (a.seller, a.buyer, a.resource, &a.seller_identity, &a.buyer_identity).cmp(&(b.seller, b.buyer, b.resource, &b.seller_identity, &b.buyer_identity)));
    (countries, out)
}

impl Product for TradeFlows {
    const KEY: &'static str = KEY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = TradeFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, window, from: Start, to: End, "countries[]"];
    type Reply = TradeFlowsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TradeFlowParams::decl(), TradeFlowsReply::decl(), TradeFlow::decl(), TradeCountryIdentity::decl(), CountryIdentityRun::decl(), PartialSpan::decl(), ResourceInfo::decl(), HourSpan::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_world::products::windows::hours(params.window).ok_or("Invalid trade flows window")?;
        if let (Some(from), Some(to)) = (params.from, params.to)
            && from > to
        {
            return Err("Invalid trade flow hours");
        }
        for tag in params.countries.iter().flatten() {
            dashboard_base::products::validate_country(tag)?;
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let (first, last) = dashboard_world::products::windows::within(&query, params.window)?;
        let from = params.from.map_or(first, |from| from.max(first));
        let to = params.to.map_or(last, |to| to.min(last));
        let mut warnings = Vec::new();
        let tokens = super::convoys::resource_tokens(&query, None, &mut warnings)?;
        let (resources, notes) = ResourceNaming::stored(source, context).infos(&tokens, cancelled)?;
        warnings.extend(notes);
        let defined: BTreeSet<u32> = resources.iter().map(|resource| resource.id).collect();
        let tags = identity_tags(&query);
        let chosen: Option<HashSet<(u32, u32)>> = params.countries.as_ref().map(|asked| {
            asked
                .iter()
                .flat_map(|tag| query.holders.within(tag, from, to))
                .filter_map(|run| match run.holder {
                    Holder::Found { slot, id } => Some((slot, id)),
                    Holder::Unresolved => None,
                })
                .collect()
        });
        let mut gathered = Gathered::default();
        if from <= to {
            for stretch in query.timeline.within(from, to) {
                dashboard_text::metadata::check_cancel(cancelled)?;
                gather(&query, stretch.run, (stretch.first, stretch.last), &tags, chosen.as_ref(), &defined, &mut gathered)?;
            }
        }
        let partial = partial(&gathered.left);
        let known = dashboard_world::products::subjects::settle(gathered.known);
        let (countries, flows) = flows(gathered.flows);
        let country_identities = countries
            .iter()
            .map(|tag| TradeCountryIdentity {
                tag: tag.clone(),
                runs: query
                    .holders
                    .within(tag, from, to)
                    .into_iter()
                    .filter_map(|run| match run.holder {
                        Holder::Found { slot, id } => Some(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
                        Holder::Unresolved => None,
                    })
                    .collect(),
            })
            .collect();
        Ok(TradeFlowsReply { branch_id: params.branch_id.clone(), first_hour: from, last_hour: to, decimals: RESOURCE_DECIMALS, resources, countries, country_identities, known, partial, flows, warnings })
    }
}

#[cfg(test)]
mod tests;
