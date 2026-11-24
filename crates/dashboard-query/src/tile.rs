
use crate::{
    chart::{ChartBucket, ChartRun},
    identity::Country,
    series::{Ask, Line, MissingSpan, SeriesLineAsk, Span},
};
use replay_columnar::summary::{BASE_HOURS, bucket_hours};
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

pub const PRODUCT: &str = "series-tiles";
pub const BINS: usize = 256;

pub const PRESENT: u8 = 1;
pub const FIRST_BREAK: u8 = 2;
pub const HAS_BREAK: u8 = 4;
pub const LEVEL: u8 = 8;
pub const RUN: u8 = 16;
pub const SEAM: u8 = 32;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub fn params_walk() -> crate::fields::Walk {
    crate::fields::walk::<Params>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    #[serde(default)]
    lines: Vec<SeriesLineAsk>,
    within: Option<Vec<(Country, Vec<Span>)>>,
    #[serde(default)]
    with_marks: bool,
    top: Option<u32>,
    #[serde(default)]
    least: bool,
    hour: Option<f64>,
    rank_by: Option<String>,
}

#[doc = " How far the membership of a top is proven on each side of its hour, in hours."]
pub const RANK_PROOF_HOURS: i32 = 48;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TileRanking {
    #[doc = " The hour the top was ranked at."]
    pub hour: i32,
    #[doc = " The hours over which the same countries are the top; ask again outside them."]
    pub from: i32,
    pub until: i32,
    #[doc = " Why the top is not ranked at this hour; no lines then."]
    pub refused: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TilePart {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
    #[doc = " The first hour of its holder's unbroken segment."]
    pub segment_first_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TileMarks {
    pub before: String,
    #[ts(type = "Array<[number, number]>")]
    pub bins: Vec<(u32, u64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TileLine {
    pub country: Country,
    #[doc = " The part's key, for a sub-keyed series."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub part: Option<String>,
    #[doc = " The tile's hours the tag holds no value in, with why."]
    pub missing: Vec<MissingSpan>,
    pub parts: Vec<TilePart>,
    #[doc = " Asked with marks, for a series with drops."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub marks: Option<TileMarks>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesTileReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    #[ts(type = "number")]
    pub bin_hours: i64,
    #[ts(type = "number")]
    pub index: i64,
    #[doc = " The first hour of its first bin."]
    pub first_hour: i32,
    #[doc = " Wholly before the timeline's last hour: it never changes."]
    pub immutable: bool,
    #[doc = " The last hour of the tile the timeline reaches; none when it reaches none."]
    pub filled: Option<i32>,
    #[doc = " Values are raw fixed-point integers as 32-bit words, `decimals` places; else 64-bit floats."]
    pub raw_values: bool,
    #[doc = " Bytes of each hour: 1, 2 or 4; 0 for a tile of hourly runs."]
    pub hour_bytes: u8,
    pub lines: Vec<TileLine>,
    #[doc = " The bins, base64."]
    pub bins: String,
    #[doc = " Asked for a top: who it is at the asked hour, and for how long."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ranking: Option<TileRanking>,
}

pub fn level_of(bin_hours: i64) -> io::Result<u8> {
    if !(1..=i64::from(i32::MAX) / BINS as i64).contains(&bin_hours) {
        return Err(invalid("Invalid tile bin hours"));
    }
    if bin_hours < BASE_HOURS {
        return Ok(replay_lanes::lanes::WALKED);
    }
    let mut level = 0u8;
    while bucket_hours(level + 1) <= bin_hours {
        level += 1;
    }
    if bin_hours % bucket_hours(level) != 0 {
        return Err(invalid("A tile's bins are whole buckets of one summary level"));
    }
    Ok(level)
}

pub fn hours(bin_hours: i64, index: i64) -> io::Result<(i32, i32)> {
    let span = BINS as i64 * bin_hours;
    let first = index.checked_mul(span).ok_or_else(|| invalid("Invalid tile index"))?;
    let last = first + span - 1;
    if first - bin_hours < i64::from(i32::MIN) || last > i64::from(i32::MAX) {
        return Err(invalid("Invalid tile index"));
    }
    Ok((first as i32, last as i32))
}

pub fn closure(first: i32, last: i32, timeline_last: i32) -> (bool, Option<i32>) {
    (last < timeline_last, (first <= timeline_last).then(|| last.min(timeline_last)))
}

pub fn reply(line: &Line<'_, '_>, branch_id: &str, params: &serde_json::Value) -> io::Result<SeriesTileReply> {
    let params: Params = crate::named::named(params).map_err(|error| invalid(format!("Invalid tile request: {error}")))?;
    let Some(top) = params.top else {
        let asks = line.asks(&params.countries, &params.lines)?;
        return tile(line, branch_id, &asks, params.within.as_deref(), params.bin_hours, params.index, params.with_marks);
    };
    if !params.countries.is_empty() || !params.lines.is_empty() {
        return Err(invalid("A top of tiles names no country: the ranking names them"));
    }
    let n = usize::try_from(top).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one country or more"))?;
    let hour = crate::series::hour_of(params.hour, "hour")?;
    let by = match params.rank_by.as_deref() {
        None => None,
        Some(name) => {
            let measure = line.series().measure(name).ok_or_else(|| invalid(format!("{} has no measure {name} to rank by", line.series().kind)))?;
            Some(Line::new(line.query(), line.series(), measure)?)
        }
    };
    let ranker = by.as_ref().unwrap_or(line);
    let (tags, ranking) = match ranker.rank_at(hour, n, params.least)? {
        Err(reason) => (Vec::new(), TileRanking { hour, from: hour, until: hour, refused: Some(reason) }),
        Ok(tags) => {
            let held: std::collections::BTreeSet<&str> = tags.iter().map(String::as_str).collect();
            let (from, until) = ranker.rank_holds(hour, n, params.least, (i32::MIN, i32::MAX), |ranked| ranked.len() == held.len() && ranked.iter().all(|tag| held.contains(tag.as_str())))?;
            (tags, TileRanking { hour, from, until, refused: None })
        }
    };
    let asks: Vec<Ask> = tags.iter().map(|tag| Ask::whole(tag.as_str())).collect();
    let mut reply = tile(line, branch_id, &asks, None, params.bin_hours, params.index, params.with_marks)?;
    reply.ranking = Some(ranking);
    Ok(reply)
}

pub fn tile(
    line: &Line<'_, '_>,
    branch_id: &str,
    asks: &[Ask],
    within: Option<&[(Country, Vec<Span>)]>,
    bin_hours: i64,
    index: i64,
    with_marks: bool,
) -> io::Result<SeriesTileReply> {
    let level = level_of(bin_hours)?;
    let (first, last) = hours(bin_hours, index)?;
    let query = line.query();
    let key = line.series().key;
    for ask in asks {
        if key.unowned() && ask.country.tag().and_then(|tag| key.fixed(tag)).is_none() {
            return Err(invalid(format!("{} names no row of {}", ask.country, line.series().kind)));
        }
        if !key.unowned() && !query.holders.named(&ask.country) {
            return Err(invalid(match &ask.country {
                Country::Tag(tag) => format!("No country of this history holds the tag {tag}"),
                Country::Slot(slot) => format!("No country of this history is the country {slot}"),
            }));
        }
    }
    let read_from = first - bin_hours as i32;
    let drawn: Vec<(Ask, Vec<(i32, i32)>)> = asks
        .iter()
        .map(|ask| {
            let spans = match within {
                Some(within) => within
                    .iter()
                    .filter(|(country, _)| *country == ask.country)
                    .flat_map(|(_, spans)| spans)
                    .map(|span| (span.first_hour.max(read_from), span.last_hour.min(last)))
                    .filter(|(a, b)| a <= b)
                    .collect(),
                None => vec![(read_from, last)],
            };
            (ask.clone(), spans)
        })
        .collect();
    let scale = line.scale();
    let mut marks = match with_marks && line.drops() {
        true => {
            let countries: Vec<Country> = asks.iter().map(|ask| ask.country.clone()).collect();
            line.drop_counts(&countries, first, last, bin_hours)?.into_iter().map(|(before, bins)| Some(TileMarks { before: before.to_string(), bins })).collect()
        }
        false => vec![None; asks.len()],
    };
    let (immutable, filled) = closure(first, last, query.timeline.last);
    let (bins, raw_values, hour_bytes) = match bin_hours {
        1 => {
            let runs: Vec<Vec<ChartRun>> = line.hourly_runs_for_tile(&drawn, read_from, last)?.into_iter().map(|(runs, _)| runs).collect();
            let (bytes, raw_values) = encode_runs(&runs, first, scale);
            (bytes, raw_values, 0)
        }
        _ => {
            let mut buckets: Vec<Option<ChartBucket>> = Vec::with_capacity(asks.len() * BINS);
            for (bins, _) in line.binned_for_tile(&drawn, read_from, last, level, bin_hours)? {
                debug_assert_eq!(bins.len(), BINS + 1);
                buckets.extend(bins.into_iter().skip(1));
            }
            encode(&buckets, first, bin_hours, scale)
        }
    };
    let held_parts: Vec<Vec<crate::series::Part>> = asks.iter().map(|ask| line.held(ask, first, last)).collect();
    let all_parts: Vec<&crate::series::Part> = held_parts.iter().flatten().collect();
    line.prefetch_unwitnessed(&all_parts, first, last)?;
    let mut lines = Vec::with_capacity(asks.len());
    for (at, (ask, parts)) in asks.iter().zip(held_parts).enumerate() {
        lines.push(TileLine {
            country: ask.country.clone(),
            part: line.part_text(ask),
            missing: line.missing_for_tile(&ask.country, first, last, &parts)?,
            parts: parts
                .iter()
                .map(|part| TilePart { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id, segment_first_hour: part.segment_first })
                .collect(),
            marks: marks[at].take(),
        });
    }
    Ok(SeriesTileReply {
        branch_id: branch_id.to_owned(),
        measure: line.measure_name().to_owned(),
        decimals: line.decimals(),
        bin_hours,
        index,
        first_hour: first,
        immutable,
        filled,
        raw_values,
        hour_bytes,
        lines,
        bins: base64(&bins),
        ranking: None,
    })
}

fn raw_word(value: f64, scale: f64) -> Option<i32> {
    let raw = (value * scale).round();
    let narrow = raw as i32;
    (f64::from(narrow) == raw && (f64::from(narrow) / scale).to_bits() == value.to_bits()).then_some(narrow)
}

pub fn encode_runs(runs: &[Vec<ChartRun>], first: i32, scale: f64) -> (Vec<u8>, bool) {
    let last = first + BINS as i32 - 1;
    let shown = || runs.iter().flatten().filter(|run| run.last_hour >= first && run.first_hour <= last);
    let raw_values = shown().all(|run| raw_word(run.value, scale).is_some());
    let mut flags = vec![0u8; runs.len() * BINS];
    let mut values = Vec::new();
    for (tag, held) in runs.iter().enumerate() {
        for run in held.iter().filter(|run| run.last_hour >= first && run.first_hour <= last) {
            let start = run.first_hour.max(first);
            let at = tag * BINS + (start - first) as usize;
            flags[at] = PRESENT
                | RUN
                | match (run.first_hour < first, run.break_before) {
                    (true, _) => SEAM,
                    (false, true) => FIRST_BREAK,
                    (false, false) => 0,
                };
            for hour in start + 1..=run.last_hour.min(last) {
                flags[tag * BINS + (hour - first) as usize] = PRESENT;
            }
            match raw_values {
                true => values.extend_from_slice(&raw_word(run.value, scale).expect("checked").to_le_bytes()),
                false => values.extend_from_slice(&run.value.to_le_bytes()),
            }
        }
    }
    flags.extend(values);
    (flags, raw_values)
}

pub fn decode_runs(bytes: &[u8], tags: usize, first: i32, scale: f64, raw_values: bool) -> io::Result<Vec<Vec<(i32, i32, f64, u8)>>> {
    let flags = bytes.get(..tags * BINS).ok_or_else(|| invalid("A tile's flags are cut short"))?;
    let starts = flags.iter().filter(|&&flag| flag & RUN != 0).count();
    let size = if raw_values { 4 } else { 8 };
    if bytes.len() != tags * BINS + starts * size {
        return Err(invalid("A tile's runs disagree with its flags"));
    }
    let mut nth = 0;
    let mut out = Vec::with_capacity(tags);
    for tag in 0..tags {
        let mut runs: Vec<(i32, i32, f64, u8)> = Vec::new();
        for (bin, &flag) in flags[tag * BINS..(tag + 1) * BINS].iter().enumerate() {
            let hour = first + bin as i32;
            if flag & RUN != 0 {
                let at = tags * BINS + nth * size;
                let mut word = [0u8; 8];
                word[..size].copy_from_slice(&bytes[at..at + size]);
                let value = match raw_values {
                    true => f64::from(i32::from_le_bytes(word[..4].try_into().expect("four"))) / scale,
                    false => f64::from_le_bytes(word),
                };
                nth += 1;
                runs.push((hour, hour, value, flag));
            } else if flag & PRESENT != 0 {
                runs.last_mut().ok_or_else(|| invalid("A tile's hour holds a value no run starts"))?.1 = hour;
            }
        }
        out.push(runs);
    }
    Ok(out)
}

pub fn hour_bytes(bin_hours: i64) -> u8 {
    match bin_hours {
        ..=256 => 1,
        ..=65_536 => 2,
        _ => 4,
    }
}

fn level(bucket: &ChartBucket) -> bool {
    let value = bucket.first_value.to_bits();
    [bucket.last_value, bucket.min_value, bucket.max_value].into_iter().all(|other| other.to_bits() == value)
}

pub fn encode(buckets: &[Option<ChartBucket>], first: i32, bin_hours: i64, scale: f64) -> (Vec<u8>, bool, u8) {
    let present: Vec<(usize, &ChartBucket)> = buckets.iter().enumerate().filter_map(|(at, bucket)| Some((at, bucket.as_ref()?))).collect();
    let raw = |value: f64| raw_word(value, scale);
    let raw_values = present
        .iter()
        .all(|(_, bucket)| [bucket.first_value, bucket.last_value, bucket.min_value, bucket.max_value].into_iter().all(|value| raw(value).is_some()));
    let hours = hour_bytes(bin_hours);
    let mut out = Vec::with_capacity(buckets.len() + present.len() * (if raw_values { 16 } else { 32 } + 5 * usize::from(hours)));
    out.extend(buckets.iter().map(|bucket| match bucket {
        None => 0,
        Some(bucket) => {
            PRESENT
                | if bucket.first_break { FIRST_BREAK } else { 0 }
                | if bucket.has_break { HAS_BREAK } else { 0 }
                | if level(bucket) { LEVEL } else { 0 }
        }
    }));
    for (_, bucket) in &present {
        let values = [bucket.first_value, bucket.last_value, bucket.min_value, bucket.max_value];
        for &value in &values[..if level(bucket) { 1 } else { 4 }] {
            match raw_values {
                true => out.extend_from_slice(&raw(value).expect("checked").to_le_bytes()),
                false => out.extend_from_slice(&value.to_le_bytes()),
            }
        }
    }
    for &(at, bucket) in &present {
        let start = i64::from(first) + (at % BINS) as i64 * bin_hours;
        for hour in [bucket.first_hour, bucket.last_hour, bucket.last_value_hour, bucket.min_hour, bucket.max_hour] {
            let offset = (i64::from(hour) - start) as u32;
            out.extend_from_slice(&offset.to_le_bytes()[..usize::from(hours)]);
        }
    }
    (out, raw_values, hours)
}

pub fn decode(bytes: &[u8], tags: usize, first: i32, bin_hours: i64, scale: f64, raw_values: bool, hour_bytes: u8) -> io::Result<Vec<Option<ChartBucket>>> {
    let flags = bytes.get(..tags * BINS).ok_or_else(|| invalid("A tile's flags are cut short"))?;
    let present = flags.iter().filter(|&&flag| flag & PRESENT != 0).count();
    let values: usize = flags.iter().filter(|&&flag| flag & PRESENT != 0).map(|&flag| if flag & LEVEL != 0 { 1 } else { 4 }).sum();
    let (value_bytes, hour_bytes) = (if raw_values { 4 } else { 8 }, usize::from(hour_bytes));
    let values_at = tags * BINS;
    let hours_at = values_at + values * value_bytes;
    if bytes.len() != hours_at + present * 5 * hour_bytes {
        return Err(invalid("A tile's bins disagree with its flags"));
    }
    let word = |at: usize, size: usize| -> u64 {
        let mut held = [0u8; 8];
        held[..size].copy_from_slice(&bytes[at..at + size]);
        u64::from_le_bytes(held)
    };
    let mut out = Vec::with_capacity(flags.len());
    let (mut nth, mut value_at) = (0, 0);
    for (at, &flag) in flags.iter().enumerate() {
        if flag & PRESENT == 0 {
            out.push(None);
            continue;
        }
        let flat = flag & LEVEL != 0;
        let value = |k: usize| {
            let at = values_at + (value_at + if flat { 0 } else { k }) * value_bytes;
            match raw_values {
                true => f64::from(word(at, 4) as u32 as i32) / scale,
                false => f64::from_bits(word(at, 8)),
            }
        };
        let start = i64::from(first) + (at % BINS) as i64 * bin_hours;
        let hour = |k: usize| (start + word(hours_at + (nth * 5 + k) * hour_bytes, hour_bytes) as i64) as i32;
        out.push(Some(ChartBucket {
            absolute_total: 0,
            first_hour: hour(0),
            last_hour: hour(1),
            first_value: value(0),
            last_value: value(1),
            last_value_hour: hour(2),
            min_hour: hour(3),
            min_value: value(2),
            max_hour: hour(4),
            max_value: value(3),
            first_break: flag & FIRST_BREAK != 0,
            has_break: flag & HAS_BREAK != 0,
        }));
        nth += 1;
        value_at += if flat { 1 } else { 4 };
    }
    Ok(out)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = u32::from(chunk[0]) << 16 | u32::from(*chunk.get(1).unwrap_or(&0)) << 8 | u32::from(*chunk.get(2).unwrap_or(&0));
        for (at, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if at <= chunk.len() { char::from(ALPHABET[(word >> shift) as usize & 63]) } else { '=' });
        }
    }
    out
}

pub mod view;

#[cfg(test)]
mod tests;
