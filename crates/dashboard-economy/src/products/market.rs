//! The international market and lend-lease, prepared on their own (family
//! `market`): every country's listed inventory and asking price levels,
//! every request and accepted contract with its terms, the daily purchase
//! contract progress, every lend-lease agreement with its schedules and
//! retained totals, and the market's dated occurrences.
//!
//! State tables are written hourly by the game's market writers; an hour is
//! observed when one of them changed or it follows a gap, and each table's
//! completeness is kept as its own run (`Known`), so a table that was not
//! recorded reads as unknown, never as an empty market. A request and an
//! accepted contract stay apart (native kind 0 and 1). Equipment pools keep
//! their native order; amounts stay exact (equipment and CIC fixed5,
//! lend-lease fuel quantities /32768, fuel shares fixed5).
//!
//! Occurrences (`MarketEvents`) are an hourly interval drain: each is kept
//! once, at its hour, in its kind's occurrence stream, and each party holder
//! keeps a compact dated index per kind (the occurrence's sequence and stream
//! position). Streams and indexes are chains of keys of at most [`CHUNK`]
//! entries: the shared series decodes a key's whole blob in each batch a read
//! touches, so no read decodes more than a chunk however long the campaign. A
//! ledger merges its segments (a chain over the hours it applies) lazily,
//! newest first by sequence, under a cursor of segment positions: a page
//! decodes the chunks holding its own rows and index entries plus one head per
//! segment, and an occurrence two chosen parties share is taken once. Keys grow
//! with kinds, holders and volume, never with trading pairs. A holder's
//! missing delivery amount (kind 34) keeps its named refusal and no amount.
//! First observation is not a contract's birth and disappearance is not its
//! completion; only the recorded creation and removal occurrences date those.
//!
//! Equipment names come from the shared variant catalogue at their own
//! reads; only variants the market names are published.

use super::keyed::{
    EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of, clip_text, selected_spans,
};
use dashboard_base::{
    invalid,
    prepared::frame_changes::FrameChanges,
    products::{
        Product,
        equipment::{StockVariantName, Texts, VariantCatalogue, VariantVersion},
    },
};
use replay_data::{
    format::FormatVersion,
    rows::{MarketEventRow, MarketPool, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{LazyLock, atomic::AtomicBool},
};
use ts_rs::TS;

pub const FAMILY: &str = "market";
pub const HISTORY: &str = "market-history";
pub const LEDGER: &str = "market-ledger";

const STATE: [TableId; 5] = [
    TableId::CountryMarket,
    TableId::MarketContractTerms,
    TableId::LendLeaseAgreements,
    TableId::PurchaseContracts,
    TableId::MarketEvents,
];

/// The tables the observer reads: the market's, the identities naming its
/// countries and the variant catalogue naming its equipment.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![TableId::Campaign, TableId::Countries, TableId::CountryLinks];
        tables.extend(STATE);
        tables.extend(VariantCatalogue::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// The ledger's occurrence kinds: request posted, contract made, contract
/// removed, delivery, payment and a delivery whose amount is unavailable.
pub const LEDGER_KINDS: [u8; 6] = [10, 11, 12, 20, 21, 34];

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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
    /// in order: `[sequence, row]` entries.
    Stream(u8, u32, i32),
    /// Chunk `2` (first hour `3`) of a party holder's index of its
    /// occurrences of kind `1`: `[sequence, stream position]` entries at each
    /// occurrence's hour.
    Party(u32, u8, u32, i32),
}

/// Entries a stream or index key holds at most. A read decodes a key's whole
/// blob in each batch it touches, so this bounds what one read decodes.
pub const CHUNK: u64 = 512;

/// A chain's length and its open chunk's first hour.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct ChainState {
    count: u64,
    first: i32,
}

/// Appends `entry` to a chain at `at`, opening its next key every [`CHUNK`]
/// entries; the entry's position in the chain.
fn append(
    keyed: &mut Keyed<Field, Extra>,
    chain: &mut ChainState,
    field: impl Fn(u32, i32) -> Field,
    at: i32,
    entry: &impl Serialize,
) -> io::Result<u64> {
    let first = if chain.count.is_multiple_of(CHUNK) { at } else { chain.first };
    let chunk = u32::try_from(chain.count / CHUNK)
        .map_err(|_| invalid("a market occurrence chain outgrew its keys"))?;
    let key = keyed.key(field(chunk, first));
    keyed.values.event(key, at, entry)?;
    chain.first = first;
    chain.count += 1;
    Ok(chain.count - 1)
}

