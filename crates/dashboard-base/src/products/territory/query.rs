//! Territory's replies, read from the family's series publication ([`super::emit`]): the chart
//! summaries of the selected or ranked tags and of a group's sum, their composite runs at an
//! hour with the world's and the regional breakdown, keyboard navigation, the scoreboard's
//! owned-state cells and the faction totals' measures. The identity history is read only from
//! its preparation, through the coverage the two share; nothing reads the recording.

use super::{
    AreaData, Scan, TerritoryMeasure, TerritoryParams, TerritoryReply, TerritoryRule, TerritoryRun,
    Track, area_data, columns,
    emit::{
        COUNTS, FACTS, FAMILY, META, Meta, OBSERVED, POSITIONS, PRESENT, RANK, REGION_KEYS,
        REGIONAL, REGIONS, ROSTERS, RegionalMeta, VALUE, VALUE_CHANNELS, WORLD, kind,
    },
    scan_warning, selected_series,
};
use crate::{
    Source,
    metadata::check_cancel,
    prepared::{
        coverage::Coverage,
        engine, group_sum,
        series::{self, Navigation},
        series_read::{Published, SeriesRead},
    },
    prepared_chart::ChartSummary,
    products::{
        LoadContext,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{RecordedHistories, selected_aliases, selected_identities},
        territory_breakdown::{
            TerritoryBreakdownReply, TerritoryRegionRun, TerritoryRegionSeries, TerritoryStateShare,
        },
        validate_tag,
    },
};
use dashboard_series::{
    Cell, KeyHandle, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use installed_content_province_area::province_area::ProvinceAreas;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, io, sync::atomic::AtomicBool};

fn invalid(message: &'static str) -> io::Error {
    crate::invalid(message)
}

/// Every tag the family ever named, with its actor.
pub fn actors(directory: &Directory) -> BTreeMap<String, u64> {
    directory
        .actors
        .iter()
        .enumerate()
        .filter_map(|(actor, held)| {
            Some((String::from_utf8(held.name.to_vec()).ok()?, actor as u64))
        })
        .collect()
}

/// A JSON text scalar.
fn text<T: serde::de::DeserializeOwned + Default>(directory: &Directory, id: u64) -> io::Result<T> {
    match directory.scalar(id).and_then(Scalar::text) {
        Some(text) => Ok(serde_json::from_slice(text)?),
        None => Ok(T::default()),
    }
}

/// The ambiguous slots, the totals past the exact range, the incomplete reads, and the states
/// and provinces no region attributes.
fn counts(directory: &Directory) -> [u64; 5] {
    let counts = directory.scalar(COUNTS).map_or(&[][..], Scalar::list);
    std::array::from_fn(|at| {
        counts
            .get(at)
            .and_then(Scalar::integer)
            .and_then(|count| u64::try_from(count).ok())
            .unwrap_or(0)
    })
}

/// The publication's area unit, its refusal and its warnings, as a reply carries them.
pub fn metadata(directory: &Directory, branch: u64) -> io::Result<TerritoryReply> {
    let meta: Meta = text(directory, META)?;
    let [ambiguous, overflowed, incomplete, _, _] = counts(directory);
    let scan = Scan {
        ambiguous_slots: ambiguous as usize,
        overflowed: overflowed as usize,
        incomplete: incomplete as usize,
        ..Scan::default()
    };
    let mut warnings = meta.warnings;
    scan_warning(&scan, &mut warnings);
    Ok(TerritoryReply {
        branch_id: branch.to_string(),
        world_totals: Vec::new(),
        series: Vec::new(),
        area_unit: meta.area_unit,
        area_reason: meta.area_reason,
        warnings,
    })
}

/// The family's keys.
struct Keys<'d> {
    directory: &'d Directory,
}

impl Keys<'_> {
    fn value(&self, actor: u64, channel: usize) -> Option<KeyHandle> {
        self.directory.key(actor, VALUE, Some(channel as u64))
    }
    fn rank(&self, actor: u64, column: usize) -> Option<KeyHandle> {
        self.directory.key(actor, RANK, Some(column as u64))
    }
    fn position(&self, column: usize, position: u64) -> Option<KeyHandle> {
        self.directory
            .key(FAMILY, POSITIONS + column as u64, Some(position))
    }
    fn facts(&self, actor: u64) -> Option<KeyHandle> {
        self.directory.key(actor, FACTS, None)
    }
    fn present(&self, actor: u64) -> Option<KeyHandle> {
        self.directory.key(actor, PRESENT, None)
    }
    fn family(&self, measure: u64, dimension: Option<u64>) -> Option<KeyHandle> {
        self.directory.key(FAMILY, measure, dimension)
    }
}

