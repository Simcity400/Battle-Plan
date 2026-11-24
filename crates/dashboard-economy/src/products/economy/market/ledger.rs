
use dashboard_base::{Source, catalogue::Catalogue, products::{LoadContext, Product, log_page::{self as logs, LogCaches, LogCell, LogFacetCountAsk, LogPageParams, LogPageReply, LogPageRow, LogTailReply, dated_tail_panel, quiet_tail_missing}}, queries::country_ids::Names};
use dashboard_follow::panel::Panel;
use dashboard_world::products::equipment_names as names;
use dashboard_query::{Query, log::{self, Cell, FacetFilter}};
use replay_data::rows::{MarketEventRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const LOG: &str = "market-events";

pub const KINDS: [i64; 10] = [3, 4, 5, 6, 10, 11, 12, 20, 21, 34];

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
    occurrence: usize,
    agreement: usize,
    country: usize,
    receiver: usize,
    kind: usize,
    mode: usize,
    ordinal: usize,
    variant: usize,
    amount: usize,
    auxiliary: usize,
    total: usize,
    lost_total: usize,
}

impl Columns {
    fn read() -> io::Result<Self> {
        let field = |name| dashboard_declare::field(TableId::MarketEvents, name);
        Ok(Self {
            occurrence: field("occurrence")?,
            agreement: field("agreement")?,
            country: field("country")?,
            receiver: field("receiver")?,
            kind: field("kind")?,
            mode: field("mode")?,
            ordinal: field("ordinal")?,
            variant: field("variant")?,
            amount: field("amount_raw")?,
            auxiliary: field("auxiliary_raw")?,
            total: field("total_raw")?,
            lost_total: field("lost_total_raw")?,
        })
    }
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn number_cell(value: Option<i64>) -> LogCell {
    value.map_or(LogCell::NotRecorded, |value| LogCell::Number { value: value.to_string() })
}

fn text_cell(value: Option<String>) -> LogCell {
    value.map_or(LogCell::Missing, |value| LogCell::Text { value })
}

pub struct Mapped {
    pub cells: Vec<LogCell>,
    pub variant: Option<u64>,
}

const EQUIPMENT: usize = 8;

fn map(key: u128, cells: &[Cell], columns: &Columns, tag: impl Fn(u32) -> Option<String>) -> Mapped {
    let kind = number(cells, columns.kind).and_then(|kind| u8::try_from(kind).ok());
    let ordinal = number(cells, columns.ordinal).and_then(|ordinal| u32::try_from(ordinal).ok());
    let reason = match kind {
        Some(34) => match ordinal {
            Some(ordinal) => MarketEventRow { kind: 34, ordinal, ..MarketEventRow::default() }.delivery_unavailable_reason().map(str::to_owned),
            None => Some("Delivery amount unavailable: refusal reason not recorded".to_owned()),
        },
        _ => None,
    };
    let party = |column: usize| match number(cells, column) {
        None => LogCell::NotRecorded,
        Some(0) => LogCell::Missing,
        Some(raw) => u32::try_from(raw).ok().and_then(&tag).map_or(LogCell::Missing, |value| LogCell::Text { value }),
    };
    let variant = number(cells, columns.variant).map(|variant| variant as u64);
    let raw = |column: usize| if reason.is_some() { LogCell::Missing } else { number_cell(number(cells, column)) };
    Mapped {
        cells: vec![
            LogCell::Text { value: key.to_string() },
            number_cell(number(cells, columns.occurrence)),
            kind.map_or(LogCell::NotRecorded, |kind| LogCell::Number { value: kind.to_string() }),
            number_cell(number(cells, columns.mode)),
            number(cells, columns.agreement).map_or(LogCell::NotRecorded, |agreement| LogCell::Text { value: format!("{:016x}", agreement as u64) }),
            party(columns.country),
            party(columns.receiver),
            variant.map_or(LogCell::NotRecorded, |variant| LogCell::Text { value: variant.to_string() }),
            LogCell::Missing,
            match variant {
                None => LogCell::Text { value: "Equipment design not recorded".into() },
                Some(0) => LogCell::Missing,
                Some(_) => LogCell::Text { value: "Equipment not named yet".into() },
            },
            raw(columns.amount),
            raw(columns.auxiliary),
            number_cell(number(cells, columns.total)),
            number_cell(number(cells, columns.lost_total)),
            text_cell(reason),
        ],
        variant,
    }
}

pub struct MarketLedger;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketLedgerParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    pub from: Option<f64>,
    pub to: f64,
    pub cursor: Option<String>,
    pub page_size: u32,
}

