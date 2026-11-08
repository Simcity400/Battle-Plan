//! Faction queries join prepared membership and member series inside their window.
use super::*;
use crate::prepared::{
    group_sum, published,
    series::{self, Run},
    series_read::SeriesRead,
};
use crate::prepared_chart::{ChartBucket, ChartRun, ChartSummary, merge};
use crate::products::country_history::RecordedHistories;
use serde_json::{Value, json};
use std::cell::RefCell;

fn clipped(spans: &[HourSpan], window: &HourSpan) -> Vec<HourSpan> {
    let first = spans.partition_point(|span| span.last_hour < window.first_hour);
    let last = spans.partition_point(|span| span.first_hour <= window.last_hour);
    spans[first..last]
        .iter()
        .map(|span| HourSpan {
            first_hour: span.first_hour.max(window.first_hour),
            last_hour: span.last_hour.min(window.last_hour),
        })
        .collect()
}

/// A member's value runs through a sum's read: the first hour of its run
/// holding the read's first hour, and its runs through the read. None for a
/// tag without values.
type MemberValues<'a> =
    dyn FnMut(&str, &HourSpan) -> io::Result<Option<(Option<i32>, Vec<Run>)>> + 'a;

/// One predecessor hour preserves the oracle's segment flags at a clipped left edge.
fn exact(
    values: &mut dyn SeriesRead,
    keys: &BTreeMap<String, u64>,
    history: &History,
    members: &NativeMembers,
    window: &HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<CountryTrack>, Sums)> {
    exact_with(
        &mut |tag, read| {
            let Some(&key) = keys.get(tag) else {
                return Ok(None);
            };
            let origin = values.value_at(key, read.first_hour)?;
            Ok(Some((
                origin.map(|run| run.first_hour),
                values.runs(key, read.first_hour, read.last_hour)?,
            )))
        },
        history,
        members,
        window,
        cancelled,
    )
}

/// [`exact`] with each member's values from `values`.
fn exact_with(
    values: &mut MemberValues<'_>,
    history: &History,
    members: &NativeMembers,
    window: &HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<CountryTrack>, Sums)> {
    let read = HourSpan {
        first_hour: window.first_hour.saturating_sub(1),
        last_hour: window.last_hour,
    };
    let members = members.clone().within(&read, cancelled)?;
    let mut tracks = Vec::new();
    for tag in members.tags() {
        check_cancel(cancelled)?;
        let mut runs = Vec::new();
        if let Some((origin, values)) = values(&tag, &read)? {
            for run in values {
                runs.push(MeasureRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    starts_segment: run.break_before
                        && (run.first_hour != read.first_hour
                            || origin.is_none_or(|origin| origin == run.first_hour)),
                    value: Some(i64::try_from(run.value).map_err(|_| {
                        crate::invalid("Prepared faction member exceeds its native integer range")
                    })?),
                });
            }
        }
        for_each_absent(
            &clipped(&history.country_reads, &read),
            &clipped(
                history
                    .country_presence
                    .get(&tag)
                    .map_or(&[], Vec::as_slice),
                &read,
            ),
            cancelled,
            |first_hour, last_hour| {
                runs.push(MeasureRun {
                    first_hour,
                    last_hour,
                    starts_segment: true,
                    value: Some(0),
                });
                Ok(())
            },
        )?;
        runs.sort_by_key(|run| run.first_hour);
        tracks.push(CountryTrack { tag, runs });
    }
    let proof = History {
        factions: history
            .factions
            .keys()
            .map(|id| (id.clone(), Vec::new()))
            .collect(),
        faction_reads: clipped(&history.faction_reads, &read),
        ..History::default()
    };
    let mut sums = sum_prepared(
        &proof,
        &members.measures(&tracks, cancelled)?,
        &members.rosters,
        Some(window),
        cancelled,
    )?;
    for runs in sums.series.values_mut() {
        runs.retain(|run| run.last_hour >= window.first_hour && run.first_hour <= window.last_hour);
        for run in runs {
            run.starts_segment &= run.first_hour >= window.first_hour;
            run.first_hour = run.first_hour.max(window.first_hour);
            run.last_hour = run.last_hour.min(window.last_hour);
        }
    }
    sums.recorded = clipped(&sums.recorded, window);
    sums.disjoint = clipped(&sums.disjoint, window);
    for track in &mut tracks {
        track.runs.retain(|run| run.last_hour >= window.first_hour);
        for run in &mut track.runs {
            run.starts_segment &= run.first_hour >= window.first_hour;
            run.first_hour = run.first_hour.max(window.first_hour);
        }
    }
    Ok((tracks, sums))
}

/// Exact sums and member tracks over the whole branch, kept per publication: a window's are
/// their slice, as [`exact`] reads them.
struct Branch {
    tracks: Vec<CountryTrack>,
    sums: Sums,
}

