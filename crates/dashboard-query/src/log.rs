
use crate::{
    Query,
    identity::{Country, Holder},
    pieces, read,
};
use dashboard_declare::{FacetFrom, Holds, Log};
use replay_columnar::{
    store::{Strings, TableState, TableView, Value},
    summary::bucket_hours,
    unit::{self, reader::LogEvent},
};
use replay_recording::{
    cells::{TableReader, read_columns},
    columnar_frames::{Damage, DamageScope, TableCoverage},
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, Mutex, PoisonError},
};
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static EXAMINED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static PATTERNS_CHECKED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn count_examined<T>(run: impl FnOnce() -> T) -> (T, u64) {
    let prev = EXAMINED.with(|cell| cell.replace(Some(0)));
    let value = run();
    let count = EXAMINED.with(|cell| cell.replace(prev)).unwrap_or(0);
    (value, count)
}

#[cfg(test)]
pub fn count_patterns_checked<T>(run: impl FnOnce() -> T) -> (T, u64) {
    let prev = PATTERNS_CHECKED.with(|cell| cell.replace(Some(0)));
    let value = run();
    let count = PATTERNS_CHECKED.with(|cell| cell.replace(prev)).unwrap_or(0);
    (value, count)
}

#[inline]
fn record_examined(n: usize) {
    #[cfg(test)]
    EXAMINED.with(|cell| {
        if let Some(count) = cell.get() {
            cell.set(Some(count.saturating_add(n as u64)));
        }
    });
    #[cfg(not(test))]
    let _ = n;
}

#[inline]
fn record_patterns_checked(n: usize) {
    #[cfg(test)]
    PATTERNS_CHECKED.with(|cell| {
        if let Some(count) = cell.get() {
            cell.set(Some(count.saturating_add(n as u64)));
        }
    });
    #[cfg(not(test))]
    let _ = n;
}

