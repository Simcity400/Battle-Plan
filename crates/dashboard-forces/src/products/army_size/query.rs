//! Army size's replies, read from the family's series publication ([`super::emit`]): the
//! manifest, a window's histories by country or by rank, their compositions, the charts over
//! a span, keyboard navigation, and the scoreboard's cells.

use super::{
    ArmyAttribution, ArmyMeasure, ArmyRun, ArmySeries, ArmySize, ArmySizeParams, ArmySizeReply,
    amount, channel,
    composition::{RawMixRun, composition_reply},
    emit::{
        AVAILABLE, COUNTS, COVERAGE, FAMILY, INVALID, MIX, MIX_KIND, NAMES, POSITIONS, RANKS,
        decimals,
    },
};
use dashboard_base::{
    prepared::{
        engine,
        series::{self, Navigation},
    },
    prepared_chart::ChartSummary,
    products::{
        Product,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{
            CountryIdentityRun, RecordedCountry, RecordedHistories, selected_aliases,
            selected_identities, selected_spans,
        },
    },
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// A total's run as a reply reads it.
#[derive(Clone, Copy, Debug)]
pub(super) struct RawRun {
    pub first: i32,
    pub last: i32,
    pub starts: bool,
    pub value: i128,
}

/// A country's runs, identity segments and compositions in a reply's window.
#[derive(Default)]
pub(super) struct Track {
    pub runs: Vec<RawRun>,
    pub segments: Vec<HourSpan>,
    pub mix: Vec<RawMixRun>,
}

pub(super) fn window_mix<'a>(track: &'a Track, range: &HourSpan) -> &'a [RawMixRun] {
    let first = track.mix.partition_point(|run| run.last < range.first_hour);
    let last = track
        .mix
        .partition_point(|run| run.first <= range.last_hour);
    &track.mix[first..last]
}

/// What the family's hours read, and what they could not resolve.
struct Read {
    divisions: bool,
    strength: bool,
    manpower: bool,
    battalions: bool,
    unreadable: u64,
    composition: u64,
    invalid: Vec<(String, u64)>,
}

impl Read {
    fn of(directory: &Directory) -> Self {
        let available = directory.scalar(AVAILABLE).map_or(&[][..], Scalar::list);
        let flag = |at: usize| matches!(available.get(at), Some(Scalar::Boolean(true)));
        let counts = directory.scalar(COUNTS).map_or(&[][..], Scalar::list);
        let count = |at: usize| {
            counts
                .get(at)
                .and_then(Scalar::integer)
                .and_then(|count| u64::try_from(count).ok())
                .unwrap_or(0)
        };
        Self {
            divisions: flag(0),
            strength: flag(1),
            manpower: flag(2),
            battalions: flag(3),
            unreadable: count(0),
            composition: count(1),
            invalid: reasons(directory, INVALID),
        }
    }
}

/// A family scalar of reasons and their counts.
pub(crate) fn reasons(directory: &Directory, id: u64) -> Vec<(String, u64)> {
    directory
        .scalar(id)
        .map_or(&[][..], Scalar::list)
        .iter()
        .filter_map(|pair| match pair.list() {
            [reason, count] => Some((
                String::from_utf8_lossy(reason.text()?).into_owned(),
                u64::try_from(count.integer()?).ok()?,
            )),
            _ => None,
        })
        .collect()
}

/// The country actor of `tag`, when the family ever named it.
fn actor(directory: &Directory, tag: &str) -> Option<u64> {
    directory.actor(tag.as_bytes())
}

/// The country actors, by tag.
fn countries(directory: &Directory) -> BTreeMap<String, u64> {
    directory
        .actors
        .iter()
        .enumerate()
        .filter(|(actor, _)| directory.key(*actor as u64, COVERAGE, None).is_some())
        .filter_map(|(actor, described)| {
            Some((
                std::str::from_utf8(&described.name).ok()?.to_owned(),
                actor as u64,
            ))
        })
        .collect()
}

/// `key`'s runs over `range`.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    range: HourSpan,
) -> io::Result<Vec<RawRun>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    Ok(
        engine::runs(snapshot, key, range.first_hour, range.last_hour)?
            .into_iter()
            .map(|run| RawRun {
                first: run.span.first,
                last: run.span.last,
                starts: run.break_before,
                value: run.value,
            })
            .collect(),
    )
}

