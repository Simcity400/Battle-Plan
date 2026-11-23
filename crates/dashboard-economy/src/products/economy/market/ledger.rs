
use dashboard_base::{Source, products::{LoadContext, Product, log_page::{self as logs, LogCell, LogPageParams, LogPageReply, LogPageRow}}, queries::country_ids::Names};
use dashboard_world::products::equipment_names as names;
use dashboard_query::log::{Cell, FacetFilter};
use replay_data::rows::{MarketEventRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
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
    let kind = number(cells, columns.kind).and_then(|kind| u8::try_from(kind).ok()).unwrap_or(u8::MAX);
    let ordinal = number(cells, columns.ordinal).and_then(|ordinal| u32::try_from(ordinal).ok()).unwrap_or(0);
    let reason = MarketEventRow { kind, ordinal, ..MarketEventRow::default() }.delivery_unavailable_reason();
    let party = |column: usize| number(cells, column).and_then(|raw| u32::try_from(raw).ok()).filter(|&raw| raw != 0).and_then(&tag);
    let variant = number(cells, columns.variant).map_or(0, |variant| variant as u64);
    let raw = |column: usize| if reason.is_some() { LogCell::Missing } else { number_cell(number(cells, column)) };
    Mapped {
        cells: vec![
            LogCell::Text { value: key.to_string() },
            number_cell(number(cells, columns.occurrence)),
            LogCell::Number { value: kind.to_string() },
            number_cell(number(cells, columns.mode)),
            LogCell::Text { value: format!("{:016x}", number(cells, columns.agreement).unwrap_or(0) as u64) },
            text_cell(party(columns.country)),
            text_cell(party(columns.receiver)),
            LogCell::Text { value: variant.to_string() },
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
        let kinds = params.facets.iter().find(|facet| facet.name == "kind").ok_or("A market ledger page names the occurrences it shows")?;
        if kinds.values.is_empty() || kinds.values.iter().any(|kind| !KINDS.contains(kind)) {
            return Err("Invalid market occurrence kind");
        }
        Ok(())
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
            let links = dashboard_base::queries::links(source, stretch.run)?;
            mapped.push(map(row.key, &row.cells, &columns, |raw| tags.tag_at(&links, raw, row.hour).map(str::to_owned)));
            by_run.entry(stretch.run).or_default().push(at);
        }
        let mut warnings = Vec::new();
        let context = names::context(source, load)?;
        match &context {
            Err(why) => {
                for row in &mut mapped {
                    if row.variant != 0 {
                        row.cells[EQUIPMENT + 1] = LogCell::Text { value: why.clone() };
                    }
                }
                warnings.push(why.clone());
            }
            Ok(_) => {
                let recording = query.replay.recording();
                for (run, ats) in by_run {
                    let variants: BTreeSet<u64> = ats.iter().map(|&at| mapped[at].variant).filter(|&variant| variant != 0).collect();
                    if variants.is_empty() {
                        continue;
                    }
                    let hours: BTreeSet<i32> = ats.iter().map(|&at| rows[at].0.hour).collect();
                    let mut names: BTreeMap<u64, String> = BTreeMap::new();
                    for hour in [hours.last(), hours.first()].into_iter().flatten() {
                        let wanted: Vec<u64> = variants.iter().copied().filter(|variant| !names.contains_key(variant)).collect();
                        if wanted.is_empty() {
                            break;
                        }
                        names.extend(plain_names(recording, run, *hour, &wanted, &context)?.into_iter().filter_map(|(id, name)| Some((id, name?))));
                    }
                    for &at in &ats {
                        let row = &mut mapped[at];
                        match names.get(&row.variant) {
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
        Ok(LogPageReply {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows: rows
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
                .collect(),
            missing,
            warnings,
        })
    }
}

fn plain_names(recording: &replay_recording::columnar_frames::Recording, run: u32, hour: i32, variants: &[u64], context: &names::Context) -> io::Result<BTreeMap<u64, Option<String>>> {
    let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
    let mut reads: Vec<replay_lanes::at::Read<'_>> = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens].into_iter().map(replay_lanes::at::Read::all).collect();
    reads.push(replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: replay_lanes::at::Keys::These(&keys) });
    let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &reads))?;
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
}