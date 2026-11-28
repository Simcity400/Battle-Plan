use super::{
    DatedGroupFilter, PieceWindow, Sample, SlotIndex, StackInput, TensionValueMode, collect_change_hours, is_complete, lookup_run, lookup_slot,
};
use engine_stability::{cthreat_source, cworld_threat};
use std::{borrow::Cow, collections::{HashMap, HashSet, VecDeque}};

const WORLD_SCALE: f64 = (cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR) as f64;
const COUNTRY_SCALE: f64 = cthreat_source::FIXED_POINT_SCALE as f64;

#[derive(Clone, Copy, Default, PartialEq)]
struct Value {
    raw: Option<i64>,
    value: Option<f64>,
    sum: Option<i64>,
    rest_known: bool,
}

fn amount(mode: TensionValueMode, raw: i128, world: Value) -> Option<f64> {
    if mode == TensionValueMode::Recorded {
        return Some(raw as f64 / COUNTRY_SCALE);
    }
    let (total, sum) = (world.raw?, world.sum?);
    if sum > 0 {
        Some((raw * i128::from(total)) as f64 / (sum as f64 * WORLD_SCALE))
    } else {
        (sum == 0 && total == 0 && raw == 0).then_some(0.0)
    }
}

#[derive(Default)]
struct Curve {
    window: Option<(i32, i32)>,
    base: usize,
    points: VecDeque<(i32, Value)>,
    gaps: VecDeque<i32>,
    rest_edges: VecDeque<i32>,
    limits: Vec<VecDeque<(f64, f64)>>,
}

impl Curve {
    fn extend(&mut self, first: i32, last: i32) -> Option<i32> {
        if let Some((start, through)) = self.window
            && start <= first
            && first <= through.saturating_add(1)
        {
            let remove = self.points.partition_point(|(hour, _)| *hour <= first).saturating_sub(1);
            if remove > 0 {
                let old = self.base;
                self.base += remove;
                self.points.drain(..remove);
                for (level, blocks) in self.limits.iter_mut().enumerate() {
                    let width = 2 << level;
                    let count = self.base.div_ceil(width) - old.div_ceil(width);
                    blocks.drain(..count.min(blocks.len()));
                }
                for edges in [&mut self.gaps, &mut self.rest_edges] {
                    let at = edges.partition_point(|&hour| hour <= first);
                    edges.drain(..at);
                }
            }
            self.window = Some((first, through.max(last)));
            return (through < last).then(|| through + 1);
        }
        *self = Self { window: Some((first, last)), ..Self::default() };
        Some(first)
    }

    fn push(&mut self, hour: i32, value: Value) {
        if let Some(&(_, old)) = self.points.back() {
            if old == value {
                return;
            }
            if old.value.is_none() != value.value.is_none() {
                self.gaps.push_back(hour);
            }
            if old.rest_known != value.rest_known {
                self.rest_edges.push_back(hour);
            }
        }
        self.points.push_back((hour, value));
        let len = self.points.len();
        let end = self.base + len;
        for level in 1..=end.trailing_zeros().min(len.ilog2()) as usize {
            let range = if level == 1 {
                let a = self.points[len - 2].1.value;
                let b = value.value;
                (
                    a.unwrap_or(f64::INFINITY).min(b.unwrap_or(f64::INFINITY)),
                    a.unwrap_or(f64::NEG_INFINITY).max(b.unwrap_or(f64::NEG_INFINITY)),
                )
            } else {
                let children = &self.limits[level - 2];
                let (a, b) = (children[children.len() - 2], children[children.len() - 1]);
                (a.0.min(b.0), a.1.max(b.1))
            };
            if self.limits.len() < level {
                self.limits.push(VecDeque::new());
            }
            self.limits[level - 1].push_back(range);
        }
    }

    fn at(&self, hour: i32) -> Value {
        self.points[self.points.partition_point(|(at, _)| *at <= hour) - 1].1
    }

    fn next(&self, from: i32) -> Option<i32> {
        self.points.get(self.points.partition_point(|(at, _)| *at < from)).map(|p| p.0)
    }

    fn range(&self, first: i32, last: i32) -> (Option<f64>, Option<f64>) {
        let mut at = self.points.partition_point(|(hour, _)| *hour <= first) - 1;
        let end = self.points.partition_point(|(hour, _)| *hour <= last);
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        while at < end {
            let global = self.base + at;
            let level = global.trailing_zeros().min((end - at).ilog2());
            let (lo, hi) = if level == 0 {
                let value = self.points[at].1.value;
                (value.unwrap_or(f64::INFINITY), value.unwrap_or(f64::NEG_INFINITY))
            } else {
                self.limits[level as usize - 1][(global >> level) - self.base.div_ceil(1 << level)]
            };
            min = min.min(lo);
            max = max.max(hi);
            at += 1 << level;
        }
        ((min <= max).then_some(min), (min <= max).then_some(max))
    }
}

