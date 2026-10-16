//! Values of a daily table, keyed (by variant, archetype, …), as runs over the
//! hours each read covers: a value starts only at a read of its table, is
//! carried over the following hours of the same country identity, and ends at
//! a gap, an incomplete table or a change of identity. The spans say at which
//! hours the table's values are known at all.
//!
//! A run is a recorded value, zero included, so a key the table recorded as zero
//! is told apart from one it never listed: inside a span, a key without a run had
//! no row at that read (for a stockpile or a production total, none), and outside
//! every span its value is unknown, never zero. A value read but not computable
//! (such as a production rate with no cost) is a run with no amount: unknown for
//! that read, never zero and never part of a sum. Every product that turns a daily
//! table into runs builds them here.

use crate::products::country_groups::HourSpan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// One recorded value held over consecutive hours while unchanged. The raw value
/// is an exact decimal string of the table's fixed point; null when the value was
/// read but cannot be computed exactly, so it is unknown at those hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AmountRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub amount_raw: Option<String>,
}

/// Runs of one table's keyed values, built read by read.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadRuns<K: Ord + Copy> {
    /// Direct consumers must `seal()` after deferred `advance()` calls.
    pub runs: BTreeMap<K, Vec<AmountRun>>,
    /// Hours the table's values are known: each read and the hours carried from it.
    pub spans: Vec<HourSpan>,
    /// The keys whose last run is still carried, and their values (none: unknown).
    open: BTreeMap<K, Option<i64>>,
    /// Totals that did not fit the table's i64 and were kept unknown, never clamped.
    pub overflowed: usize,
    carried: bool,
    frontier: Option<i32>,
    #[cfg(test)]
    #[serde(skip)]
    open_visits: usize,
}

impl<K: Ord + Copy> ReadRuns<K> {
    /// No runs yet; each read adds the keys and changes it recorded.
    pub fn new() -> Self {
        Self {
            runs: BTreeMap::new(),
            spans: Vec::new(),
            open: BTreeMap::new(),
            overflowed: 0,
            carried: false,
            frontier: None,
            #[cfg(test)]
            open_visits: 0,
        }
    }

    /// A read at `hour`: every key's value then. A key read twice sums exactly, as a
    /// pool with repeated entries holds their total; one value that cannot be computed
    /// (`None`), or a total that does not fit the table's i64, makes the key's total
    /// unknown for this read (the latter counted in `overflowed`), never clamped. A key
    /// missing from the read had no row. A value continues its run only when the hour before was
    /// carried and the value is unchanged.
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
        // Exact sums: i128 holds any partial sum of i64 entries, so intermediate
        // totals never saturate; only the final total must fit.
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
                    runs.push(AmountRun {
                        first_hour: hour,
                        last_hour: hour,
                        amount_raw: value.map(|value| value.to_string()),
                    });
                }
            }
        }
        self.open = totals;
        self.carried = true;
        self.frontier = Some(hour);
    }

    /// Authenticate an unchanged hour without visiting the open keys. A missing
    /// hour closes at the previous frontier; it never extends a run over a gap.
    pub fn advance(&mut self, hour: i32) {
        if let Some(frontier) = self.frontier
            && frontier != hour
            && frontier.checked_add(1) != Some(hour)
        {
            self.close();
        }
        self.frontier = Some(hour);
    }

    /// Materialise effective endpoints before direct `runs`/`spans` access,
    /// publication or a checkpoint. Only currently open keys are visited.
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

    /// An hour carried from the last read: its values and span go on, but only
    /// while nothing broke since that read.
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

    /// The recording broke (a gap, a missing or incomplete table, another
    /// identity): nothing is carried until the next read.
    pub fn close(&mut self) {
        self.seal();
        self.carried = false;
        self.open.clear();
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
        // A missing hour closes the old endpoint before a new accepted frame.
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

    #[test]
    fn deferred_carry_work_is_flat_over_unchanged_frames_and_closed_history() {
        use crate::prepared::observer_tests::{self, Work};
        let frame = crate::test_fixtures::scale_frame(1, 0, 1);
        let format = crate::test_fixtures::replay(vec![frame.clone()]).format;
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |hours| {
                let mut runs = ReadRuns::new();
                runs.read(0, (0..100).map(|key| (key, Some(7))));
                let measured = observer_tests::unchanged_frames(
                    Work::default(),
                    &frame,
                    hours as i32,
                    format,
                    |frame, _changes| {
                        runs.advance(frame.hour);
                        Ok(Work {
                            history_visits: runs.open_visits,
                            ..Default::default()
                        })
                    },
                )?;
                assert_eq!(measured.history_visits, 0);
                runs.seal();
                assert_eq!(runs.open_visits, 100);
                Ok(measured)
            })
            .unwrap(),
        );
        observer_tests::assert_flat(
            observer_tests::scaled_work(100, |history| {
                let mut runs = ReadRuns::new();
                runs.read(0, (0..history as u32).map(|key| (key, Some(7))));
                runs.close();
                runs.read(1, [(u32::MAX, Some(8))]);
                runs.advance(2);
                runs.seal();
                Ok(Work {
                    history_visits: runs.open_visits,
                    ..Default::default()
                })
            })
            .unwrap(),
        );
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
        // Unchanged at the next read: one run; key 2 has no row, so its run ends.
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
        // A zero is kept as a run, told apart from a key never listed.
        built.read(10, [(1, Some(0)), (2, Some(4)), (2, None)]);
        built.carry(11);
        built.read(12, [(1, Some(0)), (2, Some(4))]);
        assert_eq!(runs(&built, 1), [(10, 12, Some("0"))]);
        // One uncomputable value makes the key's total unknown at that read, not 4.
        assert_eq!(runs(&built, 2), [(10, 11, None), (12, 12, Some("4"))]);
        assert!(runs(&built, 3).is_empty());
    }

    #[test]
    fn a_break_ends_every_value_and_the_next_read_starts_a_new_span() {
        let mut built = ReadRuns::new();
        built.read(10, [(1, Some(5))]);
        built.close();
        // Hours after a break are unknown, not carried.
        built.carry(11);
        built.read(12, [(1, Some(5))]);
        // A carried hour that skips one is a break too.
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
        // Intermediate sums past i64 do not clamp: the total is exactly 100000.
        exact.read(
            1,
            [
                (1, Some(i64::MAX)),
                (1, Some(100_000)),
                (1, Some(-i64::MAX)),
            ],
        );
        assert_eq!(runs(&exact, 1), [(1, 1, Some("100000"))]);
        // A total that does not fit is unknown and counted, never clamped.
        exact.read(2, [(1, Some(i64::MAX)), (1, Some(1))]);
        assert_eq!(runs(&exact, 1)[1], (2, 2, None));
        assert_eq!(exact.overflowed, 1);
    }
}
