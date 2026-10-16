//! Window-bounded group sums over prepared member values and recorded coverage.
use super::series::{self, Reader, Run};
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
    /// Transient value, presence and rank runs for a single catalogue key.
    pub tail: Option<(&'a [Run], &'a [Run], &'a [Run])>,
}

/// Exact windows sum dated values; dense windows add aligned member envelopes.
#[allow(clippy::too_many_arguments)]
pub fn summary(
    values: &mut Reader<'_>,
    present: &mut Reader<'_>,
    rosters: &mut Reader<'_>,
    coverage: Option<&crate::prepared_chart::ChartSummary>,
    mut ranks: Option<(&mut Reader<'_>, u64)>,
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
            fn read(
                reader: &mut Reader<'_>,
                keys: &[(u64, i32, i32, Option<bool>)],
                bounds: (i32, i32),
                tail: &[Run],
            ) -> io::Result<Vec<Run>> {
                let mut rows = Vec::new();
                for &(key, first, last, boundary) in keys {
                    let from = first.max(bounds.0);
                    let to = last.min(bounds.1).min(
                        tail.first()
                            .map_or(i32::MAX, |run| run.first_hour.saturating_sub(1)),
                    );
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
                rows.extend(tail.iter().filter_map(|run| {
                    let first_hour = run.first_hour.max(bounds.0);
                    let last_hour = run.last_hour.min(bounds.1);
                    (first_hour <= last_hour).then_some(Run {
                        first_hour,
                        last_hour,
                        ..*run
                    })
                }));
                rows.sort_by_key(|run| run.first_hour);
                Ok(rows)
            }
            let (value_tail, presence_tail, rank_tail) = member.tail.unwrap_or((&[], &[], &[]));
            let rows = read(values, member.keys, bounds, value_tail)?;
            let presence = read(present, member.present_keys, bounds, presence_tail)?;
            let ranked = if let Some((reader, _)) = &mut ranks {
                member
                    .rank_key
                    .map(|actor| {
                        read(
                            reader,
                            &[(actor, i32::MIN, i32::MAX, None)],
                            bounds,
                            rank_tail,
                        )
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
        let mut line = if let Some((tail, _, _)) = member.tail {
            values.summary_with_tail(member.keys[0].0, bounds, view, 0, tail, Some(&allowed))?
        } else {
            values.summary_spans(&bind(member.keys)?, bounds, view, 0)?
        };
        if line.buckets.is_empty() {
            line.buckets = vec![None; roster.buckets.len()];
        }
        let membership = if let (Some((reader, _)), Some(actor)) = (&mut ranks, member.rank_key) {
            Some(if let Some((_, _, tail)) = member.tail {
                reader.summary_with_tail(actor, bounds, view, 0, tail, None)?
            } else {
                reader.summary(actor, bounds, view, 0)?
            })
        } else {
            None
        };
        let mut presence = if let Some((_, tail, _)) = member.tail {
            present.summary_with_tail(
                member.present_keys[0].0,
                bounds,
                view,
                0,
                tail,
                Some(&allowed),
            )?
        } else {
            present.summary_spans(&bind(member.present_keys)?, bounds, view, 0)?
        };
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
                    min_value: 0.0,
                    max_value: 0.0,
                    ..recorded
                });
            }
        }
        for (index, bucket) in line.buckets.iter_mut().enumerate() {
            let Some(recorded) = roster.buckets[index] else {
                continue;
            };
            if !expected[index] {
                continue;
            }
            let recorded_span = [HourSpan {
                first_hour: recorded.first_hour,
                last_hour: recorded.last_hour,
            }];
            let mut zeros = Vec::new();
            for span in member.zeros {
                let first_hour = span.first_hour.max(recorded.first_hour);
                let last_hour = span.last_hour.min(recorded.last_hour);
                if first_hour <= last_hour {
                    zeros.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                }
            }
            crate::products::faction_totals::for_each_absent(
                &recorded_span,
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
            zeros.sort_by_key(|span| span.first_hour);
            if let (Some(first), Some(last)) = (zeros.first(), zeros.last()) {
                if let Some(value) = bucket {
                    if first.first_hour < value.first_hour {
                        value.first_hour = first.first_hour;
                        value.first_value = 0.0;
                        value.first_break = recorded.first_break;
                    }
                    if last.last_hour > value.last_hour {
                        value.last_hour = last.last_hour;
                        if value.last_value != 0.0 {
                            value.last_value_hour = last.first_hour;
                        }
                        value.last_value = 0.0;
                    }
                    if value.min_value > 0.0 {
                        value.min_value = 0.0;
                        value.min_hour = first.first_hour;
                    }
                    if value.max_value < 0.0 {
                        value.max_value = 0.0;
                        value.max_hour = first.first_hour;
                    }
                } else {
                    let mut zero = None;
                    let mut merged = Vec::new();
                    for span in zeros {
                        crate::products::country_groups::mark_unobserved(
                            &mut merged,
                            span.first_hour,
                            span.last_hour,
                        );
                    }
                    for span in merged {
                        let next = ChartBucket {
                            absolute_total: 0,
                            first_hour: span.first_hour,
                            last_hour: span.last_hour,
                            first_value: 0.0,
                            last_value: 0.0,
                            last_value_hour: span.first_hour,
                            min_hour: span.first_hour,
                            min_value: 0.0,
                            max_hour: span.first_hour,
                            max_value: 0.0,
                            first_break: recorded.first_break,
                            has_break: recorded.has_break,
                        };
                        zero = if zero.is_some() {
                            crate::prepared_chart::merge(zero, Some(next))
                        } else {
                            Some(next)
                        };
                    }
                    if let Some(value) = &mut zero {
                        value.has_break |= value.first_hour > recorded.first_hour
                            || value.last_hour < recorded.last_hour;
                    }
                    *bucket = zero;
                }
            }
        }
        match &mut result {
            None => result = Some(line),
            Some(result) => {
                for (sum, member) in result.buckets.iter_mut().zip(line.buckets) {
                    *sum = match (*sum, member) {
                        (Some(a), Some(b)) => Some(ChartBucket {
                            absolute_total: a.absolute_total.saturating_add(b.absolute_total),
                            first_value: a.first_value + b.first_value,
                            last_value: a.last_value + b.last_value,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared::series::{Accumulator, tests::Fixture};

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
                tail: None,
            },
            Member {
                keys: &[(102, i32::MIN, i32::MAX, None)],
                present_keys: &[(12, i32::MIN, i32::MAX, None)],
                rank_key: None,
                within: None,
                excluded: &[],
                zeros: &[],
                tail: None,
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
}
