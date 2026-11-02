//! Navy size's replies, read from the family's series publication ([`super::emit`]): the chart
//! summaries of the selected or ranked tags with their runs and parts, their runs at hours,
//! keyboard navigation, and the scoreboard's cells.

use super::{
    ClassKey, NavyGrouping, NavyMeasure, NavyParams, NavySize, RawNavyRun, ScannedNavy,
    emit::{
        self, CLASSES, COUNTS, FAMILY, MAXIMUM, MIX, MIX_KIND, PART_KEYS, POSITIONS, UNDISPLAYABLE,
        VALUES, channel,
    },
    present,
};
use dashboard_base::{
    prepared::{engine, series},
    prepared_chart::ChartSummary,
    products::{
        Product,
        country_groups::{HourSpan, mark_unobserved},
        country_history::{self, RecordedHistories},
    },
    ranking::for_each_piece,
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot, codec::Binary, family::Directory, query::EventView,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// What the publication's census counted: its unread hours, by ownership its unattributed ship
/// observations and its country-hours of refused and overflowing hull HP, and each tag that is
/// not UTF-8 with its hours.
pub fn scanned(directory: &Directory) -> ScannedNavy {
    let count = |scalar: &dashboard_series::family::Scalar| {
        scalar
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0)
    };
    let counts = directory
        .scalar(COUNTS)
        .map_or(&[][..], |scalar| scalar.list());
    let at = |index: usize| counts.get(index).map_or(0, count);
    ScannedNavy {
        unread: at(0),
        unattributed: [at(1), at(2)],
        refused_strength: [at(3), at(4)],
        maximum_overflows: [at(5), at(6)],
        undisplayable_tags: directory
            .scalar(UNDISPLAYABLE)
            .map_or(&[][..], |scalar| scalar.list())
            .iter()
            .filter_map(|pair| match pair.list() {
                [tag, hours] => Some((tag.text()?.to_vec(), count(hours))),
                _ => None,
            })
            .collect(),
        ..ScannedNavy::default()
    }
}

/// Every class the publication's compositions name, by id.
pub(super) fn classes(directory: &Directory) -> io::Result<Arc<Vec<ClassKey>>> {
    let mut classes = Vec::new();
    while let Some(scalar) = directory.scalar(CLASSES + classes.len() as u64) {
        let text = scalar
            .text()
            .ok_or_else(|| invalid("A prepared navy class is not valid"))?;
        classes.push(
            serde_json::from_slice(text)
                .map_err(|_| invalid("A prepared navy class is not valid"))?,
        );
    }
    Ok(Arc::new(classes))
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
        .ok_or_else(|| invalid("A prepared navy rank names an unknown country"))
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

/// The run `run` of composition `key` as the row census holds it: nothing known for version 0,
/// else its event's total, maximum and classes.
pub(super) fn raw_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    run: &dashboard_series::Run,
    empty: &Arc<BTreeMap<u32, i64>>,
) -> io::Result<RawNavyRun> {
    let mut raw = RawNavyRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        starts_segment: run.break_before,
        total: None,
        classes: Arc::clone(empty),
        reference_total: None,
    };
    if run.value == 0 {
        return Ok(raw);
    }
    let malformed = || invalid("Prepared navy composition is not valid");
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            run.span.first,
            None,
        )?
        .ok_or_else(|| invalid("Missing prepared navy composition"))?;
    let EventView::Family(event) = event else {
        return Err(invalid("Missing prepared navy composition"));
    };
    if event.kind != MIX_KIND || event.fields.len != 3 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let known = |cell: Cell| match cell {
        Cell::Null => Ok(None),
        Cell::Integer(value) => i64::try_from(value).map(Some).map_err(|_| malformed()),
        _ => Err(malformed()),
    };
    raw.total = known(snapshot.field(event.fields, 0)?)?;
    raw.reference_total = known(snapshot.field(event.fields, 1)?)?;
    let Cell::List(pairs) = snapshot.field(event.fields, 2)? else {
        return Err(malformed());
    };
    let mut classes = BTreeMap::new();
    for at in 0..pairs.len / Cell::SIZE as u64 / 2 {
        let Cell::Unsigned(class) = snapshot.field(pairs, at * 2)? else {
            return Err(malformed());
        };
        let class = u32::try_from(class).map_err(|_| malformed())?;
        let amount = known(snapshot.field(pairs, at * 2 + 1)?)?.ok_or_else(malformed)?;
        classes.insert(class, amount);
    }
    if !classes.is_empty() {
        raw.classes = Arc::new(classes);
    }
    Ok(raw)
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