/// The branch's exact sums under the request's measure and membership, built once per
/// publication of everything they read: the groups, the identity histories, the measure's key
/// catalogue and its values. Content binds the store key.
#[allow(clippy::too_many_arguments)]
fn branch_sums(
    source: &Source,
    key: &store::SourceKey,
    branch: u64,
    params: &FactionTotalsParams,
    groups: [u64; 3],
    values: &mut dyn SeriesRead,
    measured: [u8; 32],
    keys: &BTreeMap<String, u64>,
    history: &History,
    members: &NativeMembers,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Branch>> {
    let publication = (groups, measured);
    // Each measure and membership keeps its own latest sums: a dashboard's faction widgets of
    // different measures do not displace each other's.
    let lineage = (&params.source, params.membership);
    crate::prepared::query::observations::scope(source, "faction-totals:branch", Some(key))
        .continue_entry(
            branch,
            &lineage,
            &publication,
            &lineage,
            cancelled,
            |_| {
                let window = HourSpan {
                    first_hour: i32::MIN + 1,
                    last_hour: history.last_hour,
                };
                let (tracks, sums) = exact(values, keys, history, members, &window, cancelled)?;
                Ok(Branch { tracks, sums })
            },
        )
}

/// A window's exact sums and, for member parts, its members' tracks, sliced from the branch's
/// kept sums: what [`exact`] over the window gives.
#[allow(clippy::too_many_arguments)]
fn sliced(
    source: &Source,
    key: &store::SourceKey,
    branch: u64,
    params: &FactionTotalsParams,
    groups: [u64; 3],
    values: &mut dyn SeriesRead,
    measured: [u8; 32],
    keys: &BTreeMap<String, u64>,
    history: &History,
    members: &NativeMembers,
    window: &HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<CountryTrack>, Sums)> {
    let whole = branch_sums(
        source, key, branch, params, groups, values, measured, keys, history, members, cancelled,
    )?;
    let tracks = if params.parts == Some(true) {
        slice_tracks(&whole, members, window, cancelled)?
    } else {
        Vec::new()
    };
    Ok((tracks, slice_sums(&whole, window)))
}

fn clipped_runs(runs: &[FactionTotalRun], window: &HourSpan) -> Vec<FactionTotalRun> {
    let start = runs.partition_point(|run| run.last_hour < window.first_hour);
    runs[start..]
        .iter()
        .take_while(|run| run.first_hour <= window.last_hour)
        .map(|run| FactionTotalRun {
            first_hour: run.first_hour.max(window.first_hour),
            last_hour: run.last_hour.min(window.last_hour),
            starts_segment: run.starts_segment && run.first_hour >= window.first_hour,
            value: run.value,
        })
        .collect()
}

/// A window's exact sums, sliced from the branch's.
fn slice_sums(branch: &Branch, window: &HourSpan) -> Sums {
    Sums {
        series: branch
            .sums
            .series
            .iter()
            .map(|(id, runs)| (id.clone(), clipped_runs(runs, window)))
            .collect(),
        recorded: clipped(&branch.sums.recorded, window),
        disjoint: clipped(&branch.sums.disjoint, window),
        overflowed: branch
            .sums
            .overflow
            .iter()
            .map(|span| {
                let (first, last) = (
                    span.first_hour.max(window.first_hour),
                    span.last_hour.min(window.last_hour),
                );
                (i64::from(last) - i64::from(first) + 1).max(0) as usize
            })
            .sum(),
        ..Sums::default()
    }
}

/// A window's member tracks, sliced from the branch's: the members it reads, each run clipped.
fn slice_tracks(
    branch: &Branch,
    members: &NativeMembers,
    window: &HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<Vec<CountryTrack>> {
    let read = HourSpan {
        first_hour: window.first_hour.saturating_sub(1),
        last_hour: window.last_hour,
    };
    let tags: BTreeSet<String> = members
        .clone()
        .within(&read, cancelled)?
        .tags()
        .into_iter()
        .collect();
    Ok(branch
        .tracks
        .iter()
        .filter(|track| tags.contains(&track.tag))
        .map(|track| {
            let start = track
                .runs
                .partition_point(|run| run.last_hour < window.first_hour);
            CountryTrack {
                tag: track.tag.clone(),
                runs: track.runs[start..]
                    .iter()
                    .take_while(|run| run.first_hour <= window.last_hour)
                    .map(|run| MeasureRun {
                        first_hour: run.first_hour.max(window.first_hour),
                        last_hour: run.last_hour.min(window.last_hour),
                        starts_segment: run.starts_segment && run.first_hour >= window.first_hour,
                        value: run.value,
                    })
                    .collect(),
            }
        })
        .collect())
}

/// A faction's exact total over the view, its bins laid from the view's hours: each bin holds
/// the runs its hours hold, and breaks where they start one or leave hours unknown.
fn folded(runs: &[FactionTotalRun], view: (f64, f64, u32)) -> io::Result<ChartSummary> {
    let bounds = (view.0.floor() as i32, view.1.floor() as i32);
    let (exact, bins) = series::summary_bins(bounds, view)?;
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins.len()];
    let window = match (bins.first(), bins.last()) {
        (Some(first), Some(last)) => HourSpan {
            first_hour: first.0,
            last_hour: last.1,
        },
        _ => HourSpan {
            first_hour: 1,
            last_hour: 0,
        },
    };
    let start = runs.partition_point(|run| run.last_hour < window.first_hour);
    let shown = runs[start..]
        .iter()
        .take_while(|run| run.first_hour <= window.last_hour);
    for run in shown.clone() {
        let value = run.value as f64;
        let bin = bins.partition_point(|(_, end)| *end < run.first_hour);
        for (index, &(first, last)) in bins.iter().enumerate().skip(bin) {
            if first > run.last_hour {
                break;
            }
            let (first, last) = (first.max(run.first_hour), last.min(run.last_hour));
            let piece = ChartBucket {
                absolute_total: u128::from(run.value.unsigned_abs())
                    * (i64::from(last) - i64::from(first) + 1) as u128,
                first_hour: first,
                last_hour: last,
                first_value: value,
                last_value: value,
                last_value_hour: first,
                min_hour: first,
                min_value: value,
                max_hour: first,
                max_value: value,
                first_break: run.starts_segment && first == run.first_hour,
                has_break: false,
            };
            // Merging into no bucket would mark a gap that is not there.
            buckets[index] = if buckets[index].is_some() {
                merge(buckets[index], Some(piece))
            } else {
                Some(piece)
            };
        }
    }
    for (bucket, &(first, last)) in buckets.iter_mut().zip(&bins) {
        if let Some(bucket) = bucket {
            bucket.has_break |= bucket.first_hour > first || bucket.last_hour < last;
        }
    }
    Ok(ChartSummary {
        approximate: Vec::new(),
        from: view.0,
        to: view.1,
        width: view.2,
        buckets,
        runs: exact.then(|| {
            shown
                .map(|run| ChartRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value: run.value as f64,
                    break_before: run.starts_segment,
                })
                .collect()
        }),
    })
}