#[derive(Default)]
pub(super) struct Plot {
    world: Curve,
    slots: HashMap<u32, Curve>,
    changes: ChangeHours,
}

impl Plot {
    pub(super) fn read(
        &mut self,
        input: &StackInput<'_>,
        index: &SlotIndex,
        shown: &[u32],
        group: Option<&DatedGroupFilter>,
        mode: TensionValueMode,
        other: bool,
        first: i32,
        last: i32,
        limit: usize,
        piece: Option<PieceWindow>,
    ) -> (Vec<Sample>, bool, Sample) {
        let mut hours = Vec::new();
        if let Some(start) = self.world.extend(first, last) {
            hours.push(start);
            collect_change_hours(input.tension, start, last, &mut hours);
            collect_change_hours(input.sum, start, last, &mut hours);
            hours.sort_unstable();
            hours.dedup();
            for hour in hours.drain(..) {
                let raw = lookup_run(input.tension, hour);
                let sum = lookup_run(input.sum, hour);
                let world = Value { raw, sum, value: raw.map(|raw| raw as f64 / WORLD_SCALE), rest_known: false };
                self.world.push(hour, Value { rest_known: sum.is_some() && amount(mode, 0, world).is_some(), ..world });
            }
        }
        for &slot in shown {
            let curve = self.slots.entry(slot).or_default();
            let Some(start) = curve.extend(first, last) else { continue };
            if curve.points.is_empty() {
                hours.push(start);
            }
            index.plot_edits(slot, start, last, &mut hours);
            let spans = input.aggregates_complete;
            let at = spans.partition_point(|span| span.1.saturating_add(1) < start);
            for &(lo, hi) in spans[at..].iter().take_while(|span| span.0 <= last) {
                if lo >= start {
                    hours.push(lo);
                }
                if hi < last && hi + 1 >= start {
                    hours.push(hi + 1);
                }
            }
            if mode == TensionValueMode::Share {
                let from = self.world.points.partition_point(|p| p.0 < start);
                let to = self.world.points.partition_point(|p| p.0 <= last);
                hours.extend(self.world.points.range(from..to).map(|p| p.0));
            }
            hours.sort_unstable();
            hours.dedup();
            for hour in hours.drain(..) {
                let member = group.is_none_or(|group| !group.spans(slot, hour, hour).is_empty());
                let raw = member
                    .then(|| lookup_slot(input.slots.get(&slot).map(Vec::as_slice), is_complete(spans, hour), hour))
                    .flatten();
                let value = raw.and_then(|r| amount(mode, i128::from(r), self.world.at(hour)));
                curve.push(hour, Value { raw: raw.filter(|_| value.is_some()), value, ..Value::default() });
            }
        }
        let columns: Vec<_> = shown.iter().map(|slot| &self.slots[slot]).collect();
        self.changes.select(first, limit, shown);
        let (hours, reduced) = self.changes.sample(&self.world, &columns, other, last);
        let sample = |hour, until: Option<i32>| {
            let world = self.world.at(hour);
            let mut raw_sum = Some(0i128);
            let mut sample = Sample {
                hour,
                total: world.value,
                total_raw: world.raw.map(i128::from),
                sum_raw: world.sum,
                bands: Vec::with_capacity(columns.len()),
                band_raws: Vec::with_capacity(columns.len()),
                band_min: Vec::new(),
                band_max: Vec::new(),
                other: None,
                other_raw: None,
            };
            for column in &columns {
                let point = column.at(hour);
                sample.bands.push(point.value);
                sample.band_raws.push(point.raw);
                raw_sum = raw_sum.zip(point.raw).map(|(sum, raw)| sum + i128::from(raw));
                if let Some(last) = until {
                    let (min, max) = column.range(hour, last);
                    sample.band_min.push(min);
                    sample.band_max.push(max);
                }
            }
            if other
                && world.rest_known
                && let Some((sum, shown)) = world.sum.zip(raw_sum)
            {
                let raw = i128::from(sum) - shown;
                sample.other = amount(mode, raw, world);
                sample.other_raw = sample.other.map(|_| raw);
            }
            sample
        };
        let selection = piece.map_or(0..hours.len(), |piece| {
            let witnessed = std::iter::once(&self.world).chain(columns.iter().copied()).filter_map(|curve| {
                let at = curve.points.partition_point(|p| p.0 < piece.start);
                at.checked_sub(1).map(|i| curve.points[i].0)
            }).max().unwrap_or(first);
            let closed = hours.partition_point(|&hour| hour < witnessed).saturating_sub(1);
            if piece.history { 0..closed } else { closed..hours.len() }
        });
        let samples = selection
            .map(|i| {
                let end = reduced.then(|| hours.get(i + 1).map_or(last, |&next| next - 1));
                sample(hours[i], end)
            })
            .collect();
        (samples, reduced, sample(last, None))
    }
}

