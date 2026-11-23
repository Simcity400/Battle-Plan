
use crate::{
    Query,
    identity::{Country, Holder},
    pieces, read,
};
use dashboard_declare::{FacetFrom, Holds, Log};
use replay_recording::cells::TableReader;
use replay_columnar::{
    store::{TableView, Value},
    summary::bucket_hours,
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

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub mask: u128,
    pub value: u128,
    pub group: usize,
}

impl Pattern {
    pub fn holds(&self, facet: u128) -> bool {
        facet & self.mask == self.value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Number(i64),
    Text(String),
    List(Vec<u8>),
    Missing,
    Unrecorded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub hour: i32,
    pub key: u128,
    pub facet: u128,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug)]
pub struct Paged {
    pub row: Row,
    pub group: usize,
    pub ordinal: u64,
    pub own: u64,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub base: u64,
    pub first: u64,
    pub end: u64,
    pub last: u64,
    pub rows: Vec<Paged>,
}

impl Page {
    pub fn total(&self) -> u64 {
        self.last - self.base
    }

    pub fn cursor(&self) -> Option<u64> {
        (self.end < self.last).then_some(self.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counts {
    pub before: u64,
    pub bins: Vec<(u32, u64)>,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FacetFilter {
    pub name: String,
    #[serde(default)]
    #[ts(type = "Array<number>")]
    pub values: Vec<i64>,
    #[serde(default)]
    pub countries: Vec<Country>,
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub patterns: Vec<Pattern>,
    pub groups: Vec<Option<Country>>,
}

#[derive(Clone, Copy)]
enum Facet {
    Column(usize),
    Key(u32),
}

pub struct Reader<'q, 'a> {
    query: &'q Query<'a>,
    log: &'static Log,
    facets: Vec<Facet>,
    walked: Mutex<HashMap<(u32, i64), Arc<Vec<Row>>>>,
    cursor: Mutex<Option<(u32, TableReader)>>,
}

pub fn facet_shift(at: usize) -> u32 {
    32 * (3 - at as u32)
}

impl<'q, 'a> Reader<'q, 'a> {
    pub fn new(query: &'q Query<'a>, log: &'static Log) -> io::Result<Self> {
        let facets = log
            .facets
            .iter()
            .map(|facet| {
                Ok(match facet.from {
                    FacetFrom::Column(name) => Facet::Column(dashboard_declare::field(log.table, name)?),
                    FacetFrom::Key(shift) => Facet::Key(shift),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { query, log, facets, walked: Mutex::default(), cursor: Mutex::default() })
    }

    pub fn log(&self) -> &'static Log {
        self.log
    }

    pub fn query(&self) -> &Query<'a> {
        self.query
    }

    pub fn columns(&self) -> Vec<&'static str> {
        (0..self.log.table.field_count()).filter_map(|column| self.log.table.field_name(column)).collect()
    }

    pub fn select(&self, filters: &[FacetFilter], from: i32, to: i32) -> io::Result<Selection> {
        let mut patterns: Vec<Pattern> = self
            .query
            .timeline
            .within(from, to)
            .map(|stretch| Pattern { run: stretch.run, first: stretch.first, last: stretch.last, mask: 0, value: 0, group: 0 })
            .collect();
        let mut groups: Vec<Option<Country>> = vec![None];
        let mut seen: Vec<usize> = Vec::new();
        for filter in filters {
            let mut named = Vec::new();
            for name in filter.name.split('|') {
                let at = self
                    .log
                    .facets
                    .iter()
                    .position(|facet| facet.name == name)
                    .ok_or_else(|| invalid(format!("The log {} has no facet {}", self.log.name, name)))?;
                if seen.contains(&at) {
                    return Err(invalid(format!("The facet {name} is filtered twice")));
                }
                seen.push(at);
                named.push(at);
            }
            match (filter.values.is_empty(), filter.countries.is_empty()) {
                (false, true) => {
                    let &[at] = named.as_slice() else {
                        return Err(invalid(format!("The facets {} are filtered by tags only", filter.name)));
                    };
                    let shift = facet_shift(at);
                    let bits = u128::from(u32::MAX) << shift;
                    patterns = patterns
                        .iter()
                        .flat_map(|pattern| filter.values.iter().map(move |&value| Pattern { mask: pattern.mask | bits, value: pattern.value | u128::from(value as u32) << shift, ..*pattern }))
                        .collect();
                }
                (true, false) => {
                    let mut owners = Vec::with_capacity(named.len());
                    for &at in &named {
                        let Holds::Owner(key) = self.log.facets[at].holds else {
                            return Err(invalid(format!("The facet {} of {} holds no country", self.log.facets[at].name, self.log.name)));
                        };
                        owners.push((facet_shift(at), key));
                    }
                    if groups.len() > 1 || groups[0].is_some() {
                        return Err(invalid("One filter at most is by tags"));
                    }
                    groups = filter.countries.iter().cloned().map(Some).collect();
                    let mut out = Vec::new();
                    for pattern in &patterns {
                        for (group, tag) in filter.countries.iter().enumerate() {
                            for run in self.query.holders.within(tag, pattern.first, pattern.last) {
                                let Holder::Found { slot, id } = run.holder else { continue };
                                for &(shift, key) in &owners {
                                    out.push(Pattern {
                                        first: run.first,
                                        last: run.last,
                                        mask: pattern.mask | u128::from(u32::MAX) << shift,
                                        value: pattern.value | u128::from(key.of(slot, id) as u32) << shift,
                                        group,
                                        ..*pattern
                                    });
                                }
                            }
                        }
                    }
                    patterns = out;
                }
                _ => return Err(invalid(format!("The facet {} is filtered by values or by tags", filter.name))),
            }
        }
        Ok(Selection { patterns, groups })
    }

    fn run_hours(&self, run: u32) -> io::Result<(i64, i64)> {
        let held = self.query.replay.recording().run(run)?;
        Ok((i64::from(held.first_hour), i64::from(held.first_hour) + i64::from(held.ordinals) - 1))
    }

    fn facet_of(&self, view: &TableView<'_>, slot: u32) -> u128 {
        let mut key = 0u128;
        for (at, facet) in self.facets.iter().enumerate().take(4) {
            let value = match *facet {
                Facet::Column(column) => view.number(column, slot).map_or(u32::MAX, |value| value as u32),
                Facet::Key(shift) => view.key(slot).map_or(u32::MAX, |key| (key >> shift) as u32),
            };
            key |= u128::from(value) << facet_shift(at);
        }
        key
    }

    fn cells_of(view: &TableView<'_>, columns: usize, slot: u32) -> Vec<Cell> {
        (0..columns)
            .map(|column| match view.value(column, slot) {
                Value::Number(_) => view.number(column, slot).map_or(Cell::Unrecorded, Cell::Number),
                Value::Missing => Cell::Missing,
                Value::Unrecorded => Cell::Unrecorded,
                Value::Text(_) => read::text(view, column, slot).map_or(Cell::Unrecorded, |bytes| Cell::Text(String::from_utf8_lossy(bytes).into_owned())),
                Value::List(bytes) => Cell::List(bytes.to_vec()),
            })
            .collect()
    }

    fn logged(&self, run: u32, ranges: &[(i32, i32)]) -> io::Result<Vec<Row>> {
        let six = bucket_hours(0);
        let mut buckets: Vec<i64> = ranges.iter().flat_map(|&(a, b)| i64::from(a).div_euclid(six)..=i64::from(b).div_euclid(six)).collect();
        buckets.sort_unstable();
        buckets.dedup();
        let missing: Vec<i64> = {
            let walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
            buckets.iter().copied().filter(|bucket| !walked.contains_key(&(run, *bucket))).collect()
        };
        if !missing.is_empty() {
            let mut found: HashMap<i64, Vec<Row>> = missing.iter().map(|&bucket| (bucket, Vec::new())).collect();
            self.walk(run, &joined(&missing), |row| {
                if let Some(held) = found.get_mut(&i64::from(row.hour).div_euclid(six)) {
                    held.push(row);
                }
                Ok(())
            })?;
            let mut walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
            for (bucket, mut rows) in found {
                rows.sort_by_key(|row| (row.hour, row.key));
                walked.insert((run, bucket), Arc::new(rows));
            }
        }
        let walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = Vec::new();
        for bucket in buckets {
            for row in walked.get(&(run, bucket)).into_iter().flat_map(|rows| rows.iter()) {
                if ranges.iter().any(|&(a, b)| a <= row.hour && row.hour <= b) {
                    out.push(row.clone());
                }
            }
        }
        Ok(out)
    }

    fn walk(&self, run: u32, spans: &[(i64, i64)], mut visit: impl FnMut(Row) -> io::Result<()>) -> io::Result<()> {
        let six = bucket_hours(0);
        let (run_first, run_last) = self.run_hours(run)?;
        let recording = self.query.replay.recording();
        let columns = self.log.table.field_count();
        let mut cursor = self.cursor.lock().unwrap_or_else(PoisonError::into_inner);
        let mut reader = match cursor.take() {
            Some((held, reader)) if held == run => reader,
            _ => {
                let mut reader = crate::cost::storage(|| TableReader::new(recording, run, self.log.table, None))?;
                reader.index_keys();
                reader
            }
        };
        for &(first, last) in spans {
            let (from, to) = ((first * six).max(run_first) as i32, ((last + 1) * six - 1).min(run_last) as i32);
            if from > to {
                continue;
            }
            let start = if i64::from(from) > run_first { from - 1 } else { from };
            let mut before: HashSet<u128> = HashSet::new();
            crate::cost::storage(|| reader.go_to(recording, read::ordinal(recording, run, start)?))?;
            for hour in start..=to {
                if hour > start {
                    crate::cost::storage(|| reader.step(recording))?;
                }
                let view = reader.view();
                let appended = view.is_log();
                let mut now: HashSet<u128> = HashSet::new();
                for slot in view.rows() {
                    let Some(key) = view.key(slot) else { continue };
                    if !appended {
                        now.insert(key);
                    }
                    if hour < from || (!appended && before.contains(&key)) {
                        continue;
                    }
                    visit(Row { hour, key, facet: self.facet_of(&view, slot), cells: Self::cells_of(&view, columns, slot) })?;
                }
                before = now;
            }
        }
        *cursor = Some((run, reader));
        Ok(())
    }

    pub fn each(&self, run: u32, visit: impl FnMut(Row) -> io::Result<()>) -> io::Result<()> {
        let six = bucket_hours(0);
        let (first, last) = self.run_hours(run)?;
        let filled = crate::cost::storage(|| replay_lanes::lanes::logged_hours(self.query.replay.recording(), run, self.log.table, first as i32, last as i32))?;
        let mut buckets: Vec<i64> = filled.iter().flat_map(|&(a, b)| i64::from(a).div_euclid(six)..=i64::from(b).div_euclid(six)).collect();
        buckets.dedup();
        self.walk(run, &joined(&buckets), visit)
    }

    pub fn inserted(&self, run: u32, ranges: &[(i32, i32)]) -> io::Result<Vec<Row>> {
        let (Some(&(from, _)), Some(&(_, to))) = (ranges.first(), ranges.last()) else { return Ok(Vec::new()) };
        let filled = crate::cost::storage(|| replay_lanes::lanes::logged_hours(self.query.replay.recording(), run, self.log.table, from, to))?;
        let mut wanted: Vec<(i32, i32)> = Vec::new();
        let (mut a, mut b) = (0, 0);
        while a < ranges.len() && b < filled.len() {
            let (first, last) = (ranges[a].0.max(filled[b].0), ranges[a].1.min(filled[b].1));
            if first <= last {
                wanted.push((first, last));
            }
            if ranges[a].1 < filled[b].1 { a += 1 } else { b += 1 }
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        self.logged(run, &wanted)
    }

    pub fn rows(&self, run: u32, patterns: &[Pattern], ranges: &[(i32, i32)]) -> io::Result<Vec<(usize, Row)>> {
        let mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
        let mut out = Vec::new();
        if mine.is_empty() {
            return Ok(out);
        }
        for row in self.logged(run, ranges)? {
            for &at in &mine {
                let pattern = &patterns[at];
                if pattern.first <= row.hour && row.hour <= pattern.last && pattern.holds(row.facet) {
                    out.push((at, row.clone()));
                }
            }
        }
        out.sort_by_key(|(at, row)| (row.hour, *at, row.key));
        Ok(out)
    }

    pub fn every(&self, patterns: &[Pattern], from: i32, to: i32) -> io::Result<Vec<(usize, Row)>> {
        let six = bucket_hours(0);
        let mut by_run: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
        for (at, bins) in self.bins(patterns, from, to, six)?.into_iter().enumerate() {
            for (start, _) in bins {
                by_run.entry(patterns[at].run).or_default().push((start.max(i64::from(from)) as i32, (start + six - 1).min(i64::from(to)) as i32));
            }
        }
        let mut out = Vec::new();
        for (run, mut ranges) in by_run {
            ranges.sort_unstable();
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for (a, b) in ranges {
                match merged.last_mut() {
                    Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                    _ => merged.push((a, b)),
                }
            }
            out.extend(self.rows(run, patterns, &merged)?);
        }
        out.sort_by_key(|(at, row)| (row.hour, *at, row.key));
        Ok(out)
    }

    pub fn bins(&self, patterns: &[Pattern], from: i32, to: i32, bin: i64) -> io::Result<Vec<Vec<(i64, u64)>>> {
        let six = bucket_hours(0);
        let mut out: Vec<Vec<(i64, u64)>> = vec![Vec::new(); patterns.len()];
        let mut coarse: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
        let mut fine: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
        let mut slivers: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
        let split = |a: i64, b: i64, size: i64| -> (Option<(i64, i64)>, Vec<(i64, i64)>) {
            let (first, last) = ((a + size - 1).div_euclid(size) * size, (b + 1).div_euclid(size) * size - 1);
            if first > last {
                return (None, vec![(a, b)]);
            }
            let mut rest = Vec::new();
            if a < first {
                rest.push((a, first - 1));
            }
            if last < b {
                rest.push((last + 1, b));
            }
            (Some((first, last)), rest)
        };
        let mut runs: HashMap<u32, (i64, i64)> = HashMap::new();
        for (at, pattern) in patterns.iter().enumerate() {
            let (mut a, mut b) = (i64::from(from.max(pattern.first)), i64::from(to.min(pattern.last)));
            if a > b {
                continue;
            }
            let (run_first, run_last) = match runs.get(&pattern.run) {
                Some(&held) => held,
                None => *runs.entry(pattern.run).or_insert(self.run_hours(pattern.run)?),
            };
            if a == run_first {
                a = a.div_euclid(bin) * bin;
            }
            if b == run_last {
                b = (b.div_euclid(bin) + 1) * bin - 1;
            }
            let (whole, edges) = split(a, b, bin);
            if let Some((first, last)) = whole {
                coarse.entry(pattern.run).or_default().push((at, first, last));
            }
            for (x, y) in edges {
                let (middle, ends) = split(x, y, six);
                if let Some((first, last)) = middle {
                    fine.entry(pattern.run).or_default().push((at, first, last));
                }
                for (first, last) in ends {
                    slivers.entry(pattern.run).or_default().push((at, first, last));
                }
            }
        }
        let recording = self.query.replay.recording();
        for (spans, size) in [(&coarse, bin), (&fine, six)] {
            for (&run, held) in spans {
                let (first, last) = (held.iter().map(|span| span.1).min().unwrap_or(0), held.iter().map(|span| span.2).max().unwrap_or(0));
                let (run_first, run_last) = runs[&run];
                let asked = |facet: u128| held.iter().any(|&(at, _, _)| patterns[at].holds(facet));
                let counts = pieces::counts(recording, run, self.log.table, (first.max(run_first) as i32, last.min(run_last) as i32), size, &asked)?;
                for &(at, a, b) in held {
                    let pattern = &patterns[at];
                    for (facet, counts) in &counts {
                        if !pattern.holds(*facet) {
                            continue;
                        }
                        for (index, &count) in counts.iter().enumerate() {
                            let start = first + index as i64 * size;
                            if count > 0 && a <= start && start <= b {
                                out[at].push((start.div_euclid(bin) * bin, count));
                            }
                        }
                    }
                }
            }
        }
        for (&run, held) in &slivers {
            let mut ranges: Vec<(i32, i32)> = held.iter().map(|&(_, a, b)| (a as i32, b as i32)).collect();
            ranges.sort_unstable();
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for (a, b) in ranges {
                match merged.last_mut() {
                    Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                    _ => merged.push((a, b)),
                }
            }
            for (at, row) in self.rows(run, patterns, &merged)? {
                let hour = i64::from(row.hour);
                if held.iter().any(|&(owner, a, b)| owner == at && a <= hour && hour <= b) {
                    out[at].push((hour.div_euclid(bin) * bin, 1));
                }
            }
        }
        for bins in &mut out {
            bins.sort_unstable_by_key(|(start, _)| *start);
            let mut joined: Vec<(i64, u64)> = Vec::with_capacity(bins.len());
            for &(start, count) in bins.iter() {
                match joined.last_mut() {
                    Some(held) if held.0 == start => held.1 += count,
                    _ => joined.push((start, count)),
                }
            }
            *bins = joined;
        }
        Ok(out)
    }

    pub fn counts(&self, patterns: &[Pattern], groups: usize, first: i32, last: i32, bin_hours: i64) -> io::Result<Vec<Counts>> {
        if bin_hours < 1 || (bin_hours >= bucket_hours(0) && bin_hours % bucket_hours(0) != 0) {
            return Err(invalid("A log's bins are 1 to 5 hours or whole six-hour buckets"));
        }
        let timeline_first = self.query.timeline.first;
        let mut out = vec![Counts { before: 0, bins: Vec::new() }; groups];
        if first > timeline_first {
            let coarse = bin_for(i64::from(first) - i64::from(timeline_first), 256);
            for (at, bins) in self.bins(patterns, timeline_first, first - 1, coarse)?.into_iter().enumerate() {
                out[patterns[at].group].before += bins.iter().map(|(_, count)| count).sum::<u64>();
            }
        }
        let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
        let bin_of = |hour: i64| ((hour - first_bin).div_euclid(bin_hours)) as u32;
        let mut binned: Vec<BTreeMap<u32, u64>> = vec![BTreeMap::new(); groups];
        if bin_hours >= bucket_hours(0) {
            for (at, bins) in self.bins(patterns, first, last, bin_hours)?.into_iter().enumerate() {
                for (start, count) in bins {
                    *binned[patterns[at].group].entry(bin_of(start)).or_default() += count;
                }
            }
        } else {
            let mut runs: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
            for pattern in patterns {
                let (a, b) = (pattern.first.max(first), pattern.last.min(last));
                if a <= b {
                    runs.entry(pattern.run).or_default().push((a, b));
                }
            }
            for (run, ranges) in runs {
                for (at, row) in self.rows(run, patterns, &ranges)? {
                    if first <= row.hour && row.hour <= last {
                        *binned[patterns[at].group].entry(bin_of(i64::from(row.hour))).or_default() += 1;
                    }
                }
            }
        }
        for (counts, bins) in out.iter_mut().zip(binned) {
            counts.bins = bins.into_iter().collect();
        }
        Ok(out)
    }

    fn slice(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, window: impl FnOnce(u64, u64) -> (u64, u64)) -> io::Result<Page> {
        if from > to {
            return Err(invalid("Invalid log page"));
        }
        let first_hour = self.query.timeline.first;
        let bin = bin_for(i64::from(to) - i64::from(first_hour) + 1, 256);
        let mut before = vec![0u64; groups];
        if from > first_hour {
            for (at, bins) in self.bins(patterns, first_hour, from - 1, bin)?.into_iter().enumerate() {
                before[patterns[at].group] += bins.iter().map(|(_, count)| count).sum::<u64>();
            }
        }
        let mut bins: BTreeMap<i64, Vec<(usize, u64)>> = BTreeMap::new();
        for (at, held) in self.bins(patterns, from, to, bin)?.into_iter().enumerate() {
            for (start, count) in held {
                bins.entry(start).or_default().push((at, count));
            }
        }
        let base: u64 = before.iter().sum();
        let last = base + bins.values().flatten().map(|(_, count)| count).sum::<u64>();
        let (first, end) = window(base, last);
        let mut own = before;
        let mut ordinal = base;
        let mut rows = Vec::new();
        if first < end {
            let six = bucket_hours(0);
            for (&start, held) in &bins {
                if ordinal >= end {
                    break;
                }
                let count: u64 = held.iter().map(|(_, count)| count).sum();
                if ordinal + count <= first {
                    for &(at, count) in held {
                        own[patterns[at].group] += count;
                    }
                    ordinal += count;
                    continue;
                }
                let range = ((start.max(i64::from(from))) as i32, ((start + bin - 1).min(i64::from(to))) as i32);
                let held_patterns: Vec<Pattern> = held.iter().map(|&(at, _)| patterns[at]).collect();
                let mut buckets: BTreeMap<i64, u64> = BTreeMap::new();
                for counts in self.bins(&held_patterns, range.0, range.1, six)? {
                    for (bucket, count) in counts {
                        *buckets.entry(bucket).or_default() += count;
                    }
                }
                let (skip, reach) = page_buckets(&buckets, ordinal, first, end, range);
                if skip > 0 {
                    for (counts, held_pattern) in self.bins(&held_patterns, range.0, reach[0].0 - 1, bin)?.into_iter().zip(&held_patterns) {
                        own[held_pattern.group] += counts.iter().map(|(_, count)| count).sum::<u64>();
                    }
                    ordinal += skip;
                }
                let mut runs: Vec<u32> = held_patterns.iter().map(|pattern| pattern.run).collect();
                runs.sort_unstable();
                runs.dedup();
                let mut read = Vec::new();
                for run in runs {
                    read.extend(self.rows(run, patterns, &reach)?);
                }
                read.sort_by_key(|(at, row)| (row.hour, patterns[*at].group, row.key));
                for (at, row) in read {
                    let group = patterns[at].group;
                    if ordinal >= first && ordinal < end {
                        rows.push(Paged { row, group, ordinal, own: own[group] });
                    }
                    own[group] += 1;
                    ordinal += 1;
                }
            }
        }
        Ok(Page { base, first, end, last, rows })
    }

    pub fn page(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, cursor: u64, size: u32) -> io::Result<Page> {
        self.slice(patterns, groups, from, to, |base, last| {
            let first = cursor.max(base);
            let end = first.saturating_add(u64::from(size)).min(last);
            (first, end)
        })
    }

    pub fn tail(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, limit: u32) -> io::Result<Page> {
        let mut page = self.slice(patterns, groups, from, to, |base, last| {
            let first = last.saturating_sub(u64::from(limit)).max(base);
            (first, last)
        })?;
        page.rows.reverse();
        Ok(page)
    }

    pub fn total(&self, patterns: &[Pattern], from: i32, to: i32) -> io::Result<u64> {
        if from > to {
            return Ok(0);
        }
        let bin = bin_for(i64::from(to) - i64::from(from) + 1, 256);
        let mut total = 0u64;
        for held in self.bins(patterns, from, to, bin)? {
            total += held.iter().map(|(_, count)| count).sum::<u64>();
        }
        Ok(total)
    }
}

fn page_buckets(buckets: &BTreeMap<i64, u64>, ordinal: u64, first: u64, end: u64, range: (i32, i32)) -> (u64, Vec<(i32, i32)>) {
    let six = bucket_hours(0);
    let mut skip = 0u64;
    let mut seen = ordinal;
    let mut reach: Vec<(i32, i32)> = Vec::new();
    for (&bucket, &count) in buckets {
        if seen + count <= first {
            seen += count;
            skip += count;
            continue;
        }
        let hours = (bucket.max(i64::from(range.0)) as i32, (bucket + six - 1).min(i64::from(range.1)) as i32);
        match reach.last_mut() {
            Some(held) if held.1 + 1 >= hours.0 => held.1 = held.1.max(hours.1),
            _ => reach.push(hours),
        }
        seen += count;
        if seen >= end {
            break;
        }
    }
    if reach.is_empty() {
        reach.push(range);
    }
    (skip, reach)
}

fn joined(buckets: &[i64]) -> Vec<(i64, i64)> {
    let mut spans: Vec<(i64, i64)> = Vec::new();
    for &bucket in buckets {
        match spans.last_mut() {
            Some(last) if last.1 + 1 == bucket => last.1 = bucket,
            _ => spans.push((bucket, bucket)),
        }
    }
    spans
}

pub fn bin_for(hours: i64, bins: i64) -> i64 {
    let wanted = (hours / bins.max(1)).max(1);
    let mut level = 0u8;
    while bucket_hours(level + 1) <= wanted {
        level += 1;
    }
    bucket_hours(level)
}