const DEFAULT_MAX_BUCKETS: usize = 64;
const DEFAULT_MAX_BUCKET_ENTRIES: usize = 524_288;

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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountrySpan {
    pub country: Country,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnobservedSpan {
    pub first_hour: i32,
    pub last_hour: i32,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub membership: Vec<CountrySpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub unobserved: Vec<UnobservedSpan>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub deduplicate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    EachPattern,
    FirstMatch,
}

#[derive(Clone, Debug)]
pub struct NewestPage {
    pub base: u64,
    pub offset: u64,
    pub total: u64,
    pub rows: Vec<Paged>,
}

#[derive(Clone, Debug, Default)]
struct RunIntervalTree {
    order: Vec<usize>,
    firsts: Vec<i32>,
    lasts: Vec<i32>,
    cap: usize,
    tree_max_last: Vec<i32>,
}

impl RunIntervalTree {
    fn build(patterns: &[Pattern], mut order: Vec<usize>) -> Self {
        order.sort_unstable_by_key(|&at| (patterns[at].first, patterns[at].last, patterns[at].group, at));
        let firsts: Vec<i32> = order.iter().map(|&at| patterns[at].first).collect();
        let lasts: Vec<i32> = order.iter().map(|&at| patterns[at].last).collect();
        let cap = order.len().max(1).next_power_of_two();
        let mut tree_max_last = vec![i32::MIN; cap * 2];
        for (i, &last) in lasts.iter().enumerate() {
            tree_max_last[cap + i] = last;
        }
        for i in (1..cap).rev() {
            tree_max_last[i] = tree_max_last[2 * i].max(tree_max_last[2 * i + 1]);
        }
        Self { order, firsts, lasts, cap, tree_max_last }
    }

    fn overlapping(&self, from: i32, to: i32, out: &mut Vec<usize>) {
        if self.order.is_empty() || from > to {
            return;
        }
        let hi = self.firsts.partition_point(|&f| f <= to);
        if hi == 0 || self.tree_max_last[1] < from {
            return;
        }
        let mut stack: Vec<(usize, usize, usize)> = Vec::with_capacity(64);
        stack.push((1, 0, self.cap));
        while let Some((node, l, r)) = stack.pop() {
            record_patterns_checked(1);
            if l >= hi || self.tree_max_last[node] < from {
                continue;
            }
            if r - l == 1 {
                if l < self.order.len() && self.lasts[l] >= from {
                    out.push(self.order[l]);
                }
                continue;
            }
            let mid = (l + r) / 2;
            if mid < hi && self.tree_max_last[2 * node + 1] >= from {
                stack.push((2 * node + 1, mid, r));
            }
            if self.tree_max_last[2 * node] >= from {
                stack.push((2 * node, l, mid));
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SelectionIndex {
    by_run: Vec<(u32, RunIntervalTree)>,
}

impl SelectionIndex {
    fn build(patterns: &[Pattern]) -> Self {
        let mut grouped: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (at, pattern) in patterns.iter().enumerate() {
            grouped.entry(pattern.run).or_default().push(at);
        }
        let by_run = grouped.into_iter().map(|(run, indices)| (run, RunIntervalTree::build(patterns, indices))).collect();
        Self { by_run }
    }

    fn overlapping(&self, from: i32, to: i32) -> Vec<usize> {
        let mut out = Vec::new();
        for (_, tree) in &self.by_run {
            tree.overlapping(from, to, &mut out);
        }
        out.sort_unstable();
        out
    }
}

struct FacetIndex {
    masks: Vec<(u128, HashMap<u128, Vec<usize>>)>,
}

impl FacetIndex {
    fn new(patterns: &[Pattern], held: &[(usize, i64, i64)]) -> Self {
        let mut masks: Vec<(u128, HashMap<u128, Vec<usize>>)> = Vec::new();
        for (idx, &(at, _, _)) in held.iter().enumerate() {
            let p = &patterns[at];
            if let Some((_, map)) = masks.iter_mut().find(|(mask, _)| *mask == p.mask) {
                map.entry(p.value).or_default().push(idx);
            } else {
                let mut map = HashMap::new();
                map.insert(p.value, vec![idx]);
                masks.push((p.mask, map));
            }
        }
        Self { masks }
    }

    fn asked(&self, facet: u128) -> bool {
        self.masks.iter().any(|(mask, map)| map.contains_key(&(facet & *mask)))
    }

    fn matching(&self, facet: u128, out: &mut Vec<usize>) {
        out.clear();
        for (mask, map) in &self.masks {
            if let Some(list) = map.get(&(facet & *mask)) {
                out.extend_from_slice(list);
            }
        }
        if self.masks.len() > 1 && out.len() > 1 {
            out.sort_unstable();
        }
    }
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub patterns: Vec<Pattern>,
    pub groups: Vec<Option<Country>>,
    pub overlap: Overlap,
    index: SelectionIndex,
}

impl Selection {
    fn new(patterns: Vec<Pattern>, groups: Vec<Option<Country>>, overlap: Overlap) -> Self {
        let index = SelectionIndex::build(&patterns);
        Self { patterns, groups, overlap, index }
    }
}

#[derive(Clone, Copy)]
enum Facet {
    Column(usize),
    Key(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BucketEntry {
    hour: i32,
    key: u128,
    facet: u128,
    log_ordinal: Option<u64>,
}

type BucketKey = (u64, u64, [u8; 16], u32, usize, i64);

pub struct BucketCache {
    max_buckets: usize,
    max_entries: usize,
    total_entries: usize,
    order: VecDeque<BucketKey>,
    map: HashMap<BucketKey, Arc<Vec<BucketEntry>>>,
}

impl Default for BucketCache {
    fn default() -> Self {
        Self::with_limits(DEFAULT_MAX_BUCKETS, DEFAULT_MAX_BUCKET_ENTRIES)
    }
}

impl BucketCache {
    pub fn with_limits(max_buckets: usize, max_entries: usize) -> Self {
        Self {
            max_buckets: max_buckets.max(1),
            max_entries,
            total_entries: 0,
            order: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.map.clear();
        self.total_entries = 0;
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn get(&self, key: &BucketKey) -> Option<Arc<Vec<BucketEntry>>> {
        self.map.get(key).map(Arc::clone)
    }

    fn insert(&mut self, key: BucketKey, rows: Arc<Vec<BucketEntry>>) {
        if rows.len() > self.max_entries {
            return;
        }
        if let Some(prev) = self.map.insert(key, Arc::clone(&rows)) {
            self.total_entries = self.total_entries.saturating_sub(prev.len()) + rows.len();
            return;
        }
        self.order.push_back(key);
        self.total_entries += rows.len();
        while self.map.len() > self.max_buckets || self.total_entries > self.max_entries {
            let Some(evicted) = self.order.pop_front() else { break };
            if let Some(removed) = self.map.remove(&evicted) {
                self.total_entries = self.total_entries.saturating_sub(removed.len());
            }
        }
    }
}

pub struct Reader<'q, 'a> {
    query: &'q Query<'a>,
    log: &'static Log,
    facets: Vec<Facet>,
    facet_columns: Vec<usize>,
    walked: Mutex<HashMap<(u32, i64), Arc<Vec<BucketEntry>>>>,
    shared: Option<Arc<Mutex<BucketCache>>>,
    cursor: Mutex<Option<(u32, TableReader)>>,
    facet_cursor: Mutex<Option<(u32, TableReader)>>,
}

pub fn facet_shift(at: usize) -> u32 {
    32 * (3 - at as u32)
}

fn subtract_unobserved(first: i32, last: i32, unobserved: &[UnobservedSpan]) -> Vec<(i32, i32)> {
    if first > last {
        return Vec::new();
    }
    if unobserved.is_empty() {
        return vec![(first, last)];
    }
    let mut blocked: Vec<(i32, i32)> = unobserved
        .iter()
        .filter_map(|span| {
            let a = span.first_hour.max(first);
            let b = span.last_hour.min(last);
            (a <= b).then_some((a, b))
        })
        .collect();
    blocked.sort_unstable();
    let mut out = Vec::new();
    let mut cur = i64::from(first);
    let end = i64::from(last);
    for (a, b) in blocked {
        let a = i64::from(a);
        let b = i64::from(b);
        if a > cur {
            out.push((cur as i32, (a - 1) as i32));
        }
        cur = cur.max(b + 1);
        if cur > end {
            break;
        }
    }
    if cur <= end {
        out.push((cur as i32, end as i32));
    }
    out
}

fn canonical_match_at(patterns: &[Pattern], mine: &[usize], hour: i32, facet: u128) -> Option<usize> {
    mine.iter().copied().find(|&at| {
        let p = &patterns[at];
        p.first <= hour && hour <= p.last && p.holds(facet)
    })
}

impl<'q, 'a> Reader<'q, 'a> {
    pub fn new(query: &'q Query<'a>, log: &'static Log) -> io::Result<Self> {
        Self::build(query, log, None)
    }

    pub fn with_cache(query: &'q Query<'a>, log: &'static Log, cache: Arc<Mutex<BucketCache>>) -> io::Result<Self> {
        Self::build(query, log, Some(cache))
    }

    fn build(query: &'q Query<'a>, log: &'static Log, shared: Option<Arc<Mutex<BucketCache>>>) -> io::Result<Self> {
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
        let mut facet_columns: Vec<usize> = facets
            .iter()
            .filter_map(|facet| match *facet {
                Facet::Column(col) => Some(col),
                Facet::Key(_) => None,
            })
            .collect();
        facet_columns.sort_unstable();
        facet_columns.dedup();
        Ok(Self {
            query,
            log,
            facets,
            facet_columns,
            walked: Mutex::default(),
            shared,
            cursor: Mutex::default(),
            facet_cursor: Mutex::default(),
        })
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
        let mut overlap = Overlap::EachPattern;
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
            if filter.deduplicate || !filter.membership.is_empty() || !filter.unobserved.is_empty() {
                overlap = Overlap::FirstMatch;
            }
            let has_values = !filter.values.is_empty();
            let has_countries = !filter.countries.is_empty();
            let has_membership = !filter.membership.is_empty() || !filter.unobserved.is_empty();
            match (has_values, has_countries, has_membership) {
                (true, false, false) => {
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
                (false, true, _) if filter.membership.is_empty() => {
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
                        let segments = subtract_unobserved(pattern.first, pattern.last, &filter.unobserved);
                        for (group, tag) in filter.countries.iter().enumerate() {
                            for &(seg_first, seg_last) in &segments {
                                for run in self.query.holders.within(tag, seg_first, seg_last) {
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
                    }
                    patterns = out;
                }
                (false, false, true) => {
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
                    let mut by_country: BTreeMap<Country, Vec<(i32, i32)>> = BTreeMap::new();
                    for span in &filter.membership {
                        if span.first_hour <= span.last_hour {
                            by_country.entry(span.country.clone()).or_default().push((span.first_hour, span.last_hour));
                        }
                    }
                    let mut unique_tags: Vec<Country> = filter.membership.iter().map(|span| span.country.clone()).collect();
                    unique_tags.sort_unstable();
                    unique_tags.dedup();
                    let group_index: HashMap<Country, usize> = unique_tags.iter().enumerate().map(|(idx, tag)| (tag.clone(), idx)).collect();
                    groups = if unique_tags.is_empty() { vec![None] } else { unique_tags.into_iter().map(Some).collect() };
                    for spans in by_country.values_mut() {
                        spans.sort_unstable();
                        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
                        for (a, b) in spans.drain(..) {
                            match merged.last_mut() {
                                Some(last) if i64::from(last.1) + 1 >= i64::from(a) => last.1 = last.1.max(b),
                                _ => merged.push((a, b)),
                            }
                        }
                        *spans = merged;
                    }
                    let mut out = Vec::new();
                    for pattern in &patterns {
                        let segments = subtract_unobserved(pattern.first, pattern.last, &filter.unobserved);
                        for (country, spans) in &by_country {
                            let group = group_index[country];
                            for &(span_first, span_last) in spans {
                                for &(seg_first, seg_last) in &segments {
                                    let a = seg_first.max(span_first);
                                    let b = seg_last.min(span_last);
                                    if a > b {
                                        continue;
                                    }
                                    for run in self.query.holders.within(country, a, b) {
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
                        }
                    }
                    patterns = out;
                }
                _ => return Err(invalid(format!("The facet {} is filtered by values or by tags", filter.name))),
            }
        }
        Ok(Selection::new(patterns, groups, overlap))
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

    fn facet_of_key_and_state(&self, key_val: u128, view: &TableView<'_>, slot: u32) -> u128 {
        let mut key = 0u128;
        for (at, facet) in self.facets.iter().enumerate().take(4) {
            let value = match *facet {
                Facet::Column(column) => view.number(column, slot).map_or(u32::MAX, |value| value as u32),
                Facet::Key(shift) => (key_val >> shift) as u32,
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

    fn cells_from_log_state(rows: &TableState, kinds: &[replay_columnar::state::Kind], recorded: usize, columns: usize, selected: &[usize], slot: u32, strings: &HashMap<u32, Vec<u8>>) -> Vec<Cell> {
        let empty = Strings::default();
        let view = TableView::new(rows, &kinds[..recorded], &empty);
        (0..columns)
            .map(|column| {
                if column >= recorded || !selected.contains(&column) {
                    return Cell::Unrecorded;
                }
                match rows.value(column, slot) {
                    Value::Number(_) => view.number(column, slot).map_or(Cell::Unrecorded, Cell::Number),
                    Value::Missing => Cell::Missing,
                    Value::Unrecorded => Cell::Unrecorded,
                    Value::Text(id) => strings.get(&id).map_or(Cell::Unrecorded, |bytes| Cell::Text(String::from_utf8_lossy(bytes).into_owned())),
                    Value::List(bytes) => Cell::List(bytes.to_vec()),
                }
            })
            .collect()
    }

    fn bucket_key(&self, run: u32, bucket: i64) -> BucketKey {
        let rev = self.query.replay.recording().revision();
        (rev.number, rev.length, rev.root, run, self.log.table.index(), bucket)
    }

    fn check_table_coverage(&self, run: u32, first_ordinal: u32) -> io::Result<bool> {
        let recording = self.query.replay.recording();
        let index = self.log.table.index();
        if recording.predates(index).is_some() {
            return Ok(true);
        }
        match recording.table_coverage(index, run) {
            TableCoverage::Valid => Ok(false),
            TableCoverage::WithheldAll(reason) => Err(Damage::wrap(DamageScope::Withheld { table: index as u32 }, invalid(reason.to_owned()))),
            TableCoverage::WithheldBefore { ordinal, reason } => {
                if first_ordinal < ordinal {
                    Err(Damage::wrap(DamageScope::Withheld { table: index as u32 }, invalid(reason.to_owned())))
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn is_log_table(&self, run: u32) -> io::Result<bool> {
        let recording = self.query.replay.recording();
        let index = self.log.table.index();
        if recording.predates(index).is_some() {
            return Ok(false);
        }
        let held = recording.run(run)?;
        if held.ordinals == 0 {
            return Ok(false);
        }
        let pages = crate::cost::storage(|| recording.table_pages(run, index, 0, 1))?;
        unit::reader::class_of(&pages, index)
    }

    fn load_buckets(&self, run: u32, ranges: &[(i32, i32)]) -> io::Result<Vec<(i64, Arc<Vec<BucketEntry>>)>> {
        let six = bucket_hours(0);
        let mut buckets: Vec<i64> = ranges.iter().flat_map(|&(a, b)| i64::from(a).div_euclid(six)..=i64::from(b).div_euclid(six)).collect();
        buckets.sort_unstable();
        buckets.dedup();
        let mut missing: Vec<i64> = Vec::new();
        {
            let mut walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
            let shared_guard = self.shared.as_ref().map(|cache| cache.lock().unwrap_or_else(PoisonError::into_inner));
            for &bucket in &buckets {
                if walked.contains_key(&(run, bucket)) {
                    continue;
                }
                if let Some(ref cache) = shared_guard
                    && let Some(hit) = cache.get(&self.bucket_key(run, bucket))
                {
                    walked.insert((run, bucket), hit);
                    continue;
                }
                missing.push(bucket);
            }
        }
        if !missing.is_empty() {
            let mut found: HashMap<i64, Vec<BucketEntry>> = missing.iter().map(|&bucket| (bucket, Vec::new())).collect();
            self.index_walk(run, &joined(&missing), |entry| {
                if let Some(held) = found.get_mut(&i64::from(entry.hour).div_euclid(six)) {
                    held.push(entry);
                }
                Ok(())
            })?;
            let mut walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
            let mut shared_guard = self.shared.as_ref().map(|cache| cache.lock().unwrap_or_else(PoisonError::into_inner));
            for (bucket, mut entries) in found {
                entries.sort_unstable_by_key(|entry| (entry.hour, entry.key));
                let arc = Arc::new(entries);
                if let Some(ref mut cache) = shared_guard {
                    cache.insert(self.bucket_key(run, bucket), Arc::clone(&arc));
                }
                walked.insert((run, bucket), arc);
            }
        }
        let walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(buckets
            .into_iter()
            .filter_map(|bucket| walked.get(&(run, bucket)).map(|entries| (bucket, Arc::clone(entries))))
            .collect())
    }

    fn visit_bucket_ranges(&self, run: u32, ranges: &[(i32, i32)], mut visit: impl FnMut(BucketEntry)) -> io::Result<()> {
        let six = bucket_hours(0);
        for (bucket, entries) in self.load_buckets(run, ranges)? {
            let bucket_first = (bucket * six) as i32;
            let bucket_last = ((bucket + 1) * six - 1) as i32;
            for &(a, b) in ranges {
                let lo_h = a.max(bucket_first);
                let hi_h = b.min(bucket_last);
                if lo_h > hi_h {
                    continue;
                }
                let lo = entries.partition_point(|entry| entry.hour < lo_h);
                let hi = entries.partition_point(|entry| entry.hour <= hi_h);
                record_examined(hi - lo);
                for &entry in &entries[lo..hi] {
                    visit(entry);
                }
            }
        }
        Ok(())
    }

    fn index_buckets(&self, run: u32, ranges: &[(i32, i32)]) -> io::Result<Vec<BucketEntry>> {
        let mut out = Vec::new();
        self.visit_bucket_ranges(run, ranges, |entry| out.push(entry))?;
        Ok(out)
    }

    fn index_walk(&self, run: u32, spans: &[(i64, i64)], mut visit: impl FnMut(BucketEntry) -> io::Result<()>) -> io::Result<()> {
        let six = bucket_hours(0);
        let (run_first, run_last) = self.run_hours(run)?;
        let recording = self.query.replay.recording();
        let index = self.log.table.index();
        if self.is_log_table(run)? {
            let held = recording.run(run)?;
            let kinds = replay_recording::schema::schema()?.swap_remove(index);
            let recorded = recording.columns()[index].min(kinds.len());
            let facet_cols = read_columns(recording, recorded, index, Some(&self.facet_columns));
            let empty = Strings::default();
            for &(first, last) in spans {
                let (from, to) = ((first * six).max(run_first) as i32, ((last + 1) * six - 1).min(run_last) as i32);
                if from > to {
                    continue;
                }
                let first_ord = read::ordinal(recording, run, from)?;
                let last_ord = read::ordinal(recording, run, to)?;
                if self.check_table_coverage(run, first_ord)? {
                    continue;
                }
                let first_block = held.place(first_ord).0;
                let last_block = held.place(last_ord).0;
                let pages = crate::cost::storage(|| recording.table_pages(run, index, first_block.saturating_sub(1), last_block + 1))?;
                let mut current_hour: Option<i32> = None;
                let mut hour_entries: Vec<BucketEntry> = Vec::new();
                let mut failed: Option<io::Error> = None;
                let flush = |entries: &mut Vec<BucketEntry>, visit: &mut dyn FnMut(BucketEntry) -> io::Result<()>, failed: &mut Option<io::Error>| {
                    for entry in entries.drain(..) {
                        if failed.is_none()
                            && let Err(err) = visit(entry)
                        {
                            *failed = Some(err);
                        }
                    }
                };
                crate::cost::storage(|| {
                    unit::reader::log_span(recording.extents(), &pages, &kinds[..recorded], index, &facet_cols, held.first_hour, from..=to, &mut |hour, event| {
                        if failed.is_some() {
                            return;
                        }
                        if current_hour != Some(hour) {
                            flush(&mut hour_entries, &mut visit, &mut failed);
                            current_hour = Some(hour);
                        }
                        match event {
                            LogEvent::Appended { ordinal, key, rows, at, .. } => {
                                let view = TableView::new(rows, &kinds[..recorded], &empty);
                                let facet = self.facet_of_key_and_state(key, &view, at);
                                hour_entries.push(BucketEntry { hour, key, facet, log_ordinal: Some(ordinal) });
                            }
                            LogEvent::Forgotten { from: f, to: t, .. } => {
                                hour_entries.retain(|entry| entry.log_ordinal.is_none_or(|ord| !(f..t).contains(&ord)));
                            }
                            LogEvent::Named { .. } => {}
                        }
                    })
                })?;
                flush(&mut hour_entries, &mut visit, &mut failed);
                if let Some(err) = failed {
                    return Err(err);
                }
            }
            return Ok(());
        }
        let mut cursor = self.facet_cursor.lock().unwrap_or_else(PoisonError::into_inner);
        let mut reader = match cursor.take() {
            Some((held, reader)) if held == run => reader,
            _ => {
                let mut reader = crate::cost::storage(|| TableReader::new(recording, run, self.log.table, Some(&self.facet_columns)))?;
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
                    visit(BucketEntry { hour, key, facet: self.facet_of(&view, slot), log_ordinal: None })?;
                }
                before = now;
            }
        }
        *cursor = Some((run, reader));
        Ok(())
    }

    fn materialize(&self, run: u32, entries: &[BucketEntry]) -> io::Result<Vec<Row>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let recording = self.query.replay.recording();
        let columns = self.log.table.field_count();
        let index = self.log.table.index();
        if entries.iter().all(|entry| entry.log_ordinal.is_some()) {
            let held = recording.run(run)?;
            let kinds = replay_recording::schema::schema()?.swap_remove(index);
            let recorded = recording.columns()[index].min(kinds.len());
            let all_cols = read_columns(recording, recorded, index, None);
            let mut ordered: Vec<(u64, usize)> = entries.iter().enumerate().map(|(idx, entry)| (entry.log_ordinal.unwrap(), idx)).collect();
            ordered.sort_unstable_by_key(|&(ord, _)| ord);
            let mut unique_ords: Vec<u64> = Vec::with_capacity(ordered.len());
            for &(ord, _) in &ordered {
                if unique_ords.last() != Some(&ord) {
                    unique_ords.push(ord);
                }
            }
            let min_hour = entries.iter().map(|entry| entry.hour).min().unwrap();
            let max_hour = entries.iter().map(|entry| entry.hour).max().unwrap();
            let first_ord = read::ordinal(recording, run, min_hour)?;
            let last_ord = read::ordinal(recording, run, max_hour)?;
            let first_block = held.place(first_ord).0;
            let last_block = held.place(last_ord).0;
            let pages = crate::cost::storage(|| recording.table_pages(run, index, first_block.saturating_sub(1), last_block + 1))?;
            let mut strings: HashMap<u32, Vec<u8>> = HashMap::new();
            let mut by_ord: HashMap<u64, Vec<Cell>> = HashMap::with_capacity(unique_ords.len());
            crate::cost::storage(|| {
                unit::reader::log_rows_at(recording.extents(), &pages, &kinds[..recorded], index, &all_cols, &unique_ords, &mut |event| match event {
                    LogEvent::Named { id, text } => {
                        strings.insert(id, text.to_vec());
                    }
                    LogEvent::Appended { ordinal, rows, at, .. } => {
                        by_ord.insert(ordinal, Self::cells_from_log_state(rows, &kinds, recorded, columns, &all_cols, at, &strings));
                    }
                    LogEvent::Forgotten { .. } => {}
                })
            })?;
            let mut out = Vec::with_capacity(entries.len());
            for entry in entries {
                let ord = entry.log_ordinal.unwrap();
                let cells = by_ord.get(&ord).cloned().ok_or_else(|| invalid(format!("Log {} did not return ordinal {ord}", self.log.name)))?;
                out.push(Row { hour: entry.hour, key: entry.key, facet: entry.facet, cells });
            }
            return Ok(out);
        }
        let mut cursor = self.cursor.lock().unwrap_or_else(PoisonError::into_inner);
        let mut reader = match cursor.take() {
            Some((held, reader)) if held == run => reader,
            _ => {
                let mut reader = crate::cost::storage(|| TableReader::new(recording, run, self.log.table, None))?;
                reader.index_keys();
                reader
            }
        };
        let mut by_hour: BTreeMap<i32, Vec<(u128, usize)>> = BTreeMap::new();
        for (idx, entry) in entries.iter().enumerate() {
            by_hour.entry(entry.hour).or_default().push((entry.key, idx));
        }
        let mut out: Vec<Option<Row>> = vec![None; entries.len()];
        for (hour, asks) in by_hour {
            crate::cost::storage(|| reader.go_to(recording, read::ordinal(recording, run, hour)?))?;
            let view = reader.view();
            for (key, idx) in asks {
                let entry = entries[idx];
                let cells = view.slot(key).map_or_else(|| vec![Cell::Unrecorded; columns], |slot| Self::cells_of(&view, columns, slot));
                out[idx] = Some(Row { hour: entry.hour, key: entry.key, facet: entry.facet, cells });
            }
        }
        *cursor = Some((run, reader));
        Ok(out.into_iter().map(Option::unwrap).collect())
    }

    fn logged(&self, run: u32, ranges: &[(i32, i32)]) -> io::Result<Vec<Row>> {
        let entries = self.index_buckets(run, ranges)?;
        self.materialize(run, &entries)
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
        self.rows_with(run, patterns, ranges, Overlap::EachPattern)
    }

    pub fn rows_with(&self, run: u32, patterns: &[Pattern], ranges: &[(i32, i32)], overlap: Overlap) -> io::Result<Vec<(usize, Row)>> {
        let mut mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
        if mine.is_empty() {
            return Ok(Vec::new());
        }
        mine.sort_unstable_by_key(|&at| (patterns[at].group, at));
        let mut matched: Vec<(usize, BucketEntry)> = Vec::new();
        self.visit_bucket_ranges(run, ranges, |entry| match overlap {
            Overlap::FirstMatch => {
                if let Some(at) = canonical_match_at(patterns, &mine, entry.hour, entry.facet) {
                    matched.push((at, entry));
                }
            }
            Overlap::EachPattern => {
                for &at in &mine {
                    let p = &patterns[at];
                    if p.first <= entry.hour && entry.hour <= p.last && p.holds(entry.facet) {
                        matched.push((at, entry));
                    }
                }
            }
        })?;
        matched.sort_unstable_by_key(|(at, entry)| (entry.hour, patterns[*at].group, entry.key));
        let entries: Vec<BucketEntry> = matched.iter().map(|&(_, entry)| entry).collect();
        let rows = self.materialize(run, &entries)?;
        Ok(matched.into_iter().map(|(at, _)| at).zip(rows).collect())
    }

    pub fn every(&self, patterns: &[Pattern], from: i32, to: i32) -> io::Result<Vec<(usize, Row)>> {
        self.every_with(patterns, from, to, Overlap::EachPattern)
    }

    pub fn every_with(&self, patterns: &[Pattern], from: i32, to: i32, overlap: Overlap) -> io::Result<Vec<(usize, Row)>> {
        let six = bucket_hours(0);
        let mut by_run: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
        for (at, bins) in self.bins_with(patterns, from, to, six, overlap)?.into_iter().enumerate() {
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
            out.extend(self.rows_with(run, patterns, &merged, overlap)?);
        }
        out.sort_by_key(|(at, row)| (row.hour, patterns[*at].group, row.key));
        Ok(out)
    }

    pub fn bins(&self, patterns: &[Pattern], from: i32, to: i32, bin: i64) -> io::Result<Vec<Vec<(i64, u64)>>> {
        self.bins_with(patterns, from, to, bin, Overlap::EachPattern)
    }

    pub fn bins_with(&self, patterns: &[Pattern], from: i32, to: i32, bin: i64, overlap: Overlap) -> io::Result<Vec<Vec<(i64, u64)>>> {
        let six = bucket_hours(0);
        let mut out: Vec<Vec<(i64, u64)>> = vec![Vec::new(); patterns.len()];
        if from > to || patterns.is_empty() {
            return Ok(out);
        }
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
        let mut by_run: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
        let mut runs: HashMap<u32, (i64, i64)> = HashMap::new();
        for (at, pattern) in patterns.iter().enumerate() {
            record_patterns_checked(1);
            let (run_first, run_last) = match runs.get(&pattern.run) {
                Some(&held) => held,
                None => *runs.entry(pattern.run).or_insert(self.run_hours(pattern.run)?),
            };
            let a = i64::from(from.max(pattern.first)).max(run_first);
            let b = i64::from(to.min(pattern.last)).min(run_last);
            if a <= b {
                by_run.entry(pattern.run).or_default().push((at, a, b));
            }
        }
        let recording = self.query.replay.recording();
        match overlap {
            Overlap::EachPattern => {
                let mut coarse: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
                let mut fine: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
                let mut slivers: BTreeMap<u32, Vec<(usize, i64, i64)>> = BTreeMap::new();
                for (run, held) in by_run {
                    let (run_first, run_last) = runs[&run];
                    for (at, mut a, mut b) in held {
                        if a == run_first {
                            a = a.div_euclid(bin) * bin;
                        }
                        if b == run_last {
                            b = (b.div_euclid(bin) + 1) * bin - 1;
                        }
                        let (whole, edges) = split(a, b, bin);
                        if let Some((first, last)) = whole {
                            coarse.entry(run).or_default().push((at, first, last));
                        }
                        for (x, y) in edges {
                            let (middle, ends) = split(x, y, six);
                            if let Some((first, last)) = middle {
                                fine.entry(run).or_default().push((at, first, last));
                            }
                            for (first, last) in ends {
                                slivers.entry(run).or_default().push((at, first, last));
                            }
                        }
                    }
                }
                for (spans, size) in [(&coarse, bin), (&fine, six)] {
                    for (&run, held) in spans {
                        let (first, last) = (held.iter().map(|span| span.1).min().unwrap_or(0), held.iter().map(|span| span.2).max().unwrap_or(0));
                        let (run_first, run_last) = runs[&run];
                        let mut unique_ats: Vec<usize> = held.iter().map(|&(at, _, _)| at).collect();
                        unique_ats.sort_unstable();
                        unique_ats.dedup();
                        let asked = |facet: u128| unique_ats.iter().any(|&at| patterns[at].holds(facet));
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
                    let mut unfiltered: Vec<(usize, i64, i64)> = Vec::new();
                    let mut filtered: Vec<(usize, i64, i64)> = Vec::new();
                    for &(at, a, b) in held {
                        if patterns[at].mask == 0 && patterns[at].value == 0 {
                            unfiltered.push((at, a, b));
                        } else {
                            filtered.push((at, a, b));
                        }
                    }
                    if !unfiltered.is_empty() {
                        let mut ranges: Vec<(i32, i32)> = unfiltered.iter().map(|&(_, a, b)| (a as i32, b as i32)).collect();
                        ranges.sort_unstable();
                        let mut merged: Vec<(i32, i32)> = Vec::new();
                        for (a, b) in ranges {
                            match merged.last_mut() {
                                Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                                _ => merged.push((a, b)),
                            }
                        }
                        let loaded = self.load_buckets(run, &merged)?;
                        for (at, a, b) in unfiltered {
                            let (a32, b32) = (a as i32, b as i32);
                            for &(bucket, ref entries) in &loaded {
                                let bucket_start = bucket * six;
                                let bucket_end = bucket_start + six - 1;
                                if bucket_end < a || bucket_start > b {
                                    continue;
                                }
                                let lo = entries.partition_point(|e| e.hour < a32);
                                let hi = entries.partition_point(|e| e.hour <= b32);
                                let count = (hi - lo) as u64;
                                if count > 0 {
                                    out[at].push((bucket_start.div_euclid(bin) * bin, count));
                                }
                            }
                        }
                    }
                    if !filtered.is_empty() {
                        let (run_first, run_last) = runs[&run];
                        let index = FacetIndex::new(patterns, &filtered);
                        let mut ranges: Vec<(i32, i32)> = filtered.iter().map(|&(_, a, b)| (a as i32, b as i32)).collect();
                        ranges.sort_unstable();
                        let mut merged: Vec<(i32, i32)> = Vec::new();
                        for (a, b) in ranges {
                            match merged.last_mut() {
                                Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                                _ => merged.push((a, b)),
                            }
                        }
                        let mut non_empty_buckets: HashSet<i64> = HashSet::new();
                        if let (Some(&(first_r, _)), Some(&(_, last_r))) = (merged.first(), merged.last()) {
                            let first_six = i64::from(first_r).div_euclid(six) * six;
                            let last_six_end = (i64::from(last_r).div_euclid(six) + 1) * six - 1;
                            let q_from = first_six.max(run_first) as i32;
                            let q_to = last_six_end.min(run_last) as i32;
                            if q_from <= q_to {
                                let asked = |facet: u128| index.asked(facet);
                                let counts = pieces::counts(recording, run, self.log.table, (q_from, q_to), six, &asked)?;
                                for (_, six_counts) in &counts {
                                    for (si, &count) in six_counts.iter().enumerate() {
                                        if count > 0 {
                                            non_empty_buckets.insert(first_six + si as i64 * six);
                                        }
                                    }
                                }
                            }
                        }
                        let mut kept_ranges: Vec<(i32, i32)> = Vec::new();
                        for (a, b) in merged {
                            let first_bk = i64::from(a).div_euclid(six);
                            let last_bk = i64::from(b).div_euclid(six);
                            for bk in first_bk..=last_bk {
                                let bk_start = bk * six;
                                if !non_empty_buckets.contains(&bk_start) {
                                    continue;
                                }
                                let lo = a.max(bk_start as i32);
                                let hi = b.min((bk_start + six - 1) as i32);
                                if lo <= hi {
                                    match kept_ranges.last_mut() {
                                        Some(last) if last.1 + 1 >= lo => last.1 = last.1.max(hi),
                                        _ => kept_ranges.push((lo, hi)),
                                    }
                                }
                            }
                        }
                        if !kept_ranges.is_empty() {
                            let mut buf: Vec<usize> = Vec::new();
                            self.visit_bucket_ranges(run, &kept_ranges, |entry| {
                                let hour = i64::from(entry.hour);
                                index.matching(entry.facet, &mut buf);
                                for &idx in &buf {
                                    let (at, a, b) = filtered[idx];
                                    if a <= hour && hour <= b {
                                        out[at].push((hour.div_euclid(bin) * bin, 1));
                                    }
                                }
                            })?;
                        }
                    }
                }
            }
            Overlap::FirstMatch => {
                for (run, mut held) in by_run {
                    held.sort_unstable_by_key(|&(at, _, _)| (patterns[at].group, at));
                    let index = FacetIndex::new(patterns, &held);
                    let (run_first, run_last) = runs[&run];
                    let min_a = held.iter().map(|span| span.1).min().unwrap_or(0);
                    let max_b = held.iter().map(|span| span.2).max().unwrap_or(0);
                    let mut buf: Vec<usize> = Vec::new();
                    let mut split_coarse: HashMap<u128, HashSet<i64>> = HashMap::new();
                    let mut split_coarse_bins: BTreeSet<i64> = BTreeSet::new();
                    let mut split_six: HashMap<u128, HashSet<i64>> = HashMap::new();
                    let mut sliver_ranges: Vec<(i32, i32)> = Vec::new();

                    if bin > six {
                        let first_bin = min_a.div_euclid(bin) * bin;
                        let last_bin_end = (max_b.div_euclid(bin) + 1) * bin - 1;
                        let q_from = first_bin.max(run_first) as i32;
                        let q_to = last_bin_end.min(run_last) as i32;
                        if q_from <= q_to {
                            let asked = |facet: u128| index.asked(facet);
                            let counts = pieces::counts(recording, run, self.log.table, (q_from, q_to), bin, &asked)?;
                            for (facet, bin_counts) in &counts {
                                index.matching(*facet, &mut buf);
                                for (bi, &count) in bin_counts.iter().enumerate() {
                                    if count == 0 {
                                        continue;
                                    }
                                    let bin_start = first_bin + bi as i64 * bin;
                                    let bin_end = bin_start + bin - 1;
                                    let b_lo = bin_start.max(run_first);
                                    let b_hi = bin_end.min(run_last);
                                    let eff_lo = b_lo.max(i64::from(from));
                                    let eff_hi = b_hi.min(i64::from(to));
                                    if eff_lo > eff_hi {
                                        continue;
                                    }
                                    let unclipped = b_lo >= i64::from(from) && b_hi <= i64::from(to);
                                    let winner = buf.iter().copied().find(|&idx| {
                                        record_patterns_checked(1);
                                        held[idx].1 <= eff_hi && held[idx].2 >= eff_lo
                                    });
                                    let Some(hi_idx) = winner else { continue };
                                    let (win_at, win_a, win_b) = held[hi_idx];
                                    if unclipped && win_a <= b_lo && b_hi <= win_b {
                                        out[win_at].push((bin_start, count));
                                    } else {
                                        split_coarse.entry(*facet).or_default().insert(bin_start);
                                        split_coarse_bins.insert(bin_start);
                                    }
                                }
                            }
                        }
                    }

                    let six_ranges: Vec<(i64, i64)> = if bin == six {
                        vec![(min_a.div_euclid(six) * six, (max_b.div_euclid(six) + 1) * six - 1)]
                    } else {
                        let mut merged: Vec<(i64, i64)> = Vec::new();
                        for start in split_coarse_bins {
                            let end = start + bin - 1;
                            match merged.last_mut() {
                                Some(last) if last.1 + 1 >= start => last.1 = last.1.max(end),
                                _ => merged.push((start, end)),
                            }
                        }
                        merged
                    };

                    for (r_start, r_end) in six_ranges {
                        let first_six = r_start.div_euclid(six) * six;
                        let q_from = first_six.max(run_first) as i32;
                        let q_to = r_end.min(run_last) as i32;
                        if q_from > q_to {
                            continue;
                        }
                        let asked = |facet: u128| if bin == six { index.asked(facet) } else { split_coarse.contains_key(&facet) };
                        let counts = pieces::counts(recording, run, self.log.table, (q_from, q_to), six, &asked)?;
                        for (facet, six_counts) in &counts {
                            index.matching(*facet, &mut buf);
                            for (si, &count) in six_counts.iter().enumerate() {
                                if count == 0 {
                                    continue;
                                }
                                let six_start = first_six + si as i64 * six;
                                if bin > six && !split_coarse.get(facet).is_some_and(|set| set.contains(&(six_start.div_euclid(bin) * bin))) {
                                    continue;
                                }
                                let six_end = six_start + six - 1;
                                let b_lo = six_start.max(run_first);
                                let b_hi = six_end.min(run_last);
                                let eff_lo = b_lo.max(i64::from(from));
                                let eff_hi = b_hi.min(i64::from(to));
                                if eff_lo > eff_hi {
                                    continue;
                                }
                                let unclipped = b_lo >= i64::from(from) && b_hi <= i64::from(to);
                                let winner = buf.iter().copied().find(|&idx| {
                                    record_patterns_checked(1);
                                    held[idx].1 <= eff_hi && held[idx].2 >= eff_lo
                                });
                                let Some(hi_idx) = winner else { continue };
                                let (win_at, win_a, win_b) = held[hi_idx];
                                if unclipped && win_a <= b_lo && b_hi <= win_b {
                                    out[win_at].push((six_start.div_euclid(bin) * bin, count));
                                } else {
                                    split_six.entry(*facet).or_default().insert(six_start);
                                    sliver_ranges.push((eff_lo as i32, eff_hi as i32));
                                }
                            }
                        }
                    }

                    if !split_six.is_empty() {
                        sliver_ranges.sort_unstable();
                        let mut merged: Vec<(i32, i32)> = Vec::new();
                        for (a, b) in sliver_ranges {
                            match merged.last_mut() {
                                Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                                _ => merged.push((a, b)),
                            }
                        }
                        self.visit_bucket_ranges(run, &merged, |entry| {
                            let hour = i64::from(entry.hour);
                            let six_start = hour.div_euclid(six) * six;
                            if !split_six.get(&entry.facet).is_some_and(|set| set.contains(&six_start)) {
                                return;
                            }
                            index.matching(entry.facet, &mut buf);
                            if let Some(&hi_idx) = buf.iter().find(|&&idx| {
                                record_patterns_checked(1);
                                held[idx].1 <= hour && hour <= held[idx].2
                            }) {
                                out[held[hi_idx].0].push((hour.div_euclid(bin) * bin, 1));
                            }
                        })?;
                    }
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
        self.counts_with(patterns, groups, first, last, bin_hours, Overlap::EachPattern)
    }

    pub fn counts_with(&self, patterns: &[Pattern], groups: usize, first: i32, last: i32, bin_hours: i64, overlap: Overlap) -> io::Result<Vec<Counts>> {
        if bin_hours < 1 || (bin_hours >= bucket_hours(0) && bin_hours % bucket_hours(0) != 0) {
            return Err(invalid("A log's bins are 1 to 5 hours or whole six-hour buckets"));
        }
        let timeline_first = self.query.timeline.first;
        let mut out = vec![Counts { before: 0, bins: Vec::new() }; groups];
        if first > timeline_first {
            let coarse = bin_for(i64::from(first) - i64::from(timeline_first), 256);
            for (at, bins) in self.bins_with(patterns, timeline_first, first - 1, coarse, overlap)?.into_iter().enumerate() {
                out[patterns[at].group].before += bins.iter().map(|(_, count)| count).sum::<u64>();
            }
        }
        let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
        let bin_of = |hour: i64| ((hour - first_bin).div_euclid(bin_hours)) as u32;
        let mut binned: Vec<BTreeMap<u32, u64>> = vec![BTreeMap::new(); groups];
        if bin_hours >= bucket_hours(0) {
            for (at, bins) in self.bins_with(patterns, first, last, bin_hours, overlap)?.into_iter().enumerate() {
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
            for (run, mut ranges) in runs {
                ranges.sort_unstable();
                let mut merged: Vec<(i32, i32)> = Vec::new();
                for (a, b) in ranges {
                    match merged.last_mut() {
                        Some(last) if last.1 + 1 >= a => last.1 = last.1.max(b),
                        _ => merged.push((a, b)),
                    }
                }
                let mut mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
                mine.sort_unstable_by_key(|&at| (patterns[at].group, at));
                self.visit_bucket_ranges(run, &merged, |entry| {
                    if !(first <= entry.hour && entry.hour <= last) {
                        return;
                    }
                    match overlap {
                        Overlap::FirstMatch => {
                            if let Some(at) = canonical_match_at(patterns, &mine, entry.hour, entry.facet) {
                                *binned[patterns[at].group].entry(bin_of(i64::from(entry.hour))).or_default() += 1;
                            }
                        }
                        Overlap::EachPattern => {
                            for &at in &mine {
                                let p = &patterns[at];
                                if p.first <= entry.hour && entry.hour <= p.last && p.holds(entry.facet) {
                                    *binned[p.group].entry(bin_of(i64::from(entry.hour))).or_default() += 1;
                                }
                            }
                        }
                    }
                })?;
            }
        }
        for (counts, bins) in out.iter_mut().zip(binned) {
            counts.bins = bins.into_iter().collect();
        }
        Ok(out)
    }

    fn slice(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, overlap: Overlap, window: impl FnOnce(u64, u64) -> (u64, u64)) -> io::Result<Page> {
        if from > to {
            return Err(invalid("Invalid log page"));
        }
        let first_hour = self.query.timeline.first;
        let bin = bin_for(i64::from(to) - i64::from(first_hour) + 1, 256);
        let mut before = vec![0u64; groups];
        if from > first_hour {
            for (at, bins) in self.bins_with(patterns, first_hour, from - 1, bin, overlap)?.into_iter().enumerate() {
                before[patterns[at].group] += bins.iter().map(|(_, count)| count).sum::<u64>();
            }
        }
        let mut bins: BTreeMap<i64, Vec<(usize, u64)>> = BTreeMap::new();
        for (at, held) in self.bins_with(patterns, from, to, bin, overlap)?.into_iter().enumerate() {
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
                let mut buckets: BTreeMap<i64, u64> = BTreeMap::new();
                for counts in self.bins_with(patterns, range.0, range.1, six, overlap)? {
                    for (bucket, count) in counts {
                        *buckets.entry(bucket).or_default() += count;
                    }
                }
                let (skip, reach) = page_buckets(&buckets, ordinal, first, end, range);
                if skip > 0 {
                    for (at, counts) in self.bins_with(patterns, range.0, reach[0].0 - 1, bin, overlap)?.into_iter().enumerate() {
                        own[patterns[at].group] += counts.iter().map(|(_, count)| count).sum::<u64>();
                    }
                    ordinal += skip;
                }
                let mut runs: Vec<u32> = held.iter().map(|&(at, _)| patterns[at].run).collect();
                runs.sort_unstable();
                runs.dedup();
                #[derive(Clone, Copy)]
                struct Matched {
                    hour: i32,
                    group: usize,
                    key: u128,
                    run: u32,
                    entry: BucketEntry,
                }
                let mut matched: Vec<Matched> = Vec::new();
                for run in runs {
                    let mut mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
                    mine.sort_unstable_by_key(|&at| (patterns[at].group, at));
                    self.visit_bucket_ranges(run, &reach, |entry| match overlap {
                        Overlap::FirstMatch => {
                            if let Some(at) = canonical_match_at(patterns, &mine, entry.hour, entry.facet) {
                                matched.push(Matched { hour: entry.hour, group: patterns[at].group, key: entry.key, run, entry });
                            }
                        }
                        Overlap::EachPattern => {
                            for &at in &mine {
                                let p = &patterns[at];
                                if p.first <= entry.hour && entry.hour <= p.last && p.holds(entry.facet) {
                                    matched.push(Matched { hour: entry.hour, group: p.group, key: entry.key, run, entry });
                                }
                            }
                        }
                    })?;
                }
                let slice_lo = first.saturating_sub(ordinal) as usize;
                let slice_hi = end.saturating_sub(ordinal) as usize;
                if slice_lo >= matched.len() {
                    for item in &matched {
                        own[item.group] += 1;
                    }
                    ordinal += matched.len() as u64;
                    continue;
                }
                let order_key = |m: &Matched| (m.hour, m.group, m.key);
                if slice_lo > 0 {
                    matched.select_nth_unstable_by_key(slice_lo, order_key);
                    for item in &matched[..slice_lo] {
                        own[item.group] += 1;
                    }
                    ordinal += slice_lo as u64;
                }
                let tail = &mut matched[slice_lo..];
                let take = (slice_hi - slice_lo).min(tail.len());
                if take < tail.len() {
                    tail.select_nth_unstable_by_key(take, order_key);
                }
                let chosen = &mut tail[..take];
                chosen.sort_unstable_by_key(order_key);
                let mut by_run: BTreeMap<u32, Vec<(usize, BucketEntry)>> = BTreeMap::new();
                for (pos, item) in chosen.iter().enumerate() {
                    by_run.entry(item.run).or_default().push((pos, item.entry));
                }
                let mut materialized: Vec<Option<Row>> = vec![None; chosen.len()];
                for (run, asks) in by_run {
                    let entries: Vec<BucketEntry> = asks.iter().map(|&(_, entry)| entry).collect();
                    let decoded = self.materialize(run, &entries)?;
                    for ((pos, _), row) in asks.into_iter().zip(decoded) {
                        materialized[pos] = Some(row);
                    }
                }
                for (item, row) in chosen.iter().zip(materialized.into_iter().map(Option::unwrap)) {
                    rows.push(Paged { row, group: item.group, ordinal, own: own[item.group] });
                    own[item.group] += 1;
                    ordinal += 1;
                }
                let remaining = tail.len() - take;
                if remaining > 0 {
                    for item in &tail[take..] {
                        own[item.group] += 1;
                    }
                    ordinal += remaining as u64;
                }
            }
        }
        Ok(Page { base, first, end, last, rows })
    }

    pub fn page(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, cursor: u64, size: u32) -> io::Result<Page> {
        self.page_with(patterns, groups, from, to, cursor, size, Overlap::EachPattern)
    }

    pub fn page_with(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, cursor: u64, size: u32, overlap: Overlap) -> io::Result<Page> {
        self.slice(patterns, groups, from, to, overlap, |base, last| {
            let first = cursor.max(base);
            let end = first.saturating_add(u64::from(size)).min(last);
            (first, end)
        })
    }

    pub fn newest_page_with(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, offset: u64, limit: u32, overlap: Overlap) -> io::Result<NewestPage> {
        if from > to {
            return Err(invalid("Invalid log page"));
        }
        let first_hour = self.query.timeline.first;
        let bin = bin_for(i64::from(to) - i64::from(first_hour) + 1, 256);
        let mut before = vec![0u64; groups];
        if from > first_hour {
            for (at, bins) in self.bins_with(patterns, first_hour, from - 1, bin, overlap)?.into_iter().enumerate() {
                before[patterns[at].group] += bins.iter().map(|(_, count)| count).sum::<u64>();
            }
        }
        let mut bins: BTreeMap<i64, Vec<(usize, u64)>> = BTreeMap::new();
        let mut group_total = before.clone();
        for (at, held) in self.bins_with(patterns, from, to, bin, overlap)?.into_iter().enumerate() {
            for (start, count) in held {
                group_total[patterns[at].group] += count;
                bins.entry(start).or_default().push((at, count));
            }
        }
        let base: u64 = before.iter().sum();
        let total: u64 = bins.values().flatten().map(|(_, count)| count).sum();
        let first = offset.min(total);
        let end = first.saturating_add(u64::from(limit)).min(total);
        if first >= end {
            return Ok(NewestPage { base, offset: first, total, rows: Vec::new() });
        }

        #[derive(Clone, Copy)]
        struct Chosen {
            run: u32,
            entry: BucketEntry,
            group: usize,
            ordinal: u64,
            own: u64,
        }

        let six = bucket_hours(0);
        let mut seen_newer = 0u64;
        let mut newer_in_group = vec![0u64; groups];
        let mut chosen: Vec<Chosen> = Vec::with_capacity((end - first) as usize);

        for (&start, held) in bins.iter().rev() {
            if seen_newer >= end {
                break;
            }
            let bin_count: u64 = held.iter().map(|(_, count)| count).sum();
            if seen_newer + bin_count <= first {
                for &(at, count) in held {
                    newer_in_group[patterns[at].group] += count;
                }
                seen_newer += bin_count;
                continue;
            }
            let range = ((start.max(i64::from(from))) as i32, ((start + bin - 1).min(i64::from(to))) as i32);
            let mut buckets: BTreeMap<i64, Vec<(usize, u64)>> = BTreeMap::new();
            if bin == six {
                buckets.insert(start, held.clone());
            } else {
                for (at, counts) in self.bins_with(patterns, range.0, range.1, six, overlap)?.into_iter().enumerate() {
                    for (bucket, count) in counts {
                        buckets.entry(bucket).or_default().push((at, count));
                    }
                }
            }
            for (&bucket, b_held) in buckets.iter().rev() {
                if seen_newer >= end {
                    break;
                }
                let b_count: u64 = b_held.iter().map(|(_, count)| count).sum();
                if seen_newer + b_count <= first {
                    for &(at, count) in b_held {
                        newer_in_group[patterns[at].group] += count;
                    }
                    seen_newer += b_count;
                    continue;
                }
                let b_from = bucket.max(i64::from(range.0)) as i32;
                let b_to = (bucket + six - 1).min(i64::from(range.1)) as i32;
                let mut runs: Vec<u32> = b_held.iter().map(|&(at, _)| patterns[at].run).collect();
                runs.sort_unstable();
                runs.dedup();

                if groups <= 1 && runs.len() == 1 && (overlap == Overlap::FirstMatch || patterns.len() <= 1) {
                    let run = runs[0];
                    let loaded = self.load_buckets(run, &[(b_from, b_to)])?;
                    if let Some((_, entries)) = loaded.first() {
                        let lo = entries.partition_point(|e| e.hour < b_from);
                        let hi = entries.partition_point(|e| e.hour <= b_to);
                        let slice = &entries[lo..hi];
                        let mut mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
                        mine.sort_unstable_by_key(|&at| (patterns[at].group, at));
                        let mut h_hi = slice.len();
                        while h_hi > 0 && seen_newer < end {
                            let hour = slice[h_hi - 1].hour;
                            let h_lo = slice[..h_hi].partition_point(|e| e.hour < hour);
                            let hour_slice = &slice[h_lo..h_hi];
                            let unfiltered_hour = mine.iter().any(|&at| {
                                let p = &patterns[at];
                                p.mask == 0 && p.value == 0 && p.first <= hour && hour <= p.last
                            });
                            if unfiltered_hour {
                                let hour_count = hour_slice.len() as u64;
                                if seen_newer + hour_count > first {
                                    let want_lo = first.saturating_sub(seen_newer) as usize;
                                    let want_hi = end.saturating_sub(seen_newer).min(hour_count) as usize;
                                    let ord_base = base + total - seen_newer - hour_count;
                                    let own_base = group_total[0] - newer_in_group[0] - hour_count;
                                    record_examined(want_hi - want_lo);
                                    for (idx_in_hour, &entry) in hour_slice[want_lo..want_hi].iter().enumerate() {
                                        let k = (want_lo + idx_in_hour) as u64;
                                        chosen.push(Chosen {
                                            run,
                                            entry,
                                            group: 0,
                                            ordinal: ord_base + k,
                                            own: own_base + k,
                                        });
                                    }
                                }
                                seen_newer += hour_count;
                                newer_in_group[0] += hour_count;
                                h_hi = h_lo;
                                continue;
                            }
                            let mut matched_entries: Vec<BucketEntry> = Vec::new();
                            for &entry in hour_slice {
                                record_examined(1);
                                let hit = match overlap {
                                    Overlap::FirstMatch => canonical_match_at(patterns, &mine, entry.hour, entry.facet).is_some(),
                                    Overlap::EachPattern => mine.iter().any(|&at| {
                                        let p = &patterns[at];
                                        p.first <= entry.hour && entry.hour <= p.last && p.holds(entry.facet)
                                    }),
                                };
                                if hit {
                                    matched_entries.push(entry);
                                }
                            }
                            let hour_count = matched_entries.len() as u64;
                            if seen_newer + hour_count > first {
                                let want_lo = first.saturating_sub(seen_newer) as usize;
                                let want_hi = end.saturating_sub(seen_newer).min(hour_count) as usize;
                                let ord_base = base + total - seen_newer - hour_count;
                                let own_base = group_total[0] - newer_in_group[0] - hour_count;
                                for (idx_in_hour, &entry) in matched_entries[want_lo..want_hi].iter().enumerate() {
                                    let k = (want_lo + idx_in_hour) as u64;
                                    chosen.push(Chosen {
                                        run,
                                        entry,
                                        group: 0,
                                        ordinal: ord_base + k,
                                        own: own_base + k,
                                    });
                                }
                            }
                            seen_newer += hour_count;
                            newer_in_group[0] += hour_count;
                            h_hi = h_lo;
                        }
                    }
                    continue;
                }

                #[derive(Clone, Copy)]
                struct Item {
                    hour: i32,
                    group: usize,
                    key: u128,
                    run: u32,
                    entry: BucketEntry,
                }
                let mut bucket_items: Vec<Item> = Vec::new();
                for run in runs {
                    let mut mine: Vec<usize> = (0..patterns.len()).filter(|&at| patterns[at].run == run).collect();
                    mine.sort_unstable_by_key(|&at| (patterns[at].group, at));
                    self.visit_bucket_ranges(run, &[(b_from, b_to)], |entry| match overlap {
                        Overlap::FirstMatch => {
                            if let Some(at) = canonical_match_at(patterns, &mine, entry.hour, entry.facet) {
                                bucket_items.push(Item { hour: entry.hour, group: patterns[at].group, key: entry.key, run, entry });
                            }
                        }
                        Overlap::EachPattern => {
                            for &at in &mine {
                                let p = &patterns[at];
                                if p.first <= entry.hour && entry.hour <= p.last && p.holds(entry.facet) {
                                    bucket_items.push(Item { hour: entry.hour, group: p.group, key: entry.key, run, entry });
                                }
                            }
                        }
                    })?;
                }
                bucket_items.sort_unstable_by_key(|item| (Reverse(item.hour), item.group, item.key));
                let mut pos = 0usize;
                while pos < bucket_items.len() && seen_newer < end {
                    let hour = bucket_items[pos].hour;
                    let mut next = pos + 1;
                    while next < bucket_items.len() && bucket_items[next].hour == hour {
                        next += 1;
                    }
                    let hour_slice = &bucket_items[pos..next];
                    let hour_count = hour_slice.len() as u64;
                    let mut hour_groups = vec![0u64; groups];
                    for item in hour_slice {
                        hour_groups[item.group] += 1;
                    }
                    if seen_newer + hour_count > first {
                        let ord_base = base + total - seen_newer - hour_count;
                        let mut group_pos = vec![0u64; groups];
                        for (idx_in_hour, item) in hour_slice.iter().enumerate() {
                            let disp = seen_newer + idx_in_hour as u64;
                            let k = group_pos[item.group];
                            group_pos[item.group] += 1;
                            if disp >= first && disp < end {
                                let own_base = group_total[item.group] - newer_in_group[item.group] - hour_groups[item.group];
                                chosen.push(Chosen {
                                    run: item.run,
                                    entry: item.entry,
                                    group: item.group,
                                    ordinal: ord_base + idx_in_hour as u64,
                                    own: own_base + k,
                                });
                            }
                        }
                    }
                    seen_newer += hour_count;
                    for g in 0..groups {
                        newer_in_group[g] += hour_groups[g];
                    }
                    pos = next;
                }
            }
        }

        let mut by_run: BTreeMap<u32, Vec<(usize, BucketEntry)>> = BTreeMap::new();
        for (pos, item) in chosen.iter().enumerate() {
            by_run.entry(item.run).or_default().push((pos, item.entry));
        }
        let mut materialized: Vec<Option<Row>> = vec![None; chosen.len()];
        for (run, asks) in by_run {
            let entries: Vec<BucketEntry> = asks.iter().map(|&(_, entry)| entry).collect();
            let decoded = self.materialize(run, &entries)?;
            for ((pos, _), row) in asks.into_iter().zip(decoded) {
                materialized[pos] = Some(row);
            }
        }
        let rows = chosen
            .into_iter()
            .zip(materialized.into_iter().map(Option::unwrap))
            .map(|(item, row)| Paged { row, group: item.group, ordinal: item.ordinal, own: item.own })
            .collect();
        Ok(NewestPage { base, offset: first, total, rows })
    }

    pub fn tail(&self, patterns: &[Pattern], groups: usize, from: i32, to: i32, limit: u32) -> io::Result<NewestPage> {
        self.newest_page_with(patterns, groups, from, to, 0, limit, Overlap::EachPattern)
    }

    pub fn total(&self, patterns: &[Pattern], from: i32, to: i32) -> io::Result<u64> {
        self.total_with(patterns, from, to, Overlap::EachPattern)
    }

    pub fn total_with(&self, patterns: &[Pattern], from: i32, to: i32, overlap: Overlap) -> io::Result<u64> {
        if from > to {
            return Ok(0);
        }
        let bin = bin_for(i64::from(to) - i64::from(from) + 1, 256);
        let mut total = 0u64;
        for held in self.bins_with(patterns, from, to, bin, overlap)? {
            total += held.iter().map(|(_, count)| count).sum::<u64>();
        }
        Ok(total)
    }

    pub fn has_any_with(&self, patterns: &[Pattern], from: i32, to: i32, overlap: Overlap) -> io::Result<bool> {
        Ok(self.total_with(patterns, from, to, overlap)? > 0)
    }

    pub fn has_any_in(&self, selection: &Selection, from: i32, to: i32) -> io::Result<bool> {
        if from > to || selection.patterns.is_empty() {
            return Ok(false);
        }
        let active_indices = selection.index.overlapping(from, to);
        if active_indices.is_empty() {
            return Ok(false);
        }
        let active: Vec<Pattern> = active_indices.into_iter().map(|at| selection.patterns[at]).collect();
        self.has_any_with(&active, from, to, selection.overlap)
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
