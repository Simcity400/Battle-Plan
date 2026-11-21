
use crate::span::HourSpan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AmountRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub amount_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadRuns<K: Ord + Copy> {
    pub runs: BTreeMap<K, Vec<AmountRun>>,
    pub spans: Vec<HourSpan>,
    open: BTreeMap<K, Option<i64>>,
    pub overflowed: usize,
    carried: bool,
    frontier: Option<i32>,
    #[serde(skip)]
    changes: Option<BTreeSet<K>>,
    #[cfg(test)]
    #[serde(skip)]
    open_visits: usize,
}

impl<K: Ord + Copy> ReadRuns<K> {
    pub fn new() -> Self {
        Self {
            runs: BTreeMap::new(),
            spans: Vec::new(),
            open: BTreeMap::new(),
            overflowed: 0,
            carried: false,
            frontier: None,
            changes: None,
            #[cfg(test)]
            open_visits: 0,
        }
    }

    pub fn read(&mut self, hour: i32, values: impl IntoIterator<Item = (K, Option<i64>)>) {
        self.seal();
        let follows = self.carried
            && self
                .spans
                .last()
                .is_some_and(|span| span.last_hour.checked_add(1) == Some(hour));
        match self.spans.last_mut() {
            Some(span) if follows => span.last_hour = hour,
            _ => self.spans.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            }),
        }
        let mut sums = BTreeMap::<K, Option<i128>>::new();
        for (key, value) in values {
            let total = sums.entry(key).or_insert(Some(0));
            *total = total
                .zip(value)
                .and_then(|(sum, value)| sum.checked_add(i128::from(value)));
        }
        let mut totals = BTreeMap::<K, Option<i64>>::new();
        for (key, sum) in sums {
            let total = sum.and_then(|sum| {
                let fits = i64::try_from(sum).ok();
                self.overflowed += usize::from(fits.is_none());
                fits
            });
            totals.insert(key, total);
        }
        for (&key, &value) in &totals {
            let runs = self.runs.entry(key).or_default();
            match runs.last_mut() {
                Some(run)
                    if follows
                        && self.open.get(&key) == Some(&value)
                        && run.last_hour.checked_add(1) == Some(hour) =>
                {
                    run.last_hour = hour
                }
                _ => {
                    if let Some(changes) = &mut self.changes {
                        changes.insert(key);
                    }
                    runs.push(AmountRun {
                        first_hour: hour,
                        last_hour: hour,
                        amount_raw: value.map(|value| value.to_string()),
                    });
                }
            }
        }
        if let Some(changes) = &mut self.changes {
            let mut current = totals.keys().peekable();
            for &key in self.open.keys() {
                while current.peek().is_some_and(|&&next| next < key) {
                    current.next();
                }
                if current.peek().is_none_or(|&&next| next != key) {
                    changes.insert(key);
                }
            }
        }
        self.open = totals;
        self.carried = true;
        self.frontier = Some(hour);
    }

    pub fn advance(&mut self, hour: i32) {
        if let Some(frontier) = self.frontier
            && frontier != hour
            && frontier.checked_add(1) != Some(hour)
        {
            self.close();
        }
        self.frontier = Some(hour);
    }

    pub fn advance_span(&mut self, first: i32, last: i32) {
        debug_assert!(first <= last);
        self.advance(first);
        self.frontier = Some(last);
    }

    pub fn seal(&mut self) {
        if !self.carried {
            return;
        }
        let Some(frontier) = self.frontier else {
            return;
        };
        let Some(span) = self.spans.last_mut() else {
            return;
        };
        if span.last_hour >= frontier {
            return;
        }
        span.last_hour = frontier;
        for key in self.open.keys() {
            #[cfg(test)]
            {
                self.open_visits += 1;
            }
            if let Some(run) = self.runs.get_mut(key).and_then(|runs| runs.last_mut()) {
                run.last_hour = frontier;
            }
        }
    }

    pub fn carry(&mut self, hour: i32) {
        self.seal();
        if !self.carried {
            return;
        }
        let Some(span) = self.spans.last_mut() else {
            return;
        };
        if span.last_hour.checked_add(1) != Some(hour) {
            self.close();
            return;
        }
        span.last_hour = hour;
        self.frontier = Some(hour);
        for key in self.open.keys() {
            if let Some(run) = self.runs.get_mut(key).and_then(|runs| runs.last_mut())
                && run.last_hour.checked_add(1) == Some(hour)
            {
                run.last_hour = hour;
            }
        }
    }

    pub fn close(&mut self) {
        self.seal();
        self.carried = false;
        if let Some(changes) = &mut self.changes {
            changes.extend(self.open.keys().copied());
        }
        self.open.clear();
    }

    pub fn take_changes(&mut self) -> Option<BTreeSet<K>> {
        self.changes.replace(BTreeSet::new())
    }

    pub fn changed_keys(&self) -> Option<&BTreeSet<K>> {
        self.changes.as_ref()
    }

    pub fn has_changes(&self) -> Option<bool> {
        self.changes.as_ref().map(|changes| !changes.is_empty())
    }
}

