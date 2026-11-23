use super::Analysis;
use replay_recording::command_packet::{Actor, Category, Event, Origin, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub orders: u64,
    pub paused_orders: u64,
    pub unresolved: u64,
    pub unclassified: u64,
    pub paused_unresolved: u64,
}

#[derive(Default)]
struct Counter {
    hours: Vec<i32>,
    totals: Vec<u64>,
}

impl Counter {
    fn observe(&mut self, hour: i32, value: u64) -> io::Result<()> {
        if value == 0 {
            return Ok(());
        }
        if self.hours.last() == Some(&hour) {
            *self.totals.last_mut().unwrap() += value;
        } else {
            engine_allocation::reserve(&mut self.hours, 1)?;
            engine_allocation::reserve(&mut self.totals, 1)?;
            self.hours.push(hour);
            self.totals.push(value);
        }
        Ok(())
    }

    fn prepare(&mut self) -> io::Result<()> {
        if self.hours.windows(2).any(|w| w[0] >= w[1]) {
            let mut sorted = engine_allocation::vec(self.hours.len())?;
            sorted.extend(self.hours.drain(..).zip(self.totals.drain(..)));
            sorted.sort_unstable_by_key(|&(hour, _)| hour);
            for (hour, count) in sorted {
                self.observe(hour, count)?;
            }
        }
        let mut total = 0;
        for value in &mut self.totals {
            total += *value;
            *value = total;
        }
        Ok(())
    }

    fn range(&self, first: i32, last: i32) -> u64 {
        let from = self.hours.partition_point(|&h| h < first);
        let to = self.hours.partition_point(|&h| h <= last);
        let prefix = |n: usize| n.checked_sub(1).map_or(0, |i| self.totals[i]);
        prefix(to) - prefix(from)
    }
}

#[derive(Default)]
pub struct CountSeries {
    counters: [Counter; 5],
    unknown: Counter,
    uncounted_hours: Vec<i32>,
}

impl CountSeries {
    pub(super) fn observe(&mut self, event: &Event, category: Category) -> io::Result<()> {
        let mut counts = Counts::default();
        counts.observe(event, category);
        if counts == Counts::default() && self.uncounted_hours.last() != Some(&event.hour) {
            engine_allocation::reserve(&mut self.uncounted_hours, 1)?;
            self.uncounted_hours.push(event.hour);
        }
        for (counter, value) in self.counters.iter_mut().zip([
            counts.orders,
            counts.paused_orders,
            counts.unresolved,
            counts.unclassified,
            counts.paused_unresolved,
        ]) {
            counter.observe(event.hour, value)?;
        }
        Ok(())
    }

    fn prepare(&mut self, unknown: &BTreeSet<i32>) -> io::Result<()> {
        for counter in &mut self.counters {
            counter.prepare()?;
        }
        self.uncounted_hours.sort_unstable();
        self.uncounted_hours.dedup();
        for &hour in unknown {
            if self.uncounted_hours.binary_search(&hour).is_ok()
                || self.counters.iter().any(|c| c.range(hour, hour) != 0)
            {
                self.unknown.observe(hour, 1)?;
            }
        }
        self.uncounted_hours = Vec::new();
        self.unknown.prepare()
    }

    pub fn range(&self, first: i32, last: i32) -> (Counts, bool) {
        let [
            orders,
            paused_orders,
            unresolved,
            unclassified,
            paused_unresolved,
        ] = self.counters.each_ref().map(|c| c.range(first, last));
        (
            Counts {
                orders,
                paused_orders,
                unresolved,
                unclassified,
                paused_unresolved,
            },
            self.unknown.range(first, last) != 0,
        )
    }
}

impl Counts {
    pub(super) fn observe(&mut self, event: &Event, category: Category) {
        if event.paused {
            if category == Category::Other
                || (category.counted() && event.origin != Origin::PlayerOrder)
            {
                self.paused_unresolved += 1;
            } else if category.counted() {
                self.paused_orders += 1;
            }
        } else if category == Category::Other {
            self.unclassified += 1;
        } else if category.counted() {
            if event.origin == Origin::PlayerOrder {
                self.orders += 1;
            } else {
                self.unresolved += 1;
            }
        }
    }