/// A chart of `key` over `view`, its bins laid from `bounds`, within `spans`; no bins, and
/// nothing read, for empty bounds.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    spans: &[HourSpan],
) -> io::Result<ChartSummary> {
    if bounds.0 <= bounds.1 {
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

fn dates(summary: &ChartSummary, dates: &mut BTreeSet<i32>) {
    for bucket in summary.buckets.iter().flatten() {
        dates.extend([
            bucket.first_hour,
            bucket.last_hour,
            bucket.min_hour,
            bucket.max_hour,
        ]);
    }
}

/// A series request of the `navy-size` family kind (`series-summary`, `series-state` or
/// `series-next-change`) answered from `snapshot`; nothing past `reach`, the coverage the
/// publication shares with the identity history while it is partial, is read.
#[allow(clippy::too_many_arguments)]
pub fn series<P: PageRead>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: &RecordedHistories,
    reach: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let through = |last: i32| reach.map_or(last, |reach| last.min(reach));
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let read_tags: Vec<String> = reads.iter().map(|read| read.tag.clone()).collect();
    let tags: Vec<String> = if product == "series-state" {
        read_tags.clone()
    } else {
        serde_json::from_value(params.get("tags").cloned().unwrap_or_else(|| json!([])))?
    };
    let selection: NavyParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": tags, "top": params["family"]["top"],
        "measure": params["family"]["measure"], "ownership": params["family"]["ownership"],
        "grouping": params["family"]["grouping"],
    }))?;
    NavySize::validate(&selection).map_err(dashboard_core::invalid)?;
    let ownership = selection.ownership.index();
    let strength = selection.measure == NavyMeasure::Strength;
    let index = channel(ownership, strength);
    let grouping = usize::from(selection.grouping == NavyGrouping::Type);
    let value_key = |tag: &str| {
        actor(directory, tag).and_then(|actor| directory.key(actor, VALUES + index as u64, None))
    };
    let mix_key = |tag: &str| {
        actor(directory, tag).and_then(|actor| directory.key(actor, MIX + index as u64, None))
    };
    let maximum_key = |tag: &str| {
        actor(directory, tag)
            .and_then(|actor| directory.key(actor, MAXIMUM + ownership as u64, None))
    };
    // The history's tags the family named, which ranks answer for.
    let named = history
        .countries
        .keys()
        .filter(|tag| actor(directory, tag).is_some())
        .count();
    let positions: Vec<u64> = directory
        .dimensions(FAMILY, POSITIONS + index as u64)
        .iter()
        .map(|&(_, key)| key)
        .collect();
    let ranked = |top: u32| &positions[..(top as usize).min(named).min(positions.len())];
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    if product == "series-next-change" {
        let mut navigation: series::Navigation = serde_json::from_value(params.clone())?;
        if let Some(reach) = reach.map(f64::from) {
            if reach < navigation.first_hour {
                return Ok(json!({ "hour": null }));
            }
            navigation.last_hour = navigation.last_hour.min(reach);
        }
        let mut keys: Vec<(u64, Option<Vec<HourSpan>>)> = Vec::new();
        if let Some(top) = selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut values = Vec::new();
            for &key in ranked(top) {
                keys.push((key, None));
                if let Some(run) = run_at(snapshot, Some(key), hour)? {
                    values.extend(value_key(&tag(directory, run.value)?));
                }
            }
            keys.extend(values.into_iter().map(|key| (key, None)));
        } else {
            // An alias's changes are the selection's only while a selected native country
            // holds it.
            for (tag, spans) in
                country_history::selected_spans(history, &selection.tags, &within, cancelled)?
            {
                if let Some(key) = value_key(&tag) {
                    keys.push((key, Some(spans)));
                }
            }
        }
        let keys: Vec<(u64, Option<&[HourSpan]>)> = keys
            .iter()
            .map(|(key, spans)| (*key, spans.as_deref()))
            .collect();
        return Ok(json!({ "hour": engine::navigate(snapshot, &keys, &navigation)? }));
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
        selection.top.map(|_| BTreeMap::new());
    if let Some(top) = selection.top {
        for &key in ranked(top) {
            for run in engine::runs(snapshot, key, first, through(last))? {
                let tag = tag(directory, run.value)?;
                if !history.countries.contains_key(&tag) {
                    return Err(invalid("Unknown navy rank key"));
                }
                membership
                    .as_mut()
                    .expect("ranked membership")
                    .entry(tag)
                    .or_default()
                    .push(HourSpan {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    });
            }
        }
    }
    if let Some(members) = &mut membership {
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
    let part_name = |id: u64| -> io::Result<String> {
        directory
            .scalar(PART_KEYS + id)
            .and_then(|scalar| scalar.text())
            .map(|text| String::from_utf8_lossy(text).into_owned())
            .ok_or_else(|| invalid("A prepared navy part names no key"))
    };
    let mut scanned = scanned(directory);
    scanned.classes = classes(directory)?;
    let empty = Arc::new(BTreeMap::new());
    let mut display = Vec::new();
    for tag in &wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (Some(key), Some(mix)) = (value_key(tag), mix_key(tag)) else {
            continue;
        };
        let country = history.countries.get(tag);
        let mut spans: Vec<HourSpan> = if let Some(members) = &membership {
            members.get(tag).cloned().unwrap_or_default()
        } else {
            country
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
        let line = summary(snapshot, key, bounds, view, &spans)?;
        let mut hours = BTreeSet::new();
        let mut parts = Vec::new();
        if dense {
            if strength && let Some(maximum) = maximum_key(tag) {
                dates(
                    &summary(snapshot, maximum, bounds, view, &spans)?,
                    &mut hours,
                );
            }
            let actor = actor(directory, tag).expect("named navy tag");
            let mut keyed = Vec::new();
            for &(part, key) in directory.dimensions(actor, emit::parts(index, grouping)) {
                keyed.push((part_name(part)?, key));
            }
            keyed.sort();
            for (name, key) in keyed {
                let part = summary(snapshot, key, bounds, view, &spans)?;
                dates(&part, &mut hours);
                parts.push(json!({ "key": name, "summary": part }));
            }
            dates(&line, &mut hours);
        }
        let runs: Vec<dashboard_series::Run> = if dense || product == "series-state" {
            if product == "series-state" {
                hours.extend(
                    reads
                        .iter()
                        .filter(|read| &read.tag == tag && read.hour == through(read.hour))
                        .map(|read| read.hour),
                );
            }
            let mut runs = BTreeMap::new();
            for &hour in &hours {
                if let Some(run) = run_at(snapshot, Some(mix), hour)? {
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
            let mut runs = engine::runs(snapshot, mix, first, through(last))?;
            // A run the window clips starts no segment at the window's first hour.
            if let Some(head) = runs.first_mut()
                && head.break_before
                && head.span.first == first
                && run_at(snapshot, Some(mix), first)?.is_some_and(|run| run.span.first < first)
            {
                head.break_before = false;
            }
            runs
        };
        let mut bounded = Vec::new();
        for run in runs {
            let value = raw_run(snapshot, mix, &run, &empty)?;
            let bounds = if product == "series-state" {
                (run.span.first, run.span.last)
            } else {
                (run.span.first.max(first), run.span.last.min(through(last)))
            };
            if bounds.0 <= bounds.1 {
                bounded.push((bounds, run.break_before, value));
            }
        }
        // An alias's other holders and hours outside `within` are not the selection.
        let mut raw = Vec::new();
        for_each_piece(
            &bounded,
            &spans,
            |&(bounds, _, _)| bounds,
            cancelled,
            |(bounds, starts, value), first_hour, last_hour| {
                raw.push(RawNavyRun {
                    first_hour,
                    last_hour,
                    starts_segment: *starts && first_hour == bounds.0,
                    ..value.clone()
                });
                Ok(())
            },
        )?;
        let held = if strength {
            &mut scanned.strength_series[ownership]
        } else {
            &mut scanned.series[ownership]
        };
        held.insert(tag.clone(), raw);
        scanned.segments.insert(
            tag.clone(),
            country
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
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Each tag's roster ships at `hour`, and whether its census was read then; none where it held
/// no value.
pub fn cells<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tags: &[&str],
    hour: i32,
) -> io::Result<Vec<(Option<i128>, bool)>> {
    let ships = channel(0, false) as u64;
    tags.iter()
        .map(|tag| {
            let Some(actor) = actor(directory, tag) else {
                return Ok((None, false));
            };
            let total = run_at(snapshot, directory.key(actor, VALUES + ships, None), hour)?;
            let read = run_at(snapshot, directory.key(actor, MIX + ships, None), hour)?;
            Ok((total.map(|run| run.value), read.is_some()))
        })
        .collect()
}
