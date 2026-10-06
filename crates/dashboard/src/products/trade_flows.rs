//! Who trades what with whom over a branch, for the Trade flows widget: every
//! classic resource trade deal (`ResourceExchanges`) as a flow from its seller
//! (the exporter) to its buyer (the importer) for one resource.
//!
//! The values are the ones Resource balance already shows per country, with the
//! same evidence (`resource_balance.rs`): `delivered` is a deal's daily amount,
//! exact against the save's `delivered`; `assigned_factories` is the civilian
//! factories the buyer pays, exact against the save's `lended_cic`. The convoys
//! (`convoys_assigned` of `convoys_required`) are shown as recorded: the save
//! only confirms whether a deal needs convoys at all (the fresh-save oracle's
//! `convoys_required_nonzero`), not the counts.
//!
//! A flow is one (seller, buyer, resource): several deals between the same two
//! countries for the same resource are summed, and `deals` says how many there
//! were. Countries are named by their tag at the read, as Resource balance names
//! trade partners; the widget names and flags them as the recording showed them.
//!
//! The deal table is read every hour (`replay::cadence`). Deals are known only
//! where the table and country catalogue are recorded: `known` lists those hours,
//! so an hour outside them is unknown, never "no trade". An hour listing deals it
//! cannot attribute (a seller or buyer the links do not resolve, a resource the
//! definitions do not name) is known only in part: `partial` lists those hours
//! with how many deals were left out, so they are never shown as the whole trade.
//!
//! `PurchaseContracts` (the international market's equipment contracts, with a
//! seller, a buyer, civilian industry paid and deliveries made) is not read here:
//! it trades equipment, not resources, and the recorded table has no equipment
//! type or quantity to size a flow by (`engine::cpurchase_contract`).
//!
//! The reply covers the whole branch so playback is a local lookup. A reply past
//! the product byte budget is refused with a message asking for fewer countries,
//! never cut short.
//!
//! Runs end at a gap, a missing table or a change in a named country's identity
//! (slot, tag and id), as Resource balance's runs do.

use super::{
    LoadContext, Product,
    resource_balance::mark,
    resources::{Learned, RESOURCE_DECIMALS, resource_infos},
    validate_tag,
};
use crate::{
    Source,
    metadata::{link_slot, mapped_slot, validate_campaign},
    products::country_groups::HourSpan,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct TradeFlows;

const CACHED_REQUESTS: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeFlowParams {
    pub branch_id: String,
    /// Only flows with one of these countries as seller or buyer; every flow when null.
    pub tags: Option<Vec<String>>,
}

impl TradeFlowParams {
    fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }

    /// The countries asked for, sorted and each once; none for the whole world.
    fn countries(&self) -> Option<Vec<String>> {
        self.tags.as_ref().map(|tags| {
            tags.iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
    }
}

/// One seller's sales of one resource to one buyer over the branch, as runs of
/// consecutive hours over which the summed terms stay the same. The run arrays run
/// in parallel, one entry per run, in hour order, never overlapping.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeFlow {
    /// The seller's index in `countries`.
    pub seller: u32,
    /// The buyer's index in `countries`.
    pub buyer: u32,
    /// The resource's native index.
    pub resource: u32,
    pub first_hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    /// The daily amount delivered, summed over the flow's deals, as an exact
    /// decimal string with `decimals` places.
    pub delivered_raw: Vec<String>,
    /// Civilian factories the buyer pays.
    pub factories: Vec<i32>,
    pub convoys_assigned: Vec<i32>,
    pub convoys_required: Vec<i32>,
    /// Deals summed into the run.
    pub deals: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeFlowsReply {
    pub branch_id: String,
    pub decimals: u8,
    /// Every recorded resource, in the game's order.
    pub resources: Vec<super::resources::ResourceInfo>,
    /// The tags flows name, sorted.
    pub countries: Vec<String>,
    /// Hours the deal table and country catalogue were recorded, ascending.
    pub reads: Vec<i32>,
    /// Hours whose deals are known, as ascending spans. Outside them the deals
    /// are unknown, not none.
    pub known: Vec<HourSpan>,
    /// Known hours whose read also listed deals that could not be attributed (a
    /// seller or buyer the links do not resolve, a resource the definitions do not
    /// name), as ascending spans: the flows there are only part of the trade.
    pub partial: Vec<PartialSpan>,
    /// By seller, buyer and resource.
    pub flows: Vec<TradeFlow>,
    pub warnings: Vec<String>,
}

/// Consecutive known hours whose read left out the same number of deals.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartialSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Deals the read listed but could not attribute.
    pub deals: u32,
}

/// Each canonical slot's country this hour: its tag and native id, from the first
/// row whose identity the links resolve (`mapped_slot`), as every per-country
/// product reads a country. A slot whose rows the links do not resolve is absent.
fn identities<'a>(
    countries: &'a [replay::rows::CountryRow],
    links: &[replay::rows::CountryLinkRow],
) -> HashMap<u32, (&'a str, u32)> {
    let mut ids = HashMap::with_capacity(countries.len());
    for country in countries {
        let (Ok(slot), Ok(tag)) = (
            mapped_slot(country, links),
            std::str::from_utf8(&country.tag),
        ) else {
            continue;
        };
        ids.entry(slot).or_insert((tag, country.id));
    }
    ids
}

