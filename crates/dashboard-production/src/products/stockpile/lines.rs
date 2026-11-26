use super::{
    DECIMALS, Domain,
    catalogue::{Balance, Drawn, Kind, Lines, Model, Piece, StockpileMeasure, drawn, model},
    history::Replay,
};
use dashboard_base::products::{Country, LoadContext, Product};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, merge},
    series::MissingSpan,
    tile::{self, BINS, SeriesTileReply, TileLine, TileRanking},
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "equipment-stockpile";

pub(super) const NOT_HELD: &str = "The selected country does not hold this tag at this hour.";
pub(super) const STOCK_UNREAD: &str = "The stockpile was not read at this hour.";
pub(super) const STOCK_UNKNOWN: &str = "The stock of an equipment model of this type is not known at this hour.";
pub(super) const AI_BALANCE: &str = "The game does not compute surplus or shortage for AI-controlled countries.";
pub(super) const BALANCE_UNREAD: &str = "Surplus or shortage was not read at this hour.";
pub(super) const NEED_UNKNOWN: &str = "The need for this type is not known at this hour.";
pub(super) const NO_NEED: &str = "This type has no need at this hour, so a surplus has no share of it.";
const NO_TYPE: &str = "This campaign has no such equipment type.";
const SHARE_DECIMALS: u8 = 2;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Stock,
    Share,
}

impl Shown {
    fn of(measure: StockpileMeasure) -> io::Result<Self> {
        match measure {
            StockpileMeasure::Stock => Ok(Self::Stock),
            StockpileMeasure::Share => Ok(Self::Share),
            StockpileMeasure::Need => Err(invalid("The stockpile over time shows stock or share of need")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::Share => "share",
        }
    }

    fn decimals(self) -> u8 {
        match self {
            Self::Stock => DECIMALS,
            Self::Share => SHARE_DECIMALS,
        }
    }
}

fn piece_at<T>(pieces: &[Piece<T>], hour: i32) -> Option<&Piece<T>> {
    pieces.get(pieces.partition_point(|piece| piece.last < hour)).filter(|piece| piece.first <= hour)
}

fn within(spans: &[HourSpan], hour: i32) -> bool {
    spans.get(spans.partition_point(|span| span.last_hour < hour)).is_some_and(|span| span.first_hour <= hour)
}

fn held(membership: Option<&[HourSpan]>, hour: i32) -> bool {
    membership.is_none_or(|spans| within(spans, hour))
}

fn stock_value(raw: i128) -> f64 {
    raw as f64 / 10_f64.powi(i32::from(DECIMALS))
}

fn share_value((balance, need): Balance) -> Result<f64, &'static str> {
    match need {
        None => Err(NEED_UNKNOWN),
        Some(need) if need <= 0 => Err(NO_NEED),
        Some(need) => Ok(balance as f64 / need as f64 * 100.0),
    }
}

pub(super) fn value_at(lines: &Lines, replay: &Replay, shown: Shown, hour: i32) -> Result<f64, &'static str> {
    match shown {
        Shown::Stock => match piece_at(&lines.stock, hour) {
            Some(Piece { value: Some(raw), .. }) => Ok(stock_value(*raw)),
            Some(_) => Err(STOCK_UNKNOWN),
            None => Err(STOCK_UNREAD),
        },
        Shown::Share => match piece_at(&lines.logistics, hour) {
            Some(Piece { value: Some(balance), .. }) => share_value(*balance),
            Some(_) => Err(NEED_UNKNOWN),
            None if within(&replay.ai, hour) => Err(AI_BALANCE),
            None => Err(BALANCE_UNREAD),
        },
    }
}

fn after<T>(pieces: &[Piece<T>], hour: i32) -> Option<i32> {
    let at = pieces.partition_point(|piece| piece.last < hour);
    match pieces.get(at) {
        Some(piece) if piece.first <= hour => Some(piece.last.saturating_add(1)),
        Some(piece) => Some(piece.first),
        None => None,
    }
}

fn membership_after(membership: Option<&[HourSpan]>, hour: i32) -> Option<i32> {
    let spans = membership?;
    let at = spans.partition_point(|span| span.last_hour < hour);
    spans.get(at).map(|span| if span.first_hour <= hour { span.last_hour.saturating_add(1) } else { span.first_hour })
}

pub(super) fn next_change<'a>(lines: impl IntoIterator<Item = &'a Lines>, shown: Shown, membership: Option<&[HourSpan]>, hour: i32, last: i32) -> Option<i32> {
    let ends = lines.into_iter().filter_map(|lines| match shown {
        Shown::Stock => after(&lines.stock, hour),
        Shown::Share => after(&lines.logistics, hour),
    });
    ends.chain(membership_after(membership, hour)).filter(|next| *next > hour && *next <= last).min()
}