/// The A-state supplies dated coverage; its columns use the shared reader grid.
fn coverage(spans: &[HourSpan], view: (f64, f64, u32)) -> io::Result<ChartSummary> {
    series::summary_budget(view.0, view.1, view.2, 1)?;
    let bounds = (view.0.floor() as i32, view.1.floor() as i32);
    let (exact, bins) = series::summary_bins(bounds, view)?;
    let mut buckets = Vec::with_capacity(bins.len());
    for (first, last) in bins {
        let mut bucket = None;
        let start = spans.partition_point(|span| span.last_hour < first);
        for span in spans[start..]
            .iter()
            .take_while(|span| span.first_hour <= last)
        {
            let first_hour = first.max(span.first_hour);
            let last_hour = last.min(span.last_hour);
            let part = ChartBucket {
                absolute_total: (i64::from(last_hour) - i64::from(first_hour) + 1) as u128,
                first_hour,
                last_hour,
                first_value: 1.0,
                last_value: 1.0,
                last_value_hour: first_hour,
                min_hour: first_hour,
                min_value: 1.0,
                max_hour: first_hour,
                max_value: 1.0,
                first_break: first_hour == span.first_hour,
                has_break: false,
            };
            bucket = if bucket.is_some() {
                merge(bucket, Some(part))
            } else {
                Some(part)
            };
        }
        if let Some(bucket) = &mut bucket {
            bucket.has_break |= bucket.first_hour > first || bucket.last_hour < last;
        }
        buckets.push(bucket);
    }
    Ok(ChartSummary {
        approximate: Vec::new(),
        from: view.0,
        to: view.1,
        width: view.2,
        buckets,
        runs: exact.then(|| {
            spans
                .iter()
                .map(|span| ChartRun {
                    first_hour: span.first_hour.max(bounds.0),
                    last_hour: span.last_hour.min(bounds.1),
                    value: 1.0,
                    break_before: true,
                })
                .filter(|run| run.first_hour <= run.last_hour)
                .collect()
        }),
    })
}

/// Native alias bindings and unknown identity intervals are shared by totals and parts.
#[allow(clippy::type_complexity)]
fn bindings(
    members: &NativeMembers,
    id: &str,
    cancelled: &AtomicBool,
) -> io::Result<(BTreeMap<usize, Vec<HourSpan>>, Vec<HourSpan>, Vec<HourSpan>)> {
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    let mut natives: BTreeMap<usize, Vec<HourSpan>> = BTreeMap::new();
    for (first, last, roster) in &members.rosters[id] {
        match roster {
            Some(roster) => {
                country_groups::mark_unobserved(&mut known, *first, *last);
                for &index in roster.iter() {
                    country_groups::mark_unobserved(
                        natives.entry(index).or_default(),
                        *first,
                        *last,
                    );
                }
            }
            None => unknown.push(HourSpan {
                first_hour: *first,
                last_hour: *last,
            }),
        }
    }
    for (&index, active) in &natives {
        check_cancel(cancelled)?;
        let mut observed = members.identity_reads.clone();
        observed.extend_from_slice(&members.countries[index].presence);
        observed.sort_by_key(|span| span.first_hour);
        let mut union = Vec::new();
        for span in observed {
            country_groups::mark_unobserved(&mut union, span.first_hour, span.last_hour);
        }
        for_each_absent(active, &union, cancelled, |first_hour, last_hour| {
            unknown.push(HourSpan {
                first_hour,
                last_hour,
            });
            Ok(())
        })?;
    }
    unknown.sort_by_key(|span| span.first_hour);
    let mut merged = Vec::new();
    for span in unknown {
        country_groups::mark_unobserved(&mut merged, span.first_hour, span.last_hour);
    }
    Ok((natives, known, merged))
}

#[allow(clippy::too_many_arguments)]
fn dense(
    values: &mut dyn SeriesRead,
    keys: &BTreeMap<String, u64>,
    present: &mut dyn SeriesRead,
    present_keys: &BTreeMap<String, u64>,
    rosters: &mut dyn SeriesRead,
    native: &NativeMembers,
    bindings: &BTreeMap<usize, Vec<HourSpan>>,
    known: &[HourSpan],
    unknown: &[HourSpan],
    view: (f64, f64, u32),
    cancelled: &AtomicBool,
) -> io::Result<ChartSummary> {
    let coverage = coverage(known, view)?;
    let mut dated = Vec::new();
    for (&index, within) in bindings {
        let aliases = &native.countries[index].aliases;
        let mut value_keys = Vec::new();
        let mut presence_keys = Vec::new();
        for (index, alias) in aliases.iter().enumerate() {
            let renamed = index
                .checked_sub(1)
                .and_then(|index| aliases.get(index))
                .is_some_and(|previous| {
                    previous.tag != alias.tag
                        && previous.last_hour.checked_add(1) == Some(alias.first_hour)
                });
            if let Some(&key) = keys.get(&alias.tag) {
                value_keys.push((key, alias.first_hour, alias.last_hour, Some(!renamed)));
            }
            if let Some(&key) = present_keys.get(&alias.tag) {
                presence_keys.push((key, alias.first_hour, alias.last_hour, Some(!renamed)));
            }
        }
        let mut zeros = Vec::new();
        for_each_absent(
            &native.identity_reads,
            &native.countries[index].presence,
            cancelled,
            |first_hour, last_hour| {
                zeros.push(HourSpan {
                    first_hour,
                    last_hour,
                });
                Ok(())
            },
        )?;
        dated.push((value_keys, presence_keys, within, zeros));
    }
    let mut members: Vec<_> = dated
        .iter()
        .map(|(keys, present_keys, within, zeros)| group_sum::Member {
            keys,
            present_keys,
            rank_key: None,
            within: Some(within.as_slice()),
            excluded: &[],
            zeros,
        })
        .collect();
    // A known empty native roster is zero, including hours with no country read.
    members.push(group_sum::Member {
        keys: &[],
        present_keys: &[],
        rank_key: None,
        within: Some(known),
        excluded: &[],
        zeros: &[],
    });
    group_sum::summary(
        values,
        present,
        rosters,
        Some(&coverage),
        None,
        &members,
        unknown,
        view,
        cancelled,
    )
}

