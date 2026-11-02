//! Who trades what with whom over a branch, for the Trade flows widget: every
//! classic resource trade deal (`ResourceExchanges`) as a flow from its seller
//! (the exporter) to its buyer (the importer) for one resource, prepared as the
//! `trade` series family ([`facts`], [`emit`]) and answered from its publication
//! ([`query`]).
//!
//! The values are the ones Resource balance already shows per country
//! (`resource_balance.rs`): `delivered` is a deal's daily amount and
//! `assigned_factories` is the civilian factories the buyer pays. The convoys
//! (`convoys_assigned` of `convoys_required`) are shown as recorded.
//!
//! A flow is one (native seller holder, native buyer holder, resource): several
//! deals between the same two countries for the same resource are summed, and `deals` says how many there
//! were. Native holders use (canonical slot, country id); tag renames keep one
//! numeric history, while replacements start another. Per-run aliases and the
//! shared country census date names and flags to the supplying read, including
//! peace. Chosen tags expand to their recorded native holders before filtering.
//!
//! The deal table is read every hour (`replay_data::cadence`). Deals are known only
//! where the table and country catalogue are recorded: `known` lists those hours,
//! so an hour outside them is unknown, never "no trade". An hour listing deals it
//! cannot attribute (a seller or buyer the links do not resolve, a resource the
//! definitions do not name) is known only in part: `partial` lists those hours
//! with how many deals were left out, so they are never shown as the whole trade.
//!
//! `PurchaseContracts` (the international market's equipment contracts, with a
//! seller, a buyer, civilian industry paid and deliveries made) is not read here:
//! it trades equipment, not resources, and the recorded table has no equipment
//! type or quantity to size a flow by (`engine_production::cpurchase_contract`).
//!
//! The reply covers the whole branch so playback is a local lookup. A reply past
//! the product byte budget is refused with a message asking for fewer countries,
//! never cut short.
//!
//! Runs end at a gap, a missing table or a change in a named country's identity
//! (slot, tag and id), as Resource balance's runs do.

use dashboard_base::products::{
    Product,
    country_groups::HourSpan,
    country_history::{CountryIdentityRun, Identity},
};
use dashboard_core::protocol::validate_tag;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};
use ts_rs::TS;

#[cfg(test)]
use dashboard_base::{
    products::resources::{RESOURCE_DECIMALS, ResourceNaming},
    source::Source,
};
#[cfg(test)]
use dashboard_core::config::LoadContext;
#[cfg(test)]
use std::sync::atomic::AtomicBool;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(test)]
mod row;
#[cfg(test)]
pub(crate) mod sample;
pub mod served;

#[cfg(test)]
pub use row::scan;

pub struct TradeFlows;

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
    #[cfg(test)]
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
    /// Exact native holder keys, independent of their dated tag aliases.
    pub seller_identity: String,
    pub buyer_identity: String,
    /// Per-run indexes into `countries`, never a future alias for an older read.
    pub seller_tags: Vec<u32>,
    pub buyer_tags: Vec<u32>,
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeCountryIdentity {
    pub tag: String,
    pub runs: Vec<CountryIdentityRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeFlowsReply {
    pub branch_id: String,
    pub decimals: u8,
    /// Every recorded resource, in the game's order.
    pub resources: Vec<dashboard_base::products::resources::ResourceInfo>,
    /// The tags flows name, sorted.
    pub countries: Vec<String>,
    /// The full dated native holder census, independent of the chosen deals,
    /// including peace and times after their trades stopped.
    pub country_identities: Vec<TradeCountryIdentity>,
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

/// A flow's summed terms at a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct Terms {
    delivered: i64,
    factories: i32,
    assigned: i32,
    required: i32,
    deals: u32,
}

impl Terms {
    fn add(self, row: Self) -> Option<Self> {
        Some(Self {
            delivered: self.delivered.checked_add(row.delivered)?,
            factories: self.factories.checked_add(row.factories)?,
            assigned: self.assigned.checked_add(row.assigned)?,
            required: self.required.checked_add(row.required)?,
            deals: self.deals.checked_add(row.deals)?,
        })
    }

    /// Its terms in the order a flow's integrals add them, with the hour a run counts.
    fn rates(self) -> [i128; 6] {
        [
            i128::from(self.delivered),
            i128::from(self.factories),
            i128::from(self.assigned),
            i128::from(self.required),
            i128::from(self.deals),
            1,
        ]
    }
}

/// A flow: its native seller holder, native buyer holder and resource.
pub(crate) type FlowKey = (Identity, Identity, u32);

#[derive(Clone, Debug)]
struct Run {
    first_hour: i32,
    last_hour: i32,
    terms: Terms,
    seller_tag: String,
    buyer_tag: String,
}

fn identity((slot, id): Identity) -> String {
    format!("[{slot},{id}]")
}
fn flow_token(seller: Identity, buyer: Identity, resource: u32) -> String {
    serde_json::to_string(&(identity(seller), identity(buyer), resource)).expect("flow key")
}

#[derive(Clone, Debug)]
struct Flow {
    seller: String,
    buyer: String,
    seller_identity: Identity,
    buyer_identity: Identity,
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
    pub country_identities: Vec<TradeCountryIdentity>,
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
            .flat_map(|flow| {
                [flow.seller.clone(), flow.buyer.clone()].into_iter().chain(
                    flow.runs
                        .iter()
                        .flat_map(|run| [run.seller_tag.clone(), run.buyer_tag.clone()]),
                )
            })
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
                seller_identity: identity(flow.seller_identity),
                buyer_identity: identity(flow.buyer_identity),
                seller_tags: flow.runs.iter().map(|run| index(&run.seller_tag)).collect(),
                buyer_tags: flow.runs.iter().map(|run| index(&run.buyer_tag)).collect(),
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
        flows.sort_by(|a, b| {
            (
                a.seller,
                a.buyer,
                a.resource,
                &a.seller_identity,
                &a.buyer_identity,
            )
                .cmp(&(
                    b.seller,
                    b.buyer,
                    b.resource,
                    &b.seller_identity,
                    &b.buyer_identity,
                ))
        });
        (countries, flows)
    }
}

