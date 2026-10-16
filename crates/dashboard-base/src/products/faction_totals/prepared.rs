//! Faction queries join prepared membership and member series inside their window.
use super::*;
use crate::prepared::{
    group_sum,
    series::{self, Reader},
};
use crate::prepared_chart::{ChartBucket, ChartRun, ChartSummary, merge};
use crate::products::country_history::RecordedHistories;
use serde_json::{Value, json};

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

/// One predecessor hour preserves the oracle's segment flags at a clipped left edge.
fn exact(
    values: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
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
        if let Some(&key) = keys.get(&tag) {
            let origin = values.value_at(key, read.first_hour)?;
            for run in values.runs(key, read.first_hour, read.last_hour)? {
                runs.push(MeasureRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    starts_segment: run.break_before
                        && (run.first_hour != read.first_hour
                            || origin.is_none_or(|origin| origin.first_hour == run.first_hour)),
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
    values: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    present: &mut Reader<'_>,
    present_keys: &BTreeMap<String, u64>,
    rosters: &mut Reader<'_>,
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
            tail: None,
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
        tail: None,
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
    values: &mut Reader<'_>,
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
        let numeric =
            series::navigate(&mut [(&mut *values, dated.clone())], &query)?.filter(|hour| {
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
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let Some(file) = store.read(&key, &format!("country-groups-{branch}"), 0)? else {
        return Ok(Value::Null);
    };
    let scan = country_groups::Scan::read(io::BufReader::new(file))?;
    let history = scan.faction_history(cancelled)?;
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
    // Territory's catalogue: its measures' keys, presence and metadata for
    // territory totals. Factory totals read only its presence, which joins a
    // dense summary's members; their exact replies never read territory.
    let territory_catalogue =
        || series::read_catalogue::<territory::TerritoryReply>(&store, &key, "territory", branch);
    let (keys, section, mut measures, present_keys) = match &params.source {
        FactionMeasure::Territory { measure, rule } => {
            let Some((present_keys, territory_metadata)) = territory_catalogue()? else {
                return Ok(Value::Null);
            };
            (
                present_keys.clone(),
                format!("territory-values-{branch}-{}", measure.index(*rule)),
                CountryMeasures {
                    tracks: Vec::new(),
                    unit: match measure {
                        TerritoryMeasure::States => Some("states".into()),
                        TerritoryMeasure::Provinces => Some("provinces".into()),
                        TerritoryMeasure::VictoryPoints => Some("victory points".into()),
                        TerritoryMeasure::Area => territory_metadata.area_unit.clone(),
                    },
                    unavailable_reason: matches!(measure, TerritoryMeasure::Area)
                        .then(|| territory_metadata.area_reason.clone())
                        .flatten(),
                    warnings: territory_metadata.warnings.clone(),
                },
                Some(present_keys),
            )
        }
        FactionMeasure::Factories { mode, groups } => {
            let Some((keys, metadata)) = series::read_catalogue::<
                country_factories::ScannedFactories,
            >(&store, &key, "factories", branch)?
            else {
                return Ok(Value::Null);
            };
            let channel = country_factories::prepared::channel(&country_factories::FactoryTop {
                count: 1,
                mode: *mode,
                groups: groups.clone(),
            });
            let keys = identities
                .countries
                .keys()
                .filter_map(|tag| {
                    keys.get(&series::field_key(tag, ""))
                        .map(|&key| (tag.clone(), key))
                })
                .collect();
            let mut warnings = Vec::new();
            if metadata.unreadable > 0 || metadata.unnamed > 0 {
                warnings.push(format!("{} factory reads could not name a country or show its count; affected faction totals are gaps.", metadata.unreadable + metadata.unnamed));
            }
            (
                keys,
                format!("factories-values-{branch}-{channel}"),
                CountryMeasures {
                    tracks: Vec::new(),
                    unit: Some("factories".into()),
                    unavailable_reason: None,
                    warnings,
                },
                None,
            )
        }
    };
    let mut values = Reader::new(&store, &key, section, cancelled)?;
    if matches!(params.source, FactionMeasure::Factories { .. })
        && let Some(coverage) = crate::prepared::coverage::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            FACTORIES_GATE,
            cancelled,
        )?
        .filter(|coverage| !coverage.complete)
    {
        // Factories prepared on their own may have committed pages beyond
        // the coverage of the families read with them: they are read only
        // through the last batch committed by its last hour, and a request
        // reaching past that waits.
        let reached = values.through_hour(coverage.last_hour)?;
        let requested = match product {
            "series-state" => value["reads"][0]["hour"].as_f64(),
            "series-summary" => value["to"].as_f64(),
            "series-next-change" => value["lastHour"].as_f64(),
            _ => None,
        }
        .map(f64::ceil);
        if requested
            .is_some_and(|requested| reached.is_none_or(|reached| f64::from(reached) < requested))
        {
            return Ok(Value::Null);
        }
    }
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(value)?;
        return Ok(
            json!({"hour":navigate(&mut values, &keys, &history, &members, &params, &navigation, cancelled)?}),
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
        let (tracks, sums) = exact(&mut values, &keys, &history, &members, &window, cancelled)?;
        measures.tracks = tracks;
        return Ok(serde_json::to_value(build_reply(
            source,
            &params,
            context,
            &history,
            &identities,
            &members,
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
        let (tracks, sums) = exact(&mut values, &keys, &history, &members, &window, cancelled)?;
        measures.tracks = tracks;
        let reply = build_reply(
            source,
            &params,
            context,
            &history,
            &identities,
            &members,
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
    let present_keys = match present_keys {
        Some(present_keys) => present_keys,
        None => {
            // A dense factory summary joins territory's member presence: it
            // waits for territory's coverage through the window, as a request
            // waiting for the branch's coverage does.
            let covered = crate::prepared::coverage::product_coverage(
                source,
                context.content.as_ref(),
                branch,
                <territory::Territory as Product>::KEY,
                cancelled,
            )?
            .is_some_and(|coverage| {
                coverage.complete || f64::from(coverage.last_hour) >= view.1.ceil()
            });
            if !covered {
                return Ok(Value::Null);
            }
            let Some((present_keys, _)) = territory_catalogue()? else {
                return Ok(Value::Null);
            };
            present_keys
        }
    };
    dense_reply(
        source,
        context,
        &params,
        &history,
        &identities,
        &members,
        measures,
        &mut values,
        &keys,
        &present_keys,
        &store,
        &key,
        branch,
        view,
        cancelled,
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
    values: &mut Reader<'_>,
    keys: &BTreeMap<String, u64>,
    present_keys: &BTreeMap<String, u64>,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    view: (f64, f64, u32),
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let window = HourSpan {
        first_hour: view.0.floor() as i32,
        last_hour: view.1.floor() as i32,
    };
    let bounded = members.clone().within(&window, cancelled)?;
    let mut present = Reader::new(store, key, format!("territory-present-{branch}"), cancelled)?;
    let mut rosters = Reader::new(store, key, format!("territory-rosters-{branch}"), cancelled)?;
    let mut summaries = BTreeMap::new();
    let mut member_summaries = BTreeMap::new();
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
        let summary = dense(
            values,
            keys,
            &mut present,
            present_keys,
            &mut rosters,
            &bounded,
            &tags,
            &known,
            &unknown,
            view,
            cancelled,
        )?;
        series::summary_budget(view.0, view.1, view.2, summaries.len() + 1)?;
        summaries.insert(id.clone(), summary);
        if params.parts == Some(true) {
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
                        &mut present,
                        present_keys,
                        &mut rosters,
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
    }

    // D-12 permits extra dense lines so a leader inside a column stays visible.
    if params.faction_id.is_none() && params.inventory != Some(true) {
        retain_dense_top_candidates(&mut summaries, params.top, cancelled)?;
    }
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
        let (point_tracks, point) = exact(
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
        series.membership.clear();
        for bucket in summaries[&series.id].buckets.iter().flatten() {
            country_groups::mark_unobserved(
                &mut series.membership,
                bucket.first_hour,
                bucket.last_hour,
            );
        }
    }
    let mut reply = serde_json::to_value(reply)?;
    reply["summaries"] = serde_json::to_value(summaries)?;
    if params.parts == Some(true) {
        reply["memberSummaries"] = serde_json::to_value(member_summaries)?;
    }
    super::super::check_reply_budget(&reply)?;
    Ok(reply)
}

/// A column's N-th largest minimum bounds every faction that can enter Top N.
/// Equality keeps tied candidates; exact ranking stays in the reached-hour sum.
fn retain_dense_top_candidates(
    summaries: &mut BTreeMap<String, ChartSummary>,
    top: u64,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let count = summaries
        .values()
        .next()
        .map_or(0, |summary| summary.buckets.len());
    for column in 0..count {
        check_cancel(cancelled)?;
        let mut minimums: Vec<_> = summaries
            .values()
            .filter_map(|summary| summary.buckets[column].map(|bucket| bucket.min_value))
            .collect();
        minimums.sort_by(|a, b| b.total_cmp(a));
        let rank = top.min(minimums.len() as u64) as usize;
        let threshold = rank.checked_sub(1).map(|index| minimums[index]);
        for summary in summaries.values_mut() {
            summary.buckets[column] = summary.buckets[column]
                .filter(|bucket| threshold.is_some_and(|minimum| bucket.max_value >= minimum));
        }
    }
    summaries.retain(|_, summary| summary.buckets.iter().any(Option::is_some));
    Ok(())
}

#[cfg(test)]
mod tests;
