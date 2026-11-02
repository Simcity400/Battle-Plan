//! Window-bounded group sums over prepared member values and recorded coverage.
use super::{
    series::{self, Run},
    series_read::SeriesRead,
};
use crate::{
    metadata::check_cancel, products::country_groups::HourSpan, protocol::MAX_EXACT_JS_INTEGER,
};
use std::{io, sync::atomic::AtomicBool};

/// Readers can use different catalogues for numeric values and presence proof.
pub struct Member<'a> {
    pub keys: &'a [(u64, i32, i32, Option<bool>)],
    pub present_keys: &'a [(u64, i32, i32, Option<bool>)],
    pub rank_key: Option<u64>,
    pub within: Option<&'a [HourSpan]>,
    pub excluded: &'a [HourSpan],
    pub zeros: &'a [HourSpan],
}

/// Exact windows sum dated values; dense windows add aligned member envelopes.
#[allow(clippy::too_many_arguments)]
pub fn summary(
    values: &mut dyn SeriesRead,
    present: &mut dyn SeriesRead,
    rosters: &mut dyn SeriesRead,
    coverage: Option<&crate::prepared_chart::ChartSummary>,
    mut ranks: Option<(&mut dyn SeriesRead, u64)>,
    members: &[Member<'_>],
    unobserved: &[HourSpan],
    view: (f64, f64, u32),
    cancelled: &AtomicBool,
) -> io::Result<crate::prepared_chart::ChartSummary> {
    use crate::prepared_chart::{ChartBucket, ChartRun, ChartSummary};
    let (from, to, width) = view;
    series::summary_budget(from, to, width, 1)?;
    let bounds = (from.floor() as i32, to.floor() as i32);
    if i64::from(bounds.1) - i64::from(bounds.0) < i64::from(width) {
        let roster = match coverage {
            Some(coverage) => coverage
                .runs
                .iter()
                .flatten()
                .map(|run| Run::new(run.first_hour, run.last_hour, 1, run.break_before))
                .collect(),
            None => rosters.runs(0, bounds.0, bounds.1)?,
        };
        let mut tracks = Vec::new();
        for member in members {
            check_cancel(cancelled)?;
            let rows = member_runs(values, member.keys, bounds)?;
            let presence = member_runs(present, member.present_keys, bounds)?;
            let ranked = if let Some((reader, _)) = &mut ranks {
                member
                    .rank_key
                    .map(|actor| {
                        member_runs(&mut **reader, &[(actor, i32::MIN, i32::MAX, None)], bounds)
                    })
                    .transpose()?
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            tracks.push((member, rows, presence, ranked));
        }
        fn at(runs: &[Run], hour: i32) -> Option<&Run> {
            runs.get(
                runs.partition_point(|run| run.first_hour <= hour)
                    .wrapping_sub(1),
            )
            .filter(|run| run.last_hour >= hour)
        }
        let mut result = ChartSummary {
            approximate: Vec::new(),
            from,
            to,
            width,
            buckets: Vec::new(),
            runs: Some(Vec::new()),
        };
        let mut last_known = None;
        for hour in bounds.0..=bounds.1 {
            let members: Vec<_> = tracks
                .iter()
                .filter(|(member, _, _, ranked)| {
                    member.within.is_none_or(|spans| {
                        spans
                            .iter()
                            .any(|span| span.first_hour <= hour && span.last_hour >= hour)
                    }) && ranks.as_ref().is_none_or(|(_, top)| {
                        at(ranked, hour).is_some_and(|run| run.value <= i128::from(*top))
                    })
                })
                .collect();
            let mut known = !members.is_empty()
                && !unobserved
                    .iter()
                    .any(|span| span.first_hour <= hour && span.last_hour >= hour);
            let mut total = 0_i128;
            let mut starts = last_known.and_then(|hour: i32| hour.checked_add(1)) != Some(hour);
            for (member, runs, presence, _) in members {
                if member
                    .excluded
                    .iter()
                    .any(|span| span.first_hour <= hour && span.last_hour >= hour)
                {
                    continue;
                }
                match at(runs, hour) {
                    Some(run) => {
                        total = total
                            .checked_add(run.value)
                            .ok_or_else(|| crate::invalid("Group sum exceeds integer range"))?;
                        starts |= run.break_before && run.first_hour == hour;
                    }
                    None if at(&roster, hour).is_some() && at(presence, hour).is_none() => {}
                    None => known = false,
                }
            }
            known &= total.unsigned_abs() <= u128::from(MAX_EXACT_JS_INTEGER);
            if !known {
                result.buckets.push(None);
                last_known = None;
                continue;
            }
            let value = total as f64;
            let runs = result.runs.as_mut().unwrap();
            match runs.last_mut() {
                Some(last)
                    if !starts
                        && last.value == value
                        && last.last_hour.checked_add(1) == Some(hour) =>
                {
                    last.last_hour = hour
                }
                _ => runs.push(ChartRun {
                    first_hour: hour,
                    last_hour: hour,
                    value,
                    break_before: starts,
                }),
            }
            result.buckets.push(Some(ChartBucket {
                absolute_total: total.unsigned_abs(),
                first_hour: hour,
                last_hour: hour,
                first_value: value,
                last_value: value,
                last_value_hour: hour,
                min_hour: hour,
                min_value: value,
                max_hour: hour,
                max_value: value,
                first_break: starts,
                has_break: false,
            }));
            last_known = Some(hour);
        }
        return Ok(result);
    }
    // Dense geometry adds aligned envelopes. Outside membership, a member contributes zero.
    let mut result: Option<ChartSummary> = None;
    let roster = match coverage {
        Some(coverage) => coverage.clone(),
        None => rosters.summary(0, bounds, view, 0)?,
    };
    let mut expected = vec![false; roster.buckets.len()];
    for member in members {
        check_cancel(cancelled)?;
        let mut allowed = vec![HourSpan {
            first_hour: bounds.0,
            last_hour: bounds.1,
        }];
        for omitted in member.excluded {
            allowed = allowed
                .into_iter()
                .flat_map(|span| {
                    let mut pieces = Vec::new();
                    if omitted.first_hour > span.first_hour {
                        pieces.push(HourSpan {
                            first_hour: span.first_hour,
                            last_hour: span.last_hour.min(omitted.first_hour.saturating_sub(1)),
                        });
                    }
                    if omitted.last_hour < span.last_hour {
                        pieces.push(HourSpan {
                            first_hour: span.first_hour.max(omitted.last_hour.saturating_add(1)),
                            last_hour: span.last_hour,
                        });
                    }
                    pieces
                })
                .collect();
        }
        if let Some(spans) = member.within {
            let mut clipped = Vec::new();
            crate::ranking::for_each_piece(
                &allowed,
                spans,
                |span| (span.first_hour, span.last_hour),
                cancelled,
                |_, first_hour, last_hour| {
                    clipped.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                    Ok(())
                },
            )?;
            allowed = clipped;
        }
        // A member key, its bound hours and its membership presence.
        type Binding = (u64, i32, i32, Option<bool>);
        let bind = |keys: &[Binding]| -> io::Result<Vec<Binding>> {
            let mut spans = Vec::new();
            crate::ranking::for_each_piece(
                keys,
                &allowed,
                |key| (key.1, key.2),
                cancelled,
                |key, first, last| {
                    spans.push((
                        key.0,
                        first,
                        last,
                        (first == key.1).then_some(key.3).flatten(),
                    ));
                    Ok(())
                },
            )?;
            spans.sort_by_key(|span| span.1);
            Ok(spans)
        };
        let mut line = values.summary_spans(&bind(member.keys)?, bounds, view, 0)?;
        if line.buckets.is_empty() {
            line.buckets = vec![None; roster.buckets.len()];
        }
        let membership = if let (Some((reader, _)), Some(actor)) = (&mut ranks, member.rank_key) {
            Some(reader.summary(actor, bounds, view, 0)?)
        } else {
            None
        };
        let mut presence = present.summary_spans(&bind(member.present_keys)?, bounds, view, 0)?;
        if presence.buckets.is_empty() {
            presence.buckets = vec![None; roster.buckets.len()];
        }
        for (index, bucket) in line.buckets.iter_mut().enumerate() {
            let recorded = roster.buckets[index];
            if let (Some(value), Some(present)) = (bucket.as_mut(), presence.buckets[index]) {
                value.has_break |=
                    present.first_hour < value.first_hour || present.last_hour > value.last_hour;
            }
            let in_group = recorded.is_some_and(|recorded| {
                member.within.is_none_or(|spans| {
                    spans.iter().any(|span| {
                        span.first_hour <= recorded.last_hour
                            && span.last_hour >= recorded.first_hour
                    })
                })
            }) && ranks.as_ref().is_none_or(|(_, top)| {
                membership
                    .as_ref()
                    .and_then(|summary| summary.buckets[index])
                    .is_some_and(|rank| rank.min_value <= *top as f64)
            });
            expected[index] |= in_group;
            let omitted = recorded.is_some_and(|recorded| {
                !allowed.iter().any(|span| {
                    span.first_hour <= recorded.last_hour && span.last_hour >= recorded.first_hour
                })
            });
            if !in_group || omitted || bucket.is_none() && presence.buckets[index].is_none() {
                *bucket = recorded.map(|recorded| ChartBucket {
                    absolute_total: 0,
                    first_value: 0.0,
                    last_value: 0.0,
                    last_value_hour: recorded.first_hour,
                    min_value: 0.0,
                    max_value: 0.0,
                    ..recorded
                });
                continue;
            }
            let Some(recorded) = recorded else {
                continue;
            };
            // Shared column endpoints ensure the members' parts add up.
            let span = (recorded.first_hour, recorded.last_hour);
            let (ranked, ranked_hours) = match (ranks.as_mut(), member.rank_key) {
                (Some((reader, top)), Some(actor)) => {
                    let top = *top;
                    let rank = membership
                        .as_ref()
                        .and_then(|summary| summary.buckets[index]);
                    let throughout = rank.is_some_and(|rank| {
                        rank.max_value <= top as f64
                            && !rank.has_break
                            && rank.first_hour <= span.0
                            && rank.last_hour >= span.1
                    });
                    let outside = rank
                        .filter(|rank| rank.max_value > top as f64)
                        .map(|rank| rank.max_hour);
                    if throughout {
                        (Some(((true, true, span.0), outside)), None)
                    } else {
                        let runs = reader.runs(actor, span.0, span.1)?;
                        let within: Vec<(i32, i32)> = runs
                            .iter()
                            .filter(|run| run.value <= i128::from(top))
                            .map(|run| (run.first_hour, run.last_hour))
                            .collect();
                        let ends = ranked_ends(&runs, span, i128::from(top));
                        (Some((ends, outside)), Some(within))
                    }
                }
                _ => (None, None),
            };
            let mut zeros = Vec::new();
            for zero in member.zeros {
                let first_hour = zero.first_hour.max(span.0);
                let last_hour = zero.last_hour.min(span.1);
                if first_hour <= last_hour {
                    zeros.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                }
            }
            crate::products::faction_totals::for_each_absent(
                &[HourSpan {
                    first_hour: span.0,
                    last_hour: span.1,
                }],
                &allowed,
                cancelled,
                |first_hour, last_hour| {
                    zeros.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                    Ok(())
                },
            )?;
            zeros.sort_by_key(|zero| zero.first_hour);
            // Present but unvalued members make the whole column unknown, never zero.
            let unread = match (*bucket, presence.buckets[index]) {
                (_, None) => false,
                (Some(value), Some(held))
                    if !value.has_break
                        && value.first_hour <= held.first_hour
                        && held.last_hour <= value.last_hour =>
                {
                    false
                }
                _ => {
                    let hours = |runs: Vec<Run>| {
                        runs.into_iter()
                            .map(|run| (run.first_hour, run.last_hour))
                            .collect::<Vec<_>>()
                    };
                    let mut known = hours(member_runs(values, member.keys, span)?);
                    known.extend(zeros.iter().map(|zero| (zero.first_hour, zero.last_hour)));
                    known.sort_unstable();
                    let held = hours(member_runs(present, member.present_keys, span)?);
                    let unread = without(&held, &known);
                    match &ranked_hours {
                        Some(within) => unread.iter().any(|&(first, last)| {
                            within.iter().any(|&(from, to)| from <= last && first <= to)
                        }),
                        None => !unread.is_empty(),
                    }
                }
            };
            if unread {
                *bucket = None;
                continue;
            }
            *bucket = column_part(*bucket, presence.buckets[index], recorded, ranked, &zeros);
        }
        match &mut result {
            None => result = Some(line),
            Some(result) => {
                for (sum, member) in result.buckets.iter_mut().zip(line.buckets) {
                    *sum = match (*sum, member) {
                        (Some(a), Some(b)) => Some(ChartBucket {
                            absolute_total: a.absolute_total.saturating_add(b.absolute_total),
                            first_hour: a.first_hour.min(b.first_hour),
                            last_hour: a.last_hour.max(b.last_hour),
                            first_value: a.first_value + b.first_value,
                            last_value: a.last_value + b.last_value,
                            last_value_hour: a.last_value_hour.max(b.last_value_hour),
                            min_value: a.min_value + b.min_value,
                            max_value: a.max_value + b.max_value,
                            first_break: a.first_break || b.first_break,
                            has_break: a.has_break || b.has_break,
                            ..a
                        }),
                        _ => None,
                    };
                }
            }
        }
    }
    let mut result = result.unwrap_or(ChartSummary {
        approximate: Vec::new(),
        from,
        to,
        width,
        buckets: vec![None; roster.buckets.len()],
        runs: None,
    });
    result.runs = None;
    for (index, bucket) in result.buckets.iter_mut().enumerate() {
        if !expected[index] {
            *bucket = None;
            continue;
        }
        if bucket.is_some_and(|bucket| {
            unobserved.iter().any(|span| {
                span.first_hour <= bucket.last_hour && span.last_hour >= bucket.first_hour
            })
        }) {
            *bucket = None;
        }
    }
    Ok(result)
}

fn member_runs(
    reader: &mut dyn SeriesRead,
    keys: &[(u64, i32, i32, Option<bool>)],
    bounds: (i32, i32),
) -> io::Result<Vec<Run>> {
    let mut rows = Vec::new();
    for &(key, first, last, boundary) in keys {
        let from = first.max(bounds.0);
        let to = last.min(bounds.1);
        if from > to {
            continue;
        }
        for mut row in reader.runs(key, from, to)? {
            if row.first_hour == first
                && let Some(boundary) = boundary
            {
                row.break_before = boundary;
            }
            rows.push(row);
        }
    }
    rows.sort_by_key(|run| run.first_hour);
    Ok(rows)
}

/// Both inputs must be in hour order; `minus` may overlap.
fn without(spans: &[(i32, i32)], minus: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for &(first, last) in spans {
        let mut next = i64::from(first);
        for &(from, to) in minus {
            if i64::from(to) < next {
                continue;
            }
            if i64::from(from) > i64::from(last) {
                break;
            }
            if i64::from(from) > next {
                out.push((next as i32, from - 1));
            }
            next = next.max(i64::from(to) + 1);
            if next > i64::from(last) {
                break;
            }
        }
        if next <= i64::from(last) {
            out.push((next as i32, last));
        }
    }
    out
}

/// Present but unvalued members make the column unknown, not zero.
fn column_part(
    value: Option<crate::prepared_chart::ChartBucket>,
    present: Option<crate::prepared_chart::ChartBucket>,
    recorded: crate::prepared_chart::ChartBucket,
    ranked: Option<((bool, bool, i32), Option<i32>)>,
    zeros: &[HourSpan],
) -> Option<crate::prepared_chart::ChartBucket> {
    let (first, last) = (recorded.first_hour, recorded.last_hour);
    let (ranked_first, ranked_last, ranked_since) =
        ranked.map_or((true, true, first), |(ends, _)| ends);
    let zero_at = |hour: i32| {
        zeros
            .iter()
            .any(|zero| zero.first_hour <= hour && hour <= zero.last_hour)
    };
    // Some(None) is proven zero; None is unknown.
    let part = |hour: i32, ranked: bool, valued: Option<f64>| -> Option<Option<f64>> {
        if !ranked {
            return Some(None);
        }
        if let Some(valued) = valued {
            return Some(Some(valued));
        }
        let present =
            present.is_some_and(|present| present.first_hour <= hour && hour <= present.last_hour);
        (zero_at(hour) || !present).then_some(None)
    };
    let starts = value.filter(|value| value.first_hour <= first);
    let ends = value.filter(|value| value.last_hour >= last);
    let start = part(first, ranked_first, starts.map(|value| value.first_value))?;
    let end = part(last, ranked_last, ends.map(|value| value.last_value))?;
    let since = match end {
        Some(_) => value
            .map_or(first, |value| value.last_value_hour)
            .max(ranked_since),
        None => match value {
            Some(value) => {
                let counted = if ranked_last {
                    value.last_hour
                } else {
                    value.last_hour.min(ranked_since.saturating_sub(1))
                };
                if counted == value.last_hour && value.last_value == 0.0 {
                    value.last_value_hour
                } else {
                    counted.saturating_add(1)
                }
            }
            None => first,
        },
    }
    .clamp(first, last);
    let (mut min_hour, mut min_value, mut max_hour, mut max_value) =
        value.map_or((first, 0.0, first, 0.0), |value| {
            (
                value.min_hour,
                value.min_value,
                value.max_hour,
                value.max_value,
            )
        });
    let whole = starts.is_some()
        && ends.is_some()
        && ranked_first
        && ranked_last
        && ranked_since == first
        && zeros.is_empty();
    if !whole {
        let zero = if start.is_none() {
            Some(first)
        } else if end.is_none() {
            Some(since)
        } else {
            ranked
                .and_then(|(_, outside)| outside)
                .or_else(|| zeros.first().map(|zero| zero.first_hour))
        };
        if let Some(zero) = zero.map(|zero| zero.clamp(first, last)) {
            if min_value > 0.0 {
                min_value = 0.0;
                min_hour = zero;
            }
            if max_value < 0.0 {
                max_value = 0.0;
                max_hour = zero;
            }
        }
    }
    Some(crate::prepared_chart::ChartBucket {
        absolute_total: value.map_or(0, |value| value.absolute_total),
        first_hour: first,
        last_hour: last,
        first_value: start.unwrap_or(0.0),
        last_value: end.unwrap_or(0.0),
        last_value_hour: since,
        min_hour: min_hour.clamp(first, last),
        min_value,
        max_hour: max_hour.clamp(first, last),
        max_value,
        first_break: starts.map_or(recorded.first_break, |value| value.first_break),
        has_break: recorded.has_break || value.is_some_and(|value| value.has_break),
    })
}

/// An hour without a rank is outside the Top N, matching the exact sum.
fn ranked_ends(runs: &[Run], (first, last): (i32, i32), top: i128) -> (bool, bool, i32) {
    let mut stretches: Vec<(i32, bool)> = Vec::new();
    let mut next = i64::from(first);
    for run in runs {
        if i64::from(run.first_hour) > next {
            stretches.push((next as i32, false));
        }
        stretches.push((run.first_hour, run.value <= top));
        next = i64::from(run.last_hour) + 1;
    }
    if next <= i64::from(last) {
        stretches.push((next as i32, false));
    }
    let start = stretches.first().is_some_and(|stretch| stretch.1);
    let end = stretches.last().is_some_and(|stretch| stretch.1);
    let mut since = first;
    for &(from, status) in stretches.iter().rev() {
        if status != end {
            break;
        }
        since = from;
    }
    (start, end, since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared::series::{Accumulator, Reader, tests::Fixture};

    #[test]
    fn group_sum_reads_distinct_member_and_presence_keys_with_signed_cancellation() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut values = Accumulator::default();
        let mut present = Accumulator::default();
        let mut rosters = Accumulator::default();
        for (hour, a, b) in [
            (10, -7, Some(7)),
            (11, 3, Some(-3)),
            (12, -2, None),
            (13, -2, None),
        ] {
            values.observe(101, hour, Some(a), false);
            values.observe(102, hour, b, false);
            present.observe(11, hour, Some(1), false);
            present.observe(12, hour, b.map(|_| 1), false);
            rosters.observe(0, hour, Some(1), false);
        }
        let mut writer = fixture.writer();
        values
            .flush(&mut writer, "native-values", 0, &cancelled)
            .unwrap();
        present
            .flush(&mut writer, "native-presence", 0, &cancelled)
            .unwrap();
        rosters
            .flush(&mut writer, "native-rosters", 0, &cancelled)
            .unwrap();
        writer.commit().unwrap();
        let mut values = Reader::new(
            &fixture.store,
            &fixture.key,
            "native-values".into(),
            &cancelled,
        )
        .unwrap();
        let mut present = Reader::new(
            &fixture.store,
            &fixture.key,
            "native-presence".into(),
            &cancelled,
        )
        .unwrap();
        let mut rosters = Reader::new(
            &fixture.store,
            &fixture.key,
            "native-rosters".into(),
            &cancelled,
        )
        .unwrap();
        let members = [
            Member {
                keys: &[(101, i32::MIN, i32::MAX, None)],
                present_keys: &[(11, i32::MIN, i32::MAX, None)],
                rank_key: None,
                within: None,
                excluded: &[],
                zeros: &[],
            },
            Member {
                keys: &[(102, i32::MIN, i32::MAX, None)],
                present_keys: &[(12, i32::MIN, i32::MAX, None)],
                rank_key: None,
                within: None,
                excluded: &[],
                zeros: &[],
            },
        ];
        let exact = summary(
            &mut values,
            &mut present,
            &mut rosters,
            None,
            None,
            &members,
            &[],
            (10.0, 13.0, 4),
            &cancelled,
        )
        .unwrap();
        assert_eq!(
            exact
                .buckets
                .iter()
                .map(|bucket| bucket.unwrap().first_value)
                .collect::<Vec<_>>(),
            [0.0, 0.0, -2.0, -2.0]
        );
        assert_eq!(
            exact.runs.unwrap().len(),
            2,
            "cancelling member changes do not split the sum"
        );
        let dense = summary(
            &mut values,
            &mut present,
            &mut rosters,
            None,
            None,
            &members,
            &[],
            (10.0, 13.0, 2),
            &cancelled,
        )
        .unwrap();
        assert!(dense.runs.is_none());
        let first = dense.buckets[0].unwrap();
        assert_eq!(
            (
                first.first_value,
                first.last_value,
                first.min_value,
                first.max_value
            ),
            (0.0, 0.0, -10.0, 10.0)
        );
        assert_eq!(dense.buckets[1].unwrap().last_value, -2.0);
    }

    fn top_one(series: [(i128, i128); 4], width: u32) -> crate::prepared_chart::ChartSummary {
        group(series.map(|(a, b)| (Some(a), Some(b))), Some(1), width)
    }

    fn group(
        series: [(Option<i128>, Option<i128>); 4],
        top: Option<u64>,
        width: u32,
    ) -> crate::prepared_chart::ChartSummary {
        let rank = |own: Option<i128>, other: Option<i128>| {
            own.map(|own| {
                if other.is_none_or(|other| own > other) {
                    1
                } else {
                    2
                }
            })
        };
        sums(
            series.map(|(a, b)| [(a, a.is_some(), rank(a, b)), (b, b.is_some(), rank(b, a))]),
            top,
            width,
        )
    }

    fn sums(
        hours: [[(Option<i128>, bool, Option<i128>); 2]; 4],
        top: Option<u64>,
        width: u32,
    ) -> crate::prepared_chart::ChartSummary {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut values = Accumulator::default();
        let mut present = Accumulator::default();
        let mut rosters = Accumulator::default();
        let mut ranks = Accumulator::default();
        for (hour, members) in (0..).zip(hours) {
            for (index, (value, held, rank)) in (0_u64..).zip(members) {
                values.observe(101 + index, hour, value, false);
                present.observe(11 + index, hour, held.then_some(1), false);
                ranks.observe(201 + index, hour, rank, false);
            }
            rosters.observe(0, hour, Some(1), false);
        }
        let mut writer = fixture.writer();
        for (accumulator, section) in [
            (&mut values, "native-values"),
            (&mut present, "native-presence"),
            (&mut rosters, "native-rosters"),
            (&mut ranks, "native-ranks"),
        ] {
            accumulator
                .flush(&mut writer, section, 0, &cancelled)
                .unwrap();
        }
        writer.commit().unwrap();
        let reader = |section: &str| {
            Reader::new(&fixture.store, &fixture.key, section.into(), &cancelled).unwrap()
        };
        let (mut values, mut present, mut rosters, mut ranks) = (
            reader("native-values"),
            reader("native-presence"),
            reader("native-rosters"),
            reader("native-ranks"),
        );
        let members = [
            Member {
                keys: &[(101, i32::MIN, i32::MAX, None)],
                present_keys: &[(11, i32::MIN, i32::MAX, None)],
                rank_key: Some(201),
                within: None,
                excluded: &[],
                zeros: &[],
            },
            Member {
                keys: &[(102, i32::MIN, i32::MAX, None)],
                present_keys: &[(12, i32::MIN, i32::MAX, None)],
                rank_key: Some(202),
                within: None,
                excluded: &[],
                zeros: &[],
            },
        ];
        summary(
            &mut values,
            &mut present,
            &mut rosters,
            None,
            top.map(|top| (&mut ranks as &mut dyn SeriesRead, top)),
            &members,
            &[],
            (0.0, 3.0, width),
            &cancelled,
        )
        .unwrap()
    }

    #[test]
    fn dense_top_sums_take_each_endpoint_from_the_members_ranked_there() {
        let series = [(10, 1), (10, 1), (1, 10), (1, 10)];
        assert!(
            top_one(series, 4)
                .buckets
                .iter()
                .all(|bucket| bucket.unwrap().first_value == 10.0)
        );
        let column = top_one(series, 1).buckets[0].unwrap();
        assert_eq!((column.first_value, column.last_value), (10.0, 10.0));
    }

    #[test]
    fn dense_top_sums_date_their_last_value_from_the_crossover() {
        let series = [(10, 1), (10, 1), (10, 20), (10, 20)];
        let exact: Vec<_> = top_one(series, 4)
            .buckets
            .iter()
            .map(|bucket| bucket.unwrap().first_value)
            .collect();
        assert_eq!(exact, [10.0, 10.0, 20.0, 20.0]);
        let column = top_one(series, 1).buckets[0].unwrap();
        assert_eq!(
            (column.first_hour, column.first_value),
            (0, 10.0),
            "{column:?}"
        );
        assert_eq!(
            (column.last_value_hour, column.last_value),
            (2, 20.0),
            "{column:?}"
        );
    }

    fn hourly_and_column(
        series: [(Option<i128>, Option<i128>); 4],
        top: Option<u64>,
    ) -> (Vec<f64>, crate::prepared_chart::ChartBucket) {
        let hourly = group(series, top, 4)
            .buckets
            .iter()
            .map(|bucket| bucket.unwrap().first_value)
            .collect();
        (hourly, group(series, top, 1).buckets[0].unwrap())
    }

    #[test]
    fn dense_sums_take_a_member_appearing_mid_column_at_the_shared_endpoints() {
        let series = [
            (None, Some(10)),
            (None, Some(10)),
            (Some(20), Some(10)),
            (Some(20), Some(10)),
        ];
        for (top, hours, last) in [
            (Some(1), [10.0, 10.0, 20.0, 20.0], 20.0),
            (None, [10.0, 10.0, 30.0, 30.0], 30.0),
        ] {
            let (hourly, column) = hourly_and_column(series, top);
            assert_eq!(hourly, hours, "{top:?}");
            assert_eq!(
                (column.first_hour, column.first_value),
                (0, 10.0),
                "{top:?} {column:?}"
            );
            assert_eq!(
                (column.last_hour, column.last_value_hour, column.last_value),
                (3, 2, last),
                "{top:?} {column:?}"
            );
        }
    }

    #[test]
    fn dense_sums_take_a_member_leaving_mid_column_at_the_shared_endpoints() {
        let series = [
            (Some(10), Some(5)),
            (Some(10), Some(5)),
            (None, Some(5)),
            (None, Some(5)),
        ];
        for (top, hours, first) in [
            (Some(1), [10.0, 10.0, 5.0, 5.0], 10.0),
            (None, [15.0, 15.0, 5.0, 5.0], 15.0),
        ] {
            let (hourly, column) = hourly_and_column(series, top);
            assert_eq!(hourly, hours, "{top:?}");
            assert_eq!(
                (column.first_hour, column.first_value),
                (0, first),
                "{top:?} {column:?}"
            );
            assert_eq!(
                (column.last_hour, column.last_value_hour, column.last_value),
                (3, 2, 5.0),
                "{top:?} {column:?}"
            );
        }
    }

    #[test]
    fn a_member_present_without_a_value_at_an_endpoint_leaves_the_column_unknown() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut values = Accumulator::default();
        let mut present = Accumulator::default();
        let mut rosters = Accumulator::default();
        for hour in 0..4 {
            values.observe(101, hour, Some(10), false);
            values.observe(102, hour, (hour >= 2).then_some(5), false);
            present.observe(11, hour, Some(1), false);
            present.observe(12, hour, Some(1), false);
            rosters.observe(0, hour, Some(1), false);
        }
        let mut writer = fixture.writer();
        for (accumulator, section) in [
            (&mut values, "native-values"),
            (&mut present, "native-presence"),
            (&mut rosters, "native-rosters"),
        ] {
            accumulator
                .flush(&mut writer, section, 0, &cancelled)
                .unwrap();
        }
        writer.commit().unwrap();
        let reader = |section: &str| {
            Reader::new(&fixture.store, &fixture.key, section.into(), &cancelled).unwrap()
        };
        let (mut values, mut present, mut rosters) = (
            reader("native-values"),
            reader("native-presence"),
            reader("native-rosters"),
        );
        let members = [
            Member {
                keys: &[(101, i32::MIN, i32::MAX, None)],
                present_keys: &[(11, i32::MIN, i32::MAX, None)],
                rank_key: None,
                within: None,
                excluded: &[],
                zeros: &[],
            },
            Member {
                keys: &[(102, i32::MIN, i32::MAX, None)],
                present_keys: &[(12, i32::MIN, i32::MAX, None)],
                rank_key: None,
                within: None,
                excluded: &[],
                zeros: &[],
            },
        ];
        let dense = summary(
            &mut values,
            &mut present,
            &mut rosters,
            None,
            None,
            &members,
            &[],
            (0.0, 3.0, 1),
            &cancelled,
        )
        .unwrap();
        assert_eq!(dense.buckets, [None]);
    }

    #[test]
    fn a_member_present_without_values_inside_a_column_leaves_it_unknown() {
        let inside = |hour: i32| (1..=2).contains(&hour);
        for (top, b_rank) in [(None, 1), (Some(1), 1)] {
            let hours = [0, 1, 2, 3].map(|hour| {
                [
                    (Some(10), true, Some(if inside(hour) { 2 } else { 1 })),
                    (None, inside(hour), inside(hour).then_some(b_rank)),
                ]
            });
            let exact: Vec<_> = sums(hours, top, 4)
                .buckets
                .iter()
                .map(|bucket| bucket.map(|bucket| bucket.first_value))
                .collect();
            assert_eq!(exact, [Some(10.0), None, None, Some(10.0)], "{top:?}");
            assert_eq!(sums(hours, top, 1).buckets, [None], "{top:?}");
        }
        let hours = [0, 1, 2, 3].map(|hour| {
            [
                (Some(10), true, Some(1)),
                (None, inside(hour), inside(hour).then_some(2)),
            ]
        });
        let column = sums(hours, Some(1), 1).buckets[0].unwrap();
        assert_eq!((column.first_value, column.last_value), (10.0, 10.0));
    }
}
