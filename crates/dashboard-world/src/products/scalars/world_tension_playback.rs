use super::{CachedEvent, DatedGroupFilter, Sample, SlotNames, StackInput, TensionStackEvent, lookup_slot};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque},
    ops::Bound::{Excluded, Included, Unbounded},
};

#[derive(Default)]
pub(super) struct Edits(BTreeMap<i32, Vec<u32>>);

impl Edits {
    pub(super) fn add(&mut self, hour: i32, key: u32) {
        self.0.entry(hour).or_default().push(key);
    }

    pub(super) fn next(&self, after: i32) -> Option<i32> {
        self.0.range((Excluded(after), Unbounded)).next().map(|(&hour, _)| hour)
    }

    fn crossed(&self, old: i32, new: i32, keys: &mut HashSet<u32>) {
        for (_, edited) in self.0.range((Excluded(old.min(new)), Included(old.max(new)))) {
            keys.extend(edited);
        }
    }
}

struct Peaks {
    spans: Vec<(i32, i32)>,
    tree: Vec<u64>,
}

impl Peaks {
    fn new(values: Vec<(i32, i32, u64)>) -> Self {
        let n = values.len();
        let mut tree = vec![0; n * 2];
        let spans = values
            .into_iter()
            .enumerate()
            .map(|(index, (first, last, value))| {
                tree[n + index] = value;
                (first, last)
            })
            .collect();
        for index in (1..n).rev() {
            tree[index] = tree[2 * index].max(tree[2 * index + 1]);
        }
        Self { spans, tree }
    }

    fn peak(&self, first: i32, last: i32) -> u64 {
        let n = self.spans.len();
        let mut left = n + self.spans.partition_point(|span| span.1 < first);
        let mut right = n + self.spans.partition_point(|span| span.0 <= last);
        let mut result = 0;
        while left < right {
            if left % 2 == 1 {
                result = result.max(self.tree[left]);
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                result = result.max(self.tree[right]);
            }
            left /= 2;
            right /= 2;
        }
        result
    }
}

pub(super) struct SlotIndex {
    changes: Edits,
    peaks: BTreeMap<u32, Peaks>,
}

impl SlotIndex {
    pub(super) fn new(input: &StackInput<'_>, group: Option<&DatedGroupFilter>, names: Option<&SlotNames>) -> Self {
        let mut changes = Edits::default();
        let mut peaks = BTreeMap::new();
        for (&slot, runs) in input.slots {
            let mut values = Vec::new();
            for &(first, last, value) in runs {
                changes.add(first, slot);
                if let Some(next) = last.checked_add(1) {
                    changes.add(next, slot);
                }
                if let Some(raw) = value.filter(|&value| value != 0) {
                    if let Some(group) = group {
                        for &(lo, hi) in group.spans(slot, first, last) {
                            values.push((lo.max(first), hi.min(last), raw.unsigned_abs()));
                        }
                    } else {
                        values.push((first, last, raw.unsigned_abs()));
                    }
                }
            }
            if let Some(spans) = group.and_then(|group| group.runs_by_slot.get(&slot)) {
                for &(first, last) in spans {
                    changes.add(first, slot);
                    if let Some(next) = last.checked_add(1) {
                        changes.add(next, slot);
                    }
                }
            }
            if let Some(names) = names.and_then(|names| names.0.get(&slot)) {
                for &(hour, ..) in names {
                    changes.add(hour, slot);
                }
            }
            peaks.insert(slot, Peaks::new(values));
        }
        Self { changes, peaks }
    }

    pub(super) fn plot_hours(&self, slots: &[u32], first: i32, last: i32, hours: &mut Vec<i32>) {
        for (&hour, edits) in self.changes.0.range(first..=last) {
            if edits.iter().any(|slot| {
                dashboard_base::products::at_hour::count_rows(|| 1);
                slots.binary_search(slot).is_ok()
            }) {
                hours.push(hour);
            }
        }
    }

    pub(super) fn next(&self, hour: i32) -> Option<i32> {
        self.changes.next(hour)
    }
}

type Order = (Reverse<Option<u64>>, Reverse<Option<i64>>, Reverse<u64>, u32);

