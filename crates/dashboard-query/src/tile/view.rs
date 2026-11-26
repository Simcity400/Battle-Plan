
use super::{ALPHABET, BINS, FIRST_BREAK, SEAM, SeriesTileReply, TileLine, TilePart, decode, decode_runs};
use crate::{
    chart::{ChartBucket, ChartRun, ChartSummary, MarkCluster, MarkSummary},
    identity::Country,
    series::{MissingSpan, grid},
};
use serde::Serialize;
use serde_json::Value;
use std::{io, ops::RangeInclusive};

pub const GROUP: usize = 64;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub from: i32,
    pub to: i32,
    pub width: u32,
}

impl View {
    pub fn of(chart: &Value) -> io::Result<Self> {
        let hour = |name: &str| -> io::Result<i32> {
            chart
                .get(name)
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= f64::from(i32::MIN) && *value <= f64::from(i32::MAX))
                .map(|value| value.floor() as i32)
                .ok_or_else(|| invalid(format!("The chart request names no {name}")))
        };
        let (from, to) = (hour("from")?, hour("to")?);
        let width = chart
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|width| u32::try_from(width).ok())
            .filter(|&width| width > 0)
            .ok_or_else(|| invalid("The chart request names no width"))?;
        if to < from {
            return Err(invalid("The chart's view ends before it starts"));
        }
        Ok(Self { from, to, width })
    }

    pub fn bin_hours(&self) -> i64 {
        grid(i64::from(self.to) - i64::from(self.from) + 1, self.width).1
    }

    pub fn tiles(&self) -> RangeInclusive<i64> {
        let span = BINS as i64 * self.bin_hours();
        i64::from(self.from).div_euclid(span)..=i64::from(self.to).div_euclid(span)
    }
}

pub fn requests(chart: &Value) -> io::Result<(View, Vec<Value>)> {
    let view = View::of(chart)?;
    let bin_hours = view.bin_hours();
    let mut base = chart.as_object().cloned().ok_or_else(|| invalid("A chart request is an object"))?;
    for name in ["from", "to", "width", "countries", "within"] {
        base.remove(name);
    }
    let keys = match chart.get("countries") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(keys)) => keys.clone(),
        Some(_) => return Err(invalid("A chart request's countries are a list")),
    };
    let within = match chart.get("within") {
        None | Some(Value::Null) => None,
        Some(Value::Array(within)) => Some(within),
        Some(_) => return Err(invalid("A chart request's within pairs keys with spans")),
    };
    let span = BINS as i64 * bin_hours;
    let mut out = Vec::new();
    for index in view.tiles() {
        let (read_from, last) = (index * span - bin_hours, (index + 1) * span - 1);
        for group in keys.chunks(GROUP) {
            let mut request = base.clone();
            request.insert("binHours".into(), bin_hours.into());
            request.insert("index".into(), index.into());
            request.insert("countries".into(), Value::Array(group.to_vec()));
            if let Some(within) = within {
                let mut cut = Vec::new();
                for key in group {
                    let spans = within.iter().find(|pair| pair.get(0) == Some(key)).and_then(|pair| pair.get(1));
                    cut.push(Value::Array(vec![key.clone(), Value::Array(spans_within(spans, read_from, last)?)]));
                }
                request.insert("within".into(), Value::Array(cut));
            }
            out.push(Value::Object(request));
        }
    }
    Ok((view, out))
}