    pub fn add(&mut self, other: Self) {
        self.orders += other.orders;
        self.paused_orders += other.paused_orders;
        self.unresolved += other.unresolved;
        self.unclassified += other.unclassified;
        self.paused_unresolved += other.paused_unresolved;
    }
}

pub struct Timeline {
    pub hours: (i32, i32),
    pub session: u64,
    pub capture: u64,
    pub world: u64,
    pub frequency: u64,
    pub complete: bool,
    pub spans: Vec<Span>,
    pub actors: BTreeMap<u64, Vec<Actor>>,
    pub departures: BTreeMap<u64, u64>,
    pub counts: BTreeMap<(u64, usize), CountSeries>,
    pub unknown_hours: BTreeSet<i32>,
    pub paused_unknown_hours: BTreeSet<i32>,
    pub(super) timing: Timing,
}

#[derive(Default)]
pub(super) struct Timing {
    ticks: Vec<u128>,
    faults: Vec<usize>,
    gaps: Vec<usize>,
    missing_ticks: Vec<(usize, u128)>,
}

pub struct Measurement {
    pub minutes: f64,
    pub complete: bool,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ClockMeasurement {
    pub recorded_ticks: u128,
    pub playing_ticks: Option<u128>,
    pub frequency: u64,
    pub complete: bool,
    pub first_hour: i32,
    pub last_hour: i32,
    pub from_qpc: u64,
    pub to_qpc: u64,
}

impl Timeline {
    pub(super) fn prepare(&mut self) -> io::Result<()> {
        for counts in self.counts.values_mut() {
            counts.prepare(&self.unknown_hours)?;
        }
        Ok(())
    }
}

impl Timing {
    pub(super) fn new(spans: &[Span]) -> io::Result<Self> {
        let mut timing = Timing::default();
        for values in [&mut timing.faults, &mut timing.gaps] {
            engine_allocation::reserve(values, spans.len().saturating_add(1))?;
            values.push(0);
        }
        engine_allocation::reserve(&mut timing.ticks, spans.len().saturating_add(1))?;
        timing.ticks.push(0);
        for (index, span) in spans.iter().enumerate() {
            let previous = index.checked_sub(1).map(|index| &spans[index]);
            if previous.is_some_and(|previous| previous.hour > span.hour) {
                return Err(super::invalid("command clock reversed within a live world"));
            }
            let ticks = if span.paused {
                0
            } else {
                span.to_qpc - span.from_qpc
            };
            timing
                .ticks
                .push(timing.ticks.last().unwrap() + u128::from(ticks));
            timing
                .faults
                .push(timing.faults.last().unwrap() + usize::from(!span.complete));
            let gap = previous.is_some_and(|previous| {
                previous.to_qpc < span.from_qpc
                    || i64::from(previous.hour) + 1 < i64::from(span.hour)
            });
            if let Some(previous) = previous.filter(|p| p.to_qpc < span.from_qpc) {
                let ticks = timing.missing_ticks.last().map_or(0, |&(_, ticks)| ticks)
                    + u128::from(span.from_qpc - previous.to_qpc);
                engine_allocation::reserve(&mut timing.missing_ticks, 1)?;
                timing.missing_ticks.push((index, ticks));
            }
            timing
                .gaps
                .push(timing.gaps.last().unwrap() + usize::from(gap));
        }
        Ok(timing)
    }

    pub(super) fn between(&self, spans: &[Span], from: u64, to: u64) -> u128 {
        if from >= to {
            return 0;
        }
        let first = spans.partition_point(|s| s.to_qpc <= from);
        let last = spans.partition_point(|s| s.from_qpc < to);
        if first >= last {
            return 0;
        }
        let start = &spans[first];
        let end = &spans[last - 1];
        let mut ticks = self.ticks[last] - self.ticks[first];
        if !start.paused {
            ticks -= u128::from(from.saturating_sub(start.from_qpc));
        }
        if !end.paused {
            ticks -= u128::from(end.to_qpc.saturating_sub(to));
        }
        ticks
    }
}

impl Timeline {
    pub fn clock(&self, first: i32, last: i32) -> Option<ClockMeasurement> {
        if first > last {
            return None;
        }
        let from = self.spans.partition_point(|s| s.hour < first);
        let to = self.spans.partition_point(|s| s.hour <= last);
        if from >= to {
            return None;
        }
        let start = &self.spans[from];
        let end = &self.spans[to - 1];
        let missing = |at| {
            let n = self
                .timing
                .missing_ticks
                .partition_point(|&(index, _)| index < at);
            n.checked_sub(1)
                .map_or(0, |i| self.timing.missing_ticks[i].1)
        };
        let recorded = u128::from(end.to_qpc - start.from_qpc) - (missing(to) - missing(from + 1));
        let playing = self.timing.ticks[to] - self.timing.ticks[from];
        let classified = self.timing.faults[to] == self.timing.faults[from];
        Some(ClockMeasurement {
            recorded_ticks: recorded,
            playing_ticks: classified.then_some(playing),
            frequency: self.frequency,
            complete: classified && self.timing.gaps[to] == self.timing.gaps[from + 1],
            first_hour: start.hour,
            last_hour: end.hour,
            from_qpc: start.from_qpc,
            to_qpc: end.to_qpc,
        })
    }