/// The run of `key` holding `hour`, whole.
fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(match key {
        Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run),
        None => None,
    })
}

/// `key`'s runs over `from..=to`, clipped to it.
fn clipped<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    from: i32,
    to: i32,
) -> io::Result<Vec<dashboard_series::Run>> {
    match key {
        Some(key) => engine::runs(snapshot, key, from, to),
        None => Ok(Vec::new()),
    }
}

/// The spans of `key`'s runs over `from..=to`.
fn spans<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    from: i32,
    to: i32,
) -> io::Result<Vec<HourSpan>> {
    Ok(clipped(snapshot, key, from, to)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.span.first,
            last_hour: run.span.last,
        })
        .collect())
}

/// The latest event of `key` by `hour`, its fields.
pub(super) fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: KeyHandle,
    hour: i32,
    expected: u64,
) -> io::Result<dashboard_series::Blob> {
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            hour,
            None,
        )?
        .ok_or_else(|| invalid("Territory facts are missing"))?;
    let EventView::Family(event) = event else {
        return Err(invalid("Territory facts are missing"));
    };
    if event.kind != expected {
        return Err(invalid("Territory facts are not valid"));
    }
    Ok(event.fields)
}

fn integer(cell: Cell) -> io::Result<Option<i64>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Integer(value) => i64::try_from(value)
            .map(Some)
            .map_err(|_| invalid("Territory facts are not valid")),
        _ => Err(invalid("Territory facts are not valid")),
    }
}

/// A composite run's values and segment start, as its event holds them.
fn run_values<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    fields: dashboard_series::Blob,
) -> io::Result<([Option<i64>; columns::COUNT], bool)> {
    if fields.len != (columns::COUNT as u64 + 1) * Cell::SIZE as u64 {
        return Err(invalid("Territory facts are not valid"));
    }
    let mut values = [None; columns::COUNT];
    for (at, value) in values.iter_mut().enumerate() {
        *value = integer(snapshot.field(fields, at as u64)?)?;
    }
    let Cell::Boolean(starts) = snapshot.field(fields, columns::COUNT as u64)? else {
        return Err(invalid("Territory facts are not valid"));
    };
    Ok((values, starts))
}

/// The composite runs of `key` over `from..=to`, each with its values: whole at one hour,
/// clipped to the window else, a clipped run starting no segment.
pub(super) fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    from: i32,
    to: i32,
) -> io::Result<Vec<TerritoryRun>> {
    let Some(handle) = key else {
        return Ok(Vec::new());
    };
    let markers = if from == to {
        run_at(snapshot, key, from)?.into_iter().collect()
    } else {
        clipped(snapshot, key, from, to)?
    };
    markers
        .into_iter()
        .map(|marker| {
            let fields = latest(snapshot, handle, marker.span.first, kind::RUN)?;
            let (values, starts) = run_values(snapshot, fields)?;
            Ok(TerritoryRun {
                first_hour: marker.span.first,
                last_hour: marker.span.last,
                starts_segment: starts && i128::from(marker.span.first) == marker.value,
                values,
            })
        })
        .collect()
}

