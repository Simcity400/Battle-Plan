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
#[derive(Clone, Debug)]
pub struct ReadRuns<K: Ord + Copy> {
    pub runs: BTreeMap<K, Vec<AmountRun>>,
    /// Hours the table's values are known: each read and the hours carried from it.
    pub spans: Vec<HourSpan>,
    /// The keys whose last run is still carried, and their values (none: unknown).
    open: BTreeMap<K, Option<i64>>,
    /// Totals that did not fit the table's i64 and were kept unknown, never clamped.
    pub overflowed: usize,
    carried: bool,
    count: usize,
    limit: usize,
}

/// More runs than a product's budget allows.
#[derive(Debug, PartialEq, Eq)]
pub struct OverBudget;

impl<K: Ord + Copy> ReadRuns<K> {
    /// No runs yet; `limit` bounds the runs kept, far above a long campaign's changes.
    pub fn new(limit: usize) -> Self {
        Self {
            runs: BTreeMap::new(),
            spans: Vec::new(),
            open: BTreeMap::new(),
            overflowed: 0,
            carried: false,
            count: 0,
            limit,
        }
    }

    /// A read at `hour`: every key's value then. A key read twice sums exactly, as a
    /// pool with repeated entries holds their total; one value that cannot be computed
    /// (`None`), or a total that does not fit the table's i64, makes the key's total
    /// unknown for this read (the latter counted in `overflowed`), never clamped. A key
    /// missing from the read had no row. A value continues its run only when the hour before was
    /// carried and the value is unchanged.
    pub fn read(
        &mut self,
        hour: i32,
        values: impl IntoIterator<Item = (K, Option<i64>)>,
    ) -> Result<(), OverBudget> {
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
                    self.count += 1;
                    if self.count > self.limit {
                        return Err(OverBudget);
                    }
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
        Ok(())
    }

    /// An hour carried from the last read: its values and span go on, but only
    /// while nothing broke since that read.
    pub fn carry(&mut self, hour: i32) {
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
        self.carried = false;
        self.open.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(built: &ReadRuns<u32>, key: u32) -> Vec<(i32, i32, Option<&str>)> {
        built.runs.get(&key).map_or_else(Vec::new, |runs| {
            runs.iter()
                .map(|run| (run.first_hour, run.last_hour, run.amount_raw.as_deref()))
                .collect()
        })
    }

    #[test]
    fn values_start_at_reads_carry_until_the_next_and_a_missing_key_ends_its_run() {
        let mut built = ReadRuns::new(100);
        built
            .read(10, [(1, Some(5)), (2, Some(7)), (1, Some(3))])
            .unwrap();
        built.carry(11);
        // Unchanged at the next read: one run; key 2 has no row, so its run ends.
        built.read(12, [(1, Some(8))]).unwrap();
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
        let mut built = ReadRuns::new(100);
        // A zero is kept as a run, told apart from a key never listed.
        built
            .read(10, [(1, Some(0)), (2, Some(4)), (2, None)])
            .unwrap();
        built.carry(11);
        built.read(12, [(1, Some(0)), (2, Some(4))]).unwrap();
        assert_eq!(runs(&built, 1), [(10, 12, Some("0"))]);
        // One uncomputable value makes the key's total unknown at that read, not 4.
        assert_eq!(runs(&built, 2), [(10, 11, None), (12, 12, Some("4"))]);
        assert!(runs(&built, 3).is_empty());
    }

    #[test]
    fn a_break_ends_every_value_and_the_next_read_starts_a_new_span() {
        let mut built = ReadRuns::new(100);
        built.read(10, [(1, Some(5))]).unwrap();
        built.close();
        // Hours after a break are unknown, not carried.
        built.carry(11);
        built.read(12, [(1, Some(5))]).unwrap();
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
        // The budget refuses runs past it.
        let mut exact = ReadRuns::new(100);
        // Intermediate sums past i64 do not clamp: the total is exactly 100000.
        exact
            .read(
                1,
                [
                    (1, Some(i64::MAX)),
                    (1, Some(100_000)),
                    (1, Some(-i64::MAX)),
                ],
            )
            .unwrap();
        assert_eq!(runs(&exact, 1), [(1, 1, Some("100000"))]);
        // A total that does not fit is unknown and counted, never clamped.
        exact.read(2, [(1, Some(i64::MAX)), (1, Some(1))]).unwrap();
        assert_eq!(runs(&exact, 1)[1], (2, 2, None));
        assert_eq!(exact.overflowed, 1);
        let mut small = ReadRuns::new(1);
        assert_eq!(small.read(1, [(2, Some(1))]), Ok(()));
        assert_eq!(small.read(3, [(2, Some(2))]), Err(OverBudget));
    }
}