/// The variants the market names, with their naming reads.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Extra {
    pub variants: BTreeMap<u64, Vec<VariantVersion>>,
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

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, Extra>,
    variants: VariantCatalogue,
    /// Variants the market named, published with their naming reads.
    named: BTreeSet<u64>,
    /// The previous hour's occurrence ids, so one carried twice is kept once.
    last_events: BTreeSet<u64>,
    /// Each kind's stream.
    streams: BTreeMap<u8, ChainState>,
    /// Each party holder's index of each kind.
    indexes: BTreeMap<u32, BTreeMap<u8, ChainState>>,
    /// Occurrences kept so far, across kinds: the next one's sequence, which
    /// orders every stream and index alike.
    sequence: u64,
}

impl Prepared {
    pub fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        super::keyed::observe_variants(&mut self.variants, format, frame, Some(changes));
        let watched = [TableId::Countries, TableId::CountryLinks]
            .into_iter()
            .chain(STATE)
            .any(|table| changes.rows_changed[table.index()]);
        if !gap && !watched {
            return Ok(());
        }
        self.observe(frame);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        super::keyed::observe_variants(&mut self.variants, FormatVersion::CURRENT, frame, None);
        self.observe(frame);
    }

    fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        let tags = match Tags::new(frame) {
            Ok(tags) => tags,
            Err(error) => {
                self.keyed
                    .refuse(format!("country identities unavailable: {error}"));
                self.keyed.commit(hour, &observed, |_| true);
                return;
            }
        };
        let slot = |keyed: &mut Keyed<Field, Extra>, raw: u32| {
            let slot = tags.slot(raw);
            match slot {
                Some(slot) => keyed.tag(&tags, slot, hour),
                None => keyed.refuse("a market country has no identity"),
            }
            slot
        };
        let mut named = Vec::new();
        let keyed = &mut self.keyed;
        let known = |keyed: &mut Keyed<Field, Extra>, observed: &mut Observed, table: u8| {
            observed.insert(keyed.key(Field::Known(table)), 1);
        };
        match frame.complete::<tables::CountryMarket>() {
            Ok(rows) => {
                known(keyed, &mut observed, 0);
                for row in rows {
                    let Some(seller) = slot(keyed, row.country) else { continue };
                    pool(keyed, &mut observed, &mut named, &row.inventory, &|ordinal, variant| {
                        Field::Inventory(seller, ordinal, variant)
                    });
                    for level in row.prices.iter() {
                        named.push(level.variant);
                        observed.insert(
                            keyed.key(Field::OfferLevel(seller, level.variant)),
                            i128::from(level.level),
                        );
                    }
                }
            }
            Err(error) => keyed.refuse(format!("market inventory unavailable: {error}")),
        }
        match frame.complete::<tables::MarketContractTerms>() {
            Ok(rows) => {
                known(keyed, &mut observed, 1);
                for row in rows {
                    let Some(seller) = slot(keyed, row.country) else { continue };
                    let (id, kind) = (row.id, row.kind);
                    if let Some(buyer) = slot(keyed, row.buyer) {
                        observed.insert(keyed.key(Field::ContractBuyer(seller, id, kind)), i128::from(buyer));
                    }
                    observed.insert(keyed.key(Field::ContractSpeed(seller, id, kind)), i128::from(row.speed));
                    for (which, items) in [(0, &row.equipment), (1, &row.prices)] {
                        pool(keyed, &mut observed, &mut named, items, &|ordinal, variant| {
                            Field::ContractPool(seller, id, kind, which, ordinal, variant)
                        });
                    }
                    for level in row.price_levels.iter() {
                        named.push(level.variant);
                        observed.insert(
                            keyed.key(Field::ContractLevel(seller, id, kind, level.variant)),
                            i128::from(level.level),
                        );
                    }
                }
            }
            Err(error) => keyed.refuse(format!("market contract terms unavailable: {error}")),
        }
        match frame.complete::<tables::LendLeaseAgreements>() {
            Ok(rows) => {
                known(keyed, &mut observed, 2);
                for row in rows {
                    let Some(giver) = slot(keyed, row.country) else { continue };
                    let id = row.id;
                    if let Some(receiver) = slot(keyed, row.receiver) {
                        observed.insert(keyed.key(Field::LeaseReceiver(giver, id)), i128::from(receiver));
                    }
                    for (field, value) in [
                        (Field::LeaseRevision(giver, id), i128::from(row.revision)),
                        (Field::LeaseSince(giver, id), i128::from(row.active_since)),
                        (Field::LeaseLast(giver, id), i128::from(row.last_delivery)),
                        (Field::LeaseFuel(giver, id, 0), i128::from(row.fuel_daily_raw)),
                        (Field::LeaseFuel(giver, id, 1), i128::from(row.fuel_percentage_raw)),
                        (Field::LeaseFuel(giver, id, 2), i128::from(row.fuel_sent_raw)),
                        (Field::LeaseFuel(giver, id, 3), i128::from(row.fuel_sunk_raw)),
                        (Field::LeaseFuel(giver, id, 4), i128::from(row.last_fuel_delivered_raw)),
                    ] {
                        observed.insert(keyed.key(field), value);
                    }
                    for (which, items) in
                        [&row.monthly, &row.percentage, &row.once, &row.delivered, &row.sunk]
                            .into_iter()
                            .enumerate()
                    {
                        pool(keyed, &mut observed, &mut named, items, &|ordinal, variant| {
                            Field::LeasePool(giver, id, which as u8, ordinal, variant)
                        });
                    }
                }
            }
            Err(error) => keyed.refuse(format!("lend-lease agreements unavailable: {error}")),
        }
        match frame.complete::<tables::PurchaseContracts>() {
            Ok(rows) => {
                known(keyed, &mut observed, 3);
                for row in rows {
                    let (seller, buyer) = (slot(keyed, row.seller), slot(keyed, row.buyer));
                    for (which, value) in [
                        (0, seller.map(i128::from)),
                        (1, buyer.map(i128::from)),
                        (2, Some(i128::from(row.convoys_assigned))),
                        (3, Some(i128::from(row.convoys_total))),
                        (4, Some(i128::from(row.collected_raw))),
                        (5, Some(i128::from(row.completed_deliveries))),
                    ] {
                        if let Some(value) = value {
                            observed.insert(keyed.key(Field::Purchase(row.id, which)), value);
                        }
                    }
                }
            }
            Err(error) => keyed.refuse(format!("purchase contracts unavailable: {error}")),
        }
        let mut seen = BTreeSet::new();
        match frame.complete::<tables::MarketEvents>() {
            Ok(rows) => {
                known(keyed, &mut observed, MARKET_EVENTS);
                for row in rows {
                    seen.insert(row.id);
                    if self.last_events.contains(&row.id) || !ledger_kind(row) {
                        continue;
                    }
                    let (seller, buyer) = (slot(keyed, row.country), slot(keyed, row.receiver));
                    if row.variant != 0 {
                        named.push(row.variant);
                    }
                    let entry = ledger_row(row, seller, buyer);
                    let at = row.hour.max(hour);
                    let (kind, sequence) = (row.kind, self.sequence);
                    let stream = self.streams.entry(kind).or_default();
                    let position = match append(
                        keyed,
                        stream,
                        |chunk, first| Field::Stream(kind, chunk, first),
                        at,
                        &(sequence, &entry),
                    ) {
                        Ok(position) => position,
                        Err(error) => {
                            keyed.refuse(format!("a market occurrence could not be kept: {error}"));
                            continue;
                        }
                    };
                    self.sequence += 1;
                    let parties = [seller, buyer.filter(|buyer| Some(*buyer) != seller)];
                    for holder in parties.into_iter().flatten() {
                        let index = self.indexes.entry(holder).or_default().entry(kind).or_default();
                        if let Err(error) = append(
                            keyed,
                            index,
                            |chunk, first| Field::Party(holder, kind, chunk, first),
                            at,
                            &(sequence, position),
                        ) {
                            keyed.refuse(format!("a market occurrence's party index could not be kept: {error}"));
                        }
                    }
                }
            }
            Err(error) => keyed.refuse(format!("market occurrences unavailable: {error}")),
        }
        self.last_events = seen;
        self.named.extend(named.into_iter().filter(|&variant| variant != 0));
        // Occurrence keys hold no state; state keys end when not shown.
        self.keyed.commit(hour, &observed, |field| {
            !matches!(field, Field::Stream(..) | Field::Party(..))
        });
    }

    pub fn bytes(&self) -> usize {
        self.keyed.bytes()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.meta.extra.variants = self
            .named
            .iter()
            .filter_map(|&id| Some((id, self.variants.versions.get(&id)?.clone())))
            .collect();
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

/// Each entry of an ordered pool under the key `field` names.
fn pool(
    keyed: &mut Keyed<Field, Extra>,
    observed: &mut Observed,
    named: &mut Vec<u64>,
    pool: &MarketPool,
    field: &dyn Fn(u32, u64) -> Field,
) {
    for (ordinal, item) in pool.items.iter().enumerate() {
        named.push(item.variant);
        observed.insert(
            keyed.key(field(ordinal as u32, item.variant)),
            i128::from(item.amount_raw),
        );
    }
}

/// Whether the ledger keeps the occurrence: the market's own kinds, and a
/// purchase delivery whose amount is unavailable.
fn ledger_kind(row: &MarketEventRow) -> bool {
    match row.kind {
        34 => row.mode == 20,
        kind => LEDGER_KINDS.contains(&kind),
    }
}

fn ledger_row(row: &MarketEventRow, seller: Option<u32>, buyer: Option<u32>) -> MarketLedgerRow {
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
    /// The buyer's holder.
    pub buyer: Vec<EconomyRun>,
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
    pub receiver: Vec<EconomyRun>,
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

type Gathered = BTreeMap<(u32, u64, u8), Vec<(u64, Field)>>;

pub fn history(
    store: &store::Store,
    key: &store::SourceKey,
    params: &MarketHistoryParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketHistoryReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = MarketHistoryReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        last_hour: through,
        known: Vec::new(),
        contracts: Vec::new(),
        offers: Vec::new(),
        agreements: Vec::new(),
        holders: BTreeMap::new(),
        variants: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, Extra>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let chosen: Option<BTreeMap<u32, Vec<(i32, i32)>>> =
        (!params.tags.is_empty()).then(|| published.holder_spans(&params.tags));
    let all = [(i32::MIN, i32::MAX)];
    let table = match params.view {
        MarketView::Contracts | MarketView::Requests => 1,
        MarketView::Inventory => 0,
        MarketView::Agreements => 2,
    };
    // Group every key under its owner (holder, handle, kind).
    let mut owners = Gathered::new();
    let mut purchases = BTreeMap::<u64, Vec<(u8, u64)>>::new();
    let mut known = None;
    for (&id, field) in &published.meta.fields {
        let owner = match (*field, params.view) {
            (Field::Known(held), _) if held == table => {
                known = Some(id);
                continue;
            }
            (Field::Purchase(handle, which), MarketView::Contracts) => {
                purchases.entry(handle).or_default().push((which, id));
                continue;
            }
            (
                Field::ContractBuyer(seller, handle, kind)
                | Field::ContractSpeed(seller, handle, kind)
                | Field::ContractPool(seller, handle, kind, ..)
                | Field::ContractLevel(seller, handle, kind, _),
                MarketView::Contracts | MarketView::Requests,
            ) if (kind == 1) == (params.view == MarketView::Contracts) => (seller, handle, kind),
            (Field::Inventory(seller, ..) | Field::OfferLevel(seller, _), MarketView::Inventory) => {
                (seller, 0, 0)
            }
            (
                Field::LeaseReceiver(giver, handle)
                | Field::LeaseRevision(giver, handle)
                | Field::LeaseSince(giver, handle)
                | Field::LeaseLast(giver, handle)
                | Field::LeasePool(giver, handle, ..)
                | Field::LeaseFuel(giver, handle, _),
                MarketView::Agreements,
            ) => (giver, handle, 0),
            _ => continue,
        };
        owners.entry(owner).or_default().push((id, *field));
    }
    if let Some(id) = known {
        reply.known = published.text(id, &all)?;
    }
    let mut named = BTreeSet::new();
    let mut holders = BTreeSet::new();
    let item = |published: &mut Published<'_, Field, Extra>,
                    named: &mut BTreeSet<u64>,
                    spans: &[(i32, i32)],
                    id: u64,
                    pool: u8,
                    ordinal: u32,
                    variant: u64|
     -> io::Result<MarketPoolItem> {
        named.insert(variant);
        Ok(MarketPoolItem {
            pool,
            ordinal,
            variant: format!("{variant:x}"),
            amounts: published.text(id, spans)?,
        })
    };
    let party = |runs: &[EconomyRun]| -> Vec<u32> {
        runs.iter().filter_map(|run| run.2.parse().ok()).collect()
    };
    // The other party's runs, read first so a row no chosen country is party
    // to is skipped before its pools are read.
    let counterpart = |published: &mut Published<'_, Field, Extra>,
                       fields: &[(u64, Field)],
                       party: fn(&Field) -> bool|
     -> io::Result<Vec<EconomyRun>> {
        match fields.iter().find(|(_, field)| party(field)) {
            Some(&(id, _)) => published.text(id, &all),
            None => Ok(Vec::new()),
        }
    };
    for ((owner, handle, _kind), fields) in owners {
        published.check()?;
        match params.view {
            MarketView::Contracts | MarketView::Requests => {
                let buyer = counterpart(&mut published, &fields, |field| matches!(field, Field::ContractBuyer(..)))?;
                let spans = selected_spans(chosen.as_ref(), &[owner], &buyer);
                if spans.is_empty() {
                    continue;
                }
                let buyer = clip_text(&buyer, &spans);
                let buyers = party(&buyer);
                let mut buyer_runs = Vec::new();
                let mut contract = ContractHistory {
                    seller: owner.to_string(),
                    id: format!("{handle:016x}"),
                    buyer,
                    speed: Vec::new(),
                    items: Vec::new(),
                    levels: Vec::new(),
                    purchase: Vec::new(),
                };
                for (id, field) in fields {
                    match field {
                        Field::ContractBuyer(..) => buyer_runs = published.runs(id, &spans)?,
                        Field::ContractSpeed(..) => contract.speed = published.text(id, &spans)?,
                        Field::ContractPool(_, _, _, pool, ordinal, variant) => contract.items.push(
                            item(&mut published, &mut named, &spans, id, pool, ordinal, variant)?,
                        ),
                        Field::ContractLevel(_, _, _, variant) => contract
                            .levels
                            .push(item(&mut published, &mut named, &spans, id, 2, 0, variant)?),
                        _ => {}
                    }
                }
                if let Some(keys) = purchases.get(&handle) {
                    contract.purchase = join_purchase(&mut published, keys, owner, &buyer_runs)?;
                }
                holders.insert(owner);
                holders.extend(buyers);
                reply.contracts.push(contract);
            }
            MarketView::Inventory => {
                let spans = selected_spans(chosen.as_ref(), &[owner], &[]);
                if spans.is_empty() {
                    continue;
                }
                let mut offer = OfferHistory {
                    seller: owner.to_string(),
                    inventory: Vec::new(),
                    levels: Vec::new(),
                };
                for (id, field) in fields {
                    match field {
                        Field::Inventory(_, ordinal, variant) => offer
                            .inventory
                            .push(item(&mut published, &mut named, &spans, id, 0, ordinal, variant)?),
                        Field::OfferLevel(_, variant) => offer
                            .levels
                            .push(item(&mut published, &mut named, &spans, id, 2, 0, variant)?),
                        _ => {}
                    }
                }
                holders.insert(owner);
                reply.offers.push(offer);
            }
            MarketView::Agreements => {
                let receiver = counterpart(&mut published, &fields, |field| matches!(field, Field::LeaseReceiver(..)))?;
                let spans = selected_spans(chosen.as_ref(), &[owner], &receiver);
                if spans.is_empty() {
                    continue;
                }
                let receiver = clip_text(&receiver, &spans);
                let receivers = party(&receiver);
                let mut agreement = AgreementHistory {
                    giver: owner.to_string(),
                    id: format!("{handle:016x}"),
                    receiver,
                    revision: Vec::new(),
                    since: Vec::new(),
                    last: Vec::new(),
                    items: Vec::new(),
                    fuel: vec![Vec::new(); 5],
                };
                for (id, field) in fields {
                    match field {
                        Field::LeaseRevision(..) => agreement.revision = published.text(id, &spans)?,
                        Field::LeaseSince(..) => agreement.since = published.text(id, &spans)?,
                        Field::LeaseLast(..) => agreement.last = published.text(id, &spans)?,
                        Field::LeaseFuel(_, _, which) if which < 5 => {
                            agreement.fuel[usize::from(which)] = published.text(id, &spans)?;
                        }
                        Field::LeasePool(_, _, pool, ordinal, variant) => agreement.items.push(
                            item(&mut published, &mut named, &spans, id, pool, ordinal, variant)?,
                        ),
                        _ => {}
                    }
                }
                holders.insert(owner);
                holders.extend(receivers);
                reply.agreements.push(agreement);
            }
        }
    }
    for list in reply
        .contracts
        .iter_mut()
        .flat_map(|row| [&mut row.items, &mut row.levels])
        .chain(reply.offers.iter_mut().flat_map(|row| [&mut row.inventory, &mut row.levels]))
        .chain(reply.agreements.iter_mut().map(|row| &mut row.items))
    {
        list.sort_by_key(|item| (item.pool, item.ordinal));
    }
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    reply.variants = variant_names(&published.meta.extra, &named, texts, &mut warnings);
    warnings.extend(published.warnings("Market"));
    reply.warnings = warnings;
    Ok(reply)
}

/// The purchase contract's runs while its parties match the contract's.
fn join_purchase(
    published: &mut Published<'_, Field, Extra>,
    keys: &[(u8, u64)],
    seller: u32,
    buyers: &[dashboard_base::prepared::series::Run],
) -> io::Result<Vec<Vec<EconomyRun>>> {
    let all = [(i32::MIN, i32::MAX)];
    let mut parties = [Vec::new(), Vec::new()];
    let mut values = vec![Vec::new(); 4];
    for &(which, id) in keys {
        match which {
            0 | 1 => parties[usize::from(which)] = published.runs(id, &all)?,
            2..=5 => values[usize::from(which - 2)] = published.text(id, &all)?,
            _ => {}
        }
    }
    // Hours the purchase records the contract's seller and the buyer the
    // contract itself records at that same hour: a buyer from another stretch
    // of the contract's history never joins.
    let matching: Vec<(i32, i32)> = parties[0]
        .iter()
        .filter(|run| run.value == i128::from(seller))
        .flat_map(|sold| {
            parties[1].iter().flat_map(move |bought| {
                buyers.iter().filter_map(move |held| {
                    let first = sold.first_hour.max(bought.first_hour).max(held.first_hour);
                    let last = sold.last_hour.min(bought.last_hour).min(held.last_hour);
                    (first <= last && bought.value == held.value).then_some((first, last))
                })
            })
        })
        .collect();
    Ok(values
        .into_iter()
        .map(|runs| {
            runs.into_iter()
                .flat_map(|EconomyRun(first, last, value)| {
                    matching.iter().filter_map(move |&(from, to)| {
                        let (first, last) = (first.max(from), last.min(to));
                        (first <= last).then(|| EconomyRun(first, last, value.clone()))
                    })
                })
                .collect()
        })
        .collect())
}

fn variant_names(
    extra: &Extra,
    named: &BTreeSet<u64>,
    texts: Option<&dyn Texts>,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, Vec<StockVariantName>> {
    let catalogue = VariantCatalogue::seeded(
        extra
            .variants
            .iter()
            .filter(|(id, _)| named.contains(id))
            .map(|(&id, versions)| (id, versions.clone()))
            .collect(),
    );
    let mut missing = 0_usize;
    let names = named
        .iter()
        .map(|&id| {
            let names = catalogue.names(id, texts);
            if names.iter().all(|name| name.name.is_none()) {
                missing += 1;
            }
            (format!("{id:x}"), names)
        })
        .collect();
    if missing > 0 {
        warnings.push(format!("Equipment names unavailable for {missing} market variants"));
    }
    names
}

// --- Ledger -----------------------------------------------------------------

pub struct MarketLedger;

/// The availability table of the market's occurrences.
const MARKET_EVENTS: u8 = 4;

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
    /// ledger is only "no trades" inside them.
    pub known: Vec<EconomyRun>,
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    pub variants: BTreeMap<String, Vec<StockVariantName>>,
    pub warnings: Vec<String>,
}

impl Product for MarketLedger {
    const KEY: &'static str = LEDGER;
    const CONTENT_DEPENDENT: bool = true;
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
        if params.kinds.iter().any(|kind| !LEDGER_KINDS.contains(kind)) {
            return Err("Invalid market occurrence kind");
        }
        if params.page_size == 0 || params.page_size > 4096 {
            return Err("Invalid market ledger page size");
        }
        if params.before.as_deref().is_some_and(|before| parse_cursor(before).is_err()) {
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

pub fn ledger(
    store: &store::Store,
    key: &store::SourceKey,
    params: &MarketLedgerParams,
    through: Option<i32>,
    texts: Option<&dyn Texts>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<MarketLedgerReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = MarketLedgerReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        rows: Vec::new(),
        total: None,
        next: None,
        known: Vec::new(),
        holders: BTreeMap::new(),
        variants: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, Extra>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let kinds: BTreeSet<u8> = if params.kinds.is_empty() {
        LEDGER_KINDS.into_iter().collect()
    } else {
        params.kinds.iter().copied().collect()
    };
    // Each chosen country's holders with the hours they had one of its tags
    // (and, for a group's member, were a member): an occurrence matches by a
    // party's tag at its own hour.
    let spans = (!params.tags.is_empty() || params.members.as_ref().is_some_and(|members| !members.is_empty())).then(|| chosen_spans(&published, params));
    // Each chain's keys: a kind's stream, and each holder's index of a kind.
    let mut streams = BTreeMap::<u8, Vec<(u32, i32, u64)>>::new();
    let mut parties = BTreeMap::<u32, BTreeMap<u8, Vec<(u32, i32, u64)>>>::new();
    let mut known = None;
    for (&id, field) in &published.meta.fields {
        match *field {
            Field::Known(MARKET_EVENTS) => known = Some(id),
            Field::Stream(kind, chunk, first) if kinds.contains(&kind) => {
                streams.entry(kind).or_default().push((chunk, first, id));
            }
            Field::Party(holder, kind, chunk, first) if kinds.contains(&kind) => {
                parties.entry(holder).or_default().entry(kind).or_default().push((chunk, first, id));
            }
            _ => {}
        }
    }
    if let Some(id) = known {
        reply.known = published.text(id, &[(i32::MIN, i32::MAX)])?;
    }
    let to = params.last_hour.min(through);
    if streams.is_empty() {
        reply.total = spans.is_none().then(|| "0".to_owned());
        reply.warnings = warnings;
        return Ok(reply);
    }
    let mut chains = Vec::new();
    let mut stream_of = BTreeMap::new();
    for (kind, chunks) in streams {
        stream_of.insert(kind, chains.len());
        chains.push(Chain::new(chunks, false)?);
    }
    // Every occurrence once: each kind's stream through the hour. Chosen
    // countries: each chosen holder's index of each kind over each span it had
    // a chosen tag.
    let mut segments = Vec::new();
    match &spans {
        None => segments.extend(stream_of.iter().map(|(&kind, &chain)| (chain, kind, i32::MIN, to))),
        Some(spans) => {
            for (holder, spans) in spans {
                for (&kind, chunks) in parties.get(holder).into_iter().flatten() {
                    let chain = chains.len();
                    chains.push(Chain::new(chunks.clone(), true)?);
                    for &(from, until) in spans {
                        let until = until.min(to);
                        if from <= until {
                            segments.push((chain, kind, from, until));
                        }
                    }
                }
            }
        }
    }
    let mut ledger = Ledger::open(&mut published, chains, segments, params.before.as_deref())?;
    let mut taken = Vec::new();
    while taken.len() < params.page_size as usize {
        published.check()?;
        match ledger.next(&mut published)? {
            Some(entry) => taken.push(entry),
            None => break,
        }
    }
    reply.next = ledger.cursor();
    if spans.is_none() {
        reply.total = Some(ledger.total().to_string());
    }
    let mut rows: Vec<MarketLedgerRow> = Vec::with_capacity(taken.len());
    for (kind, payload) in taken {
        let row = if spans.is_none() {
            payload
        } else {
            let position = serde_json::from_value::<u64>(payload)?;
            let stream = stream_of
                .get(&kind)
                .ok_or_else(|| invalid("A market occurrence index names no stream"))?;
            ledger.chains[*stream].entry(&mut published, position)?.2
        };
        rows.push(serde_json::from_value(row)?);
    }
    let mut named = BTreeSet::new();
    let mut holders = BTreeSet::new();
    for row in &rows {
        if let Ok(variant) = u64::from_str_radix(&row.variant, 16)
            && variant != 0
        {
            named.insert(variant);
        }
        holders.extend(
            [&row.seller, &row.buyer]
                .into_iter()
                .flatten()
                .filter_map(|holder| holder.parse::<u32>().ok()),
        );
    }
    reply.rows = rows;
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    reply.variants = variant_names(&published.meta.extra, &named, texts, &mut warnings);
    warnings.extend(published.warnings("Market"));
    reply.warnings = warnings;
    Ok(reply)
}

// Stream entries and party index entries read, counted in tests to bound a
// page's work; what those reads decode is
// `dashboard_base::prepared::series::decoded_event_rows`.
#[cfg(test)]
thread_local! {
    pub(crate) static LEDGER_ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LEDGER_INDEX_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A stream or party index: its keys in chunk order, each with its first hour.
struct Chain {
    chunks: Vec<(i32, u64)>,
    /// A party index rather than a stream, for the test counters.
    #[cfg_attr(not(test), allow(dead_code))]
    index: bool,
}

impl Chain {
    fn new(mut chunks: Vec<(u32, i32, u64)>, index: bool) -> io::Result<Self> {
        chunks.sort_unstable();
        if chunks.iter().enumerate().any(|(at, &(chunk, ..))| chunk as usize != at) {
            return Err(invalid("A market occurrence chain misses a key"));
        }
        Ok(Self {
            chunks: chunks.into_iter().map(|(_, first, key)| (first, key)).collect(),
            index,
        })
    }

    /// The chain's name in a cursor: its first key.
    fn name(&self) -> u64 {
        self.chunks[0].1
    }

    /// The position after the chain's last entry through `until`: its count
    /// there, read from the one key whose chunk holds that hour.
    fn end(&self, published: &mut Published<'_, Field, Extra>, until: i32) -> io::Result<u64> {
        let Some(last) = self.chunks.partition_point(|&(first, _)| first <= until).checked_sub(1) else {
            return Ok(0);
        };
        let held = published.reader.events(&[self.chunks[last].1], i32::MIN, until, 0, 1)?.total;
        self.read();
        Ok(last as u64 * CHUNK + held)
    }

    /// The entry at `position`: its hour, sequence and payload.
    fn entry(&self, published: &mut Published<'_, Field, Extra>, position: u64) -> io::Result<(i32, u64, Value)> {
        let key = usize::try_from(position / CHUNK)
            .ok()
            .and_then(|chunk| self.chunks.get(chunk))
            .map(|&(_, key)| key)
            .ok_or_else(|| invalid("A market occurrence position lies past its chain"))?;
        let page = published.reader.events(&[key], i32::MIN, i32::MAX, position % CHUNK, 1)?;
        self.read();
        let row = page
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| invalid("A market occurrence chain is shorter than its count"))?;
        let (sequence, payload): (u64, Value) = serde_json::from_value(row.row)?;
        Ok((row.hour, sequence, payload))
    }

    fn read(&self) {
        #[cfg(test)]
        {
            let counter = if self.index { &LEDGER_INDEX_READS } else { &LEDGER_ROWS_READ };
            counter.with(|read| read.set(read.get() + 1));
        }
    }
}

/// A chain over the hours from `from`: the position its unpaged entries end
/// at, and the newest of them.
struct Segment {
    chain: usize,
    kind: u8,
    from: i32,
    /// The position after its newest unpaged entry; entries from `from` up to
    /// it remain.
    position: u64,
    /// Its entries through the last hour, from the chain's start.
    end: u64,
    /// The newest unpaged entry: its sequence and payload.
    head: Option<(u64, Value)>,
}

/// The segments merged newest first by sequence.
struct Ledger {
    chains: Vec<Chain>,
    segments: Vec<Segment>,
}

impl Ledger {
    /// Each segment `(chain, kind, from, until)` at its position from
    /// `cursor`, else at its end, with its head. The cursor names each segment
    /// by its chain and first hour; a segment it doesn't name had no entries
    /// left then, or gained its first ones after that page was read, so they
    /// are all newer than it and none remain.
    fn open(
        published: &mut Published<'_, Field, Extra>,
        chains: Vec<Chain>,
        wanted: Vec<(usize, u8, i32, i32)>,
        cursor: Option<&str>,
    ) -> io::Result<Self> {
        let positions = cursor.map(parse_cursor).transpose()?;
        let mut segments = Vec::with_capacity(wanted.len());
        for (chain, kind, from, until) in wanted {
            let end = chains[chain].end(published, until)?;
            let position = match &positions {
                None => end,
                Some(positions) => positions.get(&(chains[chain].name(), from)).copied().unwrap_or(0).min(end),
            };
            let mut segment = Segment { chain, kind, from, position, end, head: None };
            segment.head = Self::head(published, &chains[chain], &segment)?;
            segments.push(segment);
        }
        Ok(Self { chains, segments })
    }

    fn head(
        published: &mut Published<'_, Field, Extra>,
        chain: &Chain,
        segment: &Segment,
    ) -> io::Result<Option<(u64, Value)>> {
        let Some(last) = segment.position.checked_sub(1) else {
            return Ok(None);
        };
        let (hour, sequence, payload) = chain.entry(published, last)?;
        Ok((hour >= segment.from).then_some((sequence, payload)))
    }

    /// The newest unpaged occurrence's kind and payload, taken from every
    /// segment that holds it.
    fn next(&mut self, published: &mut Published<'_, Field, Extra>) -> io::Result<Option<(u8, Value)>> {
        let Some((sequence, kind, payload)) = self
            .segments
            .iter()
            .filter_map(|segment| segment.head.as_ref().map(|(sequence, payload)| (*sequence, segment.kind, payload)))
            .max_by_key(|&(sequence, ..)| sequence)
            .map(|(sequence, kind, payload)| (sequence, kind, payload.clone()))
        else {
            return Ok(None);
        };
        let chains = &self.chains;
        for segment in &mut self.segments {
            if segment.head.as_ref().is_some_and(|(held, _)| *held == sequence) {
                segment.position -= 1;
                segment.head = Self::head(published, &chains[segment.chain], segment)?;
            }
        }
        Ok(Some((kind, payload)))
    }

    /// Every segment's entries through the last hour: the stream ledger's total.
    fn total(&self) -> u64 {
        self.segments.iter().map(|segment| segment.end).sum()
    }

    /// The positions to continue from, or None when every segment is paged.
    fn cursor(&self) -> Option<String> {
        let parts: Vec<String> = self
            .segments
            .iter()
            .filter(|segment| segment.head.is_some())
            .map(|segment| format!("{}:{}:{}", self.chains[segment.chain].name(), segment.from, segment.position))
            .collect();
        (!parts.is_empty()).then(|| parts.join("."))
    }
}

/// Each chosen holder's hours, merged: the hours it held a chosen tag, and the
/// hours it held a member's tag inside that member's own hours.
fn chosen_spans(published: &Published<'_, Field, Extra>, params: &MarketLedgerParams) -> BTreeMap<u32, Vec<(i32, i32)>> {
    let mut spans = if params.tags.is_empty() { BTreeMap::new() } else { published.holder_spans(&params.tags) };
    for member in params.members.iter().flatten() {
        for (holder, held) in published.holder_spans(std::slice::from_ref(&member.tag)) {
            let clipped = held.into_iter().filter_map(|(from, until)| {
                let (from, until) = (from.max(member.first_hour), until.min(member.last_hour));
                (from <= until).then_some((from, until))
            });
            spans.entry(holder).or_default().extend(clipped);
        }
    }
    spans.retain(|_, held| {
        held.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(held.len());
        for &(from, until) in held.iter() {
            match merged.last_mut() {
                Some(last) if from <= last.1.saturating_add(1) => last.1 = last.1.max(until),
                _ => merged.push((from, until)),
            }
        }
        *held = merged;
        !held.is_empty()
    });
    spans
}

/// A cursor: each segment's (chain, first hour) and the position its unpaged
/// entries end at.
fn parse_cursor(cursor: &str) -> io::Result<BTreeMap<(u64, i32), u64>> {
    cursor
        .split('.')
        .map(|part| {
            let mut fields = part.splitn(3, ':');
            let key = fields.next().and_then(|key| key.parse::<u64>().ok());
            let from = fields.next().and_then(|from| from.parse::<i32>().ok());
            let position = fields.next().and_then(|position| position.parse::<u64>().ok());
            match (key, from, position) {
                (Some(key), Some(from), Some(position)) => Ok(((key, from), position)),
                _ => Err(invalid("Invalid market ledger cursor")),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