/// The composition a [`MIX_KIND`] event holds.
fn mix_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    event: EventView,
) -> io::Result<[Option<BTreeMap<String, i64>>; 2]> {
    let EventView::Family(event) = event else {
        return Err(invalid("Missing prepared army composition"));
    };
    if event.kind != MIX_KIND || event.fields.len != 2 * Cell::SIZE as u64 {
        return Err(invalid("Prepared army composition is not valid"));
    }
    let mut mix: [Option<BTreeMap<String, i64>>; 2] = [None, None];
    for (side, parts) in mix.iter_mut().enumerate() {
        let Cell::List(pairs) = snapshot.field(event.fields, side as u64)? else {
            continue;
        };
        let mut held = BTreeMap::new();
        for at in 0..pairs.len / Cell::SIZE as u64 / 2 {
            let (Cell::Unsigned(name), Cell::Integer(count)) = (
                snapshot.field(pairs, at * 2)?,
                snapshot.field(pairs, at * 2 + 1)?,
            ) else {
                return Err(invalid("Prepared army composition is not valid"));
            };
            let name = directory
                .scalar(NAMES + name)
                .and_then(Scalar::text)
                .ok_or_else(|| invalid("Prepared army composition names an unknown sub-unit"))?;
            let count = i64::try_from(count)
                .map_err(|_| invalid("Prepared army composition is not valid"))?;
            held.insert(String::from_utf8_lossy(name).into_owned(), count);
        }
        *parts = Some(held);
    }
    Ok(mix)
}

/// The compositions of `actor` over `range`.
fn mixes<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    range: HourSpan,
) -> io::Result<Vec<RawMixRun>> {
    let Some(key) = directory.key(actor, MIX, None) else {
        return Ok(Vec::new());
    };
    let mut mixes = Vec::new();
    for run in runs(snapshot, Some(key), range)? {
        let event = snapshot
            .latest_event(
                Selection {
                    key,
                    membership: None,
                },
                run.first,
                None,
            )?
            .ok_or_else(|| invalid("Missing prepared army composition"))?;
        mixes.push(RawMixRun {
            first: run.first,
            last: run.last,
            parts: mix_of(snapshot, directory, event)?,
        });
    }
    Ok(mixes)
}

pub(super) fn reply_bound() -> io::Error {
    io::Error::other("Army size reply exceeds the transport reply budget; choose fewer countries")
}

fn for_each_reply_run(
    track: &Track,
    params: &ArmySizeParams,
    range: &HourSpan,
    spans: Option<&[HourSpan]>,
    cancelled: &AtomicBool,
    mut output: impl FnMut(ArmyRun) -> io::Result<()>,
) -> io::Result<()> {
    let mut pending: Option<ArmyRun> = None;
    let mut visit = |raw: &RawRun, first, last| -> io::Result<()> {
        let exact = raw.value;
        let starts_segment = (raw.starts && first == raw.first)
            || pending
                .as_ref()
                .is_none_or(|run| run.last_hour.checked_add(1) != Some(first));
        if let Some(previous) = pending.as_mut()
            && !starts_segment
            && previous.raw == exact.to_string()
        {
            previous.last_hour = last;
        } else {
            if let Some(previous) = pending.take() {
                output(previous)?;
            }
            pending = Some(ArmyRun {
                first_hour: first,
                last_hour: last,
                total: amount(Some(exact), params.measure),
                raw: exact.to_string(),
            });
        }
        Ok(())
    };
    let whole = [*range];
    dashboard_base::ranking::for_each_piece(
        &track.runs,
        spans.unwrap_or(&whole),
        |run| (run.first, run.last),
        cancelled,
        &mut visit,
    )?;
    if let Some(last) = pending {
        output(last)?;
    }
    Ok(())
}

fn window_segments(track: &Track, range: &HourSpan) -> Vec<HourSpan> {
    // Keep the original identity start for country looks; omit other windows.
    let first = track
        .segments
        .partition_point(|span| span.last_hour < range.first_hour);
    let last = track
        .segments
        .partition_point(|span| span.first_hour <= range.last_hour);
    track.segments[first..last]
        .iter()
        .map(|span| HourSpan {
            first_hour: span.first_hour,
            last_hour: span.last_hour.min(range.last_hour),
        })
        .collect()
}

