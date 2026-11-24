
use super::{Content, DECIMALS, NAME_TABLES, NEED_BOUND_RAW, StockType, Types, daily_units_raw, history, name_at};
use crate::products::production::LineKind;
use dashboard_base::products::{Country, LoadContext, Product, at_hour::{self, Keys, Read}, validate_country};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::identity::Holder;
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{Recorded, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const AI_NEED: &str = "The game doesn't compute surplus or shortage for AI-controlled countries.";
const NO_LOGISTICS: &str = "The game's logistics lists this type for none of the country's needs at this hour.";
const NEED_UNAVAILABLE: &str = "Need unavailable.";
const UNTYPED_HOLDS: &str = "Designs with no recorded type hold or make some of this; its total may miss them.";
const UNRESOLVED_TYPE: &str = "This equipment type is not resolved in the installed content.";
const STOCK_UNRECORDED: &str = "A design's stock was not recorded at this hour.";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileTypesParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockFigure {
    #[doc = " Scaled by the reply's decimals; null with `missing` where it is not known."]
    pub raw: Option<String>,
    pub missing: Option<String>,
}

impl StockFigure {
    fn known(raw: i128) -> Self {
        Self { raw: Some(raw.to_string()), missing: None }
    }

    fn missing(why: &str) -> Self {
        Self { raw: None, missing: Some(why.to_owned()) }
    }

    fn value(&self) -> Option<i128> {
        self.raw.as_deref().and_then(|raw| raw.parse().ok())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockVariantRow {
    pub id: String,
    pub name: Option<String>,
    #[doc = " The tag of the country whose design it is; null where the definition names none."]
    pub origin: Option<String>,
    pub stored: StockFigure,
    pub produced: StockFigure,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockTypeRow {
    #[doc = " The archetype key, or the unresolved mark and the recorded spelling."]
    pub key: String,
    #[doc = " The localised name; null where the content names none."]
    pub name: Option<String>,
    pub stored: StockFigure,
    #[doc = " A day."]
    pub produced: StockFigure,
    #[doc = " A day, the game's logistics need."]
    pub need: StockFigure,
    #[doc = " A day, the game's logistics balance."]
    pub balance: StockFigure,
    #[doc = " The balance as a share of the need; null without both or with no need."]
    pub share: Option<f64>,
    #[doc = " In the product's order: the most stored first."]
    pub variants: Vec<StockVariantRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileTypesReply {
    pub hour: i32,
    #[doc = " Nothing shown changes from `hour` through this hour."]
    pub valid_until: i32,
    pub decimals: u8,
    #[doc = " Why need and balance are not known for this country at this hour; null where they are."]
    pub need_missing: Option<String>,
    #[doc = " Shortages first, worst first, then surpluses, largest first, then the rest in the equipment database's order."]
    pub rows: Vec<StockTypeRow>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StockpileTypesReply);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Daily {
    Idle,
    Units(i128),
    Missing(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Held {
    pub id: u64,
    pub archetype: Option<String>,
    pub token: String,
    pub name: Option<String>,
    pub origin: Option<String>,
    pub stock: Option<Option<i64>>,
    pub daily: Daily,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Logistic {
    pub balance: i64,
    pub stock: i64,
    pub available: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Need {
    Player(BTreeMap<String, Vec<Logistic>>),
    Ai,
    Unknown(String),
}

pub(super) struct Inputs<'a> {
    pub types: &'a [StockType],
    pub variants: &'a [Held],
    pub need: &'a Need,
    pub stock_missing: Option<&'a str>,
    pub lines_missing: Option<&'a str>,
}

fn type_key(held: &Held) -> String {
    held.archetype.clone().unwrap_or_else(|| format!("{}{}", super::UNRESOLVED_MARK, if held.token.is_empty() { held.id.to_string() } else { held.token.clone() }))
}

fn stored(held: &Held, missing: Option<&str>) -> StockFigure {
    match (missing, held.stock) {
        (Some(why), _) => StockFigure::missing(why),
        (None, None) => StockFigure::known(0),
        (None, Some(None)) => StockFigure::missing(STOCK_UNRECORDED),
        (None, Some(Some(amount))) => StockFigure::known(i128::from(amount)),
    }
}

fn produced(held: &Held, missing: Option<&str>) -> StockFigure {
    match (missing, &held.daily) {
        (Some(why), _) => StockFigure::missing(why),
        (None, Daily::Idle) => StockFigure::known(0),
        (None, Daily::Units(units)) => StockFigure::known(*units),
        (None, Daily::Missing(why)) => StockFigure::missing(why),
    }
}

fn sum(figures: impl Iterator<Item = StockFigure>, blocked: bool) -> StockFigure {
    if blocked {
        return StockFigure::missing(UNTYPED_HOLDS);
    }
    let mut total = 0_i128;
    for figure in figures {
        match figure.value() {
            Some(value) => total += value,
            None => return figure,
        }
    }
    StockFigure::known(total)
}

fn holds(figure: &StockFigure) -> bool {
    figure.value() != Some(0)
}

fn logistics(rows: Option<&Vec<Logistic>>, note: Option<&str>) -> (StockFigure, StockFigure) {
    let Some(rows) = rows.filter(|rows| !rows.is_empty()) else {
        return (StockFigure::missing(NO_LOGISTICS), StockFigure::missing(NO_LOGISTICS));
    };
    let bound = NEED_BOUND_RAW.unsigned_abs();
    let (mut balance, mut need, mut why) = (0_i128, 0_i128, None);
    for row in rows {
        balance += i128::from(row.balance);
        let note = if row.available.is_some() { None } else { note };
        let native = row.available.unwrap_or(row.stock.max(0));
        let one = native.wrapping_sub(row.balance);
        match note {
            Some(note) => why = why.or(Some(note)),
            None if one.unsigned_abs() >= bound => why = why.or(Some(NEED_UNAVAILABLE)),
            None => need += i128::from(one),
        }
    }
    (StockFigure::known(balance), why.map_or_else(|| StockFigure::known(need), StockFigure::missing))
}

fn variant_order(a: &StockVariantRow, b: &StockVariantRow) -> Ordering {
    match (a.stored.value(), b.stored.value()) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| a.name.cmp(&b.name))
    .then_with(|| a.id.cmp(&b.id))
}

fn class(row: &StockTypeRow) -> (u8, f64) {
    match (row.share, row.balance.value()) {
        (Some(share), Some(balance)) if balance < 0 => (0, share),
        (Some(share), Some(_)) => (1, -share),
        _ => (2, 0.0),
    }
}

pub(super) fn rows(input: &Inputs<'_>) -> Vec<StockTypeRow> {
    let untyped_blocks = input.variants.iter().filter(|held| held.archetype.is_none()).any(|held| (held.stock.is_some() && holds(&stored(held, None))) || holds(&produced(held, None)));
    let mut keys: Vec<(String, Option<&StockType>)> = input.types.iter().map(|kind| (kind.key.clone(), Some(kind))).collect();
    let listed: BTreeSet<String> = keys.iter().map(|(key, _)| key.clone()).collect();
    let mut extra: BTreeSet<String> = input.variants.iter().map(type_key).filter(|key| !listed.contains(key)).collect();
    if let Need::Player(by_key) = input.need {
        extra.extend(by_key.keys().filter(|key| !listed.contains(*key)).cloned());
    }
    keys.extend(extra.into_iter().map(|key| (key, None)));
    let mut out = Vec::new();
    for (place, (key, kind)) in keys.into_iter().enumerate() {
        let members: Vec<&Held> = input.variants.iter().filter(|held| type_key(held) == key).collect();
        let resolved = kind.is_some();
        let blocked = resolved && untyped_blocks;
        let stored_total = if resolved { sum(members.iter().map(|held| stored(held, input.stock_missing)), blocked) } else { StockFigure::missing(UNRESOLVED_TYPE) };
        let produced_total = if resolved { sum(members.iter().map(|held| produced(held, input.lines_missing)), blocked) } else { StockFigure::missing(UNRESOLVED_TYPE) };
        let (balance, need, listed_row) = match input.need {
            Need::Ai => (StockFigure::missing(AI_NEED), StockFigure::missing(AI_NEED), false),
            Need::Unknown(why) => (StockFigure::missing(why), StockFigure::missing(why), false),
            Need::Player(by_key) => {
                let held = by_key.get(&key);
                let note = kind.map_or(Some(NEED_UNAVAILABLE), |kind| kind.need_note.as_deref());
                let (balance, need) = logistics(held, note);
                (balance, need, held.is_some_and(|rows| rows.iter().any(|row| row.balance != 0 || row.stock != 0 || row.available.is_some_and(|value| value != 0))))
            }
        };
        let shown = members.iter().any(|held| (held.stock.is_some() && holds(&stored(held, None))) || holds(&produced(held, None))) || listed_row;
        if !shown {
            continue;
        }
        let share = match (balance.value(), need.value()) {
            (Some(balance), Some(need)) if need > 0 => Some(balance as f64 / need as f64),
            _ => None,
        };
        let mut variants: Vec<StockVariantRow> = members
            .iter()
            .filter(|held| held.stock.is_some() || held.daily != Daily::Idle)
            .map(|held| StockVariantRow {
                id: held.id.to_string(),
                name: held.name.clone(),
                origin: held.origin.clone(),
                stored: stored(held, input.stock_missing),
                produced: produced(held, input.lines_missing),
            })
            .collect();
        variants.sort_by(variant_order);
        out.push((place, StockTypeRow { key, name: kind.and_then(|kind| kind.name.clone()), stored: stored_total, produced: produced_total, need, balance, share, variants }));
    }
    out.sort_by(|(a_place, a), (b_place, b)| {
        let (a_class, a_rank) = class(a);
        let (b_class, b_rank) = class(b);
        a_class.cmp(&b_class).then_with(|| a_rank.partial_cmp(&b_rank).unwrap_or(Ordering::Equal)).then_with(|| a_place.cmp(b_place))
    });
    out.into_iter().map(|(_, row)| row).collect()
}

fn next_after(hours: &[i32], hour: i32) -> Option<i32> {
    hours.get(hours.partition_point(|at| *at <= hour)).copied()
}

fn edges(spans: &[HourSpan], hour: i32) -> impl Iterator<Item = i32> + '_ {
    spans.iter().flat_map(|span| [span.first_hour, span.last_hour.saturating_add(1)]).filter(move |at| *at > hour)
}

pub(super) fn valid_until(replay: &history::Replay, hour: i32, holder_last: i32, timeline_last: i32) -> i32 {
    let reads = [&replay.reads, &replay.line_reads, &replay.logistics_reads].into_iter().filter_map(|hours| next_after(hours, hour));
    let spans = [&replay.stock, &replay.lines, &replay.balance, &replay.ai].into_iter().flat_map(|spans| edges(spans, hour));
    let next = reads.chain(spans).chain([holder_last.saturating_add(1), timeline_last.saturating_add(1)]).min().unwrap_or(hour.saturating_add(1));
    next.saturating_sub(1).max(hour)
}

fn stale(frame: &DecodedFrame, table: TableId, what: &str) -> Option<String> {
    let read = frame.tables[table.index()].sampled_at()?;
    (i64::from(frame.hour) - i64::from(read) >= i64::from(STAGGER_PERIOD)).then(|| format!("The {what} was not read within a day of this hour."))
}

fn load(source: &Source, params: &StockpileTypesParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<StockpileTypesReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let hour = params.hour;
    let run = query.holders.at(&params.country, hour).copied().ok_or_else(|| dashboard_core::refused("Equipment stockpile country is unavailable at this hour"))?;
    let Holder::Found { slot, id } = run.holder else { return Err(dashboard_core::refused("Equipment stockpile country is unavailable at this hour")) };
    let replay = history::kept(source, &query, &params.country)?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let content = Content::load(source, context)?;
    let range = |table, low: u128, high: u128| Read { table, keys: Keys::Range(low, high) };
    let mut reads = vec![
        range(TableId::StockByVariant, u128::from(slot) << 64, u128::from(slot) << 64 | u128::from(u64::MAX)),
        range(TableId::Stockpiles, u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)),
        range(TableId::LogisticsEquipment, u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)),
        Read::all(TableId::ProductionLines),
        Read::all(TableId::CountryLinks),
    ];
    reads.extend(NAME_TABLES.map(Read::all));
    let frame = at_hour::frame(&query, hour, &reads, "Equipment stockpile")?;
    let raws: BTreeSet<u32> = frame.complete::<tables::CountryLinks>().map(|links| links.iter().filter(|link| link.canonical_slot == slot).map(|link| link.id).collect()).unwrap_or_default();
    let stock_missing = match frame.complete::<tables::StockByVariant>() {
        Err(error) => Some(format!("The stockpile is not known at this hour: {error}")),
        Ok(_) => stale(&frame, TableId::Stockpiles, "stockpile"),
    };
    let stock_rows: Vec<_> = frame.complete::<tables::StockByVariant>().map(|rows| rows.iter().filter(|row| row.slot == slot).cloned().collect()).unwrap_or_default();
    let (lines_missing, lines) = match frame.complete::<tables::ProductionLines>() {
        Err(error) => (Some(format!("Production lines are not known at this hour: {error}")), Vec::new()),
        Ok(rows) => (stale(&frame, TableId::ProductionLines, "production lines"), rows.iter().filter(|line| raws.contains(&line.country)).cloned().collect()),
    };
    let (logistics_state, logistics_rows) = match frame.complete::<tables::LogisticsEquipment>() {
        Err(error) => (Err(format!("The logistics record is not known at this hour: {error}")), Vec::new()),
        Ok(rows) => match stale(&frame, TableId::LogisticsEquipment, "logistics record") {
            Some(why) => (Err(why), Vec::new()),
            None => {
                let held: Vec<_> = rows.iter().filter(|row| row.country == id).cloned().collect();
                (Ok(!held.is_empty()), held)
            }
        },
    };
    let mut ids: BTreeSet<u64> = stock_rows.iter().map(|row| row.variant).collect();
    ids.extend(lines.iter().map(|line| line.variant));
    let ids: Vec<u64> = ids.into_iter().collect();
    let tokens: BTreeSet<u32> = logistics_rows.iter().map(|row| row.token).collect();
    let mut refused = BTreeMap::new();
    let (mut named, spelled) = name_at(&query, hour, &ids, &tokens, &content, &mut refused)?;
    let mut warnings = content.warnings.clone();
    let mut types = Types::default();
    let mut variants = Vec::with_capacity(ids.len());
    for variant in ids {
        let name = named.remove(&variant).filter(|name| name.known);
        let token = name.as_ref().map(|name| name.token.clone()).unwrap_or_default();
        let mut daily = Daily::Idle;
        for line in lines.iter().filter(|line| line.variant == variant) {
            daily = match (daily, LineKind::of(i64::from(line.kind))) {
                (Daily::Missing(why), _) => Daily::Missing(why),
                (before, Some(LineKind::Military)) => match daily_units_raw(line.speed_raw, line.cost_raw) {
                    Some(units) => Daily::Units((if let Daily::Units(held) = before { held } else { 0 }) + i128::from(units)),
                    None => Daily::Missing("A line making it has no positive cost, so its output a day is unknown.".to_owned()),
                },
                _ => Daily::Missing("A naval, refit or railway-gun line makes it: its output a day is not established.".to_owned()),
            };
        }
        let stock = stock_rows.iter().find(|row| row.variant == variant).map(|row| match row.amount {
            Recorded::Value(amount) => Some(amount),
            Recorded::NotRecorded => None,
        });
        variants.push(Held {
            id: variant,
            archetype: types.variant(&content, &token),
            token,
            name: name.as_ref().and_then(|name| name.name.clone()),
            origin: name.and_then(|name| name.origin),
            stock,
            daily,
        });
    }
    let need = match logistics_state {
        Err(why) => Need::Unknown(why),
        Ok(false) => Need::Ai,
        Ok(true) => {
            let mut by_key: BTreeMap<String, Vec<Logistic>> = BTreeMap::new();
            for row in &logistics_rows {
                let spelling = spelled.get(&row.token).cloned().unwrap_or_else(|| format!("token {}", row.token));
                let available = match row.available_raw {
                    Recorded::Value(value) => value,
                    Recorded::NotRecorded => None,
                };
                by_key.entry(types.logistics(&content, &spelling)).or_default().push(Logistic { balance: row.balance_raw, stock: row.stock_raw, available });
            }
            Need::Player(by_key)
        }
    };
    for (why, count) in refused {
        warnings.push(format!("{count} equipment designs with unknown text or design operands: {why}"));
    }
    let kinds = types.finish(&content, &mut warnings);
    let rows = rows(&Inputs { types: &kinds, variants: &variants, need: &need, stock_missing: stock_missing.as_deref(), lines_missing: lines_missing.as_deref() });
    Ok(StockpileTypesReply {
        hour,
        valid_until: valid_until(&replay, hour, run.last, query.timeline.last),
        decimals: DECIMALS,
        need_missing: match &need {
            Need::Player(_) => None,
            Need::Ai => Some(AI_NEED.to_owned()),
            Need::Unknown(why) => Some(why.clone()),
        },
        rows,
        warnings,
    })
}

pub struct EquipmentStockpileTypes;

impl Product for EquipmentStockpileTypes {
    const KEY: &'static str = "equipment-stockpile-types";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StockpileTypesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = StockpileTypesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StockpileTypesParams::decl(), StockFigure::decl(), StockVariantRow::decl(), StockTypeRow::decl(), StockpileTypesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        load(source, params, context, cancelled)
    }
}
