//! One country's strategic resources over a branch, for the Resource balance
//! widget: where each resource comes from and where it goes, and the trade
//! deals behind its imports and exports.
//!
//! Every value is a recorded native pool (`CountryResources`), fixed point with
//! [`RESOURCE_DECIMALS`] places, kept exact. Against a fresh text save at a
//! full-refresh hour (W2 part-G, branch 1, hour 60827807) all 3,073 country
//! resources match the save's `resources` block exactly: `produced`,
//! `adjustment` (`transfer_overlord_subject`), `imported`, `used` (the first
//! `to_use` array), `special_total` (the first plus the second), and
//! `production_demand` (the third), `to_export`, `base_export` and `exported`.
//! What they mean:
//!
//! - `used` is what the country has for use: in 3,059 of the 3,073 it is exactly
//!   produced + adjustment + imported − `to_export`, and the rest differ only by
//!   the game's rounding. What is taken off is the export reserve the trade law
//!   sets aside (`to_export`), not what buyers took (`exported`), which differs
//!   in 266 of them.
//! - `production_demand` is minus what the production lines need, met or not:
//!   Germany's lines in the save use aluminium 11, steel 30, coal 17 and
//!   tungsten 1 and lack 5 rubber, and it holds −11, −30, −17, −1 and −5.
//! - `special_total` is what is left after the second array, which is never
//!   positive. The game names that step: its resource breakdown lists "Needed by
//!   Special Projects" between what it has for use and "Needed by Production
//!   Lines" (`RESOURCE_DETAILS_SURPLUS`), and special projects declare a
//!   `resource_cost`. Which other consumers it holds, if any, is not checked.
//! - `special_total` + `production_demand` is the game's surplus, or its deficit
//!   when negative.
//!
//! `adjustment` is resources moved between a subject and its overlord: positive
//! for the overlord, negative for the subject. `base_export` (the reserve before
//! modifiers) is not shown.
//!
//! The pools and the deals are both read every hour (`replay::cadence`), so a
//! value shown at an hour is as of that hour. Values run over the consecutive
//! hours of the same country identity and end at a gap, an incomplete table or a
//! change of identity. The resource names come from `ResourceDefinitions`, read
//! daily and carried: they are content, fixed once loaded.
//!
//! The pools and the deals are captured as separate tables with their own
//! outcomes, so each keeps its own values: an hour whose deal table is missing
//! still shows the pools, and `deal_hours` says at which hours the deals are
//! known.
//!
//! Resources are told apart by their native index, not their token: the game
//! keeps every declaration as its own resource (see [`ResourceInfo`]).
//!
//! Trades are `ResourceExchanges` rows (a classic trade deal each) naming the
//! country as exporter or importer. `delivered` is the deal's daily amount,
//! exact against the save's `delivered`. At the same hour a country's import
//! deals add up to `imported` in 3,055 of the 3,073 and its export deals to
//! `exported` in 3,065, so the widget shows any remainder as its own part.