fn validate_facets(facets: &[FacetFilter]) -> Result<(), &'static str> {
    let kinds = facets.iter().find(|facet| facet.name == "kind").ok_or("A market ledger page names the occurrences it shows")?;
    if kinds.values.is_empty() || kinds.values.iter().any(|kind| !KINDS.contains(kind)) {
        return Err("Invalid market occurrence kind");
    }
    Ok(())
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

const VARIANT_CACHE_CAPACITY: usize = 4_096;

struct VariantNameCache {
    capacity: usize,
    entries: Option<(Arc<Catalogue>, VecDeque<(u32, i32, u64)>, HashMap<(u32, i32, u64), Option<String>>)>,
}

impl Default for VariantNameCache {
    fn default() -> Self {
        Self::with_capacity(VARIANT_CACHE_CAPACITY)
    }
}

impl VariantNameCache {
    fn with_capacity(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), entries: None }
    }

    fn clear(cache: &Mutex<Self>) {
        cache.lock().unwrap_or_else(PoisonError::into_inner).entries = None;
    }

    #[cfg(test)]
    fn len(cache: &Mutex<Self>) -> usize {
        cache.lock().unwrap_or_else(PoisonError::into_inner).entries.as_ref().map_or(0, |(_, _, map)| map.len())
    }

    fn resolve<'a>(
        cache: &Mutex<Self>,
        recording: &'a replay_recording::columnar_frames::Recording,
        seeker: &mut Option<replay_lanes::at::Seeker<'a>>,
        run: u32,
        hour: i32,
        variants: &[u64],
        content: &names::Content,
        context: &names::Context,
    ) -> io::Result<BTreeMap<u64, Option<String>>> {
        let mut out = BTreeMap::new();
        let mut missing = Vec::new();
        {
            let guard = cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((cat, _, map)) = guard.entries.as_ref()
                && Arc::ptr_eq(cat, &content.catalogue)
            {
                for &variant in variants {
                    if let Some(found) = map.get(&(run, hour, variant)) {
                        out.insert(variant, found.clone());
                    } else {
                        missing.push(variant);
                    }
                }
            } else {
                missing.extend_from_slice(variants);
            }
        }
        if !missing.is_empty() {
            let resolved = plain_names_with_seeker(recording, seeker, run, hour, &missing, context)?;
            {
                let mut guard = cache.lock().unwrap_or_else(PoisonError::into_inner);
                let cap = guard.capacity;
                let reuse = guard.entries.as_ref().is_some_and(|(cat, _, _)| Arc::ptr_eq(cat, &content.catalogue));
                if !reuse {
                    guard.entries = Some((Arc::clone(&content.catalogue), VecDeque::new(), HashMap::new()));
                }
                let (_, order, map) = guard.entries.as_mut().unwrap();
                for &variant in &missing {
                    let value = resolved.get(&variant).cloned().flatten();
                    let key = (run, hour, variant);
                    if map.insert(key, value.clone()).is_none() {
                        order.push_back(key);
                        while map.len() > cap {
                            let Some(evicted) = order.pop_front() else { break };
                            map.remove(&evicted);
                        }
                    }
                    out.insert(variant, value);
                }
            }
        }
        Ok(out)
    }
}

pub(crate) fn forget(source: &Source) {
    VariantNameCache::clear(&source.state::<VariantNameCache>());
}

