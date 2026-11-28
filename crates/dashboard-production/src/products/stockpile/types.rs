
use super::{Content, DECIMALS, NEED_BOUND_RAW, Named, StockType, Types, daily_units_raw, design_reads, history, name_at, named_in, spelling_read};
use crate::products::production::LineKind;
use dashboard_base::products::{Country, LoadContext, Product, at_hour::{self, At, AtHour, Edits, Keys, Owner, Read, ReadsAt}, validate_country};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::{coverage::witnessed_selection, identity::Holder};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{LogisticsEquipmentRow, ProductionLineRow, Recorded, StockByVariantRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
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
    pub rows_missing: Option<String>,
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
    Player(BTreeMap<String, Vec<Logistic>>, Option<String>),
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

fn sum(figures: impl Iterator<Item = StockFigure>, blocked: bool, missing: Option<&str>) -> StockFigure {
    if blocked {
        return StockFigure::missing(UNTYPED_HOLDS);
    }
    if let Some(why) = missing {
        return StockFigure::missing(why);
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
    if let Need::Player(by_key, _) = input.need {
        extra.extend(by_key.keys().filter(|key| !listed.contains(*key)).cloned());
    }
    keys.extend(extra.into_iter().map(|key| (key, None)));
    let mut out = Vec::new();
    for (place, (key, kind)) in keys.into_iter().enumerate() {
        let members: Vec<&Held> = input.variants.iter().filter(|held| type_key(held) == key).collect();
        let resolved = kind.is_some();
        let blocked = resolved && untyped_blocks;
        let stored_total = if resolved { sum(members.iter().map(|held| stored(held, None)), blocked, input.stock_missing) } else { StockFigure::missing(UNRESOLVED_TYPE) };
        let produced_total = if resolved { sum(members.iter().map(|held| produced(held, None)), blocked, input.lines_missing) } else { StockFigure::missing(UNRESOLVED_TYPE) };
        let (balance, need, listed_row) = match input.need {
            Need::Ai => (StockFigure::missing(AI_NEED), StockFigure::missing(AI_NEED), false),
            Need::Unknown(why) => (StockFigure::missing(why), StockFigure::missing(why), false),
            Need::Player(by_key, missing) => {
                let held = by_key.get(&key);
                let note = kind.map_or(Some(NEED_UNAVAILABLE), |kind| kind.need_note.as_deref());
                let (balance, need) = match missing {
                    Some(why) => (StockFigure::missing(why), StockFigure::missing(why)),
                    None => logistics(held, note),
                };
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

fn stock_reads(Owner { slot, id }: Owner) -> [Read; 5] {
    let range = |table, low: u128, high: u128| Read { table, keys: Keys::Range(low, high) };
    [
        range(TableId::StockByVariant, u128::from(slot) << 64, u128::from(slot) << 64 | u128::from(u64::MAX)),
        range(TableId::Stockpiles, u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)),
        range(TableId::LogisticsEquipment, u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)),
        Read::all(TableId::ProductionLines),
        Read::all(TableId::CountryLinks),
    ]
}

struct Observed<'a> {
    stock_rows: Vec<&'a StockByVariantRow>,
    lines: Vec<&'a ProductionLineRow>,
    logistics_rows: Vec<&'a LogisticsEquipmentRow>,
    stock_missing: Option<String>,
    lines_missing: Option<String>,
    logistics_missing: Option<String>,
}

impl Observed<'_> {
    fn variants(&self) -> Vec<u64> {
        self.stock_rows.iter().map(|row| row.variant).chain(self.lines.iter().map(|line| line.variant))
            .collect::<BTreeSet<_>>().into_iter().collect()
    }

    fn tokens(&self) -> BTreeSet<u32> {
        self.logistics_rows.iter().map(|row| row.token).collect()
    }
}

fn observed(frame: &DecodedFrame, Owner { slot, id }: Owner) -> Observed<'_> {
    let links = witnessed_selection::<tables::CountryLinks>(frame);
    let raws: BTreeSet<u32> = links.rows.iter().filter(|link| link.canonical_slot == slot).map(|link| link.id).collect();
    let stock = witnessed_selection::<tables::StockByVariant>(frame);
    let stock_missing = stock.missing.or_else(|| stale(frame, TableId::Stockpiles, "stockpile"));
    let stock_rows = stock.rows.iter().filter(|row| row.slot == slot).collect();
    let production = witnessed_selection::<tables::ProductionLines>(frame);
    let lines_missing = production.missing.or(links.missing).or_else(|| stale(frame, TableId::ProductionLines, "production lines"));
    let lines = production.rows.iter().filter(|line| raws.contains(&line.country)).collect();
    let (logistics_rows, logistics_missing) = logistics_at(frame, id);
    Observed { stock_rows, lines, logistics_rows, stock_missing, lines_missing, logistics_missing }
}

fn logistics_at(frame: &DecodedFrame, id: u32) -> (Vec<&LogisticsEquipmentRow>, Option<String>) {
    let logistics = witnessed_selection::<tables::LogisticsEquipment>(frame);
    let logistics_stale = stale(frame, TableId::LogisticsEquipment, "logistics record");
    let logistics_rows = logistics.rows.iter().filter(|row| row.country == id && logistics_stale.is_none()).collect();
    (logistics_rows, logistics.missing.or(logistics_stale))
}

fn projected(
    Observed { stock_rows, lines, logistics_rows, stock_missing, lines_missing, logistics_missing }: Observed<'_>,
    (hour, valid_until): (i32, i32), content: &Content, ids: Vec<u64>,
    (mut named, spelled): (BTreeMap<u64, Named>, BTreeMap<u32, String>),
    refused: BTreeMap<String, usize>,
) -> StockpileTypesReply {
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
            archetype: types.variant(content, &token),
            token,
            name: name.as_ref().and_then(|name| name.name.clone()),
            origin: name.and_then(|name| name.origin),
            stock,
            daily,
        });
    }
    let rows_missing = stock_missing.clone().or_else(|| lines_missing.clone()).or_else(|| logistics_missing.clone());
    let need = match (logistics_rows.is_empty(), logistics_missing) {
        (true, Some(why)) => Need::Unknown(why),
        (true, None) => Need::Ai,
        (false, missing) => {
            let mut by_key: BTreeMap<String, Vec<Logistic>> = BTreeMap::new();
            for row in &logistics_rows {
                let spelling = spelled.get(&row.token).cloned().unwrap_or_else(|| format!("token {}", row.token));
                let available = match row.available_raw {
                    Recorded::Value(value) => value,
                    Recorded::NotRecorded => None,
                };
                by_key.entry(types.logistics(content, &spelling)).or_default().push(Logistic { balance: row.balance_raw, stock: row.stock_raw, available });
            }
            Need::Player(by_key, missing)
        }
    };
    for (why, count) in refused {
        warnings.push(format!("{count} equipment designs with unknown text or design operands: {why}"));
    }
    let kinds = types.finish(content, &mut warnings);
    let rows = rows(&Inputs { types: &kinds, variants: &variants, need: &need, stock_missing: stock_missing.as_deref(), lines_missing: lines_missing.as_deref() });
    StockpileTypesReply {
        hour,
        valid_until,
        decimals: DECIMALS,
        need_missing: match &need {
            Need::Player(_, missing) => missing.clone(),
            Need::Ai => Some(AI_NEED.to_owned()),
            Need::Unknown(why) => Some(why.clone()),
        },
        rows,
        rows_missing,
        warnings,
    }
}