#[derive(Default)]
struct ChangeHours {
    first: i32,
    limit: usize,
    slots: HashSet<u32>,
    through: i32,
    hours: Vec<i32>,
}

impl ChangeHours {
    fn select(&mut self, first: i32, limit: usize, slots: &[u32]) {
        if self.hours.is_empty() || self.first != first || self.limit != limit
            || self.slots.len() != slots.len() || slots.iter().any(|slot| !self.slots.contains(slot))
        {
            self.first = first;
            self.limit = limit;
            self.slots.clear();
            self.slots.extend(slots.iter().copied());
            self.through = first;
            self.hours.clear();
            self.hours.push(first);
        }
    }

    fn exact(&mut self, last: i32, mut next: impl FnMut(i32) -> Option<i32>) -> Option<&[i32]> {
        if last > self.through {
            let mut hour = self.through;
            while hour < last && (self.limit < 4 || self.hours.len() <= self.limit) {
                let Some(at) = hour.checked_add(1).and_then(&mut next).filter(|&at| at <= last) else { break };
                self.hours.push(at);
                hour = at;
            }
            self.through = last;
        }
        let count = self.hours.partition_point(|&hour| hour <= last);
        (self.limit < 4 || count <= self.limit).then_some(&self.hours[..count])
    }

    fn sample(&mut self, world: &Curve, columns: &[&Curve], other: bool, last: i32) -> (Cow<'_, [i32]>, bool) {
        let first = self.first;
        let limit = self.limit;
        let curves = || std::iter::once(world).chain(columns.iter().copied());
        let next = |from| curves().filter_map(|curve| curve.next(from)).min();
        if let Some(hours) = self.exact(last, next) {
            return (Cow::Borrowed(hours), false);
        }
        let mut hours = vec![first];
        let step = ((i64::from(last) - i64::from(first)) / limit as i64).max(1);
        let final_hour = curves()
            .filter_map(|curve| {
                let at = curve.points.partition_point(|p| p.0 <= last);
                at.checked_sub(1).map(|i| curve.points[i].0)
            })
            .max()
            .unwrap_or(first);
        let mut edge = world.rest_edges.partition_point(|&h| h <= first);
        let mut hour = first;
        while hour < final_hour {
            let mut keep = final_hour;
            let bucket = (i64::from(hour) - i64::from(first)) / step + 1;
            if let Some(at) = i32::try_from(i64::from(first) + bucket * step).ok().and_then(next) {
                keep = keep.min(at);
            }
            for curve in curves() {
                if let Some(&at) = curve.gaps.get(curve.gaps.partition_point(|&h| h <= hour)) {
                    keep = keep.min(at);
                }
            }
            while other
                && let Some(&at) = world.rest_edges.get(edge)
                && at <= keep
            {
                edge += 1;
                if columns.iter().all(|column| column.at(at).value.is_some()) {
                    keep = at;
                    break;
                }
            }
            hours.push(keep);
            hour = keep;
        }
        (Cow::Owned(hours), true)
    }
}

#[test]
fn world_tension_reuses_change_hours_across_edges_and_seeks() {
    use std::cell::Cell;

    let mut changes = ChangeHours::default();
    let visited = Cell::new(0);
    let next = |first: i32| {
        visited.set(visited.get() + 1);
        Some(first + first.rem_euclid(2))
    };
    changes.select(10, 4, &[1, 2]);
    assert_eq!(changes.exact(14, next), Some([10, 12, 14].as_slice()));
    assert_eq!(visited.replace(0), 2);

    changes.select(10, 4, &[2, 1]);
    assert_eq!(changes.exact(16, next), Some([10, 12, 14, 16].as_slice()));
    assert_eq!(visited.replace(0), 1);
    assert_eq!(changes.exact(18, next), None);
    assert_eq!(visited.replace(0), 1);

    assert_eq!(changes.exact(20, next), None);
    assert_eq!(changes.exact(12, next), Some([10, 12].as_slice()));
    assert_eq!(changes.exact(24, next), None);
    assert_eq!(visited.replace(0), 0);

    changes.select(12, 4, &[1, 2]);
    assert_eq!(changes.exact(16, next), Some([12, 14, 16].as_slice()));
    assert_eq!(visited.replace(0), 2);
    changes.select(12, 4, &[2, 3]);
    assert_eq!(changes.exact(16, |_| None), Some([12].as_slice()));
    changes.select(12, 5, &[2, 3]);
    assert_eq!(changes.exact(20, next), Some([12, 14, 16, 18, 20].as_slice()));
}