fn enrich_rows(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    context: &names::Context,
    name_cache: Option<&Mutex<VariantNameCache>>,
    rows: Vec<(dashboard_query::log::Row, u64, u64, Option<dashboard_query::identity::Country>)>,
    tagged: bool,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<LogPageRow>, Vec<String>)> {
    let columns = Columns::read()?;
    let tags = Names::new(&query.holders);
    let mut by_run_hour: BTreeMap<(u32, i32), Vec<usize>> = BTreeMap::new();
    let mut mapped = Vec::with_capacity(rows.len());
    for (at, (row, ..)) in rows.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(stretch) = query.timeline.at(row.hour) else {
            mapped.push(map(row.key, &row.cells, &columns, |_| None));
            continue;
        };
        let run_links = links.get(&stretch.run);
        mapped.push(map(row.key, &row.cells, &columns, |raw| run_links.and_then(|held| tags.tag_at(held, raw, row.hour)).map(str::to_owned)));
        by_run_hour.entry((stretch.run, row.hour)).or_default().push(at);
    }
    let mut warnings = Vec::new();
    match context {
        Err(why) => {
            for row in &mut mapped {
                if row.variant.is_some_and(|variant| variant != 0) {
                    row.cells[EQUIPMENT + 1] = LogCell::Text { value: why.clone() };
                }
            }
            warnings.push(why.clone());
        }
        Ok(content) => {
            let recording = query.replay.recording();
            let mut seeker: Option<replay_lanes::at::Seeker<'_>> = None;
            for ((run, hour), ats) in by_run_hour {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let variants: Vec<u64> = ats.iter().filter_map(|&at| mapped[at].variant).filter(|&variant| variant != 0).collect::<BTreeSet<_>>().into_iter().collect();
                if variants.is_empty() {
                    continue;
                }
                let batch = match name_cache {
                    Some(cache) => VariantNameCache::resolve(cache, recording, &mut seeker, run, hour, &variants, content, context)?,
                    None => plain_names_with_seeker(recording, &mut seeker, run, hour, &variants, context)?,
                };
                for &at in &ats {
                    let row = &mut mapped[at];
                    if let Some(variant) = row.variant.filter(|&variant| variant != 0) {
                        match batch.get(&variant).and_then(|name| name.as_ref()) {
                            Some(name) => {
                                row.cells[EQUIPMENT] = LogCell::Text { value: name.clone() };
                                row.cells[EQUIPMENT + 1] = LogCell::Missing;
                            }
                            None => row.cells[EQUIPMENT + 1] = LogCell::Text { value: "The recording does not name this design at the ledger's reads".into() },
                        }
                    }
                }
            }
        }
    }
    let unavailable = mapped.iter().filter(|row| !matches!(row.cells[14], LogCell::Missing)).count();
    if unavailable > 0 {
        warnings.push(format!("{unavailable} deliveries on this page have no recorded amount; each keeps its reason"));
    }
    let reply_rows = rows
        .into_iter()
        .zip(mapped)
        .map(|((row, ordinal, own, tag), mapped)| LogPageRow {
            hour: row.hour,
            key: row.key.to_string(),
            ordinal: ordinal.to_string(),
            country: tag,
            own_ordinal: tagged.then(|| own.to_string()),
            cells: mapped.cells,
        })
        .collect();
    Ok((reply_rows, warnings))
}

impl Product for MarketLedger {
    const KEY: &'static str = "market-ledger";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MarketLedgerParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        facets,
        from: Start,
        to: End,
        cursor,
        pageSize,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End
    ];
    type Reply = LogPageReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MarketLedgerParams::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_facets(&params.facets)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let asked = LogPageParams {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            facets: params.facets.clone(),
            from: params.from,
            to: params.to,
            cursor: params.cursor.clone(),
            page_size: params.page_size,
            columns: None,
        };
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty() || !filter.membership.is_empty());
        let caches = dashboard_base::queries::log_caches(source);
        let (page, rows, missing) = logs::page_with(&query, &asked, Some(&caches), |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()))?;
        let links = links_by_run(source, &query)?;
        let context = names::context(source, load)?;
        let cache = source.state::<VariantNameCache>();
        let (reply_rows, warnings) = enrich_rows(&query, &links, &context, Some(&cache), rows, tagged, cancelled)?;
        Ok(LogPageReply {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows: reply_rows,
            missing,
            warnings,
        })
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
    #[serde(default)]
    pub count_facets: Vec<LogFacetCountAsk>,
}

