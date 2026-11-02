//! Country factories' replies, read from the family's series publication ([`super::emit`]): the
//! chart summaries of the selected or ranked tags with their values, their values at hours,
//! keyboard navigation, the scoreboard's cells and the faction totals' member counts.

use super::{
    CountryFactories, FIELD_COUNT, FactoriesParams, FactoryCount, FactoryGroupKey,
    FactoryReadRun, FactoryRun, FactoryTop, ScannedFactories, channel,
    emit::{COUNTS, FAMILY, HOURLY, MIX, PARTS, RANKS, READS, REFUSED, UNEXPLAINED, VALUES, kind},
    present,
};
use crate::{
    Source,
    prepared::{
        coverage::Coverage,
        engine,
        series::{self, Navigation},
        series_read::{Published, SeriesRead},
    },
    prepared_chart::ChartSummary,
    products::{
        LoadContext, Product,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{self, RecordedHistories},
    },
    ranking::for_each_piece,
};
use dashboard_series::{
    Cell, KeyHandle, PageRead, Selection, Snapshot, codec::Binary, family::Directory,
    family::Scalar, query::EventView,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// What the publication's reads found: the country-hours whose `unavailable` is not the sum
/// of its parts by group, the unreadable rows and unnamed country-hours, and every tag that
/// held values.
pub fn scanned(directory: &Directory) -> ScannedFactories {
    let list = |id| {
        directory
            .scalar(id)
            .map_or(&[][..], Scalar::list)
            .iter()
            .map(|count| {
                count
                    .integer()
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>()
    };
    let counts = list(COUNTS);
    ScannedFactories {
        unexplained: FactoryGroupKey::ALL
            .into_iter()
            .zip(list(UNEXPLAINED))
            .filter(|&(_, count)| count > 0)
            .collect(),
        unreadable: counts.first().copied().unwrap_or(0),
        unnamed: counts.get(1).copied().unwrap_or(0),
        recorded: directory
            .actors
            .iter()
            .enumerate()
            .filter(|&(actor, _)| directory.key(actor as u64, MIX, None).is_some())
            .map(|(_, actor)| String::from_utf8_lossy(&actor.name).into_owned())
            .collect(),
        ..ScannedFactories::default()
    }
}

/// The tag actor of `tag`, when the family ever named it.
fn actor(directory: &Directory, tag: &str) -> Option<u64> {
    directory.actor(tag.as_bytes())
}

/// The tag `actor` names.
fn tag(directory: &Directory, actor: i128) -> io::Result<String> {
    u64::try_from(actor)
        .ok()
        .and_then(|actor| directory.actors.get(actor as usize))
        .map(|actor| String::from_utf8_lossy(&actor.name).into_owned())
        .ok_or_else(|| invalid("A prepared factory rank names an unknown country"))
}

/// `tag`'s key of `measure`, split by `dimension`.
fn key(directory: &Directory, tag: &str, measure: u64, dimension: Option<u64>) -> Option<u64> {
    actor(directory, tag).and_then(|actor| directory.key(actor, measure, dimension))
}

/// The run of `key` holding `hour`, whole.
fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(match key {
        Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run),
        None => None,
    })
}

/// The values a [`kind::MIX`] event holds, in [`super::fields`] order.
pub(super) fn mix_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: EventView,
) -> io::Result<Vec<i32>> {
    let malformed = || invalid("Prepared factory composition is not valid");
    let EventView::Family(event) = event else {
        return Err(invalid("Missing prepared factory composition"));
    };
    if event.kind != kind::MIX || event.fields.len != FIELD_COUNT as u64 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let mut values = Vec::with_capacity(FIELD_COUNT);
    for at in 0..FIELD_COUNT as u64 {
        let Cell::Integer(value) = snapshot.field(event.fields, at)? else {
            return Err(malformed());
        };
        values.push(i32::try_from(value).map_err(|_| malformed())?);
    }
    Ok(values)
}

/// `key`'s values at `hour`: its latest composition by then.
fn composition<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Vec<i32>> {
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            hour,
            None,
        )?
        .ok_or_else(|| invalid("Missing prepared factory composition"))?;
    mix_of(snapshot, event)
}

/// The supplying read a READS run holds over its hours.
fn supplying(run: &dashboard_series::Run) -> io::Result<FactoryReadRun> {
    let hourly = run.value == HOURLY;
    let read_hour = if hourly {
        run.span.first
    } else {
        i32::try_from(run.value).map_err(|_| invalid("Prepared factory read is not valid"))?
    };
    Ok(FactoryReadRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        read_hour,
        hourly,
    })
}

