//! Who trades what with whom over a branch, for the Trade flows widget: every
//! classic resource trade deal (`ResourceExchanges`) as a flow from its seller
//! (the exporter) to its buyer (the importer) for one resource.
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

use super::resource_balance::mark;
use dashboard_base::products::{
    Product,
    country_history::{self, CountryIdentityRun, Identity, RecordedHistories},
    resources::{Learned, RESOURCE_DECIMALS, ResourceNaming},
};
use dashboard_base::{
    prepared::frame_changes::field_mask, products::country_groups::HourSpan, source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use replay::rows::{DeltaRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};
use ts_rs::TS;

#[cfg(test)]
use dashboard_text::metadata::validate_campaign;
#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
#[cfg(test)]
use std::{collections::VecDeque, sync::atomic::Ordering};

pub mod prepared;
pub(crate) mod sample;

pub struct TradeFlows;

#[cfg(test)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
struct Terms {
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
}

/// An exchange's seller and buyer links, its resource and its terms.
type Operand = (u32, u32, u32, Terms);
/// A flow: its native seller holder, native buyer holder and resource.
type FlowKey = (Identity, Identity, u32);

/// The ResourceExchanges fields an [`Operand`] reads; its id is the row key.
static EXCHANGE_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::ResourceExchanges,
        &[
            "exporter",
            "importer",
            "resource",
            "delivered",
            "assigned_factories",
            "convoys_assigned",
            "convoys_required",
        ],
    )
});

/// The Countries fields the holders are read from; a country's id is the key.
static HOLDER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));

fn exchange_operand(row: &replay::rows::ResourceExchangeRow) -> Operand {
    (
        row.exporter,
        row.importer,
        row.resource,
        Terms {
            delivered: row.delivered,
            factories: row.assigned_factories,
            assigned: row.convoys_assigned,
            required: row.convoys_required,
            deals: 1,
        },
    )
}

/// Native fields in recorded order, each with its row key. Re-resolving
/// aliases must not regroup these operands before checked addition: signed
/// partial sums can overflow.
fn exchange_operands(rows: &[replay::rows::ResourceExchangeRow]) -> Vec<(u128, Operand)> {
    rows.iter()
        .map(|row| (row.key(), exchange_operand(row)))
        .collect()
}

/// The resources recorded definitions and tokens name this hour.
fn read_names(frame: &replay::frame::DecodedFrame) -> Option<Learned> {
    frame
        .complete::<tables::ResourceDefinitions>()
        .ok()
        .zip(frame.complete::<tables::LexicalTokens>().ok())
        .map(|(definitions, lexical)| {
            let mut names = Learned::default();
            names.learn(definitions, lexical);
            names
        })
}