const COUNTRY_UNAVAILABLE: &str = "Equipment stockpile country is unavailable at this hour";

fn load(source: &Source, params: &StockpileTypesParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<StockpileTypesReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let hour = params.hour;
    let run = query.holders.at(&params.country, hour).copied().ok_or_else(|| dashboard_core::refused(COUNTRY_UNAVAILABLE))?;
    let Holder::Found { slot, id } = run.holder else { return Err(dashboard_core::refused(query.holders.not_known(&params.country, hour).unwrap_or_else(|| COUNTRY_UNAVAILABLE.to_owned()))) };
    let replay = history::kept(source, &query, &params.country)?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let content = Content::load(source, context)?;
    let owner = Owner { slot, id };
    let frame = at_hour::frame(&query, hour, &stock_reads(owner), "Equipment stockpile")?;
    let observed = observed(&frame, owner);
    let ids = observed.variants();
    let mut refused = BTreeMap::new();
    let named = name_at(&query, hour, &ids, &observed.tokens(), &content, &mut refused)?;
    let through = valid_until(&replay, hour, run.last, query.timeline.last);
    Ok(projected(observed, (hour, through), &content, ids, named, refused))
}

pub struct EquipmentStockpileTypes;

pub fn panel(source: &Arc<Source>, context: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn dashboard_follow::panel::Panel>, String> {
    let panel = at_hour::exact_panel::<FollowedTypes>(source, context, asked)?;
    let exact = panel.exact().expect("an AtHour panel has exact readers");
    let branch = dashboard_query::branch(asked).map_err(|error| error.to_string())?;
    let timeline = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?.timeline;
    let recorded = Arc::clone(&timeline);
    let read = Arc::clone(&exact.advance);
    let unavailable = Arc::new(Err(COUNTRY_UNAVAILABLE.to_owned()));
    let advance: dashboard_follow::panel::ProductReader = Arc::new(move |hour, values, cancelled, step| {
        let mut answer = read(hour, values, cancelled, step);
        if recorded.at(hour).is_none() { answer.value = Arc::clone(&unavailable); }
        answer
    });
    let read = Arc::clone(&exact.cold);
    let cold = Arc::new(move |hour, cancelled: &AtomicBool| {
        if timeline.at(hour).is_none() {
            Err(COUNTRY_UNAVAILABLE.to_owned())
        } else {
            read(hour, cancelled)
        }
    });
    Ok(at_hour::product_panel(advance, cold))
}

struct FollowedTypes;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FollowedParams {
    branch_id: String,
    country: Country,
}

struct FollowedContext {
    replay: Arc<history::Replay>,
    content: Content,
    last: i32,
}

impl AtHour for FollowedTypes {
    const KEY: &'static str = EquipmentStockpileTypes::KEY;
    const TITLE: &'static str = "Equipment stockpile";
    const CONTENT_DEPENDENT: bool = EquipmentStockpileTypes::CONTENT_DEPENDENT;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Params = FollowedParams;
    type Reply = StockpileTypesReply;
    type Context = io::Result<FollowedContext>;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> { Some(&params.country) }

    fn tables(_: &Self::Params) -> Vec<TableId> {
        use TableId::*;
        vec![StockByVariant, Stockpiles, LogisticsEquipment, ProductionLines, CountryLinks, Countries,
            CountryAppearance, VariantDefinitions, VariantModules, VariantUpgrades, LexicalTokens]
    }

    fn reads_in(_: &Self::Params, owner: Option<Owner>, stage: usize, frame: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let Some(owner) = owner.filter(|_| at.context.is_ok()) else { return Vec::new() };
        match stage {
            0 => stock_reads(owner).into_iter().chain([Read::all(TableId::Countries), Read::all(TableId::CountryAppearance)]).collect(),
            1 => design_reads(observed(frame, owner).variants()).into(),
            2 => vec![spelling_read(frame, &logistics_at(frame, owner.id).0.iter().map(|row| row.token).collect())],
            _ => Vec::new(),
        }
    }

    fn plan_changes(stage: usize, edits: &Edits, _: &DecodedFrame, _: &ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => [TableId::CountryLinks, TableId::ProductionLines, TableId::StockByVariant].into_iter().any(|table| edits.fields(table, u64::MAX)),
            2 => true,
            _ => false,
        }
    }

    fn context(_: &Source, _: &LoadContext) -> io::Result<Self::Context> {
        Err(dashboard_core::invalid("Equipment stockpile needs a branch and country"))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        Ok(history::kept(source, &query, &params.country).and_then(|replay| {
            Ok(FollowedContext { replay, content: Content::load(source, load)?, last: query.timeline.last })
        }))
    }

    fn valid_reply_through(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Self::Context>, _: &Self::Reply, _: bool) -> i32 {
        frame.hour
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused(COUNTRY_UNAVAILABLE))?;
        let context = at.context.as_ref().map_err(dashboard_core::duplicate)?;
        let hour = frame.hour;
        let last = at.holders.at(&params.country, hour).map_or(hour, |run| run.last);
        let observed = observed(frame, owner);
        let ids = observed.variants();
        let mut refused = BTreeMap::new();
        let content = &context.content;
        let named = named_in(frame, &ids, &observed.tokens(), content, &mut refused);
        let through = valid_until(&context.replay, hour, last, context.last);
        Ok(projected(observed, (hour, through), content, ids, named, refused))
    }
}

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