    pub fn bounds(&self, first: i32, last: i32) -> Option<(u64, u64)> {
        let from = self.spans.partition_point(|span| span.hour < first);
        let to = self.spans.partition_point(|span| span.hour <= last);
        (from < to).then(|| (self.spans[from].from_qpc, self.spans[to - 1].to_qpc))
    }

    pub fn measure(
        &self,
        first: i32,
        last: i32,
        from_qpc: u64,
        to_qpc: u64,
    ) -> Option<Measurement> {
        if from_qpc >= to_qpc {
            return None;
        }
        let from = self
            .spans
            .partition_point(|span| span.hour < first)
            .max(self.spans.partition_point(|span| span.to_qpc <= from_qpc));
        let to = self
            .spans
            .partition_point(|span| span.hour <= last)
            .min(self.spans.partition_point(|span| span.from_qpc < to_qpc));
        if from >= to {
            return None;
        }
        let start = &self.spans[from];
        let end = &self.spans[to - 1];
        let mut ticks = self.timing.ticks[to] - self.timing.ticks[from];
        if !start.paused {
            ticks -= u128::from(from_qpc.saturating_sub(start.from_qpc));
        }
        if !end.paused {
            ticks -= u128::from(end.to_qpc.saturating_sub(to_qpc));
        }
        Some(Measurement {
            minutes: ticks as f64 / self.frequency as f64 / 60.0,
            complete: self.timing.faults[to] == self.timing.faults[from]
                && self.timing.gaps[to] == self.timing.gaps[from + 1]
                && self
                    .unknown_hours
                    .range(start.hour..=end.hour)
                    .next()
                    .is_none(),
            first_hour: start.hour,
            last_hour: end.hour,
        })
    }
}

pub struct Index {
    pub analysis: Analysis,
    pub timelines: Vec<Timeline>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_prefixes_match_raw_counts_across_all_ranges_and_faults() {
        let mut series = CountSeries::default();
        let mut raw = BTreeMap::<i32, Counts>::new();
        let hours = [i32::MAX, 10, -3, 10, i32::MIN, 9, 9, 11];
        for (i, hour) in hours.into_iter().enumerate() {
            let event = Event {
                qpc: i as u64,
                hour,
                actor: 1,
                player: 1,
                kind: 1,
                command_country: Some(1),
                sequence: i as u32,
                paused: i % 3 == 0,
                origin: if i % 2 == 0 {
                    Origin::PlayerOrder
                } else {
                    Origin::Unresolved
                },
            };
            let category = match i % 3 {
                0 => Category::Army,
                1 => Category::Other,
                _ => Category::System,
            };
            series.observe(&event, category).unwrap();
            raw.entry(hour).or_default().observe(&event, category);
        }
        let unknown = BTreeSet::from([i32::MIN, -3, 9, 12]);
        series.prepare(&unknown).unwrap();
        for first in [i32::MIN, -3, 9, 10, 11, 12, i32::MAX] {
            for last in [first, 11.max(first), i32::MAX] {
                let mut expected = Counts::default();
                for count in raw.range(first..=last).map(|(_, c)| c) {
                    expected.add(*count);
                }
                let incomplete = raw.range(first..=last).any(|(h, _)| unknown.contains(h));
                assert_eq!(series.range(first, last), (expected, incomplete));
            }
        }
    }
}