pub use super::resources::{RESOURCE_DECIMALS, ResourceInfo, ordinals};
use super::{
    CountryParams, LoadContext, Product,
    resources::{Learned, resource_infos},
};
use crate::{
    Source,
    metadata::{IdentityTrack, link_slot, tagged_country, validate_campaign},
    products::country_groups::HourSpan,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct ResourceBalance;

const CACHED_COUNTRIES: usize = 8;
/// A country's recorded pools of one resource over consecutive hours while they
/// stay unchanged. Raw values are exact decimal strings with
/// [`RESOURCE_DECIMALS`] places.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoolRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The run follows a gap, a missing table or another country identity.
    pub starts_segment: bool,
    /// Extracted in the country's states (`produced`).
    pub produced_raw: String,
    /// Moved from subjects (positive) or to the overlord (negative) (`adjustment`).
    pub subjects_raw: String,
    pub imported_raw: String,
    /// Set aside for export by the trade law (`to_export`).
    pub to_export_raw: String,
    /// What buyers took (`exported`).
    pub exported_raw: String,
    /// What the country has for use (`used`).
    pub available_raw: String,
    /// What is left for use after special projects (`special_total`).
    pub after_projects_raw: String,
    /// Minus what production lines need, met or not (`production_demand`).
    pub production_raw: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSeries {
    /// The resource's native index (`ResourceDefinitions.id`).
    pub id: u32,
    pub runs: Vec<PoolRun>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum TradeDirection {
    Import,
    Export,
}

/// One trade deal over consecutive hours while its terms stay unchanged.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeRun {
    /// The deal's native handle, as a decimal string.
    pub deal: String,
    /// The resource's native index.
    pub resource: u32,
    pub direction: TradeDirection,
    /// The other country's tag.
    pub partner: String,
    /// The daily amount delivered.
    pub delivered_raw: String,
    /// Civilian factories the importer pays.
    pub factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBalanceReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    /// Every recorded resource, in the game's order.
    pub resources: Vec<ResourceInfo>,
    /// One per resource, in `resources` order.
    pub series: Vec<ResourceSeries>,
    /// Hours whose deals are known, as ascending spans: the hours the deal table
    /// was recorded. It is captured apart from the pools, so pools shown at an
    /// hour outside these spans have unknown deals, which is not the same as no
    /// deals.
    pub deal_hours: Vec<HourSpan>,
    pub trades: Vec<TradeRun>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedBalance {
    /// Resources in the game's order: native index and token.
    pub resources: Vec<(u32, String)>,
    pub series: BTreeMap<u32, Vec<PoolRun>>,
    pub deal_hours: Vec<HourSpan>,
    pub trades: Vec<TradeRun>,
    /// Rows naming a resource the recorded definitions or tokens do not name.
    pub unnamed: usize,
    /// Deals whose other country the recorded links do not resolve.
    pub unresolved: usize,
}

/// The same recorded pools, whatever hours they cover.
fn same_pools(a: &PoolRun, b: &PoolRun) -> bool {
    [
        (&a.produced_raw, &b.produced_raw),
        (&a.subjects_raw, &b.subjects_raw),
        (&a.imported_raw, &b.imported_raw),
        (&a.to_export_raw, &b.to_export_raw),
        (&a.exported_raw, &b.exported_raw),
        (&a.available_raw, &b.available_raw),
        (&a.after_projects_raw, &b.after_projects_raw),
        (&a.production_raw, &b.production_raw),
    ]
    .into_iter()
    .all(|(a, b)| a == b)
}

/// The same deal on the same terms, whatever hours they cover.
fn same_terms(a: &TradeRun, b: &TradeRun) -> bool {
    (
        &a.deal,
        &a.resource,
        a.direction,
        &a.partner,
        &a.delivered_raw,
        a.factories,
        a.convoys_assigned,
        a.convoys_required,
    ) == (
        &b.deal,
        &b.resource,
        b.direction,
        &b.partner,
        &b.delivered_raw,
        b.factories,
        b.convoys_assigned,
        b.convoys_required,
    )
}

/// Adds an hour to ascending spans, joining it to the last span when it follows it.
pub(crate) fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        _ => spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// Scans one branch for one country's resources and trades.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedBalance> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ResourceDefinitions,
        TableId::CountryResources,
        TableId::ResourceExchanges,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Resource request cancelled");
    let mut result = ScannedBalance::default();
    let mut learned = Learned::default();
    // The identity and hour last seen, and whether the pools and the deals were
    // recorded for it at that hour. The two tables are captured apart, so each
    // has its own.
    let mut track = IdentityTrack::default();
    let mut pools_open = false;
    let mut deals_open = false;
    let mut open_trades: BTreeMap<u64, usize> = BTreeMap::new();
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
            let identity = decoded
                .complete::<tables::Countries>()
                .ok()
                .zip(decoded.complete::<tables::CountryLinks>().ok())
                .and_then(|(countries, links)| {
                    tagged_country(countries, links, tag)
                        .found()
                        .map(|(country, slot)| ((slot, country.id), links))
                });
            let continues = track.step(identity.map(|(identity, _)| identity), hour);
            // Each table's values go on only from its previous hour of the same
            // identity; where one is missing, the other keeps going.
            let pools_continue = continues && pools_open;
            if !(continues && deals_open) {
                open_trades.clear();
            }
            pools_open = false;
            deals_open = false;
            let Some(((slot, _), links)) = identity else {
                continue;
            };
            // Without names, nothing this hour records can be shown.
            let (Ok(definitions), Ok(lexical)) = (
                decoded.complete::<tables::ResourceDefinitions>(),
                decoded.complete::<tables::LexicalTokens>(),
            ) else {
                continue;
            };
            learned.learn(definitions, lexical);
            result.resources = learned.list();
            if let Ok(rows) = decoded.complete::<tables::CountryResources>() {
                pools_open = true;
                for row in rows
                    .iter()
                    .filter(|row| link_slot(links, row.country) == Some(slot))
                {
                    if !learned.contains(row.resource) {
                        result.unnamed += 1;
                        continue;
                    }
                    let values = PoolRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: false,
                        produced_raw: row.produced.to_string(),
                        subjects_raw: row.adjustment.to_string(),
                        imported_raw: row.imported.to_string(),
                        to_export_raw: row.to_export.to_string(),
                        exported_raw: row.exported.to_string(),
                        available_raw: row.used.to_string(),
                        after_projects_raw: row.special_total.to_string(),
                        production_raw: row.production_demand.to_string(),
                    };
                    let series = result.series.entry(row.resource).or_default();
                    let follows = pools_continue
                        && series
                            .last()
                            .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
                    match series.last_mut() {
                        Some(run) if follows && same_pools(run, &values) => run.last_hour = hour,
                        _ => {
                            series.push(PoolRun {
                                starts_segment: !follows,
                                ..values
                            });
                        }
                    }
                }
            }
            let Ok(exchanges) = decoded.complete::<tables::ResourceExchanges>() else {
                open_trades.clear();
                continue;
            };
            mark(&mut result.deal_hours, hour);
            deals_open = true;
            // Deals this hour still lists continue; the others ended.
            let countries = decoded.complete::<tables::Countries>()?;
            let mut still = BTreeMap::new();
            for exchange in exchanges {
                let (exporter, importer) = (
                    link_slot(links, exchange.exporter),
                    link_slot(links, exchange.importer),
                );
                let (direction, partner) = if exporter == Some(slot) {
                    (TradeDirection::Export, importer)
                } else if importer == Some(slot) {
                    (TradeDirection::Import, exporter)
                } else {
                    continue;
                };
                if !learned.contains(exchange.resource) {
                    result.unnamed += 1;
                    continue;
                }
                let Some(partner) = partner.and_then(|partner| {
                    countries
                        .iter()
                        .find(|country| country.canonical_slot == partner)
                        .and_then(|country| std::str::from_utf8(&country.tag).ok())
                }) else {
                    result.unresolved += 1;
                    continue;
                };
                let terms = TradeRun {
                    deal: exchange.id.to_string(),
                    resource: exchange.resource,
                    direction,
                    partner: partner.to_owned(),
                    delivered_raw: exchange.delivered.to_string(),
                    factories: exchange.assigned_factories,
                    convoys_assigned: exchange.convoys_assigned,
                    convoys_required: exchange.convoys_required,
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment: false,
                };
                // An open deal was listed at the previous hour.
                match open_trades.get(&exchange.id) {
                    Some(&index) if same_terms(&result.trades[index], &terms) => {
                        result.trades[index].last_hour = hour;
                        still.insert(exchange.id, index);
                    }
                    previous => {
                        result.trades.push(TradeRun {
                            starts_segment: previous.is_none(),
                            ..terms
                        });
                        still.insert(exchange.id, result.trades.len() - 1);
                    }
                }
            }
            open_trades = still;
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedBalance>)>,
}