#[derive(Default)]
struct Candidate {
    order: Option<Order>,
    unknown: bool,
    witnessed: bool,
    nonzero: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Window {
    pub first: i32,
    pub last: i32,
    pub rank: Option<i32>,
    pub complete: bool,
}

#[derive(Default)]
pub(super) struct Ranking {
    window: Option<Window>,
    candidates: HashMap<u32, Candidate>,
    ordered: BTreeSet<Order>,
    unknown: usize,
    witnessed: usize,
    nonzero: usize,
}

impl Ranking {
    pub(super) fn update(
        &mut self,
        index: &SlotIndex,
        input: &StackInput<'_>,
        window: Window,
        group: Option<&DatedGroupFilter>,
        filter: Option<&HashSet<&str>>,
        top: bool,
        named: &impl Fn(u32, i32) -> Option<(String, bool)>,
    ) {
        let Some(rank) = window.rank else {
            *self = Self::default();
            return;
        };
        let mut changed = HashSet::new();
        if let Some(old) = self.window.filter(|old| old.complete == window.complete) {
            index.changes.crossed(old.first, window.first, &mut changed);
            index.changes.crossed(old.last, window.last, &mut changed);
            index.changes.crossed(old.rank.unwrap(), rank, &mut changed);
        } else {
            changed.extend(input.slots.keys());
        }
        self.window = Some(window);
        for slot in changed {
            dashboard_base::products::at_hour::count_rows(|| 1);
            if let Some(old) = self.candidates.remove(&slot) {
                if let Some(order) = old.order {
                    self.ordered.remove(&order);
                }
                self.unknown -= usize::from(old.unknown);
                self.witnessed -= usize::from(old.witnessed);
                self.nonzero -= usize::from(old.nonzero);
            }
            let peaks = &index.peaks[&slot];
            let witnessed = peaks.peak(window.first, window.last) != 0;
            let within = group.is_none_or(|group| !group.spans(slot, rank, rank).is_empty());
            let raw = lookup_slot(input.slots.get(&slot).map(Vec::as_slice), window.complete, rank);
            let nonzero = raw.is_some_and(|raw| raw != 0);
            let eligible = filter
                .is_none_or(|wanted| named(slot, window.last).is_some_and(|(tag, _)| wanted.contains(tag.as_str())))
                && group.is_none_or(|group| !group.spans(slot, window.first, window.last).is_empty())
                && (!top || within);
            let end = within.then_some(raw).flatten();
            let unknown = eligible && end.is_none();
            let order = (eligible && witnessed).then(|| {
                let peak = peaks.peak(window.first, rank);
                let end = end.filter(|_| peak != 0);
                (Reverse(end.map(i64::unsigned_abs)), Reverse(end), Reverse(peak), slot)
            });
            if let Some(order) = order {
                self.ordered.insert(order);
            }
            self.unknown += usize::from(unknown);
            self.witnessed += usize::from(witnessed);
            self.nonzero += usize::from(nonzero);
            self.candidates.insert(slot, Candidate { order, unknown, witnessed, nonzero });
        }
    }

    pub(super) fn unknown(&self) -> bool {
        self.unknown != 0
    }

    pub(super) fn top(&self, limit: usize) -> Vec<u32> {
        self.ordered.iter().take(limit).map(|order| order.3).collect()
    }

    pub(super) fn other(&self, shown: &[u32]) -> (bool, u32) {
        let count = shown.iter().filter(|slot| self.candidates[*slot].nonzero).count();
        (self.witnessed > shown.len(), (self.nonzero - count) as u32)
    }
}

#[derive(Default)]
pub(super) struct EventIndex {
    changes: Edits,
    by_date: BTreeMap<i32, Vec<u32>>,
}

impl EventIndex {
    pub(super) fn new(events: &[CachedEvent]) -> Self {
        let mut index = Self::default();
        for (id, event) in events.iter().enumerate() {
            let id = id as u32;
            index.by_date.entry(event.hour).or_default().push(id);
            index.changes.add(event.first_read.max(event.hour), id);
            for &(at, _) in &event.added {
                index.changes.add(at, id);
            }
            if let Some(at) = event.removed_at {
                index.changes.add(at, id);
            }
        }
        index
    }

