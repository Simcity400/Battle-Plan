
use crate::{
    Query,
    chart::{ChartBucket, ChartRun, ChartSummary, MarkCluster, MarkSummary},
    identity::{Country, Holder},
    log::{self, Pattern},
    read,
};
use dashboard_declare::{Measure, Series};
use replay_recording::columnar_frames::Recording;
use replay_columnar::{envelope::UNIT_ORDINALS, manifest::Status, summary::{Computed, Source}};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, PoisonError},
};
use ts_rs::TS;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod truncated_tests;

pub mod board;
pub mod ranked;
pub mod totals;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ask {
    pub country: Country,
    pub part: u128,
}

impl Ask {
    pub fn whole(country: impl Into<Country>) -> Self {
        Self { country: country.into(), part: 0 }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesLineAsk {
    pub country: Country,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub part: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub run: u32,
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub slot: u32,
    pub id: u32,
    pub segment_first: i32,
    pub starts: bool,
}

struct ExtentRead {
    chart: replay_lanes::lanes::Chart,
    index: HashMap<u128, usize>,
    first: i32,
    last: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MissingSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value_raw: String,
    pub slot: u32,
    pub country_id: u32,
    pub starts_segment: bool,
    pub segment_first_hour: i32,
    #[doc = " Whether the value held before `firstHour` (after `lastHour`) is known to differ: a state"]
    #[doc = " read reads the block of its hour, so a run reaching that block's edge may go on past it."]
    pub first_exact: bool,
    pub last_exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDrop {
    pub hour: i32,
    pub before_raw: String,
    pub after_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesState {
    pub country: Country,
    #[doc = " The part's key, for a sub-keyed series."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub part: Option<String>,
    pub hour: i32,
    pub run: Option<SeriesRun>,
    pub drop: Option<SeriesDrop>,
    #[doc = " Why there is no value: never zero, never a guess."]
    pub missing: Option<String>,
    #[doc = " True when the inputs were recorded and the measure is not applicable (e.g. a share with a zero denominator)."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub not_applicable: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesStateReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub states: Vec<SeriesState>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesKeyReply {
    pub hour: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
pub enum NavigationKey {
    ArrowLeft,
    ArrowRight,
    Home,
    End,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRead {
    pub country: Country,
    pub hour: i32,
    #[doc = " For a sub-keyed series, one part's key; none reads every part the owner holds at the hour."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub part: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPartLife {
    pub part: String,
    #[doc = " The content id the part holds (a token, a text id), when the series declares one."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPartsReply {
    pub branch_id: String,
    #[doc = " Bits of a row key below its owner: a followed row's part is its key's low `partBits` bits."]
    pub part_bits: u32,
    pub owners: Vec<SeriesOwnerParts>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesOwnerParts {
    pub country: Country,
    pub parts: Vec<SeriesPartLife>,
    #[doc = " The span's hours whose parts are not known, with why."]
    pub missing: Vec<MissingSpan>,
}

#[derive(Deserialize)]
struct Family {
    kind: String,
    measure: String,
}

pub const FIELDS: crate::fields::Fields = {
    use crate::fields::HourRole::{End, Read, Start, Undated};
    &[
        ("branchId", Undated), ("family", Undated), ("countries", Undated), ("lines", Undated), ("reads[].hour", Read), ("from", Start), ("to", End),
        ("key", Undated), ("measures", Undated), ("firstHour", Start), ("lastHour", End), ("top", Undated),
        ("hour", Read), ("width", Undated), ("within[].1[].firstHour", Start), ("within[].1[].lastHour", End), ("binHours", Undated), ("index", Undated),
        ("withMarks", Undated), ("least", Undated), ("countries[]", Undated), ("lines[].country", Undated), ("reads[].country", Undated), ("within[].0", Undated),
        ("family.membership[].firstHour", Start), ("family.membership[].lastHour", End), ("family.top", Undated),
    ]
};

pub fn params_walk() -> crate::fields::Walk {
    crate::fields::walk::<Params>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    countries: Vec<Country>,
    #[serde(default)]
    lines: Vec<SeriesLineAsk>,
    #[serde(default)]
    reads: Vec<SeriesRead>,
    from: Option<f64>,
    to: Option<f64>,
    key: Option<NavigationKey>,
    #[serde(default)]
    measures: Vec<String>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
    top: Option<u32>,
    least: Option<bool>,
    hour: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    within: Vec<(Country, Vec<Span>)>,
    #[serde(rename = "binHours")]
    _bin_hours: Option<i64>,
    #[serde(rename = "index")]
    _index: Option<i64>,
    #[serde(rename = "withMarks")]
    _with_marks: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Span {
    pub first_hour: i32,
    pub last_hour: i32,
}

pub const PARTS: &str = "series-parts";

pub type Within = std::collections::BTreeMap<Country, Vec<(i32, i32)>>;

fn within_spans(part: Part, spans: Option<&[(i32, i32)]>) -> Vec<Part> {
    let Some(spans) = spans else { return vec![part] };
    spans
        .iter()
        .filter_map(|&(first, last)| {
            let (first, last) = (part.first.max(first), part.last.min(last));
            (first <= last).then_some(Part { first, last, ..part })
        })
        .collect()
}

pub const PAGE_LIMIT: u32 = 1024;

pub fn next_read(query: &Query<'_>, table: replay_data::rows::TableId, origin: Option<i32>, key: NavigationKey, bounds: (i32, i32)) -> io::Result<Option<i32>> {
    if bounds.0 > bounds.1 {
        return Err(invalid("Invalid navigation bounds"));
    }
    let (first, last) = (i64::from(bounds.0), i64::from(bounds.1));
    let forward = matches!(key, NavigationKey::ArrowRight | NavigationKey::Home);
    let at: i64 = match key {
        NavigationKey::Home => first - 1,
        NavigationKey::End => last + 1,
        _ => origin.map_or(if forward { first - 1 } else { last + 1 }, i64::from),
    };
    let recording = query.replay.recording();
    let mut reach = i64::from(UNIT_ORDINALS);
    loop {
        let (lo, hi) = match forward {
            true => ((at + 1).max(first), (at + reach).min(last)),
            false => ((at - reach).max(first), (at - 1).min(last)),
        };
        if lo <= hi {
            let mut found: Option<i32> = None;
            for stretch in query.timeline.within(lo as i32, hi as i32) {
                let reads = crate::cost::storage(|| replay_lanes::lanes::available_reads(recording, stretch.run, table, stretch.first, stretch.last))?;
                let hour = match forward {
                    true => reads.first().map(|read| read.0),
                    false => reads.last().map(|read| read.1),
                };
                found = match (found, hour) {
                    (Some(held), Some(hour)) => Some(if forward { held.min(hour) } else { held.max(hour) }),
                    (held, hour) => held.or(hour),
                };
            }
            if found.is_some() {
                return Ok(found);
            }
        }
        if (forward && at + reach >= last) || (!forward && at - reach <= first) {
            return Ok(None);
        }
        reach = reach.saturating_mul(8);
    }
}

pub fn hour_of(value: Option<f64>, name: &str) -> io::Result<i32> {
    let value = value.ok_or_else(|| invalid(format!("The request names no {name}")))?;
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(invalid(format!("Invalid {name}")));
    }
    Ok(value.floor() as i32)
}

pub fn answer(query: &Query<'_>, product: &str, params: &serde_json::Value) -> io::Result<Option<serde_json::Value>> {
    let Some(kind) = params.get("family").and_then(|family| family.get("kind")).and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    let Some(series) = dashboard_declare::series(kind) else {
        return Ok(None);
    };
    let value = params;
    let params: Params = crate::named::named(params).map_err(|error| invalid(format!("Invalid {kind} request: {error}")))?;
    if params.family.kind != series.kind {
        return Err(invalid("The family is not this series"));
    }
    let measure = series
        .measure(&params.family.measure)
        .ok_or_else(|| invalid(format!("{} has no measure {}", series.kind, params.family.measure)))?;
    let line = Line::new(query, series, measure)?;
    let ranking = ranked::top(&line, params.top)?;
    let value = match product {
        "series-state" => serde_json::to_value(SeriesStateReply {
            branch_id: params.branch_id.clone(),
            measure: measure.name.to_owned(),
            decimals: measure.decimals,
            states: match ranking {
                Some(n) if params.reads.is_empty() => ranked::states(&line, params.hour, n)?,
                Some(n) => ranked::among(&line, &params.reads, n, params.least == Some(true))?,
                None => line.states(&params.reads, params.top.map(|top| top as usize))?,
            },
        })?,
        "series-summary" => {
            let n = ranking.ok_or_else(|| invalid(format!("A chart of chosen {} lines is served as tiles", series.kind)))?;
            serde_json::to_value(ranked::summary(&line, &params.branch_id, params.from, params.to, params.width, n)?)?
        }
        "series-next-change" => {
            let key = params.key.ok_or_else(|| invalid("The request names no key"))?;
            let bounds = (hour_of(params.first_hour, "firstHour")?, hour_of(params.last_hour, "lastHour")?);
            let origin = params.from.map(|from| hour_of(Some(from), "from")).transpose()?;
            let mut measures = vec![measure];
            for name in &params.measures {
                measures.push(series.measure(name).ok_or_else(|| invalid(format!("{} has no measure {name}", series.kind)))?);
            }
            let asks = match ranking {
                Some(n) => ranked::stepped(&line, origin, key, bounds, n)?.into_iter().map(|tag| Ask::whole(&tag)).collect(),
                None => line.asks(&params.countries, &params.lines)?,
            };
            let within: Within = params.within.iter().map(|(country, spans)| (country.clone(), spans.iter().map(|span| (span.first_hour, span.last_hour)).collect())).collect();
            let hour = match series.kind {
                "autonomy" => line.change_lines(&asks, &measures, origin, key, bounds, &within)?,
                _ => line.navigate_lines(&asks, &measures, origin, key, bounds, &within)?,
            };
            serde_json::to_value(SeriesKeyReply { hour: hour.map(f64::from) })?
        }
        PARTS => {
            let (from, to) = (hour_of(params.from, "from")?, hour_of(params.to, "to")?);
            let part_bits = series.parts.ok_or_else(|| invalid(format!("{} has no parts", series.kind)))?.bits;
            serde_json::to_value(SeriesPartsReply { branch_id: params.branch_id.clone(), part_bits, owners: line.part_lives(&params.countries, from, to)? })?
        }
        crate::tile::PRODUCT => serde_json::to_value(crate::tile::reply(&line, &params.branch_id, value)?)?,
        _ => return Err(invalid(format!("{} does not answer {product}", series.kind))),
    };
    Ok(Some(value))
}

pub struct RankedLine {
    pub tag: String,
    pub summary: ChartSummary,
    pub membership: Vec<(i32, i32)>,
    pub missing: Vec<MissingSpan>,
}

pub fn grid(hours: i64, width: u32) -> (u8, i64) {
    let per_pixel = (hours / i64::from(width.max(1))).max(1);
    if per_pixel < replay_columnar::summary::bucket_hours(0) {
        return (replay_lanes::lanes::WALKED, per_pixel);
    }
    let mut level = 0u8;
    while replay_columnar::summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    let size = replay_columnar::summary::bucket_hours(level);
    (level, (per_pixel / size) * size)
}

pub fn whole_buckets(recording: &Recording, run: u32, (first, last): (i32, i32), level: u8) -> io::Result<(i64, i64)> {
    let (from, to) = (i64::from(first), i64::from(last));
    if level == replay_lanes::lanes::WALKED {
        return Ok((from, to));
    }
    let held = recording.run(run)?;
    let size = replay_columnar::summary::bucket_hours(level);
    let a = match from.rem_euclid(size) != 0 && first > held.first_hour {
        true => (from.div_euclid(size) + 1) * size,
        false => from,
    };
    let b = match (to + 1).rem_euclid(size) != 0 && last < held.last_hour() {
        true => (to + 1).div_euclid(size) * size - 1,
        false => to,
    };
    Ok((a, b))
}

pub fn charts_within(
    recording: &Recording,
    run: u32,
    (table, column): (TableId, u32),
    keys: Option<&[u128]>,
    (first, last): (i32, i32),
    level: u8,
    bin_hours: i64,
) -> io::Result<Vec<(replay_lanes::lanes::Chart, (i32, i32))>> {
    let (a, b) = whole_buckets(recording, run, (first, last), level)?;
    let below = match level {
        0 => replay_lanes::lanes::WALKED,
        _ => level - 1,
    };
    if a > b {
        return charts_within(recording, run, (table, column), keys, (first, last), below, bin_hours);
    }
    let mut out = Vec::new();
    if a > i64::from(first) {
        out.extend(charts_within(recording, run, (table, column), keys, (first, (a - 1) as i32), below, bin_hours)?);
    }
    let chart = crate::cost::storage(|| replay_lanes::lanes::chart_at(recording, run, table, column, keys, a as i32, b as i32, level, bin_hours))?;
    out.push((chart, (a as i32, b as i32)));
    if b < i64::from(last) {
        out.extend(charts_within(recording, run, (table, column), keys, ((b + 1) as i32, last), below, bin_hours)?);
    }
    Ok(out)
}

pub struct Line<'q, 'a> {
    query: &'q Query<'a>,
    series: &'static Series,
    measure: &'static Measure,
    source: Source,
    column: u32,
    drops: Option<log::Reader<'q, 'a>>,
    unread: Mutex<Unread>,
}

type StatusCache = (i32, i32, Arc<Vec<replay_lanes::lanes::StatusRun>>);
type ReasonCache = (i32, i32, Arc<HashMap<u32, String>>);
type SpanCache = (i32, i32, Arc<Vec<(i32, i32)>>);

struct AliasPlan {
    wanted: Vec<(u128, Vec<(u32, Vec<(i32, i32)>)>, Vec<(u128, Vec<(i32, i32)>)>)>,
    rows: Vec<u128>,
    owners: Vec<u128>,
}

#[derive(Default)]
struct Unread {
    statuses: HashMap<u32, Vec<StatusCache>>,
    status_reasons: HashMap<u32, Vec<ReasonCache>>,
    unwitnessed: HashMap<(u32, u128), Vec<SpanCache>>,
    lives: HashMap<u32, Arc<Vec<(u128, i64, i64)>>>,
}

impl<'q, 'a> Line<'q, 'a> {
    pub fn new(query: &'q Query<'a>, series: &'static Series, measure: &'static Measure) -> io::Result<Self> {
        let drops = match series.drops {
            true => {
                let log = dashboard_declare::LOGS
                    .iter()
                    .find(|log| log.table == replay_data::rows::TableId::BalanceDrops)
                    .ok_or_else(|| invalid("The balance drops log is not declared"))?;
                Some(log::Reader::new(query, log)?)
            }
            false => None,
        };
        Ok(Self {
            query,
            series,
            measure,
            source: dashboard_declare::source(series.table, measure)?,
            column: dashboard_declare::summary_id(series.table, measure)?,
            drops,
            unread: Mutex::default(),
        })
    }

    pub fn series(&self) -> &'static Series {
        self.series
    }

    fn withheld_at(&self, run: u32, hour: i32) -> Option<&str> {
        let recording = self.query.replay.recording();
        match recording.table_coverage(self.series.table.index(), run) {
            replay_recording::columnar_frames::TableCoverage::Valid => None,
            replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => Some(reason),
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
                let valid = recording.run(run).ok()?.first_hour + ordinal as i32;
                (hour < valid).then_some(reason)
            }
        }
    }

    fn withheld_span(&self, run: u32, first: i32, last: i32) -> Option<(i32, i32, &str)> {
        if first > last {
            return None;
        }
        let recording = self.query.replay.recording();
        match recording.table_coverage(self.series.table.index(), run) {
            replay_recording::columnar_frames::TableCoverage::Valid => None,
            replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => Some((first, last, reason)),
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
                let valid = recording.run(run).ok()?.first_hour + ordinal as i32;
                (first < valid).then(|| (first, last.min(valid - 1), reason))
            }
        }
    }

    fn valid_part(&self, part: &Part) -> Option<Part> {
        let recording = self.query.replay.recording();
        match recording.table_coverage(self.series.table.index(), part.run) {
            replay_recording::columnar_frames::TableCoverage::Valid => Some(*part),
            replay_recording::columnar_frames::TableCoverage::WithheldAll(_) => None,
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, .. } => {
                let valid = recording.run(part.run).ok()?.first_hour + ordinal as i32;
                let first = part.first.max(valid);
                (first <= part.last).then_some(Part { first, starts: part.starts || first > part.first, ..*part })
            }
        }
    }

    fn overlaps_withheld(&self, parts: &[Part], lo: i32, hi: i32) -> bool {
        lo <= hi
            && parts.iter().any(|part| {
                let (a, b) = (part.first.max(lo), part.last.min(hi));
                a <= b && self.withheld_span(part.run, a, b).is_some()
            })
    }

    fn part_of(&self, text: Option<&str>) -> io::Result<u128> {
        match (self.series.parts, text) {
            (None, None) => Ok(0),
            (None, Some(_)) => Err(invalid(format!("{} has no parts", self.series.kind))),
            (Some(_), None) => Err(invalid(format!("{} is asked by (tag, part)", self.series.kind))),
            (Some(parts), Some(text)) => match text.parse::<u128>() {
                Ok(part) if part <= parts.mask() => Ok(part),
                _ => Err(invalid(format!("Invalid part {text}"))),
            },
        }
    }

    pub fn asks(&self, countries: &[Country], lines: &[SeriesLineAsk]) -> io::Result<Vec<Ask>> {
        if self.series.parts.is_some() && !countries.is_empty() {
            return Err(invalid(format!("{} is asked by (tag, part) lines, never a sum of its parts", self.series.kind)));
        }
        let mut out: Vec<Ask> = countries.iter().map(Ask::whole).collect();
        for line in lines {
            out.push(Ask { country: line.country.clone(), part: self.part_of(line.part.as_deref())? });
        }
        Ok(out)
    }

    pub fn part_text(&self, ask: &Ask) -> Option<String> {
        self.series.parts.map(|_| ask.part.to_string())
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn query(&self) -> &Query<'a> {
        self.query
    }

    pub fn measure_name(&self) -> &'static str {
        self.measure.name
    }

    pub fn decimals(&self) -> u8 {
        self.measure.decimals
    }

    pub fn drops(&self) -> bool {
        self.series.drops
    }

    pub(crate) fn scale(&self) -> f64 {
        10_f64.powi(i32::from(self.measure.decimals))
    }

    fn by_key(&self) -> HashMap<u128, Vec<(i32, i32, &str)>> {
        let mut out: HashMap<u128, Vec<(i32, i32, &str)>> = HashMap::new();
        for (tag, runs) in &self.query.holders.tags {
            for run in runs {
                if let crate::identity::Holder::Found { slot, id } = run.holder {
                    out.entry(self.series.key.of(slot, id)).or_default().push((run.first, run.last, tag.as_str()));
                }
            }
        }
        out
    }

    fn owners_only(&self, what: &str) -> io::Result<()> {
        if self.series.key.unowned() {
            return Err(invalid(format!("{} is rows no country owns: {what} is not offered", self.series.kind)));
        }
        match self.series.parts {
            Some(_) => Err(invalid(format!("{} ranks parts, not countries: {what} is not offered", self.series.kind))),
            None => Ok(()),
        }
    }

    fn fixed(&self, key: u128, from: i32, to: i32) -> Vec<Part> {
        let timeline = &self.query.timeline;
        let mut out: Vec<Part> = Vec::new();
        for stretch in timeline.within(from, to) {
            let mut at = timeline.stretches.partition_point(|held| held.last < stretch.first);
            let mut chain = timeline.stretches[at].first;
            while at > 0 && matches!(timeline.stretches[at].join, crate::timeline::Join::Load { .. }) {
                at -= 1;
                chain = timeline.stretches[at].first;
            }
            out.push(Part { run: stretch.run, key, first: stretch.first, last: stretch.last, slot: 0, id: 0, segment_first: chain, starts: stretch.first == chain });
        }
        out
    }

    pub fn rank_at(&self, hour: i32, n: usize) -> io::Result<Vec<String>> {
        self.rank_at_by(hour, n, false)
    }

    pub fn rank_at_by(&self, hour: i32, n: usize, least: bool) -> io::Result<Vec<String>> {
        Ok(self.rank_at_by_checked(hour, n, least)?.unwrap_or_default())
    }

    pub(crate) fn rank_at_by_checked(&self, hour: i32, n: usize, least: bool) -> io::Result<Result<Vec<String>, String>> {
        self.owners_only("a rank at an hour")?;
        let Some(stretch) = self.query.timeline.at(hour).copied() else {
            return Ok(Err(self.query.timeline.reason(hour)));
        };
        if let Some(reason) = self.withheld_at(stretch.run, hour) {
            return Ok(Err(reason.to_string()));
        }
        let recording = self.query.replay.recording();
        let columns = read::columns_of(&self.source);
        let snapshot = crate::cost::storage(|| replay_lanes::lanes::at(recording, stretch.run, self.series.table, hour, Some(&columns)))?;
        let status_reason = match snapshot.status {
            Status::Unavailable { reason } => snapshot.strings.get(reason).and_then(|text| std::str::from_utf8(text).ok()),
            _ => None,
        };
        if let Some(reason) = board::table_unrecorded(self.series.table, snapshot.status, status_reason) {
            return Ok(Err(reason));
        }
        let view = snapshot.view();
        let holders = self.by_key();
        let tag_of = |key: u128| holders.get(&key)?.iter().find(|run| run.0 <= hour && hour <= run.1).map(|run| run.2);
        let mut values: Vec<(i64, &str)> = view.rows().filter_map(|slot| Some((self.source.value(&view, slot)?, tag_of(view.key(slot)?)?))).collect();
        if self.series.absent_none {
            let held: HashSet<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
            values.extend(holders.iter().filter(|(key, _)| !held.contains(key)).filter_map(|(&key, _)| Some((0, tag_of(key)?))));
        }
        let order = |a: &(i64, &str), b: &(i64, &str)| {
            let (a0, b0) = if least { (b.0, a.0) } else { (a.0, b.0) };
            b0.cmp(&a0).then_with(|| a.1.cmp(b.1))
        };
        if values.len() > n && n > 0 {
            values.select_nth_unstable_by(n - 1, order);
            values.truncate(n);
        }
        values.sort_unstable_by(order);
        Ok(Ok(values.into_iter().take(n).map(|(_, tag)| tag.to_owned()).collect()))
    }

    pub fn ranked(&self, from: i32, to: i32, width: u32, n: usize) -> io::Result<Vec<RankedLine>> {
        self.owners_only("a ranked view")?;
        if self.query.replay.recording().withheld_reason(self.series.table.index()).is_some() {
            return Ok(Vec::new());
        }
        let hours = i64::from(to) - i64::from(from) + 1;
        let six = replay_columnar::summary::bucket_hours(0);
        if hours < six {
            let tags = self.rank_at(to, n)?;
            let charts = self.summaries(&tags, from, to, width)?;
            return Ok(tags
                .into_iter()
                .zip(charts)
                .map(|(tag, (summary, missing))| RankedLine { tag, summary, membership: vec![(from, to)], missing })
                .collect());
        }
        let asked = width;
        let width = if hours / i64::from(width.max(1)) < six { (hours / six) as u32 } else { width };
        let (_, bin_hours) = grid(hours, width);
        let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
        let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
        let recording = self.query.replay.recording();
        let scale = self.scale();
        let holders = self.by_key();
        let tag_of = |key: u128, hour: i32| holders.get(&key)?.iter().find(|run| run.0 <= hour && hour <= run.1).map(|run| run.2);
        let mut lines: BTreeMap<String, (Vec<Option<ChartBucket>>, Vec<(i32, i32)>)> = BTreeMap::new();
        let mut withheld_spans: Vec<(i32, i32)> = Vec::new();
        let mut place = |slot: usize, drawn: Vec<(&str, ChartBucket, Vec<(i32, i32)>)>| {
            for (tag, bucket, hours) in drawn {
                let (buckets, spans) = lines.entry(tag.to_owned()).or_insert_with(|| (vec![None; bins], Vec::new()));
                let joined = hours.first().is_some_and(|&(from, _)| spans.last().is_some_and(|&(_, last)| i64::from(last) + 1 >= i64::from(from)));
                let bucket = ChartBucket { first_break: !joined, ..bucket };
                buckets[slot] = match buckets[slot] {
                    Some(held) => crate::chart::merge(Some(held), Some(bucket)),
                    None => Some(bucket),
                };
                for (from, to) in hours {
                    match spans.last_mut() {
                        Some(last) if i64::from(last.1) + 1 >= i64::from(from) => last.1 = to,
                        _ => spans.push((from, to)),
                    }
                }
            }
        };
        for mut stretch in self.query.timeline.within(from, to) {
            if let Some((w_first, w_last, _)) = self.withheld_span(stretch.run, stretch.first, stretch.last) {
                withheld_spans.push((w_first, w_last));
                if w_last >= stretch.last {
                    continue;
                }
                stretch.first = w_last + 1;
            }
            let span = i64::from(stretch.last) - i64::from(stretch.first) + 1;
            if span < six {
                let mut start = i64::from(stretch.first).div_euclid(bin_hours) * bin_hours;
                while start <= i64::from(stretch.last) {
                    let (lo, hi) = (start.max(i64::from(stretch.first)), (start + bin_hours - 1).min(i64::from(stretch.last)));
                    let slot = ((start.max(first_bin) - first_bin) / bin_hours) as usize;
                    if slot < bins {
                        place(slot, self.exact_top(stretch.run, (lo as i32, hi as i32), replay_lanes::lanes::WALKED, bin_hours, n, &holders)?);
                    }
                    start += bin_hours;
                }
                continue;
            }
            let stretch_width = (span / bin_hours).max(1) as u32;
            let (level, stretch_bins) = grid(span, stretch_width);
            let stretch_first = i64::from(stretch.first).div_euclid(stretch_bins) * stretch_bins;
            let best = crate::cost::storage(|| {
                replay_lanes::lanes::top(recording, stretch.run, self.series.table, self.column, stretch.first, stretch.last, stretch_width, n + 1, replay_lanes::lanes::By::Last)
            })?;
            let (whole_first, whole_last) = self.whole_buckets(stretch.run, (stretch.first, stretch.last), level)?;
            let edges = (whole_first > i64::from(stretch.first), whole_last < i64::from(stretch.last));
            let count = best.len();
            for (bin, members) in best.into_iter().enumerate() {
                let start = stretch_first + bin as i64 * stretch_bins;
                let (lo, hi) = (start.max(i64::from(stretch.first)), (start + stretch_bins - 1).min(i64::from(stretch.last)));
                let slot = ((start.max(first_bin) - first_bin) / bin_hours) as usize;
                if slot >= bins {
                    continue;
                }
                let drawn: Vec<(&str, ChartBucket, Vec<(i32, i32)>)> = match (bin == 0 && edges.0) || (bin + 1 == count && edges.1) {
                    true => self.exact_top(stretch.run, (lo as i32, hi as i32), level, stretch_bins, n, &holders)?,
                    false => members
                        .into_iter()
                        .filter_map(|(key, value)| Some((tag_of(key, hi as i32)?, value)))
                        .take(n)
                        .map(|(tag, value)| {
                            let at_hour = |bucket: i32| (i64::from(bucket) * six).clamp(lo, hi) as i32;
                            let bucket = ChartBucket {
                                absolute_total: 0,
                                first_hour: at_hour(value.first_bucket),
                                last_hour: (i64::from(value.last_bucket) * six + six - 1).clamp(lo, hi) as i32,
                                first_value: value.first as f64 / scale,
                                last_value: value.last as f64 / scale,
                                last_value_hour: at_hour(value.last_bucket),
                                min_hour: at_hour(value.min_bucket),
                                min_value: value.min as f64 / scale,
                                max_hour: at_hour(value.max_bucket),
                                max_value: value.max as f64 / scale,
                                first_break: false,
                                has_break: false,
                            };
                            (tag, bucket, vec![(lo as i32, hi as i32)])
                        })
                        .collect(),
                };
                place(slot, drawn);
            }
        }
        if !withheld_spans.is_empty() {
            for slot in 0..bins {
                let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                if withheld_spans.iter().any(|&(a, b)| a <= hi && lo <= b) {
                    for (buckets, _) in lines.values_mut() {
                        buckets[slot] = None;
                    }
                }
            }
        }
        if hours <= i64::from(asked) {
            let drawn: Vec<(Ask, Vec<(i32, i32)>)> = lines.iter().map(|(tag, (_, spans))| (Ask::whole(tag), spans.clone())).collect();
            let charts = self.summaries_within(&drawn, from, to, asked)?;
            return Ok(drawn
                .into_iter()
                .zip(charts)
                .map(|((ask, membership), (summary, missing))| RankedLine { tag: ask.country.key().into_owned(), summary, membership, missing })
                .collect());
        }
        lines
            .into_iter()
            .map(|(tag, (buckets, membership))| {
                let country = Country::from(&tag);
                let parts = self.parts(&country, from, to);
                let missing = self.missing(&country, from, to, &parts)?;
                Ok(RankedLine { summary: ChartSummary { from: f64::from(from), to: f64::from(to), width, buckets, runs: None }, membership, missing, tag })
            })
            .collect()
    }

    pub fn parts(&self, country: impl Into<Country>, from: i32, to: i32) -> Vec<Part> {
        self.held(&Ask::whole(country), from, to)
    }

    pub fn held(&self, ask: &Ask, from: i32, to: i32) -> Vec<Part> {
        if self.series.key.unowned() {
            return ask.country.tag().and_then(|tag| self.series.key.fixed(tag)).map_or_else(Vec::new, |key| self.fixed(key, from, to));
        }
        let timeline = &self.query.timeline;
        let country = &ask.country;
        let mut out: Vec<Part> = Vec::new();
        for run in self.query.holders.within(country, from, to) {
            let Holder::Found { slot, id } = run.holder else { continue };
            let holder_first = self.query.holders.at(country, run.first).map_or(run.first, |held| held.first);
            for stretch in timeline.within(run.first, run.last) {
                let mut at = timeline.stretches.partition_point(|held| held.last < stretch.first);
                let mut chain = timeline.stretches[at].first;
                while at > 0 && matches!(timeline.stretches[at].join, crate::timeline::Join::Load { .. }) {
                    at -= 1;
                    chain = timeline.stretches[at].first;
                }
                let segment_first = chain.max(holder_first);
                out.push(Part {
                    run: stretch.run,
                    key: self.series.row_key(slot, id, ask.part),
                    first: stretch.first,
                    last: stretch.last,
                    slot,
                    id,
                    segment_first,
                    starts: stretch.first == segment_first,
                });
            }
        }
        out
    }

    pub(crate) fn missing_at(&self, country: &Country, hour: i32) -> Option<String> {
        let Some(stretch) = self.query.timeline.at(hour) else {
            return Some(self.query.timeline.reason(hour));
        };
        if self.series.key.unowned() {
            return country.tag().and_then(|tag| self.series.key.fixed(tag)).is_none().then(|| self.query.holders.say(country, hour, |name| format!("{name} names no row of {}", self.series.kind)));
        }
        let Some(name) = self.query.holders.label(country, hour) else { return Some(self.query.holders.unnamed(country).to_owned()) };
        if let Some((_, _, reason)) = self.query.holders.unknown.iter().find(|(first, last, _)| *first <= hour && hour <= *last) {
            return Some(format!("Who holds {name} is not known: {reason}"));
        }
        match self.query.holders.at(country, hour) {
            None => Some(self.absent(country, hour)),
            Some(run) if run.holder == Holder::Unresolved => Some(format!("No link resolves the country holding {name}")),
            Some(_) => self.withheld_at(stretch.run, hour).map(str::to_owned),
        }
    }

    pub(crate) fn absent(&self, country: &Country, hour: i32) -> String {
        match country {
            Country::Tag(tag) => format!("{tag} is held by no country"),
            Country::Slot(_) => self.query.holders.say(country, hour, |name| format!("{name}'s {} was not recorded at this hour", self.measure.name)),
        }
    }

    fn owned_at(&self, run: u32, hour: i32, owner: u128, top: Option<usize>) -> io::Result<Result<Vec<u128>, String>> {
        let parts = self.series.parts.ok_or_else(|| invalid(format!("{} has no parts", self.series.kind)))?;
        let (lo, hi) = parts.range(owner);
        let recording = self.query.replay.recording();
        let columns = read::columns_of(&self.source);
        let snapshot = crate::cost::storage(|| replay_lanes::lanes::at_keys(recording, run, self.series.table, hour, Some(&columns), read::Keys::Range(lo, hi)))?;
        if let Some(reason) = status_reason(self.series.table, &snapshot.status) {
            return Ok(Err(reason));
        }
        let Some(n) = top else {
            return Ok(Ok(snapshot.rows.iter().map(|&(key, _)| key).collect()));
        };
        let view = snapshot.view();
        let mut ranked: Vec<(i64, u128)> = snapshot.rows.iter().filter_map(|&(key, slot)| Some((self.source.value(&view, slot)?, key))).collect();
        ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.truncate(n);
        Ok(Ok(ranked.into_iter().map(|(_, key)| key).collect()))
    }

    pub fn states(&self, reads: &[SeriesRead], top: Option<usize>) -> io::Result<Vec<SeriesState>> {
        if top.is_some() && self.series.parts.is_none() {
            return Err(invalid(format!("{} has no parts to rank", self.series.kind)));
        }
        let mut asks: Vec<(Ask, i32)> = Vec::new();
        let mut planned: Vec<Result<usize, SeriesState>> = Vec::new();
        let unread = |read: &SeriesRead, missing: String| SeriesState { country: read.country.clone(), part: None, hour: read.hour, run: None, drop: None, missing: Some(missing), not_applicable: None };
        for read in reads {
            if self.series.parts.is_none() || read.part.is_some() {
                asks.push((Ask { country: read.country.clone(), part: self.part_of(read.part.as_deref())? }, read.hour));
                planned.push(Ok(asks.len() - 1));
                continue;
            }
            if let Some(reason) = self.missing_at(&read.country, read.hour) {
                planned.push(Err(unread(read, reason)));
                continue;
            }
            let (Some(stretch), Some(Holder::Found { slot, id })) = (self.query.timeline.at(read.hour), self.query.holders.at(&read.country, read.hour).map(|run| run.holder)) else {
                planned.push(Err(unread(read, self.absent(&read.country, read.hour))));
                continue;
            };
            let parts = self.series.parts.expect("a sub-keyed series");
            match self.owned_at(stretch.run, read.hour, self.series.key.of(slot, id), top)? {
                Err(reason) => planned.push(Err(unread(read, reason))),
                Ok(keys) if keys.is_empty() => planned.push(Err(unread(read, self.query.holders.say(&read.country, read.hour, |name| format!("{name} holds no row of {} at this hour", self.series.kind))))),
                Ok(keys) => {
                    for key in keys {
                        asks.push((Ask { country: read.country.clone(), part: parts.part(key) }, read.hour));
                        planned.push(Ok(asks.len() - 1));
                    }
                }
            }
        }
        let mut states: Vec<Option<SeriesState>> = self.states_of(&asks)?.into_iter().map(Some).collect();
        Ok(planned
            .into_iter()
            .map(|plan| match plan {
                Ok(at) => states[at].take().expect("each ask is read once"),
                Err(state) => state,
            })
            .collect())
    }

    fn states_of(&self, reads: &[(Ask, i32)]) -> io::Result<Vec<SeriesState>> {
        let recording = self.query.replay.recording();
        let mut blocks: HashMap<(u32, u32), Vec<(usize, Part, (i32, i32))>> = HashMap::new();
        let mut out: Vec<SeriesState> = reads
            .iter()
            .map(|(ask, hour)| SeriesState { country: ask.country.clone(), part: self.part_text(ask), hour: *hour, run: None, drop: None, missing: None, not_applicable: None })
            .collect();
        let mut unresolved: HashMap<u32, Vec<(usize, i32, u128)>> = HashMap::new();
        for (at, (ask, hour)) in reads.iter().enumerate() {
            let hour = *hour;
            if let Some(reason) = self.missing_at(&ask.country, hour) {
                out[at].missing = Some(reason);
                continue;
            }
            let timeline = &self.query.timeline;
            let Some(whole) = self.held(ask, timeline.first, timeline.last).into_iter().find(|part| part.first <= hour && hour <= part.last) else {
                out[at].missing = Some(self.absent(&ask.country, hour));
                continue;
            };
            let Some(whole) = self.valid_part(&whole) else {
                out[at].missing = self.withheld_at(whole.run, hour).map(str::to_owned);
                continue;
            };
            let part = whole;
            let held = recording.run(part.run)?;
            let block = ((i64::from(hour) - i64::from(held.first_hour)) / i64::from(UNIT_ORDINALS)) as u32;
            let block_first = held.first_hour + (block * UNIT_ORDINALS) as i32;
            let block_last = (block_first + UNIT_ORDINALS as i32 - 1).min(held.first_hour + held.ordinals as i32 - 1);
            let part = self
                .held(ask, block_first, block_last)
                .into_iter()
                .filter_map(|held| self.valid_part(&held))
                .find(|held| held.first <= hour && hour <= held.last)
                .unwrap_or(part);
            blocks.entry((part.run, block)).or_default().push((at, part, (whole.first, whole.last)));
        }
        for ((run, block), wanted) in blocks {
            let held = recording.run(run)?;
            let block_first = held.first_hour + (block * UNIT_ORDINALS) as i32;
            let block_last = (block_first + UNIT_ORDINALS as i32 - 1).min(held.first_hour + held.ordinals as i32 - 1);
            let valid_floor = match recording.table_coverage(self.series.table.index(), run) {
                replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, .. } => held.first_hour + ordinal as i32,
                _ => held.first_hour,
            };
            let from = (block_first - 1).max(valid_floor);
            let mut keys: Vec<u128> = wanted.iter().map(|(_, part, _)| part.key).collect();
            keys.sort_unstable();
            keys.dedup();
            let (runs, complete) = self.key_runs_covered(run, &keys, from, block_last)?;
            let is_computed = matches!(self.source, Source::Computed { .. });
            let complete_at = |hour: i32| {
                let at = complete.partition_point(|&(_, last)| last < hour);
                complete.get(at).is_some_and(|&(first, _)| first <= hour)
            };
            for (at, part, (whole_first, whole_last)) in wanted {
                let hour = out[at].hour;
                let lo = from.max(part.first);
                let hi = block_last.min(part.last);
                let Some(runs) = runs.get(&part.key) else {
                    if !is_computed && complete_at(hour) {
                        out[at].missing = Some(self.query.holders.say(&out[at].country, hour, |name| board::measure_unrecorded(name, self.measure.name)));
                    } else {
                        unresolved.entry(run).or_default().push((at, hour, part.key));
                    }
                    continue;
                };
                let Some(found) = runs.iter().position(|run| run.0 <= hour && hour <= run.1) else {
                    if !is_computed && complete_at(hour) {
                        out[at].missing = Some(self.query.holders.say(&out[at].country, hour, |name| board::measure_unrecorded(name, self.measure.name)));
                    } else {
                        unresolved.entry(run).or_default().push((at, hour, part.key));
                    }
                    continue;
                };
                let (first, last, value) = runs[found];
                let value = match value {
                    read::CoveredValue::Value(value) => value,
                    read::CoveredValue::Unvalued if !is_computed => {
                        out[at].missing = Some(self.query.holders.say(&out[at].country, hour, |name| board::measure_unrecorded(name, self.measure.name)));
                        continue;
                    }
                    read::CoveredValue::Uncovered if !is_computed && complete_at(hour) => {
                        out[at].missing = Some(self.query.holders.say(&out[at].country, hour, |name| board::measure_unrecorded(name, self.measure.name)));
                        continue;
                    }
                    read::CoveredValue::Unvalued | read::CoveredValue::Uncovered => {
                        unresolved.entry(run).or_default().push((at, hour, part.key));
                        continue;
                    }
                };
                let (read_first, read_last) = (first, last);
                let (first, last) = (first.max(lo).max(part.first), last.min(hi));
                out[at].run = Some(SeriesRun {
                    first_hour: first,
                    last_hour: last,
                    value_raw: value.to_string(),
                    slot: part.slot,
                    country_id: part.id,
                    starts_segment: part.starts && first == whole_first,
                    segment_first_hour: part.segment_first,
                    first_exact: first == whole_first || read_first > from,
                    last_exact: last == whole_last || read_last < block_last,
                });
                if self.series.drops && hour > whole_first && first == hour && found > 0 {
                    let (_, before_last, before) = runs[found - 1];
                    if let (read::CoveredValue::Value(before), true) = (before, before_last + 1 == hour)
                        && value < before
                    {
                        out[at].drop = Some(SeriesDrop { hour, before_raw: before.to_string(), after_raw: value.to_string() });
                    }
                }
            }
        }
        if !unresolved.is_empty() {
            enum Resolved {
                NotApplicable(&'static str),
                TableMissing(String),
                CompleteUnrecorded,
            }
            let columns = read::columns_of(&self.source);
            let is_computed = matches!(self.source, Source::Computed { .. });
            for (run, items) in unresolved {
                let mut asks: Vec<(i32, u128)> = items.iter().map(|&(_, hour, key)| (hour, key)).collect();
                asks.sort_unstable();
                asks.dedup();
                let mut resolved: HashMap<(i32, u128), Resolved> = HashMap::new();
                crate::cost::storage(|| {
                    replay_lanes::lanes::at_each_with_status(recording, run, self.series.table, Some(&columns), &asks, &mut |hour, key, status, unavailable_reason, row| {
                        let state = match crate::coverage::row_coverage(self.series.table, status, unavailable_reason, row.is_some()) {
                            crate::coverage::Coverage::NotRecorded(reason) => Resolved::TableMissing(reason),
                            crate::coverage::Coverage::Recorded => match row {
                                Some((view, slot)) => match self.source.read(&view, slot) {
                                    Computed::NotApplicable(reason) => Resolved::NotApplicable(reason),
                                    Computed::Unrecorded | Computed::Value(_) => Resolved::CompleteUnrecorded,
                                },
                                None => Resolved::CompleteUnrecorded,
                            },
                        };
                        resolved.insert((hour, key), state);
                        Ok(())
                    })
                })?;
                for (at, hour, key) in items {
                    match resolved.get(&(hour, key)) {
                        Some(Resolved::NotApplicable(reason)) => {
                            out[at].missing = Some((*reason).to_owned());
                            out[at].not_applicable = Some(true);
                        }
                        Some(Resolved::TableMissing(reason)) => {
                            out[at].missing = Some(reason.clone());
                        }
                        Some(Resolved::CompleteUnrecorded) | None => {
                            out[at].missing = Some(self.query.holders.say(&out[at].country, hour, |name| board::measure_unrecorded(name, self.measure.name)));
                            if is_computed {
                                out[at].not_applicable = Some(false);
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn summary(&self, country: impl Into<Country>, from: i32, to: i32, width: u32) -> io::Result<(ChartSummary, Vec<MissingSpan>)> {
        Ok(self.summaries(&[country.into()], from, to, width)?.remove(0))
    }

    pub fn summaries<C: Clone + Into<Country>>(&self, countries: &[C], from: i32, to: i32, width: u32) -> io::Result<Vec<(ChartSummary, Vec<MissingSpan>)>> {
        let drawn: Vec<(Ask, Vec<(i32, i32)>)> = countries.iter().map(|country| (Ask::whole(country.clone()), vec![(from, to)])).collect();
        self.summaries_within(&drawn, from, to, width)
    }

    pub fn summaries_within(&self, drawn: &[(Ask, Vec<(i32, i32)>)], from: i32, to: i32, width: u32) -> io::Result<Vec<(ChartSummary, Vec<MissingSpan>)>> {
        let tags: Vec<&Country> = drawn.iter().map(|(ask, _)| &ask.country).collect();
        let hours = i64::from(to) - i64::from(from) + 1;
        let mut out = Vec::with_capacity(tags.len());
        match hours > i64::from(width) {
            true => {
                let (level, bin_hours) = grid(hours, width);
                for (tag, (buckets, held)) in tags.iter().zip(self.binned(drawn, from, to, level, bin_hours)?) {
                    let summary = ChartSummary { from: f64::from(from), to: f64::from(to), width, buckets, runs: None };
                    out.push((summary, self.missing(*tag, from, to, &held)?));
                }
            }
            false => {
                for (tag, (runs, held)) in tags.iter().zip(self.hourly_runs(drawn, from, to)?) {
                    out.push((self.hourly(runs, from, to, width), self.missing(*tag, from, to, &held)?));
                }
            }
        }
        Ok(out)
    }

    fn alias_plan(&self, run: u32, keys: &[u128], from: i32, to: i32) -> io::Result<Option<AliasPlan>> {
        let Some(parts) = self.series.parts.filter(|_| dashboard_declare::absent_is_zero(self.series.table)) else {
            return Ok(None);
        };
        let recording = self.query.replay.recording();
        let canonical = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
        let aliases = Aliases::new(&read::runs(recording, run, TableId::CountryLinks, Source::Cell(canonical), None, from, to)?);
        let table = u128::from(self.series.table.index() as u32) << 32;
        let mut wanted = Vec::with_capacity(keys.len());
        let (mut rows, mut owners) = (Vec::new(), Vec::new());
        for &key in keys {
            let owner = aliases.of(parts.owner(key) as u32, from, to);
            let target = aliases.of(parts.part(key) as u32, from, to);
            let mut listed = Vec::new();
            for (alias, spans) in &owner {
                owners.push(table | u128::from(*alias));
                for (other, others) in &target {
                    let both = meet(spans, others);
                    if !both.is_empty() {
                        let row = parts.key(u128::from(*alias), u128::from(*other));
                        rows.push(row);
                        listed.push((row, both));
                    }
                }
            }
            wanted.push((key, owner, listed));
        }
        rows.sort_unstable();
        rows.dedup();
        owners.sort_unstable();
        owners.dedup();
        Ok(Some(AliasPlan { wanted, rows, owners }))
    }

    fn key_runs(&self, run: u32, keys: &[u128], from: i32, to: i32) -> io::Result<HashMap<u128, read::Runs>> {
        self.key_runs_with(run, keys, from, to, read::runs)
    }

    fn key_runs_witnessed(&self, run: u32, keys: &[u128], from: i32, to: i32) -> io::Result<HashMap<u128, read::Runs>> {
        self.key_runs_with(run, keys, from, to, read::witnessed_runs)
    }

    fn key_runs_with(
        &self,
        run: u32,
        keys: &[u128],
        from: i32,
        to: i32,
        fetch: fn(&replay_recording::columnar_frames::Recording, u32, TableId, Source, Option<&[u128]>, i32, i32) -> io::Result<HashMap<u128, read::Runs>>,
    ) -> io::Result<HashMap<u128, read::Runs>> {
        let recording = self.query.replay.recording();
        let Some(plan) = self.alias_plan(run, keys, from, to)? else {
            return fetch(recording, run, self.series.table, self.source, Some(keys), from, to);
        };
        let complete: Vec<(i32, i32)> = self
            .statuses(run, from, to)?
            .iter()
            .filter(|held| held.last >= from && held.first <= to && matches!(held.status, Status::Complete))
            .map(|held| (held.first.max(from), held.last.min(to)))
            .collect();
        let table = u128::from(self.series.table.index() as u32) << 32;
        let held = fetch(recording, run, self.series.table, self.source, Some(&plan.rows), from, to)?;
        let sample = dashboard_declare::field(TableId::SliceReads, "sample_hour")?;
        let reads = read::runs(recording, run, TableId::SliceReads, Source::Cell(sample), Some(&plan.owners), from, to)?;
        let mut out = HashMap::with_capacity(plan.wanted.len());
        for (key, owner, listed) in plan.wanted {
            let read = union(
                owner
                    .iter()
                    .flat_map(|(alias, spans)| {
                        let read: Vec<(i32, i32)> = reads.get(&(table | u128::from(*alias))).into_iter().flatten().filter(|run| run.2.is_some()).map(|run| (run.0, run.1)).collect();
                        meet(&read, spans)
                    })
                    .collect(),
            );
            let rows: Vec<(&[(i32, i32, Option<i64>)], &[(i32, i32)])> =
                listed.iter().map(|(row, spans)| (held.get(row).map_or(&[][..], Vec::as_slice), spans.as_slice())).collect();
            out.insert(key, zero_absent(&pair_rows(&rows), &meet(&read, &complete), from, to));
        }
        Ok(out)
    }

    fn key_runs_covered(&self, run: u32, keys: &[u128], from: i32, to: i32) -> io::Result<(HashMap<u128, read::CoveredRuns>, Vec<(i32, i32)>)> {
        let recording = self.query.replay.recording();
        let Some(plan) = self.alias_plan(run, keys, from, to)? else {
            return read::covered_runs(recording, run, self.series.table, self.source, Some(keys), from, to);
        };
        let complete: Vec<(i32, i32)> = self
            .statuses(run, from, to)?
            .iter()
            .filter(|held| held.last >= from && held.first <= to && matches!(held.status, Status::Complete))
            .map(|held| (held.first.max(from), held.last.min(to)))
            .collect();
        let table = u128::from(self.series.table.index() as u32) << 32;
        let (held, _) = read::covered_runs(recording, run, self.series.table, self.source, Some(&plan.rows), from, to)?;
        let sample = dashboard_declare::field(TableId::SliceReads, "sample_hour")?;
        let reads = read::runs(recording, run, TableId::SliceReads, Source::Cell(sample), Some(&plan.owners), from, to)?;
        let mut out = HashMap::with_capacity(plan.wanted.len());
        for (key, owner, listed) in plan.wanted {
            let read = union(
                owner
                    .iter()
                    .flat_map(|(alias, spans)| {
                        let read: Vec<(i32, i32)> = reads.get(&(table | u128::from(*alias))).into_iter().flatten().filter(|run| run.2.is_some()).map(|run| (run.0, run.1)).collect();
                        meet(&read, spans)
                    })
                    .collect(),
            );
            let rows: Vec<(&[(i32, i32, read::CoveredValue)], &[(i32, i32)])> =
                listed.iter().map(|(row, spans)| (held.get(row).map_or(&[][..], Vec::as_slice), spans.as_slice())).collect();
            out.insert(key, zero_absent_covered(&pair_covered_rows(&rows), &meet(&read, &complete), from, to));
        }
        Ok((out, complete))
    }

    pub(crate) fn hourly_runs(&self, drawn: &[(Ask, Vec<(i32, i32)>)], from: i32, to: i32) -> io::Result<Vec<(Vec<ChartRun>, Vec<Part>)>> {
        self.collect_hourly_runs(drawn, from, to, Self::key_runs)
    }

    pub(crate) fn hourly_runs_for_tile(&self, drawn: &[(Ask, Vec<(i32, i32)>)], from: i32, to: i32) -> io::Result<Vec<(Vec<ChartRun>, Vec<Part>)>> {
        self.collect_hourly_runs(drawn, from, to, Self::key_runs_witnessed)
    }

    fn collect_hourly_runs(
        &self,
        drawn: &[(Ask, Vec<(i32, i32)>)],
        from: i32,
        to: i32,
        fetch: fn(&Self, u32, &[u128], i32, i32) -> io::Result<HashMap<u128, read::Runs>>,
    ) -> io::Result<Vec<(Vec<ChartRun>, Vec<Part>)>> {
        let (held, parts) = self.drawn_parts(drawn, from, to);
        let parts: Vec<Vec<Part>> = parts.into_iter().map(|group| group.into_iter().filter_map(|part| self.valid_part(&part)).collect()).collect();
        let mut by_run: HashMap<u32, (Vec<u128>, i32, i32)> = HashMap::new();
        for part in parts.iter().flatten() {
            let held = by_run.entry(part.run).or_insert((Vec::new(), part.first, part.last));
            held.0.push(part.key);
            held.1 = held.1.min(part.first);
            held.2 = held.2.max(part.last);
        }
        let mut read: HashMap<u32, HashMap<u128, read::Runs>> = HashMap::new();
        for (run, (mut keys, first, last)) in by_run {
            keys.sort_unstable();
            keys.dedup();
            read.insert(run, fetch(self, run, &keys, first, last)?);
        }
        let scale = self.scale();
        let mut out = Vec::with_capacity(drawn.len());
        for (parts, held) in parts.iter().zip(held) {
            let mut runs: Vec<ChartRun> = Vec::new();
            for part in parts {
                let mut broken = part.starts;
                let held = read.get(&part.run).and_then(|keys| keys.get(&part.key)).into_iter().flatten();
                for &(first, last, value) in held {
                    let (first, last) = (first.max(part.first), last.min(part.last));
                    if first > last {
                        continue;
                    }
                    match value {
                        Some(value) => {
                            runs.push(ChartRun { first_hour: first, last_hour: last, value: value as f64 / scale, break_before: broken });
                            broken = false;
                        }
                        None => broken = true,
                    }
                }
            }
            out.push((runs, held));
        }
        Ok(out)
    }

    fn drawn_parts(&self, drawn: &[(Ask, Vec<(i32, i32)>)], from: i32, to: i32) -> (Vec<Vec<Part>>, Vec<Vec<Part>>) {
        let held: Vec<Vec<Part>> = drawn.iter().map(|(ask, _)| self.held(ask, from, to)).collect();
        let cut = held
            .iter()
            .zip(drawn)
            .map(|(parts, (_, spans))| {
                let mut cut = Vec::new();
                for part in parts {
                    for &(first, last) in spans {
                        let (first, last) = (first.max(part.first), last.min(part.last));
                        if first <= last {
                            cut.push(Part { first, last, starts: part.starts || first > part.first, ..*part });
                        }
                    }
                }
                cut
            })
            .collect();
        (held, cut)
    }

    fn read_binned_extents(
        &self,
        drawn: &[(Ask, Vec<(i32, i32)>)],
        from: i32,
        to: i32,
        level: u8,
        bin_hours: i64,
    ) -> io::Result<(Vec<Vec<Part>>, Vec<Vec<Part>>, HashMap<u32, Vec<(i32, i32)>>, HashMap<(u32, i32, i32), Vec<ExtentRead>>)> {
        let (held, raw_parts) = self.drawn_parts(drawn, from, to);
        let parts: Vec<Vec<Part>> = raw_parts.into_iter().map(|group| group.into_iter().filter_map(|part| self.valid_part(&part)).collect()).collect();
        let mut covered: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
        for part in parts.iter().flatten() {
            covered.entry(part.run).or_default().push((part.first, part.last));
        }
        for spans in covered.values_mut() {
            spans.sort_unstable();
            spans.dedup_by(|next, kept| {
                let joins = next.0 <= kept.1.saturating_add(1);
                if joins {
                    kept.1 = kept.1.max(next.1);
                }
                joins
            });
        }
        let held_by = self.held_by_owner();
        let mut extents: HashMap<(u32, i32, i32), Vec<u128>> = HashMap::new();
        for part in parts.iter().flatten() {
            let spans = &covered[&part.run];
            let span = spans[spans.partition_point(|span| span.1 < part.first)];
            let extent = match self.shared_edge(part, span, bin_hours, &held_by) {
                true => (part.run, part.first, part.last),
                false => (part.run, span.0, span.1),
            };
            extents.entry(extent).or_default().push(part.key);
        }
        let mut read: HashMap<(u32, i32, i32), Vec<ExtentRead>> = HashMap::new();
        for (extent, mut keys) in extents {
            keys.sort_unstable();
            keys.dedup();
            let (run, first, last) = extent;
            read.insert(extent, self.charts_within(run, &keys, (first, last), level, bin_hours)?);
        }
        Ok((held, parts, covered, read))
    }

    pub(crate) fn binned(&self, drawn: &[(Ask, Vec<(i32, i32)>)], from: i32, to: i32, level: u8, bin_hours: i64) -> io::Result<Vec<(Vec<Option<ChartBucket>>, Vec<Part>)>> {
        if self.query.replay.recording().withheld_reason(self.series.table.index()).is_some() {
            let (held, _) = self.drawn_parts(drawn, from, to);
            let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
            let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
            return Ok(held.into_iter().map(|parts| (vec![None; bins], parts)).collect());
        }
        if dashboard_declare::read_by_key(self.series.table) {
            return self.binned_from_runs_table(drawn, from, to, bin_hours);
        }
        let (held, parts, covered, read) = self.read_binned_extents(drawn, from, to, level, bin_hours)?;
        let span_of = |part: &Part| {
            let spans = &covered[&part.run];
            spans[spans.partition_point(|span| span.1 < part.first)]
        };
        let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
        let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
        let mut out = Vec::with_capacity(drawn.len());
        for (parts, held) in parts.iter().zip(held) {
            let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
            for part in parts {
                let own = read.get(&(part.run, part.first, part.last)).filter(|reads| reads.iter().any(|read| read.index.contains_key(&part.key)));
                let span = span_of(part);
                let Some(reads) = own.or_else(|| read.get(&(part.run, span.0, span.1))) else { continue };
                let mut previous = false;
                for (slot, bucket) in self.fold_reads(reads, part.key, (part.first, part.last), level, first_bin, bin_hours, bins).into_iter().enumerate() {
                    let Some(bucket) = bucket else {
                        previous = false;
                        continue;
                    };
                    let start = first_bin + slot as i64 * bin_hours;
                    let now = ChartBucket { first_break: !previous || (part.starts && i64::from(part.first) >= start), ..bucket };
                    buckets[slot] = match buckets[slot].take() {
                        None => Some(now),
                        Some(held) => crate::chart::merge(Some(held), Some(now)),
                    };
                    previous = true;
                }
            }
            for (slot, bucket) in buckets.iter_mut().enumerate() {
                if bucket.is_none() {
                    continue;
                }
                let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                if self.overlaps_withheld(&held, lo, hi) {
                    *bucket = None;
                }
            }
            out.push((buckets, held));
        }
        Ok(out)
    }

    pub(crate) fn binned_for_tile(
        &self,
        drawn: &[(Ask, Vec<(i32, i32)>)],
        from: i32,
        to: i32,
        level: u8,
        bin_hours: i64,
    ) -> io::Result<Vec<(Vec<Option<ChartBucket>>, Vec<Part>)>> {
        if self.query.replay.recording().withheld_reason(self.series.table.index()).is_some() {
            let (held, _) = self.drawn_parts(drawn, from, to);
            let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
            let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
            return Ok(held.into_iter().map(|parts| (vec![None; bins], parts)).collect());
        }
        if dashboard_declare::read_by_key(self.series.table) {
            return self.binned_from_runs_for_tile(drawn, from, to, bin_hours);
        }
        let (held, parts, covered, read) = self.read_binned_extents(drawn, from, to, level, bin_hours)?;
        let span_of = |part: &Part| {
            let spans = &covered[&part.run];
            spans[spans.partition_point(|span| span.1 < part.first)]
        };
        let mut run_incomplete: HashSet<u32> = HashSet::new();
        if !covered.is_empty() {
            let mut held_spans: HashMap<u32, (i32, i32)> = HashMap::new();
            for part in held.iter().flatten().filter_map(|part| self.valid_part(part)) {
                if !covered.contains_key(&part.run) {
                    continue;
                }
                held_spans
                    .entry(part.run)
                    .and_modify(|span| {
                        span.0 = span.0.min(part.first);
                        span.1 = span.1.max(part.last);
                    })
                    .or_insert((part.first, part.last));
            }
            for (run, (a, b)) in held_spans {
                let statuses = self.statuses(run, a, b)?;
                if statuses.iter().any(|s| !matches!(s.status, Status::Complete)) {
                    run_incomplete.insert(run);
                }
            }
        }
        if !run_incomplete.is_empty() {
            let batch: Vec<&Part> = held.iter().flatten().filter(|part| run_incomplete.contains(&part.run)).collect();
            self.prefetch_unwitnessed(&batch, from, to)?;
        }
        let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
        let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
        let mut out = Vec::with_capacity(drawn.len());
        for (parts, held) in parts.iter().zip(held) {
            let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
            for part in parts {
                let own = read.get(&(part.run, part.first, part.last)).filter(|reads| reads.iter().any(|read| read.index.contains_key(&part.key)));
                let span = span_of(part);
                let Some(reads) = own.or_else(|| read.get(&(part.run, span.0, span.1))) else { continue };
                let part_gaps: Vec<(i32, i32)> = if run_incomplete.contains(&part.run) {
                    let mut missing = Vec::new();
                    self.part_coverage_row(part, part.first, part.last, &mut missing)?;
                    missing.into_iter().map(|gap| (gap.first_hour, gap.last_hour)).collect()
                } else {
                    Vec::new()
                };
                let mut previous = false;
                for (slot, bucket) in self.fold_reads(reads, part.key, (part.first, part.last), level, first_bin, bin_hours, bins).into_iter().enumerate() {
                    let Some(mut bucket) = bucket else {
                        previous = false;
                        continue;
                    };
                    let start = first_bin + slot as i64 * bin_hours;
                    let mut first_break = !previous || (part.starts && i64::from(part.first) >= start);
                    if !part_gaps.is_empty() {
                        let lo = start.max(i64::from(part.first)) as i32;
                        let hi = (start + bin_hours - 1).min(i64::from(part.last)) as i32;
                        if part_gaps.iter().any(|&(ga, gb)| ga <= hi && lo <= gb) {
                            bucket.has_break = true;
                        }
                        if lo > part.first && part_gaps.iter().any(|&(ga, gb)| ga <= lo - 1 && lo - 1 <= gb) {
                            first_break = true;
                        }
                    }
                    let now = ChartBucket { first_break, ..bucket };
                    buckets[slot] = match buckets[slot].take() {
                        None => Some(now),
                        Some(held) => crate::chart::merge(Some(held), Some(now)),
                    };
                    previous = true;
                }
            }
            for (slot, bucket) in buckets.iter_mut().enumerate() {
                if bucket.is_none() {
                    continue;
                }
                let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                if self.overlaps_withheld(&held, lo, hi) {
                    *bucket = None;
                }
            }
            out.push((buckets, held));
        }
        Ok(out)
    }

    fn whole_buckets(&self, run: u32, span: (i32, i32), level: u8) -> io::Result<(i64, i64)> {
        whole_buckets(self.query.replay.recording(), run, span, level)
    }

    fn charts_within(&self, run: u32, keys: &[u128], span: (i32, i32), level: u8, bin_hours: i64) -> io::Result<Vec<ExtentRead>> {
        Ok(charts_within(self.query.replay.recording(), run, (self.series.table, self.column), Some(keys), span, level, bin_hours)?
            .into_iter()
            .map(|(chart, (first, last))| {
                let index = chart.keys.iter().enumerate().map(|(at, &key)| (key, at)).collect();
                ExtentRead { chart, index, first, last }
            })
            .collect())
    }

    fn fold_reads(&self, reads: &[ExtentRead], key: u128, (first, last): (i32, i32), level: u8, first_bin: i64, bin_hours: i64, bins: usize) -> Vec<Option<ChartBucket>> {
        use replay_columnar::summary::bucket_hours;
        let scale = self.scale();
        let six = bucket_hours(0);
        let ratio = match level {
            replay_lanes::lanes::WALKED => 1,
            _ => bucket_hours(level) / six,
        };
        let mut folded: Vec<Option<(replay_lanes::lanes::Bin, u32, u128)>> = vec![None; bins];
        for read in reads {
            let Some(&at) = read.index.get(&key) else { continue };
            let chart = &read.chart;
            let (values, life, totals) = (chart.values(at), chart.life(at), chart.totals(at));
            for (bin, value) in values.iter().enumerate() {
                let start = chart.first_bin_hour + bin as i64 * bin_hours;
                let Some(slot) = usize::try_from((start - first_bin).div_euclid(bin_hours)).ok().filter(|&slot| slot < bins) else { continue };
                let lo = start.max(i64::from(first)).max(i64::from(read.first));
                let hi = (start + bin_hours - 1).min(i64::from(last)).min(i64::from(read.last));
                if lo > hi {
                    continue;
                }
                let (held, lived, abs) = folded[slot].get_or_insert_with(Default::default);
                *lived += life[bin];
                *abs += totals.map_or(0, |totals| totals[bin].abs);
                if value.hours == 0 {
                    continue;
                }
                if held.hours == 0 {
                    *held = *value;
                    continue;
                }
                if value.min < held.min {
                    (held.min, held.min_bucket) = (value.min, value.min_bucket);
                }
                if value.max > held.max {
                    (held.max, held.max_bucket) = (value.max, value.max_bucket);
                }
                (held.last, held.last_bucket) = (value.last, value.last_bucket);
                held.hours += value.hours;
            }
        }
        let coarse = |bucket: i32| i64::from(bucket).div_euclid(ratio) * ratio;
        folded
            .into_iter()
            .enumerate()
            .map(|(slot, folded)| {
                let (value, lived, abs) = folded.filter(|(value, ..)| value.hours > 0)?;
                let start = first_bin + slot as i64 * bin_hours;
                let (lo, hi) = (start.max(i64::from(first)), (start + bin_hours - 1).min(i64::from(last)));
                let at_hour = |bucket: i32| (coarse(bucket) * six).clamp(lo, hi) as i32;
                let last_six = (coarse(value.last_bucket) + ratio - 1) * six;
                Some(ChartBucket {
                    absolute_total: abs,
                    first_hour: at_hour(value.first_bucket),
                    last_hour: (last_six + six - 1).clamp(lo, hi) as i32,
                    first_value: value.first as f64 / scale,
                    last_value: value.last as f64 / scale,
                    last_value_hour: last_six.clamp(lo, hi) as i32,
                    min_hour: at_hour(value.min_bucket),
                    min_value: value.min as f64 / scale,
                    max_hour: at_hour(value.max_bucket),
                    max_value: value.max as f64 / scale,
                    first_break: false,
                    has_break: value.hours < lived,
                })
            })
            .collect()
    }

    fn exact_top<'t>(
        &self,
        run: u32,
        (first, last): (i32, i32),
        level: u8,
        bin_hours: i64,
        n: usize,
        holders: &HashMap<u128, Vec<(i32, i32, &'t str)>>,
    ) -> io::Result<Vec<(&'t str, ChartBucket, Vec<(i32, i32)>)>> {
        let tag_of = |key: u128, hour: i32| holders.get(&key)?.iter().find(|run| run.0 <= hour && hour <= run.1).map(|run| run.2);
        let recording = self.query.replay.recording();
        let columns = read::columns_of(&self.source);
        let snapshot = crate::cost::storage(|| replay_lanes::lanes::at(recording, run, self.series.table, last, Some(&columns)))?;
        if !matches!(snapshot.status, Status::Complete) {
            return Ok(Vec::new());
        }
        let view = snapshot.view();
        let mut ranked: Vec<(i64, u128, &'t str)> = view
            .rows()
            .filter_map(|slot| {
                let key = view.key(slot)?;
                Some((self.source.value(&view, slot)?, key, tag_of(key, last)?))
            })
            .collect();
        let order = |a: &(i64, u128, &str), b: &(i64, u128, &str)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
        if ranked.len() > n && n > 0 {
            ranked.select_nth_unstable_by(n - 1, order);
        }
        ranked.truncate(n);
        ranked.sort_unstable_by(order);
        if ranked.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_hours: BTreeMap<Vec<(i32, i32)>, Vec<u128>> = BTreeMap::new();
        for &(_, key, tag) in &ranked {
            let mut held: Vec<(i32, i32)> =
                holders.get(&key).into_iter().flatten().filter(|run| run.2 == tag && run.0 <= last && first <= run.1).map(|run| (run.0.max(first), run.1.min(last))).collect();
            held.sort_unstable();
            held.dedup_by(|next, kept| {
                let joins = i64::from(kept.1) + 1 >= i64::from(next.0);
                if joins {
                    kept.1 = kept.1.max(next.1);
                }
                joins
            });
            by_hours.entry(held).or_default().push(key);
        }
        let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
        let mut read: HashMap<u128, (Option<ChartBucket>, Vec<(i32, i32)>)> = HashMap::with_capacity(ranked.len());
        for (held, mut keys) in by_hours {
            keys.sort_unstable();
            let mut buckets: Vec<Option<ChartBucket>> = vec![None; keys.len()];
            for (at, &span) in held.iter().enumerate() {
                let reads = self.charts_within(run, &keys, span, level, bin_hours)?;
                for (bucket, &key) in buckets.iter_mut().zip(&keys) {
                    let part = self.fold_reads(&reads, key, span, level, first_bin, bin_hours, 1).pop().flatten();
                    *bucket = if at == 0 { part } else { crate::chart::merge(*bucket, part) };
                }
            }
            for (key, bucket) in keys.into_iter().zip(buckets) {
                read.insert(key, (bucket, held.clone()));
            }
        }
        Ok(ranked
            .into_iter()
            .filter_map(|(_, key, tag)| {
                let (bucket, held) = read.remove(&key)?;
                Some((tag, bucket?, held))
            })
            .collect())
    }

    fn held_by_owner(&self) -> (HashMap<u128, Vec<(i32, i32)>>, Vec<(i32, i32)>) {
        let mut owners: HashMap<u128, Vec<(i32, i32)>> = HashMap::new();
        let mut unknown: Vec<(i32, i32)> = self.query.holders.unknown.iter().map(|(first, last, _)| (*first, *last)).collect();
        for runs in self.query.holders.tags.values() {
            for run in runs {
                match run.holder {
                    Holder::Found { slot, id } => owners.entry(self.series.key.of(slot, id)).or_default().push((run.first, run.last)),
                    Holder::Unresolved => unknown.push((run.first, run.last)),
                }
            }
        }
        (owners, unknown)
    }

    fn shared_edge(&self, part: &Part, span: (i32, i32), bin_hours: i64, (owners, unknown): &(HashMap<u128, Vec<(i32, i32)>>, Vec<(i32, i32)>)) -> bool {
        if self.series.key == dashboard_declare::Key::Singleton {
            return part.first > span.0 || part.last < span.1;
        }
        let (first, last) = (i64::from(part.first), i64::from(part.last));
        let before = (i64::from(span.0).max(first.div_euclid(bin_hours) * bin_hours), first - 1);
        let after = (last + 1, i64::from(span.1).min((last.div_euclid(bin_hours) + 1) * bin_hours - 1));
        let owner = match self.series.parts {
            Some(parts) => parts.owner(part.key),
            None => part.key,
        };
        let meets = |(a, b): (i64, i64)| {
            a <= b
                && owners.get(&owner).into_iter().flatten().chain(unknown).any(|&(from, to)| i64::from(from) <= b && a <= i64::from(to))
        };
        meets(before) || meets(after)
    }

    fn fold_run_buckets(&self, runs: Vec<ChartRun>, first_bin: i64, bin_hours: i64, bins: usize) -> Vec<Option<ChartBucket>> {
        let scale = self.scale();
        let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
        for run in runs {
            let raw = (run.value * scale).round().abs() as u128;
            let mut at = i64::from(run.first_hour);
            while at <= i64::from(run.last_hour) {
                let slot = (at - first_bin).div_euclid(bin_hours);
                let end = (first_bin + (slot + 1) * bin_hours - 1).min(i64::from(run.last_hour));
                let Some(bucket) = usize::try_from(slot).ok().and_then(|slot| buckets.get_mut(slot)) else { break };
                let (first, last) = (at as i32, end as i32);
                let now = ChartBucket {
                    absolute_total: raw * (end - at + 1) as u128,
                    first_hour: first,
                    last_hour: last,
                    first_value: run.value,
                    last_value: run.value,
                    last_value_hour: first,
                    min_hour: first,
                    min_value: run.value,
                    max_hour: first,
                    max_value: run.value,
                    first_break: first == run.first_hour && run.break_before,
                    has_break: false,
                };
                *bucket = match bucket.take() {
                    None => Some(now),
                    Some(held) => crate::chart::merge(Some(held), Some(now)),
                };
                at = end + 1;
            }
        }
        buckets
    }

    fn binned_from_runs_table(
        &self,
        drawn: &[(Ask, Vec<(i32, i32)>)],
        from: i32,
        to: i32,
        bin_hours: i64,
    ) -> io::Result<Vec<(Vec<Option<ChartBucket>>, Vec<Part>)>> {
        let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
        let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
        let mut out = Vec::with_capacity(drawn.len());
        for (runs, held) in self.hourly_runs(drawn, from, to)? {
            let mut buckets = self.fold_run_buckets(runs, first_bin, bin_hours, bins);
            for (slot, bucket) in buckets.iter_mut().enumerate() {
                if bucket.is_none() {
                    continue;
                }
                let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                if self.overlaps_withheld(&held, lo, hi) {
                    *bucket = None;
                }
            }
            out.push((buckets, held));
        }
        Ok(out)
    }

    fn binned_from_runs_for_tile(
        &self,
        drawn: &[(Ask, Vec<(i32, i32)>)],
        from: i32,
        to: i32,
        bin_hours: i64,
    ) -> io::Result<Vec<(Vec<Option<ChartBucket>>, Vec<Part>)>> {
        let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
        let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
        let found = self.hourly_runs_for_tile(drawn, from, to)?;
        let all_parts: Vec<&Part> = found.iter().flat_map(|(_, held)| held.iter()).collect();
        self.prefetch_unwitnessed(&all_parts, from, to)?;
        let mut out = Vec::with_capacity(drawn.len());
        for ((_, spans), (runs, held)) in drawn.iter().zip(found) {
            let mut buckets = self.fold_run_buckets(runs, first_bin, bin_hours, bins);
            let mut raw_gaps: Vec<(i32, i32)> = Vec::new();
            for part in &held {
                let mut missing = Vec::new();
                self.part_coverage_row(part, from, to, &mut missing)?;
                raw_gaps.extend(missing.into_iter().map(|gap| (gap.first_hour, gap.last_hour)));
            }
            let part_gaps = meet(&union(raw_gaps), spans);
            if !part_gaps.is_empty() {
                for (slot, bucket) in buckets.iter_mut().enumerate() {
                    let Some(bucket) = bucket else { continue };
                    let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                    let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                    if part_gaps.iter().any(|&(ga, gb)| ga <= hi && lo <= gb) {
                        bucket.has_break = true;
                    }
                    if lo > from && part_gaps.iter().any(|&(ga, gb)| ga <= lo - 1 && lo - 1 <= gb) {
                        bucket.first_break = true;
                    }
                }
            }
            for (slot, bucket) in buckets.iter_mut().enumerate() {
                if bucket.is_none() {
                    continue;
                }
                let lo = (first_bin + slot as i64 * bin_hours).max(i64::from(from)) as i32;
                let hi = (first_bin + (slot as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
                if self.overlaps_withheld(&held, lo, hi) {
                    *bucket = None;
                }
            }
            out.push((buckets, held));
        }
        Ok(out)
    }

    pub fn missing(&self, country: impl Into<Country>, from: i32, to: i32, parts: &[Part]) -> io::Result<Vec<MissingSpan>> {
        self.collect_missing(country.into(), from, to, parts, Self::part_coverage_table)
    }

    pub(crate) fn missing_for_tile(&self, country: impl Into<Country>, from: i32, to: i32, parts: &[Part]) -> io::Result<Vec<MissingSpan>> {
        self.collect_missing(country.into(), from, to, parts, Self::part_coverage_row)
    }

    fn collect_missing(
        &self,
        country: Country,
        from: i32,
        to: i32,
        parts: &[Part],
        coverage: fn(&Self, &Part, i32, i32, &mut Vec<MissingSpan>) -> io::Result<Vec<(i64, i64)>>,
    ) -> io::Result<Vec<MissingSpan>> {
        let mut out = self.unheld(&country, from, to, parts);
        let rowless = self.query.holders.say(&country, to, |name| format!("{name}'s {} was not recorded here", self.measure.name));
        for part in parts {
            let complete = coverage(self, part, from, to, &mut out)?;
            if complete.is_empty() {
                continue;
            }
            let Some(lives) = self.lives(part.run)? else { continue };
            let life = lives.binary_search_by_key(&part.key, |&(key, ..)| key).ok().map(|at| (lives[at].1, lives[at].2));
            for (a, b) in complete {
                let outside = match life {
                    None => [(a, b), (1, 0)],
                    Some((born, ended)) => [(a, b.min(born - 1)), (a.max(ended + 1), b)],
                };
                for (x, y) in outside.into_iter().filter(|(x, y)| x <= y) {
                    out.push(MissingSpan { first_hour: x as i32, last_hour: y as i32, reason: rowless.clone() });
                }
            }
        }
        Ok(joined(out))
    }

    fn complement_gaps(witnessed: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32)> {
        let mut gaps: Vec<(i32, i32)> = Vec::new();
        let mut cursor = first;
        for &(wa, wb) in witnessed {
            if wa > cursor {
                gaps.push((cursor, wa - 1));
            }
            cursor = wb.saturating_add(1);
        }
        if cursor <= last {
            gaps.push((cursor, last));
        }
        gaps
    }

    fn cache_unwitnessed_batch(&self, run: u32, keys: &[u128], first: i32, last: i32) -> io::Result<()> {
        if keys.is_empty() || first > last {
            return Ok(());
        }
        let mut by_key: HashMap<u128, (Option<i32>, Vec<(i32, i32)>)> = HashMap::new();
        let plan = self.alias_plan(run, keys, first, last)?;
        let walk_keys: &[u128] = match &plan {
            Some(plan) => &plan.rows,
            None => keys,
        };
        if !walk_keys.is_empty() {
            crate::cost::storage(|| {
                replay_lanes::points::witnessed_points(
                    self.query.replay.recording(),
                    run,
                    self.series.table,
                    &[],
                    None,
                    read::Keys::These(walk_keys),
                    first,
                    last,
                    &mut |point| {
                        let (open, spans) = by_key.entry(point.key).or_default();
                        if point.covered {
                            open.get_or_insert(point.hour);
                        } else if let Some(start) = open.take()
                            && start < point.hour
                        {
                            spans.push((start, point.hour - 1));
                        }
                        true
                    },
                )
            })?;
            for (open, spans) in by_key.values_mut() {
                if let Some(start) = open.take() {
                    spans.push((start, last));
                }
            }
        }
        let mut unread = self.unread.lock().unwrap_or_else(PoisonError::into_inner);
        match plan {
            None => {
                for &key in keys {
                    let witnessed = by_key.get(&key).map_or(&[][..], |(_, spans)| spans.as_slice());
                    let gaps = Self::complement_gaps(witnessed, first, last);
                    unread.unwitnessed.entry((run, key)).or_default().push((first, last, Arc::new(gaps)));
                }
            }
            Some(plan) => {
                for (key, _, listed) in plan.wanted {
                    let mut out: Vec<(i32, i32)> = Vec::new();
                    for (row, spans) in &listed {
                        if let Some((_, covered)) = by_key.get(row) {
                            out.extend(meet(covered, spans));
                        }
                    }
                    let witnessed = union(out);
                    let gaps = Self::complement_gaps(&witnessed, first, last);
                    unread.unwitnessed.entry((run, key)).or_default().push((first, last, Arc::new(gaps)));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn prefetch_unwitnessed(&self, parts: &[&Part], from: i32, to: i32) -> io::Result<()> {
        let mut by_run: HashMap<u32, (i32, i32, Vec<(u128, i32, i32)>)> = HashMap::new();
        for part in parts {
            let (mut first, last) = (part.first.max(from), part.last.min(to));
            if first > last {
                continue;
            }
            if let Some((_, w_last, _)) = self.withheld_span(part.run, first, last) {
                first = w_last.saturating_add(1);
                if first > last {
                    continue;
                }
            }
            by_run
                .entry(part.run)
                .and_modify(|entry| {
                    entry.0 = entry.0.min(first);
                    entry.1 = entry.1.max(last);
                    entry.2.push((part.key, first, last));
                })
                .or_insert_with(|| (first, last, vec![(part.key, first, last)]));
        }
        for (run, (min_first, max_last, items)) in by_run {
            let statuses = self.statuses(run, min_first, max_last)?;
            for held in statuses.iter().filter(|held| held.last >= min_first && held.first <= max_last && matches!(held.status, Status::Truncated { .. })) {
                let (a, b) = (held.first.max(min_first), held.last.min(max_last));
                if a > b {
                    continue;
                }
                let mut keys: Vec<u128> = {
                    let unread = self.unread.lock().unwrap_or_else(PoisonError::into_inner);
                    items
                        .iter()
                        .filter(|&&(key, pf, pl)| {
                            pf <= b
                                && a <= pl
                                && !unread
                                    .unwitnessed
                                    .get(&(run, key))
                                    .is_some_and(|cached| cached.iter().any(|&(ca, cb, _)| ca <= a && b <= cb))
                        })
                        .map(|&(key, ..)| key)
                        .collect()
                };
                keys.sort_unstable();
                keys.dedup();
                if !keys.is_empty() {
                    self.cache_unwitnessed_batch(run, &keys, a, b)?;
                }
            }
        }
        Ok(())
    }

    fn unwitnessed_spans(&self, run: u32, key: u128, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
        if let Some(cached) = self
            .unread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwitnessed
            .get(&(run, key))
            .and_then(|held| held.iter().find(|&&(a, b, _)| a <= first && last <= b).map(|(_, _, spans)| spans.clone()))
        {
            return Ok(cached
                .iter()
                .filter_map(|&(a, b)| {
                    let (a, b) = (a.max(first), b.min(last));
                    (a <= b).then_some((a, b))
                })
                .collect());
        }
        self.cache_unwitnessed_batch(run, &[key], first, last)?;
        let cached = self
            .unread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwitnessed
            .get(&(run, key))
            .and_then(|held| held.iter().find(|&&(a, b, _)| a <= first && last <= b).map(|(_, _, spans)| spans.clone()))
            .expect("cached unwitnessed spans");
        Ok(cached
            .iter()
            .filter_map(|&(a, b)| {
                let (a, b) = (a.max(first), b.min(last));
                (a <= b).then_some((a, b))
            })
            .collect())
    }

    fn part_coverage_table(&self, part: &Part, from: i32, to: i32, out: &mut Vec<MissingSpan>) -> io::Result<Vec<(i64, i64)>> {
        let (mut first, last) = (part.first.max(from), part.last.min(to));
        let mut complete = Vec::new();
        if first > last {
            return Ok(complete);
        }
        if let Some((w_first, w_last, reason)) = self.withheld_span(part.run, first, last) {
            out.push(MissingSpan { first_hour: w_first, last_hour: w_last, reason: reason.to_owned() });
            first = w_last.saturating_add(1);
            if first > last {
                return Ok(complete);
            }
        }
        let statuses = self.statuses(part.run, first, last)?;
        for held in statuses.iter().filter(|held| held.last >= first && held.first <= last) {
            let (a, b) = (held.first.max(first), held.last.min(last));
            match status_reason(self.series.table, &held.status) {
                None => complete.push((i64::from(a), i64::from(b))),
                Some(reason) => out.push(MissingSpan { first_hour: a, last_hour: b, reason }),
            }
        }
        Ok(complete)
    }

    fn part_coverage_row(&self, part: &Part, from: i32, to: i32, out: &mut Vec<MissingSpan>) -> io::Result<Vec<(i64, i64)>> {
        let (mut first, last) = (part.first.max(from), part.last.min(to));
        let mut complete = Vec::new();
        if first > last {
            return Ok(complete);
        }
        if let Some((w_first, w_last, reason)) = self.withheld_span(part.run, first, last) {
            out.push(MissingSpan { first_hour: w_first, last_hour: w_last, reason: reason.to_owned() });
            first = w_last.saturating_add(1);
            if first > last {
                return Ok(complete);
            }
        }
        let statuses = self.statuses(part.run, first, last)?;
        let mut reasons: Option<Arc<HashMap<u32, String>>> = None;
        for held in statuses.iter().filter(|held| held.last >= first && held.first <= last) {
            let (a, b) = (held.first.max(first), held.last.min(last));
            let unavailable_reason = match held.status {
                Status::Unavailable { reason } => {
                    if reasons.is_none() {
                        reasons = Some(self.status_reasons(part.run, first, last, &statuses)?);
                    }
                    reasons.as_ref().and_then(|map| map.get(&reason).map(String::as_str))
                }
                _ => None,
            };
            match crate::coverage::row_coverage(self.series.table, held.status, unavailable_reason, false) {
                crate::coverage::Coverage::Recorded => complete.push((i64::from(a), i64::from(b))),
                crate::coverage::Coverage::NotRecorded(reason) => {
                    match crate::coverage::row_coverage(self.series.table, held.status, unavailable_reason, true) {
                        crate::coverage::Coverage::Recorded => {
                            for (ga, gb) in self.unwitnessed_spans(part.run, part.key, a, b)? {
                                out.push(MissingSpan { first_hour: ga, last_hour: gb, reason: reason.clone() });
                            }
                        }
                        crate::coverage::Coverage::NotRecorded(_) => {
                            out.push(MissingSpan { first_hour: a, last_hour: b, reason });
                        }
                    }
                }
            }
        }
        Ok(complete)
    }

    fn statuses(&self, run: u32, from: i32, to: i32) -> io::Result<Arc<Vec<replay_lanes::lanes::StatusRun>>> {
        if let Some(held) = self
            .unread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .statuses
            .get(&run)
            .and_then(|held| held.iter().find(|&&(a, b, _)| a <= from && to <= b).map(|(_, _, entry)| entry.clone()))
        {
            return Ok(held);
        }
        let runs = Arc::new(read::statuses(self.query.replay.recording(), run, self.series.table, from, to)?);
        self.unread.lock().unwrap_or_else(PoisonError::into_inner).statuses.entry(run).or_default().push((from, to, runs.clone()));
        Ok(runs)
    }

    fn status_reasons(&self, run: u32, from: i32, to: i32, runs: &[replay_lanes::lanes::StatusRun]) -> io::Result<Arc<HashMap<u32, String>>> {
        if let Some(held) = self
            .unread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .status_reasons
            .get(&run)
            .and_then(|held| held.iter().find(|&&(a, b, _)| a <= from && to <= b).map(|(_, _, entry)| entry.clone()))
        {
            return Ok(held);
        }
        let reasons = Arc::new(read::status_reasons(self.query.replay.recording(), run, self.series.table, runs)?);
        self.unread.lock().unwrap_or_else(PoisonError::into_inner).status_reasons.entry(run).or_default().push((from, to, reasons.clone()));
        Ok(reasons)
    }

    fn lives(&self, run: u32) -> io::Result<Option<Arc<Vec<(u128, i64, i64)>>>> {
        let table = self.series.table;
        if (self.series.parts.is_some() && dashboard_declare::absent_is_zero(table)) || crate::cost::declared(table).is_err() {
            return Ok(None);
        }
        if let Some(held) = self.unread.lock().unwrap_or_else(PoisonError::into_inner).lives.get(&run) {
            return Ok(Some(held.clone()));
        }
        let read = Arc::new(crate::cost::lifetimes(self.query.replay.recording(), run, table)?);
        self.unread.lock().unwrap_or_else(PoisonError::into_inner).lives.insert(run, read.clone());
        Ok(Some(read))
    }

    pub fn unheld(&self, country: &Country, from: i32, to: i32, parts: &[Part]) -> Vec<MissingSpan> {
        let mut edges: Vec<i64> = Vec::new();
        for gap in &self.query.timeline.unheld {
            edges.extend([i64::from(gap.first), i64::from(gap.last) + 1]);
        }
        for (first, last, _) in &self.query.holders.unknown {
            edges.extend([i64::from(*first), i64::from(*last) + 1]);
        }
        for run in self.query.holders.runs(country) {
            edges.extend([i64::from(run.first), i64::from(run.last) + 1]);
        }
        for stretch in &self.query.timeline.stretches {
            edges.extend([i64::from(stretch.first), i64::from(stretch.last) + 1]);
        }
        edges.sort_unstable();
        edges.dedup();
        let mut out: Vec<MissingSpan> = Vec::new();
        let push = |first: i64, last: i64, out: &mut Vec<MissingSpan>| {
            let mut at = first;
            while at <= last {
                let next = edges[edges.partition_point(|&edge| edge <= at)..].first().copied().unwrap_or(i64::MAX);
                let end = (next - 1).min(last);
                let reason = self
                    .missing_at(country, at as i32)
                    .unwrap_or_else(|| self.query.holders.say(country, at as i32, |name| format!("{name}'s {} was not recorded here", self.measure.name)));
                match out.last_mut() {
                    Some(held) if held.reason == reason && i64::from(held.last_hour) + 1 == at => held.last_hour = end as i32,
                    _ => out.push(MissingSpan { first_hour: at as i32, last_hour: end as i32, reason }),
                }
                at = end + 1;
            }
        };
        let mut hour = i64::from(from);
        for part in parts {
            if i64::from(part.first) > hour {
                push(hour, i64::from(part.first) - 1, &mut out);
            }
            hour = hour.max(i64::from(part.last) + 1);
        }
        if hour <= i64::from(to) {
            push(hour, i64::from(to), &mut out);
        }
        out
    }

    fn hourly(&self, runs: Vec<ChartRun>, from: i32, to: i32, width: u32) -> ChartSummary {
        let scale = self.scale();
        let hours = i64::from(to) - i64::from(from) + 1;
        let bin_hours = (hours + i64::from(width) - 1) / i64::from(width);
        let bins = ((hours + bin_hours - 1) / bin_hours) as usize;
        let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins];
        for run in &runs {
            let mut hour = i64::from(run.first_hour).max(i64::from(from));
            let end = i64::from(run.last_hour).min(i64::from(to));
            let mut broken = run.break_before;
            while hour <= end {
                let bin = ((hour - i64::from(from)) / bin_hours) as usize;
                let bin_end = (i64::from(from) + (bin as i64 + 1) * bin_hours - 1).min(end);
                let now = ChartBucket {
                    absolute_total: (run.value.abs() * scale) as u128 * (bin_end - hour + 1) as u128,
                    first_hour: hour as i32,
                    last_hour: bin_end as i32,
                    first_value: run.value,
                    last_value: run.value,
                    last_value_hour: hour as i32,
                    min_hour: hour as i32,
                    min_value: run.value,
                    max_hour: hour as i32,
                    max_value: run.value,
                    first_break: broken,
                    has_break: false,
                };
                buckets[bin] = match buckets[bin].take() {
                    None => Some(now),
                    Some(held) => {
                        let mut merged = crate::chart::merge(Some(held), Some(now)).expect("both held");
                        merged.has_break |= broken;
                        Some(merged)
                    }
                };
                broken = false;
                hour = bin_end + 1;
            }
        }
        let clipped = runs
            .into_iter()
            .filter(|run| run.last_hour >= from && run.first_hour <= to)
            .collect();
        ChartSummary {
            from: f64::from(from),
            to: f64::from(to),
            width,
            buckets,
            runs: (hours <= i64::from(width)).then_some(clipped),
        }
    }

    pub fn navigate<C: Clone + Into<Country>>(&self, countries: &[C], measures: &[&'static Measure], origin: Option<i32>, key: NavigationKey, bounds: (i32, i32)) -> io::Result<Option<i32>> {
        let asks: Vec<Ask> = countries.iter().map(|country| Ask::whole(country.clone())).collect();
        self.navigate_lines(&asks, measures, origin, key, bounds, &Within::new())
    }

    pub fn navigate_lines(&self, asks: &[Ask], measures: &[&'static Measure], origin: Option<i32>, key: NavigationKey, bounds: (i32, i32), within: &Within) -> io::Result<Option<i32>> {
        let held = |hour: &i32| within.is_empty() || asks.iter().any(|ask| within.get(&ask.country).is_none_or(|spans| spans.iter().any(|&(first, last)| first <= *hour && *hour <= last)));
        Ok(self.change_lines(asks, measures, origin, key, bounds, within)?.or_else(|| origin.map(|hour| hour.clamp(bounds.0, bounds.1)).filter(held)))
    }

    pub fn change_lines(&self, asks: &[Ask], measures: &[&'static Measure], origin: Option<i32>, key: NavigationKey, bounds: (i32, i32), within: &Within) -> io::Result<Option<i32>> {
        if bounds.0 > bounds.1 {
            return Err(invalid("Invalid navigation bounds"));
        }
        if self.query.replay.recording().withheld_reason(self.series.table.index()).is_some() {
            return Ok(None);
        }
        let forward = matches!(key, NavigationKey::ArrowRight | NavigationKey::Home);
        let at: i64 = match key {
            NavigationKey::Home => i64::from(bounds.0) - 1,
            NavigationKey::End => i64::from(bounds.1) + 1,
            _ => origin.map_or(if forward { i64::from(bounds.0) - 1 } else { i64::from(bounds.1) + 1 }, i64::from),
        };
        let recording = self.query.replay.recording();
        let mut best: Option<i32> = None;
        let mut offer = |hour: i32| {
            if (forward && i64::from(hour) <= at) || (!forward && i64::from(hour) >= at) {
                return;
            }
            best = Some(best.map_or(hour, |held: i32| if forward { held.min(hour) } else { held.max(hour) }));
        };
        let mut by_stretch: HashMap<(u32, i32, i32), Vec<Part>> = HashMap::new();
        for ask in asks {
            for held in self.held(ask, bounds.0, bounds.1).into_iter().filter_map(|part| self.valid_part(&part)) {
                for part in within_spans(held, within.get(&ask.country).map(Vec::as_slice)) {
                    let starts = i64::from(part.first);
                    if (forward && starts > at) || (!forward && starts < at) {
                        offer(part.first);
                    }
                    by_stretch.entry((part.run, part.first, part.last)).or_default().push(part);
                }
            }
        }
        for ((run, first, last), parts) in by_stretch {
            let mut keys: Vec<u128> = parts.iter().map(|part| part.key).collect();
            keys.sort_unstable();
            keys.dedup();
            for measure in measures {
                let source = dashboard_declare::source(self.series.table, measure)?;
                let column = dashboard_declare::summary_id(self.series.table, measure)?;
                if let Some(hour) = read::nearest_change(recording, run, self.series.table, source, Some(column), &keys, at, forward, (first, last))?
                    && first <= hour
                    && hour <= last
                {
                    offer(hour);
                }
            }
        }
        Ok(best)
    }

    fn drop_patterns<C: Clone + Into<Country>>(&self, tags: &[C], from: i32, to: i32) -> io::Result<Vec<Pattern>> {
        let kind = self.series.measures.iter().position(|measure| std::ptr::eq(measure, self.measure)).ok_or_else(|| invalid("No such measure"))?;
        let mask = u128::from(u32::MAX) << log::facet_shift(0) | u128::from(u32::MAX) << log::facet_shift(1);
        let mut out = Vec::new();
        for (group, tag) in tags.iter().enumerate() {
            for part in self.parts(tag.clone(), from, to) {
                out.push(Pattern {
                    run: part.run,
                    first: part.first,
                    last: part.last,
                    mask,
                    value: (kind as u128) << log::facet_shift(0) | u128::from(part.key as u32) << log::facet_shift(1),
                    group,
                });
            }
        }
        Ok(out)
    }

    fn drops_log(&self) -> io::Result<&log::Reader<'q, 'a>> {
        self.drops.as_ref().ok_or_else(|| invalid(format!("{} has no drops", self.series.kind)))
    }

    pub(crate) fn drop_counts<C: Clone + Into<Country>>(&self, tags: &[C], first: i32, last: i32, bin_hours: i64) -> io::Result<Vec<(u64, Vec<(u32, u64)>)>> {
        let patterns = self.drop_patterns(tags, self.query.timeline.first, last)?;
        let counts = self.drops_log()?.counts(&patterns, tags.len(), first, last, bin_hours)?;
        Ok(counts.into_iter().map(|counts| (counts.before, counts.bins)).collect())
    }

    pub fn marks_of<C: Clone + Into<Country>>(&self, tags: &[C], from: i32, to: i32, width: u32) -> io::Result<Vec<MarkSummary>> {
        let hours = i64::from(to) - i64::from(from) + 1;
        let span = ((hours * 18 + i64::from(width) - 1) / i64::from(width)).max(1);
        let bin = log::bin_for(span, 1);
        let patterns = self.drop_patterns(tags, self.query.timeline.first, to)?;
        let counted = self.drops_log()?.counts(&patterns, tags.len(), from, to, bin)?;
        let first_bin = i64::from(from).div_euclid(bin) * bin;
        let mut out = Vec::with_capacity(tags.len());
        for counts in counted {
            let mut ordinal = counts.before;
            let mut clusters: Vec<MarkCluster> = Vec::new();
            let mut current: Option<(i64, MarkCluster, u64)> = None;
            for (index, count) in counts.bins {
                let start = first_bin + i64::from(index) * bin;
                let first = start.max(i64::from(from)) as i32;
                let last = (start + bin - 1).min(i64::from(to)) as i32;
                let cell = (i64::from(first) - i64::from(from)) / span;
                match &mut current {
                    Some((held, cluster, total)) if *held == cell => {
                        cluster.last_hour = last;
                        *total += count;
                    }
                    _ => {
                        if let Some((_, mut cluster, total)) = current.take() {
                            cluster.count = total.to_string();
                            clusters.push(cluster);
                        }
                        current = Some((cell, MarkCluster { first_hour: first, last_hour: last, first_ordinal: ordinal.to_string(), count: String::new() }, count));
                    }
                }
                ordinal += count;
            }
            if let Some((_, mut cluster, total)) = current {
                cluster.count = total.to_string();
                clusters.push(cluster);
            }
            out.push(MarkSummary { from: f64::from(from), to: f64::from(to), width, clusters });
        }
        Ok(out)
    }

    pub fn part_lives<C: Clone + Into<Country>>(&self, tags: &[C], from: i32, to: i32) -> io::Result<Vec<SeriesOwnerParts>> {
        let parts = self.series.parts.ok_or_else(|| invalid(format!("{} has one row an owner: it has no parts", self.series.kind)))?;
        if dashboard_declare::read_by_key(self.series.table) {
            return Err(invalid(format!("{} is not summarized: its parts' lifetimes are not offered", self.series.kind)));
        }
        if from > to {
            return Err(invalid("Invalid span"));
        }
        let recording = self.query.replay.recording();
        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            let mut found: BTreeMap<u128, Vec<(i32, i32)>> = BTreeMap::new();
            let country: Country = tag.clone().into();
            let held = self.parts(&country, from, to);
            for part in held.iter().filter_map(|part| self.valid_part(part)) {
                let (lo, hi) = parts.range(parts.owner(part.key));
                let run = crate::cost::presences(recording, part.run, self.series.table, (lo, hi), i64::from(part.first), i64::from(part.last))?;
                for (key, spans) in &run {
                    for &(first, last) in spans {
                        let (first, last) = (first.max(i64::from(part.first)), last.min(i64::from(part.last)));
                        if first <= last {
                            found.entry(parts.part(*key)).or_default().push((first as i32, last as i32));
                        }
                    }
                }
            }
            let mut missing = self.unheld(&country, from, to, &held);
            for part in &held {
                self.part_coverage_table(part, from, to, &mut missing)?;
            }
            out.push(SeriesOwnerParts {
                country,
                parts: found
                    .into_iter()
                    .flat_map(|(part, mut spans)| {
                        spans.sort_unstable();
                        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
                        for (first, last) in spans {
                            match merged.last_mut() {
                                Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
                                _ => merged.push((first, last)),
                            }
                        }
                        let id = parts.id(part).map(|id| id.to_string());
                        merged.into_iter().map(move |(first_hour, last_hour)| SeriesPartLife { part: part.to_string(), id: id.clone(), first_hour, last_hour })
                    })
                    .collect(),
                missing: joined(missing),
            });
        }
        Ok(out)
    }
}

fn status_reason(table: replay_data::rows::TableId, status: &Status) -> Option<String> {
    let what = table.words();
    match status {
        Status::Complete => None,
        Status::NotRecorded => Some(format!("{what}: not recorded at this hour")),
        Status::Truncated { missing } => Some(format!("{what}: read only in part at this hour ({missing} rows not read)")),
        Status::Unavailable { .. } => Some(format!("{what}: not available at this hour")),
    }
}

fn joined(mut spans: Vec<MissingSpan>) -> Vec<MissingSpan> {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut out: Vec<MissingSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if held.reason == span.reason && i64::from(held.last_hour) + 1 == i64::from(span.first_hour) => held.last_hour = span.last_hour,
            _ => out.push(span),
        }
    }
    out
}

fn meet(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            out.push((first, last));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn union(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.sort_unstable();
    spans.dedup_by(|next, kept| {
        let joins = next.0 <= kept.1.saturating_add(1);
        if joins {
            kept.1 = kept.1.max(next.1);
        }
        joins
    });
    spans
}

struct Aliases {
    by_slot: HashMap<i64, Vec<(u32, i32, i32)>>,
    slots: HashMap<u32, Vec<(i32, i32, i64)>>,
}

impl Aliases {
    fn new(links: &HashMap<u128, read::Runs>) -> Self {
        let (mut by_slot, mut slots): (HashMap<i64, Vec<(u32, i32, i32)>>, HashMap<u32, Vec<(i32, i32, i64)>>) = (HashMap::new(), HashMap::new());
        for (&id, runs) in links {
            let id = id as u32;
            for &(first, last, slot) in runs {
                if let Some(slot) = slot {
                    by_slot.entry(slot).or_default().push((id, first, last));
                    slots.entry(id).or_default().push((first, last, slot));
                }
            }
        }
        Self { by_slot, slots }
    }

    fn of(&self, id: u32, from: i32, to: i32) -> Vec<(u32, Vec<(i32, i32)>)> {
        let mut found: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::from([(id, vec![(from, to)])]);
        for &(first, last, slot) in self.slots.get(&id).into_iter().flatten() {
            for &(other, lo, hi) in self.by_slot.get(&slot).into_iter().flatten() {
                let (lo, hi) = (lo.max(first).max(from), hi.min(last).min(to));
                if other != id && lo <= hi {
                    found.entry(other).or_default().push((lo, hi));
                }
            }
        }
        found.into_iter().map(|(alias, spans)| (alias, union(spans))).collect()
    }
}

fn pair_rows(rows: &[(&[(i32, i32, Option<i64>)], &[(i32, i32)])]) -> read::Runs {
    let mut pieces: Vec<(i32, i32, i64)> = Vec::new();
    for &(runs, spans) in rows {
        for &(first, last, value) in runs {
            let Some(value) = value else { continue };
            for &(lo, hi) in spans {
                let (lo, hi) = (lo.max(first), hi.min(last));
                if lo <= hi {
                    pieces.push((lo, hi, value));
                }
            }
        }
    }
    pieces.sort_by_key(|piece| piece.0);
    let mut out: read::Runs = Vec::with_capacity(pieces.len());
    let mut at = i32::MIN;
    for (first, last, value) in pieces {
        let first = first.max(at);
        if first <= last {
            out.push((first, last, Some(value)));
            at = last.saturating_add(1);
        }
    }
    out
}

fn zero_absent(held: &read::Runs, zero: &[(i32, i32)], from: i32, to: i32) -> read::Runs {
    let mut whole: read::Runs = Vec::new();
    let mut at = from;
    for &(first, last, value) in held {
        let (first, last) = (first.max(from), last.min(to));
        if first > last {
            continue;
        }
        if first > at {
            whole.push((at, first - 1, None));
        }
        whole.push((first, last, value));
        at = last.saturating_add(1);
    }
    if at <= to {
        whole.push((at, to, None));
    }
    let mut out: read::Runs = Vec::new();
    let mut push = |first: i32, last: i32, value: Option<i64>| match out.last_mut() {
        Some(held) if held.2 == value && held.1.checked_add(1) == Some(first) => held.1 = last,
        _ => out.push((first, last, value)),
    };
    for (first, last, value) in whole {
        if value.is_some() {
            push(first, last, value);
            continue;
        }
        let mut at = first;
        for &(lo, hi) in zero.iter().filter(|&&(lo, hi)| hi >= first && lo <= last) {
            let (lo, hi) = (lo.max(first), hi.min(last));
            if lo > at {
                push(at, lo - 1, None);
            }
            push(lo, hi, Some(0));
            at = hi + 1;
        }
        if at <= last {
            push(at, last, None);
        }
    }
    out
}

fn pair_covered_rows(rows: &[(&[(i32, i32, read::CoveredValue)], &[(i32, i32)])]) -> read::CoveredRuns {
    let mut pieces: Vec<(i32, i32, read::CoveredValue)> = Vec::new();
    for &(runs, spans) in rows {
        for &(first, last, value) in runs {
            if matches!(value, read::CoveredValue::Uncovered) {
                continue;
            }
            for &(lo, hi) in spans {
                let (lo, hi) = (lo.max(first), hi.min(last));
                if lo <= hi {
                    pieces.push((lo, hi, value));
                }
            }
        }
    }
    pieces.sort_by_key(|piece| (piece.0, matches!(piece.2, read::CoveredValue::Unvalued)));
    let mut out: read::CoveredRuns = Vec::with_capacity(pieces.len());
    let mut at = i32::MIN;
    for (first, last, value) in pieces {
        let first = first.max(at);
        if first <= last {
            out.push((first, last, value));
            at = last.saturating_add(1);
        }
    }
    out
}

fn zero_absent_covered(held: &read::CoveredRuns, zero: &[(i32, i32)], from: i32, to: i32) -> read::CoveredRuns {
    let mut whole: read::CoveredRuns = Vec::new();
    let mut at = from;
    for &(first, last, value) in held {
        let (first, last) = (first.max(from), last.min(to));
        if first > last {
            continue;
        }
        if first > at {
            whole.push((at, first - 1, read::CoveredValue::Uncovered));
        }
        whole.push((first, last, value));
        at = last.saturating_add(1);
    }
    if at <= to {
        whole.push((at, to, read::CoveredValue::Uncovered));
    }
    let mut out: read::CoveredRuns = Vec::new();
    let mut push = |first: i32, last: i32, value: read::CoveredValue| match out.last_mut() {
        Some(held) if held.2 == value && held.1.checked_add(1) == Some(first) => held.1 = last,
        _ => out.push((first, last, value)),
    };
    for (first, last, value) in whole {
        if !matches!(value, read::CoveredValue::Uncovered) {
            push(first, last, value);
            continue;
        }
        let mut at = first;
        for &(lo, hi) in zero.iter().filter(|&&(lo, hi)| hi >= first && lo <= last) {
            let (lo, hi) = (lo.max(first), hi.min(last));
            if lo > at {
                push(at, lo - 1, read::CoveredValue::Uncovered);
            }
            push(lo, hi, read::CoveredValue::Value(0));
            at = hi + 1;
        }
        if at <= last {
            push(at, last, read::CoveredValue::Uncovered);
        }
    }
    out
}