/// A chart of `key` over `view`, its bins laid from `bounds`, within `spans`; every bin
/// unknown where the family never named the key; no bins, and nothing read, for empty bounds.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    spans: &[HourSpan],
) -> io::Result<ChartSummary> {
    if let Some(key) = key.filter(|_| bounds.0 <= bounds.1) {
        return engine::summary(snapshot, key, bounds, view, 0, Some(spans));
    }
    let (exact, bins) = series::summary_bins(bounds, view)?;
    Ok(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets: vec![None; bins.len()],
        runs: exact.then(Vec::new),
        approximate: Vec::new(),
    })
}

/// The hours `spans` hold inside `within`.
fn clipped(
    spans: &[HourSpan],
    within: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<HourSpan>> {
    let mut clipped = Vec::new();
    for_each_piece(
        spans,
        within,
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
    Ok(clipped)
}

/// The dates a summary's buckets hold.
fn dates_of(summary: &ChartSummary, dates: &mut BTreeSet<i32>) {
    for bucket in summary.buckets.iter().flatten() {
        dates.extend([
            bucket.first_hour,
            bucket.last_hour,
            bucket.min_hour,
            bucket.max_hour,
        ]);
    }
}

/// A series request of the `country-factories` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot`, the publication pinned through `accepted`,
/// the coverage it shares with the identity history; pending before there is any.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    accepted: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let reads: Vec<crate::prepared_measure::query::MeasureRead> = if product == "series-state" {
        serde_json::from_value(params["reads"].clone())?
    } else {
        Vec::new()
    };
    let read_tags: Vec<_> = reads.iter().map(|read| read.tag.clone()).collect();
    let selection: FactoriesParams = serde_json::from_value(json!({
        "branchId": params["branchId"],
        "tags": if product == "series-state" {
            json!(read_tags)
        } else {
            params.get("tags").cloned().unwrap_or_else(|| json!([]))
        },
        "top": params["family"]["top"],
    }))?;
    CountryFactories::validate(&selection).map_err(crate::invalid)?;
    let branch: u64 = selection
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid factory branch"))?;
    let Some(shared) = accepted.clone() else {
        return Ok(Value::Null);
    };
    // The factories may be published past the identities: nothing past their shared coverage
    // is read.
    let reach = (!shared.complete)
        .then(|| shared.last_hour.min(snapshot.publication().accepted.last));
    let through = |last: i32| reach.map_or(last, |reach| last.min(reach));
    let coverage = crate::prepared::prefix::extent(source, branch, accepted)?;
    let mut scanned = scanned(directory);
    let rank: FactoryTop = serde_json::from_value(if params["family"]["top"].is_null() {
        json!({
            "count": 1,
            "mode": params["family"].get("mode").cloned().unwrap_or_else(|| json!("free")),
            "groups": params["family"].get("groups").cloned().unwrap_or_else(|| json!([])),
        })
    } else {
        params["family"]["top"].clone()
    })?;
    CountryFactories::validate(&FactoriesParams {
        branch_id: selection.branch_id.clone(),
        tags: Vec::new(),
        top: Some(rank.clone()),
    })
    .map_err(crate::invalid)?;
    let index = channel(&rank) as u64;
    // The history's tags the family named, which ranks and keys answer for.
    let named = history
        .countries
        .keys()
        .filter(|tag| actor(directory, tag).is_some())
        .count();
    let positions: Vec<KeyHandle> = directory
        .dimensions(FAMILY, RANKS + index)
        .iter()
        .map(|&(_, key)| key)
        .collect();
    let ranked = |top: u32| &positions[..(top as usize).min(named).min(positions.len())];
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    if product == "series-next-change" {
        let mut navigation: Navigation = serde_json::from_value(params.clone())?;
        if let Some(reach) = reach.map(f64::from) {
            if reach < navigation.first_hour {
                return Ok(json!({ "hour": null, "coverage": coverage, "approximate": false }));
            }
            navigation.last_hour = navigation.last_hour.min(reach);
        }
        // A selected tag navigates only the hours a selected native country holds it.
        let selected = if selection.top.is_none() {
            country_history::selected_spans(history, &selection.tags, &within, cancelled)?
        } else {
            BTreeMap::new()
        };
        let mut keys: Vec<(u64, Option<&[HourSpan]>)> = Vec::new();
        if let Some(top) = &selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut values = Vec::new();
            for &position in ranked(top.count) {
                keys.push((position, None));
                if let Some(run) = run_at(snapshot, Some(position), hour)? {
                    let tag = tag(directory, run.value)?;
                    values.extend(key(directory, &tag, VALUES, Some(index)));
                }
            }
            keys.extend(values.into_iter().map(|key| (key, None)));
        } else {
            for (tag, spans) in &selected {
                if let Some(key) = key(directory, tag, VALUES, Some(index)) {
                    keys.push((key, Some(spans.as_slice())));
                }
            }
        }
        let hour = engine::navigate(snapshot, &keys, &navigation)?;
        return Ok(json!({ "hour": hour, "coverage": coverage, "approximate": false }));
    }
    let (from, to, width) = if product == "series-state" {
        (
            f64::from(reads.iter().map(|read| read.hour).min().unwrap_or(0)),
            f64::from(reads.iter().map(|read| read.hour).max().unwrap_or(0)),
            1,
        )
    } else {
        series::viewport(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["width"]
                .as_u64()
                .and_then(|width| u32::try_from(width).ok()),
        )?
    };
    let (first, last) = (from.floor() as i32, to.ceil() as i32);
    let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
        selection.top.as_ref().map(|_| BTreeMap::new());
    if let (Some(top), Some(members)) = (&selection.top, &mut membership) {
        for &position in ranked(top.count) {
            for run in engine::runs(snapshot, position, first, through(last))? {
                let tag = tag(directory, run.value)?;
                if !history.countries.contains_key(&tag) {
                    return Err(invalid("Unknown factory rank key"));
                }
                members.entry(tag).or_default().push(HourSpan {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                });
            }
        }
        for spans in members.values_mut() {
            spans.sort_by_key(|span| span.first_hour);
            let mut merged = Vec::new();
            for span in std::mem::take(spans) {
                mark_unobserved(&mut merged, span.first_hour, span.last_hour);
            }
            *spans = merged;
        }
    }
    let mut wanted: Vec<String> = membership.as_ref().map_or_else(
        || country_history::selected_aliases(history, &selection.tags),
        |members| members.keys().cloned().collect(),
    );
    if product == "series-state" {
        wanted = read_tags;
        wanted.sort();
        wanted.dedup();
    }
    let selected = country_history::selected_identities(history, &selection.tags);
    let dense =
        product != "series-state" && i64::from(last) - i64::from(first) + 1 > i64::from(width);
    let mut display = Vec::new();
    for tag in &wanted {
        crate::metadata::check_cancel(cancelled)?;
        let mix = key(directory, tag, MIX, None);
        let supply = key(directory, tag, READS, None);
        let mut spans: Vec<_> = if let Some(members) = &membership {
            members.get(tag).cloned().unwrap_or_default()
        } else {
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect()
        };
        if let Some(within) = within.get(tag) {
            spans = clipped(&spans, within, cancelled)?;
        }
        let view = (from, to, width);
        let bounds = (first, through(last));
        let line = summary(
            snapshot,
            key(directory, tag, VALUES, Some(index)),
            bounds,
            view,
            &spans,
        )?;
        let mut dates = BTreeSet::new();
        let mut parts = Vec::new();
        if dense {
            for ordinal in 0..FIELD_COUNT {
                let part = key(directory, tag, PARTS, Some(ordinal as u64));
                let part = summary(snapshot, part, bounds, view, &spans)?;
                dates_of(&part, &mut dates);
                parts.push(json!({ "key": ordinal.to_string(), "summary": part }));
            }
            dates_of(&line, &mut dates);
        }
        let runs: Vec<dashboard_series::Run> = if dense || product == "series-state" {
            if product == "series-state" {
                dates.extend(
                    reads
                        .iter()
                        .filter(|read| &read.tag == tag && read.hour == through(read.hour))
                        .map(|read| read.hour),
                );
            }
            let mut runs = BTreeMap::new();
            for &hour in &dates {
                if let Some(run) = run_at(snapshot, mix, hour)? {
                    if product == "series-state" {
                        let starts = run.break_before && run.span.first == hour;
                        runs.insert(
                            hour,
                            dashboard_series::Run {
                                span: dashboard_series::Span::new(hour, hour)?,
                                break_before: starts,
                                ..run
                            },
                        );
                    } else {
                        runs.insert(run.span.first, run);
                    }
                }
            }
            runs.into_values().collect()
        } else {
            let mut runs = match mix {
                Some(key) => engine::runs(snapshot, key, first, through(last))?,
                None => Vec::new(),
            };
            // A run the window clips starts no segment at the window's first hour.
            if let Some(head) = runs.first_mut()
                && head.break_before
                && head.span.first == first
                && run_at(snapshot, mix, first)?.is_some_and(|run| run.span.first < first)
            {
                head.break_before = false;
            }
            runs
        };
        let mut raw = Vec::new();
        for run in runs {
            let key = mix.ok_or_else(|| invalid("Missing prepared factory composition"))?;
            let values = composition(snapshot, key, run.span.first)?;
            let (first_hour, last_hour) = if product == "series-state" {
                (run.span.first, run.span.last)
            } else {
                (run.span.first.max(first), run.span.last.min(through(last)))
            };
            if first_hour <= last_hour {
                raw.push(FactoryRun {
                    first_hour,
                    last_hour,
                    starts_segment: run.break_before,
                    values,
                });
            }
        }
        raw.sort_by_key(|run| run.first_hour);
        let read_runs = if dense || product == "series-state" {
            let mut found = BTreeMap::new();
            for run in &raw {
                for hour in [run.first_hour, run.last_hour] {
                    if let Some(read) = run_at(snapshot, supply, hour)? {
                        found.insert(read.span.first, read);
                    }
                }
            }
            found.into_values().collect()
        } else {
            match supply {
                Some(key) => engine::runs(snapshot, key, first, through(last))?,
                None => Vec::new(),
            }
        };
        let provenance = read_runs
            .iter()
            .map(supplying)
            .collect::<io::Result<Vec<_>>>()?;
        scanned.reads.insert(tag.clone(), provenance);
        scanned.series.insert(tag.clone(), raw);
        scanned.segments.insert(
            tag.clone(),
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.segments.iter())
                .filter(|span| span.first_hour <= last && span.last_hour >= first)
                .cloned()
                .collect(),
        );
        if product != "series-state" {
            series::summary_budget(from, to, width, display.len() + 1)?;
            display.push(json!({ "tag": tag, "summary": line, "parts": parts }));
        }
    }
    let mut params = selection;
    params.tags = wanted;
    let reply = present(
        source,
        &params,
        context,
        cancelled,
        &scanned,
        Some(history),
        membership,
    )?;
    let mut reply = serde_json::to_value(reply)?;
    reply["summaries"] = json!(display);
    reply["coverage"] = json!(coverage);
    crate::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// `top`'s counts by tag actor, as faction totals and the scoreboard read them.