pub(super) fn main_types(values: &[(&str, Option<Domain>, Result<f64, &'static str>)], n: usize) -> Result<Vec<String>, String> {
    if let Some((_, _, Err(reason))) = values.iter().find(|(_, _, value)| value.is_err()) {
        return Err(format!("The main types are not ranked: the stock of every type is needed. {reason}"));
    }
    let mut ranked: Vec<(&str, Option<Domain>, f64)> = values.iter().filter_map(|(key, domain, value)| value.ok().filter(|value| *value > 0.0).map(|value| (*key, *domain, value))).collect();
    ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    let air = n * 2 / 5;
    let pick = |domain: Domain, count: usize| ranked.iter().filter(move |entry| entry.1 == Some(domain)).take(count).map(|entry| entry.0.to_owned());
    let main: Vec<String> = pick(Domain::Land, n - air).chain(pick(Domain::Air, air)).collect();
    Ok(if main.is_empty() { ranked.iter().take(n).map(|entry| entry.0.to_owned()).collect() } else { main })
}

pub(super) fn bucketed(lines: &[Drawn], first: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    let start_bin = i64::from(first);
    for line in lines {
        let (start, end) = (i64::from(line.first).max(start_bin), i64::from(line.last).min(i64::from(clip)));
        let mut at = start;
        while at <= end {
            let slot = ((at - start_bin) / bin_hours) as usize;
            if slot >= BINS {
                break;
            }
            let bin_end = (start_bin + (slot as i64 + 1) * bin_hours - 1).min(end);
            let (from, to) = (at as i32, bin_end as i32);
            let piece = ChartBucket {
                absolute_total: line.absolute * (bin_end - at + 1) as u128,
                first_hour: from,
                last_hour: to,
                first_value: line.value,
                last_value: line.value,
                last_value_hour: from,
                min_hour: from,
                min_value: line.value,
                max_hour: from,
                max_value: line.value,
                first_break: line.break_before && from == line.first,
                has_break: false,
            };
            buckets[slot] = match buckets[slot].take() {
                None => Some(piece),
                Some(held) => merge(Some(held), Some(piece)),
            };
            at = bin_end + 1;
        }
    }
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        if let Some(bucket) = bucket {
            let (bin_first, bin_last) = (start_bin + slot as i64 * bin_hours, start_bin + (slot as i64 + 1) * bin_hours - 1);
            bucket.has_break |= i64::from(bucket.first_hour) > bin_first || i64::from(bucket.last_hour) < bin_last.min(i64::from(clip));
        }
    }
    buckets
}

fn runs(lines: &[Drawn], first: i32, clip: i32) -> Vec<ChartRun> {
    lines
        .iter()
        .filter(|line| line.last >= first && line.first <= clip)
        .map(|line| ChartRun { first_hour: line.first.max(first), last_hour: line.last.min(clip), value: line.value, break_before: line.break_before && line.first >= first })
        .collect()
}

pub(super) fn missing(lines: &Lines, replay: &Replay, shown: Shown, membership: Option<&[HourSpan]>, first: i32, clip: i32) -> Vec<MissingSpan> {
    let mut out: Vec<MissingSpan> = Vec::new();
    let mut hour = first;
    while hour <= clip {
        let next = match (held(membership, hour), value_at(lines, replay, shown, hour)) {
            (false, _) | (true, Ok(_)) => None,
            (true, Err(reason)) => Some(reason),
        };
        let until = next_change([lines], shown, membership, hour, clip).map_or(clip, |next| next - 1).min(clip);
        let until = match shown {
            Shown::Share => replay.ai.iter().flat_map(|span| [span.first_hour - 1, span.last_hour]).filter(|edge| *edge >= hour && *edge < until).min().unwrap_or(until),
            Shown::Stock => until,
        };
        if let Some(reason) = next {
            match out.last_mut() {
                Some(held) if held.reason == reason && held.last_hour + 1 == hour => held.last_hour = until,
                _ => out.push(MissingSpan { first_hour: hour, last_hour: until, reason: reason.to_owned() }),
            }
        }
        hour = until + 1;
    }
    out
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileTileFamily {
    pub kind: String,
    pub measure: StockpileMeasure,
    pub country: Country,
    pub included: Vec<String>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    #[expect(dead_code, reason = "names the branch this request is routed by")]
    branch_id: String,
    #[expect(dead_code, reason = "read as the stockpile tile family")]
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<String>,
    top: Option<u32>,
    hour: Option<f64>,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<StockpileTileFamily>()
}

struct Chosen<'m> {
    model: std::sync::Arc<Model>,
    shown: Shown,
    membership: Option<&'m [HourSpan]>,
}

impl Chosen<'_> {
    fn types(&self) -> impl Iterator<Item = (usize, &str)> {
        self.model.groups.iter().enumerate().filter(|(_, group)| group.kind == Kind::Type).map(|(index, group)| (index, group.key.as_str()))
    }

    fn index(&self, key: &str) -> Option<usize> {
        self.types().find(|(_, held)| *held == key).map(|(index, _)| index)
    }

    fn lines(&self, index: usize) -> &Lines {
        &self.model.lines[0][index]
    }

    fn replay(&self) -> &Replay {
        &self.model.replays[0]
    }

    fn value(&self, index: usize, hour: i32) -> Result<f64, &'static str> {
        if !held(self.membership, hour) {
            return Err(NOT_HELD);
        }
        value_at(self.lines(index), self.replay(), self.shown, hour)
    }

    fn stock(&self, index: usize, hour: i32) -> Result<f64, &'static str> {
        if !held(self.membership, hour) {
            return Err(NOT_HELD);
        }
        value_at(self.lines(index), self.replay(), Shown::Stock, hour)
    }

    fn domain(&self, key: &str) -> Option<Domain> {
        self.model.types.iter().find(|kind| kind.key == key).map(|kind| kind.domain)
    }

    fn top(&self, n: usize, hour: i32) -> (Vec<String>, TileRanking) {
        let values: Vec<(&str, Option<Domain>, Result<f64, &'static str>)> = self.types().map(|(index, key)| (key, self.domain(key), self.stock(index, hour))).collect();
        match main_types(&values, n) {
            Err(reason) => (Vec::new(), TileRanking { hour, from: hour, until: hour, refused: Some(reason) }),
            Ok(keys) => {
                let until = next_change(self.types().map(|(index, _)| self.lines(index)), Shown::Stock, self.membership, hour, self.model.last).map_or(self.model.last, |next| next - 1);
                (keys, TileRanking { hour, from: hour, until: until.max(hour), refused: None })
            }
        }
    }
}

