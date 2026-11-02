//! Dated warning counts, each reason kept once with the runs of hours its count holds for.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, btree_map::Entry};

/// Hours `.0` through `.1`, each counting `.2` of a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Run(i32, i32, usize);

/// Warning counts by hour and reason. A reason's runs ascend apart from each
/// other, and adjacent ones differ in count, so equal counts hold equal runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Warnings(BTreeMap<String, Vec<Run>>);

impl Warnings {
    pub(super) fn add(&mut self, hour: i32, reason: impl AsRef<str> + Into<String>, count: usize) {
        match self.0.get_mut(reason.as_ref()) {
            Some(runs) => add(runs, hour, hour, count),
            None => add(self.0.entry(reason.into()).or_default(), hour, hour, count),
        }
    }
    /// Adds every count of `other`.
    pub(super) fn merge(&mut self, other: Self) {
        for (reason, runs) in other.0 {
            match self.0.entry(reason) {
                Entry::Vacant(entry) => {
                    entry.insert(runs);
                }
                Entry::Occupied(mut entry) => {
                    for run in runs {
                        add(entry.get_mut(), run.0, run.1, run.2);
                    }
                }
            }
        }
    }
    /// The counts dated `first` through `last`.
    pub(super) fn between(&self, first: i32, last: i32) -> Self {
        if first > last {
            return Self::default();
        }
        Self(
            self.0
                .iter()
                .filter_map(|(reason, runs)| {
                    let start = runs.partition_point(|run| run.1 < first);
                    let held: Vec<_> = runs[start..]
                        .iter()
                        .take_while(|run| run.0 <= last)
                        .map(|run| Run(run.0.max(first), run.1.min(last), run.2))
                        .collect();
                    (!held.is_empty()).then(|| (reason.clone(), held))
                })
                .collect(),
        )
    }
    /// Drops the counts dated before `hour`.
    pub(super) fn retain_from(&mut self, hour: i32) {
        self.0.retain(|_, runs| {
            let start = runs.partition_point(|run| run.1 < hour);
            runs.drain(..start);
            if let Some(run) = runs.first_mut() {
                run.0 = run.0.max(hour);
            }
            !runs.is_empty()
        });
    }
    /// Each hour's count of each reason, in hour then reason order.
    pub(super) fn hourly(&self) -> Vec<(i32, &str, usize)> {
        let runs = || self.0.values().flatten();
        let (Some(first), Some(last)) =
            (runs().map(|run| run.0).min(), runs().map(|run| run.1).max())
        else {
            return Vec::new();
        };
        // Reasons in order into each hour's list: hour then reason order without comparing text.
        let mut hours: Vec<Vec<(&str, usize)>> =
            vec![Vec::new(); (i64::from(last) - i64::from(first) + 1) as usize];
        for (reason, runs) in &self.0 {
            for run in runs {
                for hour in run.0..=run.1 {
                    hours[(i64::from(hour) - i64::from(first)) as usize]
                        .push((reason.as_str(), run.2));
                }
            }
        }
        (first..=last)
            .zip(hours)
            .flat_map(|(hour, reasons)| {
                reasons
                    .into_iter()
                    .map(move |(reason, count)| (hour, reason, count))
            })
            .collect()
    }
    /// Each reason's first hour and total count, in reason order.
    pub(super) fn totals(&self) -> impl Iterator<Item = (&str, i32, usize)> {
        self.0.iter().filter_map(|(reason, runs)| {
            let first = runs.first()?.0;
            let total = runs
                .iter()
                .map(|run| (i64::from(run.1) - i64::from(run.0) + 1) as usize * run.2)
                .sum();
            Some((reason.as_str(), first, total))
        })
    }
    /// The runs held.
    pub(super) fn len(&self) -> usize {
        self.0.values().map(Vec::len).sum()
    }
}

/// Adds `count` to every hour `first` through `last` of `runs`.
fn add(runs: &mut Vec<Run>, first: i32, last: i32, count: usize) {
    // A reason warned again in the newest hour it counts.
    if let Some(run) = runs.last_mut()
        && first == last
        && run.1 == last
        && run.0 <= first
    {
        if count == 0 {
            return;
        }
        if run.0 < first {
            run.1 = first - 1;
            let held = run.2;
            runs.push(Run(first, last, held + count));
            return;
        }
        run.2 += count;
        if let [.., previous, run] = runs.as_mut_slice()
            && previous.2 == run.2
            && i64::from(previous.1) + 1 == i64::from(run.0)
        {
            previous.1 = run.1;
            runs.pop();
        }
        return;
    }
    if runs.last().is_none_or(|run| run.1 < first) {
        match runs.last_mut() {
            Some(run) if run.2 == count && i64::from(run.1) + 1 == i64::from(first) => {
                run.1 = last;
            }
            _ => runs.push(Run(first, last, count)),
        }
        return;
    }
    let start = runs.partition_point(|run| run.1 < first);
    let end = runs.partition_point(|run| run.0 <= last);
    let mut spliced = Vec::new();
    let mut next = i64::from(first);
    for run in &runs[start..end] {
        if run.0 < first {
            spliced.push(Run(run.0, first - 1, run.2));
        }
        let from = run.0.max(first);
        if next < i64::from(from) {
            spliced.push(Run(next as i32, from - 1, count));
        }
        let to = run.1.min(last);
        spliced.push(Run(from, to, run.2 + count));
        if run.1 > last {
            spliced.push(Run(last + 1, run.1, run.2));
        }
        next = i64::from(to) + 1;
    }
    if next <= i64::from(last) {
        spliced.push(Run(next as i32, last, count));
    }
    let added = spliced.len();
    runs.splice(start..end, spliced);
    // Rejoin equal neighbours from the run before the splice to the one after it.
    let low = start.saturating_sub(1);
    let high = (start + added + 1).min(runs.len());
    let mut joined: Vec<Run> = Vec::with_capacity(high - low);
    for run in runs.drain(low..high) {
        match joined.last_mut() {
            Some(previous)
                if previous.2 == run.2 && i64::from(previous.1) + 1 == i64::from(run.0) =>
            {
                previous.1 = run.1;
            }
            _ => joined.push(run),
        }
    }
    runs.splice(low..low, joined);
}