    pub(super) fn next(&self, last: i32) -> Option<i32> {
        self.changes.next(last)
    }
}

type EventOrder = (i32, i32, u32);

#[derive(Default)]
pub(super) struct EventWindow {
    bounds: Option<(i32, i32)>,
    rows: BTreeMap<EventOrder, TensionStackEvent>,
    by_slot: HashMap<u32, BTreeSet<EventOrder>>,
}

impl EventWindow {
    pub(super) fn update(
        &mut self,
        index: &EventIndex,
        events: &[CachedEvent],
        first: i32,
        last: i32,
        mut row_at: impl FnMut(&CachedEvent) -> Option<TensionStackEvent>,
    ) {
        let mut changed = HashSet::new();
        if let Some((old_first, old_last)) = self.bounds.filter(|(a, b)| *a <= last && first <= *b) {
            index.changes.crossed(old_last, last, &mut changed);
            for (_, ids) in index.by_date.range(old_first.min(first)..old_first.max(first)) {
                changed.extend(ids);
            }
        } else {
            self.rows.clear();
            self.by_slot.clear();
            for (_, ids) in index.by_date.range(first..=last) {
                changed.extend(ids);
            }
        }
        self.bounds = Some((first, last));
        for id in changed {
            dashboard_base::products::at_hour::count_rows(|| 1);
            let event = &events[id as usize];
            let key = (event.hour, event.first_read, id);
            if first <= event.hour
                && event.hour <= last
                && event.first_read <= last
                && let Some(row) = row_at(event)
            {
                if let Some(slot) = row.slot {
                    self.by_slot.entry(slot).or_default().insert(key);
                }
                self.rows.insert(key, row);
            } else {
                self.rows.remove(&key);
                if let Some(keys) = event.slot.and_then(|slot| self.by_slot.get_mut(&slot)) {
                    keys.remove(&key);
                }
            }
        }
    }

    pub(super) fn shown(&self, slots: Option<&HashSet<u32>>) -> Vec<TensionStackEvent> {
        let Some(slots) = slots else { return self.rows.values().cloned().collect() };
        let mut streams: Vec<_> = slots.iter().filter_map(|slot| self.by_slot.get(slot)).map(BTreeSet::iter).collect();
        let mut next = BinaryHeap::new();
        for (i, stream) in streams.iter_mut().enumerate() {
            if let Some(&key) = stream.next() {
                next.push(Reverse((key, i)));
            }
        }
        let mut rows = Vec::new();
        while let Some(Reverse((key, i))) = next.pop() {
            rows.push(self.rows[&key].clone());
            if let Some(&following) = streams[i].next() {
                next.push(Reverse((following, i)));
            }
        }
        rows
    }
}

#[derive(Default)]
pub(super) struct Plot {
    bounds: Option<(i32, i32)>,
    slots: Vec<u32>,
    other: bool,
    base: usize,
    rows: VecDeque<Sample>,
    breaks: VecDeque<usize>,
    limits: Vec<VecDeque<Vec<(f64, f64)>>>,
}

impl Plot {
    pub(super) fn extend(&mut self, first: i32, last: i32, slots: &[u32], other: bool) -> Option<(i32, bool)> {
        if let Some((start, through)) = self.bounds
            && self.slots == slots
            && self.other == other
            && start <= first
            && first <= through.saturating_add(1)
        {
            let expired = self.rows.partition_point(|row| row.hour <= first).saturating_sub(1);
            if expired > 0 {
                let old = self.base;
                self.base += expired;
                self.rows.drain(..expired);
                for (index, blocks) in self.limits.iter_mut().enumerate() {
                    let width = 2 << index;
                    let obsolete = self.base.div_ceil(width) - old.div_ceil(width);
                    blocks.drain(..obsolete.min(blocks.len()));
                }
                let obsolete = self.breaks.partition_point(|&at| at <= self.base);
                self.breaks.drain(..obsolete);
            }
            self.bounds = Some((first, through.max(last)));
            return (through < last).then(|| (through + 1, true));
        }
        *self = Self { bounds: Some((first, last)), slots: slots.to_vec(), other, ..Self::default() };
        Some((first, false))
    }