fn window_identities(country: &RecordedCountry, range: &HourSpan) -> Vec<CountryIdentityRun> {
    let first = country
        .identities
        .partition_point(|identity| identity.last_hour < range.first_hour);
    let last = country
        .identities
        .partition_point(|identity| identity.first_hour <= range.last_hour);
    country.identities[first..last]
        .iter()
        .map(|identity| CountryIdentityRun {
            first_hour: identity.first_hour.max(range.first_hour),
            last_hour: identity.last_hour.min(range.last_hour),
            slot: identity.slot,
            country_id: identity.country_id,
        })
        .collect()
}

/// Spans sorted and joined where they touch.
fn joined(spans: &mut Vec<HourSpan>) {
    spans.sort_by_key(|span| span.first_hour);
    let mut merged = Vec::new();
    for span in std::mem::take(spans) {
        mark_unobserved(&mut merged, span.first_hour, span.last_hour);
    }
    *spans = merged;
}

/// The countries at ranks 0..`top` of `channel` over `range`, each with the hours it held one.
fn top_members<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    channel: usize,
    top: usize,
    range: HourSpan,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let positions = directory.dimensions(FAMILY, POSITIONS + channel as u64);
    let mut members: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
    for &(_, key) in positions.iter().take(top) {
        for run in runs(snapshot, Some(key), range)? {
            let actor = u64::try_from(run.value)
                .ok()
                .and_then(|actor| directory.actors.get(actor as usize))
                .ok_or_else(|| invalid("A prepared army rank names an unknown country"))?;
            let tag = String::from_utf8_lossy(&actor.name).into_owned();
            members.entry(tag).or_default().push(HourSpan {
                first_hour: run.first,
                last_hour: run.last,
            });
        }
    }
    for spans in members.values_mut() {
        joined(spans);
    }
    Ok(members)
}