impl<K: Ord + Copy> Default for ReadRuns<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_preserve_hourly_frontiers_carries_gaps_and_restores() {
        let mut hourly = ReadRuns::new();
        hourly.read(-10, [(1, Some(7)), (2, None)]);
        let mut spans = hourly.clone();
        for (first, last) in [(-9, 5), (8, 12), (13, 40)] {
            for hour in first..=last {
                hourly.advance(hour);
            }
            spans.advance_span(first, last);
            hourly.seal();
            spans.seal();
            assert_eq!(
                serde_json::to_vec(&spans).unwrap(),
                serde_json::to_vec(&hourly).unwrap()
            );
            for rows in [&mut hourly, &mut spans] {
                *rows = serde_json::from_slice(&serde_json::to_vec(&*rows).unwrap()).unwrap();
                rows.read(last + 1, [(1, Some(7))]);
            }
        }
    }

    #[test]
    fn deferred_carry_seals_at_reads_gaps_publication_resume_and_append() {
        let mut hourly = ReadRuns::new();
        let mut deferred = ReadRuns::new();
        for run in [&mut hourly, &mut deferred] {
            run.read(10, [(1, Some(0)), (2, None)]);
        }
        for hour in 11..=20 {
            hourly.carry(hour);
            deferred.advance(hour);
        }
        assert_eq!(
            deferred.runs[&1][0].last_hour, 10,
            "advance must not walk keys"
        );
        deferred.seal();
        assert_eq!(deferred.runs, hourly.runs);
        assert_eq!(deferred.spans, hourly.spans);
        deferred = serde_json::from_slice(&serde_json::to_vec(&deferred).unwrap()).unwrap();
        for run in [&mut hourly, &mut deferred] {
            run.read(21, [(1, Some(3))]);
        }
        for hour in 22..=30 {
            hourly.carry(hour);
            deferred.advance(hour);
        }
        hourly.carry(32);
        deferred.advance(32);
        assert_eq!(deferred.runs, hourly.runs);
        assert_eq!(deferred.spans, hourly.spans);
        for run in [&mut hourly, &mut deferred] {
            run.read(33, [(1, Some(3))]);
        }
        for hour in 34..=40 {
            hourly.carry(hour);
            deferred.advance(hour);
        }
        deferred.seal();
        deferred = serde_json::from_slice(&serde_json::to_vec(&deferred).unwrap()).unwrap();
        for hour in 41..=45 {
            hourly.carry(hour);
            deferred.advance(hour);
        }
        deferred.close();
        hourly.close();
        assert_eq!(deferred.runs, hourly.runs);
        assert_eq!(deferred.spans, hourly.spans);
        assert_eq!(deferred.runs[&2][0].last_hour, 20);
        assert_eq!(deferred.runs[&1].last().unwrap().last_hour, 45);
    }

    fn runs(built: &ReadRuns<u32>, key: u32) -> Vec<(i32, i32, Option<&str>)> {
        built.runs.get(&key).map_or_else(Vec::new, |runs| {
            runs.iter()
                .map(|run| (run.first_hour, run.last_hour, run.amount_raw.as_deref()))
                .collect()
        })
    }

    #[test]
    fn every_recorded_change_is_kept_past_the_old_stockpile_run_budget() {
        let mut built = ReadRuns::new();
        for hour in 0..201 {
            built.read(hour, (0..5_000).map(|key| (key, Some(i64::from(hour)))));
        }
        assert_eq!(built.runs.len(), 5_000);
        assert_eq!(built.runs.values().map(Vec::len).sum::<usize>(), 1_005_000);
        assert_eq!(runs(&built, 4_999).last(), Some(&(200, 200, Some("200"))));
    }

    #[test]
    fn values_start_at_reads_carry_until_the_next_and_a_missing_key_ends_its_run() {
        let mut built = ReadRuns::new();
        built.read(10, [(1, Some(5)), (2, Some(7)), (1, Some(3))]);
        built.carry(11);
        built.read(12, [(1, Some(8))]);
        built.carry(13);
        assert_eq!(runs(&built, 1), [(10, 13, Some("8"))]);
        assert_eq!(runs(&built, 2), [(10, 11, Some("7"))]);
        assert_eq!(
            built.spans,
            [HourSpan {
                first_hour: 10,
                last_hour: 13
            }]
        );
    }

    #[test]
    fn a_recorded_zero_is_a_value_and_an_uncomputable_one_poisons_the_sum() {
        let mut built = ReadRuns::new();
        built.read(10, [(1, Some(0)), (2, Some(4)), (2, None)]);
        built.carry(11);
        built.read(12, [(1, Some(0)), (2, Some(4))]);
        assert_eq!(runs(&built, 1), [(10, 12, Some("0"))]);
        assert_eq!(runs(&built, 2), [(10, 11, None), (12, 12, Some("4"))]);
        assert!(runs(&built, 3).is_empty());
    }

    #[test]
    fn a_break_ends_every_value_and_the_next_read_starts_a_new_span() {
        let mut built = ReadRuns::new();
        built.read(10, [(1, Some(5))]);
        built.close();
        built.carry(11);
        built.read(12, [(1, Some(5))]);
        built.carry(14);
        built.carry(15);
        assert_eq!(runs(&built, 1), [(10, 10, Some("5")), (12, 12, Some("5"))]);
        assert_eq!(
            built.spans,
            [
                HourSpan {
                    first_hour: 10,
                    last_hour: 10
                },
                HourSpan {
                    first_hour: 12,
                    last_hour: 12
                }
            ]
        );
        let mut exact = ReadRuns::new();
        exact.read(
            1,
            [
                (1, Some(i64::MAX)),
                (1, Some(100_000)),
                (1, Some(-i64::MAX)),
            ],
        );
        assert_eq!(runs(&exact, 1), [(1, 1, Some("100000"))]);
        exact.read(2, [(1, Some(i64::MAX)), (1, Some(1))]);
        assert_eq!(runs(&exact, 1)[1], (2, 2, None));
        assert_eq!(exact.overflowed, 1);
    }
}
