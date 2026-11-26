use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply, OccurrenceSort},
    },
    queries::country_ids::{Names, RunLinks},
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::{
        Cell, Reader,
        occurrence::{Ask, Chip, Index, Occurrence, PlacedGap, Role, Selection},
    },
    series::{MissingSpan, ranked::GroupSelection},
    timeline::Timeline,
};
use dashboard_world::products::equipment_names::{self as names, Named};
use replay::campaign::Replay;
use replay_data::rows::{MarketEventRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    agreement: usize,
    country: usize,
    receiver: usize,
    kind: usize,
    mode: usize,
    ordinal: usize,
    variant: usize,
    amount: usize,
}

impl Columns {
    fn read() -> io::Result<Self> {
        let field = |name| dashboard_declare::field(TableId::MarketEvents, name);
        Ok(Self {
            agreement: field("agreement")?,
            country: field("country")?,
            receiver: field("receiver")?,
            kind: field("kind")?,
            mode: field("mode")?,
            ordinal: field("ordinal")?,
            variant: field("variant")?,
            amount: field("amount_raw")?,
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
    #[doc = " Delivered by the hour asked; null with `deliveryUnavailable` when not every delivery is known."]
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
    #[doc = " The hour the request was posted or the contract accepted; the first event seen when neither was."]
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
    pub paid_raw: Option<String>,
    #[doc = " Why the paid total is not known by the hour asked; null with `paidRaw` null means nothing paid yet."]
    pub paid_missing: Option<String>,
    pub quotes: Vec<MarketQuote>,
    pub terms_missing: Option<String>,
    #[doc = " The newest events by the hour asked, at most 32, for an expanded contract only; `eventsTotal` counts them all."]
    pub events: Vec<MarketContractEvent>,
    pub events_total: String,
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
    pub sort: Option<OccurrenceSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
    #[doc = " The statuses listed, each at the hour asked; every one when none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub statuses: Option<Vec<MarketContractStatus>>,
    #[doc = " The contracts, by key, whose events are listed; the others list none, and `eventsTotal` counts them."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub expanded: Option<Vec<String>>,
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
    pub rows: Vec<MarketContractRow>,
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
    hour: i32,
    kind: MarketContractEventKind,
    variant: Option<u64>,
    amount: Option<i64>,
    reason: Option<&'static str>,
}

#[derive(Clone, Debug)]
struct Contract {
    seller: Option<i64>,
    buyer: Option<i64>,
    agreement: Option<u64>,
    request: bool,
    requested: Option<i32>,
    started: Option<i32>,
    ended: Option<i32>,
    children: Vec<Child>,
}

type TermsKey = (i32, u32, u64, u8);

struct Ledger {
    index: Index<Contract>,
    terms: Mutex<HashMap<TermsKey, Option<Arc<Terms>>>>,
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

fn scan(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Ledger> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid(format!("No declared log {LOG}")))?;
    let reader = Reader::new(query, declared)?;
    let columns = Columns::read()?;
    let links = links_by_run(source, query)?;
    let mut runs: Vec<u32> = query.timeline.stretches.iter().map(|stretch| stretch.run).collect();
    runs.sort_unstable();
    runs.dedup();
    let mut rows: Vec<(i32, u128, u8, Vec<Cell>)> = Vec::new();
    for run in runs {
        dashboard_text::metadata::check_cancel(cancelled)?;
        reader.each(run, |row| {
            if query.timeline.at(row.hour).is_none_or(|stretch| stretch.run != run) {
                return Ok(());
            }
            let Some(kind) = number(&row.cells, columns.kind).and_then(|kind| u8::try_from(kind).ok()) else { return Ok(()) };
            if matches!(kind, 10 | 11 | 12 | 20 | 21 | 34) {
                rows.push((row.hour, row.key, kind, row.cells));
            }
            Ok(())
        })?;
    }
    rows.sort_by_key(|(hour, key, ..)| (*hour, *key));
    let mut made: Vec<(Contract, u32, u128, i32)> = Vec::new();
    let mut open: Opened = HashMap::new();
    let mut incarnations: HashMap<(Option<i64>, u64, bool), u32> = HashMap::new();
    for (hour, key, kind, cells) in rows {
        let mode = number(&cells, columns.mode);
        if kind == 34 && mode.is_some_and(|mode| mode != 20) {
            continue;
        }
        let seller = number(&cells, columns.country);
        let buyer = number(&cells, columns.receiver);
        let variant = number(&cells, columns.variant).map(|variant| variant as u64).filter(|&variant| variant != 0);
        let Some(agreement) = number(&cells, columns.agreement).map(|agreement| agreement as u64) else {
            let child = Child { key, hour, kind: MarketContractEventKind::Delivery, variant, amount: None, reason: Some("The contract of this event was not recorded") };
            made.push((Contract { seller, buyer, agreement: None, request: false, requested: None, started: None, ended: None, children: vec![child] }, 0, key, hour));
            continue;
        };
        let fresh = |request: bool| Contract { seller, buyer, agreement: Some(agreement), request, requested: None, started: None, ended: None, children: Vec::new() };
        let (at, child_kind) = match kind {
            10 => {
                let at = begin(&mut made, &mut open, &mut incarnations, fresh(true), key, hour);
                made[at].0.requested = Some(hour);
                (at, MarketContractEventKind::RequestPosted)
            }
            11 => {
                let at = begin(&mut made, &mut open, &mut incarnations, fresh(false), key, hour);
                made[at].0.started = Some(hour);
                (at, MarketContractEventKind::ContractStarted)
            }
            12 => match (still_open(&open, &made, (seller, agreement, true)), still_open(&open, &made, (seller, agreement, false))) {
                (Some(at), _) => {
                    made[at].0.ended = Some(hour);
                    (at, MarketContractEventKind::RequestEnded)
                }
                (None, Some(at)) => {
                    made[at].0.ended = Some(hour);
                    (at, MarketContractEventKind::ContractEnded)
                }
                (None, None) => {
                    let at = begin(&mut made, &mut open, &mut incarnations, fresh(false), key, hour);
                    made[at].0.ended = Some(hour);
                    (at, MarketContractEventKind::ContractEnded)
                }
            },
            _ => {
                let at = match still_open(&open, &made, (seller, agreement, false)) {
                    Some(at) => at,
                    None => begin(&mut made, &mut open, &mut incarnations, fresh(false), key, hour),
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
        let contract = &mut made[at].0;
        if contract.seller.is_none_or(|raw| raw == 0) {
            contract.seller = seller;
        }
        if contract.buyer.is_none_or(|raw| raw == 0) {
            contract.buyer = buyer;
        }
        contract.children.push(Child { key, hour, kind: child_kind, variant, amount, reason });
    }
    let slot = |raw: Option<i64>, hour: i32| {
        let raw = u32::try_from(raw?).ok().filter(|&raw| raw != 0)?;
        let run = query.timeline.at(hour)?.run;
        links.get(&run)?.slot_at(raw, hour)
    };
    let gaps = dashboard_base::products::log_page::missing(query, declared, query.timeline.first, query.timeline.last)?;
    let tags = Names::new(&query.holders);
    let mut occurrences = Vec::with_capacity(made.len());
    for (contract, incarnation, first_key, first_hour) in made {
        let date = contract.requested.or(contract.started).unwrap_or(first_hour);
        let observed = contract.children.first().map_or(first_hour, |child| child.hour);
        let roles = [Role { slot: slot(contract.seller, date), at: date }, Role { slot: slot(contract.buyer, date), at: date }];
        let text = roles.iter().filter_map(|role| role.slot.and_then(|slot| tags.slot_tag(slot, role.at))).collect::<Vec<_>>().join(" ");
        let opened = if contract.request { MarketContractStatus::Requested } else { MarketContractStatus::Active };
        let closed = if contract.request { MarketContractStatus::RequestEnded } else { MarketContractStatus::Ended };
        let mut states: Vec<(i32, u8)> = Vec::new();
        for child in &contract.children {
            let now = if contract.ended.is_some_and(|ended| ended <= child.hour) { closed } else { opened };
            match states.last_mut() {
                Some(last) if last.0 == child.hour => last.1 = now.code(),
                _ => states.push((child.hour, now.code())),
            }
        }
        occurrences.push(Occurrence {
            id: contract_id(&contract, incarnation, first_key),
            date: date.min(observed),
            observed,
            kind: u16::from(contract.request),
            roles: roles.into(),
            subjects: Box::new([]),
            states: states.into_boxed_slice(),
            sort_keys: Box::new([]),
            text: text.into_boxed_str(),
            payload: contract,
        });
    }
    Ok(Ledger { index: Index::new(occurrences, gaps, &LIVE)?, terms: Mutex::default() })
}

type Opened = HashMap<(Option<i64>, u64, bool), usize>;

fn begin(made: &mut Vec<(Contract, u32, u128, i32)>, open: &mut Opened, incarnations: &mut HashMap<(Option<i64>, u64, bool), u32>, contract: Contract, key: u128, hour: i32) -> usize {
    let at = (contract.seller, contract.agreement.unwrap_or_default(), contract.request);
    let incarnation = incarnations.entry(at).or_default();
    *incarnation += 1;
    made.push((contract, *incarnation, key, hour));
    open.insert(at, made.len() - 1);
    made.len() - 1
}

fn still_open(open: &Opened, made: &[(Contract, u32, u128, i32)], at: (Option<i64>, u64, bool)) -> Option<usize> {
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

fn gap_within(gaps: &[MissingSpan], from: i32, to: i32) -> Option<String> {
    gaps.iter()
        .find(|gap| gap.first_hour <= to && gap.last_hour >= from)
        .map(|gap| format!("Market events were not recorded from {} to {}: {}", gap.first_hour.max(from), gap.last_hour.min(to), gap.reason))
}

struct Prepared {
    ledger: Arc<Ledger>,
    selection: Option<Selection>,
    links: Arc<BTreeMap<u32, Arc<RunLinks>>>,
    context: Arc<names::Context>,
    namings: Arc<BTreeMap<u32, Arc<names::Naming>>>,
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
    Ok(Prepared {
        ledger: ledger(source, query, branch, cancelled)?,
        selection,
        links: Arc::new(links_by_run(source, query)?),
        context: Arc::new(context),
        namings: Arc::new(namings),
    })
}

fn load_tail_at(prepared: &Prepared, query: &Query<'_>, params: &MarketLedgerTailParams, hour: i32, cancelled: &AtomicBool) -> io::Result<MarketLedgerTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let index = &prepared.ledger.index;
    let (order, newest) = occurrences::order(params.sort.as_ref(), &[])?;
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let states: Option<Vec<u8>> = params.statuses.as_ref().map(|statuses| statuses.iter().map(|status| status.code()).collect());
    let chips = [Chip { kinds: None, states: Some(vec![MarketContractStatus::Active.code()]) }];
    let expanded: BTreeSet<&str> = params.expanded.iter().flatten().map(String::as_str).collect();
    let answer = index.ask(
        &query.holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: None,
            states: states.as_deref(),
            search: params.search.as_deref(),
            order,
            live_first: newest,
            offset,
            limit,
            chips: &chips,
        },
    )?;
    let mut warnings = Vec::new();
    if let Err(why) = prepared.context.as_ref() {
        warnings.push(why.clone());
    }
    let run_of = |hour: i32| query.timeline.at(hour).map(|stretch| stretch.run);
    let mut terms_wanted: BTreeMap<(u32, i32), Vec<(u32, u64, u8)>> = BTreeMap::new();
    let mut variants_wanted: BTreeMap<(u32, i32), BTreeSet<u64>> = BTreeMap::new();
    for shown in &answer.rows {
        let occurrence = index.get(shown.at);
        let contract = &occurrence.payload;
        if let (Some(run), Some(seller), Some(agreement)) = (run_of(occurrence.date), contract.seller.and_then(|raw| u32::try_from(raw).ok()), contract.agreement) {
            terms_wanted.entry((run, occurrence.date)).or_default().push((seller, agreement, u8::from(!contract.request)));
        }
        let listed = expanded.contains(occurrence.id.to_string().as_str());
        for child in contract.children.iter().filter(|child| child.hour <= hour) {
            let Some(variant) = child.variant else { continue };
            if let Some(run) = run_of(occurrence.date) {
                variants_wanted.entry((run, occurrence.date)).or_default().insert(variant);
            }
            if let (true, Some(run)) = (listed, run_of(child.hour)) {
                variants_wanted.entry((run, child.hour)).or_default().insert(variant);
            }
        }
    }
    let terms = read_terms(query, &prepared.ledger, &terms_wanted, cancelled)?;
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
    let tags = Names::new(&query.holders);
    let mut rows = Vec::with_capacity(answer.rows.len());
    for shown in &answer.rows {
        let occurrence = index.get(shown.at);
        let contract = &occurrence.payload;
        let date = occurrence.date;
        let children: Vec<&Child> = contract.children.iter().filter(|child| child.hour <= hour).collect();
        let ended = contract.ended.filter(|ended| *ended <= hour);
        let covered_to = ended.unwrap_or(hour);
        let uncovered = gap_within(index.gaps(), date, covered_to);
        let found = contract.seller.and_then(|raw| u32::try_from(raw).ok()).zip(contract.agreement).and_then(|(seller, agreement)| terms.get(&(date, seller, agreement, u8::from(!contract.request))));
        let terms_missing = found.is_none().then(|| "The contract's terms were not recorded at its start".to_owned());
        let mut order: Vec<u64> = found.map(|terms| terms.equipment.iter().map(|&(variant, _)| variant).collect()).unwrap_or_default();
        let mut delivered: HashMap<u64, i64> = HashMap::new();
        let mut refused: HashMap<u64, String> = HashMap::new();
        let mut refused_all: Option<String> = None;
        let mut paid: i64 = 0;
        let mut paid_any = false;
        let mut paid_missing = uncovered.clone();
        for child in &children {
            match child.kind {
                MarketContractEventKind::Delivery => match (child.variant, child.amount) {
                    (Some(variant), Some(amount)) => {
                        if !order.contains(&variant) {
                            order.push(variant);
                        }
                        *delivered.entry(variant).or_default() += amount;
                    }
                    (Some(variant), None) => {
                        refused.entry(variant).or_insert_with(|| child.reason.unwrap_or("A delivery's amount was not recorded").to_owned());
                    }
                    (None, _) => {
                        refused_all.get_or_insert_with(|| child.reason.unwrap_or("A delivery's design was not recorded").to_owned());
                    }
                },
                MarketContractEventKind::DeliveryUnavailable => match child.variant {
                    Some(variant) => {
                        if !order.contains(&variant) {
                            order.push(variant);
                        }
                        refused.entry(variant).or_insert_with(|| child.reason.unwrap_or_default().to_owned());
                    }
                    None => {
                        refused_all.get_or_insert_with(|| child.reason.unwrap_or_default().to_owned());
                    }
                },
                MarketContractEventKind::Payment => match child.amount {
                    Some(amount) => {
                        paid = paid.saturating_add(amount);
                        paid_any = true;
                    }
                    None => {
                        paid_missing.get_or_insert_with(|| "A payment's amount was not recorded".to_owned());
                    }
                },
                _ => {}
            }
        }
        let contracted: HashMap<u64, i64> = found.map(|terms| terms.equipment.iter().copied().collect()).unwrap_or_default();
        let items = order
            .iter()
            .map(|&variant| {
                let (name, icon, name_missing) = name_of(date, variant);
                let unavailable = refused.get(&variant).cloned().or_else(|| refused_all.clone()).or_else(|| uncovered.clone());
                MarketContractItem {
                    variant: variant.to_string(),
                    name,
                    name_missing,
                    icon,
                    delivered_raw: unavailable.is_none().then(|| delivered.get(&variant).copied().unwrap_or(0).to_string()),
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
                        let (name, _, name_missing) = name_of(date, variant);
                        MarketQuote { variant: variant.to_string(), name, name_missing, price_raw: price.to_string() }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let listed = expanded.contains(occurrence.id.to_string().as_str());
        let events = children
            .iter()
            .rev()
            .take(if listed { CHILDREN_SHOWN } else { 0 })
            .map(|child| {
                let (equipment, _, equipment_missing) = match child.variant {
                    Some(variant) => name_of(child.hour, variant),
                    None => (None, None, None),
                };
                MarketContractEvent {
                    key: child.key.to_string(),
                    hour: child.hour,
                    kind: child.kind,
                    variant: child.variant.map(|variant| variant.to_string()),
                    equipment,
                    equipment_missing,
                    amount_raw: child.amount.map(|amount| amount.to_string()),
                    amount_not_recorded: matches!(child.kind, MarketContractEventKind::Delivery | MarketContractEventKind::Payment) && child.amount.is_none(),
                    reason: child.reason.map(str::to_owned),
                }
            })
            .collect();
        let (seller, seller_not_recorded) = resolve_party(query, &prepared.links, &tags, contract.seller, date);
        let (buyer, buyer_not_recorded) = resolve_party(query, &prepared.links, &tags, contract.buyer, date);
        let status = shown.state.and_then(MarketContractStatus::of_code).unwrap_or(MarketContractStatus::NotRecorded);
        rows.push(MarketContractRow {
            key: occurrence.id.to_string(),
            hour: date,
            observed: occurrence.observed,
            exact: contract.requested.or(contract.started).is_some(),
            ordinal: shown.ordinal.to_string(),
            live: shown.live,
            request: contract.request,
            requested_hour: contract.requested,
            started_hour: contract.started,
            ended_hour: ended,
            status,
            seller,
            seller_not_recorded,
            buyer,
            buyer_not_recorded,
            items,
            delivery_missing: refused_all.clone().or_else(|| uncovered.clone()),
            paid_raw: (paid_missing.is_none() && paid_any).then(|| paid.to_string()),
            paid_missing,
            quotes,
            terms_missing,
            events,
            events_total: children.len().to_string(),
        });
    }
    let reply = MarketLedgerTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts: occurrences::counts(&answer, &["active"]),
        rows,
        missing: answer.gaps,
        next_change: answer.next_change,
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
        dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, search, sort, offset, limit, statuses, expanded, "expanded[]"];
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
    let prepared = prepare(source, &query, branch, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned { replay: source.replay(), timeline: Arc::clone(&query.timeline), holders: Arc::clone(&query.holders) };
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        let query = Query { replay: &owned.replay, timeline: Arc::clone(&owned.timeline), holders: Arc::clone(&owned.holders) };
        let hour = occurrences::cutoff(&query, Some(hour))?;
        load_tail_at(&prepared, &query, &params, hour, cancelled)
    }))
}

#[cfg(test)]
mod tests;