/// Answers an `army-size` request without `templateAt` from the publication; `preparing`
/// while its coverage is partial.
#[allow(clippy::too_many_arguments)]
pub fn reply<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    params: &ArmySizeParams,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    preparing: bool,
    cancelled: &AtomicBool,
) -> io::Result<ArmySizeReply> {
    ArmySize::validate(params).map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|known| known.id == branch)
    {
        return Err(invalid("Unknown branch"));
    }
    let windows = source.block_windows(branch);
    let interval = params
        .window
        .map(|index| {
            windows
                .get(index)
                .cloned()
                .ok_or_else(|| invalid("Unknown army history window"))
        })
        .transpose()?;
    let read = Read::of(directory);
    let index = channel(params.measure, params.count_for);
    let mut warnings = Vec::new();
    warnings.extend(read.invalid.iter().map(|(reason, count)| {
        format!("{count} army table reads rejected ({reason}); affected values stay gaps.")
    }));
    if read.unreadable > 0 {
        warnings.push(format!("{} division/country attribution reads could not be resolved; affected totals remain unknown.", read.unreadable));
    }
    if read.composition > 0 && params.measure == ArmyMeasure::Battalions {
        warnings.push(format!(
                "{} country attribution-hours have unresolved division ownership or missing/invalid template composition; battalion totals stay unknown.",
                read.composition));
    }
    let (measures, unknown_measures): (Vec<_>, Vec<_>) = [
        ArmyMeasure::Manpower,
        ArmyMeasure::Divisions,
        ArmyMeasure::Strength,
        ArmyMeasure::Battalions,
    ]
    .into_iter()
    .partition(|measure| match measure {
        ArmyMeasure::Divisions => read.divisions,
        ArmyMeasure::Strength => read.strength,
        ArmyMeasure::Manpower => read.manpower,
        ArmyMeasure::Battalions => read.battalions,
    });
    let unknown_measures = if preparing {
        unknown_measures
    } else {
        Vec::new()
    };
    let mut tracks = BTreeMap::new();
    let mut membership = None;
    let mut selected = None;
    let mut projected = params.tags.clone();
    if params.top.is_none() && !params.tags.is_empty() {
        let identities = selected_identities(history, &params.tags);
        projected = selected_aliases(history, &params.tags);
        selected = interval.as_ref().map(|range| {
            projected
                .iter()
                .map(|tag| {
                    let runs = history.countries.get(tag).map_or_else(Vec::new, |country| {
                        window_identities(country, range)
                            .into_iter()
                            .filter(|identity| {
                                identities.contains(&(identity.slot, identity.country_id))
                            })
                            .collect()
                    });
                    (tag.clone(), runs)
                })
                .collect::<BTreeMap<_, _>>()
        });
    }
    if let Some(range) = &interval {
        let wanted = if let Some(top) = params.top {
            let members = top_members(snapshot, directory, index, top, *range)?;
            let wanted: Vec<String> = members.keys().cloned().collect();
            membership = Some(members);
            wanted
        } else {
            projected.clone()
        };
        for tag in wanted {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let Some(actor) = actor(directory, &tag) else {
                continue;
            };
            let segments = history
                .countries
                .get(&tag)
                .map_or_else(Vec::new, |country| {
                    country
                        .segments
                        .iter()
                        .filter(|span| {
                            span.first_hour <= range.last_hour && span.last_hour >= range.first_hour
                        })
                        .cloned()
                        .collect()
                });
            let mix = if params.by_type && params.measure == ArmyMeasure::Battalions {
                mixes(snapshot, directory, actor, *range)?
            } else {
                Vec::new()
            };
            tracks.insert(
                tag,
                Track {
                    runs: runs(snapshot, directory.key(actor, index as u64, None), *range)?,
                    segments,
                    mix,
                },
            );
        }
    }
    let (mut series, series_bytes) = match &interval {
        Some(range) => reply_series(
            &tracks,
            params,
            &projected,
            range,
            selected.as_ref(),
            membership,
            cancelled,
        )?,
        None => (Vec::new(), 2),
    };
    let types = if let Some(range) = interval.as_ref()
        && params.by_type
        && params.measure == ArmyMeasure::Battalions
    {
        composition_reply(
            source,
            context,
            &tracks,
            params,
            range,
            &mut series,
            &mut warnings,
            cancelled,
        )?
    } else {
        Vec::new()
    };
    let mut reply = ArmySizeReply {
        template_sections: None,
        branch_id: params.branch_id.clone(),
        windows: if params.window.is_none() {
            windows.to_vec()
        } else {
            Vec::new()
        },
        window: params.window,
        interval,
        available: match params.measure {
            ArmyMeasure::Divisions if params.count_for == ArmyAttribution::Host => read.strength,
            ArmyMeasure::Divisions => read.divisions,
            ArmyMeasure::Strength => read.strength,
            ArmyMeasure::Manpower => read.manpower,
            ArmyMeasure::Battalions => read.battalions,
        },
        measures,
        unknown_measures,
        types,
        series: Vec::new(),
        warnings,
    };
    let budget = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
    let envelope = dashboard_core::transport::json_len(&reply, budget).ok_or_else(reply_bound)?;
    let bytes = if series
        .iter()
        .any(|series| series.mix.is_some() || series.identities.is_some())
    {
        dashboard_core::transport::json_len(&series, budget).ok_or_else(reply_bound)?
    } else {
        series_bytes
    };
    if envelope + bytes - 2 > budget {
        return Err(reply_bound());
    }
    reply.series = series;
    Ok(reply)
}

