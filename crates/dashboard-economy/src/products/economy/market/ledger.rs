use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply},
    },
    queries::country_ids::{Names, RunLinks},
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::{
        Cell, Reader,
        occurrence::{Ask, Chip, Direction, Index, Occurrence, Order, PlacedGap, Role, SearchText, Selection, Shown},
    },
    series::{MissingSpan, ranked::GroupSelection},
    timeline::Timeline,
};
use dashboard_world::products::equipment_names::{self as names, Named};
use replay::campaign::Replay;
use replay_data::rows::{MarketEventRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const LOG: &str = "market-events";

const CHILDREN_SHOWN: usize = 32;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

struct Columns {
    occurrence: usize,
    hour: usize,
    agreement: usize,
    country: usize,
    receiver: usize,
    kind: usize,
    mode: usize,
    ordinal: usize,
    variant: usize,
    amount: usize,
    total: usize,
    flags: usize,
}

impl Columns {
    fn read() -> io::Result<Self> {
        let field = |name| dashboard_declare::field(TableId::MarketEvents, name);
        Ok(Self {
            occurrence: field("occurrence")?,
            hour: field("hour")?,
            agreement: field("agreement")?,
            country: field("country")?,
            receiver: field("receiver")?,
            kind: field("kind")?,
            mode: field("mode")?,
            ordinal: field("ordinal")?,
            variant: field("variant")?,
            amount: field("amount_raw")?,
            total: field("total_raw")?,
            flags: field("flags")?,
        })
    }
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MarketContractStatus {
    Requested,
    Active,
    Ended,
    NotRecorded,
    RequestEnded,
}

const STATUSES: [MarketContractStatus; 5] =
    [MarketContractStatus::Requested, MarketContractStatus::Active, MarketContractStatus::Ended, MarketContractStatus::NotRecorded, MarketContractStatus::RequestEnded];

impl MarketContractStatus {
    fn code(self) -> u8 {
        STATUSES.iter().position(|held| *held == self).unwrap_or(0) as u8
    }

    fn of_code(code: u8) -> Option<Self> {
        STATUSES.get(usize::from(code)).copied()
    }
}

const LIVE: [u8; 2] = [0, 1];

const HIDDEN: u8 = STATUSES.len() as u8;

const CONTRACTS: [u16; 2] = [0, 1];
const EVENT: u16 = 2;
const MORE: u16 = 3;

const START_UNSEEN: &str = "The contract began before its first recorded event";
const UNTOLD_REMOVAL: &str = "do not record whether a removal ended a request or an accepted contract";
const UNDATED: &str = "do not record their own hour";
const UNTOLD: [&str; 2] = [UNTOLD_REMOVAL, UNDATED];
const NO_DESIGN: &str = "A delivery's design was not recorded";
const NO_AMOUNT: &str = "A delivery's amount was not recorded";
const CERTIFIED_TOTAL: i64 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MarketContractEventKind {
    RequestPosted,
    ContractStarted,
    ContractEnded,
    RequestEnded,
    Delivery,
    Payment,
    DeliveryUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketContractItem {
    pub variant: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    #[doc = " The latest certified cumulative delivery plus the deliveries witnessed after it, read by the hour asked; with `deliveryUnavailable` only a lower bound, null when none was recorded."]
    pub delivered_raw: Option<String>,
    pub contracted_raw: Option<String>,
    pub delivery_unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketQuote {
    pub variant: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " The recorded price of this variant in the terms, as stored; never summed across variants."]
    pub price_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketContractEvent {
    pub key: String,
    #[doc = " The contract this event belongs to, by its key, and its parties at the hour asked."]
    pub contract: String,
    pub seller: Option<String>,
    pub buyer: Option<String>,
    #[doc = " The game hour the event happened; it is listed only from the hour it was read."]
    pub hour: i32,
    pub kind: MarketContractEventKind,
    pub variant: Option<String>,
    pub equipment: Option<String>,
    pub equipment_missing: Option<String>,
    pub amount_raw: Option<String>,
    pub amount_not_recorded: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketContractRow {
    #[doc = " The recorded contract: seller, agreement and whether it is a request, and its incarnation."]
    pub key: String,
    #[doc = " The hour the request was posted or the contract accepted; the first event seen when neither was (`exact` false)."]
    pub hour: i32,
    pub observed: i32,
    pub exact: bool,
    pub ordinal: String,
    pub live: bool,
    pub request: bool,
    pub requested_hour: Option<i32>,
    pub started_hour: Option<i32>,
    pub ended_hour: Option<i32>,
    pub status: MarketContractStatus,
    pub seller: Option<String>,
    pub seller_not_recorded: bool,
    pub buyer: Option<String>,
    pub buyer_not_recorded: bool,
    pub items: Vec<MarketContractItem>,
    #[doc = " Why deliveries are not known for the whole contract (a refusal naming no design, or an unread stretch)."]
    pub delivery_missing: Option<String>,
    #[doc = " The payments witnessed by the hour asked; with `paidMissing` only a lower bound."]
    pub paid_raw: Option<String>,
    #[doc = " Why the paid total is not known by the hour asked; null with `paidRaw` null means nothing paid yet."]
    pub paid_missing: Option<String>,
    pub quotes: Vec<MarketQuote>,
    pub terms_missing: Option<String>,
    #[doc = " The events read by the hour asked; an expanded contract lists them as `event` lines after it."]
    pub events_total: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketEventsMore {
    pub key: String,
    pub hour: i32,
    pub contract: String,
    pub total: String,
    #[doc = " The older events not listed; zero when the contract lists them all."]
    pub hidden: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "line", rename_all = "camelCase")]
pub enum MarketLedgerLine {
    Contract(MarketContractRow),
    Event(MarketContractEvent),
    More(MarketEventsMore),
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLedgerTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[doc = " The statuses listed, each at the hour asked; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub statuses: Option<Vec<MarketContractStatus>>,
    #[doc = " The contracts, by key, whose newest 32 events are listed as lines after them; the others list none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub expanded: Option<Vec<String>>,
    #[doc = " The expanded contracts, by key, that list every event, paged with the other lines."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub full: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketLedgerTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub live: String,
    pub counts: Vec<OccurrenceCount>,
    pub rows: Vec<MarketLedgerLine>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MarketLedgerTailReply);

impl OccurrenceReply for MarketLedgerTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Clone, Debug)]
struct Child {
    key: u128,
    occurrence: Option<u64>,
    hour: i32,
    observed: i32,
    kind: MarketContractEventKind,
    variant: Option<u64>,
    amount: Option<i64>,
    total: Option<i64>,
    reason: Option<&'static str>,
}

impl Child {
    fn rank(&self) -> Rank {
        (self.hour, self.occurrence, self.key)
    }
}

type Rank = (i32, Option<u64>, u128);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Delivered {
    certified: Option<(i64, Rank)>,
    witnessed: Option<i64>,
    refused: Option<(Rank, &'static str)>,
}

struct Design {
    variant: u64,
    first: u32,
    steps: Vec<(u32, Delivered)>,
}

impl Design {
    fn by(&self, read: usize) -> Delivered {
        let at = self.steps.partition_point(|&(place, _)| (place as usize) < read);
        at.checked_sub(1).map_or_else(Delivered::default, |at| self.steps[at].1)
    }
}

#[derive(Default)]
struct Measures {
    paid: Vec<(u32, i64)>,
    paid_unrecorded: Option<u32>,
    refused_all: Vec<(u32, Rank, &'static str)>,
    designs: Vec<Design>,
}

impl Measures {
    fn of(children: &[Child]) -> Self {
        let mut out = Self::default();
        let mut paid: i64 = 0;
        let mut designs: HashMap<u64, usize> = HashMap::new();
        let mut uncertified: Vec<Vec<(Rank, i64)>> = Vec::new();
        for (place, child) in children.iter().enumerate() {
            let (place, rank) = (place as u32, child.rank());
            match (child.kind, child.variant) {
                (MarketContractEventKind::Payment, _) => match child.amount {
                    Some(amount) => {
                        paid = paid.saturating_add(amount);
                        out.paid.push((place, paid));
                    }
                    None => {
                        out.paid_unrecorded.get_or_insert(place);
                    }
                },
                (MarketContractEventKind::Delivery | MarketContractEventKind::DeliveryUnavailable, None) => {
                    if out.refused_all.last().is_none_or(|&(_, held, _)| rank > held) {
                        out.refused_all.push((place, rank, child.reason.unwrap_or(NO_DESIGN)));
                    }
                }
                (MarketContractEventKind::Delivery | MarketContractEventKind::DeliveryUnavailable, Some(variant)) => {
                    let at = *designs.entry(variant).or_insert_with(|| {
                        out.designs.push(Design { variant, first: place, steps: Vec::new() });
                        uncertified.push(Vec::new());
                        out.designs.len() - 1
                    });
                    let (design, increments) = (&mut out.designs[at], &mut uncertified[at]);
                    let last = design.steps.last().map(|&(_, held)| held);
                    let mut held = last.unwrap_or_default();
                    let after = held.certified.is_none_or(|(_, at)| rank > at);
                    match (child.kind, child.total, child.amount) {
                        (MarketContractEventKind::Delivery, Some(total), _) => {
                            if after {
                                held.certified = Some((total, rank));
                                let covered = increments.partition_point(|&(at, _)| at <= rank);
                                let dropped = increments.drain(..covered).fold(0i64, |sum, (_, amount)| sum.saturating_add(amount));
                                held.witnessed = held.witnessed.map(|sum| sum.saturating_sub(dropped)).filter(|_| !increments.is_empty());
                            }
                        }
                        (MarketContractEventKind::Delivery, None, Some(amount)) => {
                            if after {
                                increments.insert(increments.partition_point(|&(at, _)| at < rank), (rank, amount));
                                held.witnessed = Some(held.witnessed.unwrap_or(0).saturating_add(amount));
                            }
                        }
                        _ => {
                            if held.refused.is_none_or(|(at, _)| rank > at) {
                                held.refused = Some((rank, child.reason.unwrap_or(NO_AMOUNT)));
                            }
                        }
                    }
                    if last != Some(held) {
                        design.steps.push((place, held));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn paid_by(&self, read: usize) -> Option<i64> {
        let at = self.paid.partition_point(|&(place, _)| (place as usize) < read);
        at.checked_sub(1).map(|at| self.paid[at].1)
    }

    fn refused_all_by(&self, read: usize) -> Option<(Rank, &'static str)> {
        let at = self.refused_all.partition_point(|&(place, ..)| (place as usize) < read);
        at.checked_sub(1).map(|at| (self.refused_all[at].1, self.refused_all[at].2))
    }

    fn designs_by(&self, read: usize) -> &[Design] {
        &self.designs[..self.designs.partition_point(|design| (design.first as usize) < read)]
    }
}

struct Unread {
    gaps: Vec<MissingSpan>,
    reach: Vec<i32>,
}

impl Unread {
    fn new(mut gaps: Vec<MissingSpan>) -> Self {
        gaps.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
        let reach = gaps
            .iter()
            .scan(i32::MIN, |held, gap| {
                *held = (*held).max(gap.last_hour);
                Some(*held)
            })
            .collect();
        Self { gaps, reach }
    }

    fn since(&self, known: i32, hour: i32) -> bool {
        let begun = self.gaps.partition_point(|gap| gap.first_hour <= hour);
        begun.checked_sub(1).is_some_and(|at| self.reach[at] > known)
    }

    fn after(&self, hour: i32) -> Option<&MissingSpan> {
        self.gaps.get(self.gaps.partition_point(|gap| gap.first_hour <= hour))
    }

    fn within(&self, from: i32, hour: i32, ended: Option<i32>) -> Option<String> {
        let gap = self.gaps.get(self.reach.partition_point(|&reach| reach < from))?;
        let last = ended.map_or(gap.last_hour, |ended| gap.last_hour.min(ended));
        (gap.first_hour <= ended.unwrap_or(hour) && from <= last)
            .then(|| format!("Market events were not recorded from {} to {}: {}", gap.first_hour.max(from), last, gap.reason))
    }
}

#[derive(Clone, Debug)]
struct Contract {
    seller: Option<i64>,
    buyer: Option<i64>,
    buyer_later: Option<(i64, i32, i32)>,
    agreement: Option<u64>,
    request: bool,
    requested: Option<i32>,
    started: Option<i32>,
    ended: Option<(i32, i32)>,
    children: Vec<Child>,
}

impl Contract {
    fn read_by(&self, hour: i32) -> usize {
        self.children.partition_point(|child| child.observed <= hour)
    }
}

type Party = (Option<String>, bool);

struct Placed {
    key: u128,
    date: i32,
    observed: i32,
    roles: Box<[Role]>,
    states: Box<[(i32, u8)]>,
    text: SearchText,
    contract: Contract,
    measures: Measures,
    seller: Party,
    buyer: Party,
    buyer_later: Option<(i32, Party)>,
}

impl Placed {
    fn buyer_at(&self, hour: i32) -> &Party {
        match &self.buyer_later {
            Some((seen, later)) if *seen <= hour => later,
            _ => &self.buyer,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Line {
    Contract(u32),
    Event(u32, u32),
    More(u32),
}

#[derive(Default, PartialEq, Eq)]
struct Opening {
    expanded: BTreeSet<u32>,
    full: BTreeSet<u32>,
}

type TermsKey = (i32, u32, u64, u8);

struct Ledger {
    contracts: Vec<Placed>,
    by_key: HashMap<u128, u32>,
    unread: Unread,
    untold: Vec<(i32, [u32; 2])>,
    closed: Arc<Index<Line>>,
    opened: Mutex<Option<(Opening, Arc<Index<Line>>)>>,
    terms: Mutex<HashMap<TermsKey, Option<Arc<Terms>>>>,
}

impl Ledger {
    fn opening(&self, params: &MarketLedgerTailParams) -> Opening {
        let find = |keys: &Option<Vec<String>>| -> BTreeSet<u32> {
            keys.iter().flatten().filter_map(|key| key.parse::<u128>().ok()).filter_map(|key| self.by_key.get(&key).copied()).collect()
        };
        let expanded = find(&params.expanded);
        let full = find(&params.full).intersection(&expanded).copied().collect();
        Opening { expanded, full }
    }

    fn index(&self, opening: Opening, cancelled: &AtomicBool) -> io::Result<Arc<Index<Line>>> {
        if opening.expanded.is_empty() {
            return Ok(Arc::clone(&self.closed));
        }
        let mut kept = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((held, index)) = kept.as_ref()
            && *held == opening
        {
            return Ok(Arc::clone(index));
        }
        dashboard_text::metadata::check_cancel(cancelled)?;
        let index = Arc::new(lines(&self.contracts, &opening, self.unread.gaps.clone())?);
        *kept = Some((opening, Arc::clone(&index)));
        Ok(index)
    }
}

fn following(states: &[(i32, u8)], from: i32, hidden: Option<i32>) -> Box<[(i32, u8)]> {
    let at = states.partition_point(|&(hour, _)| hour <= from);
    let Some(&(_, current)) = at.checked_sub(1).and_then(|at| states.get(at)) else { return Box::new([]) };
    let mut out = vec![(from, current)];
    out.extend(states[at..].iter().copied().take_while(|&(hour, _)| hidden.is_none_or(|hidden| hour < hidden)));
    out.extend(hidden.map(|hidden| (hidden, HIDDEN)));
    out.into_boxed_slice()
}

fn lines(contracts: &[Placed], opening: &Opening, gaps: Vec<MissingSpan>) -> io::Result<Index<Line>> {
    let opened: usize = opening.expanded.iter().map(|&at| contracts[at as usize].contract.children.len() + 1).sum();
    let mut entries = Vec::with_capacity(contracts.len() + opened);
    let mut push = |payload: Line, placed: &Placed, kind: u16, observed: i32, states: Box<[(i32, u8)]>| {
        entries.push(Occurrence {
            id: u128::MAX - entries.len() as u128,
            date: placed.date,
            observed,
            kind,
            roles: placed.roles.clone(),
            subjects: Box::new([]),
            states,
            sort_keys: Box::new([]),
            text: placed.text.clone(),
            payload,
        });
    };
    for (at, placed) in contracts.iter().enumerate() {
        let at = at as u32;
        push(Line::Contract(at), placed, u16::from(placed.contract.request), placed.observed, placed.states.clone());
        if !opening.expanded.contains(&at) {
            continue;
        }
        let children = &placed.contract.children;
        let full = opening.full.contains(&at);
        let mut ranked: Vec<u32> = (0..children.len() as u32).collect();
        ranked.sort_unstable_by_key(|&place| Reverse(children[place as usize].rank()));
        let mut newer: BinaryHeap<i32> = BinaryHeap::with_capacity(CHILDREN_SHOWN + 1);
        for place in ranked {
            let child = &children[place as usize];
            let hidden = if !full && newer.len() == CHILDREN_SHOWN { newer.peek().copied() } else { None };
            if !full {
                newer.push(child.observed);
                if newer.len() > CHILDREN_SHOWN {
                    newer.pop();
                }
            }
            if hidden.is_some_and(|hidden| hidden <= child.observed) {
                continue;
            }
            push(Line::Event(at, place), placed, EVENT, child.observed, following(&placed.states, child.observed, hidden));
        }
        if let Some(first) = children.get(CHILDREN_SHOWN) {
            push(Line::More(at), placed, MORE, first.observed, following(&placed.states, first.observed, None));
        }
    }
    Index::new(entries, gaps, &LIVE)
}

#[derive(Default)]
struct Kept(HashMap<u64, Arc<Ledger>>);

pub fn forget(source: &Source) {
    *source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner) = Kept::default();
}

fn links_by_run(source: &Source, query: &Query<'_>) -> io::Result<BTreeMap<u32, Arc<RunLinks>>> {
    let mut out = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        if let std::collections::btree_map::Entry::Vacant(slot) = out.entry(stretch.run) {
            slot.insert(dashboard_base::queries::links(source, stretch.run)?);
        }
    }
    Ok(out)
}

fn refusal(ordinal: Option<i64>) -> &'static str {
    match ordinal.and_then(|ordinal| u32::try_from(ordinal).ok()) {
        Some(ordinal) => MarketEventRow { kind: 34, ordinal, ..MarketEventRow::default() }.delivery_unavailable_reason().unwrap_or("Delivery amount unavailable: unsupported refusal reason"),
        None => "Delivery amount unavailable: refusal reason not recorded",
    }
}

fn contract_id(contract: &Contract, incarnation: u32, row_key: u128) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let parts = [contract.seller.unwrap_or(-1) as u64, contract.agreement.map_or(u64::MAX, |id| id), u64::from(contract.request), u64::from(incarnation)];
    for part in parts {
        for byte in part.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    if contract.agreement.is_none() { row_key } else { (u128::from(u64::MAX) << 64) | u128::from(hash) }
}

fn states(contract: &Contract, unread: &Unread) -> Box<[(i32, u8)]> {
    let opened = if contract.request { MarketContractStatus::Requested } else { MarketContractStatus::Active };
    let closed = if contract.request { MarketContractStatus::RequestEnded } else { MarketContractStatus::Ended };
    let mut states: Vec<(i32, u8)> = Vec::new();
    let mut step = |hour: i32, status: MarketContractStatus| {
        let code = status.code();
        match states.last_mut() {
            Some(last) if last.0 == hour => last.1 = code,
            Some(last) if last.1 == code => return,
            _ => states.push((hour, code)),
        }
        if matches!(states.as_slice(), [.., (_, before), (_, last)] if before == last) {
            states.pop();
        }
    };
    let mut known = i32::MIN;
    for (place, child) in contract.children.iter().enumerate() {
        if contract.ended.is_some_and(|(_, seen)| seen <= child.observed) {
            step(child.observed, closed);
            continue;
        }
        known = known.max(child.hour);
        step(child.observed, if unread.since(known, child.observed) { MarketContractStatus::NotRecorded } else { opened });
        let next = contract.children.get(place + 1).map(|next| next.observed);
        if let Some(gap) = unread.after(child.observed)
            && next.is_none_or(|next| gap.first_hour < next)
        {
            step(gap.first_hour, MarketContractStatus::NotRecorded);
        }
    }
    states.into_boxed_slice()
}

fn scan(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Ledger> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid(format!("No declared log {LOG}")))?;
    let reader = Reader::new(query, declared)?;
    let columns = Columns::read()?;
    let links = links_by_run(source, query)?;
    let mut runs: Vec<u32> = query.timeline.stretches.iter().map(|stretch| stretch.run).collect();
    runs.sort_unstable();
    runs.dedup();
    let mut rows: Vec<(i32, u128, u8, i32, Vec<Cell>)> = Vec::new();
    let mut untold: Vec<(i32, usize)> = Vec::new();
    for run in runs {
        dashboard_text::metadata::check_cancel(cancelled)?;
        reader.each(run, |row| {
            if query.timeline.at(row.hour).is_none_or(|stretch| stretch.run != run) {
                return Ok(());
            }
            let Some(kind) = number(&row.cells, columns.kind).and_then(|kind| u8::try_from(kind).ok()) else { return Ok(()) };
            if !matches!(kind, 10 | 11 | 12 | 20 | 21 | 34) {
                return Ok(());
            }
            match number(&row.cells, columns.hour).and_then(|hour| i32::try_from(hour).ok()) {
                Some(hour) => rows.push((row.hour.max(hour), row.key, kind, hour, row.cells)),
                None => untold.push((row.hour, 1)),
            }
            Ok(())
        })?;
    }
    rows.sort_by_key(|(observed, key, ..)| (*observed, *key));
    let mut made: Vec<(Contract, u32, u128)> = Vec::new();
    let mut open: Opened = HashMap::new();
    let mut incarnations: HashMap<(Option<i64>, u64, bool), u32> = HashMap::new();
    let mut removed: HashMap<(Option<i64>, u64, u32), Vec<(u64, usize)>> = HashMap::new();
    let run_of = |hour: i32| query.timeline.at(hour).map(|stretch| stretch.run);
    for (observed, key, kind, hour, cells) in rows {
        let occurrence = number(&cells, columns.occurrence).map(|occurrence| occurrence as u64);
        let mode = number(&cells, columns.mode);
        if kind == 34 && mode.is_some_and(|mode| mode != 20) {
            continue;
        }
        let seller = number(&cells, columns.country);
        let buyer = number(&cells, columns.receiver);
        let variant = number(&cells, columns.variant).map(|variant| variant as u64).filter(|&variant| variant != 0);
        let Some(agreement) = number(&cells, columns.agreement).map(|agreement| agreement as u64) else {
            let child = Child { key, occurrence, hour, observed, kind: MarketContractEventKind::Delivery, variant, amount: None, total: None, reason: Some("The contract of this event was not recorded") };
            made.push((Contract { seller, buyer, buyer_later: None, agreement: None, request: false, requested: None, started: None, ended: None, children: vec![child] }, 0, key));
            continue;
        };
        let fresh = |request: bool| Contract { seller, buyer, buyer_later: None, agreement: Some(agreement), request, requested: None, started: None, ended: None, children: Vec::new() };
        let (at, child_kind) = match kind {
            10 => {
                let at = begin(&mut made, &mut open, &mut incarnations, fresh(true), key);
                made[at].0.requested = Some(hour);
                (at, MarketContractEventKind::RequestPosted)
            }
            11 => {
                let at = begin(&mut made, &mut open, &mut incarnations, fresh(false), key);
                made[at].0.started = Some(hour);
                (at, MarketContractEventKind::ContractStarted)
            }
            12 => {
                let request = match number(&cells, columns.flags) {
                    Some(0) => true,
                    Some(1) => false,
                    _ => {
                        untold.push((observed, 0));
                        continue;
                    }
                };
                let at = match still_open(&open, &made, (seller, agreement, request)) {
                    Some(at) => at,
                    None => begin(&mut made, &mut open, &mut incarnations, fresh(request), key),
                };
                made[at].0.ended = Some((hour, observed));
                if let (false, Some(run), Some(occurrence)) = (request, run_of(observed), occurrence) {
                    let ended = removed.entry((seller, agreement, run)).or_default();
                    ended.insert(ended.partition_point(|&(ending, _)| ending < occurrence), (occurrence, at));
                }
                (at, if request { MarketContractEventKind::RequestEnded } else { MarketContractEventKind::ContractEnded })
            }
            _ => {
                let ending = run_of(observed)
                    .zip(occurrence)
                    .and_then(|(run, occurrence)| removed.get(&(seller, agreement, run)).and_then(|ended| ended.get(ended.partition_point(|&(ending, _)| ending < occurrence))))
                    .map(|&(_, at)| at);
                let at = match (ending, still_open(&open, &made, (seller, agreement, false))) {
                    (Some(at), _) | (None, Some(at)) => at,
                    (None, None) => begin(&mut made, &mut open, &mut incarnations, fresh(false), key),
                };
                let child_kind = match kind {
                    20 => MarketContractEventKind::Delivery,
                    21 => MarketContractEventKind::Payment,
                    _ => MarketContractEventKind::DeliveryUnavailable,
                };
                (at, child_kind)
            }
        };
        let reason = (kind == 34).then(|| refusal(number(&cells, columns.ordinal)));
        let amount = if reason.is_some() || !matches!(kind, 20 | 21) { None } else { number(&cells, columns.amount) };
        let certified = kind == 20 && number(&cells, columns.flags).is_some_and(|flags| flags & CERTIFIED_TOTAL != 0);
        let total = if certified { number(&cells, columns.total) } else { None };
        let contract = &mut made[at].0;
        if contract.buyer_later.is_none()
            && contract.buyer.is_none_or(|raw| raw == 0)
            && let Some(raw) = buyer.filter(|&raw| raw != 0)
        {
            contract.buyer_later = Some((raw, hour, observed));
        }
        contract.children.push(Child { key, occurrence, hour, observed, kind: child_kind, variant, amount, total, reason });
    }
    untold.sort_unstable();
    let untold: Vec<(i32, [u32; 2])> = untold
        .into_iter()
        .scan([0u32; 2], |counts, (seen, reason)| {
            counts[reason] += 1;
            Some((seen, *counts))
        })
        .collect();
    let slot = |raw: Option<i64>, hour: i32| {
        let raw = u32::try_from(raw?).ok().filter(|&raw| raw != 0)?;
        let run = query.timeline.at(hour)?.run;
        links.get(&run)?.slot_at(raw, hour)
    };
    let unread = Unread::new(dashboard_base::products::log_page::missing(query, declared, query.timeline.first, query.timeline.last)?);
    let tags = Names::new(&query.holders);
    let mut contracts = Vec::with_capacity(made.len());
    for (contract, incarnation, first_key) in made {
        let Some(first) = contract.children.first() else { continue };
        let (date, observed) = (first.hour, first.observed);
        let opening = [slot(contract.seller, date), slot(contract.buyer, date)];
        let opening_text = opening.iter().flatten().filter_map(|&slot| tags.slot_tag(slot, date)).collect::<Vec<_>>().join(" ");
        let later = contract.buyer_later.map(|(raw, at, seen)| Role::dated(slot(Some(raw), at), at, seen));
        let roles: Box<[Role]> = opening.into_iter().map(|slot| Role::dated(slot, date, observed)).chain(later).collect();
        let party = |raw: Option<i64>, hour: i32| resolve_party(query, &links, &tags, raw, hour);
        let buyer_later = contract.buyer_later.map(|(raw, at, seen)| (seen, party(Some(raw), at)));
        let text = match &buyer_later {
            Some((seen, (Some(tag), _))) => SearchText::dated([(observed, opening_text.into_boxed_str()), (*seen, tag.as_str().into())]),
            _ => SearchText::from(opening_text),
        };
        contracts.push(Placed {
            key: contract_id(&contract, incarnation, first_key),
            date,
            observed,
            roles,
            states: states(&contract, &unread),
            text,
            measures: Measures::of(&contract.children),
            seller: party(contract.seller, date),
            buyer: party(contract.buyer, date),
            buyer_later,
            contract,
        });
    }
    contracts.sort_unstable_by(|a, b| b.date.cmp(&a.date).then(b.key.cmp(&a.key)));
    let mut by_key = HashMap::with_capacity(contracts.len());
    for (at, placed) in contracts.iter().enumerate() {
        if by_key.insert(placed.key, at as u32).is_some() {
            return Err(invalid("Two market contracts share one key"));
        }
    }
    let closed = Arc::new(lines(&contracts, &Opening::default(), unread.gaps.clone())?);
    Ok(Ledger { contracts, by_key, unread, untold, closed, opened: Mutex::default(), terms: Mutex::default() })
}

type Opened = HashMap<(Option<i64>, u64, bool), usize>;

fn begin(made: &mut Vec<(Contract, u32, u128)>, open: &mut Opened, incarnations: &mut HashMap<(Option<i64>, u64, bool), u32>, contract: Contract, key: u128) -> usize {
    let at = (contract.seller, contract.agreement.unwrap_or_default(), contract.request);
    let incarnation = incarnations.entry(at).or_default();
    *incarnation += 1;
    made.push((contract, *incarnation, key));
    open.insert(at, made.len() - 1);
    made.len() - 1
}

fn still_open(open: &Opened, made: &[(Contract, u32, u128)], at: (Option<i64>, u64, bool)) -> Option<usize> {
    open.get(&at).copied().filter(|&found| made[found].0.ended.is_none())
}

fn ledger(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Ledger>> {
    let kept = source.state::<Kept>();
    if let Some(found) = kept.lock().unwrap_or_else(PoisonError::into_inner).0.get(&branch) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(scan(source, query, cancelled)?);
    kept.lock().unwrap_or_else(PoisonError::into_inner).0.insert(branch, Arc::clone(&made));
    Ok(made)
}

fn resolve_party(query: &Query<'_>, links: &BTreeMap<u32, Arc<RunLinks>>, tags: &Names, raw: Option<i64>, hour: i32) -> (Option<String>, bool) {
    match raw {
        None => (None, true),
        Some(0) => (None, false),
        Some(value) => {
            let tag = query
                .timeline
                .at(hour)
                .and_then(|stretch| links.get(&stretch.run))
                .and_then(|held| u32::try_from(value).ok().and_then(|id| tags.tag_at(held, id, hour)))
                .map(str::to_owned);
            (tag, false)
        }
    }
}

struct Terms {
    equipment: Vec<(u64, i64)>,
    prices: Vec<(u64, i64)>,
}

fn read_terms(query: &Query<'_>, ledger: &Ledger, wanted: &BTreeMap<(u32, i32), Vec<(u32, u64, u8)>>, cancelled: &AtomicBool) -> io::Result<HashMap<TermsKey, Arc<Terms>>> {
    let recording = query.replay.recording();
    let mut out = HashMap::new();
    let mut seeker = None;
    for ((run, hour), keys) in wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let unread: Vec<(u32, u64, u8)> = {
            let held = ledger.terms.lock().unwrap_or_else(PoisonError::into_inner);
            let mut unread = Vec::new();
            for &(country, id, kind) in keys {
                match held.get(&(*hour, country, id, kind)) {
                    Some(Some(terms)) => {
                        out.insert((*hour, country, id, kind), Arc::clone(terms));
                    }
                    Some(None) => {}
                    None => unread.push((country, id, kind)),
                }
            }
            unread
        };
        if unread.is_empty() {
            continue;
        }
        let mut asked: Vec<u128> = unread.iter().map(|&(country, id, kind)| u128::from(country) << 72 | u128::from(id) << 8 | u128::from(kind)).collect();
        asked.sort_unstable();
        asked.dedup();
        let seeker = match &mut seeker {
            Some(seeker) => seeker,
            empty @ None => empty.insert(replay_lanes::at::Seeker::new(recording)?),
        };
        let read = [replay_lanes::at::Read { table: TableId::MarketContractTerms, columns: None, keys: replay_lanes::at::Keys::These(&asked) }];
        let frame = dashboard_query::cost::storage(|| seeker.frame(*run, *hour, &read))?;
        let Ok(rows) = frame.complete::<tables::MarketContractTerms>() else { continue };
        let mut found: HashMap<TermsKey, Option<Arc<Terms>>> = unread.iter().map(|&(country, id, kind)| ((*hour, country, id, kind), None)).collect();
        for row in rows {
            let items = |pool: &[replay_data::rows::MarketEquipment]| pool.iter().filter(|item| item.variant != 0).map(|item| (item.variant, item.amount_raw)).collect();
            let key = (*hour, row.country, row.id, row.kind);
            if let Some(slot) = found.get_mut(&key) {
                let terms = Arc::new(Terms { equipment: items(&row.equipment.items), prices: items(&row.prices.items) });
                out.insert(key, Arc::clone(&terms));
                *slot = Some(terms);
            }
        }
        ledger.terms.lock().unwrap_or_else(PoisonError::into_inner).extend(found);
    }
    Ok(out)
}

fn variant_names(
    query: &Query<'_>,
    namings: &BTreeMap<u32, Arc<names::Naming>>,
    context: &names::Context,
    wanted: BTreeMap<(u32, i32), BTreeSet<u64>>,
    cancelled: &AtomicBool,
) -> io::Result<HashMap<(i32, u64), (Option<String>, Option<String>)>> {
    let mut out = HashMap::new();
    if context.is_err() {
        return Ok(out);
    }
    let mut icons: HashMap<String, String> = HashMap::new();
    let mut sessions: BTreeMap<u32, names::NamingSession<'_>> = BTreeMap::new();
    for ((run, hour), variants) in wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let session = match sessions.entry(run) {
            std::collections::btree_map::Entry::Occupied(held) => held.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => slot.insert(namings.get(&run).ok_or_else(|| invalid(format!("Run {run} has no design names")))?.session()),
        };
        let variants: Vec<u64> = variants.into_iter().collect();
        let mut named = Named::default();
        session.name_variants(query.replay.recording(), hour, &variants, context, &mut named)?;
        for icon in named.icons() {
            icons.insert(icon.key, icon.image);
        }
        for &variant in &variants {
            let entry = named.variants.get(&variant);
            out.insert((hour, variant), (entry.and_then(|held| held.name.clone()), entry.and_then(|held| held.icon.as_ref()).and_then(|key| icons.get(key)).cloned()));
        }
    }
    Ok(out)
}

const UNNAMED: &str = "The recording does not name this design at the ledger's reads";

struct Prepared {
    ledger: Arc<Ledger>,
    index: Arc<Index<Line>>,
    selection: Option<Selection>,
    context: Arc<names::Context>,
    namings: Arc<BTreeMap<u32, Arc<names::Naming>>>,
    full: BTreeSet<u32>,
}

fn namings(source: &Source, query: &Query<'_>) -> io::Result<BTreeMap<u32, Arc<names::Naming>>> {
    let mut out = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        if let std::collections::btree_map::Entry::Vacant(slot) = out.entry(stretch.run) {
            slot.insert(names::naming(source, query.replay.recording(), stretch.run)?);
        }
    }
    Ok(out)
}

fn prepare(source: &Source, query: &Query<'_>, branch: u64, load: &LoadContext, params: &MarketLedgerTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let selection = dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    let context = names::context(source, load)?;
    let namings = if context.is_ok() { namings(source, query)? } else { BTreeMap::new() };
    let ledger = ledger(source, query, branch, cancelled)?;
    let opening = ledger.opening(params);
    let full = opening.full.clone();
    let index = ledger.index(opening, cancelled)?;
    Ok(Prepared { ledger, index, selection, context: Arc::new(context), namings: Arc::new(namings), full })
}

type NameOf<'a> = &'a dyn Fn(i32, u64) -> (Option<String>, Option<String>, Option<String>);

fn contract_row(ledger: &Ledger, shown: &Shown, placed: &Placed, hour: i32, terms: &HashMap<TermsKey, Arc<Terms>>, name_of: NameOf<'_>) -> MarketContractRow {
    let (contract, measures) = (&placed.contract, &placed.measures);
    let (date, at) = (placed.date, placed.observed);
    let read = contract.read_by(hour);
    let ended = contract.ended.filter(|&(_, seen)| seen <= hour).map(|(ended, _)| ended);
    let unseen = (!contract.request && contract.started.is_none()).then(|| START_UNSEEN.to_owned());
    let uncovered = unseen.or_else(|| ledger.unread.within(date, hour, ended));
    let found = contract.seller.and_then(|raw| u32::try_from(raw).ok()).zip(contract.agreement).and_then(|(seller, agreement)| terms.get(&(at, seller, agreement, u8::from(!contract.request))));
    let terms_missing = found.is_none().then(|| "The contract's terms were not recorded at its start".to_owned());
    let refused_all = measures.refused_all_by(read);
    let designs: HashMap<u64, Delivered> = measures.designs_by(read).iter().map(|design| (design.variant, design.by(read))).collect();
    let contracted: HashMap<u64, i64> = found.map(|terms| terms.equipment.iter().copied().collect()).unwrap_or_default();
    let mut order: Vec<u64> = found.map(|terms| terms.equipment.iter().map(|&(variant, _)| variant).collect()).unwrap_or_default();
    order.extend(measures.designs_by(read).iter().map(|design| design.variant).filter(|variant| !contracted.contains_key(variant)));
    let items: Vec<MarketContractItem> = order
        .iter()
        .map(|&variant| {
            let (name, icon, name_missing) = name_of(at, variant);
            let held = designs.get(&variant).copied().unwrap_or_default();
            let refusal = held.refused.into_iter().chain(refused_all).max_by_key(|&(rank, _)| rank);
            let (delivered, unavailable) = match held.certified {
                Some((total, rank)) => (Some(total.saturating_add(held.witnessed.unwrap_or(0))), refusal.filter(|&(refused, _)| refused > rank).map(|(_, why)| why.to_owned()).or_else(|| ledger.unread.within(rank.0.saturating_add(1), hour, ended))),
                None => {
                    let why = refusal.map(|(_, why)| why.to_owned()).or_else(|| uncovered.clone());
                    (held.witnessed.or_else(|| why.is_none().then_some(0)), why)
                }
            };
            MarketContractItem {
                variant: variant.to_string(),
                name,
                name_missing,
                icon,
                delivered_raw: delivered.map(|amount| amount.to_string()),
                contracted_raw: contracted.get(&variant).map(|amount| amount.to_string()),
                delivery_unavailable: unavailable,
            }
        })
        .collect();
    let quotes = found
        .map(|terms| {
            terms
                .prices
                .iter()
                .map(|&(variant, price)| {
                    let (name, _, name_missing) = name_of(at, variant);
                    MarketQuote { variant: variant.to_string(), name, name_missing, price_raw: price.to_string() }
                })
                .collect()
        })
        .unwrap_or_default();
    let settled = found.is_some() && items.iter().all(|item| item.delivery_unavailable.is_none());
    let unrecorded = measures.paid_unrecorded.is_some_and(|place| (place as usize) < read).then(|| "A payment's amount was not recorded".to_owned());
    let ((seller, seller_not_recorded), (buyer, buyer_not_recorded)) = (placed.seller.clone(), placed.buyer_at(hour).clone());
    MarketContractRow {
        key: placed.key.to_string(),
        hour: date,
        observed: at,
        exact: contract.requested.or(contract.started).is_some(),
        ordinal: shown.ordinal.to_string(),
        live: shown.live,
        request: contract.request,
        requested_hour: contract.requested,
        started_hour: contract.started,
        ended_hour: ended,
        status: shown.state.and_then(MarketContractStatus::of_code).unwrap_or(MarketContractStatus::NotRecorded),
        seller,
        seller_not_recorded,
        buyer,
        buyer_not_recorded,
        items,
        delivery_missing: refused_all.map(|(_, why)| why.to_owned()).or_else(|| uncovered.clone()).filter(|_| !settled),
        paid_raw: measures.paid_by(read).map(|paid| paid.to_string()),
        paid_missing: uncovered.or(unrecorded),
        quotes,
        terms_missing,
        events_total: read.to_string(),
    }
}

fn load_tail_at(prepared: &Prepared, query: &Query<'_>, params: &MarketLedgerTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<MarketLedgerTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (ledger, index) = (&prepared.ledger, &prepared.index);
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let states: Vec<u8> = match &params.statuses {
        Some(statuses) => statuses.iter().map(|status| status.code()).collect(),
        None => STATUSES.iter().map(|status| status.code()).collect(),
    };
    let chips = [
        Chip { kinds: Some(CONTRACTS.to_vec()), states: Some(vec![MarketContractStatus::Active.code()]) },
        Chip { kinds: Some(CONTRACTS.to_vec()), states: Some(LIVE.to_vec()) },
    ];
    let answer = index.ask(
        &query.holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            gap_end: None,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: None,
            states: Some(&states),
            search: params.search.as_deref(),
            order: Order::Date(Direction::Descending),
            live_first: true,
            offset,
            limit,
            chips: &chips,
        },
    )?;
    let mut warnings = Vec::new();
    if let Err(why) = prepared.context.as_ref() {
        warnings.push(why.clone());
    }
    let untold = ledger.untold.partition_point(|&(seen, _)| seen <= hour);
    let counts = untold.checked_sub(1).map_or([0; 2], |at| ledger.untold[at].1);
    for (reason, count) in UNTOLD.iter().zip(counts) {
        if count > 0 {
            warnings.push(format!("Market events not listed because they {reason}: {count}"));
        }
    }
    let run_of = |hour: i32| query.timeline.at(hour).map(|stretch| stretch.run);
    let mut terms_wanted: BTreeMap<(u32, i32), Vec<(u32, u64, u8)>> = BTreeMap::new();
    let mut variants_wanted: BTreeMap<(u32, i32), BTreeSet<u64>> = BTreeMap::new();
    let mut next_change = answer.next_change.into_iter().chain(ledger.untold.get(untold).map(|&(seen, _)| seen)).min();
    for shown in &answer.rows {
        let (Line::Contract(at) | Line::Event(at, _) | Line::More(at)) = index.get(shown.at).payload;
        let placed = &ledger.contracts[at as usize];
        let contract = &placed.contract;
        let read = contract.read_by(hour);
        next_change = next_change.into_iter().chain(contract.children.get(read).map(|next| next.observed)).min();
        match index.get(shown.at).payload {
            Line::Contract(_) => {
                let Some(run) = run_of(placed.observed) else { continue };
                if let (Some(seller), Some(agreement)) = (contract.seller.and_then(|raw| u32::try_from(raw).ok()), contract.agreement) {
                    terms_wanted.entry((run, placed.observed)).or_default().push((seller, agreement, u8::from(!contract.request)));
                }
                variants_wanted.entry((run, placed.observed)).or_default().extend(placed.measures.designs_by(read).iter().map(|design| design.variant));
            }
            Line::Event(_, place) => {
                let child = &contract.children[place as usize];
                if let (Some(variant), Some(run)) = (child.variant, run_of(child.hour)) {
                    variants_wanted.entry((run, child.hour)).or_default().insert(variant);
                }
            }
            Line::More(_) => {}
        }
    }
    let terms = read_terms(query, ledger, &terms_wanted, cancelled)?;
    for (&(when, ..), found) in &terms {
        if let Some(run) = run_of(when) {
            variants_wanted.entry((run, when)).or_default().extend(found.equipment.iter().chain(&found.prices).map(|&(variant, _)| variant));
        }
    }
    let named = variant_names(query, &prepared.namings, &prepared.context, variants_wanted, cancelled)?;
    let name_of = |when: i32, variant: u64| -> (Option<String>, Option<String>, Option<String>) {
        if let Err(why) = prepared.context.as_ref() {
            return (None, None, Some(why.clone()));
        }
        match named.get(&(when, variant)) {
            Some((Some(name), icon)) => (Some(name.clone()), icon.clone(), None),
            Some((None, icon)) => (None, icon.clone(), Some(UNNAMED.into())),
            None => (None, None, Some(UNNAMED.into())),
        }
    };
    let rows = answer
        .rows
        .iter()
        .map(|shown| match index.get(shown.at).payload {
            Line::Contract(at) => MarketLedgerLine::Contract(contract_row(ledger, shown, &ledger.contracts[at as usize], hour, &terms, &name_of)),
            Line::Event(at, place) => {
                let placed = &ledger.contracts[at as usize];
                let child = &placed.contract.children[place as usize];
                let (equipment, _, equipment_missing) = match child.variant {
                    Some(variant) => name_of(child.hour, variant),
                    None => (None, None, None),
                };
                MarketLedgerLine::Event(MarketContractEvent {
                    key: child.key.to_string(),
                    contract: placed.key.to_string(),
                    seller: placed.seller.0.clone(),
                    buyer: placed.buyer_at(hour).0.clone(),
                    hour: child.hour,
                    kind: child.kind,
                    variant: child.variant.map(|variant| variant.to_string()),
                    equipment,
                    equipment_missing,
                    amount_raw: child.amount.map(|amount| amount.to_string()),
                    amount_not_recorded: matches!(child.kind, MarketContractEventKind::Delivery | MarketContractEventKind::Payment) && child.amount.is_none(),
                    reason: child.reason.map(str::to_owned),
                })
            }
            Line::More(at) => {
                let placed = &ledger.contracts[at as usize];
                let total = placed.contract.read_by(hour);
                let hidden = if prepared.full.contains(&at) { 0 } else { total.saturating_sub(CHILDREN_SHOWN) };
                MarketLedgerLine::More(MarketEventsMore { key: format!("more:{}", placed.key), hour: placed.date, contract: placed.key.to_string(), total: total.to_string(), hidden: hidden.to_string() })
            }
        })
        .collect();
    let reply = MarketLedgerTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.chips[1].to_string(),
        counts: occurrences::counts(&answer, &["active"]),
        rows,
        missing: answer.gaps,
        next_change,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct MarketLedgerTail;

impl Product for MarketLedgerTail {
    const KEY: &'static str = "market-ledger-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = MarketLedgerTailParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, search, offset, limit, statuses, expanded, "expanded[]", full, "full[]"];
    type Reply = MarketLedgerTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MarketLedgerTailParams::decl(),
            MarketContractStatus::decl(),
            MarketContractEventKind::decl(),
            MarketContractItem::decl(),
            MarketQuote::decl(),
            MarketContractEvent::decl(),
            MarketContractRow::decl(),
            MarketEventsMore::decl(),
            MarketLedgerLine::decl(),
            MarketLedgerTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, branch, load, params, cancelled)?;
        load_tail_at(&prepared, &query, params, hour, cancelled)
    }
}

struct Owned {
    replay: Arc<Replay>,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid market-ledger-tail panel")?.remove("panel");
    let params: MarketLedgerTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid market-ledger-tail panel: {error}"))?;
    MarketLedgerTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let (finish, prepared) = dashboard_base::products::Finish::open(source, load, MarketLedgerTail::CONTENT_DEPENDENT, || prepare(source, &query, branch, load, &params, &AtomicBool::new(false)));
    let prepared = prepared.map_err(|error| error.to_string())?;
    let owned = Owned { replay: source.replay(), timeline: Arc::clone(&query.timeline), holders: Arc::clone(&query.holders) };
    Ok(occurrences::occurrence_panel(source.tables(), finish, move |hour, cancelled| {
        let query = Query { replay: &owned.replay, timeline: Arc::clone(&owned.timeline), holders: Arc::clone(&owned.holders) };
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_tail_at(&prepared, &query, &params, hour, cancelled)
    }))
}

#[cfg(test)]
mod tests;
