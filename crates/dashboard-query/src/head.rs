use crate::{
    Query,
    identity::{Country, Holder, Holders},
    read,
    series::{Ask, Line, MissingSpan, Part, SeriesLineAsk, Span, board, hour_of},
};
use dashboard_declare::{Measure, Series};
use replay_columnar::{
    manifest::Status,
    summary::{Computed, Source},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, PoisonError},
};
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const PRODUCT: &str = "series-head";

const MAX_INDEXED_COUNTRIES: usize = 2048;
const MAX_MARK_COUNTRIES: usize = 2048;
const MAX_MARK_CHECKPOINTS: usize = 64;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    kind: String,
    measure: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    family: Family,
    hour: Option<f64>,
    bin_hours: i64,
    #[serde(default)]
    countries: Vec<Country>,
    #[serde(default)]
    lines: Vec<SeriesLineAsk>,
    within: Option<Vec<(Country, Vec<Span>)>>,
    #[serde(default)]
    with_marks: bool,
    top: Option<u32>,
    least: Option<bool>,
}

pub fn params_walk() -> crate::fields::Walk {
    crate::fields::walk::<Params>()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadOpenBin {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_value: f64,
    pub last_value: f64,
    pub last_value_hour: i32,
    pub min_hour: i32,
    pub min_value: f64,
    pub max_hour: i32,
    pub max_value: f64,
    pub first_break: bool,
    pub has_break: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadValue {
    pub value_raw: Option<String>,
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub missing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub not_applicable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadMarks {
    pub before: String,
    #[ts(type = "number")]
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadLine {
    pub country: Country,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub part: Option<String>,
    pub open_bin: Option<HeadOpenBin>,
    pub value: HeadValue,
    pub missing: Vec<MissingSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub marks: Option<HeadMarks>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadRosterEntry {
    pub country: Country,
    pub value_raw: String,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeadRoster {
    pub top: u32,
    pub least: bool,
    pub entries: Vec<HeadRosterEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesHeadReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub hour: i32,
    #[ts(type = "number")]
    pub bin_hours: i64,
    pub bin_first_hour: i32,
    pub bin_last_hour: i32,
    pub closed_through: Option<i32>,
    pub lines: Vec<HeadLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub roster: Option<HeadRoster>,
}

#[derive(Clone)]
pub struct ParsedHeadParams {
    pub branch_id: String,
    pub series: &'static Series,
    pub measure: &'static Measure,
    pub bin_hours: i64,
    pub asks: Vec<Ask>,
    pub within: Option<BTreeMap<Country, Vec<(i32, i32)>>>,
    pub with_marks: bool,
    pub top: Option<u32>,
    pub least: bool,
}

impl ParsedHeadParams {
    pub fn within_contains(&self, country: &Country, hour: i32) -> bool {
        let Some(within) = &self.within else {
            return true;
        };
        let Some(spans) = within.get(country) else {
            return false;
        };
        let at = spans.partition_point(|&(_, last)| last < hour);
        spans.get(at).is_some_and(|&(first, _)| first <= hour)
    }

    pub fn within_spans_in(&self, country: &Country, from: i32, to: i32) -> Vec<(i32, i32)> {
        if from > to {
            return Vec::new();
        }
        let Some(within) = &self.within else {
            return vec![(from, to)];
        };
        let Some(spans) = within.get(country) else {
            return Vec::new();
        };
        let start = spans.partition_point(|&(_, last)| last < from);
        spans[start..]
            .iter()
            .take_while(|&&(first, _)| first <= to)
            .filter_map(|&(first, last)| {
                let (lo, hi) = (first.max(from), last.min(to));
                (lo <= hi).then_some((lo, hi))
            })
            .collect()
    }
}

pub fn coverage_missing(line: &Line<'_, '_>, country: &Country, from: i32, to: i32, parts: &[Part]) -> io::Result<Vec<MissingSpan>> {
    line.missing(country.clone(), from, to, parts)
}

pub fn bin_bounds(hour: i32, bin_hours: i64) -> io::Result<(i32, i32, Option<i32>)> {
    let first = i64::from(hour).div_euclid(bin_hours) * bin_hours;
    let bin_first = i32::try_from(first).map_err(|_| invalid("Bin start underflows i32"))?;
    let bin_last = (first + bin_hours - 1).min(i64::from(i32::MAX)) as i32;
    let closed_through = first.checked_sub(1).and_then(|value| i32::try_from(value).ok());
    Ok((bin_first, bin_last, closed_through))
}

fn normalize_within(raw: Vec<(Country, Vec<Span>)>) -> BTreeMap<Country, Vec<(i32, i32)>> {
    let mut map: BTreeMap<Country, Vec<(i32, i32)>> = BTreeMap::new();
    for (country, spans) in raw {
        let entry = map.entry(country).or_default();
        for span in spans {
            if span.first_hour <= span.last_hour {
                entry.push((span.first_hour, span.last_hour));
            }
        }
    }
    for spans in map.values_mut() {
        spans.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for (first, last) in spans.drain(..) {
            match merged.last_mut() {
                Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        *spans = merged;
    }
    map
}

pub fn parse_params(query: &Query<'_>, params: &serde_json::Value, require_hour: bool) -> io::Result<(ParsedHeadParams, Option<i32>)> {
    let parsed: Params = crate::named::named(params).map_err(|error| invalid(format!("Invalid series-head request: {error}")))?;
    let series = dashboard_declare::series(&parsed.family.kind)
        .ok_or_else(|| invalid(format!("Unknown series {}", parsed.family.kind)))?;
    let measure = series
        .measure(&parsed.family.measure)
        .ok_or_else(|| invalid(format!("{} has no measure {}", series.kind, parsed.family.measure)))?;
    crate::tile::level_of(parsed.bin_hours)?;
    let hour = match (require_hour, parsed.hour) {
        (true, hour) => Some(hour_of(hour, "hour")?),
        (false, Some(hour)) => Some(hour_of(Some(hour), "hour")?),
        (false, None) => None,
    };
    let line = Line::new(query, series, measure)?;
    let asks = if let Some(top) = parsed.top {
        if !parsed.countries.is_empty() || !parsed.lines.is_empty() {
            return Err(invalid("A ranked series-head names no explicit countries or lines"));
        }
        if top == 0 {
            return Err(invalid(format!("A ranked {} names a top of at least 1", series.kind)));
        }
        if series.key.unowned() {
            return Err(invalid(format!("{} is rows no country owns: a ranked head is not offered", series.kind)));
        }
        if series.parts.is_some() {
            return Err(invalid(format!("{} ranks parts, not countries: a ranked head is not offered", series.kind)));
        }
        if parsed.within.is_some() {
            return Err(invalid("A ranked series-head takes no within spans"));
        }
        Vec::new()
    } else {
        let asks = line.asks(&parsed.countries, &parsed.lines)?;
        for ask in &asks {
            if series.key.unowned() && ask.country.tag().and_then(|tag| series.key.fixed(tag)).is_none() {
                return Err(invalid(format!("{} names no row of {}", ask.country, series.kind)));
            }
            if !series.key.unowned() && !query.holders.named(&ask.country) {
                return Err(invalid(match &ask.country {
                    Country::Tag(tag) => format!("No country of this history holds the tag {tag}"),
                    Country::Slot(slot) => format!("No country of this history is the country {slot}"),
                }));
            }
        }
        asks
    };
    let within = parsed.within.map(normalize_within);
    Ok((
        ParsedHeadParams {
            branch_id: parsed.branch_id,
            series,
            measure,
            bin_hours: parsed.bin_hours,
            asks,
            within,
            with_marks: parsed.with_marks,
            top: parsed.top,
            least: parsed.least.unwrap_or(false),
        },
        hour,
    ))
}

#[derive(Clone, Debug)]
pub struct OpenBinAcc {
    pub bin_first: i32,
    pub bin_hours: i64,
    pub prev_hour: Option<i32>,
    pub prev_present: bool,
    pub prev_value: Option<i64>,
    pub bin: Option<HeadOpenBin>,
}

impl OpenBinAcc {
    pub fn new(bin_first: i32, bin_hours: i64, prev_present: bool) -> Self {
        Self { bin_first, bin_hours, prev_hour: None, prev_present, prev_value: None, bin: None }
    }

    pub fn push_span(&mut self, from: i32, to: i32, value: Option<i64>, starts_at_from: bool, scale: f64) {
        if from > to {
            return;
        }
        let mut cursor = i64::from(from);
        let end = i64::from(to);
        let mut starts = starts_at_from;
        while cursor <= end {
            let bin_start = cursor.div_euclid(self.bin_hours) * self.bin_hours;
            let bin_end = (bin_start + self.bin_hours - 1).min(i64::from(i32::MAX));
            let chunk_end = end.min(bin_end);
            if bin_start != i64::from(self.bin_first) {
                let contiguous = self.prev_hour.is_some_and(|prev| i64::from(prev) + 1 == cursor);
                self.prev_present = contiguous && self.prev_present;
                self.bin_first = bin_start as i32;
                self.bin = None;
                self.prev_value = None;
            }
            let contiguous_from_prev = self.prev_hour.is_none_or(|prev| i64::from(prev) + 1 == cursor);
            let entering_present = contiguous_from_prev && self.prev_present;
            match value {
                Some(raw) => {
                    let v = raw as f64 / scale;
                    let c_from = cursor as i32;
                    let c_to = chunk_end as i32;
                    match &mut self.bin {
                        None => {
                            let first_break = !entering_present || starts || c_from > self.bin_first;
                            self.bin = Some(HeadOpenBin {
                                first_hour: c_from,
                                last_hour: c_to,
                                first_value: v,
                                last_value: v,
                                last_value_hour: c_from,
                                min_hour: c_from,
                                min_value: v,
                                max_hour: c_from,
                                max_value: v,
                                first_break,
                                has_break: false,
                            });
                        }
                        Some(held) => {
                            let break_here = !entering_present || starts || i64::from(held.last_hour) + 1 != cursor;
                            if break_here {
                                held.has_break = true;
                            }
                            if break_here || self.prev_value != Some(raw) {
                                held.last_value_hour = c_from;
                            }
                            held.last_hour = c_to;
                            held.last_value = v;
                            if v < held.min_value {
                                held.min_value = v;
                                held.min_hour = c_from;
                            }
                            if v > held.max_value {
                                held.max_value = v;
                                held.max_hour = c_from;
                            }
                        }
                    }
                    self.prev_hour = Some(c_to);
                    self.prev_present = true;
                    self.prev_value = Some(raw);
                }
                None => {
                    self.prev_hour = Some(chunk_end as i32);
                    self.prev_present = false;
                    self.prev_value = None;
                }
            }
            starts = false;
            cursor = chunk_end + 1;
        }
    }
}

pub struct BranchHeadIndex {
    timeline_edges: Vec<i64>,
    holder_changes: Vec<(i32, Vec<String>)>,
    per_country: Mutex<HashMap<Country, Arc<Vec<i64>>>>,
}

impl BranchHeadIndex {
    pub fn build(query: &Query<'_>) -> Self {
        let mut timeline_edges: Vec<i64> = Vec::new();
        for stretch in &query.timeline.stretches {
            timeline_edges.push(i64::from(stretch.first));
            timeline_edges.push(i64::from(stretch.last) + 1);
        }
        for gap in &query.timeline.unheld {
            timeline_edges.push(i64::from(gap.first));
            timeline_edges.push(i64::from(gap.last) + 1);
        }
        for &(first, last, _) in &query.holders.unknown {
            timeline_edges.push(i64::from(first));
            timeline_edges.push(i64::from(last) + 1);
        }
        timeline_edges.sort_unstable();
        timeline_edges.dedup();

        let mut changes_by_hour: BTreeMap<i32, Vec<String>> = BTreeMap::new();
        for (tag, runs) in &query.holders.tags {
            for run in runs {
                changes_by_hour.entry(run.first).or_default().push(tag.clone());
                if let Some(after) = run.last.checked_add(1) {
                    changes_by_hour.entry(after).or_default().push(tag.clone());
                }
            }
        }
        for tags in changes_by_hour.values_mut() {
            tags.sort_unstable();
            tags.dedup();
        }
        let holder_changes: Vec<(i32, Vec<String>)> = changes_by_hour.into_iter().collect();

        Self {
            timeline_edges,
            holder_changes,
            per_country: Mutex::default(),
        }
    }

    pub fn timeline_edges(&self) -> &[i64] {
        &self.timeline_edges
    }

    pub fn holder_changes_in(&self, from: i32, to: i32) -> &[(i32, Vec<String>)] {
        if from > to {
            return &[];
        }
        let start = self.holder_changes.partition_point(|&(hour, _)| hour < from);
        let end = start + self.holder_changes[start..].partition_point(|&(hour, _)| hour <= to);
        &self.holder_changes[start..end]
    }

    pub fn country_edges(&self, holders: &Holders, country: &Country) -> Arc<Vec<i64>> {
        if !holders.named(country) {
            return Arc::new(self.timeline_edges.clone());
        }
        {
            let cache = self.per_country.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(found) = cache.get(country) {
                return Arc::clone(found);
            }
        }
        let mut edges = self.timeline_edges.clone();
        for run in holders.runs(country) {
            edges.push(i64::from(run.first));
            edges.push(i64::from(run.last) + 1);
        }
        edges.sort_unstable();
        edges.dedup();
        let built = Arc::new(edges);
        let mut cache = self.per_country.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= MAX_INDEXED_COUNTRIES {
            cache.clear();
        }
        cache.insert(country.clone(), Arc::clone(&built));
        built
    }
}

type MarkCacheKey = (u64, &'static str, &'static str, Country);

#[derive(Default)]
pub struct MarkPrefixIndex {
    entries: Mutex<HashMap<MarkCacheKey, Vec<(i32, u64)>>>,
}

impl MarkPrefixIndex {
    pub fn clear(&self) {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn record(&self, branch: u64, kind: &'static str, measure: &'static str, holders: &Holders, country: &Country, hour: i32, count: u64) {
        if !holders.named(country) {
            return;
        }
        let key = (branch, kind, measure, country.clone());
        let mut map = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if !map.contains_key(&key) && map.len() >= MAX_MARK_COUNTRIES {
            map.clear();
        }
        let points = map.entry(key).or_default();
        insert_checkpoint(points, hour, count);
    }

    pub fn prefix_through(&self, line: &Line<'_, '_>, country: &Country, through: i32) -> io::Result<u64> {
        let query = line.query();
        if through < query.timeline.first {
            return Ok(0);
        }
        if !query.holders.named(country) {
            return line.drop_interval_count(country, query.timeline.first, through);
        }
        let key = (query.timeline.id, line.series().kind, line.measure_name(), country.clone());
        let cached = {
            let map = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            map.get(&key).and_then(|points| {
                let at = points.partition_point(|&(hour, _)| hour <= through);
                at.checked_sub(1).map(|idx| points[idx])
            })
        };
        let total = match cached {
            Some((cp_hour, cp_count)) if cp_hour == through => return Ok(cp_count),
            Some((cp_hour, cp_count)) => {
                let from = (i64::from(cp_hour) + 1) as i32;
                cp_count + line.drop_interval_count(country, from, through)?
            }
            None => line.drop_interval_count(country, query.timeline.first, through)?,
        };
        let mut map = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if !map.contains_key(&key) && map.len() >= MAX_MARK_COUNTRIES {
            map.clear();
        }
        let points = map.entry(key).or_default();
        insert_checkpoint(points, through, total);
        Ok(total)
    }
}

fn insert_checkpoint(points: &mut Vec<(i32, u64)>, hour: i32, count: u64) {
    match points.binary_search_by_key(&hour, |&(h, _)| h) {
        Ok(at) => points[at].1 = count,
        Err(at) => points.insert(at, (hour, count)),
    }
    if points.len() > MAX_MARK_CHECKPOINTS {
        let last = *points.last().expect("non-empty");
        let mut thinned = Vec::with_capacity(MAX_MARK_CHECKPOINTS);
        for (idx, &point) in points.iter().enumerate() {
            if idx == 0 || idx % 2 == 0 {
                thinned.push(point);
            }
        }
        if thinned.last().copied() != Some(last) {
            thinned.push(last);
        }
        *points = thinned;
    }
}

pub fn answer(query: &Query<'_>, params: &serde_json::Value) -> io::Result<SeriesHeadReply> {
    let (parsed, hour) = parse_params(query, params, true)?;
    let hour = hour.expect("required");
    let line = Line::new(query, parsed.series, parsed.measure)?;
    let (reply, _) = compute_cold_counted(&line, &parsed, hour, None)?;
    Ok(reply)
}

pub fn compute_cold_counted(
    line: &Line<'_, '_>,
    parsed: &ParsedHeadParams,
    hour: i32,
    marks_cache: Option<&MarkPrefixIndex>,
) -> io::Result<(SeriesHeadReply, u64)> {
    let (bin_first, bin_last, closed_through) = bin_bounds(hour, parsed.bin_hours)?;
    let mut rows_visited = 0u64;
    let (asks, roster) = match parsed.top {
        Some(top) => {
            let (entries, rank_rows) = rank_entries_checked(line, hour, top as usize, parsed.least)?;
            rows_visited += rank_rows;
            let asks: Vec<Ask> = entries.iter().map(|entry| Ask::whole(entry.country.clone())).collect();
            (asks, Some(HeadRoster { top, least: parsed.least, entries }))
        }
        None => (parsed.asks.clone(), None),
    };
    let (lines, line_rows) = compute_lines_cold(line, parsed, &asks, hour, bin_first, marks_cache)?;
    rows_visited += line_rows;
    Ok((
        SeriesHeadReply {
            branch_id: parsed.branch_id.clone(),
            measure: line.measure_name().to_owned(),
            decimals: line.decimals(),
            hour,
            bin_hours: parsed.bin_hours,
            bin_first_hour: bin_first,
            bin_last_hour: bin_last,
            closed_through,
            lines,
            roster,
        },
        rows_visited,
    ))
}

pub fn compute_single_line_cold(
    line: &Line<'_, '_>,
    parsed: &ParsedHeadParams,
    ask: &Ask,
    hour: i32,
    marks_cache: Option<&MarkPrefixIndex>,
) -> io::Result<(HeadLine, OpenBinAcc, u64)> {
    let (bin_first, _, _) = bin_bounds(hour, parsed.bin_hours)?;
    let (mut lines, mut accs, rows) = compute_lines_with_acc_cold(line, parsed, std::slice::from_ref(ask), hour, bin_first, marks_cache)?;
    Ok((lines.remove(0), accs.remove(0), rows))
}

fn rank_entries_checked(line: &Line<'_, '_>, hour: i32, n: usize, least: bool) -> io::Result<(Vec<HeadRosterEntry>, u64)> {
    let query = line.query();
    let Some(stretch) = query.timeline.at(hour).copied() else {
        return Err(invalid(query.timeline.reason(hour)));
    };
    if let Some(reason) = line.withheld_at(stretch.run, hour) {
        return Err(invalid(reason));
    }
    let recording = query.replay.recording();
    let columns = read::columns_of(&line.source());
    let snapshot = crate::cost::storage(|| replay_lanes::lanes::at(recording, stretch.run, line.series().table, hour, Some(&columns)))?;
    let status_reason = match snapshot.status {
        Status::Unavailable { reason } => snapshot.strings.get(reason).and_then(|text| std::str::from_utf8(text).ok()),
        _ => None,
    };
    if let Some(reason) = board::table_unrecorded(line.series().table, snapshot.status, status_reason) {
        return Err(invalid(reason));
    }
    let view = snapshot.view();
    let mut rows_visited = 0u64;
    let mut holders_by_key: HashMap<u128, Vec<&str>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        let at = runs.partition_point(|run| run.last < hour);
        if let Some(run) = runs.get(at).filter(|run| run.first <= hour)
            && let Holder::Found { slot, id } = run.holder
        {
            holders_by_key.entry(line.series().key.of(slot, id)).or_default().push(tag.as_str());
        }
    }
    let mut values: Vec<(i64, &str)> = Vec::new();
    let mut seen_keys: HashSet<u128> = HashSet::new();
    for slot in view.rows() {
        rows_visited += 1;
        let Some(key) = view.key(slot) else { continue };
        if line.series().absent_none {
            seen_keys.insert(key);
        }
        let Computed::Value(value) = line.source().read(&view, slot) else { continue };
        if let Some(tags) = holders_by_key.get(&key) {
            for &tag in tags {
                values.push((value, tag));
            }
        }
    }
    if line.series().absent_none {
        for (&key, tags) in &holders_by_key {
            if !seen_keys.contains(&key) {
                for &tag in tags {
                    values.push((0, tag));
                }
            }
        }
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
    let scale = line.scale();
    let entries = values
        .into_iter()
        .take(n)
        .map(|(raw, tag)| HeadRosterEntry {
            country: Country::from(tag),
            value_raw: raw.to_string(),
            value: raw as f64 / scale,
        })
        .collect();
    Ok((entries, rows_visited))
}

fn compute_lines_cold(
    line: &Line<'_, '_>,
    parsed: &ParsedHeadParams,
    asks: &[Ask],
    hour: i32,
    bin_first: i32,
    marks_cache: Option<&MarkPrefixIndex>,
) -> io::Result<(Vec<HeadLine>, u64)> {
    let (lines, _, rows) = compute_lines_with_acc_cold(line, parsed, asks, hour, bin_first, marks_cache)?;
    Ok((lines, rows))
}

fn compute_lines_with_acc_cold(
    line: &Line<'_, '_>,
    parsed: &ParsedHeadParams,
    asks: &[Ask],
    hour: i32,
    bin_first: i32,
    marks_cache: Option<&MarkPrefixIndex>,
) -> io::Result<(Vec<HeadLine>, Vec<OpenBinAcc>, u64)> {
    let query = line.query();
    let recording = query.replay.recording();
    let scale = line.scale();
    let read_from = (i64::from(bin_first) - 1).max(i64::from(i32::MIN)) as i32;
    let has_prev = i64::from(bin_first) > i64::from(i32::MIN);
    let prev_hour = read_from;

    let mut all_parts: Vec<Vec<Part>> = Vec::with_capacity(asks.len());
    let mut valid_parts: Vec<Vec<Part>> = Vec::with_capacity(asks.len());
    let mut by_run: HashMap<u32, (Vec<u128>, i32, i32)> = HashMap::new();
    for ask in asks {
        let held = line.held(ask, read_from, hour);
        let valid: Vec<Part> = held.iter().filter_map(|part| line.valid_part(part)).collect();
        for part in &valid {
            let entry = by_run.entry(part.run).or_insert((Vec::new(), part.first, part.last));
            entry.0.push(part.key);
            entry.1 = entry.1.min(part.first);
            entry.2 = entry.2.max(part.last);
        }
        all_parts.push(held);
        valid_parts.push(valid);
    }

    let mut rows_visited = 0u64;
    let mut key_runs_by_run: HashMap<u32, HashMap<u128, read::Runs>> = HashMap::new();
    let mut statuses_by_run: HashMap<u32, Vec<replay_lanes::lanes::StatusRun>> = HashMap::new();
    for (run, (mut keys, first, last)) in by_run {
        keys.sort_unstable();
        keys.dedup();
        let runs = line.key_runs(run, &keys, first, last)?;
        for list in runs.values() {
            rows_visited += list.len().max(1) as u64;
        }
        if runs.is_empty() {
            rows_visited += keys.len() as u64;
        }
        key_runs_by_run.insert(run, runs);
        if line.series().absent_none {
            statuses_by_run.insert(run, read::statuses(recording, run, line.series().table, first, last)?);
        }
    }

    let mut out_lines: Vec<HeadLine> = Vec::with_capacity(asks.len());
    let mut out_accs: Vec<OpenBinAcc> = Vec::with_capacity(asks.len());
    let mut unresolved: HashMap<u32, Vec<(usize, u128)>> = HashMap::new();

    for (idx, ask) in asks.iter().enumerate() {
        let valid = &valid_parts[idx];
        let bin_parts: Vec<Part> = all_parts[idx]
            .iter()
            .filter_map(|part| {
                let first = part.first.max(bin_first);
                let last = part.last.min(hour);
                (first <= last).then_some(Part { first, last, starts: first == part.segment_first, ..*part })
            })
            .collect();
        let missing = coverage_missing(line, &ask.country, bin_first, hour, &bin_parts)?;

        let prev_present = has_prev
            && parsed.within_contains(&ask.country, prev_hour)
            && value_at_hour(line, valid, &key_runs_by_run, &statuses_by_run, prev_hour).is_some();

        let mut acc = OpenBinAcc::new(bin_first, parsed.bin_hours, prev_present);
        let within_spans = parsed.within_spans_in(&ask.country, bin_first, hour);
        let mut cursor = i64::from(bin_first);
        for &(w_first, w_last) in &within_spans {
            if i64::from(w_first) > cursor {
                acc.push_span(cursor as i32, (i64::from(w_first) - 1) as i32, None, false, scale);
                cursor = i64::from(w_first);
            }
            for part in valid {
                let p_lo = part.first.max(w_first);
                let p_hi = part.last.min(w_last);
                if p_lo > p_hi {
                    continue;
                }
                if i64::from(p_lo) > cursor {
                    acc.push_span(cursor as i32, (i64::from(p_lo) - 1) as i32, None, false, scale);
                }
                let runs = key_runs_by_run.get(&part.run).and_then(|map| map.get(&part.key));
                let complete = statuses_by_run.get(&part.run).map(Vec::as_slice);
                let segments = resolve_part_segments(p_lo, p_hi, runs, complete, line.series().absent_none);
                let mut part_cursor = i64::from(p_lo);
                for (s_first, s_last, val) in segments {
                    if i64::from(s_first) > part_cursor {
                        acc.push_span(part_cursor as i32, (i64::from(s_first) - 1) as i32, None, false, scale);
                    }
                    let starts_here = part.starts && s_first == part.first;
                    acc.push_span(s_first, s_last, val, starts_here, scale);
                    part_cursor = i64::from(s_last) + 1;
                }
                if part_cursor <= i64::from(p_hi) {
                    acc.push_span(part_cursor as i32, p_hi, None, false, scale);
                }
                cursor = i64::from(p_hi) + 1;
            }
            if cursor <= i64::from(w_last) {
                acc.push_span(cursor as i32, w_last, None, false, scale);
                cursor = i64::from(w_last) + 1;
            }
        }
        if cursor <= i64::from(hour) {
            acc.push_span(cursor as i32, hour, None, false, scale);
        }

        let open_bin = if parsed.bin_hours > 1 && line.overlaps_withheld(&bin_parts, bin_first, hour) {
            None
        } else {
            acc.bin.clone()
        };

        let head_value = if !parsed.within_contains(&ask.country, hour) {
            HeadValue { value_raw: None, value: None, missing: None, not_applicable: None }
        } else if let Some(reason) = line.missing_at(&ask.country, hour) {
            HeadValue { value_raw: None, value: None, missing: Some(reason), not_applicable: None }
        } else if let Some(raw) = value_at_hour(line, valid, &key_runs_by_run, &statuses_by_run, hour) {
            HeadValue { value_raw: Some(raw.to_string()), value: Some(raw as f64 / scale), missing: None, not_applicable: None }
        } else if let Some(part) = valid.iter().find(|part| part.first <= hour && hour <= part.last) {
            unresolved.entry(part.run).or_default().push((idx, part.key));
            HeadValue { value_raw: None, value: None, missing: None, not_applicable: None }
        } else {
            HeadValue { value_raw: None, value: None, missing: Some(line.absent(&ask.country, hour)), not_applicable: None }
        };

        let marks = if parsed.with_marks && line.drops() {
            let before = if i64::from(bin_first) > i64::from(query.timeline.first) {
                let prev = (i64::from(bin_first) - 1) as i32;
                match marks_cache {
                    Some(cache) => cache.prefix_through(line, &ask.country, prev)?,
                    None => line.drop_interval_count(&ask.country, query.timeline.first, prev)?,
                }
            } else {
                0
            };
            let count = if hour >= query.timeline.first {
                line.drop_interval_count(&ask.country, bin_first.max(query.timeline.first), hour)?
            } else {
                0
            };
            if let Some(cache) = marks_cache
                && hour >= query.timeline.first
            {
                cache.record(query.timeline.id, line.series().kind, line.measure_name(), &query.holders, &ask.country, hour, before + count);
            }
            Some(HeadMarks { before: before.to_string(), count })
        } else {
            None
        };

        out_lines.push(HeadLine {
            country: ask.country.clone(),
            part: line.part_text(ask),
            open_bin,
            value: head_value,
            missing,
            marks,
        });
        out_accs.push(acc);
    }

    if !unresolved.is_empty() {
        let columns = read::columns_of(&line.source());
        let computed = matches!(line.source(), Source::Computed { .. });
        for (run, items) in unresolved {
            let mut key_asks: Vec<(i32, u128)> = items.iter().map(|&(_, key)| (hour, key)).collect();
            key_asks.sort_unstable();
            key_asks.dedup();
            rows_visited += key_asks.len() as u64;
            let mut resolved: HashMap<u128, (Option<String>, Option<bool>)> = HashMap::new();
            crate::cost::storage(|| {
                replay_lanes::lanes::at_each_with_status(recording, run, line.series().table, Some(&columns), &key_asks, &mut |_h, key, status, row| {
                    let info = match status {
                        Status::Complete => match row {
                            Some((view, slot)) => match line.source().read(&view, slot) {
                                Computed::NotApplicable(reason) => (Some((*reason).to_owned()), Some(true)),
                                Computed::Unrecorded | Computed::Value(_) => (None, computed.then_some(false)),
                            },
                            None => (None, computed.then_some(false)),
                        },
                        other => (board::table_unrecorded(line.series().table, other, None), None),
                    };
                    resolved.insert(key, info);
                    Ok(())
                })
            })?;
            for (idx, key) in items {
                let fallback_reason = board::measure_unrecorded(&query.holders.name(&out_lines[idx].country, hour), line.measure_name());
                match resolved.get(&key) {
                    Some((Some(reason), na)) => {
                        out_lines[idx].value.missing = Some(reason.clone());
                        out_lines[idx].value.not_applicable = *na;
                    }
                    Some((None, na)) => {
                        out_lines[idx].value.missing = Some(fallback_reason);
                        out_lines[idx].value.not_applicable = *na;
                    }
                    None => {
                        out_lines[idx].value.missing = Some(fallback_reason);
                    }
                }
            }
        }
    }

    Ok((out_lines, out_accs, rows_visited))
}

fn value_at_hour(
    line: &Line<'_, '_>,
    valid_parts: &[Part],
    key_runs_by_run: &HashMap<u32, HashMap<u128, read::Runs>>,
    statuses_by_run: &HashMap<u32, Vec<replay_lanes::lanes::StatusRun>>,
    hour: i32,
) -> Option<i64> {
    let part = valid_parts.iter().find(|part| part.first <= hour && hour <= part.last)?;
    if let Some(runs) = key_runs_by_run.get(&part.run).and_then(|map| map.get(&part.key)) {
        let at = runs.partition_point(|run| run.1 < hour);
        if let Some(&(first, _, value)) = runs.get(at)
            && first <= hour
        {
            if let Some(val) = value {
                return Some(val);
            }
        }
    }
    if line.series().absent_none
        && let Some(statuses) = statuses_by_run.get(&part.run)
    {
        let at = statuses.partition_point(|run| run.last < hour);
        if let Some(run) = statuses.get(at)
            && run.first <= hour
            && matches!(run.status, Status::Complete)
        {
            return Some(0);
        }
    }
    None
}

fn resolve_part_segments(
    lo: i32,
    hi: i32,
    runs: Option<&read::Runs>,
    statuses: Option<&[replay_lanes::lanes::StatusRun]>,
    absent_none: bool,
) -> Vec<(i32, i32, Option<i64>)> {
    let mut raw: Vec<(i32, i32, Option<i64>)> = Vec::new();
    let mut at = i64::from(lo);
    if let Some(runs) = runs {
        let start = runs.partition_point(|run| run.1 < lo);
        for &(first, last, value) in &runs[start..] {
            if first > hi {
                break;
            }
            let a = first.max(lo);
            let b = last.min(hi);
            if a > b {
                continue;
            }
            if i64::from(a) > at {
                raw.push((at as i32, (i64::from(a) - 1) as i32, None));
            }
            raw.push((a, b, value));
            at = i64::from(b) + 1;
        }
    }
    if at <= i64::from(hi) {
        raw.push((at as i32, hi, None));
    }
    if !absent_none {
        return raw;
    }
    let Some(statuses) = statuses else {
        return raw;
    };
    let mut out: Vec<(i32, i32, Option<i64>)> = Vec::with_capacity(raw.len());
    for (first, last, value) in raw {
        if value.is_some() {
            out.push((first, last, value));
            continue;
        }
        let mut cursor = i64::from(first);
        let start = statuses.partition_point(|run| run.last < first);
        for run in &statuses[start..] {
            if run.first > last {
                break;
            }
            let a = run.first.max(first);
            let b = run.last.min(last);
            if a > b {
                continue;
            }
            if i64::from(a) > cursor {
                out.push((cursor as i32, (i64::from(a) - 1) as i32, None));
            }
            out.push((a, b, matches!(run.status, Status::Complete).then_some(0)));
            cursor = i64::from(b) + 1;
        }
        if cursor <= i64::from(last) {
            out.push((cursor as i32, last, None));
        }
    }
    out
}
