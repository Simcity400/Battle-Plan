//! The top N countries at every hour, for every product that ranks countries by
//! a value held over runs of hours (the race widgets: Factories, Equipment
//! produced). The product ranks here, so a reply carries only the countries ever
//! among the top N, each with the hours it was.

use crate::products::country_groups::HourSpan;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
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
    membership(
        series,
        count,
        span,
        |run| value(run).map(i128::from),
        true,
        false,
        cancelled,
    )
}

/// Exact table ranking in either direction. An active run with an unknown value
/// sorts after all known values; no run means the country is absent. Tags break
/// ties in recorded UTF-8 order. Callers supply identity spans independently of
/// their measure's coverage, so missing data never becomes a recorded zero.
pub fn top_membership_exact<R>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    descending: bool,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    membership(series, count, span, value, descending, true, cancelled)
}

// Reverse compares directly, so descending values retain both i128 extrema.
// Every set uses one direction; either known variant precedes Unknown.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum RankValue {
    Ascending(i128),
    Descending(Reverse<i128>),
    Unknown,
}

struct RankEvent {
    hour: i64,
    starts: bool,
    index: usize,
}

fn membership<R>(
    series: &BTreeMap<String, Vec<R>>,
    count: usize,
    span: impl Fn(&R) -> (i32, i32),
    value: impl Fn(&R) -> Option<i128>,
    descending: bool,
    include_unknown: bool,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    if count == 0 {
        return Ok(BTreeMap::new());
    }
    let tags: Vec<(&String, &Vec<R>)> = series.iter().collect();
    let mut events = Vec::new();
    let mut steps = 0usize;
    for (index, runs) in series.values().enumerate() {
        for run in runs {
            steps += 1;
            if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "The ranking was cancelled",
                ));
            }
            let (first, last) = span(run);
            events.push(RankEvent {
                hour: i64::from(first),
                starts: true,
                index,
            });
            events.push(RankEvent {
                hour: i64::from(last) + 1,
                starts: false,
                index,
            });
        }
    }
    // Ends precede starts at the same boundary, including adjacent equal runs.
    // i64 keeps the one-past-i32::MAX boundary distinct from recorded hours.
    events.sort_unstable_by_key(|event| (event.hour, event.starts));
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The ranking was cancelled",
        ));
    }
    let mut spans: Vec<Vec<HourSpan>> = vec![Vec::new(); tags.len()];
    let mut cursors = vec![0usize; tags.len()];
    let mut active = vec![None; tags.len()];
    let mut ranked = BTreeSet::new();
    let mut event_index = 0;
    while let Some(event) = events.get(event_index) {
        let hour = event.hour;
        while let Some(event) = events.get(event_index).filter(|event| event.hour == hour) {
            steps += 1;
            if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "The ranking was cancelled",
                ));
            }
            if event.starts {
                let amount = match value(&tags[event.index].1[cursors[event.index]]) {
                    Some(amount) if descending => Some(RankValue::Descending(Reverse(amount))),
                    Some(amount) => Some(RankValue::Ascending(amount)),
                    None if include_unknown => Some(RankValue::Unknown),
                    None => None,
                };
                active[event.index] = amount;
                if let Some(amount) = amount {
                    ranked.insert((amount, event.index));
                }
            } else {
                if let Some(amount) = active[event.index].take() {
                    ranked.remove(&(amount, event.index));
                }
                cursors[event.index] += 1;
            }
            event_index += 1;
        }
        let Some(next) = events.get(event_index) else {
            break;
        };
        let (start, end) = (hour as i32, (next.hour - 1) as i32);
        // Changed countries update the set; its first N supply membership.
        // The index breaks equal-value ties by the BTreeMap's UTF-8 tag order.
        for (_, index) in ranked.iter().take(count) {
            steps += 1;
            if steps.is_multiple_of(4096) && cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "The ranking was cancelled",
                ));
            }
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
    use std::cell::Cell;

    #[test]
    fn ranking_sparse_changes_read_each_run_once_at_many_boundaries() {
        let mut series: BTreeMap<_, _> = (0..5000)
            .map(|index| (format!("country/{index:04}"), vec![(0, 999, Some(0i128))]))
            .collect();
        series.insert(
            "moving".to_owned(),
            (0..1000)
                .map(|hour| (hour, hour, Some(if hour % 2 == 0 { 1 } else { -1 })))
                .collect(),
        );
        let reads = Cell::new(0);
        let top = top_membership_exact(
            &series,
            2,
            |run| (run.0, run.1),
            |run| {
                reads.set(reads.get() + 1);
                run.2
            },
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reads.get(), 6000);
        assert_eq!(top.len(), 3);
        assert_eq!(
            top["country/0000"],
            [HourSpan {
                first_hour: 0,
                last_hour: 999,
            }]
        );
        assert_eq!(top["moving"].len(), 500);
        assert!(top["moving"].iter().enumerate().all(|(index, span)| {
            span.first_hour == index as i32 * 2 && span.last_hour == span.first_hour
        }));
        assert_eq!(top["country/0001"].len(), 500);
        assert!(top["country/0001"].iter().enumerate().all(|(index, span)| {
            span.first_hour == index as i32 * 2 + 1 && span.last_hour == span.first_hour
        }));
    }

    #[test]
    fn ranking_event_walk_matches_hourly_reference_with_gaps_unknowns_and_ties() {
        let series: BTreeMap<String, Vec<(i32, i32, Option<i128>)>> = (0..64)
            .map(|index| {
                let tag = match index {
                    0 => "\u{10000}".to_owned(),
                    1 => "\u{e000}".to_owned(),
                    _ => format!("country/{index:02}"),
                };
                let runs = (0..96)
                    .filter(|hour| (hour + index) % 11 != 0)
                    .map(|hour| {
                        let amount = match (hour + index) % 19 {
                            0 => None,
                            1 => Some(i128::MIN),
                            2 => Some(i128::MAX),
                            _ => Some(i128::from((hour + index) % 7 - 3)),
                        };
                        (hour, hour, amount)
                    })
                    .collect();
                (tag, runs)
            })
            .collect();
        for descending in [false, true] {
            for include_unknown in [false, true] {
                for count in [0, 1, 7, 64, 99] {
                    let mut expected: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
                    for hour in 0..96 {
                        let mut present: Vec<_> = series
                            .iter()
                            .filter_map(|(tag, runs)| {
                                let amount = runs.iter().find(|run| run.0 == hour)?.2;
                                (include_unknown || amount.is_some()).then_some((tag, amount))
                            })
                            .collect();
                        present.sort_unstable_by(|a, b| {
                            match (a.1, b.1) {
                                (Some(a), Some(b)) if descending => b.cmp(&a),
                                (Some(a), Some(b)) => a.cmp(&b),
                                (Some(_), None) => std::cmp::Ordering::Less,
                                (None, Some(_)) => std::cmp::Ordering::Greater,
                                (None, None) => std::cmp::Ordering::Equal,
                            }
                            .then(a.0.cmp(b.0))
                        });
                        for (tag, _) in present.into_iter().take(count) {
                            let spans = expected.entry(tag.clone()).or_default();
                            match spans.last_mut() {
                                Some(span) if span.last_hour + 1 == hour => span.last_hour = hour,
                                _ => spans.push(HourSpan {
                                    first_hour: hour,
                                    last_hour: hour,
                                }),
                            }
                        }
                    }
                    let actual = membership(
                        &series,
                        count,
                        |run| (run.0, run.1),
                        |run| run.2,
                        descending,
                        include_unknown,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                    assert_eq!(
                        actual, expected,
                        "direction={descending}, unknown={include_unknown}, count={count}"
                    );
                }
            }
        }
    }

    #[test]
    fn ranking_events_preserve_i32_endpoints_and_i128_extrema() {
        let series = BTreeMap::from([
            (
                "A".to_owned(),
                vec![
                    (i32::MIN, i32::MIN, Some(i128::MIN)),
                    (i32::MIN + 1, i32::MAX - 1, Some(i128::MAX)),
                    (i32::MAX, i32::MAX, Some(i128::MIN)),
                ],
            ),
            ("B".to_owned(), vec![(i32::MIN, i32::MAX, Some(0))]),
        ]);
        for descending in [false, true] {
            let top = top_membership_exact(
                &series,
                1,
                |run| (run.0, run.1),
                |run| run.2,
                descending,
                &AtomicBool::new(false),
            )
            .unwrap();
            let endpoints = vec![
                HourSpan {
                    first_hour: i32::MIN,
                    last_hour: i32::MIN,
                },
                HourSpan {
                    first_hour: i32::MAX,
                    last_hour: i32::MAX,
                },
            ];
            let middle = vec![HourSpan {
                first_hour: i32::MIN + 1,
                last_hour: i32::MAX - 1,
            }];
            assert_eq!(&top["A"], if descending { &middle } else { &endpoints });
            assert_eq!(&top["B"], if descending { &endpoints } else { &middle });
        }
    }

    #[test]
    fn ranking_event_preparation_observes_cancellation() {
        let series = BTreeMap::from([(
            "country".to_owned(),
            (0..10000)
                .map(|hour| (hour, hour, Some(0i128)))
                .collect::<Vec<_>>(),
        )]);
        let cancelled = AtomicBool::new(false);
        let reads = Cell::new(0);
        let error = top_membership_exact(
            &series,
            1,
            |run| {
                reads.set(reads.get() + 1);
                cancelled.store(true, Ordering::Release);
                (run.0, run.1)
            },
            |run| run.2,
            true,
            &cancelled,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(reads.get() < 10000);
    }

    #[test]
    fn scoreboard_ranking_exact_missing_last_and_direction_changes() {
        let series = BTreeMap::from([
            ("A".to_owned(), vec![(0, 9, Some(i128::MAX))]),
            (
                "B".to_owned(),
                vec![(0, 4, Some(i128::MAX - 1)), (5, 9, None)],
            ),
            ("C".to_owned(), vec![(0, 9, None)]),
            ("D".to_owned(), vec![(0, 9, Some(0))]),
        ]);
        let top = |descending, count| {
            top_membership_exact(
                &series,
                count,
                |run| (run.0, run.1),
                |run| run.2,
                descending,
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        assert_eq!(top(true, 1).keys().cloned().collect::<Vec<_>>(), ["A"]);
        assert_eq!(top(false, 1).keys().cloned().collect::<Vec<_>>(), ["D"]);
        let filled = top(true, 3);
        assert_eq!(
            filled["B"],
            [HourSpan {
                first_hour: 0,
                last_hour: 9
            }]
        );
        assert!(!filled.contains_key("C"));
        assert!(top(false, 0).is_empty());
        let endpoint = BTreeMap::from([("edge".to_owned(), vec![(i32::MAX, i32::MAX, Some(1))])]);
        assert_eq!(
            top_membership_exact(
                &endpoint,
                1,
                |run| (run.0, run.1),
                |run| run.2,
                true,
                &AtomicBool::new(false)
            )
            .unwrap()["edge"],
            [HourSpan {
                first_hour: i32::MAX,
                last_hour: i32::MAX
            }]
        );
        assert_eq!(
            top_membership_exact(
                &series,
                1,
                |run| (run.0, run.1),
                |run| run.2,
                true,
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn scoreboard_ranking_5000_countries_opaque_tags_and_disappearance() {
        let series: BTreeMap<_, _> = (0..5000)
            .map(|index| {
                (
                    format!("country/{index:04}"),
                    vec![(
                        0,
                        if index == 0 { 4 } else { 9 },
                        if index < 2 { Some(0i128) } else { None },
                    )],
                )
            })
            .collect();
        for descending in [false, true] {
            let top = top_membership_exact(
                &series,
                3,
                |run| (run.0, run.1),
                |run| run.2,
                descending,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                top["country/0000"],
                [HourSpan {
                    first_hour: 0,
                    last_hour: 4
                }]
            );
            assert_eq!(
                top["country/0002"],
                [HourSpan {
                    first_hour: 0,
                    last_hour: 9
                }]
            );
            assert_eq!(
                top["country/0003"],
                [HourSpan {
                    first_hour: 5,
                    last_hour: 9
                }]
            );
            assert_eq!(top.len(), 4);
        }
        let tags = BTreeMap::from([
            ("\u{10000}".to_owned(), vec![(0, 1, None)]),
            ("\u{e000}".to_owned(), vec![(0, 1, None)]),
        ]);
        let top = top_membership_exact(
            &tags,
            1,
            |run| (run.0, run.1),
            |run| run.2,
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(top.contains_key("\u{e000}"));
    }

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