/// A chart of `key` over `view`, its bins laid from `bounds`, within `within`; every bin unknown
/// where the family never named the key.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    within: Option<&[HourSpan]>,
) -> io::Result<ChartSummary> {
    if let Some(key) = key.filter(|_| bounds.0 <= bounds.1) {
        return engine::summary(snapshot, key, bounds, view, 0, within);
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

/// The hours `key`'s rank is within `top` over `bounds`.
fn top_spans<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    bounds: (i32, i32),
    top: u64,
) -> io::Result<Vec<HourSpan>> {
    let mut spans = Vec::new();
    let Some(key) = key.filter(|_| bounds.0 <= bounds.1) else {
        return Ok(spans);
    };
    snapshot.top_spans(
        Selection {
            key,
            membership: None,
        },
        top,
        Span::new(bounds.0, bounds.1)?,
        |span| {
            mark_unobserved(&mut spans, span.first, span.last);
            Ok(())
        },
    )?;
    Ok(spans)
}

/// The hours of `spans` inside `within`.
fn pieces(
    spans: &[HourSpan],
    within: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<HourSpan>> {
    let mut pieces = Vec::new();
    crate::ranking::for_each_piece(
        spans,
        within,
        |span| (span.first_hour, span.last_hour),
        cancelled,
        |_, first_hour, last_hour| {
            pieces.push(HourSpan {
                first_hour,
                last_hour,
            });
            Ok(())
        },
    )?;
    Ok(pieces)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    #[serde(default)]
    unobserved: Vec<HourSpan>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    measure: Option<TerritoryMeasure>,
    rule: Option<TerritoryRule>,
    top: Option<u64>,
    #[serde(default)]
    metadata: bool,
    #[serde(default)]
    breakdown: bool,
    #[serde(default)]
    aggregate: bool,
}

#[derive(Deserialize)]
struct Read {
    hour: i32,
}

/// A request's validated parameters.
fn params(value: &Value) -> io::Result<(Params, u64)> {
    let params: Params = serde_json::from_value(value.clone())?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid territory branch"))?;
    for tag in &params.tags {
        validate_tag(tag).map_err(crate::invalid)?;
    }
    if params.family.top == Some(0) || params.family.top.is_some() && !params.tags.is_empty() {
        return Err(invalid("Invalid top countries"));
    }
    if params.family.breakdown && matches!(params.family.rule, Some(TerritoryRule::Occupied)) {
        return Err(invalid("Occupied territory has no regional breakdown"));
    }
    Ok((params, branch))
}

/// Whether a request asks only for the family's metadata.
pub fn metadata_only(value: &Value) -> bool {
    value["family"]["metadata"].as_bool() == Some(true)
}

/// The metadata reply before the family publishes the branch: its area unit or refusal and
/// the content's warnings, from the same read the family prepares with.
pub fn unpublished_metadata(
    source: &Source,
    context: &LoadContext,
    value: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let (params, _) = params(value)?;
    let AreaData {
        areas,
        reason,
        warnings,
    } = area_data(source, context, cancelled)?;
    Ok(serde_json::to_value(TerritoryReply {
        branch_id: params.branch_id,
        world_totals: Vec::new(),
        series: Vec::new(),
        area_unit: areas.map(|_| ProvinceAreas::UNIT.to_owned()),
        area_reason: reason,
        warnings,
    })?)
}

/// A series request of the `territory` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot`, the publication pinned through `accepted`,
/// the coverage it shares with the identity history.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    product: &str,
    value: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    accepted: &Coverage,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let (params, branch) = params(&value)?;
    let keys = Keys { directory };
    let tags = actors(directory);
    let metadata = metadata(directory, branch)?;
    let warnings = metadata.warnings.clone();
    if params.family.metadata {
        return Ok(serde_json::to_value(TerritoryReply {
            branch_id: params.branch_id,
            ..metadata
        })?);
    }
    // Nothing past the coverage shared with the identities is read.
    let reach =
        (!accepted.complete).then(|| accepted.last_hour.min(snapshot.publication().accepted.last));
    let through = |last: i32| reach.map_or(last, |reach| last.min(reach));
    let measure = params
        .family
        .measure
        .ok_or_else(|| invalid("Missing territory measure"))?;
    let rule = params
        .family
        .rule
        .ok_or_else(|| invalid("Missing territory rule"))?;
    let column = measure.index(rule);
    let channel = VALUE_CHANNELS[column];
    let inverse: BTreeMap<u64, &String> = tags.iter().map(|(tag, &actor)| (actor, tag)).collect();
    let selected: Vec<String> = if params.family.top.is_some() || params.tags.is_empty() {
        tags.keys().cloned().collect()
    } else {
        selected_aliases(history, &params.tags)
    };
    let selected_ids = (!params.tags.is_empty() && params.family.top.is_none())
        .then(|| selected_identities(history, &params.tags));
    let mut holders = BTreeMap::new();
    let mut excluded = BTreeMap::new();
    for tag in &selected {
        if let Some(ids) = &selected_ids
            && !ids.is_empty()
        {
            let country = history.countries.get(tag);
            let mut included = Vec::new();
            let mut omitted = Vec::new();
            for identity in country
                .into_iter()
                .flat_map(|country| country.identities.iter())
            {
                let span = HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                };
                if ids.contains(&(identity.slot, identity.country_id)) {
                    included.push(span);
                } else {
                    omitted.push(span);
                }
            }
            let spans = if params.within.is_empty() {
                included
            } else {
                pieces(
                    &included,
                    params.within.get(tag).map_or(&[], Vec::as_slice),
                    cancelled,
                )?
            };
            holders.insert(tag.clone(), spans);
            excluded.insert(tag.clone(), omitted);
        } else if let Some(spans) = params.within.get(tag) {
            holders.insert(tag.clone(), spans.clone());
        }
    }
    let value_key = |actor: u64| keys.value(actor, channel);
    match product {
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            if params.family.aggregate {
                let view = (from, to, width);
                let group = aggregate(
                    snapshot,
                    &keys,
                    column,
                    &tags,
                    &selected,
                    &params.within,
                    &params.unobserved,
                    params.family.top,
                    view,
                    &excluded,
                    cancelled,
                )?;
                let end = group
                    .buckets
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .map(|bucket| bucket.last_hour);
                let sum = match end {
                    Some(end) => aggregate(
                        snapshot,
                        &keys,
                        column,
                        &tags,
                        &selected,
                        &params.within,
                        &params.unobserved,
                        params.family.top,
                        (f64::from(end), f64::from(end), 1),
                        &excluded,
                        cancelled,
                    )?
                    .runs
                    .unwrap_or_default(),
                    None => Vec::new(),
                };
                return Ok(
                    json!({"branchId":params.branch_id,"measure":measure,"decimals":0,"series":[{"tag":"group","summary":group,"marks":null}],"sum":sum,"warnings":warnings}),
                );
            }
            let bounds = (from.floor() as i32, through(to.ceil() as i32));
            series::summary_budget(from, to, width, 1)?;
            let budget = crate::products::MAX_PRODUCT_REPLY_BYTES;
            series::paged(params.cursor.as_deref(), budget, |page| {
                let mut lines = Vec::new();
                for at in page.lines(selected.len()) {
                    let tag = &selected[at];
                    check_cancel(cancelled)?;
                    let Some(&actor) = tags.get(tag) else {
                        continue;
                    };
                    // A ranked line holds only the hours its country ranks within the top, of
                    // those asked for: it ends where the country leaves the selection.
                    let ranked = match params.family.top {
                        Some(top) => {
                            let ranked = top_spans(snapshot, keys.rank(actor, column), bounds, top)?;
                            let ranked = match holders.get(tag) {
                                Some(asked) => pieces(&ranked, asked, cancelled)?,
                                None => ranked,
                            };
                            if ranked.is_empty() {
                                continue;
                            }
                            Some(ranked)
                        }
                        None => None,
                    };
                    let within = ranked
                        .as_deref()
                        .or_else(|| holders.get(tag).map(Vec::as_slice));
                    let line = summary(
                        snapshot,
                        value_key(actor),
                        bounds,
                        (from, to, width),
                        within,
                    )?;
                    let end_runs = match line.buckets.iter().rev().flatten().next() {
                        Some(bucket) => runs(
                            snapshot,
                            keys.facts(actor),
                            bucket.last_hour,
                            bucket.last_hour,
                        )?,
                        None => Vec::new(),
                    };
                    let country = history.countries.get(tag);
                    let identities = country.map(|country| {
                        country
                            .identities
                            .iter()
                            .filter(|identity| {
                                selected_ids.as_ref().is_none_or(|ids| {
                                    ids.contains(&(identity.slot, identity.country_id))
                                })
                            })
                            .collect::<Vec<_>>()
                    });
                    let line = json!({"tag": tag, "summary": line, "marks": null,"endRuns":end_runs,
                        "segments":holders.get(tag).or_else(||country.map(|country|country.segments.as_ref())),
                        "identities":identities});
                    if !page.admit(at, &line)? {
                        break;
                    }
                    lines.push(line);
                }
                Ok(json!({"branchId":params.branch_id,"measure":measure,"decimals":0,"series": lines, "warnings": warnings}))
            })
        }
        "series-next-change" => {
            let mut navigation: Navigation = serde_json::from_value(value)?;
            if let Some(reach) = reach.map(f64::from) {
                if reach < navigation.first_hour {
                    return Ok(json!({ "hour": null }));
                }
                navigation.last_hour = navigation.last_hour.min(reach);
            }
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut found: Vec<(KeyHandle, Option<Vec<HourSpan>>)> = Vec::new();
            let mut ranked = Vec::new();
            if let Some(top) = params.family.top {
                for rank in 0..top.min(tags.len() as u64) {
                    let position = keys.position(column, rank);
                    ranked.extend(position);
                    if let Some(run) = run_at(snapshot, position, hour)?
                        && let Some(key) = u64::try_from(run.value).ok().and_then(value_key)
                    {
                        found.push((key, None));
                    }
                }
            } else {
                for tag in &selected {
                    if let Some(key) = tags.get(tag).and_then(|&actor| value_key(actor)) {
                        found.push((key, holders.get(tag).cloned()));
                    }
                }
            }
            let keys: Vec<(KeyHandle, Option<&[HourSpan]>)> = found
                .iter()
                .map(|(key, spans)| (*key, spans.as_deref()))
                .chain(ranked.iter().map(|&key| (key, None)))
                .collect();
            Ok(json!({"hour": engine::navigate(snapshot, &keys, &navigation)?}))
        }
        "series-state" => {
            let [read] = params.reads.as_slice() else {
                return Err(invalid("Territory state needs one reached hour"));
            };
            let hour = read.hour;
            if reach.is_some_and(|reach| hour > reach) {
                return Ok(Value::Null);
            }
            let mut scan = Scan {
                world_totals: runs(snapshot, keys.family(WORLD, None), hour, hour)?,
                rosters: spans(snapshot, keys.family(ROSTERS, None), hour, hour)?,
                ..Scan::default()
            };
            let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
                params.family.top.map(|_| BTreeMap::new());
            if let (Some(members), Some(top)) = (&mut membership, params.family.top) {
                for rank in 0..top.min(tags.len() as u64) {
                    for run in clipped(snapshot, keys.position(column, rank), hour, hour)? {
                        let tag = u64::try_from(run.value)
                            .ok()
                            .and_then(|actor| inverse.get(&actor))
                            .ok_or_else(|| invalid("A territory rank names an unknown country"))?;
                        members.entry((*tag).clone()).or_default().push(HourSpan {
                            first_hour: run.span.first,
                            last_hour: run.span.last,
                        });
                    }
                }
                // Rank-position iteration is not chronological for a tag.
                for spans in members.values_mut() {
                    spans.sort_by_key(|span| span.first_hour);
                    let mut merged = Vec::new();
                    for span in std::mem::take(spans) {
                        mark_unobserved(&mut merged, span.first_hour, span.last_hour);
                    }
                    *spans = merged;
                }
            }
            let wanted = membership.as_ref().map_or_else(
                || selected.clone(),
                |members| members.keys().cloned().collect(),
            );
            for tag in wanted {
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                let segments = history
                    .countries
                    .get(&tag)
                    .map_or_else(Vec::new, |country| country.segments.as_ref().clone());
                scan.series.insert(
                    tag.clone().into(),
                    Track {
                        runs: runs(snapshot, keys.facts(actor), hour, hour)?,
                        segments,
                        present: spans(snapshot, keys.present(actor), hour, hour)?,
                        last: None,
                    },
                );
            }
            let selection = TerritoryParams {
                branch_id: params.branch_id.clone(),
                tags: if params.tags.is_empty() {
                    tags.keys().cloned().collect()
                } else {
                    params.tags.clone()
                },
                top: None,
            };
            let totals = TerritoryReply {
                series: selected_series(&scan, &selection, cancelled, Some(history), membership)?,
                world_totals: scan.world_totals,
                branch_id: params.branch_id,
                area_unit: metadata.area_unit,
                area_reason: metadata.area_reason,
                warnings,
            };
            if params.family.aggregate {
                let sum = aggregate(
                    snapshot,
                    &keys,
                    column,
                    &tags,
                    &selected,
                    &params.within,
                    &params.unobserved,
                    params.family.top,
                    (f64::from(hour), f64::from(hour), 1),
                    &excluded,
                    cancelled,
                )?;
                let mut value = serde_json::to_value(totals)?;
                value["sum"] = serde_json::to_value(sum.runs.unwrap_or_default())?;
                Ok(value)
            } else if params.family.breakdown {
                Ok(serde_json::to_value(breakdown(
                    snapshot, &keys, hour, totals,
                )?)?)
            } else {
                Ok(serde_json::to_value(totals)?)
            }
        }
        _ => Err(invalid("Unknown territory query")),
    }
}