fn reply_series(
    tracks: &BTreeMap<String, Track>,
    params: &ArmySizeParams,
    tags: &[String],
    range: &HourSpan,
    selected: Option<&BTreeMap<String, Vec<CountryIdentityRun>>>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ArmySeries>, usize)> {
    let budget = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
    let mut seen = BTreeSet::new();
    let wanted: Vec<(String, Option<Vec<HourSpan>>)> = match &membership {
        Some(members) => members
            .iter()
            .map(|(tag, spans)| (tag.clone(), Some(spans.clone())))
            .collect(),
        None => tags
            .iter()
            .filter(|tag| seen.insert(*tag))
            .map(|tag| (tag.clone(), None))
            .collect(),
    };
    let wanted: Vec<_> = wanted
        .into_iter()
        .filter_map(|(tag, membership)| {
            let track = tracks.get(&tag)?;
            let identities = selected.map(|countries| countries[&tag].clone());
            let spans = identities.as_ref().map(|identities| {
                identities
                    .iter()
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect::<Vec<_>>()
            });
            let mut segments = window_segments(track, range);
            if let Some(spans) = &spans {
                segments.retain(|segment| {
                    let first = spans.partition_point(|span| span.last_hour < segment.first_hour);
                    spans
                        .get(first)
                        .is_some_and(|span| span.first_hour <= segment.last_hour)
                });
            }
            Some((
                ArmySeries {
                    tag,
                    identities,
                    runs: Vec::new(),
                    segments,
                    membership,
                    mix: None,
                },
                track,
                spans,
            ))
        })
        .collect();
    // Count the selected serialized series before copying its history.
    let mut bytes = 2usize;
    let mut series_comma = 0;
    for (series, track, spans) in &wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        bytes += series_comma
            + dashboard_core::transport::json_len(series, budget).ok_or_else(reply_bound)?;
        series_comma = 1;
        if bytes > budget {
            return Err(reply_bound());
        }
        let mut run_comma = 0;
        for_each_reply_run(
            track,
            params,
            range,
            spans.as_deref().or(series.membership.as_deref()),
            cancelled,
            |run| {
                bytes += run_comma
                    + dashboard_core::transport::json_len(&run, budget).ok_or_else(reply_bound)?;
                run_comma = 1;
                if bytes > budget {
                    return Err(reply_bound());
                }
                Ok(())
            },
        )?;
    }
    let series = wanted
        .into_iter()
        .map(|(mut series, track, spans)| {
            for_each_reply_run(
                track,
                params,
                range,
                spans.as_deref().or(series.membership.as_deref()),
                cancelled,
                |run| {
                    series.runs.push(run);
                    Ok(())
                },
            )?;
            Ok(series)
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((series, bytes))
}

/// The selection a series request names.
fn selection(params: &Value) -> io::Result<ArmySizeParams> {
    let selection: ArmySizeParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"], "top": params["family"]["top"],
        "measure": params["family"]["measure"], "countFor": params["family"]["countFor"], "byType": false,
    }))?;
    ArmySize::validate(&selection).map_err(dashboard_core::invalid)?;
    Ok(selection)
}