/// Adds a known hour with `deals` left out, joining it to the last span when it
/// follows it with the same count.
fn mark_partial(spans: &mut Vec<PartialSpan>, hour: i32, deals: u32) {
    match spans.last_mut() {
        Some(span) if span.deals == deals && span.last_hour.checked_add(1) == Some(hour) => {
            span.last_hour = hour
        }
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(PartialSpan {
            first_hour: hour,
            last_hour: hour,
            deals,
        }),
    }
}

/// A flow's summed terms at a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Terms {
    delivered: i64,
    factories: i32,
    assigned: i32,
    required: i32,
    deals: u32,
}

impl Terms {
    fn add(self, row: &replay::rows::ResourceExchangeRow) -> Option<Self> {
        Some(Self {
            delivered: self.delivered.checked_add(row.delivered)?,
            factories: self.factories.checked_add(row.assigned_factories)?,
            assigned: self.assigned.checked_add(row.convoys_assigned)?,
            required: self.required.checked_add(row.convoys_required)?,
            deals: self.deals.checked_add(1)?,
        })
    }
}

#[derive(Clone, Debug)]
struct Run {
    first_hour: i32,
    last_hour: i32,
    terms: Terms,
}

#[derive(Clone, Debug)]
struct Flow {
    seller: String,
    buyer: String,
    resource: u32,
    runs: Vec<Run>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedFlows {
    /// Resources in the game's order: native index and token.
    pub resources: Vec<(u32, String)>,
    pub reads: Vec<i32>,
    pub known: Vec<HourSpan>,
    pub partial: Vec<PartialSpan>,
    flows: Vec<Flow>,
    /// Deal rows naming a resource the recorded definitions or tokens do not name.
    pub unnamed: usize,
    /// Deal rows whose seller or buyer the recorded links do not resolve.
    pub unresolved: usize,
}

impl ScannedFlows {
    /// The flows as the reply lists them: countries by index into the sorted tags.
    pub fn reply_flows(&self) -> (Vec<String>, Vec<TradeFlow>) {
        let countries: Vec<String> = self
            .flows
            .iter()
            .flat_map(|flow| [flow.seller.clone(), flow.buyer.clone()])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let index = |tag: &str| {
            countries
                .binary_search_by(|item| item.as_str().cmp(tag))
                .expect("listed") as u32
        };
        let mut flows: Vec<TradeFlow> = self
            .flows
            .iter()
            .map(|flow| TradeFlow {
                seller: index(&flow.seller),
                buyer: index(&flow.buyer),
                resource: flow.resource,
                first_hours: flow.runs.iter().map(|run| run.first_hour).collect(),
                last_hours: flow.runs.iter().map(|run| run.last_hour).collect(),
                delivered_raw: flow
                    .runs
                    .iter()
                    .map(|run| run.terms.delivered.to_string())
                    .collect(),
                factories: flow.runs.iter().map(|run| run.terms.factories).collect(),
                convoys_assigned: flow.runs.iter().map(|run| run.terms.assigned).collect(),
                convoys_required: flow.runs.iter().map(|run| run.terms.required).collect(),
                deals: flow.runs.iter().map(|run| run.terms.deals).collect(),
            })
            .collect();
        flows.sort_by_key(|flow| (flow.seller, flow.buyer, flow.resource));
        (countries, flows)
    }
}

/// Scans one branch for every trade deal, or those of `tags` as seller or buyer.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: Option<&[String]>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedFlows> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ResourceDefinitions,
        TableId::ResourceExchanges,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Trade request cancelled");
    let wanted = |tag: &str| {
        tags.is_none_or(|tags| tags.binary_search_by(|item| item.as_str().cmp(tag)).is_ok())
    };
    let mut result = ScannedFlows::default();
    let mut learned = Learned::default();
    let mut index: HashMap<(String, String, u32), usize> = HashMap::new();
    let mut known = false;
    // The countries the previous hour's flows name: slot, tag and country id.
    let mut carried: Vec<(u32, String, u32)> = Vec::new();
    let mut last_hour: Option<i32> = None;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = frame.hour;
            let continues = last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
            last_hour = Some(hour);
            let exchanges = decoded.complete::<tables::ResourceExchanges>().ok();
            if !continues || exchanges.is_none() {
                known = false;
            }
            let Some(exchanges) = exchanges else {
                continue;
            };
            let (Ok(definitions), Ok(lexical), Ok(countries), Ok(links)) = (
                decoded.complete::<tables::ResourceDefinitions>(),
                decoded.complete::<tables::LexicalTokens>(),
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                // A read that cannot say who trades what: unknown from here.
                known = false;
                continue;
            };
            learned.learn(definitions, lexical);
            let ids = identities(countries, links);
            let same = carried
                .iter()
                .all(|(slot, tag, id)| ids.get(slot) == Some(&(tag.as_str(), *id)));
            let mut read: BTreeMap<(&str, &str, u32), Terms> = BTreeMap::new();
            // The slots this read's flows name, with the country each held.
            let mut named: BTreeSet<u32> = BTreeSet::new();
            // Deals this read lists but cannot attribute: the hour is known only in part.
            let mut skipped = 0_u32;
            for exchange in exchanges {
                let side = |raw| {
                    link_slot(links, raw)
                        .and_then(|slot| ids.get(&slot).map(|(tag, _)| (slot, *tag)))
                };
                let (Some((seller_slot, seller)), Some((buyer_slot, buyer))) =
                    (side(exchange.exporter), side(exchange.importer))
                else {
                    // Whoever it names could be one of the countries asked about.
                    result.unresolved += 1;
                    skipped = skipped.saturating_add(1);
                    continue;
                };
                if !(wanted(seller) || wanted(buyer)) {
                    continue;
                }
                if !learned.contains(exchange.resource) {
                    result.unnamed += 1;
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                named.insert(seller_slot);
                named.insert(buyer_slot);
                let terms = read.entry((seller, buyer, exchange.resource)).or_default();
                *terms = terms
                    .add(exchange)
                    .ok_or_else(|| crate::invalid("recorded trade terms overflow their sum"))?;
            }
            result.reads.push(hour);
            mark(&mut result.known, hour);
            if skipped > 0 {
                mark_partial(&mut result.partial, hour, skipped);
            }
            let was_known = known && same;
            known = true;
            carried = named
                .iter()
                .map(|slot| {
                    let (tag, id) = ids[slot];
                    (*slot, tag.to_owned(), id)
                })
                .collect();
            for ((seller, buyer, resource), terms) in read {
                let key = (seller.to_owned(), buyer.to_owned(), resource);
                let flow = *index.entry(key).or_insert_with(|| {
                    result.flows.push(Flow {
                        seller: seller.to_owned(),
                        buyer: buyer.to_owned(),
                        resource,
                        runs: Vec::new(),
                    });
                    result.flows.len() - 1
                });
                let runs = &mut result.flows[flow].runs;
                match runs.last_mut() {
                    Some(run)
                        if was_known
                            && run.last_hour.checked_add(1) == Some(hour)
                            && run.terms == terms =>
                    {
                        run.last_hour = hour
                    }
                    _ => {
                        runs.push(Run {
                            first_hour: hour,
                            last_hour: hour,
                            terms,
                        });
                    }
                }
            }
        }
    }
    result.resources = learned.list();
    Ok(result)
}

