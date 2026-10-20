//! Daily windows: each calendar day of a request, valued at its last reached
//! recorded hour, for heat charts that draw one cell per day. A day whose last
//! recorded hour is not known stays a gap; a value known earlier in the day
//! never stands in for its endpoint. Days are the game's (`STAGGER_PERIOD`
//! hours from hour zero), the same boundary the air superiority timeline uses.

use crate::query::{KeyedRun, KeyedSeries, KeyedWindow, Query};
use dashboard_base::{prepared::prefix, products::country_groups::HourSpan};
use std::{collections::BTreeMap, io};

/// Hours in a game day.
pub const DAY_HOURS: i32 = replay_data::cadence::STAGGER_PERIOD;

/// The first hour of the day holding `hour`.
pub fn day_start(hour: i32) -> i32 {
    (i64::from(hour.div_euclid(DAY_HOURS)) * i64::from(DAY_HOURS)).max(i64::from(i32::MIN)) as i32
}

/// One day of a request, clipped to it: its first and last hour, and its last
/// recorded hour, whose values the day shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Day {
    pub first: i32,
    pub last: i32,
    pub hour: i32,
    /// Exclusive end of the consecutive days from this one, so a long constant
    /// run projects without visiting each day.
    end: usize,
}

/// The days of a request that hold a recorded hour, in order.
pub struct Days(Vec<Day>);

impl Days {
    /// The days `recorded` (ordered recorded hours, without gaps) reaches
    /// within `first..=last`.
    pub fn new(recorded: &[HourSpan], first: i32, last: i32) -> Self {
        let mut days: Vec<Day> = Vec::new();
        if first > last {
            return Self(days);
        }
        for span in recorded {
            for day in span.first_hour.div_euclid(DAY_HOURS)..=span.last_hour.div_euclid(DAY_HOURS)
            {
                let start = i64::from(day) * i64::from(DAY_HOURS);
                let end = start + i64::from(DAY_HOURS) - 1;
                let (from, to) = (start.max(i64::from(first)), end.min(i64::from(last)));
                if from > to {
                    continue;
                }
                let hour = end.min(i64::from(span.last_hour)).min(i64::from(last)) as i32;
                match days.last_mut() {
                    Some(previous) if previous.first == from as i32 => {
                        previous.hour = previous.hour.max(hour)
                    }
                    _ => days.push(Day {
                        first: from as i32,
                        last: to as i32,
                        hour,
                        end: 0,
                    }),
                }
            }
        }
        let mut end = days.len();
        for index in (0..days.len()).rev() {
            if index + 1 == days.len()
                || days[index].last.checked_add(1) != Some(days[index + 1].first)
            {
                end = index + 1;
            }
            days[index].end = end;
        }
        Self(days)
    }

    /// The recorded days of `first..=last` a query's accepted prefix answers.
    pub fn requested(query: &Query<'_>, first: i32, last: i32) -> Self {
        let through = query.through().map_or(last, |through| last.min(through));
        let recorded =
            prefix::recorded(query.source, query.branch, day_start(first), Some(through));
        Self::new(&recorded, first, through)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Day> {
        self.0.iter()
    }

    /// Runs valued at each day's endpoint: a day whose endpoint a run holds
    /// takes its value for the whole day, and equal adjacent days coalesce.
    fn project<T: Clone + Eq>(
        &self,
        runs: impl IntoIterator<Item = (i32, i32, T)>,
    ) -> Vec<(i32, i32, T, bool)> {
        let mut result: Vec<(i32, i32, T, bool)> = Vec::new();
        for (first, last, value) in runs {
            let mut at = self.0.partition_point(|day| day.hour < first);
            let end = self.0.partition_point(|day| day.hour <= last);
            while at < end {
                let through = self.0[at].end.min(end);
                let (from, to) = (self.0[at].first, self.0[through - 1].last);
                let adjacent = result
                    .last()
                    .is_some_and(|run| run.1.checked_add(1) == Some(from));
                match result.last_mut() {
                    Some(previous) if adjacent && previous.2 == value => previous.1 = to,
                    _ => result.push((from, to, value.clone(), !adjacent)),
                }
                at = through;
            }
        }
        result
    }

    /// An hourly window sampled once a day: its known spans, reads and every
    /// series, each day valued at its endpoint.
    pub fn sample(
        &self,
        window: &mut KeyedWindow,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        window.known = self
            .project(
                window
                    .known
                    .iter()
                    .map(|span| (span.first_hour, span.last_hour, ())),
            )
            .into_iter()
            .map(|(first_hour, last_hour, (), _)| HourSpan {
                first_hour,
                last_hour,
            })
            .collect();
        window.reads = self
            .project(window.reads.iter().copied())
            .into_iter()
            .map(|(first, last, read, _)| (first, last, read))
            .collect();
        for series in &mut window.series {
            dashboard_text::metadata::check_cancel(cancelled)?;
            series.runs = self
                .project(series.runs.iter().map(|run| (run.0, run.1, run.2.as_str())))
                .into_iter()
                .map(|(first, last, value, starts)| (first, last, value.to_owned(), starts))
                .collect();
        }
        window.series.retain(|series| !series.runs.is_empty());
        Ok(())
    }
}

/// A daily window built day by day from values a query computes at each day's
/// endpoint (values that depend on the asker, such as one viewer's control).
#[derive(Default)]
pub struct Builder {
    known: Vec<HourSpan>,
    reads: Vec<(i32, i32, i32)>,
    series: BTreeMap<String, Vec<KeyedRun>>,
}

impl Builder {
    /// `day`'s values, read at `read`; days must come in order.
    pub fn push(&mut self, day: &Day, read: i32, values: impl IntoIterator<Item = (String, i128)>) {
        match self.known.last_mut() {
            Some(span) if span.last_hour.checked_add(1) == Some(day.first) => {
                span.last_hour = day.last
            }
            _ => self.known.push(HourSpan {
                first_hour: day.first,
                last_hour: day.last,
            }),
        }
        match self.reads.last_mut() {
            Some(previous)
                if previous.1.checked_add(1) == Some(day.first) && previous.2 == read =>
            {
                previous.1 = day.last
            }
            _ => self.reads.push((day.first, day.last, read)),
        }
        for (name, value) in values {
            let runs = self.series.entry(name).or_default();
            let value = value.to_string();
            let adjacent = runs
                .last()
                .is_some_and(|run| run.1.checked_add(1) == Some(day.first));
            match runs.last_mut() {
                Some(previous) if adjacent && previous.2 == value => previous.1 = day.last,
                _ => runs.push((day.first, day.last, value, !adjacent)),
            }
        }
    }

    pub fn finish(self, first_hour: i32, last_hour: i32) -> KeyedWindow {
        KeyedWindow {
            first_hour,
            last_hour,
            known: self.known,
            reads: self.reads,
            series: self
                .series
                .into_iter()
                .map(|(name, runs)| KeyedSeries { name, runs })
                .collect(),
        }
    }
}