fn signature(sums: &Sums, params: &FactionTotalsParams, hour: i32) -> BTreeMap<String, i64> {
    let mut values: Vec<_> = sums
        .series
        .iter()
        .filter_map(|(id, runs)| {
            runs.iter()
                .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                .map(|run| (id, run.value))
        })
        .filter(|(id, _)| {
            params
                .faction_id
                .as_ref()
                .is_none_or(|selected| selected == *id)
        })
        .collect();
    if params.inventory != Some(true) && params.faction_id.is_none() {
        values.sort_by(|(a, av), (b, bv)| bv.cmp(av).then_with(|| a.cmp(b)));
        values.truncate(params.top.min(values.len() as u64) as usize);
    }
    values
        .into_iter()
        .map(|(id, value)| (id.clone(), value))
        .collect()
}

fn navigate(
    values: &mut dyn SeriesRead,
    keys: &BTreeMap<String, u64>,
    history: &History,
    members: &NativeMembers,
    params: &FactionTotalsParams,
    navigation: &series::Navigation,
    cancelled: &AtomicBool,
) -> io::Result<Option<f64>> {
    let mut origin = navigation.origin()?;
    let forward = matches!(
        navigation.key,
        series::MeasureKey::Home | series::MeasureKey::ArrowRight
    );
    let mut tags: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
    let mut boundaries = BTreeSet::new();
    for (id, rosters) in &members.rosters {
        if params
            .faction_id
            .as_ref()
            .is_some_and(|selected| selected != id)
        {
            continue;
        }
        for (first, last, _) in rosters {
            boundaries.insert(*first);
            if let Some(end) = last.checked_add(1) {
                boundaries.insert(end);
            }
        }
        for row in &members.counted[id] {
            tags.entry(row.tag.clone()).or_default().push(HourSpan {
                first_hour: row.first_hour,
                last_hour: row.last_hour,
            });
            boundaries.insert(row.first_hour);
            if let Some(end) = row.last_hour.checked_add(1) {
                boundaries.insert(end);
            }
        }
    }
    for spans in tags.values_mut() {
        spans.sort_by_key(|span| span.first_hour);
        let mut merged = Vec::new();
        for span in std::mem::take(spans) {
            country_groups::mark_unobserved(&mut merged, span.first_hour, span.last_hour);
        }
        *spans = merged;
    }
    for span in members.identity_reads.iter().chain(&history.faction_reads) {
        boundaries.insert(span.first_hour);
        if let Some(end) = span.last_hour.checked_add(1) {
            boundaries.insert(end);
        }
    }
    boundaries.insert(navigation.first_hour.ceil() as i32);
    let dated: Vec<_> = tags
        .iter()
        .filter_map(|(tag, spans)| keys.get(tag).map(|&key| (key, Some(spans.as_slice()))))
        .collect();
    loop {
        check_cancel(cancelled)?;
        let query = series::Navigation {
            from: Some(origin),
            first_hour: navigation.first_hour,
            last_hour: navigation.last_hour,
            key: if forward {
                series::MeasureKey::ArrowRight
            } else {
                series::MeasureKey::ArrowLeft
            },
        };
        let numeric = values.navigate(&dated, &query)?.filter(|hour| {
            if forward {
                *hour > origin
            } else {
                *hour < origin
            }
        });
        let boundary = if forward {
            boundaries
                .iter()
                .find(|&&hour| {
                    f64::from(hour) > origin
                        && f64::from(hour) >= navigation.first_hour
                        && f64::from(hour) <= navigation.last_hour
                })
                .copied()
        } else {
            boundaries
                .iter()
                .rev()
                .find(|&&hour| {
                    f64::from(hour) < origin
                        && f64::from(hour) >= navigation.first_hour
                        && f64::from(hour) <= navigation.last_hour
                })
                .copied()
        }
        .map(f64::from);
        let candidate = match (numeric, boundary) {
            (Some(a), Some(b)) => Some(if forward { a.min(b) } else { a.max(b) }),
            (a, b) => a.or(b),
        };
        let Some(stop) = candidate else {
            return Ok(navigation
                .from
                .map(|hour| hour.clamp(navigation.first_hour, navigation.last_hour)));
        };
        let hour = stop.floor() as i32;
        let before = hour
            .checked_sub(1)
            .map(|hour| {
                exact(
                    values,
                    keys,
                    history,
                    members,
                    &HourSpan {
                        first_hour: hour,
                        last_hour: hour,
                    },
                    cancelled,
                )
            })
            .transpose()?;
        let (_, after) = exact(
            values,
            keys,
            history,
            members,
            &HourSpan {
                first_hour: hour,
                last_hour: hour,
            },
            cancelled,
        )?;
        let before = before.as_ref().map_or_else(BTreeMap::new, |(_, sums)| {
            signature(sums, params, hour.saturating_sub(1))
        });
        let after = signature(&after, params, hour);
        if before != after || stop == navigation.first_hour && !after.is_empty() {
            return Ok(Some(stop));
        }
        origin = stop;
    }
}