#[derive(Default)]
struct Inputs {
    countries: Option<dashboard_base::products::territory::CountryReads>,
    /// How many Countries rows `countries` was read from.
    country_rows: Option<usize>,
    exchanges: Option<Vec<(u128, Operand)>>,
    names: Option<Learned>,
    ready: bool,
    #[cfg(test)]
    row_visits: usize,
    /// Countries rows compared with or read into `countries`.
    #[cfg(test)]
    country_visits: usize,
}
impl Inputs {
    fn refresh(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> bool {
        let forced = !self.ready;
        let mut changed = forced;
        if forced
            || changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
        {
            let rows = frame.complete::<tables::Countries>().ok();
            #[cfg(test)]
            {
                self.country_visits += rows.map_or(0, |rows| rows.len());
            }
            let same = rows.is_some_and(|rows| {
                self.countries
                    .as_ref()
                    .is_some_and(|seen| seen.matches(frame, rows))
            });
            if !same {
                self.country_rows = rows.map(|rows| rows.len());
                self.countries = rows.map(|rows| {
                    if !forced
                        && !changes.rows_changed[TableId::CountryLinks.index()]
                        && let Some(seen) = &self.countries
                    {
                        dashboard_base::products::territory::CountryReads::from_slots(
                            frame,
                            rows,
                            Arc::clone(seen.slots()),
                        )
                    } else {
                        dashboard_base::products::territory::CountryReads::new(frame, rows)
                    }
                });
                changed = true;
            }
        }
        if forced || changes.rows_changed[TableId::ResourceExchanges.index()] {
            self.exchanges = frame
                .complete::<tables::ResourceExchanges>()
                .ok()
                .map(|rows| {
                    #[cfg(test)]
                    {
                        self.row_visits += rows.len();
                    }
                    exchange_operands(rows)
                });
            changed = true;
        }
        if forced
            || changes.rows_changed[TableId::ResourceDefinitions.index()]
            || changes.rows_changed[TableId::LexicalTokens.index()]
        {
            self.names = frame
                .complete::<tables::ResourceDefinitions>()
                .ok()
                .zip(frame.complete::<tables::LexicalTokens>().ok())
                .map(|(definitions, lexical)| {
                    #[cfg(test)]
                    {
                        self.row_visits += definitions.len();
                    }
                    let mut names = Learned::default();
                    names.learn(definitions, lexical);
                    names
                });
            changed = true;
        }
        self.ready = true;
        changed
    }
    fn identities(&self) -> HashMap<u32, (&str, u32)> {
        let mut ids = HashMap::new();
        if let Some(countries) = &self.countries {
            for (tag, identity) in countries.holders() {
                if let Some((slot, id)) = *identity {
                    ids.entry(slot).or_insert((tag.as_str(), id));
                }
            }
        }
        ids
    }
    /// Whether this frame names every resource as the current names do.
    fn same_names(&mut self, frame: &replay::frame::DecodedFrame) -> bool {
        #[cfg(test)]
        {
            self.row_visits += frame
                .complete::<tables::ResourceDefinitions>()
                .map_or(0, |definitions| definitions.len());
        }
        match (read_names(frame), &self.names) {
            (Some(names), Some(current)) => names.list() == current.list(),
            _ => false,
        }
    }
    /// The flow an exchange counts toward, with `holders` naming each slot's
    /// holder (tag and country id); None when [`flow_rows`] leaves it out: a
    /// seller or buyer the links do not resolve, or a resource the names do
    /// not name.
    fn attribute(
        &self,
        holders: &HashMap<u32, (String, u32)>,
        &(exporter, importer, resource, _): &Operand,
    ) -> Option<FlowKey> {
        let slots = self.countries.as_ref()?.slots();
        let side = |raw: u32| {
            slots
                .get(&raw)
                .and_then(|&slot| holders.get(&slot).map(|&(_, id)| (slot, id)))
        };
        let (seller, buyer) = (side(exporter)?, side(importer)?);
        self.names
            .as_ref()?
            .contains(resource)
            .then_some((seller, buyer, resource))
    }
}

/// What observing an hour from its changed rows keeps between hours, under
/// the holders and names of the last whole-table observation.
struct RowIndex {
    /// Each slot's holder: the tag and country id flows name it by.
    holders: HashMap<u32, (String, u32)>,
    /// Each active flow's exchanges by row key, ascending: the table's order.
    members: HashMap<FlowKey, Vec<u128>>,
    /// How many active flows name each slot as seller or buyer.
    named: HashMap<u32, usize>,
}

/// Count one more active flow naming `slot`; the first lists it in `carried`.
fn name_slot(
    named: &mut HashMap<u32, usize>,
    carried: &mut Vec<(u32, String, u32)>,
    holders: &HashMap<u32, (String, u32)>,
    slot: u32,
) {
    let count = named.entry(slot).or_default();
    *count += 1;
    if *count == 1
        && let Err(at) = carried.binary_search_by_key(&slot, |&(slot, _, _)| slot)
    {
        let (tag, id) = &holders[&slot];
        carried.insert(at, (slot, tag.clone(), *id));
    }
}

/// Count one fewer active flow naming `slot`; the last unlists it.
fn unname_slot(named: &mut HashMap<u32, usize>, carried: &mut Vec<(u32, String, u32)>, slot: u32) {
    let Some(count) = named.get_mut(&slot) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        named.remove(&slot);
        if let Ok(at) = carried.binary_search_by_key(&slot, |&(slot, _, _)| slot) {
            carried.remove(at);
        }
    }
}

