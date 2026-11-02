//! The international market and lend-lease (family `market`), prepared on the segment driver
//! as a series root ([`facts`], [`emit`]) and answered from it ([`query`]): every country's
//! listed inventory and asking price levels, every request and accepted contract with its
//! terms, the daily purchase contract progress, every lend-lease agreement with its schedules
//! and retained totals, and the market's dated occurrences.
//!
//! State tables are written hourly by the game's market writers; an hour is observed when one
//! of them or the countries changed, or it follows a gap, and each table's completeness is kept
//! as its own run (`Known`), so a table that was not recorded reads as unknown, never as an
//! empty market. A row whose owner has no identity leaves its table's roster unknown; one whose
//! buyer or receiver has none stands without it, and a chosen country may be that party. A
//! request and an accepted contract stay apart (native kind 0 and 1). Equipment pools keep
//! their native order; amounts stay exact (equipment and CIC fixed5, lend-lease fuel quantities
//! /32768, fuel shares fixed5).
//!
//! Every value is one key, its [`Field`] the key's actor's name. Occurrences (`MarketEvents`)
//! are an hourly interval drain: each is kept once, at the read that supplied it, in its kind's
//! occurrence stream, and each party holder keeps a compact dated index per kind (the
//! occurrence's sequence, hour and stream position). A ledger lists only occurrences whose
//! supplying read it reached, and dates and matches parties by the occurrence's own hour. One
//! whose parties have no identity is still kept, without them. Streams and indexes are chains of
//! keys of at most [`CHUNK`] entries, so no read decodes more than a chunk however long the
//! campaign; a ledger merges its segments lazily, newest first by sequence, under a cursor of
//! segment positions ([`reply`]). A holder's missing delivery amount (kind 34) keeps its named
//! refusal and no amount. First observation is not a contract's birth and disappearance is not
//! its completion; only the recorded creation and removal occurrences date those.
//!
//! Equipment names come from the shared variant catalogue at their own reads; only variants
//! the market names are published.

use crate::products::keyed::{EconomyRun, HolderTag};
use dashboard_base::products::{
    Product,
    country_groups::HourSpan,
    equipment::StockVariantName,
};
use replay_data::rows::{MarketEventRow, TableId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
pub(crate) mod reply;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

#[cfg(test)]
mod columnar_tests;

pub const FAMILY: &str = "market";
pub const HISTORY: &str = "market-history";
pub const LEDGER: &str = "market-ledger";

/// The market's state tables, at their positions in [`Field::Known`], and its occurrences.
pub(crate) const STATE: [TableId; 5] = [
    TableId::CountryMarket,
    TableId::MarketContractTerms,
    TableId::LendLeaseAgreements,
    TableId::PurchaseContracts,
    TableId::MarketEvents,
];

/// The availability table of the market's occurrences.
pub(crate) const MARKET_EVENTS: u8 = 4;

/// The ledger's occurrence kinds: request posted, contract made, contract
/// removed, delivery, payment and a delivery whose amount is unavailable.
pub const LEDGER_KINDS: [u8; 6] = [10, 11, 12, 20, 21, 34];

/// Entries a stream or index key holds at most. A read decodes a key's entries in each chunk it
/// touches, so this bounds what one read decodes.
pub const CHUNK: u64 = 512;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// 1 while the table at this position of the state tables is complete.
    Known(u8),
    /// The receiver's holder of a giver's agreement.
    LeaseReceiver(u32, u64),
    LeaseRevision(u32, u64),
    LeaseSince(u32, u64),
    LeaseLast(u32, u64),
    /// Agreement pool `2`'s entry `3` of variant `4`.
    LeasePool(u32, u64, u8, u32, u64),
    /// Fuel: daily, share, sent, lost, last delivery.
    LeaseFuel(u32, u64, u8),
    /// The buyer's holder of a seller's request (kind 0) or contract (1).
    ContractBuyer(u32, u64, u8),
    ContractSpeed(u32, u64, u8),
    /// Equipment (pool 0) or agreed prices (pool 1), entry and variant.
    ContractPool(u32, u64, u8, u8, u32, u64),
    ContractLevel(u32, u64, u8, u64),
    /// A purchase contract's daily progress: seller and buyer holders,
    /// convoys assigned and total, collected CIC, completed deliveries.
    Purchase(u64, u8),
    /// A seller's listed inventory, entry and variant.
    Inventory(u32, u32, u64),
    OfferLevel(u32, u64),
    /// Chunk `1` (first hour `2`) of every occurrence of kind `0`, each once,
    /// in order: `[sequence, hour, row]` entries at their supplying read.
    Stream(u8, u32, i32),
    /// Chunk `2` (first hour `3`) of a party holder's index of its
    /// occurrences of kind `1`: `[sequence, hour, stream position]` entries
    /// at each occurrence's supplying read.
    Party(u32, u8, u32, i32),
    /// 1 while every party of the hour's new occurrences had an identity:
    /// without it a chosen country's empty ledger proves nothing.
    Identified,
    /// 1 while a standing request's or contract's buyer has no identity: the
    /// row stands, and any chosen country may be its buyer.
    ContractUnidentified(u32, u64, u8),
    /// 1 while a giver's agreement's receiver has no identity.
    LeaseUnidentified(u32, u64),
}