pub fn query(
    source: &Source,
    context: &LoadContext,
    product: &str,
    value: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: FactionTotalsParams = serde_json::from_value(value["family"]["selection"].clone())?;
    FactionTotals::validate(&params).map_err(crate::invalid)?;
    if value["branchId"].as_str() != Some(params.branch_id.as_str()) {
        return Err(crate::invalid(
            "Faction selection belongs to another branch",
        ));
    }
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid faction branch"))?;
    if product == "series-state"
        && let Some(hour) = value["reads"][0]["hour"].as_i64()
        && !crate::prepared::coverage::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            gate(&value),
            cancelled,
        )?
        .is_some_and(|coverage| coverage.complete || i64::from(coverage.last_hour) >= hour)
    {
        return Ok(Value::Null);
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    if !crate::prepared::coverage::accepts_pages(
        &store,
        &key,
        &format!("country-groups-{branch}"),
        branch,
    )? {
        return Ok(Value::Null);
    }
    let Some(file) = store.read(&key, &format!("country-groups-{branch}"), 0)? else {
        return Ok(Value::Null);
    };
    // The history spans the branch, not the hours asked: it is kept while its
    // groups page is unchanged.
    let page = file.id().cache_key();
    let history = Arc::clone(
        &*crate::prepared::query::observations::scope(source, "faction-totals:history", Some(&key))
            .get_entry(branch, &(), &page, cancelled, || {
                country_groups::Scan::read(io::BufReader::new(file))?.faction_history(cancelled)
            })?,
    );
    let Some(identities) = super::super::country_history::prepared_branch_histories(
        source,
        branch,
        context.content.as_ref(),
        cancelled,
    )?
    else {
        return Ok(Value::Null);
    };
    let members = NativeMembers::new(&history, &identities, params.membership, cancelled)?;
    // What a reply reads besides its measure changes where memberships, names or identities do.
    let named = published::held_hour(product, &value).and_then(|hour| {
        [history.held_through(hour), identities.held_through(hour)]
            .into_iter()
            .flatten()
            .min()
    });
    let asked = Asked {
        source,
        context,
        product,
        value: &value,
        params: &params,
        branch,
        groups: page,
        key: &key,
        history: &history,
        identities: &identities,
        members: &members,
        cancelled,
    };
    match &params.source {
        FactionMeasure::Territory { measure, rule } => {
            let Some(opened) = published::open(
                source,
                context.content.as_ref(),
                "territory",
                territory::emit::SCHEMA,
                branch,
            )?
            else {
                return Ok(Value::Null);
            };
            let measured = publication(&store, &key, branch, opened.stamp())?;
            opened.read(cancelled, |snapshot, directory| {
                let metadata = territory::query::metadata(directory, branch)?;
                let measures = CountryMeasures {
                    tracks: Vec::new(),
                    unit: match measure {
                        TerritoryMeasure::States => Some("states".into()),
                        TerritoryMeasure::Provinces => Some("provinces".into()),
                        TerritoryMeasure::VictoryPoints => Some("victory points".into()),
                        TerritoryMeasure::Area => metadata.area_unit.clone(),
                    },
                    unavailable_reason: matches!(measure, TerritoryMeasure::Area)
                        .then(|| metadata.area_reason.clone())
                        .flatten(),
                    warnings: metadata.warnings.clone(),
                };
                let mut data = {
                    let cell = RefCell::new(&mut *snapshot);
                    let mut readers =
                        territory::query::measures(&cell, directory, Some((*measure, *rule)));
                    let keys = readers.keys.clone();
                    let presence = Presence {
                        present: &mut readers.present,
                        rosters: &mut readers.rosters,
                        keys: &keys,
                    };
                    answer(
                        &asked,
                        &mut readers.values,
                        &keys,
                        measured,
                        measures,
                        Some(presence),
                    )?
                };
                let last = snapshot.publication().accepted.last.min(history.last_hour);
                published::hold(&mut data, snapshot, directory, product, &value, last, named)?;
                Ok(data)
            })
        }
        FactionMeasure::Factories { mode, groups } => {
            use country_factories::{FactoryTop, emit::SCHEMA, query};
            let limit = crate::prepared::coverage::product_coverage(
                source,
                context.content.as_ref(),
                branch,
                FACTORIES_GATE,
                cancelled,
            )?
            .filter(|coverage| !coverage.complete)
            .map(|coverage| coverage.last_hour);
            // Pinned through the coverage the factories share with the families read with
            // them: a request reaching past it waits.
            let Some(opened) = published::open_through(
                source,
                context.content.as_ref(),
                "factories",
                SCHEMA,
                branch,
                limit,
            )?
            else {
                return Ok(Value::Null);
            };
            let measured = publication(&store, &key, branch, opened.stamp())?;
            // A dense factory summary joins territory's member presence: it waits for
            // territory's coverage through the window, as a request waiting for the
            // branch's coverage does.
            let joined = if dense_summary(product, &value)? {
                let view_last = value["to"].as_f64().unwrap_or(f64::NAN).ceil();
                let covered = crate::prepared::coverage::product_coverage(
                    source,
                    context.content.as_ref(),
                    branch,
                    <territory::Territory as Product>::KEY,
                    cancelled,
                )?
                .is_some_and(|coverage| {
                    coverage.complete || f64::from(coverage.last_hour) >= view_last
                });
                if !covered {
                    return Ok(Value::Null);
                }
                let Some(opened) = published::open(
                    source,
                    context.content.as_ref(),
                    "territory",
                    territory::emit::SCHEMA,
                    branch,
                )?
                else {
                    return Ok(Value::Null);
                };
                Some(opened)
            } else {
                None
            };
            let top = FactoryTop {
                count: 1,
                mode: *mode,
                groups: groups.clone(),
            };
            opened.read(cancelled, |snapshot, directory| {
                if limit.is_some() {
                    let reached = snapshot.publication().accepted.last;
                    let requested = match product {
                        "series-state" => value["reads"][0]["hour"].as_f64(),
                        "series-summary" => value["to"].as_f64(),
                        "series-next-change" => value["lastHour"].as_f64(),
                        _ => None,
                    }
                    .map(f64::ceil);
                    if requested.is_some_and(|requested| f64::from(reached) < requested) {
                        return Ok(Value::Null);
                    }
                }
                let keys = query::member_keys(directory, &identities, &top);
                let refused = if product == "series-state" {
                    let hour = value["reads"][0]["hour"]
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| crate::invalid("Invalid faction reached hour"))?;
                    query::refused_at(snapshot, directory, hour)?
                        .map_or(0, |refused| refused.max(0) as usize)
                } else {
                    let scanned = query::scanned(directory);
                    scanned.unreadable + scanned.unnamed
                };
                let mut warnings = Vec::new();
                if refused > 0 {
                    warnings.push(format!("{refused} factory reads could not name a country or show its count; affected faction totals are gaps."));
                }
                let measures = CountryMeasures {
                    tracks: Vec::new(),
                    unit: Some("factories".into()),
                    unavailable_reason: None,
                    warnings,
                };
                let Some(joined) = joined else {
                    let mut data = {
                        let cell = RefCell::new(&mut *snapshot);
                        let mut values = query::counts(&cell, directory, &top);
                        answer(&asked, &mut values, &keys, measured, measures, None)?
                    };
                    let last = snapshot.publication().accepted.last.min(history.last_hour);
                    published::hold(&mut data, snapshot, directory, product, &value, last, named)?;
                    return Ok(data);
                };
                let cell = RefCell::new(&mut *snapshot);
                let mut values = query::counts(&cell, directory, &top);
                joined.read(cancelled, |snapshot, directory| {
                    let cell = RefCell::new(&mut *snapshot);
                    let mut readers = territory::query::measures(&cell, directory, None);
                    let present_keys = readers.keys.clone();
                    let presence = Presence {
                        present: &mut readers.present,
                        rosters: &mut readers.rosters,
                        keys: &present_keys,
                    };
                    answer(
                        &asked,
                        &mut values,
                        &keys,
                        measured,
                        measures,
                        Some(presence),
                    )
                })
            })
        }
    }
}