pub fn counts<'r, 's, P: PageRead>(
    cell: &'r RefCell<&'r mut Snapshot<'s, P>>,
    directory: &'r Directory,
    top: &FactoryTop,
) -> Published<'r, 's, P> {
    let index = channel(top) as u64;
    Published::new(cell, move |actor| directory.key(actor, VALUES, Some(index)))
}

/// Each tag's count under `mode` over every group at `hour`; none where it held no values.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    mode: FactoryCount,
    hour: i32,
) -> io::Result<Vec<Option<i128>>> {
    let cell = RefCell::new(&mut *snapshot);
    let top = FactoryTop {
        count: 1,
        mode,
        groups: Vec::new(),
    };
    let mut values = counts(&cell, directory, &top);
    tags.iter()
        .map(|tag| match actor(directory, tag) {
            Some(actor) => Ok(values.value_at(actor, hour)?.map(|run| run.value)),
            None => Ok(None),
        })
        .collect()
}

/// Each history tag's actor where it holds `top`'s count, for the faction totals' member
/// values ([`counts`]).
pub fn member_keys(
    directory: &Directory,
    history: &RecordedHistories,
    top: &FactoryTop,
) -> BTreeMap<String, u64> {
    let index = channel(top) as u64;
    history
        .countries
        .keys()
        .filter_map(|tag| {
            let actor = actor(directory, tag)?;
            directory.key(actor, VALUES, Some(index))?;
            Some((tag.clone(), actor))
        })
        .collect()
}

/// The reads refused at `hour`: unresolved, repeated or unshowable rows and tags that are not
/// text; none at an hour not prepared.
pub fn refused_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<Option<i128>> {
    run_at(snapshot, directory.key(FAMILY, REFUSED, None), hour)
        .map(|run| run.map(|run| run.value))
}