fn spans_within(spans: Option<&Value>, from: i64, to: i64) -> io::Result<Vec<Value>> {
    let Some(spans) = spans else { return Ok(Vec::new()) };
    let spans = spans.as_array().ok_or_else(|| invalid("A key's within spans are a list"))?;
    let mut out = Vec::new();
    for span in spans {
        let hour = |name: &str| span.get(name).and_then(Value::as_i64).ok_or_else(|| invalid(format!("A within span names no {name}")));
        let (first, last) = (hour("firstHour")?.max(from), hour("lastHour")?.min(to));
        if first <= last {
            out.push(serde_json::json!({ "firstHour": first, "lastHour": last }));
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewLine {
    pub country: Country,
    pub summary: ChartSummary,
    pub missing: Vec<MissingSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial: Vec<MissingSpan>,
    pub parts: Vec<TilePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<MarkSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssembledView {
    pub measure: String,
    pub decimals: u8,
    pub bin_hours: i64,
    pub series: Vec<ViewLine>,
}

enum Bins {
    Buckets(Vec<Option<ChartBucket>>),
    Runs(Vec<(i32, i32, f64, u8)>),
}

pub fn assemble(view: View, replies: &[SeriesTileReply]) -> io::Result<AssembledView> {
    let bin_hours = view.bin_hours();
    let tiles = view.tiles();
    let first_tile = *tiles.start();
    let count = (tiles.end() - first_tile + 1) as usize;
    let mut by_tile: Vec<Vec<(&TileLine, Bins)>> = (0..count).map(|_| Vec::new()).collect();
    let (mut measure, mut decimals) = (String::new(), 0);
    for reply in replies {
        if reply.bin_hours != bin_hours || !tiles.contains(&reply.index) {
            return Err(invalid(format!("A tile of {} hour bins at {} is not one of the view's", reply.bin_hours, reply.index)));
        }
        if i64::from(reply.first_hour) != reply.index * BINS as i64 * bin_hours {
            return Err(invalid("A tile's first hour is not its index's"));
        }
        (measure, decimals) = (reply.measure.clone(), reply.decimals);
        let bytes = unbase64(&reply.bins)?;
        let keys = reply.lines.len();
        let scale = 10_f64.powi(i32::from(reply.decimals));
        let held = &mut by_tile[(reply.index - first_tile) as usize];
        match reply.hour_bytes {
            0 => {
                for (line, runs) in reply.lines.iter().zip(decode_runs(&bytes, keys, reply.first_hour, scale, reply.raw_values)?) {
                    held.push((line, Bins::Runs(runs)));
                }
            }
            hour_bytes => {
                let buckets = decode(&bytes, keys, reply.first_hour, bin_hours, scale, reply.raw_values, hour_bytes)?;
                for (line, bins) in reply.lines.iter().zip(buckets.chunks(BINS)) {
                    held.push((line, Bins::Buckets(bins.to_vec())));
                }
            }
        }
    }
    if replies.is_empty() {
        return Ok(AssembledView { measure, decimals, bin_hours, series: Vec::new() });
    }
    let tags: Vec<&Country> = by_tile[0].iter().map(|(line, _)| &line.country).collect();
    for (at, held) in by_tile.iter().enumerate() {
        if held.len() != tags.len() || held.iter().zip(&tags).any(|((line, _), country)| line.country != **country) {
            return Err(invalid(format!("Tile {} holds other keys than the view's first", first_tile + at as i64)));
        }
    }
    let series = (0..tags.len())
        .map(|key| {
            let own: Vec<&(&TileLine, Bins)> = by_tile.iter().map(|held| &held[key]).collect();
            line(view, bin_hours, first_tile, &own)
        })
        .collect::<io::Result<_>>()?;
    Ok(AssembledView { measure, decimals, bin_hours, series })
}

fn line(view: View, bin_hours: i64, first_tile: i64, tiles: &[&(&TileLine, Bins)]) -> io::Result<ViewLine> {
    let (from, to) = (i64::from(view.from), i64::from(view.to));
    let mut parts: Vec<TilePart> = Vec::new();
    let mut missing: Vec<MissingSpan> = Vec::new();
    let mut partial: Vec<MissingSpan> = Vec::new();
    let push_span = |out: &mut Vec<MissingSpan>, span: &MissingSpan| {
        let (first, last) = (i64::from(span.first_hour).max(from), i64::from(span.last_hour).min(to));
        if first > last {
            return;
        }
        match out.last_mut() {
            Some(held) if held.reason == span.reason && i64::from(held.last_hour) + 1 == first => held.last_hour = last as i32,
            _ => out.push(MissingSpan { first_hour: first as i32, last_hour: last as i32, reason: span.reason.clone() }),
        }
    };
    for (line, _) in tiles {
        for part in &line.parts {
            if i64::from(part.last_hour) >= from && i64::from(part.first_hour) <= to && !parts.contains(part) {
                parts.push(part.clone());
            }
        }
        for span in &line.missing {
            push_span(&mut missing, span);
        }
        for span in &line.partial {
            push_span(&mut partial, span);
        }
    }
    let summary = match &tiles[0].1 {
        Bins::Runs(_) => {
            let mut runs: Vec<ChartRun> = Vec::new();
            for (_, bins) in tiles {
                let Bins::Runs(held) = bins else { return Err(invalid("A view's tiles mix hours and bins")) };
                for &(first, last, value, flags) in held {
                    match runs.last_mut() {
                        Some(run) if flags & SEAM != 0 && run.last_hour + 1 == first => run.last_hour = last,
                        _ => runs.push(ChartRun { first_hour: first, last_hour: last, value, break_before: flags & FIRST_BREAK != 0 }),
                    }
                }
            }
            let segment = parts.iter().find(|part| part.first_hour <= view.from && view.from <= part.last_hour).map(|part| part.segment_first_hour);
            let runs: Vec<ChartRun> = runs
                .into_iter()
                .filter(|run| run.last_hour >= view.from && run.first_hour <= view.to)
                .map(|run| match run.first_hour <= view.from {
                    true => ChartRun { first_hour: view.from, last_hour: run.last_hour.min(view.to), break_before: segment == Some(view.from), ..run },
                    false => ChartRun { last_hour: run.last_hour.min(view.to), ..run },
                })
                .collect();
            let mut buckets = vec![None; (to - from + 1) as usize];
            for run in &runs {
                for hour in run.first_hour..=run.last_hour {
                    buckets[(i64::from(hour) - from) as usize] = Some(ChartBucket {
                        absolute_total: 0,
                        first_hour: hour,
                        last_hour: hour,
                        first_value: run.value,
                        last_value: run.value,
                        last_value_hour: hour,
                        min_hour: hour,
                        min_value: run.value,
                        max_hour: hour,
                        max_value: run.value,
                        first_break: hour == run.first_hour && run.break_before,
                        has_break: false,
                    });
                }
            }
            ChartSummary { from: f64::from(view.from), to: f64::from(view.to), width: view.width, buckets, runs: Some(runs) }
        }
        Bins::Buckets(_) => {
            let mut all: Vec<Option<ChartBucket>> = Vec::with_capacity(tiles.len() * BINS);
            for (_, bins) in tiles {
                let Bins::Buckets(held) = bins else { return Err(invalid("A view's tiles mix hours and bins")) };
                all.extend_from_slice(held);
            }
            let origin = first_tile * BINS as i64;
            let (first, last) = ((from.div_euclid(bin_hours) - origin) as usize, (to.div_euclid(bin_hours) - origin) as usize);
            let mut buckets = all[first..=last].to_vec();
            if let Some(Some(bucket)) = buckets.first_mut() {
                bucket.first_break = true;
            }
            ChartSummary { from: f64::from(view.from), to: f64::from(view.to), width: view.width, buckets, runs: None }
        }
    };
    let marks = match tiles.iter().all(|(line, _)| line.marks.is_some()) {
        true => {
            let first_bin = from.div_euclid(bin_hours) * bin_hours;
            let mut clusters = Vec::new();
            for (at, (line, _)) in tiles.iter().enumerate() {
                let marks = line.marks.as_ref().expect("checked");
                let mut ordinal: u64 = marks.before.parse().map_err(|_| invalid("A tile's drops before it are not a count"))?;
                let tile_first = (first_tile + at as i64) * BINS as i64 * bin_hours;
                for &(bin, count) in &marks.bins {
                    let start = tile_first + i64::from(bin) * bin_hours;
                    if start >= first_bin && start <= to {
                        clusters.push(MarkCluster {
                            first_hour: start as i32,
                            last_hour: (start + bin_hours - 1) as i32,
                            first_ordinal: ordinal.to_string(),
                            count: count.to_string(),
                        });
                    }
                    ordinal += count;
                }
            }
            Some(MarkSummary { from: f64::from(view.from), to: f64::from(view.to), width: view.width, clusters })
        }
        false => None,
    };
    let country = tiles[0].0.country.clone();
    Ok(ViewLine { country, summary, missing, partial, parts, marks })
}

fn unbase64(text: &str) -> io::Result<Vec<u8>> {
    let malformed = || invalid("A tile's bins are not base64");
    if text.len() % 4 != 0 {
        return Err(malformed());
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&byte| byte == b'=').count();
        if pad > 2 {
            return Err(malformed());
        }
        let mut word = 0u32;
        for (at, &byte) in chunk[..4 - pad].iter().enumerate() {
            let value = ALPHABET.iter().position(|&held| held == byte).ok_or_else(malformed)?;
            word |= (value as u32) << (18 - 6 * at);
        }
        out.extend_from_slice(&word.to_be_bytes()[1..4 - pad]);
    }
    Ok(out)
}
