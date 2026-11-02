//! Subject autonomy's series replies, read from the family's publication ([`super::emit`]): the
//! Autonomy widget's chart summaries, its subjects' values at hours and its keyboard navigation,
//! each with the identities of the countries it names.

use super::{
    Autonomy, AutonomyParams, AutonomyRun, ScannedAutonomy,
    emit::{
        BAND, FAMILY, LEVELS, LOWER, OVERLORDS, RATIO, RECORDED, RELATIVE, SCHEMA, SCORE, SHARED,
        UNIDENTIFIED, UPPER, kind,
    },
    selection,
};
use dashboard_base::{
    Source,
    prepared::{
        coverage, engine, published,
        query::progressive,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, merge},
    products::{LoadContext, Product, country_groups::HourSpan},
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};

/// Rows one event page answers at most: replies stay bounded by the reply budget, not this.
const PAGE: usize = 1 << 24;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Requested {
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    within: BTreeMap<String, Vec<(String, i32, i32, bool)>>,
    tag: Option<String>,
}

#[derive(Deserialize)]
struct Read {
    hour: i32,
}

/// The Autonomy widget's series request (`series-summary`, `series-state` or
/// `series-next-change`) answered from the family's publication pinned through its coverage, with
/// the identities of the countries it names; null before the family publishes the branch. Every
/// hour past the coverage is pending.
pub fn series(
    source: &Source,
    product: &str,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let (params, requested) = parse(value)?;
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid autonomy branch"))?;
    let accepted = match coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        "autonomy",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        other => other?,
    };
    let Some(shared) = &accepted else {
        return Ok(Value::Null);
    };
    // Pinned through the coverage it shares with the identities: no read passes it.
    let Some(opened) = published::open_through(
        source,
        context.content.as_ref(),
        "autonomy",
        SCHEMA,
        branch,
        (!shared.complete).then_some(shared.last_hour),
    )?
    else {
        return Ok(Value::Null);
    };
    let mut result = opened.read(cancelled, |snapshot, directory| {
        reply(
            source, product, value, &params, &requested, snapshot, directory, context, cancelled,
        )
    })?;
    let requested_tags: Vec<String> = params
        .subjects
        .iter()
        .chain(&params.overlords)
        .chain(
            requested
                .within
                .values()
                .flatten()
                .map(|(tag, _, _, _)| tag),
        )
        .filter(|tag| !tag.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let _cpu = replay_frame::cpu::scope_phase("autonomy:query_identities_finish");
    let none = BTreeMap::new();
    let history = progressive::histories(
        store,
        key,
        branch,
        accepted.as_ref(),
        &none,
        &none,
        source.replay_ref(),
        &requested_tags,
        cancelled,
    )?;
    result["identities"] = serde_json::to_value(
        history
            .countries
            .iter()
            .map(|(tag, country)| serde_json::json!({"tag": tag, "identities": country.identities}))
            .collect::<Vec<_>>(),
    )?;
    let mut prefix = dashboard_base::prepared::prefix::Prefix::new(
        source,
        branch,
        dashboard_base::prepared::prefix::extent(source, branch, accepted)?,
        None,
    );
    prefix.sampled(&[]);
    prefix.mark(&mut result)?;
    Ok(result)
}

/// The request's selection and what it asks.
fn parse(value: &Value) -> io::Result<(AutonomyParams, Requested)> {
    let params: AutonomyParams = serde_json::from_value(serde_json::json!({
        "branchId": value["branchId"],
        "subjects": value.get("subjects").unwrap_or(&serde_json::json!([])),
        "overlords": value.get("overlords").unwrap_or(&serde_json::json!([])),
    }))?;
    Autonomy::validate(&params).map_err(dashboard_core::invalid)?;
    Ok((params, serde_json::from_value(value.clone())?))
}

/// `product`'s reply to `value` from `snapshot`, without the identities and coverage.
#[cfg(test)]
pub(super) fn answer<P: PageRead>(
    source: &Source,
    product: &str,
    value: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    context: &LoadContext,
) -> io::Result<Value> {
    let (params, requested) = parse(value)?;
    reply(
        source,
        product,
        value,
        &params,
        &requested,
        snapshot,
        directory,
        context,
        &AtomicBool::new(false),
    )
}