/// One occurrence as the ledger keeps it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketLedgerRow {
    pub id: String,
    pub occurrence: String,
    pub hour: i32,
    pub kind: u8,
    pub mode: u8,
    /// The contract's native handle, hexadecimal.
    pub contract: String,
    /// The seller's and buyer's holders; null where the identity is unknown.
    pub seller: Option<String>,
    pub buyer: Option<String>,
    /// The variant, hexadecimal; "0" where none.
    pub variant: String,
    /// The exact amount; null where the recording names it unavailable.
    pub amount_raw: Option<String>,
    pub total_raw: String,
    /// The refusal of an unavailable amount.
    pub reason: Option<String>,
}

/// Whether the ledger keeps the occurrence: the market's own kinds, and a
/// purchase delivery whose amount is unavailable.
pub(crate) fn ledger_kind(row: &MarketEventRow) -> bool {
    match row.kind {
        34 => row.mode == 20,
        kind => LEDGER_KINDS.contains(&kind),
    }
}

pub(crate) fn ledger_row(row: &MarketEventRow, seller: Option<u32>, buyer: Option<u32>) -> MarketLedgerRow {
    let reason = row.delivery_unavailable_reason();
    MarketLedgerRow {
        id: row.id.to_string(),
        occurrence: row.occurrence.to_string(),
        hour: row.hour,
        kind: row.kind,
        mode: row.mode,
        contract: format!("{:016x}", row.agreement),
        seller: seller.map(|slot| slot.to_string()),
        buyer: buyer.map(|slot| slot.to_string()),
        variant: format!("{:x}", row.variant),
        amount_raw: reason.is_none().then(|| row.amount_raw.to_string()),
        total_raw: row.total_raw.to_string(),
        reason: reason.map(str::to_owned),
    }
}


// --- History ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MarketView {
    Contracts,
    Requests,
    Inventory,
    Agreements,
}

pub struct MarketHistory;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketHistoryParams {
    pub branch_id: String,
    pub view: MarketView,
    /// Rows these countries take part in; every row when empty.
    pub tags: Vec<String>,
}

/// One entry of an ordered equipment pool over the branch.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketPoolItem {
    /// The pool's native position within its owner.
    pub pool: u8,
    pub ordinal: u32,
    /// Hexadecimal variant handle.
    pub variant: String,
    pub amounts: Vec<EconomyRun>,
}