/// The chart of each selected or ranked country over a span, within `bounds`, the branch's
/// first and last hours.
pub fn summary<P: PageRead>(
    params: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    bounds: (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let selection = selection(params)?;
    let (from, to, width) = series::viewport(
        params["from"].as_f64(),
        params["to"].as_f64(),
        params["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let index = channel(selection.measure, selection.count_for);
    let countries = countries(directory);
    let view = HourSpan {
        first_hour: (from.floor() as i32).max(bounds.0),
        last_hour: (to.floor() as i32).min(bounds.1),
    };
    let mut membership = None;
    if let Some(top) = selection.top {
        let dense = to.floor() - from.floor() + 1.0 > f64::from(width);
        // Exact screens read bounded rank runs; dense screens read each country's rank, exact
        // runs only where it crosses the cut. Either way a line holds only its ranked hours.
        let members = if view.first_hour > view.last_hour {
            BTreeMap::new()
        } else if dense {
            let mut members = BTreeMap::new();
            let span = Span::new(view.first_hour, view.last_hour)?;
            for (tag, &actor) in &countries {
                let Some(key) = directory.key(actor, RANKS + index as u64, None) else {
                    continue;
                };
                let mut spans = Vec::new();
                snapshot.top_spans(
                    Selection {
                        key,
                        membership: None,
                    },
                    top as u64,
                    span,
                    |span| {
                        spans.push(HourSpan {
                            first_hour: span.first,
                            last_hour: span.last,
                        });
                        Ok(())
                    },
                )?;
                if !spans.is_empty() {
                    joined(&mut spans);
                    members.insert(tag.clone(), spans);
                }
            }
            members
        } else {
            top_members(snapshot, directory, index, top, view)?
        };
        membership = Some(members);
    }
    let mut wanted: Vec<String> = if selection.top.is_some() {
        countries.keys().cloned().collect()
    } else {
        selected_aliases(history, &selection.tags)
    };
    wanted.sort();
    wanted.dedup();
    let selected = selected_identities(history, &selection.tags);
    let mut lines = Vec::new();
    for tag in wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let identity_spans = history
            .countries
            .get(&tag)
            .into_iter()
            .flat_map(|country| country.identities.iter())
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .map(|identity| HourSpan {
                first_hour: identity.first_hour,
                last_hour: identity.last_hour,
            })
            .collect::<Vec<_>>();
        let within = match &membership {
            None => identity_spans.as_slice(),
            Some(members) => members.get(&tag).map_or(&[][..], Vec::as_slice),
        };
        if selection.top.is_some() && within.is_empty() {
            continue;
        }
        let key = countries
            .get(&tag)
            .and_then(|&actor| directory.key(actor, index as u64, None));
        let summary = match key {
            Some(key) => engine::summary(
                snapshot,
                key,
                bounds,
                (from, to, width),
                decimals(index),
                Some(within),
            )?,
            None => ChartSummary {
                from,
                to,
                width,
                buckets: Vec::new(),
                runs: Some(Vec::new()),
                approximate: Vec::new(),
            },
        };
        if selection.top.is_some() && summary.buckets.iter().all(Option::is_none) {
            continue;
        }
        series::summary_budget(from, to, width, lines.len() + 1)?;
        lines.push(json!({
            "tag": tag, "summary": summary,
            "identities": history.countries.get(&tag).map(|country| &country.identities),
        }));
    }
    Ok(json!({ "series": lines }))
}

/// The next hour a selected or ranked country's total, or a rank's holder, changes.
pub fn next_change<P: PageRead>(
    params: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let selection = selection(params)?;
    let navigation: Navigation = serde_json::from_value(params.clone())?;
    let origin = navigation.origin()?;
    let index = channel(selection.measure, selection.count_for);
    let mut keys: Vec<(u64, Option<Vec<HourSpan>>)> = Vec::new();
    if let Some(top) = selection.top {
        let hour = origin
            .clamp(navigation.first_hour, navigation.last_hour)
            .floor() as i32;
        let positions = directory.dimensions(FAMILY, POSITIONS + index as u64);
        for &(_, key) in positions.iter().take(top) {
            keys.push((key, None));
            if let Some((run, _)) = engine::run_at(snapshot, key, hour)?
                && let Some(value) = u64::try_from(run.value)
                    .ok()
                    .and_then(|actor| directory.key(actor, index as u64, None))
            {
                keys.push((value, None));
            }
        }
    } else {
        let within: BTreeMap<String, Vec<HourSpan>> =
            serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
        for (tag, spans) in selected_spans(history, &selection.tags, &within, cancelled)? {
            if let Some(key) =
                actor(directory, &tag).and_then(|actor| directory.key(actor, index as u64, None))
            {
                keys.push((key, Some(spans)));
            }
        }
    }
    let keys: Vec<(u64, Option<&[HourSpan]>)> = keys
        .iter()
        .map(|(key, spans)| (*key, spans.as_deref()))
        .collect();
    Ok(json!({ "hour": engine::navigate(snapshot, &keys, &navigation)? }))
}

/// Each tag's owner manpower at `hour`, and whether its army was known then; None where it
/// held no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    hour: i32,
) -> io::Result<Vec<(Option<i128>, bool)>> {
    cells_by(directory, tags, &mut |key| {
        Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run.value))
    })
}

/// [`cells`], each key's value at the hour given by `read`.
pub fn cells_by(
    directory: &Directory,
    tags: &[&str],
    read: &mut dyn FnMut(u64) -> io::Result<Option<i128>>,
) -> io::Result<Vec<(Option<i128>, bool)>> {
    let index = channel(ArmyMeasure::Manpower, ArmyAttribution::Owner) as u64;
    tags.iter()
        .map(|tag| {
            let Some(actor) = actor(directory, tag) else {
                return Ok((None, false));
            };
            let value = match directory.key(actor, index, None) {
                Some(key) => read(key)?,
                None => None,
            };
            let known = match directory.key(actor, COVERAGE, None) {
                Some(key) => read(key)?.is_some(),
                None => false,
            };
            Ok((value, known))
        })
        .collect()
}
