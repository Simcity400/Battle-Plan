//! The top N countries at every hour, for every product that ranks countries by
//! a value held over runs of hours (the race widgets: Factories, Equipment
//! produced). The product ranks here, so a reply carries only the countries ever
//! among the top N, each with the hours it was.

use crate::products::country_groups::HourSpan;
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Visit the intersection of ordered, nonoverlapping runs and spans in one
/// linear pass. A clipped piece retains its original supplying run; gaps and
/// read dates remain the caller's recorded facts. The visitor can refuse a
/// budget before copying the piece.
pub fn for_each_piece<R>(
    runs: &[R],
    spans: &[HourSpan],
    span: impl Fn(&R) -> (i32, i32),
    cancelled: &AtomicBool,
    mut piece: impl FnMut(&R, i32, i32) -> io::Result<()>,
) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    let (mut run_index, mut span_index) = (0, 0);
    let mut steps = 0usize;
    while let (Some(run), Some(selected)) = (runs.get(run_index), spans.get(span_index)) {
        steps += 1;
        if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let (first, last) = span(run);
        let first_hour = first.max(selected.first_hour);
        let last_hour = last.min(selected.last_hour);
        if first_hour <= last_hour {
            piece(run, first_hour, last_hour)?;
        }
        if last < selected.last_hour {
            run_index += 1;
        } else {
            span_index += 1;
        }
    }
    Ok(())
}

/// The hours each country was among the top N, by tag: at every hour a value can
/// change (a run starting or ending in any series), every country with a known
/// value then is ranked (highest first, ties by tag), so between two such hours
/// the top N stay the same. A country without a run at an hour, or whose value
/// there is unknown (`value` gives none), is not ranked there, never as zero.
///
/// Each series' runs are in hour order and do not overlap; `span` gives a run's
/// first and last hour.
pub fn top_membership<R>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i64>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let tags: Vec<(&String, &Vec<R>)> = series.iter().collect();
    let mut bounds: Vec<i32> = tags
        .iter()
        .flat_map(|(_, runs)| runs.iter())
        .flat_map(|run| {
            let (first, last) = span(run);
            [first, last.saturating_add(1)]
        })
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut cursors = vec![0usize; tags.len()];
    let mut spans: Vec<Vec<HourSpan>> = vec![Vec::new(); tags.len()];
    let mut present: Vec<(i64, usize)> = Vec::with_capacity(tags.len());
    for (step, window) in bounds.windows(2).enumerate() {
        if step.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The ranking was cancelled",
            ));
        }
        let (start, end) = (window[0], window[1] - 1);
        present.clear();
        for (index, (_, runs)) in tags.iter().enumerate() {
            let cursor = &mut cursors[index];
            while runs.get(*cursor).is_some_and(|run| span(run).1 < start) {
                *cursor += 1;
            }
            // Every run starts at a bound, so one holding `start` holds to `end`.
            if let Some(value) = runs
                .get(*cursor)
                .filter(|run| span(run).0 <= start)
                .and_then(&value)
            {
                present.push((value, index));
            }
        }
        // Highest first; tags are in order, so the index breaks ties by tag.
        let order = |a: &(i64, usize), b: &(i64, usize)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
        if count == 0 {
            present.clear();
        } else if present.len() > count {
            present.select_nth_unstable_by(count - 1, order);
            present.truncate(count);
        }
        for (_, index) in &present {
            let list = &mut spans[*index];
            match list.last_mut() {
                Some(span) if span.last_hour.checked_add(1) == Some(start) => span.last_hour = end,
                _ => list.push(HourSpan {
                    first_hour: start,
                    last_hour: end,
                }),
            }
        }
    }
    Ok(tags
        .into_iter()
        .zip(spans)
        .filter(|(_, spans)| !spans.is_empty())
        .map(|((tag, _), spans)| (tag.clone(), spans))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_clipping_preserves_gaps_read_origins_and_refusals() {
        let runs = [(0, 5), (7, 10), (20, 23)];
        let spans = [
            HourSpan {
                first_hour: 3,
                last_hour: 8,
            },
            HourSpan {
                first_hour: 10,
                last_hour: 21,
            },
        ];
        let mut pieces = Vec::new();
        for_each_piece(
            &runs,
            &spans,
            |run| *run,
            &AtomicBool::new(false),
            |run, first, last| {
                pieces.push((run.0, first, last));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(pieces, [(0, 3, 5), (7, 7, 8), (7, 10, 10), (20, 20, 21)]);
        let error = for_each_piece(
            &runs,
            &spans,
            |run| *run,
            &AtomicBool::new(false),
            |_, _, _| Err(io::Error::other("budget")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "budget");
        assert_eq!(
            for_each_piece(
                &runs,
                &spans,
                |run| *run,
                &AtomicBool::new(true),
                |_, _, _| Ok(())
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn ranked_piece_walk_preserves_gaps_at_5000_runs_and_cancels() {
        let runs: Vec<_> = (0..5000).map(|i| (i * 3, i * 3 + 1)).collect();
        let spans: Vec<_> = (0..5000)
            .map(|i| HourSpan {
                first_hour: i * 3 + 1,
                last_hour: i * 3 + 2,
            })
            .collect();
        let mut pieces = Vec::new();
        for_each_piece(
            &runs,
            &spans,
            |run| *run,
            &AtomicBool::new(false),
            |_, first, last| {
                pieces.push((first, last));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(pieces.len(), 5000);
        assert!(
            pieces
                .iter()
                .enumerate()
                .all(|(i, &(first, last))| first == i as i32 * 3 + 1 && last == first)
        );
        assert_eq!(
            for_each_piece(
                &runs,
                &spans,
                |run| *run,
                &AtomicBool::new(true),
                |_, _, _| panic!("canceled walk supplied a piece")
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn the_top_n_change_where_any_value_changes_and_unknown_values_are_not_ranked() {
        // (first, last, value); None is unknown.
        let series: BTreeMap<String, Vec<(i32, i32, Option<i64>)>> = [
            ("AAA".to_owned(), vec![(0, 9, Some(5))]),
            ("BBB".to_owned(), vec![(0, 4, Some(3)), (5, 9, Some(9))]),
            ("CCC".to_owned(), vec![(0, 2, None), (3, 9, Some(4))]),
        ]
        .into();
        let top = top_membership(
            &series,
            2,
            |run| (run.0, run.1),
            |run| run.2,
            &AtomicBool::new(false),
        )
        .unwrap();
        let spans = |tag: &str| -> Vec<(i32, i32)> {
            top.get(tag).map_or_else(Vec::new, |spans| {
                spans
                    .iter()
                    .map(|span| (span.first_hour, span.last_hour))
                    .collect()
            })
        };
        assert_eq!(spans("AAA"), [(0, 9)]);
        // Unknown at 0..=2 is not ranked, so BBB holds second place until CCC is known.
        assert_eq!(spans("BBB"), [(0, 2), (5, 9)]);
        assert_eq!(spans("CCC"), [(3, 4)]);
    }
}
