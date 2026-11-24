use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        log_page::{self as logs, LogCaches, TailPanelReply, dated_tail_panel, quiet_tail_missing},
    },
    queries::country_ids::Names,
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    log::{self, Cell, FacetFilter},
    series::{MissingSpan, PAGE_LIMIT, hour_of},
};
use dashboard_world::products::equipment_names::{self as names, Named};
use replay_data::rows::{MarketEventRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const LOG: &str = "market-events";

pub const KINDS: [i64; 10] = [3, 4, 5, 6, 10, 11, 12, 20, 21, 34];

pub const CONTRACT_KINDS: [i64; 6] = [10, 11, 12, 20, 21, 34];

pub const COLUMNS: [&str; 15] = [
    "id",
    "occurrence",
    "kind",
    "mode",
    "contract",
    "seller",
    "buyer",
    "variant",
    "equipment",
    "equipmentMissing",
    "amountRaw",
    "auxiliaryRaw",
    "totalRaw",
    "lostTotalRaw",
    "reason",
];

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MarketContractEventKind {
    RequestPosted,
    ContractStarted,
    ContractEnded,
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
    pub delivered_raw: Option<String>,
    pub contracted_raw: Option<String>,
    pub delivery_unavailable: Option<String>,
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
    pub key: String,
    pub hour: i32,
    pub started_hour: Option<i32>,
    pub ended_hour: Option<i32>,
    pub status: MarketContractStatus,
    pub seller: Option<String>,
    pub seller_not_recorded: bool,
    pub buyer: Option<String>,
    pub buyer_not_recorded: bool,
    pub items: Vec<MarketContractItem>,
    pub paid_raw: Option<String>,
    pub paid_not_recorded: bool,
    pub agreed_price_raw: Option<String>,
    pub events: Vec<MarketContractEvent>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketLedgerTailReply {
    pub branch_id: String,
    pub base_ordinal: String,
    pub first_ordinal: String,
    pub offset: String,
    pub total: String,
    pub rows: Vec<MarketContractRow>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MarketLedgerTailReply);

impl TailPanelReply for MarketLedgerTailReply {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn missing(&self) -> &[MissingSpan] {
        &self.missing
    }

    fn set_missing(&mut self, missing: Vec<MissingSpan>) {
        self.missing = missing;
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLedgerTailParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    pub limit: u32,
}

fn validate_facets(facets: &[FacetFilter]) -> Result<(), &'static str> {
    if let Some(kinds) = facets.iter().find(|facet| facet.name == "kind")
        && (kinds.values.is_empty() || kinds.values.iter().any(|kind| !CONTRACT_KINDS.contains(kind)))
    {
        return Err("Invalid market contract event kind");
    }
    Ok(())
}

fn effective_facets(facets: &[FacetFilter]) -> Vec<FacetFilter> {
    let mut out = Vec::with_capacity(facets.len() + 1);
    let mut has_kind = false;
    for facet in facets {
        if facet.name == "kind" {
            has_kind = true;
            out.push(facet.clone());
        } else if facet.name == "country|receiver" {
            let mut dedup = facet.clone();
            dedup.deduplicate = true;
            out.push(dedup);
        } else {
            out.push(facet.clone());
        }
    }
    if !has_kind {
        out.push(FacetFilter { name: "kind".into(), values: CONTRACT_KINDS.to_vec(), ..Default::default() });
    }
    out
}

fn links_by_run(source: &Source, query: &Query<'_>) -> io::Result<BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>> {
    let mut out = BTreeMap::new();
    for stretch in &query.timeline.stretches {
        if let std::collections::btree_map::Entry::Vacant(slot) = out.entry(stretch.run) {
            slot.insert(dashboard_base::queries::links(source, stretch.run)?);
        }
    }
    Ok(out)
}

#[derive(Clone)]
struct RawEvent {
    key: u128,
    hour: i32,
    kind: MarketContractEventKind,
    seller_raw: Option<i64>,
    buyer_raw: Option<i64>,
    variant: Option<u64>,
    amount_raw: Option<i64>,
    reason: Option<String>,
}

struct ContractAcc {
    agreement: Option<u64>,
    key: String,
    last_hour: i32,
    first_hour: i32,
    started_hour: Option<i32>,
    ended_hour: Option<i32>,
    requested: bool,
    started: bool,
    ended: bool,
    seller_raw: Option<i64>,
    buyer_raw: Option<i64>,
    seller_hour: i32,
    buyer_hour: i32,
    events: Vec<RawEvent>,
}

fn parse_event(row: &log::Row, columns: &Columns) -> Option<(Option<u64>, RawEvent)> {
    let kind_raw = number(&row.cells, columns.kind).and_then(|kind| u8::try_from(kind).ok())?;
    let mode = number(&row.cells, columns.mode).and_then(|mode| u8::try_from(mode).ok());
    let kind = match kind_raw {
        10 => MarketContractEventKind::RequestPosted,
        11 => MarketContractEventKind::ContractStarted,
        12 => MarketContractEventKind::ContractEnded,
        20 => MarketContractEventKind::Delivery,
        21 => MarketContractEventKind::Payment,
        34 if mode == Some(20) || mode.is_none() => MarketContractEventKind::DeliveryUnavailable,
        _ => return None,
    };
    let ordinal = number(&row.cells, columns.ordinal).and_then(|ordinal| u32::try_from(ordinal).ok());
    let reason = if kind == MarketContractEventKind::DeliveryUnavailable {
        Some(match ordinal {
            Some(ordinal) => MarketEventRow { kind: 34, ordinal, ..MarketEventRow::default() }
                .delivery_unavailable_reason()
                .unwrap_or("Delivery amount unavailable: unsupported refusal reason")
                .to_owned(),
            None => "Delivery amount unavailable: refusal reason not recorded".to_owned(),
        })
    } else {
        None
    };
    let agreement = number(&row.cells, columns.agreement).map(|agreement| agreement as u64);
    let variant = number(&row.cells, columns.variant).map(|variant| variant as u64).filter(|&variant| variant != 0);
    let amount_raw = if reason.is_some() { None } else { number(&row.cells, columns.amount) };
    Some((
        agreement,
        RawEvent {
            key: row.key,
            hour: row.hour,
            kind,
            seller_raw: number(&row.cells, columns.country),
            buyer_raw: number(&row.cells, columns.receiver),
            variant,
            amount_raw,
            reason,
        },
    ))
}

fn resolve_party(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    tags: &Names,
    raw: Option<i64>,
    hour: i32,
) -> (Option<String>, bool) {
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

struct TermsInfo {
    equipment: Vec<(u64, i64)>,
    agreed_price_raw: Option<i64>,
}

fn enrich_contracts(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    context: &names::Context,
    contracts: Vec<ContractAcc>,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<MarketContractRow>, Vec<String>)> {
    let tags = Names::new(&query.holders);
    let mut warnings = Vec::new();
    if let Err(why) = context {
        warnings.push(why.clone());
    }

    let mut terms_by_run_hour: BTreeMap<(u32, i32), Vec<u64>> = BTreeMap::new();
    let mut contract_probe: Vec<Option<(u32, i32, u64)>> = Vec::with_capacity(contracts.len());
    for acc in &contracts {
        let probe_hour = acc.started_hour.unwrap_or(acc.first_hour);
        let probe = acc
            .agreement
            .and_then(|id| query.timeline.at(probe_hour).map(|stretch| (stretch.run, probe_hour, id)));
        if let Some((run, hour, id)) = probe {
            terms_by_run_hour.entry((run, hour)).or_default().push(id);
        }
        contract_probe.push(probe);
    }

    let recording = query.replay.recording();
    let mut seeker: Option<replay_lanes::at::Seeker<'_>> = None;
    let mut terms_map: HashMap<(u32, i32, u64), TermsInfo> = HashMap::new();
    for ((run, hour), ids) in &terms_by_run_hour {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held_seeker = match seeker.as_mut() {
            Some(held) => held,
            None => seeker.insert(replay_lanes::at::Seeker::new(recording)?),
        };
        let frame = dashboard_query::cost::storage(|| held_seeker.frame(*run, *hour, &[replay_lanes::at::Read::all(TableId::MarketContractTerms)]))?;
        if let Ok(rows) = frame.complete::<tables::MarketContractTerms>() {
            let wanted: BTreeSet<u64> = ids.iter().copied().collect();
            for row in rows {
                if !wanted.contains(&row.id) {
                    continue;
                }
                let equipment: Vec<(u64, i64)> = row.equipment.items.iter().filter(|item| item.variant != 0).map(|item| (item.variant, item.amount_raw)).collect();
                let agreed_price_raw = (!row.prices.items.is_empty()).then(|| row.prices.items.iter().fold(0_i64, |sum, item| sum.saturating_add(item.amount_raw)));
                terms_map.insert((*run, *hour, row.id), TermsInfo { equipment, agreed_price_raw });
            }
        }
    }

    let mut variants_by_run_hour: BTreeMap<(u32, i32), BTreeSet<u64>> = BTreeMap::new();
    for (idx, acc) in contracts.iter().enumerate() {
        if let Some((run, hour, id)) = contract_probe[idx]
            && let Some(info) = terms_map.get(&(run, hour, id))
        {
            for &(variant, _) in &info.equipment {
                variants_by_run_hour.entry((run, hour)).or_default().insert(variant);
            }
        }
        for event in &acc.events {
            if let Some(variant) = event.variant
                && let Some(stretch) = query.timeline.at(event.hour)
            {
                variants_by_run_hour.entry((stretch.run, event.hour)).or_default().insert(variant);
            }
        }
    }

    let mut resolved_names: HashMap<(u32, i32, u64), (Option<String>, Option<String>)> = HashMap::new();
    let mut icon_images: HashMap<String, String> = HashMap::new();
    if context.is_ok() {
        for ((run, hour), variant_set) in variants_by_run_hour {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if variant_set.is_empty() {
                continue;
            }
            let held_seeker = match seeker.as_mut() {
                Some(held) => held,
                None => seeker.insert(replay_lanes::at::Seeker::new(recording)?),
            };
            let variants: Vec<u64> = variant_set.into_iter().collect();
            let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
            let mut reads: Vec<replay_lanes::at::Read<'_>> = names::WHOLE.iter().map(|&table| replay_lanes::at::Read::all(table)).collect();
            reads.push(replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: replay_lanes::at::Keys::These(&keys) });
            let mut frame = dashboard_query::cost::storage(|| held_seeker.frame(run, hour, &reads))?;
            let tokens: Vec<u128> = names::variant_tokens(&frame).collect::<BTreeSet<_>>().into_iter().map(u128::from).collect();
            if !tokens.is_empty() {
                let token_read = [replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay_lanes::at::Keys::These(&tokens) }];
                let lexical = dashboard_query::cost::storage(|| held_seeker.frame(run, hour, &token_read))?;
                frame.tables[TableId::LexicalTokens.index()] = lexical.tables[TableId::LexicalTokens.index()].clone();
            }
            let mut named = Named::default();
            names::name(&frame, &variants, &[], context, &mut named);
            for icon in named.icons() {
                icon_images.insert(icon.key, icon.image);
            }
            for &variant in &variants {
                let entry = named.variants.get(&variant);
                let name = entry.and_then(|v| v.name.clone());
                let icon = entry.and_then(|v| v.icon.as_ref()).and_then(|key| icon_images.get(key)).cloned();
                resolved_names.insert((run, hour, variant), (name, icon));
            }
        }
    }

    let lookup_variant = |hour: i32, variant: u64| -> (Option<String>, Option<String>, Option<String>) {
        if let Err(why) = context {
            return (None, None, Some(why.clone()));
        }
        let found = query
            .timeline
            .at(hour)
            .and_then(|stretch| resolved_names.get(&(stretch.run, hour, variant)));
        match found {
            Some((Some(name), icon)) => (Some(name.clone()), icon.clone(), None),
            Some((None, icon)) => (None, icon.clone(), Some("The recording does not name this design at the ledger's reads".into())),
            None => (None, None, Some("The recording does not name this design at the ledger's reads".into())),
        }
    };

    let mut unavailable_count = 0_usize;
    let mut out = Vec::with_capacity(contracts.len());
    for (idx, mut acc) in contracts.into_iter().enumerate() {
        let (seller, seller_not_recorded) = resolve_party(query, links, &tags, acc.seller_raw, acc.seller_hour);
        let (buyer, buyer_not_recorded) = resolve_party(query, links, &tags, acc.buyer_raw, acc.buyer_hour);
        let status = if acc.ended {
            MarketContractStatus::Ended
        } else if acc.started {
            MarketContractStatus::Active
        } else if acc.requested {
            MarketContractStatus::Requested
        } else if acc.events.iter().any(|ev| matches!(ev.kind, MarketContractEventKind::Delivery | MarketContractEventKind::Payment | MarketContractEventKind::DeliveryUnavailable)) {
            MarketContractStatus::Active
        } else {
            MarketContractStatus::NotRecorded
        };

        let terms = contract_probe[idx].and_then(|key| terms_map.get(&key));
        let terms_hour = contract_probe[idx].map(|(_, hour, _)| hour).unwrap_or(acc.first_hour);

        let mut item_order: Vec<u64> = Vec::new();
        let mut contracted: HashMap<u64, i64> = HashMap::new();
        let mut delivered: HashMap<u64, i64> = HashMap::new();
        let mut delivery_missing: HashMap<u64, String> = HashMap::new();
        let mut variant_hour: HashMap<u64, i32> = HashMap::new();
        let mut contract_delivery_missing: Option<String> = None;

        if let Some(info) = terms {
            for &(variant, amount) in &info.equipment {
                if !item_order.contains(&variant) {
                    item_order.push(variant);
                }
                *contracted.entry(variant).or_insert(0) = contracted.get(&variant).copied().unwrap_or(0).saturating_add(amount);
                variant_hour.entry(variant).or_insert(terms_hour);
            }
        }

        let mut paid_sum: i64 = 0;
        let mut paid_any = false;
        let mut paid_not_recorded = false;

        for ev in &acc.events {
            match ev.kind {
                MarketContractEventKind::Delivery => {
                    if let Some(variant) = ev.variant {
                        if !item_order.contains(&variant) {
                            item_order.push(variant);
                        }
                        variant_hour.insert(variant, ev.hour);
                        match ev.amount_raw {
                            Some(amount) => {
                                *delivered.entry(variant).or_insert(0) = delivered.get(&variant).copied().unwrap_or(0).saturating_add(amount);
                            }
                            None => {
                                delivery_missing.entry(variant).or_insert_with(|| "Delivery amount was not recorded".into());
                            }
                        }
                    } else if ev.amount_raw.is_none() {
                        contract_delivery_missing.get_or_insert_with(|| "Delivery amount was not recorded".into());
                    }
                }
                MarketContractEventKind::DeliveryUnavailable => {
                    unavailable_count += 1;
                    let reason = ev.reason.clone().unwrap_or_else(|| "Delivery amount unavailable".into());
                    if let Some(variant) = ev.variant {
                        if !item_order.contains(&variant) {
                            item_order.push(variant);
                        }
                        variant_hour.insert(variant, ev.hour);
                        delivery_missing.entry(variant).or_insert(reason);
                    } else {
                        contract_delivery_missing.get_or_insert(reason);
                    }
                }
                MarketContractEventKind::Payment => match ev.amount_raw {
                    Some(amount) => {
                        paid_any = true;
                        paid_sum = paid_sum.saturating_add(amount);
                    }
                    None => {
                        paid_not_recorded = true;
                    }
                },
                MarketContractEventKind::RequestPosted | MarketContractEventKind::ContractStarted | MarketContractEventKind::ContractEnded => {}
            }
        }

        let items: Vec<MarketContractItem> = item_order
            .into_iter()
            .map(|variant| {
                let hour = variant_hour.get(&variant).copied().unwrap_or(acc.last_hour);
                let (name, icon, name_missing) = lookup_variant(hour, variant);
                let unavail = delivery_missing.get(&variant).cloned().or_else(|| contract_delivery_missing.clone());
                let delivered_raw = if unavail.is_some() {
                    None
                } else {
                    Some(delivered.get(&variant).copied().unwrap_or(0).to_string())
                };
                MarketContractItem {
                    variant: variant.to_string(),
                    name,
                    name_missing,
                    icon,
                    delivered_raw,
                    contracted_raw: contracted.get(&variant).map(|amount| amount.to_string()),
                    delivery_unavailable: unavail,
                }
            })
            .collect();

        acc.events.sort_by(|a, b| b.hour.cmp(&a.hour).then_with(|| b.key.cmp(&a.key)));
        let events: Vec<MarketContractEvent> = acc
            .events
            .into_iter()
            .map(|ev| {
                let (equipment, equipment_missing) = match ev.variant {
                    Some(variant) => {
                        let (name, _, missing) = lookup_variant(ev.hour, variant);
                        (name, missing)
                    }
                    None => (None, None),
                };
                let amount_not_recorded = matches!(ev.kind, MarketContractEventKind::Delivery | MarketContractEventKind::Payment) && ev.amount_raw.is_none();
                MarketContractEvent {
                    key: ev.key.to_string(),
                    hour: ev.hour,
                    kind: ev.kind,
                    variant: ev.variant.map(|v| v.to_string()),
                    equipment,
                    equipment_missing,
                    amount_raw: ev.amount_raw.map(|v| v.to_string()),
                    amount_not_recorded,
                    reason: ev.reason,
                }
            })
            .collect();

        out.push(MarketContractRow {
            key: acc.key,
            hour: acc.last_hour,
            started_hour: acc.started_hour,
            ended_hour: acc.ended_hour,
            status,
            seller,
            seller_not_recorded,
            buyer,
            buyer_not_recorded,
            items,
            paid_raw: if paid_not_recorded || !paid_any { None } else { Some(paid_sum.to_string()) },
            paid_not_recorded,
            agreed_price_raw: terms.and_then(|info| info.agreed_price_raw).map(|v| v.to_string()),
            events,
        });
    }

    if unavailable_count > 0 {
        warnings.push(format!("{unavailable_count} deliveries on this page have no recorded amount; each keeps its reason"));
    }
    Ok((out, warnings))
}

fn load_tail_at(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    context: &names::Context,
    log_caches: Option<&LogCaches>,
    params: &MarketLedgerTailParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<MarketLedgerTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let declared = dashboard_declare::log(LOG).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("No declared log {LOG}")))?;
    let from = params.from.map(|value| hour_of(Some(value), "from")).transpose()?.unwrap_or(query.timeline.first);
    if from > hour {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid market ledger range"));
    }
    let offset = params.offset.as_deref().unwrap_or("0").parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid offset"))?;
    let limit = usize::try_from(params.limit.clamp(1, PAGE_LIMIT)).unwrap_or(PAGE_LIMIT as usize);
    let facets = effective_facets(&params.facets);
    let reader = match log_caches {
        Some(caches) => log::Reader::with_cache(query, declared, Arc::clone(&caches.buckets))?,
        None => log::Reader::new(query, declared)?,
    };
    let selection = reader.select(&facets, query.timeline.first, hour)?;
    let columns = Columns::read()?;

    let mut contracts: HashMap<String, ContractAcc> = HashMap::new();
    let mut seen_keys: std::collections::HashSet<u128> = std::collections::HashSet::new();
    let mut cursor = 0_u64;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let page = reader.page(&selection.patterns, selection.groups.len(), from, hour, cursor, PAGE_LIMIT)?;
        for paged in &page.rows {
            if !seen_keys.insert(paged.row.key) {
                continue;
            }
            let Some((agreement, ev)) = parse_event(&paged.row, &columns) else {
                continue;
            };
            let key = match agreement {
                Some(id) => format!("{id:016x}"),
                None => format!("unknown:{}", paged.row.key),
            };
            let acc = contracts.entry(key.clone()).or_insert_with(|| ContractAcc {
                agreement,
                key,
                last_hour: ev.hour,
                first_hour: ev.hour,
                started_hour: None,
                ended_hour: None,
                requested: false,
                started: false,
                ended: false,
                seller_raw: ev.seller_raw,
                buyer_raw: ev.buyer_raw,
                seller_hour: ev.hour,
                buyer_hour: ev.hour,
                events: Vec::new(),
            });
            acc.last_hour = acc.last_hour.max(ev.hour);
            acc.first_hour = acc.first_hour.min(ev.hour);
            match ev.kind {
                MarketContractEventKind::RequestPosted => {
                    acc.requested = true;
                    acc.started_hour.get_or_insert(ev.hour);
                }
                MarketContractEventKind::ContractStarted => {
                    acc.started = true;
                    acc.started_hour = Some(acc.started_hour.map_or(ev.hour, |prev| prev.min(ev.hour)));
                }
                MarketContractEventKind::ContractEnded => {
                    acc.ended = true;
                    acc.ended_hour = Some(acc.ended_hour.map_or(ev.hour, |prev| prev.max(ev.hour)));
                }
                _ => {}
            }
            if ev.seller_raw.is_some_and(|raw| raw != 0) && (acc.seller_raw.is_none() || acc.seller_raw == Some(0) || ev.hour >= acc.seller_hour) {
                acc.seller_raw = ev.seller_raw;
                acc.seller_hour = ev.hour;
            }
            if ev.buyer_raw.is_some_and(|raw| raw != 0) && (acc.buyer_raw.is_none() || acc.buyer_raw == Some(0) || ev.hour >= acc.buyer_hour) {
                acc.buyer_raw = ev.buyer_raw;
                acc.buyer_hour = ev.hour;
            }
            acc.events.push(ev);
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => break,
        }
    }

    let mut ordered: Vec<ContractAcc> = contracts.into_values().collect();
    ordered.sort_by(|a, b| b.last_hour.cmp(&a.last_hour).then_with(|| b.key.cmp(&a.key)));
    let total = ordered.len() as u64;
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(ordered.len());
    let end = start.saturating_add(limit).min(ordered.len());
    let slice: Vec<ContractAcc> = ordered.into_iter().skip(start).take(end - start).collect();

    let (rows, warnings) = enrich_contracts(query, links, context, slice, cancelled)?;
    let missing = logs::missing(query, declared, from, hour)?;
    let first_ordinal = if rows.is_empty() { 0 } else { offset };
    Ok(MarketLedgerTailReply {
        branch_id: params.branch_id.clone(),
        base_ordinal: "0".into(),
        first_ordinal: first_ordinal.to_string(),
        offset: offset.to_string(),
        total: total.to_string(),
        rows,
        missing,
        warnings,
    })
}