    pub(super) fn push(&mut self, row: Sample) {
        if let Some(old) = self.rows.back() {
            if old.total_raw == row.total_raw
                && old.sum_raw == row.sum_raw
                && old.band_raws == row.band_raws
                && old.other_raw == row.other_raw
            {
                return;
            }
            if old.total.is_none() != row.total.is_none()
                || old.other.is_none() != row.other.is_none()
                || old.bands.iter().zip(&row.bands).any(|(a, b)| a.is_none() != b.is_none())
            {
                self.breaks.push_back(self.base + self.rows.len());
            }
        }
        self.rows.push_back(row);
        let next = self.rows.len();
        let end = self.base + next;
        for level in 1..=end.trailing_zeros().min(next.ilog2()) as usize {
            let ranges = if level == 1 {
                self.rows[next - 2]
                    .bands
                    .iter()
                    .zip(&self.rows[next - 1].bands)
                    .map(|(a, b)| {
                        let min = a.unwrap_or(f64::INFINITY).min(b.unwrap_or(f64::INFINITY));
                        let max = a.unwrap_or(f64::NEG_INFINITY).max(b.unwrap_or(f64::NEG_INFINITY));
                        (min, max)
                    })
                    .collect()
            } else {
                let children = &self.limits[level - 2];
                let end = children.len();
                children[end - 2].iter().zip(&children[end - 1]).map(|(a, b)| (a.0.min(b.0), a.1.max(b.1))).collect()
            };
            if self.limits.len() < level {
                self.limits.push(VecDeque::new());
            }
            self.limits[level - 1].push_back(ranges);
        }
    }

    fn bounds_for(&self, row: &mut Sample, mut at: usize, end: usize) {
        row.band_min.clone_from(&row.bands);
        row.band_max.clone_from(&row.bands);
        while at < end {
            let global = self.base + at;
            let level = global.trailing_zeros().min((end - at).ilog2());
            for i in 0..row.bands.len() {
                let (min, max) = if level == 0 {
                    let Some(value) = self.rows[at].bands[i] else { continue };
                    (value, value)
                } else {
                    self.limits[level as usize - 1][(global >> level) - self.base.div_ceil(1 << level)][i]
                };
                if min <= max {
                    row.band_min[i] = Some(row.band_min[i].map_or(min, |v| v.min(min)));
                    row.band_max[i] = Some(row.band_max[i].map_or(max, |v| v.max(max)));
                }
            }
            at += 1 << level;
        }
    }

    pub(super) fn samples(&self, first: i32, last: i32, max_points: usize) -> (Vec<Sample>, bool) {
        let start = self.rows.partition_point(|row| row.hour <= first).saturating_sub(1);
        let end = self.rows.partition_point(|row| row.hour <= last);
        if end - start <= max_points || max_points < 4 {
            let mut rows: Vec<_> = self.rows.range(start..end).cloned().collect();
            if let Some(row) = rows.first_mut() {
                row.hour = first;
            }
            return (rows, false);
        }
        let step = ((i64::from(last) - i64::from(first)) / max_points as i64).max(1);
        let mut rows = Vec::with_capacity(max_points + 2);
        let mut index = start;
        let mut boundary = self.breaks.partition_point(|&at| at <= self.base + start);
        while index < end {
            let mut row = self.rows[index].clone();
            row.hour = row.hour.max(first);
            let bucket = (i64::from(row.hour) - i64::from(first)) / step;
            let next_hour = i64::from(first) + (bucket + 1) * step;
            let mut next = self.rows.partition_point(|point| i64::from(point.hour) < next_hour).min(end);
            if let Some(&at) = self.breaks.get(boundary) {
                let at = at - self.base;
                next = next.min(at);
                if next == at {
                    boundary += 1;
                }
            }
            if index + 1 < end {
                next = next.min(end - 1);
            }
            self.bounds_for(&mut row, index + 1, next);
            rows.push(row);
            index = next;
        }
        (rows, true)
    }
}