/// The same window sum serves fixed picks and dated smart-group membership.
#[allow(clippy::too_many_arguments)]
fn aggregate<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &Keys<'_>,
    column: usize,
    tags: &BTreeMap<String, u64>,
    selected: &[String],
    within: &BTreeMap<String, Vec<HourSpan>>,
    unobserved: &[HourSpan],
    top: Option<u64>,
    view: (f64, f64, u32),
    excluded: &BTreeMap<String, Vec<HourSpan>>,
    cancelled: &AtomicBool,
) -> io::Result<ChartSummary> {
    let channel = VALUE_CHANNELS[column];
    let cell = RefCell::new(&mut *snapshot);
    let mut values = Published::new(&cell, |actor| keys.value(actor, channel));
    let mut present = Published::new(&cell, |actor| keys.present(actor));
    let mut rosters = Published::new(&cell, |_| keys.family(ROSTERS, None));
    let mut ranks = Published::new(&cell, |actor| keys.rank(actor, column));
    let bindings: Vec<Vec<_>> = selected
        .iter()
        .map(|tag| {
            tags.get(tag)
                .map(|&actor| (actor, i32::MIN, i32::MAX, None))
                .into_iter()
                .collect()
        })
        .collect();
    let members: Vec<_> = selected
        .iter()
        .zip(&bindings)
        .map(|(tag, keys)| group_sum::Member {
            keys,
            present_keys: keys,
            rank_key: tags.get(tag).copied(),
            within: within.get(tag).map(Vec::as_slice),
            excluded: excluded.get(tag).map_or(&[], Vec::as_slice),
            zeros: &[],
        })
        .collect();
    group_sum::summary(
        &mut values,
        &mut present,
        &mut rosters,
        None,
        top.map(|top| (&mut ranks as &mut dyn SeriesRead, top)),
        &members,
        unobserved,
        view,
        cancelled,
    )
}

