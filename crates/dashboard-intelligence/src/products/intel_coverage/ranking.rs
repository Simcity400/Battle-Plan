//! Dated network counts from sparse pair-presence runs, never hourly seeks.

use super::intersect_spans;
use dashboard_base::{
    metadata::check_cancel,
    prepared::series::{Accumulator, Run},
    products::country_groups::HourSpan,
};
use dashboard_keyed::query::{KeyedSeries, KeyedWindow};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

pub(super) fn series(
    evidence: &KeyedWindow,
    known: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<KeyedSeries>> {
    let mut deltas: BTreeMap<&str, BTreeMap<i64, i128>> = evidence
        .series
        .iter()
        .filter_map(|series| series.name.strip_prefix("member|"))
        .map(|part| (part, BTreeMap::new()))
        .collect();
    for series in &evidence.series {
        check_cancel(cancelled)?;
        let mut parts = series.name.split('|');
        let (Some(owner), Some(target), Some("coveredControlled")) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        for part in [owner, target] {
            let Some(changes) = deltas.get_mut(part) else {
                continue;
            };
            // A zero cache still witnesses one directed network.
            for run in &series.runs {
                *changes.entry(i64::from(run.0)).or_default() += 1;
                *changes.entry(i64::from(run.1) + 1).or_default() -= 1;
            }
        }
    }
    let mut ranked = Vec::new();
    for member in &evidence.series {
        let Some(part) = member.name.strip_prefix("member|") else {
            continue;
        };
        check_cancel(cancelled)?;
        let mut accumulator = Accumulator::default();
        let mut changes = deltas
            .remove(part)
            .expect("member count changes")
            .into_iter()
            .peekable();
        let mut count = 0;
        for run in &member.runs {
            let membership = [HourSpan {
                first_hour: run.0,
                last_hour: run.1,
            }];
            let start = known.partition_point(|span| span.last_hour < run.0);
            let stop = known.partition_point(|span| span.first_hour <= run.1);
            for span in intersect_spans(&known[start..stop], &membership) {
                while changes
                    .peek()
                    .is_some_and(|(hour, _)| *hour <= i64::from(span.first_hour))
                {
                    count += changes.next().expect("reached count change").1;
                }
                let mut first = span.first_hour;
                let mut starts = run.3 || first > run.0;
                while changes
                    .peek()
                    .is_some_and(|(hour, _)| *hour <= i64::from(span.last_hour))
                {
                    let (hour, delta) = changes.next().expect("reached count change");
                    let hour = hour as i32;
                    accumulator.push(0, Run::new(first, hour - 1, count, starts));
                    first = hour;
                    starts = false;
                    count += delta;
                }
                accumulator.push(0, Run::new(first, span.last_hour, count, starts));
            }
        }
        let runs = accumulator
            .pending_runs(0)
            .iter()
            .map(|run| {
                (
                    run.first_hour,
                    run.last_hour,
                    run.value.to_string(),
                    run.break_before,
                )
            })
            .collect::<Vec<_>>();
        if !runs.is_empty() {
            ranked.push(KeyedSeries {
                name: format!("rank|{part}"),
                runs,
            });
        }
    }
    Ok(ranked)
}

/// Union of the top countries at each change, with country-key tie breaking.
pub(super) fn shown_anywhere(
    ranking: &[KeyedSeries],
    shown: usize,
    cancelled: &AtomicBool,
) -> io::Result<BTreeSet<String>> {
    let mut events: BTreeMap<i64, Vec<(&str, Option<i128>)>> = BTreeMap::new();
    for series in ranking {
        check_cancel(cancelled)?;
        let part = series.name.strip_prefix("rank|").expect("ranking series");
        for run in &series.runs {
            let count = run.2.parse::<i128>().map_err(io::Error::other)?;
            events
                .entry(i64::from(run.0))
                .or_default()
                .push((part, Some(count)));
            events
                .entry(i64::from(run.1) + 1)
                .or_default()
                .push((part, None));
        }
    }
    let mut counts = BTreeMap::new();
    let mut ordered = BTreeSet::new();
    let mut chosen = BTreeSet::new();
    for changes in events.values() {
        check_cancel(cancelled)?;
        // Remove ended runs before adding their successors at the same hour.
        for (part, _) in changes {
            if let Some(count) = counts.remove(part) {
                ordered.remove(&(Reverse(count), *part));
            }
        }
        for &(part, count) in changes {
            if let Some(count) = count {
                counts.insert(part, count);
                ordered.insert((Reverse(count), part));
            }
        }
        chosen.extend(
            ordered
                .iter()
                .take(shown)
                .map(|(_, part)| (*part).to_owned()),
        );
        if chosen.len() == ranking.len() {
            break;
        }
    }
    Ok(chosen)
}
