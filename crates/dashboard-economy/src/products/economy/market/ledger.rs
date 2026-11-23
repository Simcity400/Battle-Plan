
use dashboard_base::{Source, catalogue::Catalogue, products::{LoadContext, Product, log_page::{self as logs, LogCell, LogFacetCountAsk, LogPageParams, LogPageReply, LogPageRow, LogTailReply, dated_tail_panel}}, queries::country_ids::Names};
use dashboard_follow::panel::Panel;
use dashboard_world::products::equipment_names as names;
use dashboard_query::{Query, log::{Cell, FacetFilter}};
use replay_data::rows::{MarketEventRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    pub variant: u64,
}

const EQUIPMENT: usize = 8;

fn map(key: u128, cells: &[Cell], columns: &Columns, tag: impl Fn(u32) -> Option<String>) -> Mapped {
    let raw_kind = number(cells, columns.kind).and_then(|kind| u8::try_from(kind).ok());
    let kind = raw_kind.unwrap_or(u8::MAX);
    let ordinal = number(cells, columns.ordinal).and_then(|ordinal| u32::try_from(ordinal).ok()).unwrap_or(0);
    let reason = MarketEventRow { kind, ordinal, ..MarketEventRow::default() }.delivery_unavailable_reason();
    let party = |column: usize| number(cells, column).and_then(|raw| u32::try_from(raw).ok()).filter(|&raw| raw != 0).and_then(&tag);
    let raw_agreement = number(cells, columns.agreement);
    let raw_variant = number(cells, columns.variant);
    let variant = raw_variant.map_or(0, |variant| variant as u64);
    let raw = |column: usize| if reason.is_some() { LogCell::Missing } else { number_cell(number(cells, column)) };
    Mapped {
        cells: vec![
            LogCell::Text { value: key.to_string() },
            number_cell(number(cells, columns.occurrence)),
            raw_kind.map_or(LogCell::NotRecorded, |kind| LogCell::Number { value: kind.to_string() }),
            number_cell(number(cells, columns.mode)),
            raw_agreement.map_or(LogCell::NotRecorded, |agreement| LogCell::Text { value: format!("{:016x}", agreement as u64) }),
            text_cell(party(columns.country)),
            text_cell(party(columns.receiver)),
            raw_variant.map_or(LogCell::NotRecorded, |variant| LogCell::Text { value: (variant as u64).to_string() }),
            LogCell::Missing,
            if variant == 0 { LogCell::Missing } else { LogCell::Text { value: "Equipment not named yet".into() } },
            raw(columns.amount),
            raw(columns.auxiliary),
            number_cell(number(cells, columns.total)),
            number_cell(number(cells, columns.lost_total)),
            text_cell(reason.map(str::to_owned)),
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
        if !out.contains_key(&stretch.run) {
            out.insert(stretch.run, dashboard_base::queries::links(source, stretch.run)?);
        }
    }
    Ok(out)
}

#[derive(Default)]
struct VariantNameCache {
    entries: Option<(Arc<Catalogue>, HashMap<(u32, i32, u64), Option<String>>)>,
}

impl VariantNameCache {
    fn clear(cache: &Mutex<Self>) {
        cache.lock().unwrap_or_else(PoisonError::into_inner).entries = None;
    }

    fn resolve(
        cache: &Mutex<Self>,
        recording: &replay_recording::columnar_frames::Recording,
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
            if let Some((cat, map)) = guard.entries.as_ref()
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
            let resolved = plain_names(recording, run, hour, &missing, context)?;
            {
                let mut guard = cache.lock().unwrap_or_else(PoisonError::into_inner);
                let reuse = guard.entries.as_ref().is_some_and(|(cat, _)| Arc::ptr_eq(cat, &content.catalogue));
                if !reuse {
                    guard.entries = Some((Arc::clone(&content.catalogue), HashMap::new()));
                }
                let (_, map) = guard.entries.as_mut().unwrap();
                for &variant in &missing {
                    let value = resolved.get(&variant).cloned().flatten();
                    map.insert((run, hour, variant), value.clone());
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
    let mut by_run: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut mapped = Vec::with_capacity(rows.len());
    for (at, (row, ..)) in rows.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(stretch) = query.timeline.at(row.hour) else {
            mapped.push(map(row.key, &row.cells, &columns, |_| None));
            continue;
        };
        let run_links = links.get(&stretch.run);
        mapped.push(map(row.key, &row.cells, &columns, |raw| run_links.and_then(|held| tags.tag_at(held, raw, row.hour)).map(str::to_owned)));
        by_run.entry(stretch.run).or_default().push(at);
    }
    let mut warnings = Vec::new();
    match context {
        Err(why) => {
            for row in &mut mapped {
                if row.variant != 0 {
                    row.cells[EQUIPMENT + 1] = LogCell::Text { value: why.clone() };
                }
            }
            warnings.push(why.clone());
        }
        Ok(content) => {
            let recording = query.replay.recording();
            for (run, ats) in by_run {
                let variants: BTreeSet<u64> = ats.iter().map(|&at| mapped[at].variant).filter(|&variant| variant != 0).collect();
                if variants.is_empty() {
                    continue;
                }
                let hours: BTreeSet<i32> = ats.iter().map(|&at| rows[at].0.hour).collect();
                let mut resolved: BTreeMap<u64, String> = BTreeMap::new();
                for hour in [hours.last(), hours.first()].into_iter().flatten() {
                    let wanted: Vec<u64> = variants.iter().copied().filter(|variant| !resolved.contains_key(variant)).collect();
                    if wanted.is_empty() {
                        break;
                    }
                    let batch = match name_cache {
                        Some(cache) => VariantNameCache::resolve(cache, recording, run, *hour, &wanted, content, context)?,
                        None => plain_names(recording, run, *hour, &wanted, context)?,
                    };
                    resolved.extend(batch.into_iter().filter_map(|(id, name)| Some((id, name?))));
                }
                for &at in &ats {
                    let row = &mut mapped[at];
                    match resolved.get(&row.variant) {
                        Some(name) => {
                            row.cells[EQUIPMENT] = LogCell::Text { value: name.clone() };
                            row.cells[EQUIPMENT + 1] = LogCell::Missing;
                        }
                        None if row.variant != 0 => row.cells[EQUIPMENT + 1] = LogCell::Text { value: "The recording does not name this design at the ledger's reads".into() },
                        None => {}
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, facets, from: Start, to: End, cursor, pageSize, "facets[].countries[]", "facets[].values[]"];
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
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
        let (page, rows, missing) = logs::page(&query, &asked, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()))?;
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
    pub limit: u32,
    #[serde(default)]
    pub count_facets: Vec<LogFacetCountAsk>,
}

fn load_tail_at(
    query: &Query<'_>,
    links: &BTreeMap<u32, Arc<dashboard_base::queries::country_ids::RunLinks>>,
    context: &names::Context,
    name_cache: Option<&Mutex<VariantNameCache>>,
    params: &MarketLedgerTailParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<LogTailReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
    let (page, rows, facet_totals, missing) = logs::tail(query, LOG, &params.facets, params.from, hour, params.limit, &params.count_facets, |paged, tag| {
        (paged.row.clone(), paged.ordinal, paged.own, tag.cloned())
    })?;
    let (reply_rows, warnings) = enrich_rows(query, links, context, name_cache, rows, tagged, cancelled)?;
    Ok(LogTailReply {
        branch_id: params.branch_id.clone(),
        log: LOG.to_owned(),
        columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
        base_ordinal: page.base.to_string(),
        first_ordinal: page.first.to_string(),
        total: page.total().to_string(),
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
        limit,
        countFacets,
        "facets[].countries[]",
        "facets[].values[]",
        "countFacets[].facets[].countries[]",
        "countFacets[].facets[].values[]"
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
        load_tail_at(&query, &links, &context, Some(&cache), params, hour, cancelled)
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
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    Ok(dated_tail_panel(executable, move |hour, cancelled| {
        let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
        load_tail_at(&query, &links, &context, Some(&cache), &params, hour, cancelled)
    }))
}

fn plain_names(recording: &replay_recording::columnar_frames::Recording, run: u32, hour: i32, variants: &[u64], context: &names::Context) -> io::Result<BTreeMap<u64, Option<String>>> {
    let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
    let mut reads: Vec<replay_lanes::at::Read<'_>> = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance].into_iter().map(replay_lanes::at::Read::all).collect();
    reads.push(replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: replay_lanes::at::Keys::These(&keys) });
    let mut frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &reads))?;
    let tokens: Vec<u128> = names::variant_tokens(&frame).collect::<BTreeSet<_>>().into_iter().map(u128::from).collect();
    let token_read = [replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay_lanes::at::Keys::These(&tokens) }];
    let lexical = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &token_read))?;
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
        assert_eq!((mapped.variant, cell(&mapped, "variant")), (6, LogCell::Text { value: "6".into() }));
        assert!(matches!(cell(&mapped, "equipmentMissing"), LogCell::Text { .. }), "a design is unnamed until the page names it, and says so");
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
        let facets = vec![FacetFilter { name: "kind".into(), values: vec![20], countries: Vec::new() }];
        let page = MarketLedger::load(
            &source,
            &MarketLedgerParams { branch_id: "1".into(), facets: facets.clone(), from: None, to: 110.0, cursor: None, page_size: 64 },
            &load,
            &AtomicBool::new(false),
        )
        .unwrap();
        let tail = MarketLedgerTail::load(
            &source,
            &MarketLedgerTailParams { branch_id: "1".into(), facets: facets.clone(), from: None, hour: Some(110), limit: 64, count_facets: Vec::new() },
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
        let tail_params = MarketLedgerTailParams { branch_id: "1".into(), facets, from: None, hour: None, limit: 64, count_facets: Vec::new() };
        for hour in [110, 104, 112, 110] {
            let cached = load_tail_at(&query, &links, &context, Some(&cache), &tail_params, hour, &AtomicBool::new(false)).unwrap();
            let uncached = load_tail_at(&query, &links, &context, None, &tail_params, hour, &AtomicBool::new(false)).unwrap();
            assert_eq!(cached, uncached, "cached vs uncached at {hour}");
        }
    }
}