/// A subject's band: the hour it starts, the last hour the subject was observed before, whether
/// its runs start anew there, its overlord, its level's place among the scan's levels, whether
/// no level lies below or above it, and whether a relation names its ratio.
struct Band {
    first: i32,
    previous: Option<i32>,
    starts: bool,
    overlord: Option<String>,
    level: u32,
    lowest: bool,
    highest: bool,
    ratio: bool,
}

impl Band {
    /// It as the reply's run through `last`; its values are read at each point.
    fn run(&self, last: i32) -> AutonomyRun {
        AutonomyRun {
            first_hour: self.first,
            last_hour: last,
            starts_segment: self.starts,
            overlord: self.overlord.clone(),
            level: self.level,
            score_raw: String::new(),
            lower_raw: String::new(),
            upper_raw: String::new(),
            lowest: self.lowest,
            highest: self.highest,
            ratio_raw: self.ratio.then(String::new),
        }
    }
}

/// The band a [`kind::BAND`] event holds.
fn band<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: EventView) -> io::Result<Band> {
    let malformed = || invalid("A prepared autonomy band is not valid");
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind::BAND || event.fields.len != 7 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let flag = |cell: Cell| match cell {
        Cell::Boolean(flag) => Ok(flag),
        _ => Err(malformed()),
    };
    let previous = match snapshot.field(event.fields, 0)? {
        Cell::Null => None,
        Cell::Integer(hour) => Some(i32::try_from(hour).map_err(|_| malformed())?),
        _ => return Err(malformed()),
    };
    let starts = flag(snapshot.field(event.fields, 1)?)?;
    let overlord = match snapshot.field(event.fields, 2)? {
        Cell::Null => None,
        Cell::Text(blob) => {
            let length = usize::try_from(blob.len).map_err(|_| malformed())?;
            let mut text = vec![0; length];
            snapshot.cell_bytes(blob, 0, &mut text)?;
            Some(String::from_utf8(text).map_err(|_| malformed())?)
        }
        _ => return Err(malformed()),
    };
    let Cell::Unsigned(level) = snapshot.field(event.fields, 3)? else {
        return Err(malformed());
    };
    Ok(Band {
        first: event.native_hour,
        previous,
        starts,
        overlord,
        level: u32::try_from(level).map_err(|_| malformed())?,
        lowest: flag(snapshot.field(event.fields, 4)?)?,
        highest: flag(snapshot.field(event.fields, 5)?)?,
        ratio: flag(snapshot.field(event.fields, 6)?)?,
    })
}

/// A subject's latest band, the last hour it was observed, and whether it is its only band.
struct Latest {
    band: Band,
    last: i32,
    single: bool,
}

fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    accepted: Span,
) -> io::Result<Option<Latest>> {
    let selection = Selection {
        key,
        membership: None,
    };
    let Some(event) = snapshot.latest_event(selection, accepted.last, None)? else {
        return Ok(None);
    };
    let band = band(snapshot, event)?;
    let last = engine::runs(snapshot, key, band.first, accepted.last)?
        .first()
        .map(|run| run.span.last)
        .ok_or_else(|| invalid("A prepared autonomy band holds no hour"))?;
    let single = match band.first.checked_sub(1) {
        Some(before) if before >= accepted.first => snapshot
            .latest_event(selection, before, None)?
            .is_none(),
        _ => true,
    };
    Ok(Some(Latest { band, last, single }))
}

/// The bands whose events lie in `first..=last`, in hour order.
fn bands_within<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    first: i32,
    last: i32,
) -> io::Result<Vec<Band>> {
    let selection = [Selection {
        key,
        membership: None,
    }];
    let mut events = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native: Span::new(first, last)?,
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            PAGE,
            |event| {
                events.push(event);
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    events
        .into_iter()
        .map(|event| band(snapshot, event))
        .collect()
}

/// `run` as the reader's run.
fn reader_run(run: dashboard_series::Run) -> series::Run {
    series::Run::new(run.span.first, run.span.last, run.value, run.break_before)
}

/// `key`'s runs over `from..=to`, those at either end whole.
fn numeric_runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    (from, to): (i32, i32),
) -> io::Result<Vec<series::Run>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let mut runs: Vec<series::Run> = engine::runs(snapshot, key, from, to)?
        .into_iter()
        .map(reader_run)
        .collect();
    if let Some(run) = runs.first_mut()
        && run.first_hour == from
        && let Some((whole, _)) = engine::run_at(snapshot, key, from)?
    {
        *run = reader_run(whole);
    }
    if let Some(run) = runs.last_mut()
        && run.last_hour == to
        && let Some((whole, _)) = engine::run_at(snapshot, key, to)?
    {
        *run = reader_run(whole);
    }
    Ok(runs)
}

