use super::{
    Content, EquipmentProducedRaceParams, EquipmentProducedRaceReply, MAX_EXACT, ProducedCount, ProducedMeasure, ProducedUnknown,
    Typed, asked_parts, back_of, decimals, invalid, produced_measure, run_types,
    unknown_of,
};
#[cfg(not(test))]
use super::race_reply;
#[cfg(test)]
use tests::counted_race as race_reply;
use dashboard_base::{
    Source,
    products::{Country, LoadContext, validate_country},
    span::HourSpan,
};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, merge},
    identity::Holder,
    series::{Line, MissingSpan, totals::{self, Gap}},
    tile::{self, BINS, SeriesTileReply, TileLine, TileRanking},
};
use dashboard_text::metadata::check_cancel;
use replay_columnar::manifest::Status;
use replay_data::rows::{ProducedValueRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "equipment-produced-lines";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquipmentProducedTileFamily {
    pub kind: String,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[serde(default)]
    pub types: Vec<String>,
    pub refits: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<EquipmentProducedTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    #[expect(dead_code, reason = "names the branch this request is routed by")]
    branch_id: String,
    #[expect(dead_code, reason = "read as the equipment produced tile family")]
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    within: Option<Vec<(Country, Vec<HourSpan>)>>,
    top: Option<u32>,
    hour: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct DrawnPiece {
    first: i32,
    last: i32,
    value: f64,
    break_before: bool,
}

pub(super) fn unknown_reason(gap: Gap) -> &'static str {
    let reason = match gap {
        Gap::NotRecorded => ProducedUnknown::NotRecorded,
        Gap::Cell { reason, .. } => unknown_of(reason),
        Gap::WindowIncomplete => ProducedUnknown::WindowIncomplete,
        Gap::Restarted => ProducedUnknown::ObservationRestarted,
        Gap::OutOfRange => ProducedUnknown::OutOfRange,
    };
    match reason {
        ProducedUnknown::NotRecorded => "Production was not recorded for this country then.",
        ProducedUnknown::UnitsInexact => "A production line left a delivery out of its count.",
        ProducedUnknown::SpendingUnavailable => "The factory IC a line applied was not recorded.",
        ProducedUnknown::LostOutput => "A production line went without its final count.",
        ProducedUnknown::MissedPublication => "Production was not read at some hours.",
        ProducedUnknown::TypeNotRecorded => "Designs whose type was not recorded hold output that could be of this type.",
        ProducedUnknown::WindowIncomplete => "The window reaches before this country's recorded production.",
        ProducedUnknown::ObservationRestarted => "Production counting began again inside the window.",
        ProducedUnknown::RocketTableUnavailable => "Rocket site output was not recorded then.",
        ProducedUnknown::RocketInputInexact => "Rocket site output was not counted exactly.",
        ProducedUnknown::RocketOwnerUnreadable => "Rocket site output of an unknown country was recorded.",
        ProducedUnknown::OutOfRange => "The amount is too large to show exactly.",
    }
}

fn push_missing(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: &str) {
    if first_hour > last_hour {
        return;
    }
    if let Some(prev) = out.last_mut()
        && prev.reason == reason
        && prev.last_hour.saturating_add(1) >= first_hour
    {
        prev.last_hour = prev.last_hour.max(last_hour);
        return;
    }
    out.push(MissingSpan { first_hour, last_hour, reason: reason.to_owned() });
}

fn clip_pieces(pieces: Vec<DrawnPiece>, within: Option<&[HourSpan]>) -> Vec<DrawnPiece> {
    let Some(spans) = within else { return pieces };
    let mut out = Vec::new();
    for piece in pieces {
        for span in spans {
            let first = piece.first.max(span.first_hour);
            let last = piece.last.min(span.last_hour);
            if first <= last {
                out.push(DrawnPiece {
                    first,
                    last,
                    value: piece.value,
                    break_before: piece.break_before || first > piece.first,
                });
            }
        }
    }
    out
}

fn clip_missing(spans: &[MissingSpan], within: Option<&[HourSpan]>) -> Vec<MissingSpan> {
    let Some(within) = within else {
        return spans.to_vec();
    };
    let mut out = Vec::new();
    for span in spans {
        for bound in within {
            let first = span.first_hour.max(bound.first_hour);
            let last = span.last_hour.min(bound.last_hour);
            if first <= last {
                push_missing(&mut out, first, last, &span.reason);
            }
        }
    }
    out
}

fn bucketed(lines: &[DrawnPiece], start: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    if clip < start {
        return buckets;
    }
    let start_bin = i64::from(start);
    let end_bin = start_bin + BINS as i64 * bin_hours - 1;
    for line in lines {
        let lo = i64::from(line.first).max(start_bin);
        let hi = i64::from(line.last).min(i64::from(clip)).min(end_bin);
        if lo > hi {
            continue;
        }
        let mut at = lo;
        while at <= hi {
            let slot = ((at - start_bin) / bin_hours) as usize;
            let bin_end = (start_bin + (slot as i64 + 1) * bin_hours - 1).min(hi);
            let from = at as i32;
            let to = bin_end as i32;
            let piece = ChartBucket {
                absolute_total: (line.value.max(0.0) as u128) * (bin_end - at + 1) as u128,
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

fn runs(lines: &[DrawnPiece], first: i32, clip: i32) -> Vec<ChartRun> {
    lines
        .iter()
        .filter(|line| line.last >= first && line.first <= clip)
        .map(|line| ChartRun {
            first_hour: line.first.max(first),
            last_hour: line.last.min(clip),
            value: line.value,
            break_before: line.break_before && line.first >= first,
        })
        .collect()
}

pub(super) enum Baseline {
    Unrestricted,
    Opening(i32),
    From(i32),
    Unavailable(&'static str),
}

impl Baseline {
    pub(super) fn hour(&self) -> Option<i32> {
        match *self {
            Self::From(hour) => Some(hour),
            _ => None,
        }
    }
}

pub(super) const RANGE_BASELINE_UNRECORDED: &str = "Equipment delivered before the selected start was not recorded, so the amount since it is not known";

pub(super) fn range_baseline(line: &Line<'_, '_>, query: &Query<'_>, measure: ProducedMeasure, from_hour: Option<i32>) -> io::Result<Baseline> {
    if measure != ProducedMeasure::Total {
        return Ok(Baseline::Unrestricted);
    }
    let Some(start) = from_hour else {
        return Ok(Baseline::Unrestricted);
    };
    let Some(stretch) = query.timeline.at(start).copied() else {
        return Ok(Baseline::Unavailable(RANGE_BASELINE_UNRECORDED));
    };
    if start <= stretch.first {
        let observed = stretch.first == query.timeline.first && line.clock_at(start)?.since == Some(start);
        return Ok(if observed { Baseline::Opening(start) } else { Baseline::Unavailable(RANGE_BASELINE_UNRECORDED) });
    }
    Ok(line.clock_at(start - 1)?.read.map_or(Baseline::Unavailable(RANGE_BASELINE_UNRECORDED), Baseline::From))
}

struct RankHistory {
    rows: History,
    amount: Option<usize>,
    read: Option<usize>,
    since: Option<usize>,
}

impl RankHistory {
    fn load(query: &Query<'_>, run: u32, keys: &[u128], amount: usize, first: i32, last: i32) -> io::Result<Self> {
        let table = TableId::ProducedValues;
        let read = dashboard_declare::field(table, "as_of")?;
        let since = dashboard_declare::field(table, "since")?;
        let rows = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), run, table, &[amount, read, since], Keys::These(keys), first, last))?;
        Ok(Self { amount: rows.columns.binary_search(&amount).ok(), read: rows.columns.binary_search(&read).ok(), since: rows.columns.binary_search(&since).ok(), rows })
    }

    fn cell(&self, key: u128, at: i32, column: Option<usize>) -> Option<Option<i64>> {
        let Reading::Cells(cells) = self.rows.reading(key, at) else { return None };
        Some(match column.and_then(|i| cells.get(i)) { Some(Cell::Number(value)) => Some(*value), _ => None })
    }

    fn clock(&self, at: i32, column: Option<usize>) -> Option<i32> {
        i32::try_from(self.cell(ProducedValueRow::clock(), at, column)??).ok()
    }

    fn complete(&self, at: i32) -> bool {
        let i = self.rows.statuses.partition_point(|run| run.last < at);
        self.rows.statuses.get(i).is_some_and(|run| run.first <= at && run.status == Status::Complete)
    }

    fn cuts(&self, key: u128, samples: &[RankSample], date: impl Fn(&RankSample) -> i32, cuts: &mut Vec<usize>) {
        if let Ok(i) = self.rows.keys.binary_search_by_key(&key, |row| row.0) {
            cuts.extend(self.rows.keys[i].1.iter().skip(1).map(|step| samples.partition_point(|s| date(s) < step.hour)));
        }
    }
}

struct RankSample {
    hour: i32,
    read: i32,
    start: i32,
}

type Score = Result<Option<i64>, &'static str>;

fn production_keys(parts: &[u128], id: u32) -> impl Clone + Iterator<Item = u128> {
    parts.iter().map(move |&part| ProducedValueRow::key_of(id, part as u32))
}

struct RankWindow<'a> {
    now: &'a RankHistory,
    before: Option<&'a RankHistory>,
    samples: &'a [RankSample],
    parts: &'a [u128],
}

impl RankWindow<'_> {
    fn scores(&self, id: u32, mut visit: impl FnMut(std::ops::Range<usize>, Score)) {
        let keys = production_keys(self.parts, id);
        let rest = ProducedValueRow::key_of(id, ProducedValueRow::REST);
        let mut cuts = vec![0, self.samples.len()];
        for key in keys.clone().chain([rest]) {
            self.now.cuts(key, self.samples, |s| s.read, &mut cuts);
            if let Some(before) = self.before {
                before.cuts(key, self.samples, |s| s.start, &mut cuts);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for range in cuts.windows(2) {
            let sample = &self.samples[range[0]];
            let now = |key| self.now.cell(key, sample.read, self.now.amount);
            let before = |key| self.before.and_then(|h| h.cell(key, sample.start, h.amount));
            let opening = self.before.map(|_| &before as totals::RankedCells<'_>);
            visit(range[0]..range[1], totals::ranked_total(keys.clone(), Some(rest), &now, opening, &|v| v > 0));
        }
    }

    fn matching(&self, query: &Query<'_>, reply: &EquipmentProducedRaceReply, n: usize) -> usize {
        let mut end = self.samples.len();
        let hour = self.samples[0].hour;
        let countries: BTreeMap<_, _> = query.holders.tags.keys().filter_map(|tag| match query.holders.at(tag, hour)?.holder {
            Holder::Found { id, .. } => Some((tag.as_str(), id)),
            _ => None,
        }).collect();
        if let Some(reason) = reply.missing.as_deref() {
            let mut refused = vec![None; end];
            for &id in countries.values() {
                self.scores(id, |range, score| {
                    if let Err(reason) = score {
                        for first in &mut refused[range] {
                            first.get_or_insert(reason);
                        }
                    }
                });
            }
            return refused.iter().position(|&actual| actual != Some(reason)).unwrap_or(end);
        }

        let mut floor = vec![0; end];
        let mut ranked = BTreeSet::new();
        let mut previous = None;
        for row in &reply.rows {
            let Some(tag) = query.holders.tag_at(&row.country, hour) else { return 0 };
            let Some(&id) = countries.get(tag) else { return 0 };
            if let Country::Tag(_) = row.country
                && let Some(held @ Holder::Found { slot, .. }) = query.holders.at(tag, hour).map(|run| run.holder)
                && query.holders.at(&Country::Slot(slot), hour).is_some_and(|run| run.holder == held) {
                return 0;
            }
            if !ranked.insert(tag) { return 0; }
            self.scores(id, |range, score| {
                let Ok(Some(value)) = score else { end = end.min(range.start); return };
                for (i, bound) in floor.iter_mut().enumerate().take(range.end.min(end)).skip(range.start) {
                    if previous.is_some_and(|ahead| *bound < value || (*bound == value && ahead > tag)) {
                        end = i;
                        break;
                    }
                    *bound = value;
                }
            });
            previous = Some(tag);
        }
        if ranked.len() < n { floor.fill(0); }
        let lowest = floor[..end].iter().copied().min().unwrap_or(0);
        for (&tag, &id) in &countries {
            if end == 0 { break; }
            if ranked.contains(tag) { continue; }
            self.scores(id, |range, score| match score {
                Err(_) => end = end.min(range.start),
                Ok(Some(value)) if value >= lowest => {
                    if let Some(i) = (range.start..range.end.min(end)).find(|&i| value > floor[i] || (value == floor[i] && previous.is_some_and(|last| tag < last))) {
                        end = i;
                    }
                }
                _ => {}
            });
        }
        end
    }
}

fn ranking_until(source: &Source, query: &Query<'_>, params: &EquipmentProducedRaceParams, reply: &EquipmentProducedRaceReply, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<i32> {
    let until = reply.last_hour;
    let Some(stretch) = query.timeline.at(params.hour) else { return Ok(until) };
    let last = stretch.last.min(until.saturating_add(BINS as i32));
    if until >= last { return Ok(until); }
    let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
    let metric = series.measure(produced_measure(params.count, params.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
    let dashboard_declare::Column::Cell(column) = metric.column else { return Ok(until) };
    let amount = dashboard_declare::field(series.table, column)?;
    let baseline = range_baseline(&Line::new(query, series, metric)?, query, params.measure, params.from_hour)?;
    if matches!(baseline, Baseline::Unavailable(_)) { return Ok(stretch.last); }
    let parts = if params.types.is_empty() {
        vec![u128::from(ProducedValueRow::WHOLE)]
    } else {
        let content = Content::read(source, context, &mut Vec::new());
        let types = run_types(source, query, &content, &[stretch.run].into(), &mut BTreeSet::new())?;
        asked_parts(&types, &params.types).remove(&stretch.run).unwrap_or_default()
    };
    let ids: BTreeSet<_> = query.holders.tags.values().flat_map(|runs| {
        let start = runs.partition_point(|run| run.last <= until);
        runs[start..].iter().take_while(|run| run.first <= last).filter_map(|run| match run.holder {
            Holder::Found { id, .. } => Some(id),
            _ => None,
        })
    }).collect();
    let keys: Vec<_> = ids.into_iter()
        .flat_map(|id| production_keys(&parts, id).chain([ProducedValueRow::key_of(id, ProducedValueRow::REST)]))
        .chain([ProducedValueRow::clock()])
        .collect();
    let fixed = back_of(params.measure);
    let back = |read| match baseline.hour() { Some(base) => (base <= read).then_some(base), None => fixed.and_then(|f| f(read)) };
    let windowed = baseline.hour().is_some() || fixed.is_some();
    let mut prior_read = reply.read_hour;
    let mut prove = |until: i32, last: i32| -> io::Result<i32> {
        check_cancel(cancelled)?;
        let first = prior_read.unwrap_or(until).clamp(stretch.first.max(until.saturating_sub(tile::RANK_PROOF_HOURS)), until);
        let starts = (first..=last).filter_map(back).filter(|&h| stretch.first <= h && h <= last);
        let opening = starts.clone().min().zip(starts.max());
        let (from, separate) = match opening {
            Some((lo, hi)) if hi >= first.saturating_sub(1) => (first.min(lo), None),
            _ => (first, opening),
        };
        let now = RankHistory::load(query, stretch.run, &keys, amount, from, last)?;
        let earlier = separate.map(|(lo, hi)| RankHistory::load(query, stretch.run, &keys, amount, lo, hi)).transpose()?;
        let before = opening.map(|_| earlier.as_ref().unwrap_or(&now));
        let sample_at = |hour| {
            let complete = now.complete(hour);
            if !complete && matches!(baseline, Baseline::Opening(_)) { return None; }
            let since = now.clock(hour, now.since);
            if matches!(baseline, Baseline::Opening(open) if Some(open) != since) {
                return Some(Err(Cow::Borrowed(unknown_reason(Gap::Restarted))));
            }
            if let Some(reason) = query.holders.not_ranked(hour) { return Some(Err(Cow::Owned(reason))); }
            let Some(read) = now.clock(hour, now.read).filter(|&r| complete && r >= stretch.first) else { return Some(Err(Cow::Borrowed(totals::TOTALS_NOT_READ))) };
            if read < first || read > hour { return None; }
            if !now.complete(read) { return Some(Err(Cow::Borrowed(totals::TOTALS_NOT_READ))); }
            let start = if windowed {
                let Some(start) = back(read).filter(|&s| s >= stretch.first && since.is_none_or(|since| since <= s)) else {
                    return Some(Err(Cow::Borrowed(totals::TOTALS_WINDOW_UNRECORDED)));
                };
                if let Some(reason) = query.holders.not_ranked(start) { return Some(Err(Cow::Owned(reason))); }
                if !before.is_some_and(|h| h.complete(start) && h.clock(start, h.read) == Some(start)) {
                    return Some(Err(Cow::Borrowed(totals::TOTALS_WINDOW_UNRECORDED)));
                }
                start
            } else { read };
            Some(Ok(RankSample { hour, read, start }))
        };
        let mut proved = until;
        let mut samples: Vec<RankSample> = Vec::new();
        for hour in until + 1..=last {
            let Some(check) = sample_at(hour) else { break };
            match check {
                Err(reason) if reply.missing.as_deref() != Some(reason.as_ref()) => break,
                Err(_) => {}
                Ok(sample) => {
                    if samples.last().is_some_and(|s| s.read > sample.read || s.start > sample.start) { break; }
                    samples.push(sample);
                }
            }
            proved = hour;
        }
        check_cancel(cancelled)?;
        let mut rest = samples.as_slice();
        while let Some(first) = rest.first() {
            let edge = query.holders.interval(first.hour).1;
            let (held, later) = rest.split_at(rest.partition_point(|s| s.hour <= edge));
            let window = RankWindow { now: &now, before, samples: held, parts: &parts };
            let count = window.matching(query, reply, params.top.unwrap_or(u32::MAX) as usize);
            if let Some(changed) = held.get(count) { return Ok(changed.hour - 1); }
            rest = later;
        }
        prior_read = now.clock(proved, now.read);
        Ok(proved)
    };
    let mut proved = until;
    let mut step = 1;
    while proved < last {
        let end = proved.saturating_add(step).min(last);
        match prove(proved, end) {
            Ok(through) if through == end => proved = end,
            Ok(through) => return Ok(through),
            Err(error) if replay_recording::columnar_frames::Damage::of(&error).is_some() => return Ok(proved),
            Err(error) => return Err(error),
        }
        step *= 4;
    }
    Ok(proved)
}

pub fn tile(source: &Source, query: &Query<'_>, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<SeriesTileReply> {
    let family: EquipmentProducedTileFamily = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null))
        .map_err(|error| invalid(format!("Invalid equipment produced tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Unknown equipment produced tile family {}", family.kind)));
    }
    let ask: TileAsk = dashboard_query::named::named(params)
        .map_err(|error| invalid(format!("Invalid equipment produced tile request: {error}")))?;
    tile::level_of(ask.bin_hours)?;
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let branch_id = query.timeline.id.to_string();

    let (keys, ranking) = match ask.top {
        None => {
            ask.countries.iter().try_for_each(validate_country).map_err(invalid)?;
            let mut seen = BTreeSet::new();
            let deduped = ask.countries.into_iter().filter(|country| seen.insert(country.clone())).collect();
            (deduped, None)
        }
        Some(n) => {
            if !ask.countries.is_empty() {
                return Err(invalid("A top of equipment produced tiles names no country: the ranking names them"));
            }
            let n = u32::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one country or more"))?;
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let at_params = EquipmentProducedRaceParams {
                branch_id: branch_id.clone(),
                hour,
                countries: Vec::new(),
                group: None,
                top: Some(n),
                count: family.count,
                measure: family.measure,
                types: family.types.clone(),
                refits: family.refits,
                from_hour: family.from_hour,
            };
            let at_reply = race_reply(source, query, &at_params, context, cancelled)?;
            let from = at_reply.first_hour.min(hour);
            let until = ranking_until(source, query, &at_params, &at_reply, context, cancelled)?;
            check_cancel(cancelled)?;
            if let Some(reason) = at_reply.missing {
                (Vec::new(), Some(TileRanking { hour, from, until, refused: Some(reason) }))
            } else {
                let ranked_keys: Vec<Country> = at_reply.rows.into_iter().map(|row| row.country).collect();
                (ranked_keys, Some(TileRanking { hour, from, until, refused: None }))
            }
        }
    };

    let within_by_country: BTreeMap<Country, Vec<HourSpan>> = ask.within.unwrap_or_default().into_iter().collect();
    let scan_first = first.max(query.timeline.first);
    let clip = last.min(query.timeline.last);
    let dec = decimals(family.count);
    let scale = 10_f64.powi(i32::from(dec));

    let mut drawn_lines: Vec<Vec<DrawnPiece>> = Vec::with_capacity(keys.len());
    let mut tile_lines: Vec<TileLine> = Vec::with_capacity(keys.len());

    if scan_first <= clip && !keys.is_empty() {
        let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
        let measure_decl = series.measure(produced_measure(family.count, family.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
        let line = Line::new(query, series, measure_decl)?;
        let baseline = range_baseline(&line, query, family.measure, family.from_hour)?;
        let unavailable = match baseline {
            Baseline::Unavailable(reason) => Some(reason),
            _ => None,
        };
        let fixed_back = back_of(family.measure);
        let range_fn = |base: i32| move |read: i32| (read >= base).then_some(base);
        let range_closure = baseline.hour().map(range_fn);
        let back: Option<&dyn Fn(i32) -> Option<i32>> = match (&range_closure, &fixed_back) {
            (Some(func), _) => Some(func as &dyn Fn(i32) -> Option<i32>),
            (None, Some(func)) => Some(func as &dyn Fn(i32) -> Option<i32>),
            (None, None) => None,
        };
        let met: BTreeSet<u32> = query.timeline.within(scan_first, clip).map(|stretch| stretch.run).collect();
        let mut warnings = Vec::new();
        let mut notes = BTreeSet::new();
        let types: BTreeMap<u32, BTreeMap<u32, Typed>> = match family.types.is_empty() {
            true => met.iter().map(|&run| (run, BTreeMap::new())).collect(),
            false => {
                let content = Content::read(source, context, &mut warnings);
                run_types(source, query, &content, &met, &mut notes)?
            }
        };
        let parts = asked_parts(&types, &family.types);
        let parts_of = |run: u32| parts.get(&run).cloned().unwrap_or_default();
        check_cancel(cancelled)?;
        let total_runs = match unavailable {
            Some(_) => vec![Vec::new(); keys.len()],
            None => line.totals(&keys, &parts_of, back, scan_first, clip)?,
        };
        let restarted: Vec<(i32, i32)> = match baseline {
            Baseline::Opening(start) => line.since_runs(scan_first, clip)?.into_iter().filter(|run| run.2 != Some(start)).map(|run| (run.0, run.1)).collect(),
            _ => Vec::new(),
        };
        for (country, runs_for_country) in keys.into_iter().zip(total_runs) {
            let within = within_by_country.get(&country).map(Vec::as_slice);
            let mut pieces = Vec::new();
            let mut missing = Vec::new();
            if let Some(reason) = unavailable {
                push_missing(&mut missing, scan_first, clip, reason);
            }
            for &(lo, hi) in &restarted {
                push_missing(&mut missing, lo, hi, unknown_reason(Gap::Restarted));
            }
            for run in runs_for_country {
                let mut kept = vec![(run.first, run.last)];
                for &(lo, hi) in &restarted {
                    kept = kept
                        .into_iter()
                        .flat_map(|(a, b)| match b < lo || a > hi {
                            true => vec![(a, b)],
                            false => [(a, lo - 1), (hi + 1, b)].into_iter().filter(|(x, y)| x <= y).collect(),
                        })
                        .collect();
                }
                for (first, last) in kept {
                    match run.value {
                        Ok(raw) if i128::from(raw).abs() <= MAX_EXACT => pieces.push(DrawnPiece {
                            first,
                            last,
                            value: raw as f64 / scale,
                            break_before: run.opens || first != run.first,
                        }),
                        Ok(_) => push_missing(&mut missing, first, last, unknown_reason(Gap::OutOfRange)),
                        Err(gap) => push_missing(&mut missing, first, last, unknown_reason(gap)),
                    }
                }
            }
            missing.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
            let pieces = clip_pieces(pieces, within);
            let missing = clip_missing(&missing, within);
            drawn_lines.push(pieces);
            tile_lines.push(TileLine { country, part: None, missing, parts: Vec::new(), marks: None });
        }
    } else {
        for country in keys {
            drawn_lines.push(Vec::new());
            tile_lines.push(TileLine { country, part: None, missing: Vec::new(), parts: Vec::new(), marks: None });
        }
    }

    let (bins, raw_values, hour_bytes) = match ask.bin_hours {
        1 => {
            let run_lines: Vec<Vec<ChartRun>> = drawn_lines.iter().map(|lines| runs(lines, first, clip)).collect();
            let (bytes, raw) = tile::encode_runs(&run_lines, first, scale);
            (bytes, raw, 0)
        }
        bin_hours => {
            let buckets: Vec<Option<ChartBucket>> = drawn_lines.iter().flat_map(|lines| bucketed(lines, first, clip, bin_hours)).collect();
            tile::encode(&buckets, first, bin_hours, scale)
        }
    };
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    Ok(SeriesTileReply {
        branch_id,
        measure: produced_measure(family.count, family.refits).to_owned(),
        decimals: dec,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::equipment_produced::{EquipmentProducedRaceReply, tests::{two_producers, two_producers_read_from}};

    #[derive(Default)]
    struct RaceEvaluations(usize);

    pub(super) fn counted_race(source: &Source, query: &Query<'_>, params: &EquipmentProducedRaceParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<EquipmentProducedRaceReply> {
        source.state::<RaceEvaluations>().lock().unwrap().0 += 1;
        super::super::race_reply(source, query, params, load, cancelled)
    }

    fn top_one(source: &Source, hour: i32, from: Option<i32>) -> SeriesTileReply {
        tile(
            source, &dashboard_base::queries::query(source, 1).unwrap(),
            &serde_json::json!({
                "branchId": "1", "family": { "kind": SERIES, "count": "units", "measure": "total", "refits": false, "fromHour": from },
                "binHours": 1, "index": i64::from(hour).div_euclid(BINS as i64), "top": 1, "hour": hour
            }),
            &LoadContext { content: None }, &AtomicBool::new(false),
        ).unwrap()
    }

    #[test]
    fn held_equipment_ranking_ends_when_the_runner_up_passes_the_leader() {
        let first = 1_000;
        let source = two_producers("rank-crossing", first, first + 240);
        let held = top_one(&source, first + 175, None);
        assert_eq!(held.lines[0].country, Country::Slot(7));
        assert_eq!(held.ranking.unwrap().until, first + 207, "the runner-up gains at 184 and ties at 196, then leads at 208");
        assert_eq!(top_one(&source, first + 208, None).lines[0].country, Country::Slot(8));
    }

    #[test]
    fn held_equipment_ranking_ends_when_the_leaders_range_total_becomes_unknown() {
        let first = 1_000;
        let source = two_producers_read_from("rank-restart", first, first + 165, first, true, None, Some(first + 150));
        let held = top_one(&source, first + 120, Some(first));
        assert_eq!(held.lines[0].country, Country::Slot(7));
        assert_eq!(held.ranking.unwrap().until, first + 149);
        let refused = top_one(&source, first + 150, Some(first));
        assert!(refused.lines.is_empty());
        assert_eq!(refused.ranking.unwrap().refused.as_deref(), Some(unknown_reason(Gap::Restarted)));
    }

    #[test]
    fn held_equipment_ranking_matches_the_race_across_a_rename_and_window_openings() {
        let first = 1_000;
        let last = first + 220;
        let src = two_producers_read_from("rank-windows", first, last, first, true, Some(first + 184), None);
        assert_eq!(top_one(&src, first + 175, None).ranking.unwrap().until, first + 207, "the renamed leader keeps its slot through the tie");
        let mut query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);
        for (measure, from_hour) in [(ProducedMeasure::Total, None), (ProducedMeasure::Total, Some(first + 165)), (ProducedMeasure::Day, None)] {
            let mut params = EquipmentProducedRaceParams {
                branch_id: "1".into(), hour: first + 50, countries: vec![], group: None, top: Some(2),
                count: ProducedCount::Units, measure, types: vec![], refits: false, from_hour,
            };
            let mut until = i32::MIN;
            let mut expected = (Vec::new(), None);
            while params.hour <= last {
                let race = super::super::race_reply(&src, &query, &params, &load, &cancelled).unwrap();
                if params.hour > until {
                    until = ranking_until(&src, &query, &params, &race, &load, &cancelled).unwrap();
                    assert!(until >= params.hour);
                    expected = (race.rows.iter().map(|r| r.country.clone()).collect(), race.missing.clone());
                }
                assert_eq!(expected, (race.rows.into_iter().map(|r| r.country).collect(), race.missing), "rank at {} for {measure:?}", params.hour);
                params.hour += 1;
            }
        }
        let mut tags = query.holders.tags.clone();
        tags.get_mut("ITA").unwrap()[0].first = first + 210;
        query.holders = dashboard_query::identity::Holders::new(tags, Vec::new()).into();
        let mut params = EquipmentProducedRaceParams {
            branch_id: "1".into(), hour: first + 190, countries: vec![], group: None, top: Some(1),
            count: ProducedCount::Units, measure: ProducedMeasure::Total, types: vec![], refits: false, from_hour: None,
        };
        let held = super::super::race_reply(&src, &query, &params, &load, &cancelled).unwrap();
        assert_eq!(held.rows[0].country, Country::Slot(7));
        assert_eq!(ranking_until(&src, &query, &params, &held, &load, &cancelled).unwrap(), first + 209);
        params.hour = first + 210;
        assert_eq!(super::super::race_reply(&src, &query, &params, &load, &cancelled).unwrap().rows[0].country, Country::Slot(8));
    }

    #[test]
    fn held_equipment_ranking_counts_full_races_over_100_played_hours() {
        use dashboard_follow::panel::{AtHour, Declared, ProductRead};
        use dashboard_base::test_fixtures;
        use replay_data::rows::{CountryLinkRow, CountryRow, ProductionLineOutputRow, Recorded, RocketProductionOutputRow, TableId, TableRows};
        use replay_frame::frame::TableView;
        use std::sync::{Arc, Mutex};

        let first = 1_000;
        let frames = (first..=first + 104).map(|hour| {
            let mut frame = test_fixtures::source(1, hour, Some(1));
            let TableView::Complete(TableRows::Countries(held)) = &*frame.tables[TableId::Countries.index()] else { unreachable!() };
            let other = CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..held[0].clone() };
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![held[0].clone(), other]));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 },
            ]));
            let output = ProductionLineOutputRow {
                line: 1, country: 9001, variant: 100, units: 9, since_hour: first, observed_since_creation: true,
                exact: hour < first + 100, spent_raw: Recorded::Value(Some(i64::from(hour))), purpose: Recorded::Value(1), ..Default::default()
            };
            let runner_up = ProductionLineOutputRow { line: 2, country: 9002, units: 3, exact: true, ..output };
            let rockets = RocketProductionOutputRow { since_hour: first, exact: true, units_exact: true, ..Default::default() };
            *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(vec![output, runner_up]));
            *frame.tables[TableId::RocketProductionOutput.index()] = TableView::Complete(TableRows::RocketProductionOutput(vec![rockets]));
            frame.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
            frame
        }).collect();
        let replay = test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
        let source = Arc::new(Source::from_replay(dashboard_base::SourceKey { source_id: "rank-played".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
        let chart = Arc::clone(&source);
        let until = Mutex::new(i32::MIN);
        let panel = AtHour {
            cold: Arc::new(|_, _| unreachable!()), read: Arc::new(|_, _| unreachable!()),
            advance: Arc::new(move |hour, _, _, _| {
                let mut until = until.lock().unwrap();
                let rows = if hour > *until {
                    let reply = top_one(&chart, hour, None);
                    if hour < first + 100 { assert_eq!(reply.lines[0].country, Country::Slot(7)); }
                    *until = reply.ranking.unwrap().until;
                    reply.lines.len() as u64
                } else { 0 };
                ProductRead {
                    value: Arc::new(Ok(chart.state::<RaceEvaluations>().lock().unwrap().0.into())), rows, fresh: rows > 0, pending: false, live: None,
                    lanes: vec![Declared { table: TableId::ProducedValues.index(), columns: vec![], keyed: false, passing: false, hourly: true }],
                }
            }),
        };
        let played = test_fixtures::played(&source.replay(), &panel, first, first + 99);
        let hours = played.iter().map(|step| step.hour).collect::<BTreeSet<_>>();
        assert_eq!(hours.len(), 100);
        let count = played.last().unwrap().value.as_ref().as_ref().unwrap().as_u64();
        assert_eq!(count, Some(1), "full races for 100 played hours with the same leader");
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let mut previous_until = i32::MIN;
        let mut previous_count = 0;
        for hour in hours {
            if hour > previous_until {
                let params = EquipmentProducedRaceParams {
                    branch_id: "1".into(), hour, countries: vec![], group: None, top: Some(1), count: ProducedCount::Units,
                    measure: ProducedMeasure::Total, types: vec![], refits: false, from_hour: None,
                };
                previous_until = super::super::race_reply(&source, &query, &params, &LoadContext { content: None }, &false.into()).unwrap().last_hour;
                previous_count += 1;
            }
        }
        assert_eq!(previous_count, 100, "the full race's expiry with hourly production reads");
        assert_eq!(top_one(&source, first, None).ranking.unwrap().until, first + 99);
        let missing = top_one(&source, first + 100, None).ranking.unwrap();
        assert_eq!(missing.refused.as_deref(), Some(totals::TOTALS_VALUE_UNKNOWN));
        assert_eq!(missing.until, first + 104);
    }

    #[test]
    fn equipment_produced_tiles_cover_total_day_and_range_from_hour() {
        const FIRST: i32 = 1_000;
        const LAST: i32 = 1_300;
        let src = two_producers("produced-tiles", FIRST, LAST);
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);

        let top_reply = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "count": "units",
                    "measure": "total",
                    "types": [],
                    "refits": false
                },
                "binHours": 1,
                "index": i64::from(LAST).div_euclid(BINS as i64),
                "top": 5,
                "hour": LAST
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(top_reply.ranking.as_ref().and_then(|r| r.refused.as_deref()), None);
        assert_eq!(top_reply.lines.len(), 2);
        assert!(!top_reply.bins.is_empty());

        let range_start = FIRST + 120;
        let full_at = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let range_at = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: Some(range_start),
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let full_ger = full_at.rows[0].value.unwrap();
        let range_ger = range_at.rows[0].value.unwrap();
        assert!(range_ger > 0 && range_ger < full_ger, "range total {range_ger} must exclude pre-range production from full total {full_ger}");
        assert_eq!(range_at.observation_hour, Some(range_start), "a Range total is counted from the Range's start");
        let one_hour = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams { hour: 1120, from_hour: Some(1120), ..EquipmentProducedRaceParams { branch_id: "1".into(), hour: LAST, countries: vec![Country::Tag("GER".into())], group: None, top: None, count: ProducedCount::Units, measure: ProducedMeasure::Total, types: Vec::new(), refits: false, from_hour: None } },
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(one_hour.rows[0].value, Some(1), "a Range of the one hour GER delivers in counts that delivery");
        let detail = |from_hour| {
            crate::products::equipment_produced::at_reply(&src, &query, &crate::products::equipment_produced::ProducedAtParams { branch_id: "1".into(), country: Country::Tag("GER".into()), hour: LAST, measure: ProducedMeasure::Total, refits: false, from_hour }, &load).unwrap()
        };
        let ranged = detail(Some(range_start));
        assert_eq!(ranged.observation, Some(range_start), "the detail counts from the same opening as the race");
        let units: i64 = ranged.models.iter().filter_map(|model| model.units.value).sum();
        assert_eq!(units, range_ger, "the detail's models add up to the race's Range total");
        assert_eq!(detail(Some(FIRST)).observation, detail(None).observation, "a Range from the stretch's first hour opens nowhere, in the detail as in the race");

        let range_reply = |hour: i32, from_hour: i32| race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: Some(from_hour),
            },
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(range_reply(range_start, range_start).rows[0].value, Some(1), "the unit delivered at the range's first hour is in the range");
        let before = range_reply(LAST, FIRST - 50);
        assert_eq!(before.missing.as_deref(), Some(RANGE_BASELINE_UNRECORDED), "a range starting where the count is not recorded says so, never the whole total");
        assert!(before.rows.is_empty());
        assert_eq!(range_reply(LAST, FIRST).rows[0].value, full_at.rows[0].value, "a range from the hour observation began is the whole observed count");
        let late = crate::products::equipment_produced::tests::two_producers_read_from("produced-tiles-late", FIRST, LAST, FIRST + 5, true, None, None);
        let late_query = dashboard_base::queries::query(&late, 1).unwrap();
        let from_first = race_reply(
            &late,
            &late_query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: Some(FIRST),
            },
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(from_first.missing.as_deref(), Some(RANGE_BASELINE_UNRECORDED), "production first read after the branch began has no proven opening balance at its first hour: {:?}", from_first.rows);
        let ranged_top = |src: &Source, hour: i32, from_hour: i32| {
            let query = dashboard_base::queries::query(src, 1).unwrap();
            race_reply(
                src,
                &query,
                &EquipmentProducedRaceParams {
                    branch_id: "1".into(),
                    hour,
                    countries: Vec::new(),
                    group: None,
                    top: Some(2),
                    count: ProducedCount::Units,
                    measure: ProducedMeasure::Total,
                    types: Vec::new(),
                    refits: false,
                    from_hour: Some(from_hour),
                },
                &load,
                &cancelled,
            )
            .unwrap()
        };
        let renamed = crate::products::equipment_produced::tests::two_producers_read_from("produced-tiles-renamed", FIRST, LAST, FIRST, true, Some(FIRST + 200), None);
        let leader = ranged_top(&renamed, LAST, FIRST + 120);
        assert_eq!(leader.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), ["ITA", "DEU"], "a country renamed inside the window stays a contender, read by its own rows: {leader:?}");
        assert_eq!(leader.rows[1].value, Some(8), "and its amount is the country's deliveries since the start, read by its slot: {leader:?}");
        assert_eq!(leader.rows[1].counted, leader.rows[1].country, "a ranked row was counted for its slot, and its detail asks for that slot");
        let renamed_query = dashboard_base::queries::query(&renamed, 1).unwrap();
        let asked = race_reply(&renamed, &renamed_query, &EquipmentProducedRaceParams { branch_id: "1".into(), hour: LAST, countries: vec![Country::Tag("DEU".into())], group: None, top: None, count: ProducedCount::Units, measure: ProducedMeasure::Total, types: Vec::new(), refits: false, from_hour: Some(FIRST + 120) }, &load, &cancelled).unwrap();
        assert_eq!(asked.rows[0].counted, Country::Tag("DEU".into()), "a row asked for by its tag was counted for that tag, and its detail asks for the same history: {asked:?}");
        let restarted = crate::products::equipment_produced::tests::two_producers_read_from("produced-tiles-restarted", FIRST, LAST, FIRST, true, None, Some(FIRST + 150));
        let after = ranged_top(&restarted, LAST, FIRST);
        assert_eq!(after.missing.as_deref(), Some(unknown_reason(Gap::Restarted)), "a range from the first hour is not known after counting restarted: {:?}", after.rows);
        assert!(ranged_top(&restarted, FIRST + 100, FIRST).missing.is_none(), "and is known before");
        let restarted_query = dashboard_base::queries::query(&restarted, 1).unwrap();
        let restarted_detail = crate::products::equipment_produced::at_reply(&restarted, &restarted_query, &crate::products::equipment_produced::ProducedAtParams { branch_id: "1".into(), country: Country::Slot(7), hour: LAST, measure: ProducedMeasure::Total, refits: false, from_hour: Some(FIRST) }, &load).unwrap();
        let restarted_reason = Some(crate::products::equipment_produced::ProducedUnknown::ObservationRestarted);
        assert!(
            !restarted_detail.models.is_empty() && restarted_detail.models.iter().all(|model| model.units.reason == restarted_reason && model.ic.reason == restarted_reason),
            "the detail of a Range from the opening is not known after counting restarted, as the race says: {:?}",
            restarted_detail.models
        );
        assert_eq!(restarted_detail.ic_total.reason, Some(crate::products::equipment_produced::ProducedUnknown::ObservationRestarted));
        let restart_tile = tile(
            &restarted,
            &restarted_query,
            &serde_json::json!({ "branchId": "1", "family": { "kind": SERIES, "count": "units", "measure": "total", "types": [], "refits": false, "fromHour": FIRST }, "binHours": 1, "index": i64::from(FIRST + 150).div_euclid(BINS as i64), "countries": [7] }),
            &load,
            &cancelled,
        )
        .unwrap();
        let bytes = {
            const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = Vec::new();
            for chunk in restart_tile.bins.as_bytes().chunks_exact(4) {
                let word = chunk.iter().fold(0u32, |word, &byte| (word << 6) | ALPHABET.iter().position(|&c| c == byte).map_or(0, |at| at as u32));
                out.extend([(word >> 16) as u8, (word >> 8) as u8, word as u8]);
            }
            out.truncate(out.len() - restart_tile.bins.bytes().rev().take_while(|&byte| byte == b'=').count());
            out
        };
        let drawn = tile::decode_runs(&bytes, 1, restart_tile.first_hour, 1.0, restart_tile.raw_values).unwrap().remove(0);
        assert!(drawn.iter().all(|run| run.1 < FIRST + 150), "nothing is drawn after counting restarted, even inside a run that began before: {drawn:?}");
        assert!(drawn.iter().any(|run| run.0 <= FIRST + 149 && FIRST + 149 <= run.1), "and the hour before is drawn: {drawn:?}");
        let newcomer = ranged_top(&src, LAST, FIRST + 50);
        assert_eq!(newcomer.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), ["ITA", "GER"], "a country with no deliveries recorded at the start has delivered nothing by then: {newcomer:?}");
        let shared = two_producers("produced-tiles-shared", FIRST, LAST);
        let mut shared_query = dashboard_base::queries::query(&shared, 1).unwrap();
        let mut tags = shared_query.holders.tags.clone();
        tags.insert("PRU".to_owned(), vec![dashboard_query::identity::HolderRun { first: FIRST, last: LAST, holder: dashboard_query::identity::Holder::Found { slot: 7, id: 11 }, starts_segment: true }]);
        shared_query.holders = std::sync::Arc::new(dashboard_query::identity::Holders::new(tags, shared_query.holders.unknown.clone()));
        let shared_top = race_reply(
            &shared,
            &shared_query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: Vec::new(),
                group: None,
                top: Some(5),
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let ger = shared_top.rows.iter().find(|row| row.tag == "GER").unwrap_or_else(|| panic!("GER is ranked: {shared_top:?}"));
        assert_eq!(ger.country, Country::Tag("GER".into()), "a ranked country sharing its slot with another is named by what was ranked, never by the slot: {shared_top:?}");
        let unsure = crate::products::equipment_produced::tests::two_producers_read_from("produced-tiles-unsure", FIRST, LAST, FIRST, false, None, None);
        let unsure_query = dashboard_base::queries::query(&unsure, 1).unwrap();
        let unsure_top = race_reply(
            &unsure,
            &unsure_query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: Vec::new(),
                group: None,
                top: Some(5),
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(unsure_top.missing.as_deref(), Some(dashboard_query::series::totals::TOTALS_VALUE_UNKNOWN), "a contender whose amount is unknown leaves the top unproved: {:?}", unsure_top.rows);
        let week = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: FIRST + 10,
                countries: Vec::new(),
                group: None,
                top: Some(5),
                count: ProducedCount::Units,
                measure: ProducedMeasure::Week,
                types: Vec::new(),
                refits: false,
                from_hour: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        assert!(week.rows.is_empty() && week.missing.as_deref().is_some_and(|reason| reason.contains("window")), "a week not yet whole is not known, never 'nothing delivered': {:?}", week.missing);

        let range_tile = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "count": "units",
                    "measure": "total",
                    "types": [],
                    "refits": false,
                    "fromHour": range_start
                },
                "binHours": 1,
                "index": i64::from(LAST).div_euclid(BINS as i64),
                "countries": [7]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(range_tile.lines.len(), 1);
        assert!(!range_tile.bins.is_empty());
    }
}
