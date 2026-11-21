
use serde::Serialize;
#[cfg(any(test, feature = "test-support"))]
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    io,
};

#[derive(Default)]
pub struct Distribution {
    samples_ns: Vec<u64>,
    pub total_ns: u64,
    cached: Cell<Option<Percentiles>>,
    scratch: RefCell<Vec<u64>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Percentiles {
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

fn exact_percentiles(samples: &mut [u64]) -> Percentiles {
    if samples.is_empty() {
        return Percentiles::default();
    }
    let max_ns = samples.iter().copied().max().unwrap();
    let ranks = [50, 95, 99].map(|percent| {
        samples.len() / 100 * percent + (samples.len() % 100 * percent).div_ceil(100) - 1
    });
    let mut values = [0; 3];
    let mut remaining = samples;
    let mut offset = 0;
    let mut selected = 0;
    for (rank, value) in ranks.into_iter().zip(&mut values) {
        if rank >= offset {
            let (_, at_rank, suffix) = remaining.select_nth_unstable(rank - offset);
            selected = *at_rank;
            remaining = suffix;
            offset = rank + 1;
        }
        *value = selected;
    }
    Percentiles {
        p50_ns: values[0],
        p95_ns: values[1],
        p99_ns: values[2],
        max_ns,
    }
}

impl Distribution {
    pub fn record_ticks(&mut self, ticks: i64, frequency: u64) {
        let ns = ((ticks.max(0) as u128 * 1_000_000_000) / u128::from(frequency)) as u64;
        self.record_ns(ns);
    }

    pub fn record_ns(&mut self, ns: u64) {
        if engine_allocation::reserve(&mut self.samples_ns, 1).is_err() {
            return;
        }
        self.total_ns = self.total_ns.saturating_add(ns);
        self.samples_ns.push(ns);
        self.cached.set(None);
    }

    pub fn try_percentiles(&self) -> io::Result<Percentiles> {
        if let Some(cached) = self.cached.get() {
            return Ok(cached);
        }
        let mut scratch = self.scratch.borrow_mut();
        scratch.clear();
        engine_allocation::reserve(&mut scratch, self.samples_ns.len())?;
        scratch.extend_from_slice(&self.samples_ns);
        let result = exact_percentiles(&mut scratch);
        self.cached.set(Some(result));
        Ok(result)
    }

    pub fn status(&self) -> io::Result<DistributionStatus> {
        let percentiles = self.try_percentiles()?;
        let count = self.samples_ns.len();
        Ok(DistributionStatus {
            calls: count,
            total_ns: self.total_ns,
            mean_ns: if count == 0 {
                0.0
            } else {
                self.total_ns as f64 / count as f64
            },
            p50_ns: percentiles.p50_ns,
            p95_ns: percentiles.p95_ns,
            p99_ns: percentiles.p99_ns,
            max_ns: percentiles.max_ns,
        })
    }
    #[cfg(test)]
    fn percentiles(&self) -> Percentiles {
        self.try_percentiles().unwrap()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn json(&self) -> Value {
        serde_json::to_value(self.status().unwrap()).unwrap()
    }
}

#[derive(Clone, Copy, Default, Serialize)]
pub struct DistributionStatus {
    calls: usize,
    total_ns: u64,
    mean_ns: f64,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn work_ticks_use_nanoseconds_and_nearest_rank_percentiles() {
        let mut values = Distribution::default();
        assert_eq!(values.json()["calls"], 0);
        for ticks in [4, 1, 3, 2] {
            values.record_ticks(ticks, 10_000_000);
        }
        let report = values.json();
        assert_eq!(report["calls"], 4);
        assert_eq!(report["total_ns"], 1_000);
        assert_eq!(report["mean_ns"], 250.0);
        assert_eq!(report["p50_ns"], 200);
        assert_eq!(report["p95_ns"], 400);
    }

    #[test]
    fn selected_percentiles_match_sorted_ranks_for_small_tied_and_extreme_samples() {
        for len in 0..=257 {
            let mut input: Vec<_> = (0..len)
                .map(|index| match index % 5 {
                    0 => 0,
                    1 => u64::MAX,
                    2 => (index % 7) as u64,
                    3 => (len - index) as u64,
                    _ => 17,
                })
                .collect();
            let mut sorted = input.clone();
            sorted.sort_unstable();
            let at = |percent: usize| {
                if sorted.is_empty() {
                    0
                } else {
                    sorted[(sorted.len() * percent).div_ceil(100) - 1]
                }
            };
            let expected = Percentiles {
                p50_ns: at(50),
                p95_ns: at(95),
                p99_ns: at(99),
                max_ns: sorted.last().copied().unwrap_or(0),
            };
            assert_eq!(exact_percentiles(&mut input), expected, "length {len}");
            sorted.reverse();
            assert_eq!(
                exact_percentiles(&mut sorted),
                expected,
                "reverse length {len}"
            );
        }
    }

    #[test]
    fn cached_percentiles_refresh_after_samples_below_and_above_the_old_range() {
        let mut values = Distribution::default();
        assert_eq!(values.percentiles(), Percentiles::default());
        assert_eq!(values.cached.get(), Some(Percentiles::default()));
        for ns in [10, 20, 30] {
            values.record_ns(ns);
        }
        assert_eq!(values.cached.get(), None);
        let first = values.json();
        assert_eq!(
            first,
            json!({
                "calls": 3, "total_ns": 60, "mean_ns": 20.0,
                "p50_ns": 20, "p95_ns": 30, "p99_ns": 30, "max_ns": 30,
            })
        );
        let cached = values.cached.get();
        assert!(cached.is_some());
        assert_eq!(values.json(), first);
        assert_eq!(values.cached.get(), cached);

        values.record_ns(5);
        assert_eq!(values.cached.get(), None);
        assert_eq!(
            values.json(),
            json!({
                "calls": 4, "total_ns": 65, "mean_ns": 16.25,
                "p50_ns": 10, "p95_ns": 30, "p99_ns": 30, "max_ns": 30,
            })
        );
        values.record_ticks(100, 1_000_000_000);
        assert_eq!(values.cached.get(), None);
        let last = values.json();
        assert_eq!(
            last,
            json!({
                "calls": 5, "total_ns": 165, "mean_ns": 33.0,
                "p50_ns": 20, "p95_ns": 100, "p99_ns": 100, "max_ns": 100,
            })
        );
        assert_eq!(values.json(), last);
        assert_eq!(values.samples_ns, [10, 20, 30, 5, 100]);
    }
}