/// A family scalar's list.
fn list(directory: &Directory, id: u64) -> &[Scalar] {
    directory.scalar(id).map_or(&[][..], Scalar::list)
}

fn text(scalar: &Scalar) -> io::Result<String> {
    scalar
        .text()
        .map(|text| String::from_utf8_lossy(text).into_owned())
        .ok_or_else(|| invalid("A prepared autonomy name is not text"))
}

/// What the reply reads of the publication but the subjects' runs: the levels with their lowest
/// move-down scores, the subjects in the order first seen, each overlord's subjects, the
/// subject-hours with more than one overlord, and the hours recorded and partly known.
pub(super) fn scanned<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    accepted: Span,
) -> io::Result<ScannedAutonomy> {
    let malformed = || invalid("Prepared autonomy levels are not valid");
    let mut levels = Vec::new();
    let mut lowest_lower = Vec::new();
    for level in list(directory, LEVELS) {
        let [id, lowest] = level.list() else {
            return Err(malformed());
        };
        levels.push(text(id)?);
        lowest_lower.push(
            lowest
                .integer()
                .and_then(|lowest| i64::try_from(lowest).ok())
                .ok_or_else(malformed)?,
        );
    }
    let mut overlord_subjects: HashMap<String, BTreeSet<String>> = HashMap::new();
    for held in list(directory, OVERLORDS) {
        let [overlord, subjects] = held.list() else {
            return Err(malformed());
        };
        overlord_subjects.insert(
            text(overlord)?,
            subjects.list().iter().map(text).collect::<io::Result<_>>()?,
        );
    }
    let mut spans = |measure| -> io::Result<Vec<HourSpan>> {
        Ok(match directory.key(FAMILY, measure, None) {
            Some(key) => engine::runs(snapshot, key, accepted.first, accepted.last)?
                .into_iter()
                .map(|run| HourSpan {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                })
                .collect(),
            None => Vec::new(),
        })
    };
    let recorded = spans(RECORDED)?;
    let unidentified = spans(UNIDENTIFIED)?;
    Ok(ScannedAutonomy {
        levels,
        lowest_lower,
        subjects: directory
            .actors
            .iter()
            .map(|actor| (String::from_utf8_lossy(&actor.name).into_owned(), Vec::new()))
            .collect(),
        shared_subjects: directory
            .scalar(SHARED)
            .and_then(Scalar::integer)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0),
        recorded,
        unidentified,
        overlord_subjects,
    })
}

