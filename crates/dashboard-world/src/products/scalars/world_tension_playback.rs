use super::{CachedEvent, DatedGroupFilter, SlotNames, StackInput, TensionStackEvent, lookup_slot};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet},
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
    source: Option<std::sync::Arc<SlotIndex>>,
    dates: HashMap<u32, Vec<i32>>,
    changes: Edits,
    peaks: BTreeMap<u32, Peaks>,
}

impl SlotIndex {
    pub(super) fn new(
        input: &StackInput<'_>,
        group: Option<&DatedGroupFilter>,
        names: Option<&SlotNames>,
        source: Option<std::sync::Arc<SlotIndex>>,
    ) -> Self {
        let mut changes = Edits::default();
        let mut peaks = BTreeMap::new();
        for (&slot, runs) in input.slots {
            if group.is_some_and(|g| !g.runs_by_slot.contains_key(&slot)) {
                continue;
            }
            let mut values = Vec::new();
            for &(first, last, value) in runs {
                if source.is_none() {
                    changes.add(first, slot);
                    if let Some(next) = last.checked_add(1) {
                        changes.add(next, slot);
                    }
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
        let mut dates = HashMap::<u32, Vec<i32>>::new();
        for (&hour, keys) in &mut changes.0 {
            keys.sort_unstable();
            keys.dedup();
            for &slot in keys.iter() {
                dates.entry(slot).or_default().push(hour);
            }
        }
        Self { changes, peaks, dates, source }
    }

    fn indexes(&self) -> impl Iterator<Item = &SlotIndex> {
        self.source.as_deref().into_iter().chain(std::iter::once(self))
    }

    pub(super) fn plot_edits(&self, slot: u32, first: i32, last: i32, out: &mut Vec<i32>) {
        for index in self.indexes() {
            if let Some(dates) = index.dates.get(&slot) {
                let edits = &dates[dates.partition_point(|&hour| hour < first)..dates.partition_point(|&hour| hour <= last)];
                dashboard_base::products::at_hour::count_rows(|| edits.len() as u64);
                out.extend_from_slice(edits);
            }
        }
    }

    fn changed(&self, old: Window, window: Window) -> Option<HashSet<u32>> {
        let mut remaining = self.peaks.len();
        let mut slots = HashSet::new();
        let mut ranges = [(old.first, window.first), (old.last, window.last), (old.rank?, window.rank?)];
        for (a, b) in &mut ranges {
            if *a > *b {
                std::mem::swap(a, b);
            }
        }
        ranges.sort_unstable();
        let mut through = i32::MIN;
        for (a, b) in ranges {
            let after = a.max(through);
            if after >= b {
                continue;
            }
            through = b;
            for index in self.indexes() {
                for (_, ids) in index.changes.0.range((Excluded(after), Included(b))) {
                    remaining = remaining.checked_sub(ids.len())?;
                    dashboard_base::products::at_hour::count_rows(|| ids.len() as u64);
                    slots.extend(ids.iter().filter(|slot| self.peaks.contains_key(slot)).copied());
                }
            }
        }
        Some(slots)
    }

    pub(super) fn next(&self, hour: i32) -> Option<i32> {
        self.indexes().filter_map(|i| i.changes.next(hour)).min()
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
        filter: Option<&dyn Fn(&str) -> bool>,
        top: bool,
        named: &impl Fn(u32, i32) -> Option<(String, bool)>,
    ) {
        let Some(rank) = window.rank else {
            *self = Self::default();
            return;
        };
        let changed = self
            .window
            .filter(|old| old.complete == window.complete)
            .and_then(|old| index.changed(old, window))
            .unwrap_or_else(|| index.peaks.keys().copied().collect());
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
            let eligible = filter.is_none_or(|wanted| named(slot, window.last).is_some_and(|(tag, _)| wanted(&tag)))
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

    pub(super) fn top(&self, limit: Option<usize>) -> Vec<u32> {
        let slots = self.ordered.iter().map(|order| order.3);
        match limit {
            Some(limit) => slots.take(limit).collect(),
            None => slots.collect(),
        }
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
        for ids in index.changes.0.values_mut() {
            ids.sort_unstable_by_key(|&id| (events[id as usize].hour, id));
            ids.dedup();
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
        let mut changed = HashSet::<u32>::new();
        if let Some((old_first, old_last)) =
            self.bounds.filter(|(a, b)| *a <= last && first <= *b && a.abs_diff(first).max(b.abs_diff(last)) <= last.abs_diff(first))
        {
            let latest = old_last.max(last);
            for (_, ids) in index.changes.0.range((Excluded(old_last.min(last)), Included(latest))) {
                let from = ids.partition_point(|&id| events[id as usize].hour < first);
                let to = ids.partition_point(|&id| events[id as usize].hour <= latest);
                changed.extend(&ids[from..to]);
            }
            for (_, ids) in index.by_date.range(old_first.min(first)..old_first.max(first)) {
                let known = ids.partition_point(|&id| events[id as usize].first_read <= latest);
                changed.extend(&ids[..known]);
            }
        } else {
            self.rows.clear();
            self.by_slot.clear();
            for (_, ids) in index.by_date.range(first..=last) {
                let known = ids.partition_point(|&id| events[id as usize].first_read <= last);
                changed.extend(&ids[..known]);
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

    pub(super) fn visit(&self, slots: Option<&HashSet<u32>>, mut visit: impl FnMut(u32, &TensionStackEvent)) {
        let Some(slots) = slots else {
            for (key, row) in &self.rows { visit(key.2, row); }
            return;
        };
        let mut streams: Vec<_> = slots.iter().filter_map(|slot| self.by_slot.get(slot)).map(BTreeSet::iter).collect();
        let mut next = BinaryHeap::new();
        for (i, stream) in streams.iter_mut().enumerate() {
            if let Some(&key) = stream.next() {
                next.push(Reverse((key, i)));
            }
        }
        while let Some(Reverse((key, i))) = next.pop() {
            visit(key.2, &self.rows[&key]);
            if let Some(&following) = streams[i].next() {
                next.push(Reverse((following, i)));
            }
        }
    }
}