/// Native aggregation shared by history, preparation and stateless sampling.
#[allow(clippy::type_complexity)]
fn flow_rows<'a>(
    inputs: &'a Inputs,
    ids: &HashMap<u32, (&'a str, u32)>,
    wanted: Option<&BTreeSet<Identity>>,
    rejected: &mut Vec<(Option<(Identity, Identity)>, u32)>,
) -> io::Result<Option<BTreeMap<(Identity, Identity, u32), (&'a str, &'a str, Terms)>>> {
    let (Some(exchanges), Some(names), Some(countries)) =
        (&inputs.exchanges, &inputs.names, &inputs.countries)
    else {
        return Ok(None);
    };
    let slots = countries.slots();
    let mut read = BTreeMap::new();
    for &(_, (exporter, importer, resource, operand)) in exchanges {
        let side = |raw| {
            slots
                .get(&raw)
                .and_then(|slot| ids.get(slot).map(|(tag, id)| (*slot, *tag, *id)))
        };
        let (Some((seller_slot, seller, seller_id)), Some((buyer_slot, buyer, buyer_id))) =
            (side(exporter), side(importer))
        else {
            rejected.push((None, 1));
            continue;
        };
        let seller_identity = (seller_slot, seller_id);
        let buyer_identity = (buyer_slot, buyer_id);
        if wanted
            .is_some_and(|ids| !ids.contains(&seller_identity) && !ids.contains(&buyer_identity))
        {
            continue;
        }
        if !names.contains(resource) {
            rejected.push((Some((seller_identity, buyer_identity)), 1));
            continue;
        }
        let terms = read
            .entry((seller_identity, buyer_identity, resource))
            .or_insert((seller, buyer, Terms::default()));
        terms.2 = terms
            .2
            .add(operand)
            .ok_or_else(|| dashboard_core::invalid("recorded trade terms overflow their sum"))?;
    }
    Ok(Some(read))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Flow {
    seller: String,
    buyer: String,
    seller_identity: Identity,
    buyer_identity: Identity,
    resource: u32,
    runs: Vec<Run>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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

/// Scans one branch for every trade deal, or those of `tags` as seller or buyer.
#[cfg(test)]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tags: Option<&[String]>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedFlows> {
    let history = country_history::scan_histories(replay, branch, false, &[], cancelled)?;
    scan_with_history(replay, branch, tags, &history, cancelled)
}
#[cfg(test)]
fn scan_with_history(
    replay: &Replay,
    branch: u64,
    tags: Option<&[String]>,
    history: &RecordedHistories,
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
    let mut scan = Scan {
        wanted: tags.map(|tags| country_history::selected_identities(history, tags)),
        ..Default::default()
    };
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
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
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    scan.result.country_identities = history
        .countries
        .iter()
        .filter(|(_, country)| !country.identities.is_empty())
        .map(|(tag, country)| TradeCountryIdentity {
            tag: tag.clone(),
            runs: country.identities.as_ref().clone(),
        })
        .collect();
    Ok(scan.result)
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct Scan {
    result: ScannedFlows,
    learned: Learned,
    #[serde(with = "dashboard_core::map_pairs")]
    index: HashMap<(Identity, Identity, u32), usize>,
    known: bool,
    carried: Vec<(u32, String, u32)>,
    last_hour: Option<i32>,
    wanted: Option<BTreeSet<Identity>>,
    #[serde(skip)]
    rejected: Vec<(Option<(Identity, Identity)>, u32)>,
    active: BTreeSet<usize>,
    materialized: Option<i32>,
    #[serde(skip)]
    inputs: Inputs,
    /// Kept while hours are observed from their changed rows; any
    /// whole-table observation drops it.
    #[serde(skip)]
    row_index: Option<RowIndex>,
    #[cfg(test)]
    #[serde(skip)]
    flow_extensions: usize,
}
impl Scan {
    #[cfg(test)]
    fn observe(&mut self, decoded: &replay::frame::DecodedFrame) -> io::Result<()> {
        let requested = self.wanted.as_ref().map(|ids| {
            let mut requested = ids
                .iter()
                .map(|&id| (identity(id), "actor".into()))
                .collect::<Vec<_>>();
            if requested.is_empty() {
                requested.push((String::new(), String::new()));
            }
            requested
        });
        self.inputs.refresh(
            decoded,
            &dashboard_base::prepared::frame_changes::FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
                updates: None,
            },
        );
        let sampled = sample::sample_inputs(decoded, requested.as_deref(), &self.inputs)?;
        self.observe_sampled(decoded, &sampled)
    }
    fn advance(&mut self, hour: i32) {
        self.last_hour = Some(hour);
    }
    fn seal(&mut self) {
        if self.known
            && let Some(through) = self.last_hour
        {
            for &index in &self.active {
                if let Some(run) = self.result.flows[index]
                    .runs
                    .last_mut()
                    .filter(|run| run.last_hour < through)
                {
                    run.last_hour = through;
                    #[cfg(test)]
                    {
                        self.flow_extensions += 1;
                    }
                }
            }
        }
        self.seal_spans();
    }
    /// Shared read and refusal clocks, independent of individual flow endpoints.
    fn seal_spans(&mut self) {
        let Some((from, through)) = self
            .materialized
            .zip(self.last_hour)
            .filter(|(from, through)| from < through)
        else {
            return;
        };
        if self.known {
            if let Some(span) = self.result.known.last_mut() {
                span.last_hour = through;
            }
            if let Some(read) = self.result.reads.last_mut() {
                *read = through;
            }
            if let Some(span) = self
                .result
                .partial
                .last_mut()
                .filter(|span| span.last_hour == from)
            {
                span.last_hour = through;
            }
            let hours =
                usize::try_from(i64::from(through) - i64::from(from)).expect("trade frontier");
            for (pair, count) in &self.rejected {
                if pair.is_some() {
                    self.result.unnamed += hours * *count as usize;
                } else {
                    self.result.unresolved += hours * *count as usize;
                }
            }
        }
        self.materialized = Some(through);
    }
    fn observe_sampled(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        sampled: &dashboard_base::prepared::query::progressive::Samples,
    ) -> io::Result<()> {
        let Self {
            result,
            learned,
            index,
            known,
            carried,
            last_hour,
            wanted,
            rejected,
            active,
            materialized,
            inputs,
            row_index: _,
            #[cfg(test)]
                flow_extensions: _,
        } = self;
        rejected.clear();
        active.clear();
        *materialized = Some(decoded.hour);
        let hour = decoded.hour;
        let continues = last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        *last_hour = Some(hour);
        if !continues {
            *known = false;
        }
        if sampled
            .get(&(String::new(), "known".into()))
            .and_then(|(_, value)| *value)
            .is_none()
        {
            // A read that cannot say who trades what: unknown from here.
            *known = false;
            return Ok(());
        }
        if let Some(names) = &inputs.names {
            learned.learn_names(names);
        }
        let ids: HashMap<u32, (&str, u32)> = sampled
            .iter()
            .filter_map(|((tag, field), (holder, _))| {
                (field == "holder")
                    .then_some(holder.map(|(slot, id)| (slot, (tag.as_str(), id))))
                    .flatten()
            })
            .collect();
        let mut read = BTreeMap::new();
        for ((token, field), (_, amount)) in sampled {
            if field == "rejected" {
                let pair: Option<(Identity, Identity)> = serde_json::from_str(token)?;
                if pair.is_none_or(|(seller, buyer)| {
                    wanted
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&seller) || ids.contains(&buyer))
                }) {
                    rejected.push((
                        pair,
                        amount.unwrap_or(0).try_into().map_err(|_| {
                            dashboard_core::invalid("Invalid sampled rejected deal count")
                        })?,
                    ));
                }
            }
            if field != "delivered" {
                continue;
            }
            let (seller, buyer, resource) = sample::pair(token)?;
            if wanted
                .as_ref()
                .is_some_and(|ids| !ids.contains(&seller) && !ids.contains(&buyer))
            {
                continue;
            }
            let mut amounts = [0; 5];
            for (field, amount) in sample::FIELDS.iter().zip(&mut amounts) {
                *amount = sampled
                    .get(&(token.clone(), (*field).to_owned()))
                    .and_then(|(_, value)| *value)
                    .ok_or_else(|| dashboard_core::invalid("Missing native sampled trade terms"))?;
            }
            read.insert(
                (seller, buyer, resource),
                (ids[&seller.0].0, ids[&buyer.0].0, sample::terms(amounts)?),
            );
        }
        let same = carried
            .iter()
            .all(|(slot, tag, id)| ids.get(slot) == Some(&(tag.as_str(), *id)));
        // The slots this read's flows name, with the country each held.
        let named: BTreeSet<u32> = read
            .keys()
            .flat_map(|(seller, buyer, _)| [seller.0, buyer.0])
            .collect();
        // Deals this read lists but cannot attribute: the hour is known only in part.
        let mut skipped = 0_u32;
        for (pair, count) in rejected.iter() {
            if pair.is_some() {
                result.unnamed += *count as usize;
            } else {
                result.unresolved += *count as usize;
            }
            skipped = skipped.saturating_add(*count);
        }
        result.reads.push(hour);
        mark(&mut result.known, hour);
        if skipped > 0 {
            mark_partial(&mut result.partial, hour, skipped);
        }
        let was_known = *known && same;
        *known = true;
        *carried = named
            .iter()
            .map(|slot| {
                let (tag, id) = ids[slot];
                (*slot, tag.to_owned(), id)
            })
            .collect();
        for ((seller_identity, buyer_identity, resource), (seller, buyer, terms)) in read {
            let key = (seller_identity, buyer_identity, resource);
            let flow = *index.entry(key).or_insert_with(|| {
                result.flows.push(Flow {
                    seller: seller.to_owned(),
                    buyer: buyer.to_owned(),
                    seller_identity,
                    buyer_identity,
                    resource,
                    runs: Vec::new(),
                });
                result.flows.len() - 1
            });
            active.insert(flow);
            let runs = &mut result.flows[flow].runs;
            match runs.last_mut() {
                Some(run)
                    if was_known
                        && run.last_hour.checked_add(1) == Some(hour)
                        && run.terms == terms
                        && run.seller_tag == seller
                        && run.buyer_tag == buyer =>
                {
                    run.last_hour = hour
                }
                _ => {
                    runs.push(Run {
                        first_hour: hour,
                        last_hour: hour,
                        terms,
                        seller_tag: seller.to_owned(),
                        buyer_tag: buyer.to_owned(),
                    });
                }
            }
        }
        result.resources = learned.list();
        Ok(())
    }
    /// The row index of the current operands, holders and active flows; None
    /// when the operands are not in key order, or do not group into exactly
    /// the active flows.
    fn index_rows(&self) -> Option<RowIndex> {
        let exchanges = self.inputs.exchanges.as_ref()?;
        if !exchanges.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return None;
        }
        let holders: HashMap<u32, (String, u32)> = self
            .inputs
            .identities()
            .into_iter()
            .map(|(slot, (tag, id))| (slot, (tag.to_owned(), id)))
            .collect();
        let mut members = HashMap::<FlowKey, Vec<u128>>::new();
        for (key, operand) in exchanges {
            if let Some(flow) = self.inputs.attribute(&holders, operand) {
                members.entry(flow).or_default().push(*key);
            }
        }
        let mut named = HashMap::<u32, usize>::new();
        for &index in &self.active {
            let flow = &self.result.flows[index];
            for slot in [flow.seller_identity.0, flow.buyer_identity.0] {
                *named.entry(slot).or_default() += 1;
            }
        }
        let exact = members.len() == self.active.len()
            && named.len() == self.carried.len()
            && members.keys().all(|flow| {
                self.index
                    .get(flow)
                    .is_some_and(|index| self.active.contains(index))
            });
        exact.then_some(RowIndex {
            holders,
            members,
            named,
        })
    }
    /// Observe a known hour continuing the last observed one, whose exchanges
    /// differ from the previous hour's by exactly the recorded `changed` and
    /// `deleted` rows, with the holders and names unchanged: the hour
    /// [`Self::observe_sampled`] makes of the whole tables, regrouping only
    /// the flows those rows name before or after the hour. Every other flow
    /// continues. Returns each flow observed again, with whether it was active
    /// before. None, changing nothing, when a changed row is not at hand, or
    /// is or was left out of the flows: what an hour leaves out is counted
    /// from the whole tables.
    fn observe_rows(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changed: &[DeltaRow],
        deleted: &[u128],
    ) -> io::Result<Option<BTreeMap<usize, bool>>> {
        let hour = frame.hour;
        if !self.known
            || self.wanted.is_some()
            || self.last_hour.and_then(|last| last.checked_add(1)) != Some(hour)
        {
            return Ok(None);
        }
        if self.row_index.is_none() {
            #[cfg(test)]
            {
                self.inputs.row_visits += self.inputs.exchanges.as_ref().map_or(0, Vec::len);
            }
            self.row_index = self.index_rows();
        }
        let (Some(index), Some(exchanges)) = (&self.row_index, &self.inputs.exchanges) else {
            return Ok(None);
        };
        let at = |key: u128| exchanges.binary_search_by_key(&key, |&(key, _)| key);
        // Read every changed operand before changing anything. A change to
        // fields no operand reads leaves the operand as it was.
        let mut updates = BTreeMap::<u128, Option<Operand>>::new();
        if !changed.is_empty() {
            let Ok(rows) = frame.complete::<tables::ResourceExchanges>() else {
                return Ok(None);
            };
            for change in changed {
                if change.mask & *EXCHANGE_FIELDS == 0 && at(change.key).is_ok() {
                    continue;
                }
                let Ok(row) = rows.binary_search_by_key(&change.key, |row| row.key()) else {
                    return Ok(None);
                };
                updates.insert(change.key, Some(exchange_operand(&rows[row])));
            }
        }
        updates.extend(deleted.iter().map(|&key| (key, None)));
        // Each changed exchange's flow before the hour and after it.
        let mut moves = Vec::with_capacity(updates.len());
        for (&key, operand) in &updates {
            let before = at(key).ok().map(|position| &exchanges[position].1);
            let mut flows: [Option<FlowKey>; 2] = [None; 2];
            for (flow, side) in flows.iter_mut().zip([before, operand.as_ref()]) {
                if let Some(side) = side {
                    let Some(attributed) = self.inputs.attribute(&index.holders, side) else {
                        return Ok(None);
                    };
                    *flow = Some(attributed);
                }
            }
            moves.push(flows);
        }
        #[cfg(test)]
        {
            self.inputs.row_visits += updates.len();
        }
        let index = self.row_index.as_mut().expect("trade row index");
        let exchanges = self.inputs.exchanges.as_mut().expect("trade exchanges");
        let mut regrouped = BTreeSet::<FlowKey>::new();
        for ((key, operand), [before, after]) in updates.into_iter().zip(moves) {
            match (
                exchanges.binary_search_by_key(&key, |&(key, _)| key),
                operand,
            ) {
                (Ok(position), Some(operand)) => exchanges[position].1 = operand,
                (Err(position), Some(operand)) => exchanges.insert(position, (key, operand)),
                (Ok(position), None) => {
                    exchanges.remove(position);
                }
                (Err(_), None) => {}
            }
            if let Some(flow) = before {
                if let Some(keys) = index.members.get_mut(&flow) {
                    if let Ok(position) = keys.binary_search(&key) {
                        keys.remove(position);
                    }
                    if keys.is_empty() {
                        index.members.remove(&flow);
                    }
                }
                regrouped.insert(flow);
            }
            if let Some(flow) = after {
                let keys = index.members.entry(flow).or_default();
                if let Err(position) = keys.binary_search(&key) {
                    keys.insert(position, key);
                }
                regrouped.insert(flow);
            }
        }
        // Each regrouped flow's terms, summed over its exchanges in table
        // order as the whole tables sum them; none once no exchange counts
        // toward it.
        let mut sums = Vec::with_capacity(regrouped.len());
        for flow in regrouped {
            let mut terms: Option<Terms> = None;
            for key in index.members.get(&flow).into_iter().flatten() {
                let position = exchanges
                    .binary_search_by_key(key, |&(key, _)| key)
                    .expect("grouped trade exchange");
                terms = Some(
                    terms
                        .unwrap_or_default()
                        .add(exchanges[position].1.3)
                        .ok_or_else(|| {
                            dashboard_core::invalid("recorded trade terms overflow their sum")
                        })?,
                );
            }
            sums.push((flow, terms));
        }
        self.seal_spans();
        self.materialized = Some(hour);
        self.last_hour = Some(hour);
        let mut skipped = 0_u32;
        for (pair, count) in &self.rejected {
            if pair.is_some() {
                self.result.unnamed += *count as usize;
            } else {
                self.result.unresolved += *count as usize;
            }
            skipped = skipped.saturating_add(*count);
        }
        self.result.reads.push(hour);
        mark(&mut self.result.known, hour);
        if skipped > 0 {
            mark_partial(&mut self.result.partial, hour, skipped);
        }
        let Self {
            result,
            index: flows,
            carried,
            active,
            row_index,
            ..
        } = self;
        let index = row_index.as_mut().expect("trade row index");
        let mut touched = BTreeMap::new();
        for (flow, terms) in sums {
            let existing = flows.get(&flow).copied();
            let was_active = existing.is_some_and(|at| active.contains(&at));
            if let Some(at) = existing.filter(|_| was_active)
                && let Some(run) = result.flows[at].runs.last_mut()
            {
                run.last_hour = hour - 1;
                #[cfg(test)]
                {
                    self.flow_extensions += 1;
                }
            }
            let Some(terms) = terms else {
                if let Some(at) = existing.filter(|_| was_active) {
                    active.remove(&at);
                    for slot in [flow.0.0, flow.1.0] {
                        unname_slot(&mut index.named, carried, slot);
                    }
                    touched.insert(at, true);
                }
                continue;
            };
            let seller = &index.holders[&flow.0.0].0;
            let buyer = &index.holders[&flow.1.0].0;
            let at = *flows.entry(flow).or_insert_with(|| {
                result.flows.push(Flow {
                    seller: seller.clone(),
                    buyer: buyer.clone(),
                    seller_identity: flow.0,
                    buyer_identity: flow.1,
                    resource: flow.2,
                    runs: Vec::new(),
                });
                result.flows.len() - 1
            });
            active.insert(at);
            let runs = &mut result.flows[at].runs;
            match runs.last_mut() {
                Some(run)
                    if run.last_hour.checked_add(1) == Some(hour)
                        && run.terms == terms
                        && run.seller_tag == *seller
                        && run.buyer_tag == *buyer =>
                {
                    run.last_hour = hour
                }
                _ => runs.push(Run {
                    first_hour: hour,
                    last_hour: hour,
                    terms,
                    seller_tag: seller.clone(),
                    buyer_tag: buyer.clone(),
                }),
            }
            if !was_active {
                for slot in [flow.0.0, flow.1.0] {
                    name_slot(&mut index.named, carried, &index.holders, slot);
                }
            }
            touched.insert(at, was_active);
        }
        // Other active flows hold through last_hour and materialize at the
        // next whole-table observation or publication, without a visit here.
        Ok(Some(touched))
    }
}

/// A scan's branch and countries (none for the whole world).
#[cfg(test)]
type ScanKey = (u64, Option<Vec<String>>);

#[cfg(test)]
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
                let history = country_history::branch_histories(
                    source,
                    params.branch(),
                    false,
                    &[],
                    cancelled,
                )?;
                let scanned = Arc::new(scan_with_history(
                    &source.replay(),
                    params.branch(),
                    key.1.as_deref(),
                    &history,
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
        reply(source, params, context, &scanned, cancelled)
    }
}

pub(crate) fn reply(
    source: &Source,
    params: &TradeFlowParams,
    context: &LoadContext,
    scanned: &ScannedFlows,
    cancelled: &AtomicBool,
) -> io::Result<TradeFlowsReply> {
    reply_named(
        &ResourceNaming::of(source, context),
        params,
        scanned,
        cancelled,
    )
}

/// [`reply`], with the resources named through `naming`.
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
mod tests;