/// The reply to one request, as the row family's prepared reader answered it.
#[allow(clippy::too_many_arguments)]
fn reply<P: PageRead>(
    source: &Source,
    product: &str,
    value: &Value,
    params: &AutonomyParams,
    requested: &Requested,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let hours: Vec<_> = requested.reads.iter().map(|read| read.hour).collect();
    let navigation = if product == "series-next-change" {
        let navigation: Navigation = serde_json::from_value(value.clone())?;
        Some((
            requested.tag.clone().unwrap_or_default(),
            navigation.origin()?,
            matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight),
        ))
    } else {
        None
    };
    let (from, to, width) = if product == "series-summary" {
        series::viewport(requested.from, requested.to, requested.width)?
    } else if product == "series-next-change" {
        let navigation: Navigation = serde_json::from_value(value.clone())?;
        (navigation.first_hour, navigation.last_hour, 1)
    } else {
        let hour = f64::from(hours.first().copied().unwrap_or(0));
        (hour, hour, 1)
    };
    series::summary_budget(from, to, width, requested.within.len())?;
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid autonomy branch"))?;
    let accepted = snapshot.publication().accepted;
    let mut scan = scanned(snapshot, directory, accepted)?;
    let indices: HashMap<String, u64> = scan
        .subjects
        .iter()
        .enumerate()
        .map(|(index, (tag, _))| (tag.clone(), index as u64))
        .collect();
    let key = |index: u64, measure: u64| directory.key(index, measure, None);
    let wanted = selection(params, &scan.overlord_subjects);
    let mut chosen = params.clone();
    chosen.overlords.clear();
    chosen.subjects = scan
        .subjects
        .iter()
        .filter(|(tag, _)| wanted(tag))
        .map(|(tag, _)| tag.clone())
        .collect();
    let chosen_tags: HashSet<String> = chosen.subjects.iter().cloned().collect();
    let selected_aliases: HashSet<&str> = requested
        .within
        .values()
        .flatten()
        .map(|(tag, _, _, _)| tag.as_str())
        .collect();
    let bounds = source
        .with_metadata(|metadata| {
            metadata
                .branches
                .iter()
                .find(|start| start.id.parse::<u64>().ok() == Some(branch))
                .map(|start| (start.first_hour, start.last_hour))
        })
        .ok_or_else(|| invalid("Unknown autonomy branch"))?;
    if let Some((selected, origin, forward)) = &navigation {
        let mut candidates = Vec::new();
        for (tag, first, last, _) in requested.within.get(selected).into_iter().flatten() {
            let Some(&index) = indices
                .get(tag.as_str())
                .filter(|_| chosen_tags.contains(tag.as_str()))
            else {
                continue;
            };
            let lo = (*first).max(from.ceil() as i32);
            let hi = (*last).min(to.floor() as i32);
            if lo > hi {
                continue;
            }
            if f64::from(lo) > *origin && *forward {
                candidates.push(lo);
            }
            let keys: Vec<(u64, Option<&[HourSpan]>)> = [SCORE, LOWER, UPPER, RATIO]
                .into_iter()
                .filter_map(|measure| key(index, measure))
                .map(|key| (key, Some(scan.recorded.as_slice())))
                .collect();
            if let Some(hour) = engine::navigate(
                snapshot,
                &keys,
                &Navigation {
                    from: Some(*origin),
                    first_hour: f64::from(lo),
                    last_hour: f64::from(hi),
                    key: if *forward {
                        MeasureKey::ArrowRight
                    } else {
                        MeasureKey::ArrowLeft
                    },
                },
            )?
            .filter(|hour| {
                if *forward {
                    *hour > *origin
                } else {
                    *hour < *origin
                }
            }) {
                candidates.push(hour as i32);
            }
        }
        let reached = candidates
            .into_iter()
            .reduce(|a, b| if *forward { a.min(b) } else { a.max(b) });
        return Ok(serde_json::json!({ "hour": reached }));
    }
    let reached_hours: BTreeSet<_> = hours.iter().copied().collect();
    let mut windows: Vec<(i32, i32)> = Vec::new();
    if product == "series-state" {
        for &hour in &reached_hours {
            if hour < bounds.0 || hour > bounds.1 {
                continue;
            }
            match windows.last_mut() {
                Some((_, last)) if i32::checked_add(*last, 1) == Some(hour) => *last = hour,
                _ => windows.push((hour, hour)),
            }
        }
    } else {
        let lo = (from.floor() as i32).max(bounds.0);
        let hi = (to.floor() as i32).min(bounds.1);
        if lo <= hi {
            windows.push((lo, hi));
        }
    }
    let mut bands: Vec<Vec<AutonomyRun>> = vec![Vec::new(); scan.subjects.len()];
    for (index, (tag, _)) in scan.subjects.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !chosen_tags.contains(tag.as_str()) {
            continue;
        }
        let band_key = key(index as u64, BAND).ok_or_else(|| invalid("Missing autonomy band"))?;
        let latest = latest(snapshot, band_key, accepted)?
            .ok_or_else(|| invalid("A prepared autonomy subject holds no band"))?;
        let held = &mut bands[index];
        for &(lo, hi) in &windows {
            if latest.band.first <= lo || latest.single {
                if latest.band.first <= hi && lo <= latest.last {
                    held.push(latest.band.run(latest.last));
                }
                continue;
            }
            let at_lo = snapshot.latest_event(
                Selection {
                    key: band_key,
                    membership: None,
                },
                lo,
                None,
            )?;
            if let Some(event) = at_lo {
                held.push(band(snapshot, event)?.run(latest.last));
            }
            if lo < hi {
                for next in bands_within(snapshot, band_key, lo + 1, hi)? {
                    if let Some(previous) = held.last_mut() {
                        previous.last_hour =
                            next.previous.unwrap_or(next.first.saturating_sub(1));
                    }
                    held.push(next.run(latest.last));
                }
            }
        }
        held.sort_by_key(|run| run.first_hour);
        held.dedup_by(|next, kept| {
            if next.first_hour != kept.first_hour {
                return false;
            }
            kept.last_hour = kept.last_hour.max(next.last_hour);
            true
        });
        for at in 1..held.len() {
            let end = held[at].first_hour.saturating_sub(1);
            held[at - 1].last_hour = held[at - 1].last_hour.min(end);
        }
    }
    let fine = (to.floor().min(f64::from(bounds.1)) - from.floor().max(f64::from(bounds.0)) + 1.0)
        < 2.0 * f64::from(width);
    for (index, (tag, runs)) in scan.subjects.iter_mut().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !chosen_tags.contains(tag.as_str()) {
            continue;
        }
        let dense = selected_aliases.contains(tag.as_str()) && fine;
        // Only points read the values: a summary drawn from the bins reads none.
        let read = product == "series-state" || dense || !reached_hours.is_empty();
        let mut fields: [Vec<series::Run>; 4] = Default::default();
        if read {
            for (values, measure) in fields.iter_mut().zip([SCORE, LOWER, UPPER, RATIO]) {
                let key = key(index as u64, measure);
                for &(lo, hi) in &windows {
                    if !bands[index]
                        .iter()
                        .any(|run| run.first_hour <= hi && lo <= run.last_hour)
                    {
                        continue;
                    }
                    let lo = lo.max(
                        bands[index]
                            .first()
                            .expect("intersecting autonomy band")
                            .first_hour,
                    );
                    let hi = hi.min(
                        bands[index]
                            .last()
                            .expect("intersecting autonomy band")
                            .last_hour,
                    );
                    values.extend(numeric_runs(snapshot, key, (lo, hi))?);
                }
                values.sort_by_key(|run| run.first_hour);
                values.dedup();
            }
        }
        let mut points = reached_hours.clone();
        if dense {
            points.insert(from.floor() as i32);
            points.extend(
                fields
                    .iter()
                    .flatten()
                    .map(|run| run.first_hour)
                    .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to),
            );
            points.extend(
                bands[index]
                    .iter()
                    .map(|run| run.first_hour)
                    .filter(|hour| from <= f64::from(*hour) && f64::from(*hour) <= to),
            );
        }
        if product == "series-state" {
            bands[index].retain_mut(|band| {
                let last = fields[0]
                    .iter()
                    .filter(|run| {
                        let lo = run.first_hour.max(band.first_hour);
                        let hi = run.last_hour.min(band.last_hour);
                        lo <= hi && reached_hours.range(lo..=hi).next().is_some()
                    })
                    .map(|run| run.last_hour.min(band.last_hour))
                    .max();
                if let Some(last) = last {
                    band.last_hour = last;
                    true
                } else {
                    false
                }
            });
        } else {
            for band in &mut bands[index] {
                band.last_hour = band.last_hour.min(to.floor() as i32);
            }
        }
        for hour in points {
            if let Some(run) = point(&fields, &bands[index], hour) {
                runs.push(run);
            }
        }
        runs.sort_by_key(|run| run.first_hour);
        runs.dedup();
    }
    let mut summaries = BTreeMap::new();
    if product == "series-summary" {
        for (id, selected_spans) in &requested.within {
            let spans: Vec<_> = selected_spans
                .iter()
                .filter_map(|(tag, first, last, starts)| {
                    let index = indices
                        .get(tag.as_str())
                        .filter(|_| chosen_tags.contains(tag.as_str()))?;
                    Some((key(*index, RELATIVE), *first, *last, *starts))
                })
                .collect();
            summaries.insert(
                id.clone(),
                summary_spans(snapshot, &spans, bounds, (from, to, width))?,
            );
        }
    }
    let (mut reply, places) = scan.reply(source, &chosen, context)?;
    reply
        .subjects
        .retain(|subject| chosen_tags.contains(subject.tag.as_str()));
    let mut value = serde_json::to_value(reply)?;
    value["bands"] = serde_json::to_value(
        scan.subjects
            .iter()
            .zip(bands)
            .filter(|((tag, _), _)| chosen_tags.contains(tag.as_str()))
            .map(|((tag, _), rows)| {
                serde_json::json!({
                    "tag": tag,
                    "runs": rows
                        .into_iter()
                        .map(|run| serde_json::json!({
                            "firstHour": run.first_hour, "lastHour": run.last_hour,
                            "startsSegment": run.starts_segment, "overlord": run.overlord,
                            "level": places[run.level as usize], "lowest": run.lowest,
                            "highest": run.highest,
                        }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    )?;
    value["summaries"] = serde_json::to_value(summaries)?;
    value["exactHours"] = serde_json::to_value(hours)?;
    Ok(value)
}

/// The charts of the RELATIVE keys of `spans` (key, first and last hour, and whether the span
/// starts a segment), each clipped to its span, merged: a span's first hour starts a segment
/// where it says so.
fn summary_spans<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    spans: &[(Option<u64>, i32, i32, bool)],
    bounds: (i32, i32),
    view: (f64, f64, u32),
) -> io::Result<ChartSummary> {
    series::viewport(Some(view.0), Some(view.1), Some(view.2))?;
    if spans.iter().any(|span| span.1 > span.2)
        || spans.windows(2).any(|pair| pair[0].2 >= pair[1].1)
    {
        return Err(invalid("Invalid dated series spans"));
    }
    let mut result: Option<ChartSummary> = None;
    for &(key, first, last, starts) in spans {
        if f64::from(last) < view.0.floor() || f64::from(first) > view.1.floor() {
            continue;
        }
        let within = [HourSpan {
            first_hour: first,
            last_hour: last,
        }];
        let mut part = match key {
            Some(key) => engine::summary(snapshot, key, bounds, view, 0, Some(&within))?,
            None => {
                let (exact, bins) = series::summary_bins(bounds, view)?;
                ChartSummary {
                    from: view.0,
                    to: view.1,
                    width: view.2,
                    buckets: vec![None; bins.len()],
                    runs: exact.then(Vec::new),
                    approximate: Vec::new(),
                }
            }
        };
        for bucket in part.buckets.iter_mut().flatten() {
            if bucket.first_hour == first {
                bucket.first_break = starts;
            }
        }
        for run in part.runs.iter_mut().flatten() {
            if run.first_hour == first {
                run.break_before = starts;
            }
        }
        if let Some(result) = &mut result {
            for (bucket, next) in result.buckets.iter_mut().zip(part.buckets) {
                *bucket = match (*bucket, next) {
                    (Some(_), Some(_)) => merge(*bucket, next),
                    (previous, next) => previous.or(next),
                };
            }
            if let (Some(runs), Some(next)) = (&mut result.runs, part.runs) {
                runs.extend(next);
            }
        } else {
            result = Some(part);
        }
    }
    Ok(result.unwrap_or(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets: Vec::new(),
        runs: Some(Vec::new()),
        approximate: Vec::new(),
    }))
}

/// The subject's values at `hour` within its band there, the run clipped to where every value
/// and the band hold.
fn point(fields: &[Vec<series::Run>; 4], bands: &[AutonomyRun], hour: i32) -> Option<AutonomyRun> {
    let mut run = bands
        .partition_point(|run| run.first_hour <= hour)
        .checked_sub(1)
        .and_then(|index| bands.get(index))
        .filter(|run| hour <= run.last_hour)?
        .clone();
    let mut values = Vec::new();
    for rows in fields.iter().take(3 + usize::from(run.ratio_raw.is_some())) {
        values.push(
            rows.partition_point(|run| run.first_hour <= hour)
                .checked_sub(1)
                .and_then(|at| rows.get(at))
                .filter(|run| hour <= run.last_hour)?,
        );
    }
    let first = values
        .iter()
        .fold(run.first_hour, |first, value| first.max(value.first_hour));
    run.starts_segment &= first == run.first_hour;
    run.first_hour = first;
    run.last_hour = values
        .iter()
        .fold(run.last_hour, |last, value| last.min(value.last_hour));
    run.score_raw = values[0].value.to_string();
    run.lower_raw = values[1].value.to_string();
    run.upper_raw = values[2].value.to_string();
    run.ratio_raw = values.get(3).map(|value| value.value.to_string());
    Some(run)
}
