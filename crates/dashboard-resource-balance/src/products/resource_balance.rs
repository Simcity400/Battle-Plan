//! One country's strategic resources over a branch, for the Resource balance
//! widget: where each resource comes from and where it goes, and the trade
//! deals behind its imports and exports.
//!
//! Every value is a recorded pool (`CountryResources`), fixed point with
//! [`RESOURCE_DECIMALS`] places, kept exact. The pools are `produced`,
//! `adjustment`, `imported`, `used`, `special_total`, `production_demand`,
//! `to_export`, `base_export` and `exported`.
//!
//! - `used` is what the country has for use: produced + adjustment + imported
//!   minus `to_export`, with the game's rounding. What is taken off is the export
//!   reserve the trade law sets aside, separate from what buyers took (`exported`).
//! - `production_demand` is minus what the production lines need, met or not.
//! - `special_total` is what remains after the special-project reduction, which
//!   is never positive. Other consumers of that reduction are not identified.
//! - `special_total` + `production_demand` is the surplus, or its deficit
//!   when negative.
//!
//! `adjustment` is resources moved between a subject and its overlord: positive
//! for the overlord, negative for the subject. `base_export` (the reserve before
//! modifiers) is not shown.
//!
//! The family is prepared on the segment driver ([`facts`], [`emit`]) and answered from its
//! series publication ([`query`]).
//!
//! The pools and the deals are both read every hour (`replay_data::cadence`), so a
//! value shown at an hour is as of that hour. Values run over the consecutive
//! hours of the same country identity and end at a gap, an incomplete table or a
//! change of identity. The resource names come from `ResourceDefinitions`, read
//! every hour with the pools: they are content, fixed once loaded.
//!
//! The pools and the deals are captured as separate tables with their own
//! outcomes, so each keeps its own values: an hour whose deal table is missing
//! still shows the pools, and `deal_hours` says at which hours the deals are
//! known.
//!
//! Resources are told apart by their recorded index, not their token: the game
//! keeps every declaration as its own resource (see [`ResourceInfo`]).
//!
//! Trades are `ResourceExchanges` rows (a classic trade deal each) naming the
//! country as exporter or importer. `delivered` is the deal's daily amount.
//! Import and export deals can leave a remainder against the recorded pools,
//! so the widget shows that remainder as its own part.

pub use dashboard_base::products::resources::{RESOURCE_DECIMALS, ResourceInfo, ordinals};
use dashboard_base::products::{CountryParams, Product, resources::stored_resource_infos};
use dashboard_base::{products::country_groups::HourSpan, source::Source};
use dashboard_core::config::LoadContext;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct ResourceBalance;
pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

#[cfg(any(test, feature = "test-support"))]
pub use row::scan;

/// The family the segment driver prepares resource balance as.
pub const NAME: &str = "resources";

#[cfg(any(test, feature = "test-support"))]
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

/// The countries the row oracle scanned last, by branch and tag.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Cache {
    entries: std::collections::VecDeque<((u64, String), std::sync::Arc<ScannedBalance>)>,
}

impl Product for ResourceBalance {
    const KEY: &'static str = "resource-balance";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

    #[cfg(any(test, feature = "test-support"))]
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
                .map(|(_, scanned)| std::sync::Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = std::sync::Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, std::sync::Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        reply(source, params, context, &scanned, cancelled)
    }
}

pub fn reply(
    source: &Source,
    params: &CountryParams,
    context: &LoadContext,
    scanned: &ScannedBalance,
    cancelled: &AtomicBool,
) -> io::Result<ResourceBalanceReply> {
    // Without game folders resources keep their tokens and no icons.
    let (resources, mut warnings) =
        stored_resource_infos(source, context, &scanned.resources, cancelled)?;
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

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