impl Product for TradeFlows {
    const KEY: &'static str = "trade-flows";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = TradeFlowParams;
    type Reply = TradeFlowsReply;

    fn types() -> Vec<String> {
        vec![
            TradeFlowParams::decl(),
            TradeFlow::decl(),
            TradeCountryIdentity::decl(),
            CountryIdentityRun::decl(),
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

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        row::load(source, params, context, cancelled)
    }
}

#[cfg(test)]
pub(crate) fn reply(
    source: &Source,
    params: &TradeFlowParams,
    context: &LoadContext,
    scanned: &ScannedFlows,
    cancelled: &AtomicBool,
) -> io::Result<TradeFlowsReply> {
    reply_named(
        &ResourceNaming::stored(source, context),
        params,
        scanned,
        cancelled,
    )
}

/// [`reply`], with the resources named through `naming`.
#[cfg(test)]
pub(crate) fn reply_named(
    naming: &ResourceNaming,
    params: &TradeFlowParams,
    scanned: &ScannedFlows,
    cancelled: &AtomicBool,
) -> io::Result<TradeFlowsReply> {
    // Without the game folders the flows still draw, resources named by token.
    let (resources, mut warnings) = naming.infos(&scanned.resources, cancelled)?;
    recording_warnings(
        scanned.unnamed,
        scanned.unresolved,
        !scanned.reads.is_empty(),
        &mut warnings,
    );
    let (countries, flows) = scanned.reply_flows();
    let reply = TradeFlowsReply {
        branch_id: params.branch_id.clone(),
        decimals: RESOURCE_DECIMALS,
        resources,
        countries,
        country_identities: scanned.country_identities.clone(),
        reads: scanned.reads.clone(),
        known: scanned.known.clone(),
        partial: scanned.partial.clone(),
        flows,
        warnings,
    };
    fits_one_reply(&reply, dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)?;
    Ok(reply)
}

fn recording_warnings(
    unnamed: usize,
    unresolved: usize,
    had_reads: bool,
    warnings: &mut Vec<String>,
) {
    if unnamed > 0 {
        warnings.push(format!(
                "{} recorded deal reads name a resource the recorded definitions do not; they are left out.",
                unnamed
            ));
    }
    if unresolved > 0 {
        warnings.push(format!(
                "{} recorded deal reads name a seller or buyer the recorded links do not resolve; they are left out.",
                unresolved
            ));
    }
    if !had_reads {
        warnings.push("No trade deals were recorded in this branch.".to_owned());
    }
}

/// Refuses a reply longer than `limit` bytes as JSON with a message saying what to
/// do, before the transport would refuse it with none.
pub fn fits_one_reply(reply: &TradeFlowsReply, limit: usize) -> io::Result<()> {
    match dashboard_core::transport::json_len(reply, limit) {
        Some(_) => Ok(()),
        None => Err(io::Error::other(format!(
            "Trade history for these countries takes more than {} MB, more than one reply holds; choose fewer countries in the widget settings.",
            limit / (1024 * 1024)
        ))),
    }
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