impl Product for ResourceBalance {
    const KEY: &'static str = "resource-balance";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = ResourceBalanceReply;

    fn types() -> Vec<String> {
        vec![
            PoolRun::decl(),
            ResourceSeries::decl(),
            TradeDirection::decl(),
            TradeRun::decl(),
            ResourceInfo::decl(),
            ResourceBalanceReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
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
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        // Without game folders resources keep their tokens and no icons.
        let (resources, mut warnings) =
            resource_infos(source, context, &scanned.resources, cancelled)?;
        if scanned.unnamed > 0 {
            warnings.push(format!(
                "{} recorded rows name a resource the recorded definitions do not; they are left out.",
                scanned.unnamed
            ));
        }
        if scanned.unresolved > 0 {
            warnings.push(format!(
                "{} recorded trade rows name another country the recorded links do not resolve; they are left out.",
                scanned.unresolved
            ));
        }
        if scanned.series.is_empty() {
            warnings.push(format!(
                "{}: no recorded resources in this branch",
                params.tag
            ));
        }
        Ok(ResourceBalanceReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: RESOURCE_DECIMALS,
            series: scanned
                .resources
                .iter()
                .map(|(id, _)| ResourceSeries {
                    id: *id,
                    runs: scanned.series.get(id).cloned().unwrap_or_default(),
                })
                .collect(),
            resources,
            deal_hours: scanned.deal_hours.clone(),
            trades: scanned.trades.clone(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