/// A regional run's values, state shares and segment start, as its event holds them.
pub(super) fn region_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    fields: dashboard_series::Blob,
    marker: &dashboard_series::Run,
) -> io::Result<TerritoryRegionRun> {
    let malformed = || invalid("Regional territory facts are not valid");
    if fields.len != 9 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let mut values: [Option<String>; 6] = Default::default();
    for (at, value) in values.iter_mut().enumerate() {
        *value = integer(snapshot.field(fields, at as u64)?)?.map(|sum| sum.to_string());
    }
    let mut state_shares: [Option<Vec<TerritoryStateShare>>; 2] = Default::default();
    for (rule, shares) in state_shares.iter_mut().enumerate() {
        *shares = match snapshot.field(fields, 6 + rule as u64)? {
            Cell::Null => None,
            Cell::List(pairs) => {
                let mut terms = Vec::new();
                for at in 0..pairs.len / Cell::SIZE as u64 {
                    let Cell::List(pair) = snapshot.field(pairs, at)? else {
                        return Err(malformed());
                    };
                    let (Cell::Unsigned(denominator), Cell::Unsigned(numerator)) =
                        (snapshot.field(pair, 0)?, snapshot.field(pair, 1)?)
                    else {
                        return Err(malformed());
                    };
                    terms.push(TerritoryStateShare {
                        numerator: numerator.to_string(),
                        denominator: u32::try_from(denominator).map_err(|_| malformed())?,
                    });
                }
                Some(terms)
            }
            _ => return Err(malformed()),
        };
    }
    let Cell::Boolean(starts) = snapshot.field(fields, 8)? else {
        return Err(malformed());
    };
    Ok(TerritoryRegionRun {
        first_hour: marker.span.first,
        last_hour: marker.span.last,
        starts_segment: starts && i128::from(marker.span.first) == marker.value,
        values,
        state_shares,
    })
}