/// Territory's presence of the members a dense summary joins: their presence and the
/// rosters, by the tags' actors.
struct Presence<'p> {
    present: &'p mut dyn SeriesRead,
    rosters: &'p mut dyn SeriesRead,
    keys: &'p BTreeMap<String, u64>,
}

/// What a faction reply reads besides its measure.
struct Asked<'a> {
    source: &'a Source,
    context: &'a LoadContext,
    product: &'a str,
    value: &'a Value,
    params: &'a FactionTotalsParams,
    branch: u64,
    groups: [u64; 3],
    key: &'a store::SourceKey,
    history: &'a History,
    identities: &'a RecordedHistories,
    members: &'a NativeMembers,
    cancelled: &'a AtomicBool,
}

/// Whether a request is a summary drawing more hours than its width.
fn dense_summary(product: &str, value: &Value) -> io::Result<bool> {
    if product != "series-summary" {
        return Ok(false);
    }
    let view = series::viewport(
        value["from"].as_f64(),
        value["to"].as_f64(),
        value["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let (exact, _) = series::summary_bins((view.0.floor() as i32, view.1.floor() as i32), view)?;
    Ok(!exact)
}

/// What identifies a family's publication with the identity history it is read with.
fn publication(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    (range, generation): ([u64; 3], u64),
) -> io::Result<[u8; 32]> {
    let identities = crate::prepared::query::observations::published(
        store,
        key,
        &[format!("country-identities-{branch}").as_str()],
    )?;
    let mut hash = blake3::Hasher::new();
    hash.update(&identities);
    for part in range.into_iter().chain([generation]) {
        hash.update(&part.to_le_bytes());
    }
    Ok(*hash.finalize().as_bytes())
}

/// A faction reply from `values`, its measure's values by tag key, and the members' territory
/// presence where a dense summary joins it.
fn answer(
    asked: &Asked<'_>,
    values: &mut dyn SeriesRead,
    keys: &BTreeMap<String, u64>,
    measured: [u8; 32],
    mut measures: CountryMeasures,
    presence: Option<Presence<'_>>,
) -> io::Result<Value> {
    let &Asked {
        source,
        context,
        product,
        value,
        params,
        branch,
        groups: page,
        key,
        history,
        identities,
        members,
        cancelled,
    } = asked;
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(value.clone())?;
        return Ok(
            json!({"hour":navigate(values, keys, history, members, params, &navigation, cancelled)?}),
        );
    }
    if product == "series-state" {
        let reads = value["reads"]
            .as_array()
            .ok_or_else(|| crate::invalid("Missing faction reached hour"))?;
        let [read] = reads.as_slice() else {
            return Err(crate::invalid("Faction state needs one reached hour"));
        };
        let hour = read["hour"]
            .as_i64()
            .and_then(|hour| i32::try_from(hour).ok())
            .ok_or_else(|| crate::invalid("Invalid faction reached hour"))?;
        let window = HourSpan {
            first_hour: hour,
            last_hour: hour,
        };
        let (tracks, sums) = sliced(
            source, key, branch, params, page, values, measured, keys, history, members, &window,
            cancelled,
        )?;
        measures.tracks = tracks;
        return Ok(serde_json::to_value(build_reply(
            source,
            params,
            context,
            history,
            identities,
            members,
            &measures,
            &sums,
            Some(&window),
            cancelled,
        )?)?);
    }
    if product != "series-summary" {
        return Err(crate::invalid("Unknown faction query"));
    }
    let view = series::viewport(
        value["from"].as_f64(),
        value["to"].as_f64(),
        value["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let window = HourSpan {
        first_hour: view.0.floor() as i32,
        last_hour: view.1.floor() as i32,
    };
    series::summary_budget(view.0, view.1, view.2, 1)?;
    let (is_exact, _) = series::summary_bins((window.first_hour, window.last_hour), view)?;
    if is_exact {
        let (tracks, sums) = sliced(
            source, key, branch, params, page, values, measured, keys, history, members, &window,
            cancelled,
        )?;
        measures.tracks = tracks;
        let reply = build_reply(
            source,
            params,
            context,
            history,
            identities,
            members,
            &measures,
            &sums,
            Some(&window),
            cancelled,
        )?;
        let summaries: BTreeMap<_, _> = reply
            .series
            .iter()
            .map(|series| {
                (
                    series.id.clone(),
                    ChartSummary {
                        approximate: Vec::new(),
                        from: view.0,
                        to: view.1,
                        width: view.2,
                        buckets: Vec::new(),
                        runs: Some(
                            series
                                .runs
                                .iter()
                                .map(|run| ChartRun {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    value: run.value as f64,
                                    break_before: run.starts_segment,
                                })
                                .collect(),
                        ),
                    },
                )
            })
            .collect();
        let mut reply = serde_json::to_value(reply)?;
        reply["summaries"] = serde_json::to_value(summaries)?;
        super::super::check_reply_budget(&reply)?;
        return Ok(reply);
    }
    let Some(presence) = presence else {
        return Ok(Value::Null);
    };
    dense_reply(
        source, context, params, history, identities, members, measures, values, keys, presence,
        key, branch, page, measured, view, cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
fn dense_reply(
    source: &Source,
    context: &LoadContext,
    params: &FactionTotalsParams,
    history: &History,
    identities: &RecordedHistories,
    members: &NativeMembers,
    mut measures: CountryMeasures,
    values: &mut dyn SeriesRead,
    keys: &BTreeMap<String, u64>,
    presence: Presence<'_>,
    key: &store::SourceKey,
    branch: u64,
    groups: [u64; 3],
    measured: [u8; 32],
    view: (f64, f64, u32),
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let Presence {
        present,
        rosters,
        keys: present_keys,
    } = presence;
    let window = HourSpan {
        first_hour: view.0.floor() as i32,
        last_hour: view.1.floor() as i32,
    };
    // Every view, column ranking and endpoint reads the branch's exact sums, built once.
    let whole = branch_sums(
        source, key, branch, params, groups, values, measured, keys, history, members, cancelled,
    )?;
    // Exact sums inside the window also read the hour before it, for its segment flags.
    let nearby = members.clone().within(
        &HourSpan {
            first_hour: window.first_hour.saturating_sub(1),
            last_hour: window.last_hour,
        },
        cancelled,
    )?;
    let bounded = nearby.clone().within(&window, cancelled)?;
    // Only member parts read presence and rosters, each member's on its own.
    let mut readers = (params.parts == Some(true)).then_some((present, rosters));
    let ranks = params.faction_id.is_none() && params.inventory != Some(true);
    let mut summaries = BTreeMap::new();
    let mut member_summaries = BTreeMap::new();
    let mut valued = BTreeMap::new();
    for id in history.factions.keys() {
        check_cancel(cancelled)?;
        if params
            .faction_id
            .as_ref()
            .is_some_and(|selected| selected != id)
        {
            continue;
        }
        let (tags, known, unknown) = bindings(&bounded, id, cancelled)?;
        let summary = folded(
            whole.sums.series.get(id).map_or(&[][..], Vec::as_slice),
            view,
        )?;
        series::summary_budget(view.0, view.1, view.2, summaries.len() + 1)?;
        summaries.insert(id.clone(), summary);
        if let Some((present, rosters)) = &mut readers {
            for (&index, within) in &tags {
                let id = serde_json::to_string(&bounded.countries[index].identity)?;
                series::summary_budget(
                    view.0,
                    view.1,
                    view.2,
                    summaries.len() + member_summaries.len() + 1,
                )?;
                member_summaries.insert(
                    id,
                    dense(
                        values,
                        keys,
                        &mut **present,
                        present_keys,
                        &mut **rosters,
                        &bounded,
                        &BTreeMap::from([(index, within.clone())]),
                        &known,
                        &unknown,
                        view,
                        cancelled,
                    )?,
                );
            }
        }
        if ranks {
            valued.insert(id.clone(), known);
        }
    }

    // D-12 keeps a faction briefly in the Top N inside a column, drawn from its ranked hours.
    let mut ranked = if ranks {
        Some(retain_dense_top(
            &mut summaries,
            &valued,
            params.top,
            view,
            &mut |hours| Ok(slice_sums(&whole, hours).series),
            cancelled,
        )?)
    } else {
        None
    };
    let mut sums = Sums {
        series: history
            .factions
            .keys()
            .map(|id| (id.clone(), Vec::new()))
            .collect(),
        ..Sums::default()
    };
    let endpoints: BTreeSet<_> = summaries
        .values()
        .filter_map(|summary| {
            summary
                .buckets
                .iter()
                .rev()
                .flatten()
                .next()
                .map(|bucket| bucket.last_hour)
        })
        .collect();
    let mut tracks: BTreeMap<String, Vec<MeasureRun>> = BTreeMap::new();
    for hour in endpoints {
        let at = HourSpan {
            first_hour: hour,
            last_hour: hour,
        };
        let point = slice_sums(&whole, &at);
        // Only member parts read the members' tracks.
        let point_tracks = if params.parts == Some(true) {
            slice_tracks(&whole, &nearby, &at, cancelled)?
        } else {
            Vec::new()
        };
        for (id, runs) in point.series {
            sums.series
                .get_mut(&id)
                .expect("faction catalogue")
                .extend(runs);
        }
        for span in point.recorded {
            country_groups::mark_unobserved(&mut sums.recorded, span.first_hour, span.last_hour);
        }
        for span in point.disjoint {
            country_groups::mark_unobserved(&mut sums.disjoint, span.first_hour, span.last_hour);
        }
        sums.overflowed = sums.overflowed.saturating_add(point.overflowed);
        for track in point_tracks {
            tracks.entry(track.tag).or_default().extend(track.runs);
        }
    }
    measures.tracks = tracks
        .into_iter()
        .map(|(tag, runs)| CountryTrack { tag, runs })
        .collect();
    let mut inventory = params.clone();
    inventory.inventory = Some(true);
    let mut reply = build_reply(
        source,
        &inventory,
        context,
        history,
        identities,
        members,
        &measures,
        &sums,
        Some(&window),
        cancelled,
    )?;
    reply
        .series
        .retain(|series| summaries.contains_key(&series.id));
    for series in &mut reply.series {
        let end = summaries[&series.id]
            .buckets
            .iter()
            .rev()
            .flatten()
            .next()
            .map(|bucket| bucket.last_hour);
        series.runs.retain(|run| Some(run.last_hour) == end);
        series.membership = match &mut ranked {
            Some(ranked) => ranked.remove(&series.id).unwrap_or_default(),
            None => {
                let mut membership = Vec::new();
                for bucket in summaries[&series.id].buckets.iter().flatten() {
                    country_groups::mark_unobserved(
                        &mut membership,
                        bucket.first_hour,
                        bucket.last_hour,
                    );
                }
                membership
            }
        };
    }
    let mut reply = serde_json::to_value(reply)?;
    reply["summaries"] = serde_json::to_value(summaries)?;
    if params.parts == Some(true) {
        reply["memberSummaries"] = serde_json::to_value(member_summaries)?;
    }
    super::super::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Exact faction sums through a window's hours, by faction.
type WindowSums<'a> =
    dyn FnMut(&HourSpan) -> io::Result<BTreeMap<String, Vec<FactionTotalRun>>> + 'a;

/// Each column's Top N as the exact sums rank them, and each kept faction's ranked hours.
/// A column the bucket bounds cannot settle is summed exactly and drawn from its ranked hours.
fn retain_dense_top(
    summaries: &mut BTreeMap<String, ChartSummary>,
    valued: &BTreeMap<String, Vec<HourSpan>>,
    top: u64,
    view: (f64, f64, u32),
    sums: &mut WindowSums<'_>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let top = usize::try_from(top).unwrap_or(usize::MAX);
    let (_, bins) = series::summary_bins((view.0.floor() as i32, view.1.floor() as i32), view)?;
    let ids: Vec<String> = summaries.keys().cloned().collect();
    let mut ranked = vec![Vec::new(); ids.len()];
    // Per column, the factions only its exact sums can rank.
    let mut open = vec![Vec::new(); bins.len()];
    let mut cursors = vec![0; ids.len()];
    let order = |a: &(f64, usize), b: &(f64, usize)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
    // How many `bounds` rank above faction `index` at `value`, ties in faction order.
    let above = |bounds: &[(f64, usize)], value: f64, index: usize| {
        bounds.partition_point(|&(bound, other)| bound > value || (bound == value && other < index))
    };
    for (column, &(first, last)) in bins.iter().enumerate() {
        check_cancel(cancelled)?;
        let mut held = Vec::new();
        // Factions without a bucket here that may still hold a total in it.
        let mut unbounded = 0;
        for (index, (id, summary)) in summaries.iter().enumerate() {
            if let Some(bucket) = summary.buckets.get(column).copied().flatten() {
                held.push((index, bucket));
                continue;
            }
            let spans = valued.get(id).map_or(&[][..], Vec::as_slice);
            let cursor = &mut cursors[index];
            *cursor += spans[*cursor..].partition_point(|span| span.last_hour < first);
            let holds = spans
                .get(*cursor)
                .is_some_and(|span| span.first_hour <= last);
            unbounded += usize::from(holds);
        }
        if held.len() + unbounded <= top {
            for (index, bucket) in held {
                ranked[index].push(HourSpan {
                    first_hour: bucket.first_hour,
                    last_hour: bucket.last_hour,
                });
            }
            continue;
        }
        let mut highs: Vec<_> = held
            .iter()
            .map(|(index, bucket)| (bucket.max_value, *index))
            .collect();
        // Only a faction holding a total at every hour of the column bounds others out.
        let mut lows: Vec<_> = held
            .iter()
            .filter(|(_, bucket)| {
                !bucket.has_break && (bucket.first_hour, bucket.last_hour) == (first, last)
            })
            .map(|(index, bucket)| (bucket.min_value, *index))
            .collect();
        highs.sort_unstable_by(order);
        lows.sort_unstable_by(order);
        for (index, bucket) in held {
            // Others that may outrank it at some hour, less itself.
            let may = above(&highs, bucket.min_value, index)
                - usize::from(bucket.max_value > bucket.min_value);
            if unbounded + may < top {
                ranked[index].push(HourSpan {
                    first_hour: bucket.first_hour,
                    last_hour: bucket.last_hour,
                });
            } else if above(&lows, bucket.max_value, index) < top {
                open[column].push(index);
            } else {
                summaries
                    .get_mut(&ids[index])
                    .expect("ranked faction summary")
                    .buckets[column] = None;
            }
        }
    }
    let mut next = 0;
    while next < bins.len() {
        if open[next].is_empty() {
            next += 1;
            continue;
        }
        // Adjacent open columns share one exact read.
        let start = next;
        while open.get(next).is_some_and(|factions| !factions.is_empty()) {
            next += 1;
        }
        let totals = sums(&HourSpan {
            first_hour: bins[start].0,
            last_hour: bins[next - 1].1,
        })?;
        let members = crate::ranking::top_membership(
            &totals,
            top.min(totals.len()),
            |run| (run.first_hour, run.last_hour),
            |run| Some(run.value),
            cancelled,
        )?;
        for column in start..next {
            let (first, last) = bins[column];
            let hours = |first_hour: i32, last_hour: i32| {
                i64::from(last_hour.min(last)) - i64::from(first_hour.max(first)) + 1
            };
            for &index in &open[column] {
                check_cancel(cancelled)?;
                let id = &ids[index];
                let runs = totals.get(id).map_or(&[][..], Vec::as_slice);
                let runs = &runs[runs.partition_point(|run| run.last_hour < first)
                    ..runs.partition_point(|run| run.first_hour <= last)];
                let within = clipped(
                    members.get(id).map_or(&[][..], Vec::as_slice),
                    &HourSpan {
                        first_hour: first,
                        last_hour: last,
                    },
                );
                let mut bucket = None;
                crate::ranking::for_each_piece(
                    runs,
                    &within,
                    |run| (run.first_hour, run.last_hour),
                    cancelled,
                    |run, first_hour, last_hour| {
                        let value = run.value as f64;
                        let part = Some(ChartBucket {
                            absolute_total: u128::from(run.value.unsigned_abs())
                                * hours(first_hour, last_hour) as u128,
                            first_hour,
                            last_hour,
                            first_value: value,
                            last_value: value,
                            last_value_hour: first_hour,
                            min_hour: first_hour,
                            min_value: value,
                            max_hour: first_hour,
                            max_value: value,
                            first_break: run.starts_segment && first_hour == run.first_hour,
                            has_break: false,
                        });
                        bucket = if bucket.is_some() {
                            merge(bucket, part)
                        } else {
                            part
                        };
                        ranked[index].push(HourSpan {
                            first_hour,
                            last_hour,
                        });
                        Ok(())
                    },
                )?;
                let ranked_hours: i64 = within
                    .iter()
                    .map(|span| hours(span.first_hour, span.last_hour))
                    .sum();
                let valued_hours: i64 = runs
                    .iter()
                    .map(|run| hours(run.first_hour, run.last_hour))
                    .sum();
                // Ranked at every hour it holds a total, it keeps its whole bucket.
                if ranked_hours == 0 || ranked_hours < valued_hours {
                    summaries
                        .get_mut(id)
                        .expect("ranked faction summary")
                        .buckets[column] = bucket;
                }
            }
        }
    }
    let mut kept = BTreeMap::new();
    for (id, spans) in ids.into_iter().zip(ranked) {
        if summaries[&id].buckets.iter().any(Option::is_some) {
            kept.insert(id, crate::ranking::joined_spans(spans, i32::MIN, i32::MAX));
        } else {
            summaries.remove(&id);
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests;