/// A scan's branch and countries (none for the whole world).
type ScanKey = (u64, Option<Vec<String>>);

#[derive(Default)]
struct Cache {
    entries: VecDeque<(ScanKey, Arc<ScannedFlows>)>,
}

impl Product for TradeFlows {
    const KEY: &'static str = "trade-flows";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TradeFlowParams;
    type Reply = TradeFlowsReply;

    fn types() -> Vec<String> {
        vec![
            TradeFlowParams::decl(),
            TradeFlow::decl(),
            PartialSpan::decl(),
            TradeFlowsReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if let Some(tags) = &params.tags {
            for tag in tags {
                validate_tag(tag)?;
            }
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.countries());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    key.1.as_deref(),
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_REQUESTS {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        // Without the game folders the flows still draw, resources named by token.
        let (resources, mut warnings) =
            resource_infos(source, context, &scanned.resources, cancelled)?;
        if scanned.unnamed > 0 {
            warnings.push(format!(
                "{} recorded deal reads name a resource the recorded definitions do not; they are left out.",
                scanned.unnamed
            ));
        }
        if scanned.unresolved > 0 {
            warnings.push(format!(
                "{} recorded deal reads name a seller or buyer the recorded links do not resolve; they are left out.",
                scanned.unresolved
            ));
        }
        if scanned.reads.is_empty() {
            warnings.push("No trade deals were recorded in this branch.".to_owned());
        }
        let (countries, flows) = scanned.reply_flows();
        let reply = TradeFlowsReply {
            branch_id: params.branch_id.clone(),
            decimals: RESOURCE_DECIMALS,
            resources,
            countries,
            reads: scanned.reads.clone(),
            known: scanned.known.clone(),
            partial: scanned.partial.clone(),
            flows,
            warnings,
        };
        fits_one_reply(&reply, super::MAX_PRODUCT_REPLY_BYTES)?;
        Ok(reply)
    }
}

/// Refuses a reply longer than `limit` bytes as JSON with a message saying what to
/// do, before the transport would refuse it with none.
pub fn fits_one_reply(reply: &TradeFlowsReply, limit: usize) -> io::Result<()> {
    match crate::transport::json_len(reply, limit) {
        Some(_) => Ok(()),
        None => Err(io::Error::other(format!(
            "Trade history for these countries takes more than {} MB, more than one reply holds; choose fewer countries in the widget settings.",
            limit / (1024 * 1024)
        ))),
    }
}

#[cfg(test)]
mod tests;