fn chosen<'m>(source: &Source, query: &Query<'_>, family: &'m StockpileTileFamily, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Chosen<'m>> {
    let model = model(source, query, std::slice::from_ref(&family.country), &family.included, load, cancelled)?;
    Ok(Chosen { model, shown: Shown::of(family.measure)?, membership: family.membership.as_deref() })
}

pub fn tile(source: &Source, query: &Query<'_>, params: &Value, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<SeriesTileReply> {
    let family: StockpileTileFamily = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null)).map_err(|error| invalid(format!("Invalid stockpile tile family: {error}")))?;
    let ask: TileAsk = dashboard_query::named::named(params).map_err(|error| invalid(format!("Invalid stockpile tile request: {error}")))?;
    tile::level_of(ask.bin_hours)?;
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let chosen = chosen(source, query, &family, load, cancelled)?;
    let (keys, ranking) = match ask.top {
        None => (ask.countries, None),
        Some(n) => {
            if !ask.countries.is_empty() {
                return Err(invalid("A top of stockpile tiles names no type: the ranking names them"));
            }
            let n = usize::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one type or more"))?;
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let (keys, ranking) = chosen.top(n, hour);
            (keys, Some(ranking))
        }
    };
    let clip = last.min(chosen.model.last);
    let mut drawn_lines: Vec<Vec<Drawn>> = Vec::with_capacity(keys.len());
    let mut tile_lines = Vec::with_capacity(keys.len());
    for key in &keys {
        check_cancel(cancelled)?;
        let mut pieces = Vec::new();
        let missing = match chosen.index(key) {
            None => vec![MissingSpan { first_hour: first, last_hour: clip, reason: NO_TYPE.to_owned() }].into_iter().filter(|span| span.first_hour <= span.last_hour).collect(),
            Some(index) => {
                let lines = chosen.lines(index);
                match chosen.shown {
                    Shown::Stock => drawn(&lines.stock, |raw| Some((stock_value(raw), raw.unsigned_abs())), (first, clip), chosen.membership, &mut pieces),
                    Shown::Share => drawn(&lines.logistics, |balance| share_value(balance).ok().map(|share| (share, 0)), (first, clip), chosen.membership, &mut pieces),
                }
                missing(lines, chosen.replay(), chosen.shown, chosen.membership, first, clip)
            }
        };
        drawn_lines.push(pieces);
        tile_lines.push(TileLine { country: Country::Tag(key.clone()), part: None, missing, partial: Vec::new(), parts: Vec::new(), marks: None });
    }
    let scale = 10_f64.powi(i32::from(chosen.shown.decimals()));
    let (bins, raw_values, hour_bytes) = match ask.bin_hours {
        1 => {
            let runs: Vec<Vec<ChartRun>> = drawn_lines.iter().map(|lines| runs(lines, first, clip)).collect();
            let (bytes, raw) = tile::encode_runs(&runs, first, scale);
            (bytes, raw, 0)
        }
        bin_hours => {
            let buckets: Vec<Option<ChartBucket>> = drawn_lines.iter().flat_map(|lines| bucketed(lines, first, clip, bin_hours)).collect();
            tile::encode(&buckets, first, bin_hours, scale)
        }
    };
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    Ok(SeriesTileReply {
        branch_id: query.timeline.id.to_string(),
        measure: chosen.shown.name().to_owned(),
        decimals: chosen.shown.decimals(),
        bin_hours: ask.bin_hours,
        index: ask.index,
        first_hour: first,
        immutable,
        filled,
        raw_values,
        hour_bytes,
        lines: tile_lines,
        bins: tile::base64(&bins),
        ranking,
    })
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileLinesParams {
    pub branch_id: String,
    pub family: StockpileTileFamily,
    pub hour: i32,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileLineAt {
    pub key: String,
    #[doc = " The localised type name; null when the content does not name it."]
    pub name: Option<String>,
    pub domain: Option<Domain>,
    #[doc = " The type's place in the recording's catalogue, for its colour."]
    pub ordinal: u32,
    pub value: Option<f64>,
    #[doc = " Why there is no value at the hour: never zero, never a guess."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileTop {
    pub keys: Vec<String>,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileLinesReply {
    pub branch_id: String,
    pub hour: i32,
    pub measure: String,
    pub decimals: u8,
    #[doc = " Every equipment type of the country, in the catalogue's order."]
    pub lines: Vec<StockpileLineAt>,
    #[doc = " Asked for a top: the main types at the hour."]
    pub top: Option<StockpileTop>,
    #[doc = " The first later hour at which any line or the top changes; null when none does."]
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StockpileLinesReply);

fn lines_at(source: &Source, query: &Query<'_>, params: &StockpileLinesParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<StockpileLinesReply> {
    let chosen = chosen(source, query, &params.family, load, cancelled)?;
    let hour = params.hour.min(chosen.model.last);
    let ordinal = |key: &str| chosen.model.types.iter().position(|kind| kind.key == key).unwrap_or(chosen.model.types.len()) as u32;
    let lines = chosen
        .types()
        .map(|(index, key)| {
            let value = chosen.value(index, hour);
            StockpileLineAt {
                key: key.to_owned(),
                name: chosen.model.types.iter().find(|kind| kind.key == key).and_then(|kind| kind.name.clone()),
                domain: chosen.domain(key),
                ordinal: ordinal(key),
                value: value.ok(),
                missing: value.err().map(str::to_owned),
            }
        })
        .collect();
    let top = params.top.map(|n| usize::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one type or more"))).transpose()?;
    let top = top.map(|n| {
        let (keys, ranking) = chosen.top(n, hour);
        StockpileTop { keys, refused: ranking.refused }
    });
    let all = || chosen.types().map(|(index, _)| chosen.lines(index));
    let next = next_change(all(), chosen.shown, chosen.membership, hour, chosen.model.last);
    let next = match top {
        Some(_) => next.into_iter().chain(next_change(all(), Shown::Stock, chosen.membership, hour, chosen.model.last)).min(),
        None => next,
    };
    Ok(StockpileLinesReply {
        branch_id: params.branch_id.clone(),
        hour,
        measure: chosen.shown.name().to_owned(),
        decimals: chosen.shown.decimals(),
        lines,
        top,
        next_change: next,
        warnings: chosen.model.warnings.clone(),
    })
}

pub struct EquipmentStockpileLines;

impl Product for EquipmentStockpileLines {
    const KEY: &'static str = "equipment-stockpile-lines";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StockpileLinesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId, hour: Read, top, "family.kind", "family.measure", "family.country", "family.included[]",
        "family.membership[].firstHour": Start, "family.membership[].lastHour": End
    ];
    type Reply = StockpileLinesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StockpileMeasure::decl(), StockpileTileFamily::decl(), StockpileLinesParams::decl(), StockpileLineAt::decl(), StockpileTop::decl(), StockpileLinesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params.family.kind != SERIES || params.family.measure == StockpileMeasure::Need {
            return Err("Invalid stockpile lines family");
        }
        dashboard_base::products::validate_country(&params.family.country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        lines_at(source, &query, params, load, cancelled)
    }
}