/// A request or accepted contract over the branch.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContractHistory {
    pub seller: String,
    pub id: String,
    /// The buyer's holder; no run while it had no identity.
    pub buyer: Vec<EconomyRun>,
    /// Read every hour the row stands: its runs are the row's lifetime.
    pub speed: Vec<EconomyRun>,
    /// Pool 0 equipment, pool 1 agreed prices (CIC).
    pub items: Vec<MarketPoolItem>,
    /// Asking price levels by variant (pool 2).
    pub levels: Vec<MarketPoolItem>,
    /// The daily purchase contract with the same handle and parties:
    /// convoys assigned, convoys total, collected CIC, completed deliveries.
    pub purchase: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OfferHistory {
    pub seller: String,
    pub inventory: Vec<MarketPoolItem>,
    pub levels: Vec<MarketPoolItem>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgreementHistory {
    pub giver: String,
    pub id: String,
    /// The receiver's holder; no run while it had no identity.
    pub receiver: Vec<EconomyRun>,
    /// Read every hour the agreement stands: its runs are its lifetime.
    pub revision: Vec<EconomyRun>,
    pub since: Vec<EconomyRun>,
    pub last: Vec<EconomyRun>,
    /// Pools 0 monthly, 1 production share, 2 once, 3 delivered, 4 lost.
    pub items: Vec<MarketPoolItem>,
    /// Daily, share, sent, lost, last delivery.
    pub fuel: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketHistoryReply {
    pub branch_id: String,
    pub view: MarketView,
    pub last_hour: Option<i32>,
    /// Hours the view's tables were recorded complete (value "1").
    pub known: Vec<EconomyRun>,
    pub contracts: Vec<ContractHistory>,
    pub offers: Vec<OfferHistory>,
    pub agreements: Vec<AgreementHistory>,
    /// Each holder named by a row, with its tag runs.
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    /// Each variant named by a row, with its names at each read.
    pub variants: BTreeMap<String, Vec<StockVariantName>>,
    pub warnings: Vec<String>,
}

impl Product for MarketHistory {
    const KEY: &'static str = HISTORY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = MarketHistoryParams;
    type Reply = MarketHistoryReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            dashboard_base::products::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            MarketView::decl(),
            MarketHistoryParams::decl(),
            MarketHistoryReply::decl(),
            MarketPoolItem::decl(),
            ContractHistory::decl(),
            OfferHistory::decl(),
            AgreementHistory::decl(),
        ]
    }
}

// --- Ledger -----------------------------------------------------------------

pub struct MarketLedger;


#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLedgerParams {
    pub branch_id: String,
    /// Occurrences these countries sell or buy in, at any hour; every one when
    /// both this and `members` are empty.
    pub tags: Vec<String>,
    /// Occurrences a group's members sell or buy in while members: each tag
    /// over its own hours.
    #[serde(default)]
    #[ts(optional)]
    pub members: Option<Vec<LedgerMember>>,
    /// The kinds shown; [`LEDGER_KINDS`] by default.
    pub kinds: Vec<u8>,
    /// The first hour shown; from the branch's start when absent.
    #[serde(default)]
    #[ts(optional)]
    pub first_hour: Option<i32>,
    /// The last hour shown.
    pub last_hour: i32,
    /// The cursor of the next older page, from the reply's `next`; the
    /// newest page when absent.
    #[serde(default)]
    #[ts(optional)]
    pub before: Option<String>,
    pub page_size: u32,
}

/// A tag over the hours it counts, inclusive.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LedgerMember {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketLedgerReply {
    pub branch_id: String,
    /// Through the last prepared hour.
    pub last_hour: Option<i32>,
    /// Newest first.
    pub rows: Vec<MarketLedgerRow>,
    /// Occurrences selected through the last hour, where counted exactly
    /// (every country, from the streams' lengths); null for chosen countries,
    /// whose pages continue by `next` alone.
    pub total: Option<String>,
    /// The `before` of the next older page; null on the oldest page.
    pub next: Option<String>,
    /// Hours the market's occurrences were read, as "1" runs: an empty
    /// ledger is only "no trades" inside them. For chosen countries, only
    /// hours whose occurrences' parties all had identities.
    pub known: Vec<EconomyRun>,
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    /// The hours across this page's rows whose country identities were read: a
    /// party's tag at a row's hour is known only inside them.
    pub identified: Vec<HourSpan>,
    pub variants: BTreeMap<String, Vec<StockVariantName>>,
    pub warnings: Vec<String>,
}

impl Product for MarketLedger {
    const KEY: &'static str = LEDGER;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = MarketLedgerParams;
    type Reply = MarketLedgerReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for tag in params.tags.iter().chain(params.members.iter().flatten().map(|member| &member.tag)) {
            dashboard_base::products::validate_tag(tag)?;
        }
        if params.members.iter().flatten().any(|member| member.first_hour > member.last_hour) {
            return Err("Invalid market ledger member hours");
        }
        if params.first_hour.is_some_and(|first| first > params.last_hour) {
            return Err("Invalid market ledger hours");
        }
        if params.kinds.iter().any(|kind| !LEDGER_KINDS.contains(kind)) {
            return Err("Invalid market occurrence kind");
        }
        if params.page_size == 0 || params.page_size > 4096 {
            return Err("Invalid market ledger page size");
        }
        if params.before.as_deref().is_some_and(|before| reply::parse_cursor(before).is_err()) {
            return Err("Invalid market ledger cursor");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            MarketLedgerParams::decl(),
            LedgerMember::decl(),
            MarketLedgerReply::decl(),
            MarketLedgerRow::decl(),
        ]
    }
}