fn load_tail_at(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    context: &names::Context,
    name_cache: Option<&Mutex<VariantNameCache>>,
    log_caches: Option<&LogCaches>,
    params: &MarketLedgerTailParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<LogTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty() || !filter.membership.is_empty());
    let offset = params.offset.as_deref().unwrap_or("0").parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid offset"))?;
    let (page, rows, facet_totals, missing) = logs::tail_with(query, LOG, &params.facets, params.from, hour, offset, params.limit, &params.count_facets, log_caches, |paged, tag| {
        (paged.row.clone(), paged.ordinal, paged.own, tag.cloned())
    })?;
    let (reply_rows, warnings) = enrich_rows(query, links, context, name_cache, rows, tagged, cancelled)?;
    let first_ordinal = reply_rows.first().map_or_else(|| page.base.to_string(), |row| row.ordinal.clone());
    Ok(LogTailReply {
        branch_id: params.branch_id.clone(),
        log: LOG.to_owned(),
        columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
        base_ordinal: page.base.to_string(),
        first_ordinal,
        offset: page.offset.to_string(),
        total: page.total.to_string(),
        rows: reply_rows,
        facet_totals,
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
        countFacets,
        "facets[].countries[]",
        "facets[].values[]",
        "facets[].membership[].country",
        "facets[].membership[].firstHour": Start,
        "facets[].membership[].lastHour": End,
        "facets[].unobserved[].firstHour": Start,
        "facets[].unobserved[].lastHour": End,
        "countFacets[].facets[].countries[]",
        "countFacets[].facets[].values[]",
        "countFacets[].facets[].membership[].country",
        "countFacets[].facets[].membership[].firstHour": Start,
        "countFacets[].facets[].membership[].lastHour": End,
        "countFacets[].facets[].unobserved[].firstHour": Start,
        "countFacets[].facets[].unobserved[].lastHour": End
    ];
    type Reply = LogTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MarketLedgerTailParams::decl()]
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
        let cache = source.state::<VariantNameCache>();
        let log_caches = dashboard_base::queries::log_caches(source);
        load_tail_at(&query, &links, &context, Some(&cache), Some(&log_caches), params, hour, cancelled)
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
    let cache = source.state::<VariantNameCache>();
    let log_caches = dashboard_base::queries::log_caches(source);
    let declared = dashboard_declare::log(LOG).ok_or_else(|| format!("No declared log {LOG}"))?;
    let pre_reader = log::Reader::with_cache(&query, declared, Arc::clone(&log_caches.buckets)).map_err(|error| error.to_string())?;
    let selection = Arc::new(pre_reader.select(&params.facets, query.timeline.first, query.timeline.last).map_err(|error| error.to_string())?);
    let count_selections: Arc<Vec<log::Selection>> = Arc::new(
        params
            .count_facets
            .iter()
            .map(|ask| pre_reader.select(&ask.facets, query.timeline.first, query.timeline.last))
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?,
    );
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    let q_replay = Arc::clone(&replay);
    let q_timeline = timeline.clone();
    let q_holders = Arc::clone(&holders);
    let q_params = Arc::clone(&params);
    let q_caches = log_caches.clone();
    Ok(dated_tail_panel(
        executable,
        move |prev_hour, hour, prev_reply, cancelled| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let query = Query { replay: &q_replay, timeline: q_timeline.clone(), holders: q_holders.clone() };
            quiet_tail_missing(&query, LOG, &q_caches, &selection, &count_selections, &q_params.facets, q_params.from, prev_hour, hour, prev_reply)
        },
        move |hour, cancelled| {
            let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
            load_tail_at(&query, &links, &context, Some(&cache), Some(&log_caches), &params, hour, cancelled)
        },
    ))
}