/// The regional breakdown at `hour`: every regional series' run there, the regional reads'
/// coverage, and `totals`' countries and world.
fn breakdown<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &Keys<'_>,
    hour: i32,
    totals: TerritoryReply,
) -> io::Result<TerritoryBreakdownReply> {
    let directory = keys.directory;
    let regional: RegionalMeta = text(directory, REGIONAL)?;
    if let Some(reason) = regional.reason {
        return Err(io::Error::other(reason));
    }
    let [_, _, _, missing_states, missing_provinces] = counts(directory);
    let mut warnings = regional.warnings;
    if missing_states != 0 {
        warnings.push(format!("{missing_states} recorded states have no resolved native province roster; their state contributions remain in Region not attributable"));
    }
    if missing_provinces != 0 {
        warnings.push(format!("{missing_provinces} recorded provinces have missing or conflicting strategic region membership; their contributions remain in Region not attributable"));
    }
    let mut series = Vec::new();
    let mut index = 0;
    while let Some(scalar) = directory.scalar(REGION_KEYS + index) {
        let text = scalar
            .text()
            .ok_or_else(|| invalid("A regional series key is not valid"))?;
        let (tag, region, missing, _): (Option<String>, Option<u32>, Option<String>, bool) =
            serde_json::from_slice(text)?;
        let key = keys.family(REGIONS, Some(index));
        let mut runs = Vec::new();
        if let (Some(handle), Some(marker)) = (key, run_at(snapshot, key, hour)?) {
            let fields = latest(snapshot, handle, marker.span.first, kind::REGION)?;
            runs.push(region_run(snapshot, fields, &marker)?);
        }
        series.push(TerritoryRegionSeries {
            tag,
            region,
            missing,
            runs,
        });
        index += 1;
    }
    series.sort_unstable_by(|a, b| {
        (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
    });
    let mut observed: [Vec<HourSpan>; 8] = Default::default();
    for (column, spans_of) in observed.iter_mut().enumerate() {
        *spans_of = spans(
            snapshot,
            keys.family(OBSERVED, Some(column as u64)),
            hour,
            hour,
        )?;
    }
    let reply = TerritoryBreakdownReply {
        branch_id: totals.branch_id,
        regions: regional.regions,
        series,
        countries: totals.series,
        world_totals: totals.world_totals,
        observed,
        area_unit: totals.area_unit,
        area_reason: totals.area_reason,
        warnings: warnings.into_iter().chain(totals.warnings).collect(),
    };
    crate::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Each tag's native owned-state count at `hour`; none where it held none.
pub fn owned_states<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    hour: i32,
) -> io::Result<Vec<Option<i128>>> {
    owned_states_by(directory, tags, &mut |key| {
        Ok(run_at(snapshot, Some(key), hour)?.map(|run| run.value))
    })
}

/// [`owned_states`], each key's value at the hour given by `read`.
pub fn owned_states_by(
    directory: &Directory,
    tags: &[&str],
    read: &mut dyn FnMut(u64) -> io::Result<Option<i128>>,
) -> io::Result<Vec<Option<i128>>> {
    let keys = Keys { directory };
    tags.iter()
        .map(|tag| {
            match directory
                .actor(tag.as_bytes())
                .and_then(|actor| keys.value(actor, super::emit::NATIVE_OWNED_STATES))
            {
                Some(key) => read(key),
                None => Ok(None),
            }
        })
        .collect()
}

/// The readers faction totals sum a territory measure from: its values, presence and rosters,
/// by tag actor.
pub struct Measures<'r, 's, P: PageRead> {
    pub values: Published<'r, 's, P>,
    pub present: Published<'r, 's, P>,
    pub rosters: Published<'r, 's, P>,
    /// Every tag the family ever named, with its actor.
    pub keys: BTreeMap<String, u64>,
}

/// Faction totals' readers of `measure` under `rule` from `cell`'s publication.
pub fn measures<'r, 's, P: PageRead>(
    cell: &'r RefCell<&'r mut Snapshot<'s, P>>,
    directory: &'r Directory,
    measure: Option<(TerritoryMeasure, TerritoryRule)>,
) -> Measures<'r, 's, P> {
    let channel = measure.map(|(measure, rule)| VALUE_CHANNELS[measure.index(rule)]);
    Measures {
        values: Published::new(cell, move |actor| {
            directory.key(actor, VALUE, Some(channel? as u64))
        }),
        present: Published::new(cell, move |actor| directory.key(actor, PRESENT, None)),
        rosters: Published::new(cell, move |_| directory.key(FAMILY, ROSTERS, None)),
        keys: actors(directory),
    }
}