pub struct MarketLedgerTail;

impl Product for MarketLedgerTail {
    const KEY: &'static str = "market-ledger-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = MarketLedgerTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        facets,
        from: Start,
        hour: Read,
        offset,
        limit,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
    type Reply = MarketLedgerTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MarketLedgerTailParams::decl(),
            MarketContractStatus::decl(),
            MarketContractEventKind::decl(),
            MarketContractItem::decl(),
            MarketContractEvent::decl(),
            MarketContractRow::decl(),
            MarketLedgerTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_facets(&params.facets)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = params.hour.unwrap_or(query.timeline.last);
        let links = links_by_run(source, &query)?;
        let context = names::context(source, load)?;
        let log_caches = dashboard_base::queries::log_caches(source);
        load_tail_at(&query, &links, &context, Some(&log_caches), params, hour, cancelled)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid market-ledger-tail panel")?.remove("panel");
    let params: MarketLedgerTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid market-ledger-tail panel: {error}"))?;
    MarketLedgerTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let links = Arc::new(links_by_run(source, &query).map_err(|error| error.to_string())?);
    let context = Arc::new(names::context(source, load).map_err(|error| error.to_string())?);
    let log_caches = dashboard_base::queries::log_caches(source);
    let declared = dashboard_declare::log(LOG).ok_or_else(|| format!("No declared log {LOG}"))?;
    let facets = effective_facets(&params.facets);
    let pre_reader = log::Reader::with_cache(&query, declared, Arc::clone(&log_caches.buckets)).map_err(|error| error.to_string())?;
    let selection = Arc::new(pre_reader.select(&facets, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let empty_counts = Arc::new(Vec::new());
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    let q_replay = Arc::clone(&replay);
    let q_timeline = timeline.clone();
    let q_holders = Arc::clone(&holders);
    let q_facets = Arc::new(facets);
    let q_from = params.from;
    let q_caches = log_caches.clone();
    Ok(dated_tail_panel(
        executable,
        move |prev_hour, hour, prev_reply, cancelled| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let query = Query { replay: &q_replay, timeline: q_timeline.clone(), holders: q_holders.clone() };
            quiet_tail_missing(&query, LOG, &q_caches, &selection, &empty_counts, &q_facets, q_from, prev_hour, hour, prev_reply)
        },
        move |hour, cancelled| {
            let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
            load_tail_at(&query, &links, &context, Some(&log_caches), &params, hour, cancelled)
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records, source};
    use dashboard_query::identity::Country;
    use replay::campaign::Replay;
    use replay_data::rows::{LexicalTokenRow, MarketContractTermsRow, MarketEquipment, MarketPool, TableRows, VariantDefinitionRow};
    use replay_frame::frame::TableView;

    #[test]
    fn contracts_group_deliveries_and_payments_separately_with_contracted_totals() {
        let (cat, _folder) = dashboard_base::catalogue::tests::localisation_catalogue(&[
            ("hoi4.exe", "test"),
            ("user/settings.txt", "language=l_english force_pow2_textures=no"),
            ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
            ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }"),
            ("localisation/mod_l_english.yml", "\u{feff}l_english:\n rifle_0:0 \"Default Rifle\"\n"),
        ]);
        let context: names::Context = Ok(names::Content::from_catalogue(Arc::new(cat)));
        let frames: Vec<_> = (100..=112)
            .map(|hour| {
                let mut frame = ownership_frame(1, hour, 9001);
                *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"rifle_0".to_vec() }]));
                *frame.tables[TableId::VariantDefinitions.index()] = TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                    id: 6,
                    equipment_token: 1,
                    display_name: b"Infantry Equipment I".to_vec(),
                    ..Default::default()
                }]));
                if (100..=108).contains(&hour) {
                    *frame.tables[TableId::MarketContractTerms.index()] = TableView::Complete(TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                        country: 9001,
                        buyer: 44,
                        id: 0x12,
                        kind: 1,
                        equipment: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 500_000 }] }.into(),
                        prices: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 96_000 }] }.into(),
                        ..Default::default()
                    }]));
                }
                let events = match hour {
                    100 => vec![MarketEventRow { id: 1, occurrence: 1, hour, country: 9001, receiver: 44, kind: 11, agreement: 0x12, ..Default::default() }],
                    102 => vec![MarketEventRow { id: 2, occurrence: 2, hour, country: 9001, receiver: 44, kind: 21, agreement: 0x12, amount_raw: 48_000, ..Default::default() }],
                    104 => vec![MarketEventRow { id: 3, occurrence: 3, hour, country: 9001, receiver: 44, kind: 20, agreement: 0x12, variant: 6, amount_raw: 250_000, ..Default::default() }],
                    106 => vec![MarketEventRow { id: 4, occurrence: 4, hour, country: 9001, receiver: 44, kind: 21, agreement: 0x12, amount_raw: 48_000, ..Default::default() }],
                    108 => vec![
                        MarketEventRow { id: 5, occurrence: 5, hour, country: 9001, receiver: 44, kind: 20, agreement: 0x12, variant: 6, amount_raw: 250_000, ..Default::default() },
                        MarketEventRow { id: 6, occurrence: 6, hour, country: 9001, receiver: 44, kind: 12, agreement: 0x12, ..Default::default() },
                    ],
                    _ => Vec::new(),
                };
                *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let src = Source::from_replay(
            dashboard_base::SourceKey { source_id: "mkt-contracts".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let links = links_by_run(&src, &query).unwrap();
        let facets = vec![FacetFilter { name: "country|receiver".into(), countries: vec![Country::from("FRA"), Country::from("GER")], ..Default::default() }];
        let params = MarketLedgerTailParams { branch_id: "1".into(), facets: facets.clone(), from: None, hour: Some(105), offset: None, limit: 64 };
        let mid = load_tail_at(&query, &links, &context, None, &params, 105, &AtomicBool::new(false)).unwrap();
        assert_eq!(mid.total, "1", "seller and buyer filter deduplicates to one contract row");
        let contract = &mid.rows[0];
        assert_eq!(contract.status, MarketContractStatus::Active);
        assert_eq!(contract.paid_raw.as_deref(), Some("48000"));
        assert_eq!(contract.agreed_price_raw.as_deref(), Some("96000"));
        assert_eq!(contract.items.len(), 1);
        assert_eq!(contract.items[0].name.as_deref(), Some("Infantry Equipment I"));
        assert_eq!(contract.items[0].delivered_raw.as_deref(), Some("250000"));
        assert_eq!(contract.items[0].contracted_raw.as_deref(), Some("500000"));
        assert_eq!(
            contract.events.iter().map(|ev| (ev.hour, ev.kind)).collect::<Vec<_>>(),
            vec![
                (104, MarketContractEventKind::Delivery),
                (102, MarketContractEventKind::Payment),
                (100, MarketContractEventKind::ContractStarted),
            ],
            "dated deliveries and payments stay separate child events"
        );

        let done = load_tail_at(&query, &links, &context, None, &params, 110, &AtomicBool::new(false)).unwrap();
        assert_eq!(done.rows[0].status, MarketContractStatus::Ended);
        assert_eq!(done.rows[0].paid_raw.as_deref(), Some("96000"));
        assert_eq!(done.rows[0].items[0].delivered_raw.as_deref(), Some("500000"));
        assert_eq!(done.rows[0].events.len(), 6);

        let load = LoadContext { content: None };
        let asked = serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "facets": facets, "limit": 64 });
        let panel = dated_panel(&src, &load, &asked).unwrap();
        let exact = panel.exact().unwrap();
        for hour in [100, 102, 105, 108, 104, 112] {
            let stepped = (exact.advance)(hour, true, &AtomicBool::new(false), None);
            let cold = (exact.cold)(hour, &AtomicBool::new(false));
            assert_eq!(*stepped.value, cold, "stepped vs cold at {hour}");
        }
    }

    #[test]
    fn unavailable_delivery_withholds_delivered_total_and_keeps_reason() {
        let frames: Vec<_> = (100..=104)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let events = if hour == 102 {
                    vec![MarketEventRow { id: 10, occurrence: 10, hour, country: 9001, receiver: 44, kind: 34, mode: 20, ordinal: 4, agreement: 0x33, variant: 6, ..Default::default() }]
                } else {
                    Vec::new()
                };
                *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let src = Source::from_replay(
            dashboard_base::SourceKey { source_id: "mkt-unavail".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = LoadContext { content: None };
        let reply = MarketLedgerTail::load(
            &src,
            &MarketLedgerTailParams { branch_id: "1".into(), facets: Vec::new(), from: None, hour: Some(104), offset: None, limit: 64 },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.rows.len(), 1);
        assert_eq!(reply.rows[0].items[0].delivered_raw, None);
        assert_eq!(
            reply.rows[0].items[0].delivery_unavailable.as_deref(),
            Some("Delivery amount unavailable: purchase manifest not acquired before removal")
        );
        assert_eq!(reply.rows[0].events[0].kind, MarketContractEventKind::DeliveryUnavailable);
    }
}