fn plain_names_with_seeker<'r>(
    recording: &'r replay_recording::columnar_frames::Recording,
    seeker: &mut Option<replay_lanes::at::Seeker<'r>>,
    run: u32,
    hour: i32,
    variants: &[u64],
    context: &names::Context,
) -> io::Result<BTreeMap<u64, Option<String>>> {
    let held_seeker = match seeker.as_mut() {
        Some(held) => held,
        None => seeker.insert(replay_lanes::at::Seeker::new(recording)?),
    };
    let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
    let mut reads: Vec<replay_lanes::at::Read<'_>> = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance].into_iter().map(replay_lanes::at::Read::all).collect();
    reads.push(replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: replay_lanes::at::Keys::These(&keys) });
    let mut frame = dashboard_query::cost::storage(|| held_seeker.frame(run, hour, &reads))?;
    let tokens: Vec<u128> = names::variant_tokens(&frame).collect::<BTreeSet<_>>().into_iter().map(u128::from).collect();
    let token_read = [replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay_lanes::at::Keys::These(&tokens) }];
    let lexical = dashboard_query::cost::storage(|| held_seeker.frame(run, hour, &token_read))?;
    frame.tables[TableId::LexicalTokens.index()] = lexical.tables[TableId::LexicalTokens.index()].clone();
    Ok(names::plain_names(&frame, variants, context))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(columns: &Columns, set: &[(fn(&Columns) -> usize, i64)]) -> Vec<Cell> {
        let mut out = vec![Cell::Number(0); TableId::MarketEvents.field_count()];
        for (column, value) in set {
            out[column(columns)] = Cell::Number(*value);
        }
        out
    }

    fn cell(mapped: &Mapped, name: &str) -> LogCell {
        mapped.cells[COLUMNS.iter().position(|column| *column == name).unwrap()].clone()
    }

    #[test]
    fn an_occurrence_names_its_parties_and_an_unavailable_amount_keeps_its_reason() {
        let columns = Columns::read().unwrap();
        let tag = |raw: u32| (raw == 44).then(|| "FRA".to_owned());
        let unavailable = cells(&columns, &[(|c| c.kind, 34), (|c| c.mode, 20), (|c| c.ordinal, 4), (|c| c.country, 44), (|c| c.receiver, 9001), (|c| c.agreement, 0x12)]);
        let mapped = map(21, &unavailable, &columns, tag);
        assert_eq!(cell(&mapped, "seller"), LogCell::Text { value: "FRA".into() });
        assert_eq!(cell(&mapped, "buyer"), LogCell::Missing, "a buyer with no tag at the hour is unnamed");
        assert_eq!(cell(&mapped, "contract"), LogCell::Text { value: "0000000000000012".into() });
        assert_eq!(cell(&mapped, "amountRaw"), LogCell::Missing, "an unavailable amount is no amount, never zero");
        assert_eq!(cell(&mapped, "reason"), LogCell::Text { value: "Delivery amount unavailable: purchase manifest not acquired before removal".into() });
        let delivered = cells(&columns, &[(|c| c.kind, 20), (|c| c.country, 44), (|c| c.amount, 250_000), (|c| c.variant, 6), (|c| c.total, 900_000)]);
        let mapped = map(22, &delivered, &columns, tag);
        assert_eq!(cell(&mapped, "amountRaw"), LogCell::Number { value: "250000".into() });
        assert_eq!(cell(&mapped, "totalRaw"), LogCell::Number { value: "900000".into() });
        assert_eq!(cell(&mapped, "reason"), LogCell::Missing);
        assert_eq!((mapped.variant, cell(&mapped, "variant")), (Some(6), LogCell::Text { value: "6".into() }));
        assert!(matches!(cell(&mapped, "equipmentMissing"), LogCell::Text { .. }), "a design is unnamed until the page names it, and says so");
        let unrecorded = vec![Cell::Unrecorded; TableId::MarketEvents.field_count()];
        let mapped = map(23, &unrecorded, &columns, tag);
        assert_eq!(cell(&mapped, "kind"), LogCell::NotRecorded, "an unrecorded kind is never 255");
        assert_eq!(cell(&mapped, "contract"), LogCell::NotRecorded, "an unrecorded agreement handle is never all zeroes");
        assert_eq!(cell(&mapped, "seller"), LogCell::NotRecorded);
        assert_eq!(cell(&mapped, "buyer"), LogCell::NotRecorded);
        assert_eq!((mapped.variant, cell(&mapped, "variant")), (None, LogCell::NotRecorded), "an unrecorded variant is never zero");
        assert_eq!(cell(&mapped, "equipmentMissing"), LogCell::Text { value: "Equipment design not recorded".into() });
        let mut missing_refusal = vec![Cell::Unrecorded; TableId::MarketEvents.field_count()];
        missing_refusal[columns.kind] = Cell::Number(34);
        let mapped = map(24, &missing_refusal, &columns, tag);
        assert_eq!(cell(&mapped, "reason"), LogCell::Text { value: "Delivery amount unavailable: refusal reason not recorded".into() });
    }

    #[test]
    fn unrecorded_market_cells_stay_not_recorded_instead_of_defaulting_to_zero() {
        let columns = Columns::read().unwrap();
        let missing = vec![Cell::Unrecorded; TableId::MarketEvents.field_count()];
        let mapped = map(99, &missing, &columns, |_| None);
        assert_eq!(cell(&mapped, "kind"), LogCell::NotRecorded);
        assert_eq!(cell(&mapped, "contract"), LogCell::NotRecorded);
        assert_eq!(cell(&mapped, "variant"), LogCell::NotRecorded);
    }

    #[test]
    fn market_ledger_tail_and_dated_panel_match_page_in_reverse() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        use replay::campaign::Replay;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;

        let frames = (100..=112)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let events = if hour % 2 == 0 {
                    vec![MarketEventRow { id: hour as u64, occurrence: hour as u64, hour, country: 9001, kind: 20, agreement: hour as u64, amount_raw: 100_000, ..MarketEventRow::default() }]
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
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "mkt-tail".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = LoadContext { content: None };
        let facets = vec![FacetFilter { name: "kind".into(), values: vec![20], ..Default::default() }];
        let page = MarketLedger::load(
            &source,
            &MarketLedgerParams { branch_id: "1".into(), facets: facets.clone(), from: None, to: 110.0, cursor: None, page_size: 64 },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        let tail = MarketLedgerTail::load(
            &source,
            &MarketLedgerTailParams { branch_id: "1".into(), facets: facets.clone(), from: None, hour: Some(110), offset: None, limit: 64, count_facets: Vec::new() },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(tail.total, page.total);
        assert_eq!(tail.rows, page.rows.into_iter().rev().collect::<Vec<_>>());
        let asked = serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "facets": facets, "limit": 64 });
        let panel = dated_panel(&source, &load, &asked).unwrap();
        let exact = panel.exact().unwrap();
        for hour in [100, 106, 110, 104, 112] {
            let stepped = (exact.advance)(hour, true, &AtomicBool::new(false), None);
            let cold = (exact.cold)(hour, &AtomicBool::new(false));
            assert_eq!(*stepped.value, cold, "stepped vs cold at {hour}");
        }
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let links = links_by_run(&source, &query).unwrap();
        let context = names::context(&source, &load).unwrap();
        let cache = source.state::<VariantNameCache>();
        let log_caches = dashboard_base::queries::log_caches(&source);
        let tail_params = MarketLedgerTailParams { branch_id: "1".into(), facets, from: None, hour: None, offset: None, limit: 64, count_facets: Vec::new() };
        for hour in [110, 104, 112, 110] {
            let cached = load_tail_at(&query, &links, &context, Some(&cache), Some(&log_caches), &tail_params, hour, &AtomicBool::new(false)).unwrap();
            let uncached = load_tail_at(&query, &links, &context, None, None, &tail_params, hour, &AtomicBool::new(false)).unwrap();
            assert_eq!(cached, uncached, "cached vs uncached at {hour}");
        }
    }

    #[test]
    fn variant_names_resolve_per_row_hour_across_rename_and_bounded_cache_matches_uncached() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        use replay::campaign::Replay;
        use replay_data::rows::{LexicalTokenRow, TableRows, VariantDefinitionRow};
        use replay_frame::frame::TableView;

        let (cat, _folder) = dashboard_base::catalogue::tests::localisation_catalogue(&[
            ("hoi4.exe", "test"),
            ("user/settings.txt", "language=l_english force_pow2_textures=no"),
            ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
            ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }"),
            ("localisation/mod_l_english.yml", "\u{feff}l_english:\n rifle_0:0 \"Default Rifle\"\n cannon_0:0 \"Field Gun\"\n"),
        ]);
        let context: names::Context = Ok(names::Content::from_catalogue(Arc::new(cat)));
        let frames = (100..=110)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let rifle_name = if hour < 106 { b"Mk I Rifle".to_vec() } else { b"Mk II Rifle".to_vec() };
                *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
                    LexicalTokenRow { token: 1, literal: b"rifle_0".to_vec() },
                    LexicalTokenRow { token: 2, literal: b"cannon_0".to_vec() },
                ]));
                *frame.tables[TableId::VariantDefinitions.index()] = TableView::Complete(TableRows::VariantDefinitions(vec![
                    VariantDefinitionRow { id: 6, equipment_token: 1, display_name: rifle_name, ..Default::default() },
                    VariantDefinitionRow { id: 7, equipment_token: 2, display_name: b"Artillery A".to_vec(), ..Default::default() },
                ]));
                let events = match hour {
                    100 => vec![MarketEventRow { id: 1, occurrence: 1, hour, country: 9001, kind: 20, agreement: 1, variant: 6, amount_raw: 10_000, ..MarketEventRow::default() }],
                    104 => vec![MarketEventRow { id: 2, occurrence: 2, hour, country: 9001, kind: 20, agreement: 2, variant: 7, amount_raw: 20_000, ..MarketEventRow::default() }],
                    108 => vec![MarketEventRow { id: 3, occurrence: 3, hour, country: 9001, kind: 20, agreement: 3, variant: 6, amount_raw: 30_000, ..MarketEventRow::default() }],
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
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "mkt-rename".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let links = links_by_run(&source, &query).unwrap();
        let facets = vec![FacetFilter { name: "kind".into(), values: vec![20], ..Default::default() }];
        let tail_params = MarketLedgerTailParams { branch_id: "1".into(), facets: facets.clone(), from: None, hour: None, offset: None, limit: 64, count_facets: Vec::new() };
        let source_cache = source.state::<VariantNameCache>();

        let whole_tail = load_tail_at(&query, &links, &context, Some(&source_cache), None, &tail_params, 110, &AtomicBool::new(false)).unwrap();
        let uncached_tail = load_tail_at(&query, &links, &context, None, None, &tail_params, 110, &AtomicBool::new(false)).unwrap();
        assert_eq!(whole_tail, uncached_tail);
        let equip_by_hour: Vec<(i32, LogCell)> = whole_tail.rows.iter().map(|r| (r.hour, r.cells[EQUIPMENT].clone())).collect();
        assert_eq!(
            equip_by_hour,
            vec![
                (108, LogCell::Text { value: "Mk II Rifle".into() }),
                (104, LogCell::Text { value: "Artillery A".into() }),
                (100, LogCell::Text { value: "Mk I Rifle".into() }),
            ],
            "each row resolves its variant name at its own hour"
        );

        let small_tail_params = MarketLedgerTailParams { limit: 2, ..tail_params.clone() };
        let small_tail = load_tail_at(&query, &links, &context, Some(&source_cache), None, &small_tail_params, 105, &AtomicBool::new(false)).unwrap();
        assert_eq!(small_tail.rows[1], whole_tail.rows[2], "overlapping smaller tail gives the exact same row at hour 100");

        let asked_page = LogPageParams { branch_id: "1".into(), log: LOG.to_owned(), facets, from: None, to: 110.0, cursor: Some("0".into()), page_size: 2, columns: None };
        let (_, page_raw, _) = logs::page(&query, &asked_page, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned())).unwrap();
        let (page_rows, _) = enrich_rows(&query, &links, &context, Some(&source_cache), page_raw, false, &AtomicBool::new(false)).unwrap();
        assert_eq!(page_rows[0], whole_tail.rows[2], "overlapping page and tail across a rename produce the identical row");
        assert_eq!(page_rows[1], whole_tail.rows[1]);

        assert_eq!(VariantNameCache::len(&source_cache), 3, "cache holds all 3 (run, hour, variant) keys");
        let warm = load_tail_at(&query, &links, &context, Some(&source_cache), None, &tail_params, 110, &AtomicBool::new(false)).unwrap();
        assert_eq!(warm, uncached_tail, "warm cache hit matches uncached");

        let bounded = Mutex::new(VariantNameCache::with_capacity(2));
        for hour in [104, 110, 100, 110] {
            let got = load_tail_at(&query, &links, &context, Some(&bounded), None, &tail_params, hour, &AtomicBool::new(false)).unwrap();
            let want = load_tail_at(&query, &links, &context, None, None, &tail_params, hour, &AtomicBool::new(false)).unwrap();
            assert_eq!(got, want, "bounded cache with eviction matches uncached at {hour}");
            assert!(VariantNameCache::len(&bounded) <= 2, "bounded cache never exceeds capacity");
        }

        forget(&source);
        assert_eq!(VariantNameCache::len(&source_cache), 0, "source invalidation clears VariantNameCache");
        let after_forget = load_tail_at(&query, &links, &context, Some(&source_cache), None, &tail_params, 110, &AtomicBool::new(false)).unwrap();
        assert_eq!(after_forget, uncached_tail, "re-resolution after forget matches uncached");
    }

    #[test]
    fn market_ledger_preserves_each_pattern_while_tail_deduplicates_when_opted_in() {
        use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records};
        use dashboard_query::identity::Country;
        use replay::campaign::Replay;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;

        let frames = (100..=110)
            .map(|hour| {
                let mut frame = ownership_frame(1, hour, 9001);
                let events = if hour == 104 {
                    vec![MarketEventRow { id: 77, occurrence: 77, hour, country: 9001, receiver: 44, kind: 20, agreement: 9, amount_raw: 100_000, ..MarketEventRow::default() }]
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
            dashboard_base::SourceKey { source_id: "mkt-overlap".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = LoadContext { content: None };
        let default_facets = vec![
            FacetFilter { name: "kind".into(), values: vec![20], ..Default::default() },
            FacetFilter { name: "country|receiver".into(), countries: vec![Country::from("FRA"), Country::from("GER")], ..Default::default() },
        ];
        let legacy = MarketLedger::load(
            &src,
            &MarketLedgerParams { branch_id: "1".into(), facets: default_facets, from: None, to: 110.0, cursor: None, page_size: 64 },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(legacy.total, "2", "legacy Trade Flows caller keeps EachPattern match per selected party");
        assert_eq!(legacy.rows.len(), 2);

        let dedup_facets = vec![
            FacetFilter { name: "kind".into(), values: vec![20], ..Default::default() },
            FacetFilter { name: "country|receiver".into(), countries: vec![Country::from("FRA"), Country::from("GER")], deduplicate: true, ..Default::default() },
        ];
        let dedup_tail = MarketLedgerTail::load(
            &src,
            &MarketLedgerTailParams { branch_id: "1".into(), facets: dedup_facets, from: None, hour: Some(110), offset: None, limit: 64, count_facets: Vec::new() },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(dedup_tail.total, "1", "market ledger opt-in deduplicates seller-and-buyer match to one row");
        assert_eq!(dedup_tail.rows.len(), 1);
    }
}